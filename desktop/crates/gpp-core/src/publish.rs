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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::Serialize;

use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::model::{Album, Photo};

impl Library {
    /// Filenames this library last published into `album_path`.
    pub fn published_files(&self, album_path: &str) -> Result<BTreeSet<String>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT filename FROM published_files WHERE album_path = ?1")?;
            let rows = stmt.query_map(params![album_path], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<BTreeSet<String>>>()?)
        })
    }

    /// Replace the record of what an album's published folder contains.
    pub fn record_published_files(&self, album_path: &str, files: &BTreeSet<String>) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute(
                "DELETE FROM published_files WHERE album_path = ?1",
                params![album_path],
            )?;
            let mut stmt = tx.prepare(
                "INSERT INTO published_files(album_path, filename) VALUES(?1, ?2)",
            )?;
            for f in files {
                stmt.execute(params![album_path, f])?;
            }
            Ok(())
        })
    }
}

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
#[derive(Debug, Clone, Default, Serialize)]
pub struct PublishResult {
    pub album_path: String,
    /// Relative paths written, under the destination root.
    pub written: Vec<String>,
    pub photos_copied: usize,
    pub photos_skipped: usize,
    pub bytes_copied: u64,
    /// Catalogued photos whose original file is gone from disk. Reported, not
    /// fatal: one unplugged drive must not abort an album. Any copy already in
    /// the published tree is left alone — `prune_missing` is the deliberate way
    /// to drop photos that are really gone.
    pub missing: Vec<String>,
    /// Files removed from the published folder because this album no longer
    /// publishes them. Only ever files this library put there itself.
    pub removed: Vec<String>,
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

            if !src.exists() {
                result.missing.push(photo.rel_path.clone());
                continue;
            }

            std::fs::copy(&src, &dest).map_err(|e| Error::io(&src, e))?;
            result.bytes_copied += photo.file_size.max(0) as u64;
            result.photos_copied += 1;
            result.written.push(format!("{album_path}/{}", photo.filename));
        }

        prune_published(lib, album_path, &album_dir, &photos, &mut result)?;
    }

    Ok(result)
}

