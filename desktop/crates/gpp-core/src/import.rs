//! The import pipeline: scan → hash → metadata → thumbnails → catalog.
//!
//! CPU work (hashing, EXIF, resizing) runs in parallel across all cores via
//! rayon; database writes are funnelled through a single transaction at the
//! end. That split is deliberate — SQLite hates concurrent writers, and batching
//! turns thousands of inserts into one commit.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use rusqlite::{params, OptionalExtension};
use walkdir::WalkDir;

use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::media::{self, Metadata};
use crate::model::{ImportProgress, ImportSummary, PhotoKind};

/// Options for one import run.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Recurse into subdirectories.
    pub recursive: bool,
    /// Generate thumbnails during import. Turning this off makes import fast
    /// and defers the work to first view.
    pub generate_thumbnails: bool,
    /// Re-read metadata for files already catalogued and unchanged.
    pub force: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            recursive: true,
            generate_thumbnails: true,
            force: false,
        }
    }
}

/// One scanned file, before any expensive work.
#[derive(Debug, Clone)]
struct Candidate {
    abs_path: PathBuf,
    rel_path: String,
    filename: String,
    kind: PhotoKind,
    file_size: i64,
    mtime_ms: i64,
}

/// What happened to one candidate.
enum Outcome {
    Done(Box<Processed>),
    /// Unchanged since the last import.
    Unchanged,
    /// Cancelled before this file was touched.
    Stopped,
}

/// Fully processed file, ready for insertion.
struct Processed {
    candidate: Candidate,
    content_hash: String,
    metadata: Metadata,
    lqip: Option<String>,
    /// Dimensions after orientation is applied.
    width: Option<u32>,
    height: Option<u32>,
    /// No decoder could read this file's pixels.
    undecodable: bool,
}

