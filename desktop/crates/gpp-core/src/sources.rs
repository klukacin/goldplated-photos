//! Sources — the roots a catalogued photograph may live under.
//!
//! For five schema versions a library was one folder: `photos.rel_path` was
//! relative to the library root, and importing anything from anywhere else had
//! to copy it in first. That is the right default — a library that owns its
//! files is one folder to back up — but it made one thing impossible that a
//! photographer with a decade of Lightroom actually needs: cataloguing work
//! *where it already is*, on the drives it already fills, without moving a
//! terabyte first.
//!
//! A **source** is such a root. The library root is source #1, the *primary*;
//! any number of others may be registered, and a photo row names the one its
//! `rel_path` is relative to. Files under a non-primary source are
//! **referenced**: read, hashed, thumbnailed, developed, published — never
//! moved, never copied, never written to.
//!
//! Three rules the rest of the core leans on:
//!
//! - **`.gpp/` is always on the primary.** The catalog, the thumbnails and the
//!   render cache belong to the library, not to whichever drive a photograph
//!   came off; an external source that is unplugged must not take the library's
//!   own index with it. It follows that only the primary receives copy-in
//!   imports and pulls: a referenced source is a place photographs *are*, never
//!   a place this app puts things.
//! - **Sources never overlap.** One may not sit inside another, in either
//!   direction. Two roots that contain the same file give that file two
//!   `(source, rel_path)` identities, and then a prune, a publish and a sync
//!   each disagree about how many photographs the library holds.
//! - **Offline is not missing.** A source is *online* when its path is a
//!   readable directory right now. Offline, the grid still works — thumbnails
//!   are content-addressed and live on the primary — while anything that must
//!   open the original fails with [`Error::SourceOffline`], naming the source
//!   so the answer ("plug the drive in") is on screen. Above all,
//!   [`Library::prune_missing`] leaves those rows alone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::catalog::{root_display_name, Library};
use crate::error::{Error, Result};

/// How many catalogued files [`Library::relocate_source`] re-hashes before it
/// believes a folder is the source it claims to be.
///
/// A handful: enough that a wrong-but-plausible folder (last year's backup, a
/// half-finished copy) is caught, cheap enough that pointing the app at a
/// re-mounted drive is instant. A source holding fewer photos than this has all
/// of them checked.
const RELOCATE_SAMPLE: usize = 5;

/// What kind of place a source is. Stored as the lowercase string in
/// `sources.kind`, which the schema constrains, so these spellings live in
/// catalogs on disk and are written out rather than derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// The library root — exactly one, and the only source this app writes to.
    Primary,
    /// A folder on a disk that is always attached.
    Internal,
    /// A removable drive: expected to come and go.
    External,
    /// A network share or mounted volume.
    Network,
}

impl SourceKind {
    /// The string stored in the `sources.kind` column.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Primary => "primary",
            SourceKind::Internal => "internal",
            SourceKind::External => "external",
            SourceKind::Network => "network",
        }
    }

    /// Parse from the stored string; anything unrecognised reads as external,
    /// which is the cautious answer — it is the kind that may be offline.
    pub fn parse(s: &str) -> Self {
        match s {
            "primary" => SourceKind::Primary,
            "internal" => SourceKind::Internal,
            "network" => SourceKind::Network,
            _ => SourceKind::External,
        }
    }
}

/// One source as a UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub id: i64,
    /// Display name. Defaults to the folder's own name — the word the
    /// photographer already uses for that drive.
    pub name: String,
    /// Absolute path of the root. For the primary this is the library root as
    /// this session opened it, not whatever spelling the catalog was carrying.
    pub path: String,
    pub kind: SourceKind,
    /// Best-effort volume identifier, filled where the platform offers one
    /// cheaply (on unix, the device id). Advisory only: nothing resolves a path
    /// through it.
    pub volume_hint: Option<String>,
    pub is_primary: bool,
    /// Whether the root is a readable directory **right now** — probed at the
    /// moment this listing was built, never cached, because that is the whole
    /// question an unplugged drive asks.
    pub online: bool,
    /// Catalog rows on this source.
    pub photo_count: i64,
}

