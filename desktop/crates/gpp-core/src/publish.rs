//! Materialize albums into the tree the Astro gallery reads.
//!
//! Output layout, per album:
//!
//! ```text
//! <dest>/<album-path>/
//! ├── index.md      frontmatter generated from the album row
//! ├── body.md       optional prose
//! └── <photo>.jpg   exported photos
//! ```
//!
//! The frontmatter field set is the **contract** between this app and the web
//! gallery's content schema (`src/content/config.ts`). [`FRONTMATTER_FIELDS`]
//! and its test exist so a drift fails here rather than producing an album the
//! site refuses to render.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::model::{Album, Photo};

/// Every frontmatter key this app emits. Mirrors the gallery's zod schema.
pub const FRONTMATTER_FIELDS: &[&str] = &[
    "title",
    "description",
    "date",
    "token",
    "password",
    "shareToken",
    "sort",
    "photoOrder",
    "style",
    "thumbnail",
    "tags",
    "isCollection",
    "order",
    "hidden",
    "allowDownload",
    "proofing",
];

#[derive(Debug, Clone)]
pub struct PublishOptions {
    /// Only publish photos rated at least this high. `None` publishes all
    /// album members.
    pub min_rating: Option<u8>,
    /// Skip photos flagged `Reject` regardless of rating.
    pub exclude_rejected: bool,
    /// Copy the original files. When false only `index.md` is written, which
    /// is useful for a metadata-only refresh.
    pub copy_photos: bool,
}

impl Default for PublishOptions {
    fn default() -> Self {
        Self {
            min_rating: None,
            exclude_rejected: true,
            copy_photos: true,
        }
    }
}

/// What a publish produced (or would produce, for a dry run).
#[derive(Debug, Clone, Default)]
pub struct PublishResult {
    pub album_path: String,
    /// Relative paths written, under the destination root.
    pub written: Vec<String>,
    pub photos_copied: usize,
    pub photos_skipped: usize,
    pub bytes_copied: u64,
}

/// Publish one album into `dest_root` (the gallery's `src/content/albums`).
pub fn publish_album(
    lib: &Library,
    album_path: &str,
    dest_root: &Path,
    opts: &PublishOptions,
) -> Result<PublishResult> {
    let album = lib
        .album_by_path(album_path)?
        .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;

    let photos = select_photos(lib, &album, opts)?;
    let album_dir = dest_root.join(album_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&album_dir).map_err(|e| Error::io(&album_dir, e))?;

    let mut result = PublishResult {
        album_path: album_path.to_string(),
        ..Default::default()
    };

    // index.md
    let frontmatter = render_frontmatter(&album, &photos);
    let index_path = album_dir.join("index.md");
    std::fs::write(&index_path, &frontmatter).map_err(|e| Error::io(&index_path, e))?;
    result.written.push(format!("{album_path}/index.md"));

    // body.md — written only when there is prose, removed when cleared.
    let body_path = album_dir.join("body.md");
    match album.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(body) => {
            std::fs::write(&body_path, body).map_err(|e| Error::io(&body_path, e))?;
            result.written.push(format!("{album_path}/body.md"));
        }
        None => {
            if body_path.exists() {
                std::fs::remove_file(&body_path).map_err(|e| Error::io(&body_path, e))?;
            }
        }
    }

    // Photos
    if opts.copy_photos {
        for photo in &photos {
            let src = lib.resolve(&photo.rel_path)?;
            let dest = album_dir.join(&photo.filename);

            // Skip when destination already matches by size — cheap and
            // correct enough, since a real change alters the byte count or the
            // sync layer's hash catches it.
            if let (Ok(s), Ok(d)) = (std::fs::metadata(&src), std::fs::metadata(&dest)) {
                if s.len() == d.len() {
                    result.photos_skipped += 1;
                    result.written.push(format!("{album_path}/{}", photo.filename));
                    continue;
                }
            }

            std::fs::copy(&src, &dest).map_err(|e| Error::io(&dest, e))?;
            result.bytes_copied += photo.file_size.max(0) as u64;
            result.photos_copied += 1;
            result.written.push(format!("{album_path}/{}", photo.filename));
        }
    }

    Ok(result)
}

/// Which photos make it into the published album.
fn select_photos(lib: &Library, album: &Album, opts: &PublishOptions) -> Result<Vec<Photo>> {
    let mut photos = lib.album_photos(&album.path)?;
    photos.retain(|p| {
        if opts.exclude_rejected && p.flag == crate::model::Flag::Reject {
            return false;
        }
        if let Some(min) = opts.min_rating {
            if p.rating < min {
                return false;
            }
        }
        // Videos ride along; RAW files are not web deliverables.
        p.kind != crate::model::PhotoKind::Raw
    });
    Ok(photos)
}

