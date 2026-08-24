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
//!
//! # What publishing may and may not touch
//!
//! The destination is a folder other tools also write: the web admin panel puts
//! files there, and the server puts proofing submissions under `.meta/`. So a
//! publish adds and overwrites freely, but it only ever *removes* a file it has
//! a record of putting there itself ([`Library::published_files`]). Nothing
//! else in the folder is even looked at.
//!
//! Two more rules that the whole module bends around:
//!
//! - **What ships is the developed frame.** With no adjustments that is the
//!   camera's own file, copied byte for byte. With adjustments it is a render at
//!   [`crate::develop::DELIVERY_JPEG_QUALITY`], so moving one slider cannot
//!   quietly cost the client detail.
//! - **A filename is a URL.** Two photos wanting one published name are
//!   reported as a collision and one of them ships nothing — never renamed,
//!   because the photographer may already have sent that gallery out.
//!
//! One bad frame never fails an album. A missing original, an undecodable file,
//! a develop that blew up: each is named in the [`PublishResult`] and the other
//! four hundred photographs still reach the client.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::Serialize;

use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::model::{Album, Photo};

impl Library {
    /// Filenames this library last published into `album_path`, in the
    /// **default** publish target's tree.
    pub fn published_files(&self, album_path: &str) -> Result<BTreeSet<String>> {
        match self.default_target_id()? {
            Some(id) => self.published_files_for(id, album_path),
            None => Ok(BTreeSet::new()),
        }
    }

    /// Filenames this library last published into `album_path`, per target —
    /// two publish destinations keep separate books (schema v5), so pruning
    /// one tree cannot be driven by what was written into the other.
    pub fn published_files_for(&self, target_id: i64, album_path: &str) -> Result<BTreeSet<String>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT filename FROM published_files WHERE target_id = ?1 AND album_path = ?2",
            )?;
            let rows = stmt.query_map(params![target_id, album_path], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<BTreeSet<String>>>()?)
        })
    }

    /// Replace the record of what an album's published folder contains, in
    /// the default target's books.
    pub fn record_published_files(&self, album_path: &str, files: &BTreeSet<String>) -> Result<()> {
        let id = self.ensure_default_target()?;
        self.record_published_files_for(id, album_path, files)
    }

    /// Replace the record of what an album's published folder contains, per
    /// target.
    pub fn record_published_files_for(
        &self,
        target_id: i64,
        album_path: &str,
        files: &BTreeSet<String>,
    ) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute(
                "DELETE FROM published_files WHERE target_id = ?1 AND album_path = ?2",
                params![target_id, album_path],
            )?;
            let mut stmt = tx.prepare(
                "INSERT INTO published_files(target_id, album_path, filename) VALUES(?1, ?2, ?3)",
            )?;
            for f in files {
                stmt.execute(params![target_id, album_path, f])?;
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

/// What to select out of an album, and whether to move pixels at all.
///
/// The default is the safe one for a delivery: every album member except the
/// rejects, photos included. Note that these decide what is *published*, and
/// a photo they exclude is also a photo the next publish prunes off the site —
/// raising `min_rating` after a gallery has gone out withdraws frames the
/// client has already seen.
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
    /// The album this describes — carried so a batch publish can report each
    /// album's outcome separately instead of merging them into one total.
    pub album_path: String,
    /// Relative paths written, under the destination root. One entry per file
    /// that exists on disk afterwards — never the same path twice.
    pub written: Vec<String>,
    /// Photo files put in place, counted per destination. Two catalog photos
    /// racing for one name produce one copy, so this counts one.
    pub photos_copied: usize,
    /// Photos already in place with a matching byte count, so nothing was
    /// rewritten. They are still in [`written`](Self::written) and still on the
    /// site — "skipped" is about work avoided, not about a photo left out.
    pub photos_skipped: usize,
    /// Bytes the copies actually wrote — the *developed* frames, which on a
    /// delivered album are mostly crops. Deliberately not the catalog's
    /// `file_size`, which is the original's and was reporting a figure that had
    /// never been written anywhere.
    pub bytes_copied: u64,
    /// Catalogued photos whose original file is gone from disk. Reported, not
    /// fatal: one unplugged drive must not abort an album. Any copy already in
    /// the published tree is left alone — `prune_missing` is the deliberate way
    /// to drop photos that are really gone.
    pub missing: Vec<String>,
    /// Catalogued photos no decoder could read, so they were not shipped: on
    /// the site they would be broken images.
    pub unrenderable: Vec<String>,
    /// Files removed from the published folder because this album no longer
    /// publishes them. Only ever files this library put there itself.
    pub removed: Vec<String>,
    /// Photos in this album that wanted the same published filename. Only the
    /// first reached the site; the rest are named here and shipped nothing.
    pub collisions: Vec<PublishCollision>,
}