/// One source row, as the core passes it around internally.
pub(crate) struct SourceRow {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) kind: SourceKind,
    pub(crate) volume_hint: Option<String>,
    pub(crate) is_primary: bool,
}

impl SourceRow {
    pub(crate) fn is_online(&self) -> bool {
        self.path.is_dir()
    }
}

impl Library {
    // ------------------------------------------------------------- reading

    /// Every registered source, lowest id first — the primary always leads.
    ///
    /// `online` is probed here, per call: the answer changes when a drive is
    /// plugged in, and a cached one would be wrong exactly when it mattered.
    pub fn sources(&self) -> Result<Vec<SourceInfo>> {
        let counts = self.photo_counts_by_source()?;
        Ok(self
            .source_rows()?
            .into_iter()
            .map(|row| SourceInfo {
                online: row.is_online(),
                photo_count: counts.get(&row.id).copied().unwrap_or(0),
                id: row.id,
                name: row.name,
                path: row.path.display().to_string(),
                kind: row.kind,
                volume_hint: row.volume_hint,
                is_primary: row.is_primary,
            })
            .collect())
    }

    /// The primary source's id — the library root's row.
    pub fn primary_source_id(&self) -> Result<i64> {
        self.with_conn(|c| {
            c.query_row("SELECT id FROM sources WHERE is_primary = 1", [], |r| r.get(0))
                .optional()?
                .ok_or_else(|| Error::other("this library has no primary source row"))
        })
    }