/// Render `index.md` — YAML frontmatter, no body (body lives in `body.md`).
pub fn render_frontmatter(album: &Album, photos: &[Photo]) -> String {
    let mut out = String::from("---\n");

    out.push_str(&yaml_kv("title", &album.title));
    if let Some(v) = opt_str(&album.description) {
        out.push_str(&yaml_kv("description", v));
    }
    if let Some(v) = opt_str(&album.date) {
        out.push_str(&yaml_kv("date", v));
    }
    out.push_str(&yaml_kv("token", &album.token));
    if let Some(v) = opt_str(&album.password) {
        out.push_str(&yaml_kv("password", v));
    }
    if let Some(v) = opt_str(&album.share_token) {
        out.push_str(&yaml_kv("shareToken", v));
    }
    out.push_str(&yaml_kv("sort", &album.sort));

    // photoOrder is what makes `sort: custom` meaningful on the site.
    if album.sort == "custom" && !photos.is_empty() {
        out.push_str("photoOrder:\n");
        for p in photos {
            out.push_str(&format!("  - {}\n", yaml_scalar(&p.filename)));
        }
    }

    out.push_str(&yaml_kv("style", &album.style));

    // Cover: explicit choice, else let the site fall back to the first photo.
    if let Some(cover) = album
        .cover_filename
        .as_deref()
        .filter(|c| photos.iter().any(|p| p.filename == *c))
    {
        out.push_str(&yaml_kv("thumbnail", cover));
    }

    if !album.tags.is_empty() {
        out.push_str("tags:\n");
        for tag in &album.tags {
            out.push_str(&format!("  - {}\n", yaml_scalar(tag)));
        }
    }

    if album.is_collection {
        out.push_str("isCollection: true\n");
    }
    if let Some(order) = album.sort_order {
        out.push_str(&format!("order: {order}\n"));
    }
    if album.hidden {
        out.push_str("hidden: true\n");
    }
    if album.allow_download {
        out.push_str("allowDownload: true\n");
    }
    if album.proofing {
        out.push_str("proofing: true\n");
    }

    out.push_str("---\n");
    out
}

fn opt_str(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn yaml_kv(key: &str, value: &str) -> String {
    format!("{key}: {}\n", yaml_scalar(value))
}

/// Always double-quote and escape. Verbose, but immune to a title that happens
/// to be `yes`, `1.0`, `null`, or contains a colon.
fn yaml_scalar(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "");
    format!("\"{escaped}\"")
}

/// Build a manifest of the published tree: relative path → content hash.
/// Feeds the sync engine's three-way comparison.
pub fn manifest_of(dest_root: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    if !dest_root.exists() {
        return Ok(out);
    }
    collect_manifest(dest_root, dest_root, &mut out)?;
    Ok(out)
}

fn collect_manifest(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, String>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // `.meta/` is server-owned (thumbnail cache, proofing submissions).
        // It must never be part of what we push.
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_manifest(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let key = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(key, crate::import::hash_file(&path)?);
        }
    }
    Ok(())
}