/// Two or more photos in one album competing for a single published filename.
#[derive(Debug, Clone, Serialize)]
pub struct PublishCollision {
    /// The contested path, relative to the destination root.
    pub dest: String,
    /// The album's photos that map to it, in album order. The first is the one
    /// that was published; every later one was left out.
    pub sources: Vec<String>,
}

/// The name a photograph takes in the published tree.
///
/// Almost always its own, but a HEIF frame is published as JPEG and so changes
/// extension. Two reasons, and both bite:
///
/// Chrome and Firefox cannot display HEIC — only Safari can — so a HEIC in the
/// gallery is a broken image for most of the people it exists for, and a client
/// who downloads one gets something their photo viewer refuses. And the moment
/// a frame carries an adjustment the published bytes *are* a JPEG, because that
/// is what the renderer emits; shipping those under a `.HEIC` name is a lie
/// about the file.
///
/// Converting either way keeps the name stable across a develop, which matters
/// because the published filename is the URL. A client who has the link should
/// not lose it because the photographer moved a slider.
///
/// All of this rides on `media::is_heif`, which is `false` in a build without
/// the `heif` feature — such a build cannot transcode, so it must not promise a
/// `.jpg` it has no way to produce. (It also cannot *catalogue* a HEIF, so the
/// case only arises on a catalog written by a heif-enabled build; the file is
/// then copied under its own name, as any other photo is.)
pub fn published_filename(photo: &Photo) -> String {
    if !crate::media::is_heif(std::path::Path::new(&photo.filename)) {
        return photo.filename.clone();
    }
    let stem = photo.filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(&photo.filename);
    format!("{stem}.jpg")
}

/// Publish one album into `dest_root` (the gallery's `src/content/albums`),
/// keeping the books under the **default** publish target.
pub fn publish_album(
    lib: &Library,
    album_path: &str,
    dest_root: &Path,
    opts: &PublishOptions,
) -> Result<PublishResult> {
    let target_id = lib.ensure_default_target()?;
    publish_album_for(lib, target_id, album_path, dest_root, opts)
}

