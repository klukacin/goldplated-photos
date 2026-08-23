//! Media decoding, metadata extraction and thumbnail generation.
//!
//! RAW support is deliberately behind [`RawDecoder`]. Phase A ships
//! [`NullRawDecoder`], which catalogues RAW files (metadata + embedded preview
//! when present) without developing them. Phase B drops in a LibRaw-backed
//! implementation and nothing else in the codebase changes.
//!
//! # Two things everything downstream assumes
//!
//! **Upright.** [`load_oriented`] applies the EXIF orientation on the way in, so
//! every size, every thumbnail and every developed render in this crate is the
//! photograph the right way up. A caller that reads pixel dimensions off the
//! file instead gets the sensor's idea of them, which for a portrait shot on a
//! turned camera is the other way round — [`swap_for_orientation`] is there for
//! the cases where decoding would be too expensive to bother.
//!
//! **Derived, never authoritative.** Everything this module writes lands under
//! `.gpp/thumbs` and can be deleted at any time; the originals are only ever
//! read. That is what makes clearing the cache a safe suggestion to give a
//! photographer over the phone.

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

/// The whole of the import filter: a file in a folder being imported is
/// catalogued if — and only if — its extension is in one of these three lists.
///
/// Nothing sniffs magic bytes, so a `.jpg` that is really a text file is
/// catalogued and fails later at decode, and a photograph saved with no
/// extension is simply not seen. Comparison is lowercase, so `.JPG` off a
/// camera card matches. Extending [`IMAGE_EXTENSIONS`] means promising [`decode`]
/// can open it — which for everything but HEIF means the `image` crate, and for
/// HEIF means the pure-Rust HEVC path. A format nothing can decode belongs in
/// [`RAW_EXTENSIONS`], where the catalog records the file and its metadata but
/// publish deliberately leaves it behind — a RAW is a negative, not something
/// to hand a client.
// Two spellings of the list rather than one with holes in it, because the
// `heif` feature decides membership: an extension in this list is a promise
// [`decode`] can keep, and a build without the HEVC decoder cannot keep it for
// HEIF. Off, a `.heic` is simply not seen — not catalogued, not flagged — which
// is exactly what every build did before HEIF support existed, and better than
// cataloguing a file whose every decode would fail.
#[cfg(feature = "heif")]
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "heic", "heif",
];
/// The same import filter, in a build without the HEVC decoder — see above.
#[cfg(not(feature = "heif"))]
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "tif", "tiff"];

/// Camera RAW extensions. Catalogued and previewed, never published.
pub const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "dng", "pef", "srw",
    "raw", "3fr", "iiq", "x3f",
];
/// Video extensions. These ride along into a published album untouched — no
/// thumbnail, no develop, no re-encode.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "mov", "avi", "mkv", "m4v"];

/// HEIF-family extensions, which the `image` crate cannot open — see
/// [`decode`].
#[cfg(feature = "heif")]
const HEIF_EXTENSIONS: &[&str] = &["heic", "heif"];

/// Whether this file takes the HEVC decode path. Compiled to `false` without
/// the `heif` feature, so callers — [`decode`], [`read_dimensions`], and
/// `publish::published_filename`'s rename-to-`.jpg` — need no cfg of their
/// own: everything HEIF-shaped simply stops happening.
#[cfg(feature = "heif")]
pub(crate) fn is_heif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| HEIF_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}
#[cfg(not(feature = "heif"))]
pub(crate) fn is_heif(_path: &Path) -> bool {
    false
}

/// Decode a still image, whatever container it arrived in.
///
/// The `image` crate covers everything here except HEIF, which is what an
/// iPhone shoots by default and what half the guests at a wedding will send.
/// That gap is filled by a pure-Rust HEVC decoder rather than libheif: libheif
/// is LGPL, which the licence policy does not allow, and linking C would cost
/// the portability contract that keeps an iPad build possible. The decoder is
/// behind the `heif` feature (on by default); without it, no HEIF file gets
/// this far — [`classify`] never catalogues one.
///
/// The price is speed — around 9 MP/s on one core, so roughly two and a half
/// seconds for a 24 MP frame against a few hundred milliseconds for JPEG.
/// Import runs across every core, so a card of them is minutes rather than
/// hours, but it is why a HEIC import is visibly slower than a JPEG one.
pub fn decode(path: &Path) -> Result<DynamicImage> {
    #[cfg(feature = "heif")]
    if is_heif(path) {
        return decode_heif(path);
    }
    Ok(image::open(path)?)
}