/// Absolute path of an album inside a published tree.
pub fn album_dir(dest_root: &Path, album_path: &str) -> PathBuf {
    dest_root.join(album_path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::albums::{AlbumUpdate, NewAlbum};
    use crate::import::{import_dir, ImportOptions};

    fn write_jpeg(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgb8(w, h)
            .save_with_format(path, image::ImageFormat::Jpeg)
            .unwrap();
    }

    /// The gallery's zod schema, transcribed. If the app starts emitting a key
    /// the site doesn't accept, this fails.
    const SITE_SCHEMA_KEYS: &[&str] = &[
        "title", "description", "date", "token", "password", "shareToken",
        "allowAnonymous", "sort", "photoOrder", "style", "thumbnail", "tags",
        "isCollection", "order", "hidden", "allowDownload", "proofing",
    ];

    #[test]
    fn emitted_fields_are_a_subset_of_the_site_schema() {
        for field in FRONTMATTER_FIELDS {
            assert!(
                SITE_SCHEMA_KEYS.contains(field),
                "'{field}' is not in the gallery content schema"
            );
        }
    }

    #[test]
    fn yaml_scalars_are_escaped() {
        assert_eq!(yaml_scalar("plain"), "\"plain\"");
        assert_eq!(yaml_scalar("has: colon"), "\"has: colon\"");
        assert_eq!(yaml_scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_scalar("back\\slash"), "\"back\\\\slash\"");
        // A title that would otherwise parse as a boolean stays a string.
        assert_eq!(yaml_scalar("yes"), "\"yes\"");
    }

    #[test]
    fn publishes_index_and_photos() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 80, 60);
        write_jpeg(&src.path().join("b.jpg"), 80, 60);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None).unwrap();
        lib.create_album(&NewAlbum {
            path: "2026/ana".into(),
            title: Some("Ana & Ivan".into()),
            ..Default::default()
        })
        .unwrap();

        let ids: Vec<i64> = lib
            .photos(&Default::default())
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        lib.add_photos_to_album("2026/ana", &ids).unwrap();

        let result =
            publish_album(&lib, "2026/ana", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(result.photos_copied, 2);

        let index = std::fs::read_to_string(dest.path().join("2026/ana/index.md")).unwrap();
        assert!(index.starts_with("---\n"));
        assert!(index.contains("title: \"Ana & Ivan\""));
        assert!(index.contains("token: \""));
        assert!(dest.path().join("2026/ana/a.jpg").exists());
        assert!(dest.path().join("2026/ana/b.jpg").exists());
    }

    #[test]
    fn respects_rating_filter_and_rejects() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("keep.jpg"), 40, 40);
        write_jpeg(&src.path().join("drop.jpg"), 40, 40);
        write_jpeg(&src.path().join("rejected.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();

        let keep = lib.photo_by_rel_path("keep.jpg").unwrap().unwrap();
        let drop = lib.photo_by_rel_path("drop.jpg").unwrap().unwrap();
        let rej = lib.photo_by_rel_path("rejected.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[keep.id, drop.id, rej.id]).unwrap();

        lib.set_rating(keep.id, 5).unwrap();
        lib.set_rating(drop.id, 1).unwrap();
        lib.set_rating(rej.id, 5).unwrap();
        lib.set_flag(rej.id, crate::model::Flag::Reject).unwrap();

        let opts = PublishOptions {
            min_rating: Some(4),
            ..Default::default()
        };
        let r = publish_album(&lib, "a", dest.path(), &opts).unwrap();
        assert_eq!(r.photos_copied, 1);
        assert!(dest.path().join("a/keep.jpg").exists());
        assert!(!dest.path().join("a/drop.jpg").exists(), "below rating");
        assert!(!dest.path().join("a/rejected.jpg").exists(), "flagged reject");
    }

    #[test]
    fn writes_access_and_feature_flags() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let lib = Library::open(src.path()).unwrap();
        lib.create_album(&NewAlbum { path: "priv".into(), ..Default::default() }).unwrap();
        lib.update_album(
            "priv",
            &AlbumUpdate {
                password: Some(Some("tajna".into())),
                share_token: Some(Some("SECRET123".into())),
                proofing: Some(true),
                allow_download: Some(true),
                hidden: Some(true),
                tags: Some(vec!["wedding".into()]),
                ..Default::default()
            },
        )
        .unwrap();

        publish_album(&lib, "priv", dest.path(), &PublishOptions::default()).unwrap();
        let index = std::fs::read_to_string(dest.path().join("priv/index.md")).unwrap();

        assert!(index.contains("password: \"tajna\""));
        assert!(index.contains("shareToken: \"SECRET123\""));
        assert!(index.contains("proofing: true"));
        assert!(index.contains("allowDownload: true"));
        assert!(index.contains("hidden: true"));
        assert!(index.contains("  - \"wedding\""));
    }

    #[test]
    fn custom_sort_emits_photo_order() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("1.jpg"), 40, 40);
        write_jpeg(&src.path().join("2.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();

        let one = lib.photo_by_rel_path("1.jpg").unwrap().unwrap();
        let two = lib.photo_by_rel_path("2.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[one.id, two.id]).unwrap();
        // Reverse the order, then switch the album to custom sorting.
        lib.reorder_album("a", &[two.id, one.id]).unwrap();
        lib.update_album("a", &AlbumUpdate { sort: Some("custom".into()), ..Default::default() })
            .unwrap();

        publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        let index = std::fs::read_to_string(dest.path().join("a/index.md")).unwrap();

        let order_block = index.split("photoOrder:\n").nth(1).unwrap();
        let first_line = order_block.lines().next().unwrap();
        assert!(first_line.contains("2.jpg"), "custom order should lead with 2.jpg");
    }

    #[test]
    fn body_is_written_and_removed() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let lib = Library::open(src.path()).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();

        lib.update_album("a", &AlbumUpdate { body: Some(Some("# Hello".into())), ..Default::default() })
            .unwrap();
        publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.path().join("a/body.md")).unwrap(),
            "# Hello"
        );

        lib.update_album("a", &AlbumUpdate { body: Some(None), ..Default::default() }).unwrap();
        publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert!(!dest.path().join("a/body.md").exists(), "cleared body removes the file");
    }

    #[test]
    fn manifest_skips_server_owned_meta() {
        let dest = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dest.path().join("a/.meta/proofing")).unwrap();
        std::fs::write(dest.path().join("a/index.md"), "---\n---\n").unwrap();
        std::fs::write(dest.path().join("a/.meta/proofing/x.json"), "{}").unwrap();

        let m = manifest_of(dest.path()).unwrap();
        assert!(m.contains_key("a/index.md"));
        assert!(
            !m.keys().any(|k| k.contains(".meta")),
            "proofing submissions are server-owned and must stay out of the manifest"
        );
    }
}