/// [`publish_album`], with the publish records kept under one specific target.
pub fn publish_album_for(
    lib: &Library,
    target_id: i64,
    album_path: &str,
    dest_root: &Path,
    opts: &PublishOptions,
) -> Result<PublishResult> {
    let album = lib
        .album_by_path(album_path)?
        .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;

    let selected = select_photos(lib, &album, opts)?;
    let (photos, collisions) = split_filename_collisions(album_path, selected);
    let album_dir = dest_root.join(album_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&album_dir).map_err(|e| Error::io(&album_dir, e))?;

    let mut result = PublishResult {
        album_path: album_path.to_string(),
        collisions,
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
            // A file nothing could decode at import time would reach the site
            // as a broken image. It stays in the catalog — it is the
            // photographer's file — but the published gallery is meant to work.
            if photo.kind == crate::model::PhotoKind::Photo
                && photo.width.is_none()
                && photo.height.is_none()
            {
                result.unrenderable.push(photo.rel_path.clone());
                continue;
            }

            // What ships is the developed photo. With no adjustments this is the
            // original file itself — no copy, no render, nothing cached.
            let original = lib.resolve(&photo.rel_path)?;
            let src = match crate::develop::ensure_rendered(
                &original,
                &lib.thumb_dir(),
                photo,
                &lib.edits(photo.id)?,
            ) {
                Ok(src) => src,
                // Rendering means opening the original, so the two losses the
                // unedited path already survives — a drive unplugged, a file
                // gone unreadable — arrive here as an error instead the moment
                // a photo carries an adjustment. One bad frame in a developed
                // wedding must not stop the other four hundred from reaching
                // the client; the album says which one it was.
                Err(e) => {
                    if original.exists() {
                        tracing::warn!(photo = %photo.rel_path, error = %e, "develop failed");
                        result.unrenderable.push(photo.rel_path.clone());
                    } else {
                        result.missing.push(photo.rel_path.clone());
                    }
                    continue;
                }
            };
            let published = published_filename(photo);
            let dest = album_dir.join(&published);

            // A HEIF frame with no adjustments would otherwise be *copied*,
            // which under its new `.jpg` name would be HEIC bytes wearing the
            // wrong extension — the same lie in the other direction. Transcode
            // it instead, at the quality a developed frame is delivered at.
            let src = if published != photo.filename && src == original {
                let img = crate::media::load_oriented(&original, photo.orientation)?;
                let jpeg = crate::media::encode_jpeg(&img, crate::develop::DELIVERY_JPEG_QUALITY)?;
                let transcoded = lib.thumb_dir().join(format!("{}.jpg", photo.content_hash));
                crate::media::write_atomic(&transcoded, &jpeg)?;
                transcoded
            } else {
                src
            };

            // Skip only when the destination already holds exactly these
            // bytes. Size alone was the old test, with a comment claiming a
            // same-length change would be caught by "the sync layer's hash" —
            // it is not: the sync manifest hashes the *published* tree, so a
            // develop that happened to produce the same byte count left the
            // stale frame on the site and nothing anywhere disagreed. A size
            // mismatch still short-circuits straight to the copy, so the
            // common changed-file case pays no hashing; equal sizes cost one
            // blake3 of each side, which is the price of knowing rather than
            // guessing.
            if let (Ok(s), Ok(d)) = (std::fs::metadata(&src), std::fs::metadata(&dest)) {
                let unchanged = s.len() == d.len()
                    && matches!(
                        (crate::import::hash_file(&src), crate::import::hash_file(&dest)),
                        (Ok(a), Ok(b)) if a == b
                    );
                if unchanged {
                    result.photos_skipped += 1;
                    result.written.push(format!("{album_path}/{published}"));
                    continue;
                }
            }

            if !src.exists() {
                result.missing.push(photo.rel_path.clone());
                continue;
            }

            // Count what the copy actually wrote. The catalog's `file_size` is
            // the *original's*, and what ships is the developed frame — on a
            // delivered album, mostly a crop — so charging the original's bytes
            // reported a figure that was never written anywhere.
            result.bytes_copied += std::fs::copy(&src, &dest).map_err(|e| Error::io(&src, e))?;
            result.photos_copied += 1;
            result.written.push(format!("{album_path}/{published}"));
        }

        prune_published(lib, target_id, album_path, &album_dir, &photos, &mut result)?;
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
    target_id: i64,
    album_path: &str,
    album_dir: &Path,
    photos: &[Photo],
    result: &mut PublishResult,
) -> Result<()> {
    let current: BTreeSet<String> = photos.iter().map(published_filename).collect();

    for stale in lib.published_files_for(target_id, album_path)?.difference(&current) {
        // A photo whose original vanished keeps its published copy: that is a
        // broken drive, not a decision to unpublish. Matched on the filename
        // itself — as a suffix, `my-gone.jpg` going missing also spared an
        // unrelated `gone.jpg` that the album really had dropped.
        if result
            .missing
            .iter()
            .any(|m| m.rsplit('/').next() == Some(stale.as_str()))
        {
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
    lib.record_published_files_for(target_id, album_path, &recorded)
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

/// Split the selected photos into the ones that get published and the
/// filename fights that stopped the rest.
///
/// A published album is one flat folder, but a library is not: two cards at
/// one wedding both hand you a `DSC_0001.jpg`. Whichever comes second cannot
/// have the name, and the copy loop would simply overwrite the first — the
/// album then ships one frame while every count says two.
///
/// Renaming the loser would be the tidy fix and is deliberately not done: the
/// published filename *is* the URL, and a photographer who has already sent a
/// client their gallery cannot have this app quietly reshuffle it. So the
/// first photo in album order keeps the name, the others ship nothing, and the
/// result says exactly which frames those were.
fn split_filename_collisions(
    album_path: &str,
    selected: Vec<Photo>,
) -> (Vec<Photo>, Vec<PublishCollision>) {
    let mut kept: Vec<Photo> = Vec::with_capacity(selected.len());
    // Destination filename → index into `kept` of the photo holding it.
    let mut claimed: BTreeMap<String, usize> = BTreeMap::new();
    let mut losers: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for photo in selected {
        match claimed.get(&published_filename(&photo)) {
            Some(_) => losers
                .entry(published_filename(&photo))
                .or_default()
                .push(photo.rel_path),
            None => {
                claimed.insert(published_filename(&photo), kept.len());
                kept.push(photo);
            }
        }
    }

    let collisions = losers
        .into_iter()
        .map(|(filename, rest)| {
            let winner = kept[claimed[&filename]].rel_path.clone();
            PublishCollision {
                dest: format!("{album_path}/{filename}"),
                sources: std::iter::once(winner).chain(rest).collect(),
            }
        })
        .collect();

    (kept, collisions)
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
            out.push_str(&format!("  - {}\n", yaml_scalar(&published_filename(p))));
        }
    }

    out.push_str(&yaml_kv("style", &album.style));

    // Cover: explicit choice, else let the site fall back to the first photo.
    // `cover_filename` is the *library* name; what the tree holds — and what
    // `thumbnail:` must name — is the published one, which for a HEIC differs
    // (`x.heic` ships as `x.jpg`). Comparing the raw name against published
    // candidates meant a HEIC cover never emitted `thumbnail:` at all. Either
    // spelling is accepted, and the published one is what gets written.
    if let Some(cover) = album.cover_filename.as_deref().and_then(|c| {
        photos
            .iter()
            .find(|p| p.filename == c || published_filename(p) == c)
            .map(published_filename)
    }) {
        out.push_str(&yaml_kv("thumbnail", &cover));
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
///
/// Control characters are escaped rather than passed through or dropped. A
/// double-quoted YAML scalar may not hold one raw, and js-yaml — what Astro
/// parses these files with — stops at the first with "expected valid JSON
/// character". That is not one broken album: an unparseable content collection
/// fails the whole `npm run build`, so one byte here takes the site down. And
/// the app does not choose these strings — a title or a tag comes back from a
/// pulled `index.md`, a `photoOrder` entry is a filename off a card, and on
/// Unix a filename may hold any byte but `/` and NUL.
fn yaml_scalar(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
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

    /// A double-quoted YAML scalar may not carry a raw control character, and
    /// js-yaml — what Astro reads these files with — stops at the first one
    /// with "expected valid JSON character". That is not one broken album: a
    /// content collection that fails to parse fails the whole `npm run build`,
    /// so a single byte here takes the entire site down.
    ///
    /// The app does not choose these strings. A title, a description or a tag
    /// arrives from a pulled `index.md`, and a `photoOrder` entry is a filename
    /// off a card — on Unix a filename may hold any byte but `/` and NUL.
    #[test]
    fn a_control_character_cannot_reach_the_frontmatter_raw() {
        for (name, value) in [
            ("a NUL", "Ana\u{0}Ivan"),
            ("a bell", "Ana\u{7}Ivan"),
            ("an escape", "Ana\u{1b}[31mIvan"),
            ("a vertical tab", "Ana\u{b}Ivan"),
            ("a carriage return", "Ana\rIvan"),
            ("DEL", "Ana\u{7f}Ivan"),
        ] {
            let rendered = yaml_scalar(value);
            assert!(
                !rendered
                    .chars()
                    .any(|c| (c as u32) < 0x20 || c == '\u{7f}'),
                "{name} reached the frontmatter raw: {rendered:?}"
            );
        }
    }

    /// Escaping is only half a boundary; the other half is reading it back, or
    /// a pull → publish → pull loop rewrites the photographer's own text a
    /// little further every time round.
    #[test]
    fn every_escape_this_module_writes_is_one_it_can_read_back() {
        for value in [
            "Ana\u{0}Ivan",
            "Ana\u{7}Ivan",
            "Ana\rIvan",
            "Ana\tIvan",
            "Ana\nIvan",
            "Ana\u{7f}Ivan",
            r"C:\new\photos",
            "say \"hi\"",
        ] {
            let rendered = format!("---\n{}---\n", yaml_kv("title", value));
            assert_eq!(
                parse_frontmatter(&rendered).title.as_deref(),
                Some(value),
                "round trip of {value:?}"
            );
        }
    }

    #[test]
    fn publishes_index_and_photos() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 80, 60);
        write_jpeg(&src.path().join("b.jpg"), 80, 60);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
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
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
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

    /// The guard that spares a photo whose original vanished matched the stale
    /// name as a *suffix*, so any missing photo whose name ended the same way
    /// kept an unpublished file alive on the site.
    #[test]
    fn a_missing_photo_only_spares_itself_from_the_prune() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/gone.jpg"), 40, 40);
        write_jpeg(&src.path().join("a/my-gone.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let gone = lib.photo_by_rel_path("a/gone.jpg").unwrap().unwrap();
        let mine = lib.photo_by_rel_path("a/my-gone.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[gone.id, mine.id]).unwrap();
        publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();

        // One photo is unpublished on purpose; an unrelated one loses its
        // original to an unplugged drive.
        lib.remove_photos_from_album("a", &[gone.id]).unwrap();
        std::fs::remove_file(src.path().join("a/my-gone.jpg")).unwrap();

        let second = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(second.missing, vec!["a/my-gone.jpg".to_string()]);
        assert_eq!(second.removed, vec!["a/gone.jpg".to_string()]);
        assert!(!dest.path().join("a/gone.jpg").exists(), "unpublished, so gone from the site");
        assert!(dest.path().join("a/my-gone.jpg").exists(), "a broken drive is not a decision");
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
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let ids: Vec<i64> = lib.photos(&Default::default()).unwrap().iter().map(|p| p.id).collect();
        lib.add_photos_to_album("a", &ids).unwrap();

        std::fs::remove_file(src.path().join("a/gone.jpg")).unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(r.photos_copied, 1);
        assert_eq!(r.missing, vec!["a/gone.jpg".to_string()]);
        assert!(dest.path().join("a/here.jpg").exists());
    }

    /// A file no decoder could read is kept in the catalog but must not be
    /// shipped: on the site it is a broken image, and the photographer would
    /// only find out from a visitor.
    #[test]
    fn a_photo_without_a_preview_is_not_published() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/good.jpg"), 40, 40);
        std::fs::write(src.path().join("a/corrupt.jpg"), b"\xff\xd8\xff\xe0 not a jpeg").unwrap();

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let ids: Vec<i64> = lib.photos(&Default::default()).unwrap().iter().map(|p| p.id).collect();
        lib.add_photos_to_album("a", &ids).unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(r.photos_copied, 1);
        assert_eq!(r.unrenderable, vec!["a/corrupt.jpg".to_string()]);
        assert!(dest.path().join("a/good.jpg").exists());
        assert!(!dest.path().join("a/corrupt.jpg").exists());
    }

    /// Two cards at one wedding both start at `DSC_0001.jpg`. The published
    /// album is a flat folder, so only one of them can have the name — and the
    /// photographer has to be told which frame is not on the site, because the
    /// only other way to find out is a client asking where their photo went.
    #[test]
    fn same_named_photos_from_two_cards_are_reported_not_silently_merged() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("cardA/DSC_0001.jpg"), 40, 40);
        write_jpeg(&src.path().join("cardB/DSC_0001.jpg"), 60, 60);
        write_jpeg(&src.path().join("cardA/DSC_0002.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let first = lib.photo_by_rel_path("cardA/DSC_0001.jpg").unwrap().unwrap();
        let second = lib.photo_by_rel_path("cardB/DSC_0001.jpg").unwrap().unwrap();
        let other = lib.photo_by_rel_path("cardA/DSC_0002.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[first.id, second.id, other.id]).unwrap();
        lib.update_album("a", &AlbumUpdate { sort: Some("custom".into()), ..Default::default() })
            .unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();

        assert_eq!(
            r.collisions.len(),
            1,
            "the two DSC_0001.jpg frames want the same published name"
        );
        assert_eq!(r.collisions[0].dest, "a/DSC_0001.jpg");
        assert_eq!(
            r.collisions[0].sources,
            vec!["cardA/DSC_0001.jpg".to_string(), "cardB/DSC_0001.jpg".to_string()]
        );

        // Two files reached the site, not three: the count must not claim a
        // photo was published when its name was taken.
        assert_eq!(r.photos_copied, 2);
        let written: BTreeSet<&String> = r.written.iter().collect();
        assert_eq!(written.len(), r.written.len(), "no destination listed twice");
        assert!(r.written.contains(&"a/DSC_0001.jpg".to_string()));

        assert_eq!(
            lib.published_files("a").unwrap(),
            ["DSC_0001.jpg".to_string(), "DSC_0002.jpg".to_string()]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );

        // A photoOrder naming the same file twice is a list the gallery cannot
        // make sense of.
        let index = std::fs::read_to_string(dest.path().join("a/index.md")).unwrap();
        let order_block = index.split("photoOrder:\n").nth(1).unwrap();
        let names: Vec<&str> = order_block
            .lines()
            .take_while(|l| l.trim_start().starts_with("- "))
            .collect();
        assert_eq!(names.len(), 2, "photoOrder lists each published file once: {names:?}");
    }

    /// The same two losses the unedited path already survives — an original
    /// gone from disk, an original nothing can decode — reached the caller as
    /// an error the moment the photo carried an adjustment, because rendering
    /// it means opening it. So the wedding published fine right up until the
    /// photographer put a crop on the one frame whose drive had gone, and then
    /// stopped publishing at all, with an i/o error naming a path in `.gpp`.
    #[test]
    fn an_edited_photo_that_cannot_be_rendered_does_not_take_the_album_down() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        for name in ["good.jpg", "gone.jpg", "corrupt.jpg"] {
            write_jpeg(&src.path().join("a").join(name), 40, 40);
        }

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let ids: Vec<i64> = lib.photos(&Default::default()).unwrap().iter().map(|p| p.id).collect();
        lib.add_photos_to_album("a", &ids).unwrap();

        // Every frame is developed — this is a delivered album.
        for id in &ids {
            let mut stack = lib.edits(*id).unwrap();
            stack.set(crate::develop::EditOp::Exposure { ev: 0.2 });
            lib.set_edits(*id, &stack).unwrap();
        }

        // Then the card goes bad: one original vanishes, one is left unreadable.
        std::fs::remove_file(src.path().join("a/gone.jpg")).unwrap();
        std::fs::write(src.path().join("a/corrupt.jpg"), b"\xff\xd8\xff\xe0 not a jpeg").unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(r.photos_copied, 1, "the frames that survived still ship");
        assert!(dest.path().join("a/good.jpg").exists());
        assert_eq!(r.missing, vec!["a/gone.jpg".to_string()]);
        assert_eq!(r.unrenderable, vec!["a/corrupt.jpg".to_string()]);
    }

    /// A crop handle dragged flat against the edge of one frame.
    ///
    /// It rounded to a rectangle with no pixels in it, the JPEG encoder refused
    /// that, and the error came back out of `publish_album` — so the whole
    /// wedding stopped publishing over one photo's crop, with an "image error"
    /// naming no file.
    #[test]
    fn a_crop_with_no_area_does_not_take_the_album_down() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/one.jpg"), 40, 40);
        write_jpeg(&src.path().join("a/two.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let one = lib.photo_by_rel_path("a/one.jpg").unwrap().unwrap();
        let two = lib.photo_by_rel_path("a/two.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[one.id, two.id]).unwrap();

        let mut stack = lib.edits(one.id).unwrap();
        stack.set(crate::develop::EditOp::Crop { x: 1.0, y: 0.0, w: 0.3, h: 1.0 });
        lib.set_edits(one.id, &stack).unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(r.photos_copied, 2, "both frames reached the gallery");
        assert!(dest.path().join("a/two.jpg").exists());
    }

    /// `bytes_copied` is the number the photographer watches to know how much
    /// of a wedding is still going onto the disk, and what ships is the
    /// developed frame, not the original. Counting the original's size instead
    /// reported a figure that was never written anywhere — badly wrong the
    /// moment a crop is involved, which on a delivered album is most of them.
    #[test]
    fn bytes_copied_counts_the_file_that_was_written() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/one.jpg"), 400, 300);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let one = lib.photo_by_rel_path("a/one.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[one.id]).unwrap();

        let mut stack = lib.edits(one.id).unwrap();
        stack.set(crate::develop::EditOp::Crop { x: 0.0, y: 0.0, w: 0.25, h: 0.25 });
        lib.set_edits(one.id, &stack).unwrap();

        let r = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(r.photos_copied, 1);

        let on_disk = std::fs::metadata(dest.path().join("a/one.jpg")).unwrap().len();
        assert_ne!(
            on_disk, one.file_size as u64,
            "the crop has to change the byte count or this test proves nothing"
        );
        assert_eq!(r.bytes_copied, on_disk);
    }

    /// The skip that spares an unchanged photo used to test size alone, on the
    /// claim that "the sync layer's hash catches" a same-length change. It
    /// does not — the sync manifest hashes the *published* tree, so published
    /// bytes that differ from the source at the same byte count were simply
    /// never refreshed, and the stale frame stayed on the site.
    #[test]
    fn a_published_file_with_the_same_length_but_different_bytes_is_rewritten() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a/one.jpg"), 60, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
        lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        let one = lib.photo_by_rel_path("a/one.jpg").unwrap().unwrap();
        lib.add_photos_to_album("a", &[one.id]).unwrap();
        publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();

        // The published copy drifts — same length, different bytes. An edit
        // that renders to the same byte count looks exactly like this from
        // where the copy loop stands: src and dest agree on size and on
        // nothing else.
        let published = dest.path().join("a/one.jpg");
        let mut bytes = std::fs::read(&published).unwrap();
        let last = bytes.len() - 3;
        bytes[last] ^= 0xFF;
        std::fs::write(&published, &bytes).unwrap();

        let again = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(again.photos_copied, 1, "a same-length difference must be recopied");
        assert_eq!(again.photos_skipped, 0);
        assert_eq!(
            std::fs::read(&published).unwrap(),
            std::fs::read(src.path().join("a/one.jpg")).unwrap(),
            "the published tree still holds the stale bytes"
        );

        // A genuinely unchanged file keeps its fast path.
        let third = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
        assert_eq!(third.photos_copied, 0);
        assert_eq!(third.photos_skipped, 1);
    }

    #[test]
    fn respects_rating_filter_and_rejects() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("keep.jpg"), 40, 40);
        write_jpeg(&src.path().join("drop.jpg"), 40, 40);
        write_jpeg(&src.path().join("rejected.jpg"), 40, 40);

        let lib = Library::open(src.path()).unwrap();
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
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
        import_dir(&lib, src.path(), &ImportOptions::default(), None, None).unwrap();
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
///
/// Two things follow from that, and both matter when a pull turns one of these
/// into an [`AlbumUpdate`](crate::albums::AlbumUpdate):
///
/// - **This is a document a server wrote**, possibly by another photographer's
///   machine or another tool entirely. Every string in it is untrusted input,
///   not something this app chose.
/// - **`None` and `false` mean "the key was not there"**, which is not the same
///   as "the album does not have it". A key the parser does not recognise is
///   dropped, and a field left at its default here must not be written back as
///   a deliberate clear, or a pull quietly strips settings off the album.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedFrontmatter {
    pub title: Option<String>,
    pub description: Option<String>,
    pub date: Option<String>,
    /// The gallery's internal album id, and the one field that must survive a
    /// round trip unchanged: it is what the site's access cookie names this
    /// album by, so a pull that loses it logs out every client currently
    /// holding an unlock.
    pub token: Option<String>,
    /// Plain text, as the site stores it. A pull therefore carries the client's
    /// password back into this machine's catalog.
    pub password: Option<String>,
    /// The share-link secret. Same caveat as [`password`](Self::password), and
    /// worth more: it is the entire protection on a link-shared album.
    pub share_token: Option<String>,
    pub sort: Option<String>,
    pub style: Option<String>,
    /// The cover photo's *filename*, not an id — this side of the wire has no
    /// idea what the other machine's catalog calls it.
    pub thumbnail: Option<String>,
    /// Filenames in published order, meaningful only when `sort` is `custom`.
    ///
    /// May name only some of the album: the gallery puts what it does not name
    /// after what it does, which is why applying one has to renumber the rest
    /// rather than leave them where they were.
    pub photo_order: Vec<String>,
    pub tags: Vec<String>,
    /// The four flags are written only when true, so an absent key is a
    /// genuine `false` here rather than a gap — the one place in this struct
    /// where a default and an absence really are the same thing.
    pub is_collection: bool,
    pub hidden: bool,
    pub allow_download: bool,
    pub proofing: bool,
    /// The album's rank among its siblings — the gallery's `order`, which this
    /// crate stores as `sort_order`.
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

/// Strip surrounding quotes and undo their escaping.
///
/// Two quoting styles arrive here, from two different writers. This module's
/// own [`yaml_scalar`] double-quotes with backslash escapes. The web admin
/// writes these files with js-yaml, which prefers *single*-quoted scalars —
/// `'Ana: the day'`, `'it''s'` — whose one and only escape is the doubled
/// apostrophe. A pull reads both, so both have to unquote here, or a
/// single-quoted title comes back wearing its quotes (and `''` still doubled).
fn unquote(value: &str) -> String {
    let trimmed = value.trim();

    // Single-quoted (js-yaml's style): no backslash escapes at all, `''` → `'`.
    if let Some(inner) = trimmed
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    {
        return inner.replace("''", "'");
    }

    let Some(inner) = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return trimmed.to_string();
    };

    // One pass, not three `replace` calls. Escaping runs backslash-first, so
    // undoing it a pass at a time reads the second backslash of `\\n` as the
    // start of a newline escape: a description holding `C:\new` came back
    // with a real line break in it, and a pull wrote that into the catalog.
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            // `\xNN`, which is what a control character was written as. Only a
            // pair of hex digits is one: anything else is text that happened to
            // start `\x` and is handed back unchanged, the way it went out.
            Some('x') => match take_hex_pair(&mut chars) {
                Some(c) => out.push(c),
                None => out.push('x'),
            },
            Some(escaped) => out.push(escaped),
            None => out.push('\\'),
        }
    }
    out
}