#[cfg(feature = "heif")]
fn decode_heif(path: &Path) -> Result<DynamicImage> {
    let decoded = heif_oxide::decode_file(path)
        .map_err(|e| Error::other(format!("{}: {e:?}", path.display())))?;
    let rgba = decoded.to_rgba8();
    image::RgbaImage::from_raw(decoded.width, decoded.height, rgba)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| Error::other(format!("{}: decoded pixels do not fit the frame", path.display())))
}

/// The frame's size, as cheaply as the format allows.
///
/// The metadata-only import path exists to be fast — it reads the header rather
/// than decoding twenty-four megapixels to learn two numbers. HEIF has no such
/// shortcut here, so it costs a full decode; a caller with EXIF dimensions
/// already in hand should not call this at all.
pub fn read_dimensions(path: &Path) -> Result<(u32, u32)> {
    #[cfg(feature = "heif")]
    if is_heif(path) {
        let img = decode_heif(path)?;
        return Ok((img.width(), img.height()));
    }
    Ok(image::image_dimensions(path)?)
}

/// Write a file so that nothing ever observes it half-finished.
///
/// The cache is read while it is being written — the webview loads thumbnails
/// from disk at the same moment a slider drag is re-rendering them — and a
/// reader that catches a partially written JPEG shows a broken image. Worse,
/// the `exists()` guards elsewhere in this module would then accept that
/// truncated file as a finished render. Writing beside the destination and
/// renaming into place means readers see either the old file or the new one.
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;

    // Unique per process and per call, so two threads rendering the same key
    // cannot collide on the temp file either.
    let temp = parent.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temp, bytes).map_err(|e| Error::io(&temp, e))?;
    match std::fs::rename(&temp, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(Error::io(dest, e))
        }
    }
}

/// JPEG quality for the grid and preview thumbnails.
///
/// Deliberately below [`crate::develop::DELIVERY_JPEG_QUALITY`]: these are
/// never delivered to anyone, they are redrawn constantly while culling, and at
/// a few hundred pixels the difference is invisible while the size is not. This
/// is the value the crate defaulted to before the encoder was made explicit, so
/// no existing thumbnail is invalidated by naming it.
pub(crate) const THUMBNAIL_JPEG_QUALITY: u8 = 75;

/// Encode an image as JPEG into memory, ready for [`write_atomic`].
pub(crate) fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    use image::ImageEncoder;

    // Spelled out rather than left to `write_to`, whose default is 75. That
    // default was reaching the one copy a client receives: an untouched photo
    // is published by copying the camera's own file, so an adjusted one going
    // out at 75 meant a single slider quietly cost the delivered frame detail
    // the camera had recorded.
    let rgb = img.to_rgb8();
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, quality)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(Error::Image)?;
    Ok(buf)
}

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
///
/// Every field is optional and every one of them is routinely absent: a
/// screenshot, a scan, a frame exported by another editor, a file a client
/// emailed. Nothing here may be treated as required, and `None` never means
/// "not read yet" — [`read_metadata`] returns a fully-populated `Metadata` or a
/// default one, and never an error.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    /// Sensor dimensions as EXIF records them — *before* orientation. For a
    /// portrait frame shot on a turned camera these are the landscape numbers;
    /// [`swap_for_orientation`] turns them into what the viewer will see.
    pub width: Option<u32>,
    /// See [`width`](Self::width) — the same caveat applies.
    pub height: Option<u32>,
    /// EXIF orientation, 1..8. Feed it to [`apply_orientation`] rather than
    /// interpreting it: 5–8 involve a mirror as well as a turn, and getting
    /// those four wrong flips a photograph in a way that looks almost right.
    pub orientation: Option<u16>,
    /// ISO-8601, UTC-naive (EXIF has no timezone).
    pub captured_at: Option<String>,
    /// Free text written by the camera body. Not sanitised here — these strings
    /// travel as far as the gallery's info overlay, which escapes them, because
    /// a photo from a second shooter can carry anything at all in them.
    pub camera_make: Option<String>,
    /// See [`camera_make`](Self::camera_make).
    pub camera_model: Option<String>,
    /// See [`camera_make`](Self::camera_make). Often absent even on bodies that
    /// record everything else — plenty of lenses do not report themselves.
    pub lens: Option<String>,
    pub iso: Option<i64>,
    /// f-number, so 2.8 means f/2.8.
    pub aperture: Option<f64>,
    /// Exposure time in seconds — 1/200 s arrives as 0.005, not as 200.
    pub shutter: Option<f64>,
    /// Millimetres, as recorded: the physical focal length, with no crop factor
    /// applied.
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
    /// Already oriented. A decoder that hands back sensor-order pixels and
    /// leaves the turn to its caller will have every RAW in the library
    /// published on its side.
    pub image: DynamicImage,
    /// What the decoder recovered, which for a RAW is usually richer than
    /// [`read_metadata`] can see — the EXIF crate reads containers, not
    /// proprietary maker formats.
    pub metadata: Metadata,
}

