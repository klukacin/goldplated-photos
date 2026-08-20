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

/// Fully processed file, ready for insertion.
struct Processed {
    candidate: Candidate,
    content_hash: String,
    metadata: Metadata,
    lqip: Option<String>,
    /// Dimensions after orientation is applied.
    width: Option<u32>,
    height: Option<u32>,
}

/// Import every supported file under `dir` (which must be inside the library).
///
/// `on_progress` is called from the worker threads; keep it cheap and
/// thread-safe.
pub fn import_dir(
    lib: &Library,
    dir: &Path,
    opts: &ImportOptions,
    on_progress: Option<&(dyn Fn(ImportProgress) + Sync)>,
) -> Result<ImportSummary> {
    let candidates = scan(lib, dir, opts.recursive)?;
    if candidates.is_empty() {
        return Ok(ImportSummary::default());
    }

    // What the catalog already knows, so unchanged files can be skipped
    // without touching the disk.
    let known = load_known(lib)?;

    let total = candidates.len();
    let counter = AtomicUsize::new(0);
    let thumb_root = lib.thumb_dir();

    let results: Vec<std::result::Result<Option<Processed>, (String, String)>> = candidates
        .into_par_iter()
        .map(|cand| {
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
                        return Ok(None);
                    }
                }
            }

            match process_one(&cand, &thumb_root, opts.generate_thumbnails) {
                Ok(p) => Ok(Some(p)),
                Err(e) => Err((cand.rel_path.clone(), e.to_string())),
            }
        })
        .collect();

    let mut summary = ImportSummary::default();
    let mut to_insert = Vec::new();
    for r in results {
        match r {
            Ok(Some(p)) => to_insert.push(p),
            Ok(None) => summary.skipped += 1,
            Err((path, msg)) => summary.failed.push((path, msg)),
        }
    }

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

/// Walk the tree and collect supported files.
fn scan(lib: &Library, dir: &Path, recursive: bool) -> Result<Vec<Candidate>> {
    let root = lib.root();
    let dir = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        root.join(dir)
    };
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
                // A file we can't decode is still worth cataloguing.
                tracing::warn!(path = %cand.rel_path, error = %e, "thumbnail generation failed");
            }
        }
    } else if cand.kind == PhotoKind::Photo {
        if let Ok(img) = image::open(&cand.abs_path) {
            let img = media::apply_orientation(img, metadata.orientation);
            width = Some(img.width());
            height = Some(img.height());
        }
    }

    Ok(Processed {
        candidate: cand.clone(),
        content_hash,
        metadata,
        lqip,
        width,
        height,
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

    #[test]
    fn imports_and_is_incremental() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_jpeg(&root.join("a.jpg"), 800, 600);
        write_jpeg(&root.join("sub/b.jpg"), 400, 400);

        let lib = Library::open(root).unwrap();
        let opts = ImportOptions::default();

        let s1 = import_dir(&lib, root, &opts, None).unwrap();
        assert_eq!(s1.imported, 2, "both files imported");
        assert_eq!(lib.photo_count().unwrap(), 2);

        // Second run: nothing changed on disk, so nothing is reprocessed.
        let s2 = import_dir(&lib, root, &opts, None).unwrap();
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
        import_dir(&lib, root, &ImportOptions::default(), None).unwrap();

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
        import_dir(&lib, root, &ImportOptions::default(), None).unwrap();

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
        import_dir(&lib, root, &ImportOptions::default(), None).unwrap();
        let before = lib.photo_by_rel_path("a.jpg").unwrap().unwrap();

        // Replace with different content under the same name.
        std::thread::sleep(std::time::Duration::from_millis(10));
        write_jpeg(&file, 200, 150);

        let s = import_dir(&lib, root, &ImportOptions::default(), None).unwrap();
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
        let s = import_dir(&lib, root, &ImportOptions::default(), None).unwrap();
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
        import_dir(&lib, root, &ImportOptions::default(), None).unwrap();
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
        import_dir(&lib, root, &ImportOptions::default(), None).unwrap();

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