/// Import every supported file under `dir`.
///
/// A folder inside the library is catalogued where it lies. A folder outside
/// it — a camera card, a downloads folder — is copied in first, because the
/// catalog addresses photos by their path under the library root and cannot
/// point at files that live somewhere else. Nothing at the source is altered
/// or removed.
///
/// `on_progress` is called from the worker threads; keep it cheap and
/// thread-safe.
///
/// `should_stop` is polled before each file, in both the copy-in step and the
/// processing pass, and makes the run finish early with
/// [`ImportSummary::cancelled`] set. A 2000-frame card is tens of minutes of
/// decoding, so "stop" has to mean the next file, not the last one.
pub fn import_dir(
    lib: &Library,
    dir: &Path,
    opts: &ImportOptions,
    on_progress: Option<&(dyn Fn(ImportProgress) + Sync)>,
    should_stop: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<ImportSummary> {
    let stop = || should_stop.map(|f| f()).unwrap_or(false);

    let brought_in = bring_inside(lib, dir, opts.recursive, on_progress, should_stop)?;
    let (dir, copied) = match &brought_in {
        Some(c) => (c.dest.as_path(), Some(c)),
        None => (dir, None),
    };
    // Named only when something actually arrived: an outside folder with
    // nothing importable reserves a destination that is never created.
    let copied_into = copied.filter(|c| c.files > 0).map(|c| c.rel.clone());

    // A copy stopped part-way ends the run here rather than cataloguing what
    // arrived: that pass is the expensive half, and the user just asked for it
    // not to happen. Nothing is lost — the copies are on disk under the library
    // root, and the next import picks them up as ordinary files.
    if copied.map(|c| c.cancelled).unwrap_or(false) {
        return Ok(ImportSummary {
            copied_in: copied.map(|c| c.files).unwrap_or(0),
            copied_into: copied_into.clone(),
            cancelled: true,
            ..Default::default()
        });
    }

    let candidates = scan(lib, dir, opts.recursive)?;
    if candidates.is_empty() {
        return Ok(ImportSummary {
            copied_in: copied.map(|c| c.files).unwrap_or(0),
            copied_into: copied_into.clone(),
            cancelled: stop(),
            ..Default::default()
        });
    }

    // What the catalog already knows, so unchanged files can be skipped
    // without touching the disk.
    let known = load_known(lib)?;

    let total = candidates.len();
    let counter = AtomicUsize::new(0);
    let thumb_root = lib.thumb_dir();

    let results: Vec<std::result::Result<Outcome, (String, String)>> = candidates
        .into_par_iter()
        .map(|cand| {
            // Checked before the progress tick, so a cancelled run does not
            // report a bar racing to 100% over files it never opened.
            if stop() {
                return Ok(Outcome::Stopped);
            }

            let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(cb) = on_progress {
                cb(ImportProgress {
                    processed: done,
                    total,
                    current: cand.rel_path.clone(),
                });
            }

            // Unchanged since last import → nothing to do.
            if !opts.force {
                if let Some((size, mtime)) = known.get(&cand.rel_path) {
                    if *size == cand.file_size && *mtime == cand.mtime_ms {
                        return Ok(Outcome::Unchanged);
                    }
                }
            }

            match process_one(&cand, &thumb_root, opts.generate_thumbnails) {
                Ok(p) => Ok(Outcome::Done(Box::new(p))),
                Err(e) => Err((cand.rel_path.clone(), e.to_string())),
            }
        })
        .collect();

    let mut summary = ImportSummary {
        copied_in: copied.map(|c| c.files).unwrap_or(0),
        copied_into: copied_into.clone(),
        ..Default::default()
    };
    let mut to_insert = Vec::new();
    for r in results {
        match r {
            Ok(Outcome::Done(p)) => {
                if p.undecodable {
                    summary.undecodable.push(p.candidate.rel_path.clone());
                }
                to_insert.push(p);
            }
            Ok(Outcome::Unchanged) => summary.skipped += 1,
            Ok(Outcome::Stopped) => summary.cancelled = true,
            Err((path, msg)) => summary.failed.push((path, msg)),
        }
    }

    // A cancelled run still commits what it finished. The alternative — throw
    // the transaction away — would mean a photographer who stops a 2000-frame
    // import after twenty minutes keeps none of the 600 that already decoded,
    // and their thumbnails would sit on disk with no catalog row pointing at
    // them. Every file here was processed end to end before it got into this
    // list, so the prefix that commits is whole; the rest is simply absent, and
    // re-running the import picks it up.
    //
    // Single transaction for every write.
    let now = chrono::Utc::now().to_rfc3339();
    lib.with_tx(|tx| {
        let mut existing_hash = tx.prepare(
            "SELECT content_hash FROM photos WHERE rel_path = ?1",
        )?;
        let mut dup_check = tx.prepare(
            "SELECT COUNT(*) FROM photos WHERE content_hash = ?1 AND rel_path <> ?2",
        )?;
        let mut upsert = tx.prepare(
            "INSERT INTO photos(
                rel_path, filename, content_hash, file_size, mtime_ms, kind,
                width, height, orientation, captured_at, camera_make, camera_model,
                lens, iso, aperture, shutter, focal_length, blur_lqip, imported_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
             ON CONFLICT(rel_path) DO UPDATE SET
                content_hash = excluded.content_hash,
                file_size    = excluded.file_size,
                mtime_ms     = excluded.mtime_ms,
                width        = excluded.width,
                height       = excluded.height,
                orientation  = excluded.orientation,
                captured_at  = excluded.captured_at,
                camera_make  = excluded.camera_make,
                camera_model = excluded.camera_model,
                lens         = excluded.lens,
                iso          = excluded.iso,
                aperture     = excluded.aperture,
                shutter      = excluded.shutter,
                focal_length = excluded.focal_length,
                blur_lqip    = excluded.blur_lqip",
        )?;

        for p in &to_insert {
            let was_known: Option<String> = existing_hash
                .query_row(params![p.candidate.rel_path], |r| r.get(0))
                .optional()?;

            // Same bytes already catalogued under a different path.
            let dupes: i64 = dup_check.query_row(
                params![p.content_hash, p.candidate.rel_path],
                |r| r.get(0),
            )?;

            upsert.execute(params![
                p.candidate.rel_path,
                p.candidate.filename,
                p.content_hash,
                p.candidate.file_size,
                p.candidate.mtime_ms,
                p.candidate.kind.as_str(),
                p.width.map(|v| v as i64),
                p.height.map(|v| v as i64),
                p.metadata.orientation.map(|v| v as i64),
                p.metadata.captured_at,
                p.metadata.camera_make,
                p.metadata.camera_model,
                p.metadata.lens,
                p.metadata.iso,
                p.metadata.aperture,
                p.metadata.shutter,
                p.metadata.focal_length,
                p.lqip,
                now,
            ])?;

            if was_known.is_some() {
                summary.updated += 1;
            } else if dupes > 0 {
                summary.duplicates += 1;
                summary.imported += 1;
            } else {
                summary.imported += 1;
            }
        }
        Ok(())
    })?;

    Ok(summary)
}

/// Where an outside folder was copied to, and how much of it arrived.
struct BroughtIn {
    dest: PathBuf,
    /// The destination relative to the library root, for the report.
    rel: String,
    files: usize,
    /// The copy stopped early, so `files` is a prefix of the source folder.
    cancelled: bool,
}

/// Copy an outside folder into the library so it can be catalogued.
///
/// Returns `None` when `dir` is already inside the library, which is the
/// in-place case and needs no copying.
///
/// The destination keeps the source folder's name, and a name already taken
/// gains a numeric suffix rather than merging into it — two cards both called
/// `DCIM` are two shoots, not one. Subfolder structure is preserved. A file
/// that somehow already exists at the destination with identical bytes is left
/// alone, so a re-run after an interruption resumes instead of duplicating.
///
/// Copying a card is the slowest part of importing one, so `should_stop` is
/// honoured here too — between files, never during one, so no half-written
/// file is left behind.
fn bring_inside(
    lib: &Library,
    dir: &Path,
    recursive: bool,
    on_progress: Option<&(dyn Fn(ImportProgress) + Sync)>,
    should_stop: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<Option<BroughtIn>> {
    let root = lib.root();
    let source = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        root.join(dir)
    };
    // canonicalize so that symlinks and `..` cannot disguise an inside path as
    // an outside one, or the reverse.
    let source = source.canonicalize().unwrap_or(source);
    let root_real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if source.starts_with(&root_real) {
        return Ok(None);
    }
    if !source.is_dir() {
        return Err(Error::InvalidPath(source.display().to_string()));
    }

    let base = source
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("imported");
    let (dest, rel) = free_destination(&root_real, base);

    // Collect first so progress can report a total.
    let mut walker = WalkDir::new(&source).follow_links(false);
    if !recursive {
        walker = walker.max_depth(1);
    }
    let files: Vec<PathBuf> = walker
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_hidden(e.path()))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && media::classify(e.path()).is_some())
        .map(|e| e.path().to_path_buf())
        .collect();
    // Nothing this app can read. `None` means "already inside the library", and
    // saying that here sent the caller on to scan the *outside* folder, which
    // the catalog cannot address: picking a folder of documents answered
    // "invalid path" rather than "nothing to import".
    if files.is_empty() {
        return Ok(Some(BroughtIn {
            dest,
            rel,
            files: 0,
            cancelled: false,
        }));
    }

    let total = files.len();
    let mut copied = 0;
    let mut cancelled = false;
    for (i, file) in files.iter().enumerate() {
        if should_stop.map(|f| f()).unwrap_or(false) {
            cancelled = true;
            break;
        }
        let Ok(sub) = file.strip_prefix(&source) else {
            continue;
        };
        let target = dest.join(sub);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        if let Some(cb) = on_progress {
            cb(ImportProgress {
                processed: i + 1,
                total,
                current: format!("copying {}", sub.display()),
            });
        }
        if same_file(file, &target) {
            copied += 1;
            continue;
        }
        std::fs::copy(file, &target).map_err(|e| Error::io(file, e))?;
        copied += 1;
    }

    Ok(Some(BroughtIn {
        dest,
        rel,
        files: copied,
        cancelled,
    }))
}