/// Pluggable RAW support.
///
/// Implementations are supplied by the shell so the core never links a RAW
/// library directly — that keeps licence decisions (LibRaw is LGPL-2.1 **or**
/// CDDL-1.0) at the edge of the build rather than baked into the core.
pub trait RawDecoder: Send + Sync {
    /// Whether this decoder handles a given lowercase extension, with no dot.
    /// Answering `true` and then failing in [`decode`](Self::decode) is worse
    /// than answering `false`: the file is catalogued either way, but a
    /// declined format falls back to the embedded preview instead of surfacing
    /// as an error the photographer has to read.
    fn supports(&self, ext: &str) -> bool;
    /// Develop the RAW into displayable, already-oriented pixels. Expensive by
    /// nature — callers cache the result under a render key rather than calling
    /// this per frame drawn.
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
    /// Dimensions of the photograph *as displayed* — the source decoded and
    /// turned upright, not the numbers in its EXIF. This is what the catalog
    /// stores and what the gallery reserves space with, so a portrait frame
    /// laid out from the sensor's width instead would leave a hole in the grid
    /// that fills in sideways.
    pub width: u32,
    /// See [`width`](Self::width).
    pub height: u32,
    /// base64 data URI of the tiny blur placeholder.
    pub lqip: Option<String>,
}

/// Load an image, applying EXIF orientation so downstream sizes are upright.
pub fn load_oriented(path: &Path, orientation: Option<u16>) -> Result<DynamicImage> {
    let img = decode(path)?;
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

/// What [`apply_orientation`] would do to a size, without decoding anything.
///
/// EXIF orientations 5–8 turn the image a quarter turn, so the stored width
/// and height are the other way round from how the photo is displayed.
pub fn swap_for_orientation(w: u32, h: u32, orientation: Option<u16>) -> (u32, u32) {
    match orientation.unwrap_or(1) {
        5..=8 => (h, w),
        _ => (w, h),
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

    // Lanczos3 either way. `fast_image_resize` does the convolution with SIMD,
    // which matters because import pays this three times per photograph — on a
    // 24 MP frame the image crate's resampler took ~0.4–0.5 s per size and fir
    // ~25 ms (measured, release build), so a thousand-frame wedding card keeps
    // or loses whole minutes here. A pixel layout fir has no kernel for falls
    // back to the image crate: slower, same picture.
    let mut dst = DynamicImage::new(nw, nh, img.color());
    let opts = fast_image_resize::ResizeOptions::new().resize_alg(
        fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3),
    );
    match fast_image_resize::Resizer::new().resize(img, &mut dst, &opts) {
        Ok(()) => dst,
        Err(_) => img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3),
    }
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
        let resized = resize_to_fit(&img, max_edge);
        write_atomic(&dest, &encode_jpeg(&resized, THUMBNAIL_JPEG_QUALITY)?)?;
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

    /// A reader must never catch a cache entry mid-write. Streaming straight
    /// into the destination — what this used to do — showed up in the app as
    /// broken thumbnails while a slider drag re-rendered the grid.
    #[test]
    fn concurrent_writers_never_expose_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("shard").join("entry.jpg");
        const LEN: usize = 512 * 1024;

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (dest, stop) = (dest.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut seen = 0usize;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(bytes) = std::fs::read(&dest) {
                        assert_eq!(bytes.len(), LEN, "a reader saw a half-written file");
                        seen += 1;
                    }
                }
                seen
            })
        };

        let writers: Vec<_> = (0u8..8)
            .map(|n| {
                let dest = dest.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        write_atomic(&dest, &vec![n; LEN]).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.join().unwrap();

        // And nothing is left behind beside it.
        let leftovers: Vec<_> = std::fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|name| name.starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left behind: {leftovers:?}");
    }

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