    /// One source row, or a message naming the id that is not there.
    pub(crate) fn source_row(&self, id: i64) -> Result<SourceRow> {
        self.source_rows()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::other(format!("no source with id {id}")))
    }

    /// Every source row, primary first, with the primary's path taken from the
    /// open library rather than from the catalog.
    pub(crate) fn source_rows(&self) -> Result<Vec<SourceRow>> {
        let root = self.root().to_path_buf();
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, path, kind, is_primary, volume_hint FROM sources ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                let is_primary: i64 = r.get(4)?;
                Ok(SourceRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    path: PathBuf::from(r.get::<_, String>(2)?),
                    kind: SourceKind::parse(&r.get::<_, String>(3)?),
                    is_primary: is_primary != 0,
                    volume_hint: r.get(5)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                let mut row = row?;
                // The primary *is* the open library, whatever spelling the
                // catalog was carrying when it was last written. A drive that
                // mounts somewhere new must not make every photograph in the
                // library unreachable.
                if row.is_primary {
                    row.path = root.clone();
                }
                out.push(row);
            }
            Ok(out)
        })
    }

    /// id → root path, for the loops that resolve many photos at once.
    pub(crate) fn source_roots(&self) -> Result<HashMap<i64, PathBuf>> {
        Ok(self
            .source_rows()?
            .into_iter()
            .map(|s| (s.id, s.path))
            .collect())
    }

    /// The root of one source, whether it is online or not.
    pub(crate) fn source_root(&self, id: i64) -> Result<PathBuf> {
        Ok(self.source_row(id)?.path)
    }

    /// The registered source an absolute path lies inside, if any — the deepest
    /// one, though sources may not overlap, so there is at most one.
    ///
    /// Offline sources count: a folder under an unplugged drive's root still
    /// belongs to that source, and answering "no source" would import it by
    /// copying it into the library, which is the opposite of what was asked.
    pub(crate) fn source_containing(&self, abs: &Path) -> Result<Option<SourceRow>> {
        let abs = abs.canonicalize().unwrap_or_else(|_| abs.to_path_buf());
        Ok(self.source_rows()?.into_iter().find(|s| {
            let root = s.path.canonicalize().unwrap_or_else(|_| s.path.clone());
            abs.starts_with(&root)
        }))
    }

    fn photo_counts_by_source(&self) -> Result<HashMap<i64, i64>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT source_id, COUNT(*) FROM photos GROUP BY source_id")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut out = HashMap::new();
            for row in rows {
                let (k, v) = row?;
                out.insert(k, v);
            }
            Ok(out)
        })
    }

    // ------------------------------------------------------------- writing

    /// Register a folder as a source, cataloguing nothing: this only says
    /// "photographs may live here too".
    ///
    /// Refused when the folder is not there, when another source already
    /// occupies it, or when it overlaps one in either direction — including the
    /// library root, since a folder inside the library is already reachable and
    /// a folder *containing* the library would swallow `.gpp` itself.
    pub fn add_source(
        &self,
        path: &Path,
        name: Option<&str>,
        kind: Option<SourceKind>,
    ) -> Result<i64> {
        if matches!(kind, Some(SourceKind::Primary)) {
            return Err(Error::other(
                "the primary source is the library root — it is not something to add",
            ));
        }
        let canonical = path.canonicalize().map_err(|e| Error::io(path, e))?;
        if !canonical.is_dir() {
            return Err(Error::other(format!(
                "{} is not a folder — a source is a root photographs live under",
                canonical.display()
            )));
        }

        for existing in self.source_rows()? {
            let other = existing
                .path
                .canonicalize()
                .unwrap_or_else(|_| existing.path.clone());
            if other == canonical {
                return Err(Error::other(format!(
                    "{} is already registered as the source '{}'",
                    canonical.display(),
                    existing.name
                )));
            }
            if canonical.starts_with(&other) {
                return Err(Error::other(format!(
                    "{} is inside the source '{}' ({}) — its photographs are already \
                     reachable from there, and two roots over one file give it two identities",
                    canonical.display(),
                    existing.name,
                    other.display()
                )));
            }
            if other.starts_with(&canonical) {
                return Err(Error::other(format!(
                    "{} contains the source '{}' ({}) — a source may not hold another",
                    canonical.display(),
                    existing.name,
                    other.display()
                )));
            }
        }

        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| root_display_name(&canonical));
        let kind = kind.unwrap_or(SourceKind::External);
        let hint = volume_hint(&canonical);

        self.with_conn(|c| {
            c.execute(
                "INSERT INTO sources(name, path, kind, volume_hint, is_primary, added_at) \
                 VALUES(?1, ?2, ?3, ?4, 0, ?5)",
                params![
                    name,
                    canonical.display().to_string(),
                    kind.as_str(),
                    hint,
                    chrono::Utc::now().to_rfc3339()
                ],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    /// Forget a source. **Never touches a file.**
    ///
    /// `drop_photos` has to be answered, not defaulted: with `false` a source
    /// that still holds catalog rows is refused, naming how many, because
    /// removing it would otherwise silently discard every rating, flag,
    /// membership and develop stack attached to those photographs. With `true`
    /// those rows go — and the files they described stay exactly where they
    /// are, ready to be catalogued again by re-adding the source.
    pub fn remove_source(&self, id: i64, drop_photos: bool) -> Result<usize> {
        let row = self.source_row(id)?;
        if row.is_primary {
            return Err(Error::other(
                "the primary source is the library root and cannot be removed — \
                 it is where the catalog and every thumbnail live",
            ));
        }
        let photos: i64 = self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM photos WHERE source_id = ?1",
                params![id],
                |r| r.get(0),
            )?)
        })?;
        if photos > 0 && !drop_photos {
            return Err(Error::other(format!(
                "the source '{}' still holds {photos} catalogued photo(s) — removing it \
                 drops their ratings, flags, album memberships and adjustments. Pass \
                 drop_photos to say so; the files themselves are never touched",
                row.name
            )));
        }

        let dropped = self.with_conn(|c| {
            let n = if drop_photos {
                c.execute("DELETE FROM photos WHERE source_id = ?1", params![id])?
            } else {
                0
            };
            c.execute("DELETE FROM sources WHERE id = ?1", params![id])?;
            Ok(n)
        })?;
        Ok(dropped)
    }

    /// Point a source at a new folder — a drive that mounted somewhere else, a
    /// share that moved.
    ///
    /// **Validated by sampling, not by trust.** A handful of the source's own
    /// catalogued files are re-hashed at the new path and must match the
    /// `content_hash` on record. Pointing a source at last year's backup, or at
    /// a copy that is still transferring, would otherwise re-attach a thousand
    /// rows to the wrong negatives — and the render keys, the published
    /// filenames and the sync baselines would all follow it quietly. A mismatch
    /// names the file and changes nothing.
    pub fn relocate_source(&self, id: i64, new_path: &Path) -> Result<()> {
        let row = self.source_row(id)?;
        if row.is_primary {
            return Err(Error::other(
                "the primary source is the library root — open the library at its new \
                 location instead of relocating it",
            ));
        }
        let canonical = new_path.canonicalize().map_err(|e| Error::io(new_path, e))?;
        if !canonical.is_dir() {
            return Err(Error::other(format!(
                "{} is not a folder",
                canonical.display()
            )));
        }
        for existing in self.source_rows()? {
            if existing.id == id {
                continue;
            }
            let other = existing
                .path
                .canonicalize()
                .unwrap_or_else(|_| existing.path.clone());
            if canonical.starts_with(&other) || other.starts_with(&canonical) {
                return Err(Error::other(format!(
                    "{} overlaps the source '{}' ({}) — sources may not contain one another",
                    canonical.display(),
                    existing.name,
                    other.display()
                )));
            }
        }

        // The sample: the source's own files, oldest rows first so the choice
        // is deterministic and a re-run reports the same file.
        let sample: Vec<(String, String)> = self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT rel_path, content_hash FROM photos WHERE source_id = ?1 \
                 ORDER BY id LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![id, RELOCATE_SAMPLE as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })?;

        for (rel, expected) in &sample {
            let candidate = crate::catalog::resolve_under(&canonical, rel)?;
            if !candidate.is_file() {
                return Err(Error::SourceMismatch {
                    message: format!(
                        "{} does not look like the source '{}': {rel} is not there",
                        canonical.display(),
                        row.name
                    ),
                    file: rel.clone(),
                });
            }
            let actual = crate::import::hash_file(&candidate)?;
            if &actual != expected {
                return Err(Error::SourceMismatch {
                    message: format!(
                        "{} does not look like the source '{}': {rel} is a different \
                         file there",
                        canonical.display(),
                        row.name
                    ),
                    file: rel.clone(),
                });
            }
        }

        let hint = volume_hint(&canonical);
        self.with_conn(|c| {
            c.execute(
                "UPDATE sources SET path = ?1, volume_hint = ?2 WHERE id = ?3",
                params![canonical.display().to_string(), hint, id],
            )?;
            Ok(())
        })
    }
}

