//! Domain types shared across the core and exposed to the shell.

use serde::{Deserialize, Serialize};

/// What kind of media a catalog entry is.
///
/// `Raw` is catalogued in phase A (metadata + embedded preview) but not
/// developed; see `media::RawDecoder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhotoKind {
    Photo,
    Raw,
    Video,
}

impl PhotoKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PhotoKind::Photo => "photo",
            PhotoKind::Raw => "raw",
            PhotoKind::Video => "video",
        }
    }

    /// Parse from the string form stored in the database (infallible; unknown
    /// values fall back to the default variant).
    pub fn parse(s: &str) -> Self {
        match s {
            "raw" => PhotoKind::Raw,
            "video" => PhotoKind::Video,
            _ => PhotoKind::Photo,
        }
    }
}

/// Pick / reject flag, the Lightroom-style triage primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    None,
    Pick,
    Reject,
}

impl Flag {
    pub fn as_str(self) -> &'static str {
        match self {
            Flag::None => "none",
            Flag::Pick => "pick",
            Flag::Reject => "reject",
        }
    }

    /// Parse from the string form stored in the database (infallible; unknown
    /// values fall back to the default variant).
    pub fn parse(s: &str) -> Self {
        match s {
            "pick" => Flag::Pick,
            "reject" => Flag::Reject,
            _ => Flag::None,
        }
    }
}

/// A catalogued file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub id: i64,
    /// Path relative to the library root, always '/'-separated.
    pub rel_path: String,
    pub filename: String,
    pub content_hash: String,
    pub file_size: i64,
    pub mtime_ms: i64,
    pub kind: PhotoKind,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    /// ISO-8601 capture time from EXIF, when present.
    pub captured_at: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i64>,
    pub aperture: Option<f64>,
    pub shutter: Option<f64>,
    pub focal_length: Option<f64>,
    pub rating: u8,
    pub flag: Flag,
    pub color_label: Option<String>,
    /// Tiny base64 data URI used as a blur placeholder by the web gallery.
    pub blur_lqip: Option<String>,
    pub imported_at: String,
}

impl Photo {
    /// Camera make + model as one display string, matching what the web
    /// gallery's search indexes.
    pub fn camera(&self) -> Option<String> {
        let combined = [self.camera_make.as_deref(), self.camera_model.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let trimmed = combined.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }
}

/// Query filter for the photo grid. All conditions are ANDed; `None` means
/// "don't constrain on this".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhotoFilter {
    /// Minimum star rating (inclusive).
    pub min_rating: Option<u8>,
    pub flag: Option<Flag>,
    pub color_label: Option<String>,
    pub kind: Option<PhotoKind>,
    pub camera_model: Option<String>,
    /// Free text over filename and camera.
    pub text: Option<String>,
    /// Restrict to members of this album path.
    pub album_path: Option<String>,
    /// ISO date bounds on `captured_at` (inclusive), e.g. "2026-06-01".
    pub captured_from: Option<String>,
    pub captured_to: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub sort: PhotoSort,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhotoSort {
    #[default]
    CapturedAsc,
    CapturedDesc,
    NameAsc,
    NameDesc,
    RatingDesc,
    /// Position within an album; only meaningful with `album_path` set.
    AlbumOrder,
}

/// Album settings — mirrors the frontmatter schema the Astro site reads.
/// The mapping is asserted by a test in `publish`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: i64,
    /// e.g. "2026/weddings/ana-ivan"
    pub path: String,
    pub parent_path: Option<String>,
    pub title: String,
    pub description: Option<String>,
    /// ISO date (yyyy-mm-dd).
    pub date: Option<String>,
    /// Internal id stored in the site's access cookie. Grants nothing alone.
    pub token: String,
    pub password: Option<String>,
    /// Random secret enabling the "secret link" access mode.
    pub share_token: Option<String>,
    pub sort: String,
    pub style: String,
    pub cover_filename: Option<String>,
    pub is_collection: bool,
    pub hidden: bool,
    pub allow_download: bool,
    pub proofing: bool,
    pub sort_order: Option<i64>,
    pub tags: Vec<String>,
    pub body: Option<String>,
}

impl Album {
    /// An album is "locked" when it has a password and/or a share token —
    /// the same rule the web gallery applies.
    pub fn is_locked(&self) -> bool {
        self.password.is_some() || self.share_token.is_some()
    }

    /// Last path segment, used as the folder name.
    pub fn slug(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// Progress callback payload for long-running imports.
#[derive(Debug, Clone, Serialize)]
pub struct ImportProgress {
    pub processed: usize,
    pub total: usize,
    pub current: String,
}

/// Summary returned by an import run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportSummary {
    pub imported: usize,
    /// Files already in the catalog with the same content hash.
    pub duplicates: usize,
    /// Files whose content changed since last import.
    pub updated: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
}