/// `<root>/<name>`, or `<root>/<name>-2` and so on when that is taken.
fn free_destination(root: &Path, name: &str) -> (PathBuf, String) {
    let first = root.join(name);
    if !first.exists() {
        return (first, name.to_string());
    }
    for n in 2..1000 {
        let rel = format!("{name}-{n}");
        let candidate = root.join(&rel);
        if !candidate.exists() {
            return (candidate, rel);
        }
    }
    // Vanishingly unlikely; better than looping forever.
    let rel = format!("{name}-{}", std::process::id());
    (root.join(&rel), rel)
}

/// Same length and same bytes — cheap enough for the resume check, and the
/// length test rejects almost everything before any reading happens.
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (a.metadata(), b.metadata()) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Walk the tree and collect supported files.
fn scan(lib: &Library, dir: &Path, recursive: bool) -> Result<Vec<Candidate>> {
    let root = lib.root();
    let dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        root.join(dir)
    };
    // The root is canonical (`Library::open` sees to that), so the folder has
    // to be compared in the same spelling — a caller may name it through a
    // symlink, and `/tmp` on macOS is one. Without this, an in-place import of
    // a folder that is plainly inside the library failed as "invalid path".
    let dir = dir.canonicalize().unwrap_or(dir);
    if !dir.starts_with(root) {
        return Err(Error::InvalidPath(dir.display().to_string()));
    }

    let mut walker = WalkDir::new(&dir).follow_links(false);
    if !recursive {
        walker = walker.max_depth(1);
    }

    let mut out = Vec::new();
    // depth 0 is the scan root itself — never reject it, or a library living
    // under a dot-directory (`~/.photos`, a temp dir) would scan as empty.
    for entry in walker
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_hidden(e.path()))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue, // unreadable entries are reported by absence
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(kind) = media::classify(path) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };
        let Some(rel_path) = to_rel(root, path) else {
            continue;
        };

        out.push(Candidate {
            filename: path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string(),
            rel_path,
            abs_path: path.to_path_buf(),
            kind,
            file_size: meta.len() as i64,
            mtime_ms: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        });
    }
    Ok(out)
}