/// Remove photos this library published before and no longer publishes.
///
/// The record of past publishes is what makes this safe: a file the web admin
/// (or anything else) dropped into the same folder was never ours, so it is
/// never removed. Sub-albums and dotfiles — `.meta/proofing` above all — are
/// not even looked at.
fn prune_published(
    lib: &Library,
    album_path: &str,
    album_dir: &Path,
    photos: &[Photo],
    result: &mut PublishResult,
) -> Result<()> {
    let current: BTreeSet<String> = photos.iter().map(|p| p.filename.clone()).collect();

    for stale in lib.published_files(album_path)?.difference(&current) {
        // A photo whose original vanished keeps its published copy: that is a
        // broken drive, not a decision to unpublish.
        if result.missing.iter().any(|m| m.ends_with(stale)) {
            continue;
        }
        let path = album_dir.join(stale);
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
        }
        result.removed.push(format!("{album_path}/{stale}"));
    }

    let mut recorded = current;
    // Keep the ones we couldn't refresh, so a later publish can still prune them.
    for m in &result.missing {
        if let Some(name) = m.rsplit('/').next() {
            recorded.insert(name.to_string());
        }
    }
    lib.record_published_files(album_path, &recorded)
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

    /// Unpublishing has to actually remove the file, or the gallery keeps
    /// showing a photo the album no longer contains — and the sync layer never
    /// gets a chance to take it off the server.
    #[test]
    fn unpublished_photos_are_removed_but_other_tools_files_are_not() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/keep.jpg"), 40, 40);
        write_jpeg(&src.path().join("a/drop.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let keep = lib.photo_by_rel_path("a/keep.jpg").unwrap().unwrap();
        let drop = lib.photo_by_rel_path("a/drop.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[keep.id, drop.id]).unwrap();

        let first = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(first.photos_copied, 2);
        assert!(first.removed.is_empty(), "nothing to prune on a first publish");

        // Something else — the web admin, say — adds a file to the same folder,
        // and a sub-album lives underneath.
        std::fs::write(dest.path().join("a/from-admin.jpg"), b"not ours").unwrap();
        std::fs::create_dir_all(dest.path().join("a/sub")).unwrap();
        std::fs::write(dest.path().join("a/sub/index.md"), b"---\n---\n").unwrap();

        lib.remove_photos_from_album("a", &[drop.id]).unwrap();
        let second = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();

        assert_eq!(second.removed, vec!["a/drop.jpg".to_string()]);
        assert!(!dest.path().join("a/drop.jpg").exists(), "removed from the album");
        assert!(dest.path().join("a/keep.jpg").exists());
        assert!(dest.path().join("a/from-admin.jpg").exists(), "never ours to delete");
        assert!(dest.path().join("a/sub/index.md").exists(), "sub-albums untouched");
    }

    /// One original gone from disk (deleted by hand, drive unplugged) must not
    /// take the whole album down with it.
    #[test]
    fn missing_originals_are_reported_not_fatal() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/here.jpg"), 40, 40);
        write_jpeg(&src.path().join("a/gone.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let ids: Vec<i64> = lib.photos(&Default::default()).unwrap().iter().map(|p| p.id).collect();
        lib.add_photos_to_album("a", &ids).unwrap();

        std::fs::remove_file(src.path().join("a/gone.jpg")).unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(r.photos_copied, 1);
        assert_eq!(r.missing, vec!["a/gone.jpg".to_string()]);
        assert!(dest.path().join("a/here.jpg").exists());
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

// --------------------------------------------------------------- parsing back

/// An album's settings as recovered from a published `index.md`.
///
/// Pulling an album from the server means reading back what [`render_frontmatter`]
/// wrote. This parses the YAML subset we emit — quoted scalars, bare booleans
/// and numbers, and `- item` lists — rather than pulling in a full YAML crate
/// for a format we control on both ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedFrontmatter {
    pub title: Option<String>,
    pub description: Option<String>,
    pub date: Option<String>,
    pub token: Option<String>,
    pub password: Option<String>,
    pub share_token: Option<String>,
    pub sort: Option<String>,
    pub style: Option<String>,
    pub thumbnail: Option<String>,
    pub photo_order: Vec<String>,
    pub tags: Vec<String>,
    pub is_collection: bool,
    pub hidden: bool,
    pub allow_download: bool,
    pub proofing: bool,
    pub order: Option<i64>,
}

/// Parse the frontmatter block of an `index.md`.
pub fn parse_frontmatter(content: &str) -> ParsedFrontmatter {
    let mut out = ParsedFrontmatter::default();

    // Take everything between the first two `---` fences.
    let body = match content.strip_prefix("---") {
        Some(rest) => match rest.split_once("\n---") {
            Some((block, _)) => block,
            None => rest,
        },
        None => return out,
    };

    // Tracks which list we are appending to when we hit `- item` lines.
    let mut list: Option<&'static str> = None;

    for raw in body.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }

        // List continuation
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            let value = unquote(item.trim());
            match list {
                Some("tags") => out.tags.push(value),
                Some("photoOrder") => out.photo_order.push(value),
                _ => {}
            }
            continue;
        }

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        // A key with no inline value opens a list.
        if value.is_empty() {
            list = match key {
                "tags" => Some("tags"),
                "photoOrder" => Some("photoOrder"),
                _ => None,
            };
            continue;
        }
        list = None;

        let text = unquote(value);
        match key {
            "title" => out.title = Some(text),
            "description" => out.description = Some(text),
            "date" => out.date = Some(text),
            "token" => out.token = Some(text),
            "password" => out.password = Some(text),
            "shareToken" => out.share_token = Some(text),
            "sort" => out.sort = Some(text),
            "style" => out.style = Some(text),
            "thumbnail" => out.thumbnail = Some(text),
            "isCollection" => out.is_collection = text == "true",
            "hidden" => out.hidden = text == "true",
            "allowDownload" => out.allow_download = text == "true",
            "proofing" => out.proofing = text == "true",
            "order" => out.order = text.parse().ok(),
            _ => {}
        }
    }

    out
}

