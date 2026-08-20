//! Media decoding, metadata extraction and thumbnail generation.
//!
//! RAW support is deliberately behind [`RawDecoder`]. Phase A ships
//! [`NullRawDecoder`], which catalogues RAW files (metadata + embedded preview
//! when present) without developing them. Phase B drops in a LibRaw-backed
//! implementation and nothing else in the codebase changes.

use std::io::BufReader;
use std::path::Path;

use base64::Engine;
use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::PhotoKind;

/// Thumbnail sizes, matching what the web gallery serves.
pub const THUMB_SIZES: [(&str, u32); 3] = [("small", 400), ("medium", 1200), ("large", 1920)];
/// Long edge of the blur placeholder baked into the gallery HTML.
const LQIP_SIZE: u32 = 20;

pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "tif", "tiff"];
pub const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "dng", "pef", "srw",
    "raw", "3fr", "iiq", "x3f",
];
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "mov", "avi", "mkv", "m4v"];

/// Classify a file by extension.
pub fn classify(path: &Path) -> Option<PhotoKind> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        Some(PhotoKind::Photo)
    } else if RAW_EXTENSIONS.contains(&ext.as_str()) {
        Some(PhotoKind::Raw)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(PhotoKind::Video)
    } else {
        None
    }
}

// ------------------------------------------------------------------ metadata

/// Camera metadata read from EXIF.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    /// ISO-8601, UTC-naive (EXIF has no timezone).
    pub captured_at: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i64>,
    pub aperture: Option<f64>,
    pub shutter: Option<f64>,
    pub focal_length: Option<f64>,
}