/// Skip dotfiles and our own derived-data directory.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

/// Library-relative, '/'-separated path.
fn to_rel(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        parts.push(c.as_os_str().to_str()?);
    }
    Some(parts.join("/"))
}

/// Everything expensive for one file, done off the DB thread.
fn process_one(cand: &Candidate, thumb_root: &Path, thumbnails: bool) -> Result<Processed> {
    let content_hash = hash_file(&cand.abs_path)?;
    let metadata = media::read_metadata(&cand.abs_path);

    let mut width = metadata.width;
    let mut height = metadata.height;
    let mut lqip = None;
    let mut undecodable = false;

    // Videos and RAW get catalogued on metadata alone in this phase.
    if cand.kind == PhotoKind::Photo && thumbnails {
        match media::generate_derived(
            &cand.abs_path,
            thumb_root,
            &content_hash,
            metadata.orientation,
        ) {
            Ok(d) => {
                width = Some(d.width);
                height = Some(d.height);
                lqip = d.lqip;
            }
            Err(e) => {
                // Still catalogued — a file on disk the catalog has forgotten
                // is worse than one it cannot preview — but flagged, so the
                // import can say so and the grid can show why.
                tracing::warn!(path = %cand.rel_path, error = %e, "thumbnail generation failed");
                undecodable = true;
            }
        }
    } else if cand.kind == PhotoKind::Photo && (width.is_none() || height.is_none()) {
        // The no-thumbnail path exists to be fast, so read the size from the
        // container rather than decoding twenty-four megapixels to learn two
        // numbers: the image crate parses only the header, and for HEIF the
        // primary item's `ispe` property answers without an HEVC decode
        // (`read_dimensions` falls back to the full decode only when the
        // container cannot say for certain). Either way the numbers are the
        // pre-EXIF-orientation ones, so the swap below stores what the
        // viewer will see — the same convention the thumbnail path stores.
        match media::read_dimensions(&cand.abs_path) {
            Ok((w, h)) => {
                let (w, h) = media::swap_for_orientation(w, h, metadata.orientation);
                width = Some(w);
                height = Some(h);
            }
            Err(_) => undecodable = true,
        }
    }

    Ok(Processed {
        candidate: cand.clone(),
        content_hash,
        metadata,
        lqip,
        width,
        height,
        undecodable,
    })
}