/// A cheap, best-effort identifier for the volume a folder sits on.
///
/// On unix the device id out of `stat`, which distinguishes one mounted drive
/// from another and costs a syscall. Nowhere else: a portable answer would mean
/// either a platform crate or shelling out to a mount tool, and the portability
/// contract forbids the second while the licence policy narrows the first. It
/// is advisory in any case — nothing resolves a path through it — so `None` is
/// a perfectly good answer.
fn volume_hint(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        path.metadata().ok().map(|m| format!("dev:{}", m.dev()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_jpeg(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgb8(w, h)
            .save_with_format(path, image::ImageFormat::Jpeg)
            .unwrap();
    }

    /// A fresh library has exactly one source: itself.
    #[test]
    fn a_new_library_is_its_own_primary_source() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();

        let sources = lib.sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].is_primary);
        assert_eq!(sources[0].kind, SourceKind::Primary);
        assert_eq!(sources[0].path, lib.root().display().to_string());
        assert!(sources[0].online, "the library's own folder is there");
        assert_eq!(sources[0].photo_count, 0);
        assert_eq!(lib.primary_source_id().unwrap(), sources[0].id);
    }

    /// Sources may not nest, in either direction, and none may be added twice.
    /// Two roots over one file give that file two identities, and then a prune,
    /// a publish and a sync each disagree about what the library holds.
    #[test]
    fn overlapping_sources_are_refused_by_name() {
        let lib_dir = tempfile::tempdir().unwrap();
        let outer = tempfile::tempdir().unwrap();
        let inner = outer.path().join("inside");
        std::fs::create_dir_all(&inner).unwrap();
        let lib = Library::open(lib_dir.path()).unwrap();

        // Inside the library root.
        let within = lib_dir.path().join("shoot");
        std::fs::create_dir_all(&within).unwrap();
        let err = lib.add_source(&within, None, None).unwrap_err().to_string();
        assert!(err.contains("inside the source"), "{err}");

        // Containing the library root.
        let holder = lib_dir.path().parent().unwrap().to_path_buf();
        assert!(lib.add_source(&holder, None, None).is_err());

        let id = lib.add_source(outer.path(), Some("Archive"), None).unwrap();
        // The same folder twice.
        let err = lib.add_source(outer.path(), None, None).unwrap_err().to_string();
        assert!(err.contains("already registered"), "{err}");
        // A folder inside it.
        let err = lib.add_source(&inner, None, None).unwrap_err().to_string();
        assert!(err.contains("Archive"), "the refusal must name the source: {err}");
        // And a folder that would contain it.
        let err = lib
            .add_source(outer.path().parent().unwrap(), None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("contains the source"), "{err}");

        assert_eq!(lib.sources().unwrap().len(), 2);
        assert_eq!(lib.source_row(id).unwrap().name, "Archive");
    }

    /// A name defaults to the folder's own, and a kind to external — the one
    /// that is allowed to disappear.
    #[test]
    fn a_source_takes_its_folders_name_by_default() {
        let lib_dir = tempfile::tempdir().unwrap();
        let drive = tempfile::tempdir().unwrap();
        let named = drive.path().join("Wedding Archive");
        std::fs::create_dir_all(&named).unwrap();

        let lib = Library::open(lib_dir.path()).unwrap();
        let id = lib.add_source(&named, None, None).unwrap();
        let info = lib.sources().unwrap().into_iter().find(|s| s.id == id).unwrap();
        assert_eq!(info.name, "Wedding Archive");
        assert_eq!(info.kind, SourceKind::External);
        assert!(!info.is_primary);
        #[cfg(unix)]
        assert!(info.volume_hint.is_some(), "unix offers a device id cheaply");
    }

    /// Removing a source with photos on it is refused unless the caller says
    /// in so many words that the rows may go — and even then no file is
    /// touched.
    #[test]
    fn removing_a_source_that_still_holds_photos_needs_an_answer() {
        let lib_dir = tempfile::tempdir().unwrap();
        let drive = tempfile::tempdir().unwrap();
        write_jpeg(&drive.path().join("shoot/one.jpg"), 40, 30);

        let lib = Library::open(lib_dir.path()).unwrap();
        let id = lib.add_source(drive.path(), Some("Card"), None).unwrap();
        crate::import::import_dir(
            &lib,
            drive.path(),
            &crate::import::ImportOptions::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(lib.photo_count().unwrap(), 1);

        let err = lib.remove_source(id, false).unwrap_err().to_string();
        assert!(err.contains("Card") && err.contains('1'), "{err}");
        assert_eq!(lib.sources().unwrap().len(), 2, "nothing was removed");

        assert_eq!(lib.remove_source(id, true).unwrap(), 1);
        assert_eq!(lib.photo_count().unwrap(), 0);
        assert_eq!(lib.sources().unwrap().len(), 1);
        assert!(
            drive.path().join("shoot/one.jpg").exists(),
            "forgetting a source must never delete a photograph"
        );
    }

    /// The primary is the library itself: it cannot be removed or relocated.
    #[test]
    fn the_primary_source_is_not_removable() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let primary = lib.primary_source_id().unwrap();

        assert!(lib.remove_source(primary, true).is_err());
        assert!(lib.relocate_source(primary, elsewhere.path()).is_err());
        assert!(lib.add_source(dir.path(), None, Some(SourceKind::Primary)).is_err());
        assert_eq!(lib.sources().unwrap().len(), 1);
    }

    /// Relocating samples the catalogue: a folder that does not hold these
    /// photographs is refused, and the refusal names the file that gave it
    /// away.
    #[test]
    fn relocating_to_the_wrong_folder_is_refused_and_names_the_file() {
        let lib_dir = tempfile::tempdir().unwrap();
        let drive = tempfile::tempdir().unwrap();
        let decoy = tempfile::tempdir().unwrap();
        write_jpeg(&drive.path().join("shoot/one.jpg"), 40, 30);
        // The decoy has the same layout and a *different* photograph in it.
        write_jpeg(&decoy.path().join("shoot/one.jpg"), 80, 60);

        let lib = Library::open(lib_dir.path()).unwrap();
        let id = lib.add_source(drive.path(), Some("Card"), None).unwrap();
        crate::import::import_dir(
            &lib,
            drive.path(),
            &crate::import::ImportOptions::default(),
            None,
            None,
        )
        .unwrap();

        let err = lib.relocate_source(id, decoy.path()).unwrap_err();
        assert!(
            matches!(&err, Error::SourceMismatch { file, .. } if file == "shoot/one.jpg"),
            "the refusal must name the file: {err}"
        );
        assert_eq!(
            lib.source_row(id).unwrap().path.canonicalize().unwrap(),
            drive.path().canonicalize().unwrap(),
            "a refused relocation must change nothing"
        );

        // An empty folder is refused too — the file is simply not there.
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(
            lib.relocate_source(id, empty.path()),
            Err(Error::SourceMismatch { .. })
        ));
    }

    /// The drive really did move: same photographs, new mount point.
    #[test]
    fn relocating_to_the_real_folder_reattaches_every_photo() {
        let lib_dir = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        write_jpeg(&first.path().join("shoot/one.jpg"), 40, 30);
        write_jpeg(&first.path().join("shoot/two.jpg"), 42, 30);

        let lib = Library::open(lib_dir.path()).unwrap();
        let id = lib.add_source(first.path(), Some("Card"), None).unwrap();
        crate::import::import_dir(
            &lib,
            first.path(),
            &crate::import::ImportOptions::default(),
            None,
            None,
        )
        .unwrap();

        // The same bytes, remounted elsewhere.
        std::fs::create_dir_all(second.path().join("shoot")).unwrap();
        for name in ["one.jpg", "two.jpg"] {
            std::fs::copy(
                first.path().join("shoot").join(name),
                second.path().join("shoot").join(name),
            )
            .unwrap();
        }

        lib.relocate_source(id, second.path()).unwrap();
        let photo = lib
            .photo_by_source_rel_path(id, "shoot/one.jpg")
            .unwrap()
            .unwrap();
        assert_eq!(
            lib.photo_path(&photo).unwrap(),
            second.path().canonicalize().unwrap().join("shoot/one.jpg")
        );
        let info = lib.sources().unwrap().into_iter().find(|s| s.id == id).unwrap();
        assert!(info.online);
    }

    /// A source with no photos yet has nothing to sample, and relocating it is
    /// a plain rename of where it points.
    #[test]
    fn an_empty_source_relocates_without_a_sample() {
        let lib_dir = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let lib = Library::open(lib_dir.path()).unwrap();
        let id = lib.add_source(first.path(), None, None).unwrap();
        lib.relocate_source(id, second.path()).unwrap();
        assert_eq!(
            lib.source_row(id).unwrap().path,
            second.path().canonicalize().unwrap()
        );
    }

    /// A relative path names a folder just as well as an absolute one, and a
    /// path that is not there at all is an ordinary I/O failure naming it.
    #[test]
    fn adding_a_folder_that_is_not_there_fails_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let ghost = dir.path().parent().unwrap().join("no-such-drive-12345");
        let err = lib.add_source(&ghost, None, None).unwrap_err().to_string();
        assert!(err.contains("no-such-drive-12345"), "{err}");
    }
}