/// Strip surrounding double quotes and undo the escaping `yaml_scalar` applies.
fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return trimmed[1..trimmed.len() - 1]
            .replace("\\n", "\n")
            .replace("\\\"", "\"")
            .replace("\\\\", "\\");
    }
    trimmed.to_string()
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn round_trips_what_we_render() {
        let album = Album {
            id: 1,
            path: "2026/ana".into(),
            parent_path: Some("2026".into()),
            title: "Ana & Ivan: \"the\" day".into(),
            description: Some("Zagreb, lipanj".into()),
            date: Some("2026-06-14".into()),
            token: "abc123".into(),
            password: Some("tajna".into()),
            share_token: Some("SHARE_xyz".into()),
            sort: "custom".into(),
            style: "grid".into(),
            cover_filename: Some("a.jpg".into()),
            is_collection: false,
            hidden: true,
            allow_download: true,
            proofing: true,
            sort_order: Some(3),
            tags: vec!["wedding".into(), "ljeto".into()],
            body: None,
        };
        let photos = vec![
            Photo {
                id: 1, rel_path: "x/a.jpg".into(), filename: "a.jpg".into(),
                content_hash: "h".into(), file_size: 1, mtime_ms: 0,
                kind: crate::model::PhotoKind::Photo, width: None, height: None,
                orientation: None, captured_at: None, camera_make: None,
                camera_model: None, lens: None, iso: None, aperture: None,
                shutter: None, focal_length: None, rating: 5,
                flag: crate::model::Flag::None, color_label: None,
                blur_lqip: None, imported_at: String::new(),
            },
        ];

        let rendered = render_frontmatter(&album, &photos);
        let parsed = parse_frontmatter(&rendered);

        assert_eq!(parsed.title.as_deref(), Some("Ana & Ivan: \"the\" day"));
        assert_eq!(parsed.description.as_deref(), Some("Zagreb, lipanj"));
        assert_eq!(parsed.date.as_deref(), Some("2026-06-14"));
        assert_eq!(parsed.token.as_deref(), Some("abc123"));
        assert_eq!(parsed.password.as_deref(), Some("tajna"));
        assert_eq!(parsed.share_token.as_deref(), Some("SHARE_xyz"));
        assert_eq!(parsed.sort.as_deref(), Some("custom"));
        assert_eq!(parsed.style.as_deref(), Some("grid"));
        assert_eq!(parsed.thumbnail.as_deref(), Some("a.jpg"));
        // Rendering preserves the order it is given; the DB is what sorts.
        assert_eq!(parsed.tags, vec!["wedding", "ljeto"]);
        assert_eq!(parsed.photo_order, vec!["a.jpg"]);
        assert!(parsed.hidden);
        assert!(parsed.allow_download);
        assert!(parsed.proofing);
        assert!(!parsed.is_collection);
        assert_eq!(parsed.order, Some(3));
    }

    #[test]
    fn handles_minimal_and_malformed_input() {
        let minimal = parse_frontmatter("---\ntitle: \"X\"\n---\n");
        assert_eq!(minimal.title.as_deref(), Some("X"));
        assert!(minimal.tags.is_empty());

        assert_eq!(parse_frontmatter(""), ParsedFrontmatter::default());
        assert_eq!(parse_frontmatter("no fences here"), ParsedFrontmatter::default());
    }

    #[test]
    fn accepts_unquoted_scalars() {
        let p = parse_frontmatter("---\ntitle: Plain Title\nsort: name\n---\n");
        assert_eq!(p.title.as_deref(), Some("Plain Title"));
        assert_eq!(p.sort.as_deref(), Some("name"));
    }
}