/// blake3 of the file contents, streamed so large RAWs don't blow memory.
pub fn hash_file(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// rel_path → (size, mtime) for everything already catalogued.
fn load_known(lib: &Library) -> Result<std::collections::HashMap<String, (i64, i64)>> {
    lib.with_conn(|c| {
        let mut stmt = c.prepare("SELECT rel_path, file_size, mtime_ms FROM photos")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?)))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PhotoFilter;

    /// Write a real JPEG so decode/EXIF paths are exercised, not mocked.
    fn write_jpeg(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let img = image::DynamicImage::new_rgb8(w, h);
        img.save_with_format(path, image::ImageFormat::Jpeg).unwrap();
    }

    /// The obvious gesture — point Import at a camera card — used to fail
    /// with "invalid path", because the catalog can only address files under
    /// the library root. The folder is copied in instead.
    #[test]
    fn a_folder_from_outside_the_library_is_copied_in() {
        let lib_dir = tempfile::tempdir().unwrap();
        let card = tempfile::tempdir().unwrap();
        let lib = Library::open(lib_dir.path()).unwrap();

        write_jpeg(&card.path().join("DCIM/a.jpg"), 40, 30);
        write_jpeg(&card.path().join("DCIM/sub/b.jpg"), 40, 30);

        let summary = import_dir(&lib, &card.path().join("DCIM"), &ImportOptions::default(), None, None)
            .unwrap();

        assert_eq!(summary.copied_in, 2);
        assert_eq!(summary.copied_into.as_deref(), Some("DCIM"));
        assert_eq!(summary.imported, 2);

        // The copies live under the library, keeping their subfolder.
        assert!(lib_dir.path().join("DCIM/a.jpg").exists());
        assert!(lib_dir.path().join("DCIM/sub/b.jpg").exists());
        // And the card is untouched.
        assert!(card.path().join("DCIM/a.jpg").exists());

        let photos = lib.photos(&PhotoFilter::default()).unwrap();
        let mut paths: Vec<_> = photos.iter().map(|p| p.rel_path.clone()).collect();
        paths.sort();
        assert_eq!(paths, vec!["DCIM/a.jpg", "DCIM/sub/b.jpg"]);
    }

    /// Pointing Import at an outside folder holding nothing it can read must
    /// say "nothing to import", not "invalid path". The catalog genuinely
    /// cannot address a folder outside the library — but the person who picked
    /// it never asked it to, and the copy-in step is what spares them that.
    #[test]
    fn an_outside_folder_with_nothing_importable_is_not_an_error() {
        let lib_dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("notes.txt"), "hello").unwrap();

        let lib = Library::open(lib_dir.path()).unwrap();
        let summary =
            import_dir(&lib, elsewhere.path(), &ImportOptions::default(), None, None).unwrap();

        assert_eq!(summary.imported, 0);
        assert_eq!(summary.copied_in, 0);
        assert_eq!(summary.copied_into, None, "nothing was copied anywhere");
        assert!(!summary.cancelled);
    }

    /// A library whose path runs through a symlink — `/var` and `/tmp` are
    /// symlinks on macOS, so "my library on the external drive" often is one.
    /// The copy-in step canonicalized the root and put every file under the
    /// canonical spelling, but the scan validated and stripped against the raw
    /// stored root: the whole card was copied in and then nothing was
    /// catalogued, the run dying with InvalidPath. The root is canonicalized
    /// once, at open, so every later comparison uses one spelling.
    #[cfg(unix)]
    #[test]
    fn importing_through_a_symlinked_library_root_still_catalogues() {
        let real = tempfile::tempdir().unwrap();
        let holder = tempfile::tempdir().unwrap();
        let link = holder.path().join("gallery");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();

        let card = tempfile::tempdir().unwrap();
        write_jpeg(&card.path().join("DCIM/a.jpg"), 40, 30);

        let lib = Library::open(&link).unwrap();
        let summary =
            import_dir(&lib, &card.path().join("DCIM"), &ImportOptions::default(), None, None)
                .unwrap();
        assert_eq!(summary.copied_in, 1);
        assert_eq!(summary.imported, 1, "the copy arrived but was never catalogued");
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);

        // In-place import through the symlinked spelling works too.
        let in_place =
            import_dir(&lib, &link.join("DCIM"), &ImportOptions::default(), None, None).unwrap();
        assert_eq!(in_place.skipped, 1, "same file, already catalogued");

        let photos = lib.photos(&PhotoFilter::default()).unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].rel_path, "DCIM/a.jpg");
    }

    /// A second card of the same name is a second shoot, not an overwrite.
    #[test]
    fn a_second_card_with_the_same_name_lands_beside_the_first() {
        let lib_dir = tempfile::tempdir().unwrap();
        let lib = Library::open(lib_dir.path()).unwrap();

        for (card, w) in [(tempfile::tempdir().unwrap(), 40), (tempfile::tempdir().unwrap(), 60)] {
            write_jpeg(&card.path().join("DCIM/only.jpg"), w, 30);
            let summary =
                import_dir(&lib, &card.path().join("DCIM"), &ImportOptions::default(), None, None)
                    .unwrap();
            assert_eq!(summary.copied_in, 1);
        }

        assert!(lib_dir.path().join("DCIM/only.jpg").exists());
        assert!(lib_dir.path().join("DCIM-2/only.jpg").exists());
    }

    /// Re-running an interrupted copy must not duplicate what already arrived.
    #[test]
    fn re_importing_the_same_card_does_not_duplicate_it() {
        let lib_dir = tempfile::tempdir().unwrap();
        let card = tempfile::tempdir().unwrap();
        let lib = Library::open(lib_dir.path()).unwrap();
        write_jpeg(&card.path().join("DCIM/a.jpg"), 40, 30);

        import_dir(&lib, &card.path().join("DCIM"), &ImportOptions::default(), None, None).unwrap();
        // Same source, second run: it lands in DCIM-2 as a distinct folder, but
        // the catalog must not grow a phantom copy of a file that is byte for
        // byte what it already holds.
        let second =
            import_dir(&lib, &card.path().join("DCIM"), &ImportOptions::default(), None, None).unwrap();
        assert_eq!(second.duplicates, 1, "same bytes should register as a duplicate");
    }

    /// Run `f` with rayon pinned to one worker.
    ///
    /// Cancellation is inherently racy across threads — with eight workers a
    /// six-file import can start every file before the flag is seen. One worker
    /// makes "stop after two" mean exactly two, so the test asserts the
    /// behaviour instead of a coin flip.
    fn single_threaded<T: Send>(f: impl FnOnce() -> T + Send) -> T {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(f)
    }

    /// The point of cancelling: what already landed stays landed, what was
    /// never reached is simply not there, and the summary admits which it is.
    #[test]
    fn a_cancelled_import_keeps_exactly_what_it_processed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let names: Vec<String> = (0..8).map(|i| format!("f{i}.jpg")).collect();
        for name in &names {
            write_jpeg(&root.join(name), 120, 90);
        }

        let lib = Library::open(root).unwrap();
        // Progress fires only for files the run actually opened, so the log is
        // the ground truth for what should have been imported.
        let seen = std::sync::Mutex::new(Vec::new());
        let summary = single_threaded(|| {
            import_dir(
                &lib,
                root,
                &ImportOptions::default(),
                Some(&|p| seen.lock().unwrap().push(p.current)),
                Some(&|| seen.lock().unwrap().len() >= 3),
            )
            .unwrap()
        });

        let processed = seen.into_inner().unwrap();
        assert!(summary.cancelled, "the summary must not read as a full run");
        assert_eq!(processed.len(), 3, "it stopped at the next file, not the last");
        assert_eq!(summary.imported, 3);

        for name in &names {
            let row = lib.photo_by_rel_path(name).unwrap();
            if processed.contains(name) {
                assert!(row.is_some(), "{name} was processed but is not in the catalog");
            } else {
                assert!(row.is_none(), "{name} was never processed but was catalogued");
            }
        }
        assert_eq!(lib.photo_count().unwrap(), 3);
    }

    /// Cancelling changes nothing about the next run: the files that were
    /// missed are ordinary uncatalogued files, and a second import takes them.
    #[test]
    fn re_running_after_a_cancel_finishes_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..5 {
            write_jpeg(&root.join(format!("f{i}.jpg")), 120, 90);
        }

        let lib = Library::open(root).unwrap();
        let seen = std::sync::Mutex::new(0usize);
        single_threaded(|| {
            import_dir(
                &lib,
                root,
                &ImportOptions::default(),
                Some(&|_| *seen.lock().unwrap() += 1),
                Some(&|| *seen.lock().unwrap() >= 2),
            )
            .unwrap()
        });
        assert_eq!(lib.photo_count().unwrap(), 2);

        let second = import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();
        assert!(!second.cancelled);
        assert_eq!(second.imported, 3, "only the three that were missed");
        assert_eq!(second.skipped, 2, "the first two are unchanged");
        assert_eq!(lib.photo_count().unwrap(), 5);
    }

    /// Copying a card is the slow half for an outside folder, so the stop
    /// signal has to reach it — and stop between files, leaving no torn copy.
    #[test]
    fn cancelling_during_the_copy_in_stops_the_whole_import() {
        let lib_dir = tempfile::tempdir().unwrap();
        let card = tempfile::tempdir().unwrap();
        let lib = Library::open(lib_dir.path()).unwrap();
        for i in 0..4 {
            write_jpeg(&card.path().join(format!("DCIM/f{i}.jpg")), 120, 90);
        }

        let ticks = std::sync::Mutex::new(0usize);
        let summary = import_dir(
            &lib,
            &card.path().join("DCIM"),
            &ImportOptions::default(),
            Some(&|_| *ticks.lock().unwrap() += 1),
            Some(&|| *ticks.lock().unwrap() >= 1),
        )
        .unwrap();

        assert!(summary.cancelled);
        assert_eq!(summary.copied_in, 1, "one file arrived before the stop");
        // Cataloguing never ran, but the copy that did land is a whole file the
        // next import can pick up.
        assert_eq!(summary.imported, 0);
        assert_eq!(lib.photo_count().unwrap(), 0);
        let landed = lib_dir.path().join("DCIM");
        assert_eq!(std::fs::read_dir(&landed).unwrap().count(), 1);
        // And the card itself is untouched, cancelled or not.
        assert_eq!(std::fs::read_dir(card.path().join("DCIM")).unwrap().count(), 4);
    }

    #[test]
    fn imports_and_is_incremental() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("a.jpg"), 800, 600);
        write_jpeg(&root.join("sub/b.jpg"), 400, 400);

        let lib = Library::open(root).unwrap();
        let opts = ImportOptions::default();

        let s1 = import_dir(&lib, root, &opts, None, None).unwrap();
        assert_eq!(s1.imported, 2, "both files imported");
        assert_eq!(lib.photo_count().unwrap(), 2);

        // Second run: nothing changed on disk, so nothing is reprocessed.
        let s2 = import_dir(&lib, root, &opts, None, None).unwrap();
        assert_eq!(s2.imported, 0);
        assert_eq!(s2.skipped, 2);
        assert_eq!(lib.photo_count().unwrap(), 2, "no duplicate rows");
    }

    #[test]
    fn records_dimensions_and_lqip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("a.jpg"), 800, 600);

        let lib = Library::open(root).unwrap();
        import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();

        let photo = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();
        assert_eq!(photo.width, Some(800));
        assert_eq!(photo.height, Some(600));
        assert!(photo.blur_lqip.unwrap().starts_with("data:image/jpeg;base64,"));
        assert_eq!(photo.kind, PhotoKind::Photo);
        assert_eq!(photo.rating, 0);
    }

    #[test]
    fn writes_thumbnails_for_every_size() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("a.jpg"), 3000, 2000);

        let lib = Library::open(root).unwrap();
        import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();

        let photo = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();
        for (name, _) in media::THUMB_SIZES {
            let p = media::thumb_path(&lib.thumb_dir(), &photo.content_hash, name);
            assert!(p.exists(), "missing thumbnail {name}");
        }
    }

    #[test]
    fn detects_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("a.jpg");
        write_jpeg(&file, 100, 100);

        let lib = Library::open(root).unwrap();
        import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();
        let before = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();

        // Replace with different content under the same name.
        std::thread::sleep(std::time::Duration::from_millis(10));
        write_jpeg(&file, 200, 150);

        let s = import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();
        assert_eq!(s.updated, 1);
        let after = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();
        assert_ne!(before.content_hash, after.content_hash);
        assert_eq!(after.width, Some(200));
        assert_eq!(lib.photo_count().unwrap(), 1);
    }

    #[test]
    fn skips_unsupported_and_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("good.jpg"), 50, 50);
        std::fs::write(root.join("notes.txt"), "hello").unwrap();
        write_jpeg(&root.join(".hidden/secret.jpg"), 50, 50);

        let lib = Library::open(root).unwrap();
        let s = import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();
        assert_eq!(s.imported, 1);
        assert_eq!(lib.photo_count().unwrap(), 1);
    }

    #[test]
    fn prune_removes_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("a.jpg"), 50, 50);
        write_jpeg(&root.join("b.jpg"), 50, 50);

        let lib = Library::open(root).unwrap();
        import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();
        std::fs::remove_file(root.join("b.jpg")).unwrap();

        assert_eq!(lib.prune_missing().unwrap(), 1);
        assert_eq!(lib.photo_count().unwrap(), 1);
    }

    #[test]
    fn hash_is_content_based() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"same").unwrap();
        std::fs::write(&b, b"same").unwrap();
        assert_eq!(hash_file(&a).unwrap(), hash_file(&b).unwrap());
        std::fs::write(&b, b"different").unwrap();
        assert_ne!(hash_file(&a).unwrap(), hash_file(&b).unwrap());
    }

    #[test]
    fn filters_by_rating_after_import() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("a.jpg"), 50, 50);
        write_jpeg(&root.join("b.jpg"), 50, 50);

        let lib = Library::open(root).unwrap();
        import_dir(&lib, root, &ImportOptions::default(), None, None).unwrap();

        let a = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();
        lib.set_rating(a.id, 5).unwrap();

        let picks = lib
            .photos(&PhotoFilter {
                min_rating: Some(4),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].filename, "a.jpg");
    }
}