/// Extract EXIF metadata. Missing or malformed EXIF is not an error — plenty of
/// perfectly good files have none.
pub fn read_metadata(path: &Path) -> Metadata {
    let mut meta = Metadata::default();

    let Ok(file) = std::fs::File::open(path) else {
        return meta;
    };
    let mut reader = BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let Ok(exif) = exif_reader.read_from_container(&mut reader) else {
        return meta;
    };

    use exif::{In, Tag, Value};

    let uint = |tag: Tag| -> Option<i64> {
        exif.get_field(tag, In::PRIMARY).and_then(|f| match &f.value {
            Value::Short(v) => v.first().map(|x| *x as i64),
            Value::Long(v) => v.first().map(|x| *x as i64),
            _ => None,
        })
    };
    let rational = |tag: Tag| -> Option<f64> {
        exif.get_field(tag, In::PRIMARY).and_then(|f| match &f.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            _ => None,
        })
    };
    let text = |tag: Tag| -> Option<String> {
        exif.get_field(tag, In::PRIMARY).map(|f| {
            f.display_value()
                .to_string()
                .trim_matches('"')
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
    };

    meta.orientation = uint(Tag::Orientation).map(|v| v as u16);
    meta.camera_make = text(Tag::Make);
    meta.camera_model = text(Tag::Model);
    meta.lens = text(Tag::LensModel);
    meta.iso = uint(Tag::PhotographicSensitivity);
    meta.aperture = rational(Tag::FNumber);
    meta.shutter = rational(Tag::ExposureTime);
    meta.focal_length = rational(Tag::FocalLength);

    // EXIF dates look like "2026:06:14 10:30:00"; normalise to ISO-8601.
    meta.captured_at = text(Tag::DateTimeOriginal)
        .or_else(|| text(Tag::DateTime))
        .and_then(|raw| normalize_exif_datetime(&raw));

    meta
}

/// `"2026:06:14 10:30:00"` → `"2026-06-14T10:30:00"`.
pub fn normalize_exif_datetime(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (date, time) = raw.split_once(' ')?;
    let parts: Vec<&str> = date.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    // Reject the all-zero placeholder some cameras write.
    if parts[0] == "0000" {
        return None;
    }
    Some(format!("{}-{}-{}T{}", parts[0], parts[1], parts[2], time))
}

// ------------------------------------------------------------------ RAW hook

/// Decoded RAW image plus whatever metadata the decoder recovered.
pub struct DecodedRaw {
    pub image: DynamicImage,
    pub metadata: Metadata,
}

/// Pluggable RAW support.
///
/// Implementations are supplied by the shell so the core never links a RAW
/// library directly — that keeps licence decisions (LibRaw is LGPL-2.1 **or**
/// CDDL-1.0) at the edge of the build rather than baked into the core.
pub trait RawDecoder: Send + Sync {
    fn supports(&self, ext: &str) -> bool;
    fn decode(&self, path: &Path) -> Result<DecodedRaw>;
    /// The embedded JPEG most RAW files carry — enough to show a grid.
    fn embedded_preview(&self, path: &Path) -> Result<Option<Vec<u8>>>;
}

/// Phase-A stand-in: knows nothing, decodes nothing.
pub struct NullRawDecoder;

impl RawDecoder for NullRawDecoder {
    fn supports(&self, _ext: &str) -> bool {
        false
    }
    fn decode(&self, path: &Path) -> Result<DecodedRaw> {
        Err(Error::Unsupported(format!(
            "RAW decoding not built in this version: {}",
            path.display()
        )))
    }
    fn embedded_preview(&self, _path: &Path) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

// --------------------------------------------------------------- thumbnails

/// Result of generating derived images for one file.
#[derive(Debug, Clone, Default)]
pub struct Derived {
    pub width: u32,
    pub height: u32,
    /// base64 data URI of the tiny blur placeholder.
    pub lqip: Option<String>,
}

/// Load an image, applying EXIF orientation so downstream sizes are upright.
pub fn load_oriented(path: &Path, orientation: Option<u16>) -> Result<DynamicImage> {
    let img = image::open(path)?;
    Ok(apply_orientation(img, orientation))
}

/// Apply the EXIF orientation transform (values 1..8).
pub fn apply_orientation(img: DynamicImage, orientation: Option<u16>) -> DynamicImage {
    match orientation.unwrap_or(1) {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// Resize preserving aspect ratio so the long edge is at most `max_edge`.
/// Never enlarges.
pub fn resize_to_fit(img: &DynamicImage, max_edge: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || (w <= max_edge && h <= max_edge) {
        return img.clone();
    }
    let scale = max_edge as f64 / w.max(h) as f64;
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    // Lanczos3 for quality; fast_image_resize accelerates this with SIMD when
    // the feature set allows, falling back cleanly otherwise.
    img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3)
}

/// Content-addressed thumbnail location: `<thumbs>/<hash[0:2]>/<hash>_<size>.jpg`.
///
/// Addressing by content rather than path means moving or renaming a file
/// costs nothing, and re-importing an identical file reuses existing work.
pub fn thumb_path(thumb_root: &Path, content_hash: &str, size_name: &str) -> std::path::PathBuf {
    let shard = &content_hash[..2.min(content_hash.len())];
    thumb_root
        .join(shard)
        .join(format!("{content_hash}_{size_name}.jpg"))
}

/// Generate all thumbnail sizes plus the blur placeholder for one image.
///
/// Existing thumbnails are left alone, so re-running an import is cheap.
pub fn generate_derived(
    source: &Path,
    thumb_root: &Path,
    content_hash: &str,
    orientation: Option<u16>,
) -> Result<Derived> {
    let img = load_oriented(source, orientation)?;
    let mut out = Derived {
        width: img.width(),
        height: img.height(),
        lqip: None,
    };

    for (name, max_edge) in THUMB_SIZES {
        let dest = thumb_path(thumb_root, content_hash, name);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let resized = resize_to_fit(&img, max_edge);
        let mut file = std::fs::File::create(&dest).map_err(|e| Error::io(&dest, e))?;
        resized
            .to_rgb8()
            .write_to(&mut file, ImageFormat::Jpeg)
            .map_err(Error::Image)?;
    }

    out.lqip = Some(make_lqip(&img)?);
    Ok(out)
}

/// Tiny base64 JPEG used as a blur-up placeholder in the web grid.
pub fn make_lqip(img: &DynamicImage) -> Result<String> {
    let small = resize_to_fit(img, LQIP_SIZE);
    let mut buf = std::io::Cursor::new(Vec::new());
    small
        .to_rgb8()
        .write_to(&mut buf, ImageFormat::Jpeg)
        .map_err(Error::Image)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
    Ok(format!("data:image/jpeg;base64,{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classifies_by_extension() {
        assert_eq!(classify(&PathBuf::from("a.JPG")), Some(PhotoKind::Photo));
        assert_eq!(classify(&PathBuf::from("a.cr3")), Some(PhotoKind::Raw));
        assert_eq!(classify(&PathBuf::from("a.mp4")), Some(PhotoKind::Video));
        assert_eq!(classify(&PathBuf::from("a.txt")), None);
        assert_eq!(classify(&PathBuf::from("noext")), None);
    }

    #[test]
    fn normalizes_exif_dates() {
        assert_eq!(
            normalize_exif_datetime("2026:06:14 10:30:00").as_deref(),
            Some("2026-06-14T10:30:00")
        );
        assert_eq!(normalize_exif_datetime("0000:00:00 00:00:00"), None);
        assert_eq!(normalize_exif_datetime("garbage"), None);
    }

    #[test]
    fn resize_never_enlarges() {
        let img = DynamicImage::new_rgb8(100, 50);
        let out = resize_to_fit(&img, 400);
        assert_eq!((out.width(), out.height()), (100, 50));
    }

    #[test]
    fn resize_fits_long_edge() {
        let img = DynamicImage::new_rgb8(2000, 1000);
        let out = resize_to_fit(&img, 400);
        assert_eq!(out.width(), 400);
        assert_eq!(out.height(), 200);
    }

    #[test]
    fn thumb_paths_are_sharded() {
        let p = thumb_path(Path::new("/t"), "abcdef", "small");
        assert_eq!(p, PathBuf::from("/t/ab/abcdef_small.jpg"));
    }

    #[test]
    fn orientation_6_rotates_portrait() {
        let img = DynamicImage::new_rgb8(100, 50);
        let out = apply_orientation(img, Some(6));
        assert_eq!((out.width(), out.height()), (50, 100));
    }

    #[test]
    fn null_raw_decoder_declines() {
        let d = NullRawDecoder;
        assert!(!d.supports("cr3"));
        assert!(d.decode(Path::new("x.cr3")).is_err());
        assert!(d.embedded_preview(Path::new("x.cr3")).unwrap().is_none());
    }
}