/// Two hex digits from the front of `chars`, consumed only if both are there.
fn take_hex_pair(chars: &mut std::str::Chars<'_>) -> Option<char> {
    let mut peek = chars.clone();
    let hi = peek.next()?.to_digit(16)?;
    let lo = peek.next()?.to_digit(16)?;
    *chars = peek;
    char::from_u32(hi * 16 + lo)
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

    /// Escaping runs backslash-first, so undoing it one `replace` at a time
    /// reads the backslash of an escaped backslash as the start of the *next*
    /// escape: a Windows path in a description came back with a real newline
    /// in it, and a pull wrote that into the catalog.
    #[test]
    fn a_backslash_in_a_field_survives_the_round_trip() {
        for value in [r"C:\new\photos", r"a\nb", r"a\b", "say \"hi\"", "plain"] {
            let rendered = format!("---\n{}---\n", yaml_kv("title", value));
            assert_eq!(
                parse_frontmatter(&rendered).title.as_deref(),
                Some(value),
                "round trip of {value:?}"
            );
        }
    }

    /// The web admin writes these files with js-yaml, which single-quotes:
    /// `'Ana: the day'`, `'it''s'`. This parser only ever met its own
    /// double-quoted output, so a single-quoted title came back wearing its
    /// quotes — and a doubled apostrophe stayed doubled.
    #[test]
    fn accepts_js_yaml_single_quoted_scalars() {
        let p = parse_frontmatter(
            "---\ntitle: 'Ana: the day'\npassword: 'tajna lozinka'\n---\n",
        );
        assert_eq!(p.title.as_deref(), Some("Ana: the day"));
        assert_eq!(p.password.as_deref(), Some("tajna lozinka"));

        let doubled = parse_frontmatter("---\ntitle: 'it''s the day'\n---\n");
        assert_eq!(doubled.title.as_deref(), Some("it's the day"));

        // And the double-quoted style this module writes is unaffected.
        let ours = parse_frontmatter("---\ntitle: \"it's 'quoted'\"\n---\n");
        assert_eq!(ours.title.as_deref(), Some("it's 'quoted'"));
    }

    #[test]
    fn accepts_unquoted_scalars() {
        let p = parse_frontmatter("---\ntitle: Plain Title\nsort: name\n---\n");
        assert_eq!(p.title.as_deref(), Some("Plain Title"));
        assert_eq!(p.sort.as_deref(), Some("name"));
    }
}
