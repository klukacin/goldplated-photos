//! Lightroom Classic catalog (.lrcat) import.
//!
//! A `.lrcat` is a SQLite database. This module reads the parts of it a
//! photographer's decade of work actually lives in — folders and files,
//! ratings, picks, colour labels, collections and keywords — and brings them
//! into a gpp library: photos copied in (or catalogued in place when they
//! already live under the library root), collections mapped to albums, and
//! keywords to photo tags.
//!
//! # Safety around the original
//!
//! Lightroom holds WAL locks on a catalog while it runs, and the original is
//! another program's database either way. So the `.lrcat` (and its `-wal` /
//! `-shm` companions, when present) is **copied** under the library's `.gpp`
//! directory first and only the copy is ever opened. Nothing in this module
//! writes to the original catalog or to any file Lightroom manages.
//!
//! # Defensive reading
//!
//! The lrcat schema is Adobe's, undocumented, and moves between LR versions.
//! Every query here is written against the schema as understood from LR
//! Classic catalogs; a **required** table that is missing fails with an error
//! naming the table (and the likely version mismatch), while **optional** data
//! — collections, keywords — that is missing or oddly shaped is skipped with a
//! note in the scan report rather than failing the run.
//!
//! The test fixture in this file *generates* a minimal catalog with exactly
//! these tables, which validates the reader against our understanding of the
//! schema. Validation against a real Lightroom-written catalog is pending —
//! treat surprises from the field as expected, and extend the fixture when one
//! arrives.
//!
//! # Idempotency and provenance
//!
//! Every imported photo and collection is linked in `lr_links` /
//! `lr_album_links` under a stable catalog id, so re-running the import syncs
//! instead of duplicating: new LR photos and collections are added, a
//! collection renamed in LR renames its album here (unless the album was also
//! renamed locally — that is a conflict, reported and left alone), and
//! LR-side deletions are reported, never propagated. Ratings, flags and
//! colour labels follow the catalog-wide rule: a value someone set by hand in
//! this library is never overwritten.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::albums::{normalize_path, AlbumUpdate, NewAlbum};
use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::import;
use crate::model::{Flag, ImportProgress, PhotoKind};

/// Where the copied photos land by default: `<root>/lr/<root-folder>/...`.
const DEFAULT_DEST_SUBDIR: &str = "lr";

// ---------------------------------------------------------------- lr reading

/// One `AgLibraryRootFolder` row.
struct LrRoot {
    id: i64,
    absolute_path: String,
    name: String,
}

/// One image with a resolvable file: `Adobe_images` joined through
/// `AgLibraryFile` and `AgLibraryFolder` to its root folder.
struct LrImage {
    /// `Adobe_images.id_local` — the id `lr_links` remembers.
    id: i64,
    root_id: i64,
    /// `pathFromRoot` + filename, '/'-separated, no leading slash.
    rel_from_root: String,
    rating: u8,
    flag: Flag,
    color_label: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum LrCollectionKind {
    /// `com.adobe.ag.library.collection` — an ordinary collection.
    Collection,
    /// `com.adobe.ag.library.group` — a collection set (a folder of them).
    Set,
    /// `com.adobe.ag.library.smart_collection` — rule-driven; imported as a
    /// snapshot of its current members.
    Smart,
}

impl LrCollectionKind {
    fn as_str(self) -> &'static str {
        match self {
            LrCollectionKind::Collection => "collection",
            LrCollectionKind::Set => "set",
            LrCollectionKind::Smart => "smart",
        }
    }
}

struct LrCollection {
    id: i64,
    name: String,
    parent: Option<i64>,
    kind: LrCollectionKind,
    /// `(image id, positionInCollection)` — position `None` when LR holds no
    /// custom order for that member.
    members: Vec<(i64, Option<f64>)>,
}

/// Everything read out of one catalog, before any decision is made.
struct LrData {
    catalog_id: String,
    roots: Vec<LrRoot>,
    /// Keyed by `Adobe_images.id_local`, iterated in id order for determinism.
    images: BTreeMap<i64, LrImage>,
    collections: Vec<LrCollection>,
    /// Flattened keyword names per image — the keyword's own name and each
    /// ancestor's, lowercased and trimmed, LR's nameless root skipped.
    keywords_per_image: HashMap<i64, Vec<String>>,
    keyword_count: usize,
    images_without_files: usize,
    notes: Vec<String>,
}

/// Copy the catalog (and WAL companions) under `.gpp` and open the copy.
fn open_catalog_copy(lib: &Library, lrcat: &Path) -> Result<(Connection, PathBuf)> {
    if !lrcat.is_file() {
        return Err(Error::other(format!(
            "no Lightroom catalog at {}",
            lrcat.display()
        )));
    }
    let staging = lib.gpp_dir().join("lr-import");
    std::fs::create_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;

    let key = blake3::hash(lrcat.display().to_string().as_bytes()).to_hex();
    let copy = staging.join(format!("{}.lrcat", &key.as_str()[..16]));
    std::fs::copy(lrcat, &copy).map_err(|e| Error::io(lrcat, e))?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{}", lrcat.display(), suffix));
        let side_copy = PathBuf::from(format!("{}{}", copy.display(), suffix));
        if side.is_file() {
            std::fs::copy(&side, &side_copy).map_err(|e| Error::io(&side, e))?;
        } else {
            // A stale companion from a previous copy would corrupt this one.
            let _ = std::fs::remove_file(&side_copy);
        }
    }

    // Read-only where possible; recovering a copied WAL can need write access
    // to the copy, which is ours to give.
    let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .or_else(|_| Connection::open(&copy))?;
    Ok((conn, copy))
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1 COLLATE NOCASE",
        params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Fail a required-table read with an error a person can act on.
fn required<T>(table: &str, r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(|e| {
        Error::other(format!(
            "could not read the Lightroom catalog's {table} table ({e}) — \
             the catalog is likely from a Lightroom version whose schema this \
             importer does not understand"
        ))
    })
}

/// Read the whole catalog into memory. The interesting failure modes are all
/// here; everything after works on plain Rust data.
fn read_lr_catalog(lib: &Library, lrcat: &Path) -> Result<LrData> {
    let (conn, copy) = open_catalog_copy(lib, lrcat)?;
    let data = read_lr_connection(&conn, lrcat);
    drop(conn);
    // Best-effort cleanup of the staging copy; a leftover is only disk.
    let _ = std::fs::remove_file(&copy);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", copy.display(), suffix));
    }
    data
}

fn read_lr_connection(conn: &Connection, lrcat: &Path) -> Result<LrData> {
    for table in [
        "AgLibraryRootFolder",
        "AgLibraryFolder",
        "AgLibraryFile",
        "Adobe_images",
    ] {
        if !table_exists(conn, table)? {
            return Err(Error::other(format!(
                "{} has no {table} table — not a Lightroom Classic catalog this \
                 importer understands (or one from a Lightroom version whose \
                 schema has changed)",
                lrcat.display()
            )));
        }
    }

    let mut notes = Vec::new();

    // A stable id for this catalog: an id stored inside it when one exists,
    // else the blake3 of its canonicalized path. The path fallback means a
    // catalog moved on disk reads as a new source — the price of catalogs
    // that carry no id of their own.
    let catalog_id = read_catalog_uuid(conn).unwrap_or_else(|| {
        let canon = lrcat
            .canonicalize()
            .unwrap_or_else(|_| lrcat.to_path_buf());
        format!("path-{}", blake3::hash(canon.display().to_string().as_bytes()).to_hex())
    });

    // Roots.
    let roots: Vec<LrRoot> = required("AgLibraryRootFolder", (|| {
        let mut stmt =
            conn.prepare("SELECT id_local, absolutePath, name FROM AgLibraryRootFolder")?;
        let rows = stmt.query_map([], |r| {
            Ok(LrRoot {
                id: r.get(0)?,
                absolute_path: r.get(1)?,
                name: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })())?;

    // Folders: id → (root id, pathFromRoot).
    let folders: HashMap<i64, (i64, String)> = required("AgLibraryFolder", (|| {
        let mut stmt =
            conn.prepare("SELECT id_local, rootFolder, pathFromRoot FROM AgLibraryFolder")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?)))
        })?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
    })())?;

    // Files: id → (folder id, filename).
    let files: HashMap<i64, (i64, String)> = required("AgLibraryFile", (|| {
        let mut stmt = conn.prepare("SELECT id_local, folder, idx_filename FROM AgLibraryFile")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?)))
        })?;
        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
    })())?;

    // Images, joined by hand so an image whose chain does not resolve is a
    // counted absence rather than a dropped row.
    let mut images = BTreeMap::new();
    let mut images_without_files = 0usize;
    required("Adobe_images", (|| {
        let mut stmt = conn.prepare(
            "SELECT id_local, rootFile, rating, colorLabels, pick FROM Adobe_images",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<f64>>(4)?,
            ))
        })?;
        for row in rows {
            let (id, root_file, rating, labels, pick) = row?;
            let resolved = root_file
                .and_then(|f| files.get(&f))
                .and_then(|(folder, filename)| {
                    folders
                        .get(folder)
                        .map(|(root, path_from_root)| (*root, path_from_root, filename))
                });
            let Some((root_id, path_from_root, filename)) = resolved else {
                images_without_files += 1;
                continue;
            };
            let mut rel = path_from_root.trim_matches('/').to_string();
            if !rel.is_empty() {
                rel.push('/');
            }
            rel.push_str(filename);

            images.insert(
                id,
                LrImage {
                    id,
                    root_id,
                    rel_from_root: rel,
                    rating: rating.map(|v| v.clamp(0.0, 5.0) as u8).unwrap_or(0),
                    flag: match pick {
                        Some(v) if v > 0.5 => Flag::Pick,
                        Some(v) if v < -0.5 => Flag::Reject,
                        _ => Flag::None,
                    },
                    color_label: labels.filter(|l| !l.trim().is_empty()),
                },
            );
        }
        Ok(())
    })())?;

    // Collections — optional.
    let mut collections: Vec<LrCollection> = Vec::new();
    if table_exists(conn, "AgLibraryCollection")? {
        let read = (|| -> rusqlite::Result<Vec<std::result::Result<LrCollection, String>>> {
            let mut stmt = conn
                .prepare("SELECT id_local, name, parent, creationId FROM AgLibraryCollection")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out
                .into_iter()
                .map(|(id, name, parent, creation)| {
                    let kind = match creation.as_deref() {
                        Some("com.adobe.ag.library.collection") => LrCollectionKind::Collection,
                        Some("com.adobe.ag.library.group") => LrCollectionKind::Set,
                        Some("com.adobe.ag.library.smart_collection") => LrCollectionKind::Smart,
                        other => {
                            return Err(format!(
                                "collection \"{name}\" has an unrecognised creationId \
                                 ({}) — skipped",
                                other.unwrap_or("none")
                            ))
                        }
                    };
                    Ok(LrCollection {
                        id,
                        name,
                        parent,
                        kind,
                        members: Vec::new(),
                    })
                })
                .collect::<Vec<std::result::Result<LrCollection, String>>>())
        })();
        match read {
            Ok(rows) => {
                for row in rows {
                    match row {
                        Ok(c) => collections.push(c),
                        Err(note) => notes.push(note),
                    }
                }
            }
            Err(e) => notes.push(format!("collections could not be read ({e}) — skipped")),
        }

        // Memberships. The column is `keyword`-style in our understanding but
        // real catalogs may spell it differently; both are tried.
        if table_exists(conn, "AgLibraryCollectionimage")? {
            let by_id: HashMap<i64, usize> = collections
                .iter()
                .enumerate()
                .map(|(i, c)| (c.id, i))
                .collect();
            let read = read_collection_members(conn);
            match read {
                Ok(members) => {
                    for (collection, image, position) in members {
                        if let Some(&i) = by_id.get(&collection) {
                            collections[i].members.push((image, position));
                        }
                    }
                }
                Err(e) => notes.push(format!(
                    "collection memberships could not be read ({e}) — collections \
                     will import empty"
                )),
            }
        } else {
            notes.push("no AgLibraryCollectionimage table — collections import empty".into());
        }
    } else {
        notes.push("no AgLibraryCollection table — no collections to import".into());
    }

    // Keywords — optional.
    let mut keywords_per_image: HashMap<i64, Vec<String>> = HashMap::new();
    let mut keyword_count = 0usize;
    if table_exists(conn, "AgLibraryKeyword")? && table_exists(conn, "AgLibraryKeywordImage")? {
        let read = (|| -> rusqlite::Result<()> {
            let mut stmt =
                conn.prepare("SELECT id_local, name, parent FROM AgLibraryKeyword")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?;
            let nodes: HashMap<i64, (Option<String>, Option<i64>)> = rows
                .map(|r| r.map(|(id, name, parent)| (id, (name, parent))))
                .collect::<rusqlite::Result<_>>()?;
            keyword_count = nodes
                .values()
                .filter(|(name, _)| name.as_deref().map(|n| !n.trim().is_empty()).unwrap_or(false))
                .count();

            // A keyword tags an image with its own name and each ancestor's —
            // that is what "flattened" means here: the hierarchy collapses to a
            // set of plain tag names, LR's nameless root excluded.
            let flatten = |mut at: i64| -> Vec<String> {
                let mut out = Vec::new();
                let mut seen = HashSet::new();
                while seen.insert(at) {
                    let Some((name, parent)) = nodes.get(&at) else { break };
                    if let Some(name) = name {
                        let name = name.trim().to_lowercase();
                        if !name.is_empty() {
                            out.push(name);
                        }
                    }
                    match parent {
                        Some(p) => at = *p,
                        None => break,
                    }
                }
                out
            };

            let links = read_keyword_links(conn)?;
            for (keyword, image) in links {
                let names = flatten(keyword);
                if names.is_empty() {
                    continue;
                }
                let entry = keywords_per_image.entry(image).or_default();
                for name in names {
                    if !entry.contains(&name) {
                        entry.push(name);
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = read {
            notes.push(format!("keywords could not be read ({e}) — skipped"));
        }
    } else {
        notes.push("no keyword tables — no keywords to import".into());
    }

    Ok(LrData {
        catalog_id,
        roots,
        images,
        collections,
        keywords_per_image,
        keyword_count,
        images_without_files,
        notes,
    })
}

/// `(collection, image, positionInCollection)` rows, whatever the column case.
fn read_collection_members(conn: &Connection) -> rusqlite::Result<Vec<(i64, i64, Option<f64>)>> {
    let mut stmt = conn.prepare(
        "SELECT collection, image, positionInCollection FROM AgLibraryCollectionimage",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// `(keyword, image)` links. Our fixture spells the column `keyword`; a real
/// catalog may spell it `tag`, so that is tried second.
fn read_keyword_links(conn: &Connection) -> rusqlite::Result<Vec<(i64, i64)>> {
    let first = conn
        .prepare("SELECT keyword, image FROM AgLibraryKeywordImage")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        });
    match first {
        Ok(rows) => Ok(rows),
        Err(_) => {
            let mut stmt = conn.prepare("SELECT tag, image FROM AgLibraryKeywordImage")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        }
    }
}

/// An id the catalog itself carries, if it carries one.
fn read_catalog_uuid(conn: &Connection) -> Option<String> {
    if !table_exists(conn, "Adobe_variablesTable").ok()? {
        return None;
    }
    conn.query_row(
        "SELECT value FROM Adobe_variablesTable WHERE name = 'Adobe_DBUuid'",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .filter(|v| !v.trim().is_empty())
}

// --------------------------------------------------------------------- paths

/// Absolute path of one LR image's file: root folder path + path from root.
fn source_path(root: &LrRoot, image: &LrImage) -> PathBuf {
    let mut p = PathBuf::from(root.absolute_path.trim_end_matches('/'));
    for seg in image.rel_from_root.split('/') {
        if !seg.is_empty() {
            p.push(seg);
        }
    }
    p
}

/// Is this absolute path inside `root`, and at which relative path?
///
/// Both sides are canonicalized before comparing: an LR catalog records the
/// path the photographer's Finder showed, and on macOS `/tmp` and `/var` are
/// symlinks, so the same folder has two spellings and only one of them is a
/// prefix of the other.
fn rel_in_source(root: &Path, abs: &Path) -> Option<String> {
    let abs = abs.canonicalize().ok()?;
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = abs.strip_prefix(&root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        parts.push(c.as_os_str().to_str()?.to_string());
    }
    Some(parts.join("/"))
}

/// One path segment made safe for the library: no separators, no traversal,
/// never empty.
fn safe_segment(seg: &str) -> String {
    let cleaned: String = seg
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == '\0' { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." || cleaned.starts_with('.') {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

// ------------------------------------------------------------------- lr scan

/// One root folder as the scan reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrRootReport {
    /// The root folder's absolute path, as the catalog records it.
    pub path: String,
    pub name: String,
    pub file_count: usize,
    /// Files the catalog references that are not on disk at that path.
    pub missing_files: usize,
    /// The root lies inside a registered source — the library root or one
    /// added since — so an import catalogues its files where they are instead
    /// of copying. `false` means the files live outside every source, and an
    /// import copies them in unless it is told to reference them.
    pub in_place: bool,
    /// The root can be registered as a source and imported **by reference**:
    /// catalogued where it lies, nothing copied, nothing moved. True for any
    /// path that is a readable folder — which is the whole condition, since
    /// referencing is just "address the files where they already are".
    ///
    /// The one thing that can still refuse it is overlap: a root that contains
    /// the library, or another source, cannot become a source of its own. The
    /// import reports that per root when it happens and copies instead.
    pub can_reference: bool,
}

/// One collection as the scan reports it, path within the LR hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrCollectionReport {
    /// Set hierarchy + name, '/'-joined, as the album path would read
    /// (lowercased, before any `album_prefix`).
    pub path: String,
    /// `collection`, `set` or `smart`.
    pub kind: String,
    pub member_count: usize,
}

/// What [`scan`] answers — a read-only look at a catalog, nothing written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrScanReport {
    /// Stable identifier this catalog will be linked under.
    pub catalog_id: String,
    pub root_folders: Vec<LrRootReport>,
    pub collections: Vec<LrCollectionReport>,
    pub keyword_count: usize,
    /// `Adobe_images` rows whose file chain does not resolve.
    pub images_without_files: usize,
    /// Optional data that was skipped, and why.
    pub notes: Vec<String>,
}

/// Look inside a catalog without changing anything.
pub fn scan(lib: &Library, lrcat: &Path) -> Result<LrScanReport> {
    let data = read_lr_catalog(lib, lrcat)?;

    let mut root_folders = Vec::new();
    for root in &data.roots {
        let mut file_count = 0usize;
        let mut missing = 0usize;
        for image in data.images.values().filter(|i| i.root_id == root.id) {
            file_count += 1;
            if !source_path(root, image).is_file() {
                missing += 1;
            }
        }
        let abs = PathBuf::from(root.absolute_path.trim_end_matches('/'));
        let in_place = lib.source_containing(&abs)?.is_some();
        root_folders.push(LrRootReport {
            path: root.absolute_path.clone(),
            name: root.name.clone(),
            file_count,
            missing_files: missing,
            in_place,
            can_reference: abs.is_dir(),
        });
    }

    let paths = collection_paths(&data.collections);
    let collections = data
        .collections
        .iter()
        .map(|c| LrCollectionReport {
            path: paths.get(&c.id).cloned().unwrap_or_else(|| c.name.clone()),
            kind: c.kind.as_str().to_string(),
            member_count: c.members.len(),
        })
        .collect();

    Ok(LrScanReport {
        catalog_id: data.catalog_id,
        root_folders,
        collections,
        keyword_count: data.keyword_count,
        images_without_files: data.images_without_files,
        notes: data.notes,
    })
}

/// The album-style path of every collection: ancestor set names then its own,
/// each segment normalised the way album paths are.
fn collection_paths(collections: &[LrCollection]) -> HashMap<i64, String> {
    let by_id: HashMap<i64, &LrCollection> = collections.iter().map(|c| (c.id, c)).collect();
    let mut out = HashMap::new();
    for c in collections {
        let mut segments = vec![safe_segment(&c.name)];
        let mut at = c.parent;
        let mut seen = HashSet::new();
        while let Some(parent) = at {
            if !seen.insert(parent) {
                break; // a cycle in the parent chain; stop rather than loop
            }
            let Some(p) = by_id.get(&parent) else { break };
            segments.push(safe_segment(&p.name));
            at = p.parent;
        }
        segments.reverse();
        let joined = segments.join("/");
        let path = normalize_path(&joined).unwrap_or(joined);
        out.insert(c.id, path);
    }
    out
}

// ----------------------------------------------------------------- lr import

/// How a name collision with an existing, unlinked album is resolved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergePolicy {
    /// Decide per album: merge into the existing album unless it carries an
    /// `lr_album_links` row from a *different* catalog — someone else's import
    /// owns it — in which case a `-lr` suffixed sibling is created.
    #[default]
    Auto,
    /// Add members to the existing album.
    Merge,
    /// Create a new album with `-lr` appended to the name.
    Suffix,
}

/// Where one Lightroom root folder's files should end up.
///
/// This is the choice the owner asked for as *bez migracije* — "without
/// migration". A decade of work is terabytes, and copying it into a new folder
/// to be able to catalogue it is a non-answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LrPlacement {
    /// Catalogue in place where the root already lies inside a registered
    /// source; copy in otherwise. The default, and what every import did
    /// before referencing existed.
    #[default]
    Auto,
    /// Catalogue where the files are. When the root is not inside any source
    /// yet, this registers it as one — the only way to address those files —
    /// and says so in the report.
    InPlace,
    /// Copy into the library under `dest_subdir`, whatever the root is.
    Copy,
    /// Register the root folder as a **source** (named after the LR root) and
    /// catalogue its files where they lie. Nothing is copied and nothing is
    /// moved. A root already inside a source needs no new one and is simply
    /// catalogued in place.
    Reference,
}

/// A placement chosen for one specific LR root folder, overriding the run's
/// default `mode`.
///
/// `root_id` is `AgLibraryRootFolder.id_local` — the id
/// [`LrScanReport`] hands the UI, so a panel that lists the roots can send back
/// exactly what the photographer ticked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LrRootPlacement {
    pub root_id: i64,
    pub mode: LrPlacement,
}

/// Options for one import run. Everything is optional; `{}` imports the whole
/// catalog with the defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LrImportOptions {
    /// Folder under the library root that copied files land in, per LR root
    /// folder: `<dest_subdir>/<root-folder-name>/<pathFromRoot>/<file>`.
    /// Defaults to `"lr"`. In-place files ignore this — they stay put.
    #[serde(default)]
    pub dest_subdir: Option<String>,
    /// Import only collections whose LR path matches one of these patterns
    /// (exact path, or a glob using `*`). `None` imports all. Photos are
    /// imported regardless — this narrows the album mapping only.
    #[serde(default)]
    pub collections: Option<Vec<String>>,
    /// Prefix prepended to every created album path, e.g. `"lightroom"`.
    #[serde(default)]
    pub album_prefix: Option<String>,
    /// What to do when a collection's album path is already taken by an album
    /// this catalog has no link to.
    #[serde(default)]
    pub collision: MergePolicy,
    /// Where files land, for every root that has no entry in
    /// [`roots`](Self::roots). Defaults to [`LrPlacement::Auto`], which is what
    /// every import did before referencing existed.
    #[serde(default)]
    pub mode: LrPlacement,
    /// Per-root overrides, by `AgLibraryRootFolder.id_local`. A photographer
    /// keeps the current year on the laptop and eight years on a NAS; those are
    /// two answers, not one.
    #[serde(default)]
    pub roots: Option<Vec<LrRootPlacement>>,
    /// Compute the full report without writing anything — no copies, no
    /// catalog rows, no links, and no source registered.
    #[serde(default)]
    pub dry_run: bool,
}

/// What one [`lr_import`] run did (or, under `dry_run`, would do).
#[derive(Debug, Clone, Default, Serialize)]
pub struct LrImportReport {
    /// Files copied into the library because they lived outside it.
    pub photos_copied: usize,
    /// LR images whose bytes were already in the catalog under some path —
    /// linked to the existing row, nothing copied.
    pub photos_linked_existing: usize,
    /// Files catalogued where they lie — nothing copied. Covers both the root
    /// that already sat under the library and the one referenced on its own
    /// drive; [`photos_referenced`](Self::photos_referenced) is the second of
    /// those two.
    pub photos_in_place: usize,
    /// The subset of [`photos_in_place`](Self::photos_in_place) that lives on a
    /// source other than the library root — referenced, never moved. This is
    /// the number that answers "how much of my Lightroom library did I import
    /// without copying a byte".
    pub photos_referenced: usize,
    /// Roots registered as sources by this run, `"<name> (<path>)"`. A source
    /// is a lasting change to the library — it is what makes those files
    /// findable again next time — so a run that adds one says so.
    pub sources_registered: Vec<String>,
    /// LR images whose file is not on disk where the catalog says.
    pub skipped_missing_files: usize,
    pub albums_created: usize,
    /// Albums that already existed and gained members, order or a rename.
    pub albums_updated: usize,
    pub memberships_added: usize,
    /// New photo-tag links written from LR keywords.
    pub tags_added: usize,
    /// Divergences left untouched — e.g. a collection renamed both in LR and
    /// here. Each entry names the collection and what was not done.
    pub conflicts: Vec<String>,
    /// Album paths that were already taken by an unlinked album, and how the
    /// collision policy resolved each.
    pub collisions: Vec<String>,
    /// Photos and albums this library imported earlier that the LR catalog no
    /// longer has. Reported only — deletions are never propagated.
    pub lr_deleted: Vec<String>,
    pub bytes_copied: u64,
    /// The run stopped on request; every count is a partial tally.
    pub cancelled: bool,
    /// Nothing was written; the counts are a forecast.
    pub dry_run: bool,
}

/// Import (or, with `dry_run`, forecast importing) a Lightroom catalog.
///
/// See the module docs for the ground rules: the original catalog is never
/// opened, sources are never modified, hand-entered ratings/flags/labels are
/// never overwritten, and re-running with the same catalog syncs rather than
/// duplicates.
pub fn lr_import(
    lib: &Library,
    lrcat: &Path,
    opts: &LrImportOptions,
    on_progress: Option<&(dyn Fn(ImportProgress) + Sync)>,
    should_stop: Option<&(dyn Fn() -> bool + Sync)>,
) -> Result<LrImportReport> {
    let stop = || should_stop.map(|f| f()).unwrap_or(false);
    let data = read_lr_catalog(lib, lrcat)?;
    let dry = opts.dry_run;

    let mut report = LrImportReport {
        dry_run: dry,
        ..Default::default()
    };

    let dest_subdir = match &opts.dest_subdir {
        Some(s) => {
            let s = safe_segment(s);
            if s.is_empty() {
                DEFAULT_DEST_SUBDIR.to_string()
            } else {
                s
            }
        }
        None => DEFAULT_DEST_SUBDIR.to_string(),
    };

    let roots: HashMap<i64, &LrRoot> = data.roots.iter().map(|r| (r.id, r)).collect();
    let mut links = lib.lr_photo_links(&data.catalog_id)?;
    // Where each root's files go — including any source this run registers.
    let placements = plan_root_placements(lib, &data, opts, &mut report)?;
    let primary = lib.primary_source_id()?;

    // ----------------------------------------------------------- photo pass
    //
    // For every LR image: find or make the catalog row it maps to, remember
    // the mapping, and carry LR's rating/flag/label/keywords across (without
    // ever overwriting a value someone set here by hand).
    let total = data.images.len();
    let thumb_root = lib.thumb_dir();
    let now = chrono::Utc::now().to_rfc3339();
    // image id → photo id, for the collections pass. Dry runs use a negative
    // placeholder for photos that do not exist yet.
    let mut photo_of: HashMap<i64, i64> = HashMap::new();
    let mut next_phantom = -1i64;

    for (done, image) in data.images.values().enumerate() {
        if stop() {
            report.cancelled = true;
            return Ok(report);
        }
        if let Some(cb) = on_progress {
            cb(ImportProgress {
                processed: done + 1,
                total,
                current: image.rel_from_root.clone(),
            });
        }

        // Already linked from an earlier run?
        if let Some(photo_id) = links.get(&image.id) {
            if lib.photo_by_id(*photo_id).is_ok() {
                photo_of.insert(image.id, *photo_id);
                if !dry {
                    report.tags_added += apply_lr_metadata(
                        lib,
                        *photo_id,
                        image,
                        data.keywords_per_image.get(&image.id),
                    )?;
                }
                continue;
            }
            // The linked row is gone (pruned); fall through and re-resolve.
        }

        let Some(root) = roots.get(&image.root_id) else {
            report.skipped_missing_files += 1;
            continue;
        };
        let source = source_path(root, image);
        if !source.is_file() {
            report.skipped_missing_files += 1;
            continue;
        }

        // In place, or copy in — decided per root, up front, because the two
        // treat duplicate bytes differently: a file that stays where it is gets
        // catalogued at its own path even when another path holds the same
        // bytes (a file the catalog forgets is worse than a duplicate row),
        // while a file that would need copying is linked to the existing row
        // instead of copied again.
        let placement = placements
            .get(&image.root_id)
            .copied()
            .unwrap_or(Placement::CopyIn);
        if placement == Placement::CopyIn {
            let hash = import::hash_file(&source)?;
            if let Some(existing) = lib.photo_id_by_hash(&hash)? {
                report.photos_linked_existing += 1;
                photo_of.insert(image.id, existing);
                if !dry {
                    lib.set_lr_photo_link(&data.catalog_id, image.id, existing)?;
                    links.insert(image.id, existing);
                    report.tags_added += apply_lr_metadata(
                        lib,
                        existing,
                        image,
                        data.keywords_per_image.get(&image.id),
                    )?;
                }
                continue;
            }
        }

        let (abs, rel, source_id) = match placement {
            Placement::InSource(id) => {
                let root_path = lib.source_root(id)?;
                let Some(rel) = rel_in_source(&root_path, &source) else {
                    report.conflicts.push(format!(
                        "{}: its file is not under the source it was placed in — skipped",
                        image.rel_from_root
                    ));
                    continue;
                };
                report.photos_in_place += 1;
                if id != primary {
                    report.photos_referenced += 1;
                }
                (source.clone(), rel, id)
            }
            // Dry runs only; the `if dry` below is what it falls into.
            Placement::WouldReference => {
                report.photos_in_place += 1;
                report.photos_referenced += 1;
                (source.clone(), image.rel_from_root.clone(), primary)
            }
            Placement::CopyIn => {
                let mut rel = format!("{dest_subdir}/{}", safe_segment(&root.name));
                for seg in image.rel_from_root.split('/').filter(|s| !s.is_empty()) {
                    rel.push('/');
                    rel.push_str(&safe_segment(seg));
                }
                let dest = lib.resolve(&rel)?;
                let size = source.metadata().map(|m| m.len()).unwrap_or(0);
                if dest.is_file() && !import::same_file(&source, &dest) {
                    report.conflicts.push(format!(
                        "{}: {} already exists in the library with different \
                         bytes — not overwritten, photo skipped",
                        image.rel_from_root, rel
                    ));
                    continue;
                }
                report.photos_copied += 1;
                if !dest.is_file() {
                    report.bytes_copied += size;
                    if !dry {
                        if let Some(parent) = dest.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| Error::io(parent, e))?;
                        }
                        std::fs::copy(&source, &dest).map_err(|e| Error::io(&source, e))?;
                    }
                }
                (dest, rel, primary)
            }
        };

        if dry {
            photo_of.insert(image.id, next_phantom);
            next_phantom -= 1;
            report.tags_added += data
                .keywords_per_image
                .get(&image.id)
                .map(|k| k.len())
                .unwrap_or(0);
            continue;
        }

        // Catalogue it through the same pipeline a directory import uses —
        // one file per transaction here; an LR import is copy-dominated.
        let Some(kind) = crate::media::classify(&abs) else {
            report.conflicts.push(format!(
                "{}: not a file format this app imports — skipped",
                image.rel_from_root
            ));
            continue;
        };
        let cand = import::candidate_for(&abs, &rel, source_id, kind)?;
        let processed = import::process_one(&cand, &thumb_root, kind == PhotoKind::Photo)?;
        let photo_id = lib.with_tx(|tx| Ok(import::upsert_processed(tx, &processed, &now)?.photo_id))?;
        lib.set_lr_photo_link(&data.catalog_id, image.id, photo_id)?;
        links.insert(image.id, photo_id);
        photo_of.insert(image.id, photo_id);
        report.tags_added +=
            apply_lr_metadata(lib, photo_id, image, data.keywords_per_image.get(&image.id))?;
    }

    // LR-side photo deletions: linked images the catalog no longer has.
    for (lr_image, photo_id) in &links {
        if !data.images.contains_key(lr_image) {
            let name = lib
                .photo_by_id(*photo_id)
                .map(|p| p.rel_path)
                .unwrap_or_else(|_| format!("photo #{photo_id}"));
            report
                .lr_deleted
                .push(format!("{name} (deleted in Lightroom; kept here)"));
        }
    }

    // ------------------------------------------------------ collections pass
    import_collections(lib, &data, opts, &photo_of, &mut report)?;

    Ok(report)
}

/// Where one root's files are going, resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Placement {
    /// Catalogue in place, under this registered source.
    InSource(i64),
    /// A dry run's answer for a root a real run would register as a source.
    /// Only ever produced under `dry_run`.
    WouldReference,
    /// Copy into the primary under `dest_subdir`.
    CopyIn,
}

/// Decide, per LR root folder, whether its files are catalogued where they lie
/// or copied in — registering a source where that is what was asked for.
///
/// Done once, up front, rather than per image: registering a source is a
/// library-level act, and a run that touched a thousand frames should have made
/// that decision once, visibly, in the report.
fn plan_root_placements(
    lib: &Library,
    data: &LrData,
    opts: &LrImportOptions,
    report: &mut LrImportReport,
) -> Result<HashMap<i64, Placement>> {
    let per_root: HashMap<i64, LrPlacement> = opts
        .roots
        .iter()
        .flatten()
        .map(|r| (r.root_id, r.mode))
        .collect();

    let mut out = HashMap::new();
    for root in &data.roots {
        let mode = per_root.get(&root.id).copied().unwrap_or(opts.mode);
        let abs = PathBuf::from(root.absolute_path.trim_end_matches('/'));
        let abs = abs.canonicalize().unwrap_or(abs);
        let containing = lib.source_containing(&abs)?;

        let placement = match (mode, containing) {
            // Already reachable: nothing to register, nothing to copy.
            (LrPlacement::Auto | LrPlacement::InPlace | LrPlacement::Reference, Some(source)) => {
                Placement::InSource(source.id)
            }
            (LrPlacement::Copy, _) | (LrPlacement::Auto, None) => Placement::CopyIn,
            (LrPlacement::InPlace | LrPlacement::Reference, None) => {
                if opts.dry_run {
                    report
                        .sources_registered
                        .push(format!("{} ({})", root.name, abs.display()));
                    Placement::WouldReference
                } else {
                    let name = (!root.name.trim().is_empty()).then(|| root.name.clone());
                    match lib.add_source(&abs, name.as_deref(), None) {
                        Ok(id) => {
                            report
                                .sources_registered
                                .push(format!("{} ({})", root.name, abs.display()));
                            if mode == LrPlacement::InPlace {
                                // In-place on an outside root can only be done
                                // by referencing it, and a new source is a
                                // lasting change to the library. Say so rather
                                // than let it be discovered later.
                                report.conflicts.push(format!(
                                    "root folder \"{}\": it is outside the library, so \
                                     importing in place registered {} as a source",
                                    root.name,
                                    abs.display()
                                ));
                            }
                            Placement::InSource(id)
                        }
                        // Overlap, or a path that is not there. Copying is the
                        // answer that still gets the photographs in.
                        Err(e) => {
                            report.conflicts.push(format!(
                                "root folder \"{}\": could not be referenced ({e}) — \
                                 its files were copied in instead",
                                root.name
                            ));
                            Placement::CopyIn
                        }
                    }
                }
            }
        };
        out.insert(root.id, placement);
    }
    Ok(out)
}

/// Carry LR's rating, flag, colour label and keywords onto one photo row.
///
/// The rule is the catalog's own: a hand-entered value is never overwritten,
/// so each field lands only where the row still holds its default. Keywords
/// are additive — LR's are merged into whatever tags the photo already has.
/// Returns how many tag links were newly made.
fn apply_lr_metadata(
    lib: &Library,
    photo_id: i64,
    image: &LrImage,
    keywords: Option<&Vec<String>>,
) -> Result<usize> {
    lib.with_tx(|tx| {
        if image.rating > 0 {
            tx.execute(
                "UPDATE photos SET rating = ?1 WHERE id = ?2 AND rating = 0",
                params![image.rating.min(5) as i64, photo_id],
            )?;
        }
        if image.flag != Flag::None {
            tx.execute(
                "UPDATE photos SET flag = ?1 WHERE id = ?2 AND flag = 'none'",
                params![image.flag.as_str(), photo_id],
            )?;
        }
        if let Some(label) = &image.color_label {
            tx.execute(
                "UPDATE photos SET color_label = ?1 WHERE id = ?2 AND color_label IS NULL",
                params![label, photo_id],
            )?;
        }
        match keywords {
            Some(k) if !k.is_empty() => crate::albums::write_photo_tags(tx, photo_id, k),
            _ => Ok(0),
        }
    })
}

/// Does an LR collection path match one of the requested patterns?
/// A pattern is an exact path or a `*` glob (each `*` matches any run).
fn matches_pattern(path: &str, pattern: &str) -> bool {
    fn glob(pat: &[u8], text: &[u8]) -> bool {
        match pat.first() {
            None => text.is_empty(),
            Some(b'*') => {
                glob(&pat[1..], text) || (!text.is_empty() && glob(pat, &text[1..]))
            }
            Some(c) => text.first() == Some(c) && glob(&pat[1..], &text[1..]),
        }
    }
    glob(
        pattern.to_lowercase().as_bytes(),
        path.to_lowercase().as_bytes(),
    )
}

fn import_collections(
    lib: &Library,
    data: &LrData,
    opts: &LrImportOptions,
    photo_of: &HashMap<i64, i64>,
    report: &mut LrImportReport,
) -> Result<()> {
    let dry = opts.dry_run;
    let lr_paths = collection_paths(&data.collections);
    let mut album_links = lib.lr_album_links(&data.catalog_id)?;
    let prefix = opts
        .album_prefix
        .as_deref()
        .map(normalize_path)
        .transpose()?;

    // Parents before children, so a set's album exists when its collections
    // arrive.
    let mut ordered: Vec<&LrCollection> = data.collections.iter().collect();
    ordered.sort_by_key(|c| lr_paths.get(&c.id).cloned().unwrap_or_default());

    for collection in &ordered {
        let lr_path = lr_paths
            .get(&collection.id)
            .cloned()
            .unwrap_or_else(|| collection.name.clone());

        if let Some(wanted) = &opts.collections {
            let included = wanted.iter().any(|w| matches_pattern(&lr_path, w))
                // A set is included when anything under it is.
                || (collection.kind == LrCollectionKind::Set
                    && ordered.iter().any(|other| {
                        lr_paths
                            .get(&other.id)
                            .map(|p| p.starts_with(&format!("{lr_path}/")))
                            .unwrap_or(false)
                            && wanted.iter().any(|w| {
                                matches_pattern(lr_paths.get(&other.id).unwrap(), w)
                            })
                    }));
            if !included {
                continue;
            }
        }

        let computed = match &prefix {
            Some(p) => format!("{p}/{lr_path}"),
            None => lr_path.clone(),
        };

        // Resolve where this collection's album lives, honouring an earlier
        // link before anything else.
        let target = match album_links.get(&collection.id).cloned() {
            Some(linked) => {
                let at_linked = lib.album_by_path(&linked)?.is_some();
                if linked == computed {
                    if at_linked {
                        Some(linked)
                    } else {
                        // Deleted locally since. Recreating it would override a
                        // deliberate local act; report instead.
                        report.conflicts.push(format!(
                            "collection \"{}\": its album {linked} was deleted here — \
                             not recreated",
                            collection.name
                        ));
                        None
                    }
                } else if at_linked {
                    // Renamed in LR. Follow, unless the computed path is taken
                    // or the local album has its own plans.
                    if lib.album_by_path(&computed)?.is_some() {
                        report.conflicts.push(format!(
                            "collection \"{}\": renamed in Lightroom to {computed}, but \
                             an album already sits there — left at {linked}",
                            collection.name
                        ));
                        Some(linked)
                    } else {
                        if !dry {
                            lib.move_album(&linked, &computed)?;
                            lib.set_lr_album_link(&data.catalog_id, collection.id, &computed)?;
                        }
                        report.albums_updated += 1;
                        Some(computed.clone())
                    }
                } else if lib.album_by_path(&computed)?.is_some() {
                    // The album moved locally to exactly where LR now says —
                    // or was recreated there. Either way, relink.
                    if !dry {
                        lib.set_lr_album_link(&data.catalog_id, collection.id, &computed)?;
                    }
                    Some(computed.clone())
                } else {
                    // Renamed on both sides (or deleted here): nobody can say
                    // which name wins, so nothing changes.
                    report.conflicts.push(format!(
                        "collection \"{}\": renamed in Lightroom to {computed} and \
                         renamed or deleted here (album {linked} is gone) — nothing \
                         changed",
                        collection.name
                    ));
                    None
                }
            }
            None => {
                // No link yet: either the path is free, or an unrelated album
                // holds it and the collision policy decides.
                match lib.album_by_path(&computed)? {
                    None => Some(computed.clone()),
                    Some(_) => {
                        let merge = match opts.collision {
                            MergePolicy::Merge => true,
                            MergePolicy::Suffix => false,
                            MergePolicy::Auto => {
                                !lib.album_has_foreign_lr_link(&computed, &data.catalog_id)?
                            }
                        };
                        if merge {
                            report.collisions.push(format!(
                                "{computed} already exists — merged \"{}\" into it",
                                collection.name
                            ));
                            Some(computed.clone())
                        } else {
                            let suffixed = free_suffixed_path(lib, &computed)?;
                            report.collisions.push(format!(
                                "{computed} already exists — created {suffixed} for \"{}\"",
                                collection.name
                            ));
                            Some(suffixed)
                        }
                    }
                }
            }
        };
        let Some(target) = target else { continue };

        // Create the album if it is not there yet.
        let existing = lib.album_by_path(&target)?;
        if existing.is_none() {
            report.albums_created += 1;
            if !dry {
                lib.create_album(&NewAlbum {
                    path: target.clone(),
                    title: Some(collection.name.clone()),
                    description: (collection.kind == LrCollectionKind::Smart).then(|| {
                        "Imported from a Lightroom smart collection — a snapshot of \
                         its members at import time, not a live rule."
                            .to_string()
                    }),
                    is_collection: collection.kind == LrCollectionKind::Set,
                    ..Default::default()
                })?;
            }
        }
        if !dry {
            lib.set_lr_album_link(&data.catalog_id, collection.id, &target)?;
            album_links.insert(collection.id, target.clone());
        }

        // Members — sets hold collections, not photos.
        if collection.kind != LrCollectionKind::Set && !collection.members.is_empty() {
            let mut members = collection.members.clone();
            // LR's order where it has one; id order breaks the ties, and
            // members LR holds no position for follow the positioned ones.
            members.sort_by(|a, b| match (a.1, b.1) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.0.cmp(&b.0),
            });
            let has_order = members.iter().any(|(_, p)| p.is_some());

            let ids: Vec<i64> = members
                .iter()
                .filter_map(|(image, _)| photo_of.get(image).copied())
                .collect();
            if dry {
                // Forecast: photos not yet catalogued (phantoms, negative ids)
                // would all be added; existing ones only if not members yet.
                let current: HashSet<i64> = match existing.is_some() {
                    true => lib
                        .album_photos(&target)?
                        .into_iter()
                        .map(|p| p.id)
                        .collect(),
                    false => HashSet::new(),
                };
                let would_add = ids.iter().filter(|id| !current.contains(id)).count();
                report.memberships_added += would_add;
                if existing.is_some() && would_add > 0 {
                    report.albums_updated += 1;
                }
            } else {
                let real: Vec<i64> = ids.into_iter().filter(|id| *id > 0).collect();
                let added = lib.add_photos_to_album(&target, &real)?;
                report.memberships_added += added;
                if added > 0 {
                    if existing.is_some() {
                        report.albums_updated += 1;
                    }
                    // Only when membership moved: re-running an unchanged
                    // import must not re-impose LR's order over a local
                    // rearrangement.
                    if has_order {
                        lib.reorder_album(&target, &real)?;
                        lib.update_album(
                            &target,
                            &AlbumUpdate {
                                sort: Some("custom".into()),
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }
        }
    }

    // LR-side collection deletions: linked albums whose collection is gone.
    let live: HashSet<i64> = data.collections.iter().map(|c| c.id).collect();
    for (lr_collection, album_path) in &album_links {
        if !live.contains(lr_collection) {
            report.lr_deleted.push(format!(
                "album {album_path} (its Lightroom collection was deleted; kept here)"
            ));
        }
    }

    Ok(())
}

/// `path-lr`, or `path-lr-2` and so on when that is taken too.
fn free_suffixed_path(lib: &Library, path: &str) -> Result<String> {
    let first = format!("{path}-lr");
    if lib.album_by_path(&first)?.is_none() {
        return Ok(first);
    }
    for n in 2..1000 {
        let candidate = format!("{path}-lr-{n}");
        if lib.album_by_path(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(Error::other(format!(
        "no free album path near {path}-lr — a thousand suffixes are taken"
    )))
}

// -------------------------------------------------------- catalog link store

impl Library {
    /// Photo id by content hash, any path. First imported wins ties — two rows
    /// with the same bytes are the same negative either way.
    pub(crate) fn photo_id_by_hash(&self, hash: &str) -> Result<Option<i64>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT id FROM photos WHERE content_hash = ?1 ORDER BY id LIMIT 1",
                params![hash],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    /// Every photo link recorded for one source catalog: lr image id → photo id.
    pub(crate) fn lr_photo_links(&self, lrcat_id: &str) -> Result<HashMap<i64, i64>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT lr_image, photo_id FROM lr_links WHERE lrcat_id = ?1")?;
            let rows = stmt.query_map(params![lrcat_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut out = HashMap::new();
            for row in rows {
                let (k, v) = row?;
                out.insert(k, v);
            }
            Ok(out)
        })
    }

    pub(crate) fn set_lr_photo_link(
        &self,
        lrcat_id: &str,
        lr_image: i64,
        photo_id: i64,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO lr_links(lrcat_id, lr_image, photo_id) VALUES(?1,?2,?3) \
                 ON CONFLICT(lrcat_id, lr_image) DO UPDATE SET photo_id = excluded.photo_id",
                params![lrcat_id, lr_image, photo_id],
            )?;
            Ok(())
        })
    }

    /// Every album link recorded for one source catalog.
    pub(crate) fn lr_album_links(&self, lrcat_id: &str) -> Result<HashMap<i64, String>> {
        self.with_conn(|c| {
            let mut stmt = c
                .prepare("SELECT lr_collection, album_path FROM lr_album_links WHERE lrcat_id = ?1")?;
            let rows = stmt.query_map(params![lrcat_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut out = HashMap::new();
            for row in rows {
                let (k, v) = row?;
                out.insert(k, v);
            }
            Ok(out)
        })
    }

    pub(crate) fn set_lr_album_link(
        &self,
        lrcat_id: &str,
        lr_collection: i64,
        album_path: &str,
    ) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO lr_album_links(lrcat_id, lr_collection, album_path) \
                 VALUES(?1,?2,?3) \
                 ON CONFLICT(lrcat_id, lr_collection) DO UPDATE SET \
                 album_path = excluded.album_path",
                params![lrcat_id, lr_collection, album_path],
            )?;
            Ok(())
        })
    }

    /// Is this album path claimed by an `lr_album_links` row from a catalog
    /// other than `lrcat_id`? That is what makes the Auto collision policy
    /// refuse to merge: the album is another import's, not a hand-made one.
    pub(crate) fn album_has_foreign_lr_link(&self, path: &str, lrcat_id: &str) -> Result<bool> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM lr_album_links WHERE album_path = ?1 AND lrcat_id <> ?2",
                params![path, lrcat_id],
                |r| r.get(0),
            )?;
            Ok(n > 0)
        })
    }
}

// --------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PhotoFilter;

    fn write_jpeg(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgb8(w, h)
            .save_with_format(path, image::ImageFormat::Jpeg)
            .unwrap();
    }

    fn create_lr_schema(conn: &Connection) {
        conn.execute_batch(
            r#"
            CREATE TABLE AgLibraryRootFolder (
              id_local INTEGER PRIMARY KEY, absolutePath TEXT, name TEXT);
            CREATE TABLE AgLibraryFolder (
              id_local INTEGER PRIMARY KEY, pathFromRoot TEXT, rootFolder INTEGER);
            CREATE TABLE AgLibraryFile (
              id_local INTEGER PRIMARY KEY, idx_filename TEXT, folder INTEGER,
              extension TEXT, baseName TEXT);
            CREATE TABLE Adobe_images (
              id_local INTEGER PRIMARY KEY, rootFile INTEGER, rating REAL,
              colorLabels TEXT, pick REAL, captureTime TEXT, orientation TEXT,
              fileFormat TEXT);
            CREATE TABLE AgLibraryCollection (
              id_local INTEGER PRIMARY KEY, name TEXT, parent INTEGER, creationId TEXT);
            CREATE TABLE AgLibraryCollectionimage (
              id_local INTEGER PRIMARY KEY, collection INTEGER, image INTEGER,
              positionInCollection REAL);
            CREATE TABLE AgLibraryKeyword (
              id_local INTEGER PRIMARY KEY, name TEXT, parent INTEGER);
            CREATE TABLE AgLibraryKeywordImage (
              id_local INTEGER PRIMARY KEY, keyword INTEGER, image INTEGER);
            "#,
        )
        .unwrap();
    }

    /// The fixture the module docs promise: two root folders (nested folders
    /// in one), six files including one that is referenced but missing on
    /// disk, an image with no file at all, a collection inside a set, a smart
    /// collection, an unknown creationId, a keyword hierarchy, and
    /// ratings/picks/labels spread across the images.
    fn standard_fixture(lrcat: &Path, root_a: &Path, root_b: &Path) {
        write_jpeg(&root_a.join("a1.jpg"), 40, 30);
        write_jpeg(&root_a.join("a2.jpg"), 42, 30);
        write_jpeg(&root_a.join("sub/a3.jpg"), 44, 30);
        write_jpeg(&root_b.join("b1.jpg"), 46, 30);
        write_jpeg(&root_b.join("b2.jpg"), 48, 30);
        // gone.jpg is referenced by the catalog but never written.

        let conn = Connection::open(lrcat).unwrap();
        create_lr_schema(&conn);
        let slash = |p: &Path| format!("{}/", p.display());
        conn.execute_batch(&format!(
            r#"
            INSERT INTO AgLibraryRootFolder VALUES (1, '{root_a}', 'shoot-a');
            INSERT INTO AgLibraryRootFolder VALUES (2, '{root_b}', 'shoot-b');
            INSERT INTO AgLibraryFolder VALUES (10, '', 1);
            INSERT INTO AgLibraryFolder VALUES (11, 'sub/', 1);
            INSERT INTO AgLibraryFolder VALUES (20, '', 2);
            INSERT INTO AgLibraryFile VALUES (100, 'a1.jpg',   10, 'jpg', 'a1');
            INSERT INTO AgLibraryFile VALUES (101, 'a2.jpg',   10, 'jpg', 'a2');
            INSERT INTO AgLibraryFile VALUES (102, 'a3.jpg',   11, 'jpg', 'a3');
            INSERT INTO AgLibraryFile VALUES (200, 'b1.jpg',   20, 'jpg', 'b1');
            INSERT INTO AgLibraryFile VALUES (201, 'b2.jpg',   20, 'jpg', 'b2');
            INSERT INTO AgLibraryFile VALUES (202, 'gone.jpg', 20, 'jpg', 'gone');
            INSERT INTO Adobe_images VALUES (1000, 100, 3.0, NULL,  1.0, NULL, NULL, 'JPG');
            INSERT INTO Adobe_images VALUES (1001, 101, NULL, 'Red', -1.0, NULL, NULL, 'JPG');
            INSERT INTO Adobe_images VALUES (1002, 102, 5.0, NULL,  0.0, NULL, NULL, 'JPG');
            INSERT INTO Adobe_images VALUES (1003, 200, NULL, NULL, NULL, NULL, NULL, 'JPG');
            INSERT INTO Adobe_images VALUES (1004, 201, NULL, '',   NULL, NULL, NULL, 'JPG');
            INSERT INTO Adobe_images VALUES (1005, 202, NULL, NULL, NULL, NULL, NULL, 'JPG');
            INSERT INTO Adobe_images VALUES (1006, NULL, NULL, NULL, NULL, NULL, NULL, 'JPG');
            INSERT INTO AgLibraryCollection VALUES (1, 'Weddings', NULL, 'com.adobe.ag.library.group');
            INSERT INTO AgLibraryCollection VALUES (2, 'Ana Ivan', 1, 'com.adobe.ag.library.collection');
            INSERT INTO AgLibraryCollection VALUES (3, 'Best of 2026', NULL, 'com.adobe.ag.library.smart_collection');
            INSERT INTO AgLibraryCollection VALUES (4, 'Mystery', NULL, 'com.adobe.ag.mystery.kind');
            INSERT INTO AgLibraryCollectionimage VALUES (1, 2, 1000, 2.0);
            INSERT INTO AgLibraryCollectionimage VALUES (2, 2, 1001, 1.0);
            INSERT INTO AgLibraryCollectionimage VALUES (3, 2, 1003, 3.0);
            INSERT INTO AgLibraryCollectionimage VALUES (4, 3, 1002, NULL);
            INSERT INTO AgLibraryKeyword VALUES (1, NULL, NULL);
            INSERT INTO AgLibraryKeyword VALUES (2, 'Wedding', 1);
            INSERT INTO AgLibraryKeyword VALUES (3, 'Bride', 2);
            INSERT INTO AgLibraryKeywordImage VALUES (1, 3, 1000);
            INSERT INTO AgLibraryKeywordImage VALUES (2, 2, 1003);
            "#,
            root_a = slash(root_a),
            root_b = slash(root_b),
        ))
        .unwrap();
    }

    /// A library whose root holds `shoot-a`, an outside `shoot-b`, and the
    /// fixture catalog describing both.
    fn fixture() -> (Library, tempfile::TempDir, tempfile::TempDir, PathBuf) {
        let lib_dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let root_a = lib_dir.path().join("shoot-a");
        let root_b = elsewhere.path().join("shoot-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let lrcat = elsewhere.path().join("photos.lrcat");
        standard_fixture(&lrcat, &root_a, &root_b);
        let lib = Library::open(lib_dir.path()).unwrap();
        (lib, lib_dir, elsewhere, lrcat)
    }

    fn photo_by_name<'a>(
        photos: &'a [crate::model::Photo],
        name: &str,
    ) -> &'a crate::model::Photo {
        photos
            .iter()
            .find(|p| p.filename == name)
            .unwrap_or_else(|| panic!("{name} is not in the catalog"))
    }

    // ------------------------------------------------------------- the reader

    #[test]
    fn the_reader_sees_folders_ratings_collections_keywords_and_missing_files() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        let report = scan(&lib, &lrcat).unwrap();

        assert!(!report.catalog_id.is_empty());
        assert_eq!(report.images_without_files, 1, "the image with no rootFile");
        assert_eq!(report.keyword_count, 2, "the nameless root keyword is not one");

        let a = report.root_folders.iter().find(|r| r.name == "shoot-a").unwrap();
        assert_eq!((a.file_count, a.missing_files), (3, 0));
        assert!(a.in_place, "shoot-a lies under the library root");
        let b = report.root_folders.iter().find(|r| r.name == "shoot-b").unwrap();
        assert_eq!((b.file_count, b.missing_files), (3, 1), "gone.jpg is referenced, absent");
        assert!(!b.in_place, "shoot-b is outside every source");
        // …but it is a folder that is there, so it can be catalogued where it
        // lies instead of copied — which is the whole answer to "import my
        // Lightroom library without migrating a terabyte".
        assert!(b.can_reference);
        assert!(a.can_reference, "an in-place root is referenceable by definition");

        let mut kinds: Vec<(String, String, usize)> = report
            .collections
            .iter()
            .map(|c| (c.path.clone(), c.kind.clone(), c.member_count))
            .collect();
        kinds.sort();
        assert_eq!(
            kinds,
            vec![
                ("best of 2026".into(), "smart".into(), 1),
                ("weddings".into(), "set".into(), 0),
                ("weddings/ana ivan".into(), "collection".into(), 3),
            ]
        );
        // The unknown creationId was skipped, with a note saying so.
        assert!(
            report.notes.iter().any(|n| n.contains("Mystery")),
            "the skipped collection has to be named: {:?}",
            report.notes
        );
    }

    #[test]
    fn a_catalog_missing_a_required_table_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let lrcat = dir.path().join("broken.lrcat");
        let conn = Connection::open(&lrcat).unwrap();
        conn.execute_batch(
            "CREATE TABLE AgLibraryRootFolder (id_local INTEGER, absolutePath TEXT, name TEXT);",
        )
        .unwrap();
        drop(conn);

        let lib = Library::open(dir.path().join("lib")).unwrap();
        let err = scan(&lib, &lrcat).unwrap_err().to_string();
        assert!(
            err.contains("AgLibraryFolder") && err.to_lowercase().contains("lightroom"),
            "the refusal must name the missing table and the likely cause: {err}"
        );
    }

    #[test]
    fn scanning_writes_nothing_and_never_opens_the_original() {
        let (lib, _lib_dir, elsewhere, lrcat) = fixture();
        let before = std::fs::read(&lrcat).unwrap();
        let mtime = std::fs::metadata(&lrcat).unwrap().modified().unwrap();

        scan(&lib, &lrcat).unwrap();

        assert_eq!(std::fs::read(&lrcat).unwrap(), before, "the original changed");
        assert_eq!(std::fs::metadata(&lrcat).unwrap().modified().unwrap(), mtime);
        // No stray -wal/-shm beside the original either.
        assert!(!elsewhere.path().join("photos.lrcat-wal").exists());
        assert_eq!(lib.photo_count().unwrap(), 0);
        assert!(lib.albums().unwrap().is_empty());
    }

    // ------------------------------------------------------------- the import

    #[test]
    fn lr_import_copies_outside_roots_and_catalogues_inside_ones_in_place() {
        let (lib, lib_dir, elsewhere, lrcat) = fixture();
        let report =
            lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();

        assert_eq!(report.photos_in_place, 3, "shoot-a is already under the root");
        assert_eq!(report.photos_copied, 2, "b1 and b2 came in from outside");
        assert_eq!(report.skipped_missing_files, 1, "gone.jpg is not on disk");
        assert!(report.bytes_copied > 0);
        assert!(!report.cancelled);

        // The copies landed under lr/<root-folder-name>/, the in-place files
        // stayed exactly where they were, and the sources are untouched.
        assert!(lib_dir.path().join("lr/shoot-b/b1.jpg").exists());
        assert!(lib_dir.path().join("lr/shoot-b/b2.jpg").exists());
        assert!(elsewhere.path().join("shoot-b/b1.jpg").exists());
        assert!(!lib_dir.path().join("lr/shoot-a").exists());

        let photos = lib.photos(&PhotoFilter::default()).unwrap();
        assert_eq!(photos.len(), 5);
        assert_eq!(photo_by_name(&photos, "a3.jpg").rel_path, "shoot-a/sub/a3.jpg");

        // Ratings, picks and labels crossed over.
        let a1 = photo_by_name(&photos, "a1.jpg");
        assert_eq!(a1.rating, 3);
        assert_eq!(a1.flag, Flag::Pick);
        let a2 = photo_by_name(&photos, "a2.jpg");
        assert_eq!(a2.rating, 0, "a NULL rating is unrated");
        assert_eq!(a2.flag, Flag::Reject);
        assert_eq!(a2.color_label.as_deref(), Some("Red"));
        let b2 = photo_by_name(&photos, "b2.jpg");
        assert_eq!(b2.color_label, None, "an empty colorLabels string is no label");

        // Keywords became tags — the keyword's own name and its ancestors'.
        let mut a1_tags = lib.photo_tags(a1.id).unwrap();
        a1_tags.sort();
        assert_eq!(a1_tags, vec!["bride", "wedding"]);
        let b1 = photo_by_name(&photos, "b1.jpg");
        assert_eq!(lib.photo_tags(b1.id).unwrap(), vec!["wedding"]);

        // Collections became albums: the set a collection, the smart one a
        // snapshot with a description saying so.
        assert_eq!(report.albums_created, 3);
        let weddings = lib.album_by_path("weddings").unwrap().unwrap();
        assert!(weddings.is_collection);
        let ana = lib.album_by_path("weddings/ana ivan").unwrap().unwrap();
        assert!(!ana.is_collection);
        assert_eq!(ana.title, "Ana Ivan");
        assert_eq!(ana.sort, "custom", "LR held a custom order");
        let smart = lib.album_by_path("best of 2026").unwrap().unwrap();
        assert!(smart.description.as_deref().unwrap_or("").contains("smart collection"));

        // Membership follows LR's positionInCollection: a2 (1.0), a1 (2.0),
        // b1 (3.0).
        let members: Vec<String> = lib
            .album_photos("weddings/ana ivan")
            .unwrap()
            .into_iter()
            .map(|p| p.filename)
            .collect();
        assert_eq!(members, vec!["a2.jpg", "a1.jpg", "b1.jpg"]);
        assert_eq!(report.memberships_added, 4);
        assert_eq!(
            lib.album_photos("best of 2026").unwrap().len(),
            1,
            "the smart collection's snapshot member"
        );
    }

    /// Reference mode: the outside root becomes a source, and not one byte is
    /// copied.
    ///
    /// This is the mode the whole feature exists for. A photographer with a
    /// decade of Lightroom has terabytes on drives that are already organised;
    /// asking them to duplicate all of it into a new folder before the app can
    /// see it is a non-answer, and it was the only answer this importer had.
    #[test]
    fn reference_mode_registers_the_root_as_a_source_and_copies_nothing() {
        let (lib, lib_dir, elsewhere, lrcat) = fixture();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                mode: LrPlacement::Reference,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(report.photos_copied, 0, "reference mode must copy nothing");
        assert_eq!(report.bytes_copied, 0);
        assert_eq!(report.photos_in_place, 5, "three inside, two referenced");
        assert_eq!(report.photos_referenced, 2, "shoot-b's two readable frames");
        assert_eq!(report.skipped_missing_files, 1, "gone.jpg is still not on disk");
        assert!(
            report.sources_registered.iter().any(|s| s.contains("shoot-b")),
            "registering a source is a lasting change and must be reported: {:?}",
            report.sources_registered
        );
        assert!(!lib_dir.path().join("lr").exists(), "nothing was copied in");

        // The library now knows two roots: itself, and the LR root folder.
        let sources = lib.sources().unwrap();
        assert_eq!(sources.len(), 2);
        let referenced = sources.iter().find(|s| !s.is_primary).unwrap();
        assert_eq!(referenced.name, "shoot-b", "named after the LR root folder");
        assert_eq!(referenced.kind, crate::sources::SourceKind::External);
        assert_eq!(referenced.photo_count, 2);
        assert!(referenced.online);

        // …and b1's row points at the file where it always was.
        let photos = lib.photos(&PhotoFilter::default()).unwrap();
        let b1 = photo_by_name(&photos, "b1.jpg");
        assert_eq!(b1.source_id, referenced.id);
        assert_eq!(b1.rel_path, "b1.jpg", "relative to its own source, not the library");
        assert_eq!(
            lib.photo_path(b1).unwrap(),
            elsewhere.path().canonicalize().unwrap().join("shoot-b/b1.jpg")
        );
        // Everything else about the import is unchanged: the inside root is
        // still in place, and the collections still became albums.
        assert_eq!(photo_by_name(&photos, "a3.jpg").rel_path, "shoot-a/sub/a3.jpg");
        assert_eq!(lib.album_photos("weddings/ana ivan").unwrap().len(), 3);
    }

    /// Re-running a referencing import syncs rather than registering a second
    /// source or re-cataloguing anything.
    #[test]
    fn a_second_referencing_run_adds_no_second_source() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        let opts = LrImportOptions {
            mode: LrPlacement::Reference,
            ..Default::default()
        };
        lr_import(&lib, &lrcat, &opts, None, None).unwrap();

        let again = lr_import(&lib, &lrcat, &opts, None, None).unwrap();
        assert_eq!(again.photos_in_place, 0);
        assert_eq!(again.photos_referenced, 0);
        assert_eq!(again.photos_copied, 0);
        assert!(
            again.sources_registered.is_empty(),
            "the root is already a source: {:?}",
            again.sources_registered
        );
        assert_eq!(lib.sources().unwrap().len(), 2);
        assert_eq!(lib.photo_count().unwrap(), 5);
    }

    /// A dry run in reference mode forecasts the source and registers nothing.
    #[test]
    fn a_dry_referencing_run_registers_no_source() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                mode: LrPlacement::Reference,
                dry_run: true,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(report.photos_referenced, 2);
        assert_eq!(report.photos_copied, 0);
        assert!(report.sources_registered.iter().any(|s| s.contains("shoot-b")));
        assert_eq!(lib.sources().unwrap().len(), 1, "a dry run registered a source");
        assert_eq!(lib.photo_count().unwrap(), 0);
    }

    /// The choice is per root folder: this year's shoot copied onto the laptop,
    /// eight years on the NAS referenced where they are.
    #[test]
    fn a_per_root_placement_overrides_the_runs_default() {
        let (lib, lib_dir, _elsewhere, lrcat) = fixture();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                mode: LrPlacement::Reference,
                // Root 2 is shoot-b, the outside one — copy that one after all.
                roots: Some(vec![LrRootPlacement {
                    root_id: 2,
                    mode: LrPlacement::Copy,
                }]),
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(report.photos_copied, 2, "the override won");
        assert_eq!(report.photos_referenced, 0);
        assert!(report.sources_registered.is_empty());
        assert!(lib_dir.path().join("lr/shoot-b/b1.jpg").exists());
        assert_eq!(lib.sources().unwrap().len(), 1);
    }

    /// Asking for in-place on a root that is outside the library can only be
    /// honoured by referencing it — a lasting change to the library — so it is
    /// done, and said out loud.
    #[test]
    fn in_place_on_an_outside_root_references_it_and_says_so() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                mode: LrPlacement::InPlace,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(report.photos_copied, 0);
        assert_eq!(report.photos_referenced, 2);
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.contains("shoot-b") && c.contains("registered")),
            "the new source has to be named: {:?}",
            report.conflicts
        );
    }

    /// A root that cannot become a source — it holds the library itself — falls
    /// back to copying rather than losing the photographs, and says why.
    #[test]
    fn a_root_that_cannot_be_referenced_copies_instead_and_names_the_reason() {
        // The library lives *inside* the LR root folder, so registering that
        // root would make one source contain another.
        let outer = tempfile::tempdir().unwrap();
        let root_b = outer.path().join("shoot-b");
        let lib_dir = outer.path().join("gallery");
        let root_a = lib_dir.join("shoot-a");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let lrcat = outer.path().join("photos.lrcat");
        // The catalog's second root is the folder that contains the library.
        standard_fixture(&lrcat, &root_a, outer.path());
        let lib = Library::open(&lib_dir).unwrap();

        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                mode: LrPlacement::Reference,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(lib.sources().unwrap().len(), 1, "no source was registered");
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.contains("could not be referenced")),
            "the fallback has to explain itself: {:?}",
            report.conflicts
        );
        assert!(report.photos_copied > 0, "the photographs still got in");
    }

    #[test]
    fn a_second_run_syncs_instead_of_duplicating() {
        let (lib, lib_dir, _elsewhere, lrcat) = fixture();
        lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();
        let count = lib.photo_count().unwrap();
        let albums = lib.albums().unwrap().len();

        // A hand-set rating between the runs must survive the second one.
        let photos = lib.photos(&PhotoFilter::default()).unwrap();
        let a1 = photo_by_name(&photos, "a1.jpg");
        lib.set_rating(a1.id, 1).unwrap();

        let again = lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();
        assert_eq!(again.photos_copied, 0);
        assert_eq!(again.photos_in_place, 0);
        assert_eq!(again.photos_linked_existing, 0);
        assert_eq!(again.albums_created, 0);
        assert_eq!(again.memberships_added, 0);
        assert_eq!(again.tags_added, 0);
        assert_eq!(lib.photo_count().unwrap(), count);
        assert_eq!(lib.albums().unwrap().len(), albums);
        assert!(!lib_dir.path().join("lr-2").exists(), "no second copy tree");

        assert_eq!(
            lib.photo_by_id(a1.id).unwrap().rating,
            1,
            "the hand-set rating was clobbered by Lightroom's 3"
        );
    }

    #[test]
    fn bytes_already_in_the_catalog_are_linked_not_copied_again() {
        let (lib, lib_dir, elsewhere, lrcat) = fixture();
        // The same photograph b1.jpg already lives in the library elsewhere.
        std::fs::create_dir_all(lib_dir.path().join("older")).unwrap();
        std::fs::copy(
            elsewhere.path().join("shoot-b/b1.jpg"),
            lib_dir.path().join("older/duplicate.jpg"),
        )
        .unwrap();
        crate::import::import_dir(
            &lib,
            lib_dir.path().join("older").as_path(),
            &crate::import::ImportOptions::default(),
            None,
            None,
        )
        .unwrap();

        let report =
            lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();
        assert_eq!(report.photos_linked_existing, 1, "b1's bytes were already here");
        assert_eq!(report.photos_copied, 1, "only b2 still needed copying");
        assert!(!lib_dir.path().join("lr/shoot-b/b1.jpg").exists());

        // And the existing row is the one in the album, carrying LR's keywords.
        let dup = lib.photo_by_rel_path("older/duplicate.jpg").unwrap().unwrap();
        assert_eq!(lib.photo_tags(dup.id).unwrap(), vec!["wedding"]);
        assert!(lib
            .album_photos("weddings/ana ivan")
            .unwrap()
            .iter()
            .any(|p| p.id == dup.id));
    }

    #[test]
    fn a_collection_renamed_in_lightroom_renames_its_album_here() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();

        {
            let conn = Connection::open(&lrcat).unwrap();
            conn.execute(
                "UPDATE AgLibraryCollection SET name = 'Ana and Ivan' WHERE id_local = 2",
                [],
            )
            .unwrap();
        }
        let report =
            lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
        assert!(lib.album_by_path("weddings/ana ivan").unwrap().is_none());
        let moved = lib.album_by_path("weddings/ana and ivan").unwrap().unwrap();
        assert_eq!(
            lib.album_photos(&moved.path).unwrap().len(),
            3,
            "the members came along with the rename"
        );
    }

    #[test]
    fn renamed_on_both_sides_is_a_reported_conflict_and_nothing_moves() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();

        // Renamed here...
        lib.move_album("weddings/ana ivan", "weddings/our-pick").unwrap();
        // ...and renamed in Lightroom.
        {
            let conn = Connection::open(&lrcat).unwrap();
            conn.execute(
                "UPDATE AgLibraryCollection SET name = 'Ana and Ivan' WHERE id_local = 2",
                [],
            )
            .unwrap();
        }

        let report =
            lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();
        assert!(
            report.conflicts.iter().any(|c| c.contains("Ana and Ivan")),
            "the conflict must name the collection: {:?}",
            report.conflicts
        );
        assert!(lib.album_by_path("weddings/our-pick").unwrap().is_some(), "local name kept");
        assert!(lib.album_by_path("weddings/ana and ivan").unwrap().is_none(), "nothing created");
    }

    #[test]
    fn collision_policies_merge_or_suffix_an_existing_album() {
        // Suffix: the existing album is left alone and a `-lr` sibling holds
        // the imported members.
        let (lib, _l, _e, lrcat) = fixture();
        lib.create_album(&NewAlbum {
            path: "weddings/ana ivan".into(),
            title: Some("Mine".into()),
            ..Default::default()
        })
        .unwrap();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                collision: MergePolicy::Suffix,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        assert!(report.collisions.iter().any(|c| c.contains("-lr")), "{:?}", report.collisions);
        assert_eq!(lib.album_photos("weddings/ana ivan").unwrap().len(), 0, "untouched");
        assert_eq!(lib.album_photos("weddings/ana ivan-lr").unwrap().len(), 3);
        assert_eq!(
            lib.album_by_path("weddings/ana ivan").unwrap().unwrap().title,
            "Mine"
        );

        // Merge: the members land in the existing album.
        let (lib, _l, _e, lrcat) = fixture();
        lib.create_album(&NewAlbum {
            path: "weddings/ana ivan".into(),
            title: Some("Mine".into()),
            ..Default::default()
        })
        .unwrap();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                collision: MergePolicy::Merge,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
        assert!(!report.collisions.is_empty());
        assert_eq!(lib.album_photos("weddings/ana ivan").unwrap().len(), 3);
        assert_eq!(
            lib.album_by_path("weddings/ana ivan").unwrap().unwrap().title,
            "Mine",
            "merging adds members, it does not rename"
        );
    }

    /// The Auto policy merges into a hand-made album but refuses one that a
    /// different catalog's import owns.
    #[test]
    fn the_auto_policy_suffixes_only_against_another_catalogs_album() {
        let (lib, _l, _e, lrcat) = fixture();
        lib.create_album(&NewAlbum {
            path: "weddings/ana ivan".into(),
            ..Default::default()
        })
        .unwrap();
        // Pretend a different lrcat already linked that album.
        lib.set_lr_album_link("some-other-catalog", 99, "weddings/ana ivan").unwrap();

        let report =
            lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();
        assert!(report.collisions.iter().any(|c| c.contains("-lr")), "{:?}", report.collisions);
        assert_eq!(lib.album_photos("weddings/ana ivan-lr").unwrap().len(), 3);
    }

    #[test]
    fn a_dry_run_reports_the_work_and_writes_none_of_it() {
        let (lib, lib_dir, _elsewhere, lrcat) = fixture();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                dry_run: true,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert!(report.dry_run);
        assert_eq!(report.photos_in_place, 3);
        assert_eq!(report.photos_copied, 2);
        assert_eq!(report.albums_created, 3);
        assert_eq!(report.memberships_added, 4);
        assert!(report.bytes_copied > 0);

        assert_eq!(lib.photo_count().unwrap(), 0, "a dry run catalogued something");
        assert!(lib.albums().unwrap().is_empty(), "a dry run created an album");
        assert!(!lib_dir.path().join("lr").exists(), "a dry run copied files");
        assert!(lib.lr_photo_links(&scan(&lib, &lrcat).unwrap().catalog_id).unwrap().is_empty());
    }

    #[test]
    fn only_the_asked_for_collections_are_mapped_but_photos_still_import() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        let report = lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                collections: Some(vec!["weddings/*".into()]),
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert_eq!(report.photos_in_place + report.photos_copied, 5, "photos are not filtered");
        assert!(lib.album_by_path("weddings/ana ivan").unwrap().is_some());
        assert!(lib.album_by_path("weddings").unwrap().is_some(), "the set came as its parent");
        assert!(
            lib.album_by_path("best of 2026").unwrap().is_none(),
            "the smart collection was not asked for"
        );
    }

    #[test]
    fn a_prefix_and_dest_subdir_steer_where_things_land() {
        let (lib, lib_dir, _elsewhere, lrcat) = fixture();
        lr_import(
            &lib,
            &lrcat,
            &LrImportOptions {
                dest_subdir: Some("from-lightroom".into()),
                album_prefix: Some("Lightroom".into()),
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();

        assert!(lib_dir.path().join("from-lightroom/shoot-b/b1.jpg").exists());
        assert!(!lib_dir.path().join("lr").exists());
        assert!(lib.album_by_path("lightroom/weddings/ana ivan").unwrap().is_some());
        assert!(lib.album_by_path("weddings/ana ivan").unwrap().is_none());
    }

    #[test]
    fn lightroom_side_deletions_are_reported_never_propagated() {
        let (lib, _lib_dir, _elsewhere, lrcat) = fixture();
        lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();

        {
            let conn = Connection::open(&lrcat).unwrap();
            conn.execute("DELETE FROM Adobe_images WHERE id_local = 1000", []).unwrap();
            conn.execute("DELETE FROM AgLibraryCollection WHERE id_local = 3", []).unwrap();
        }
        let report =
            lr_import(&lib, &lrcat, &LrImportOptions::default(), None, None).unwrap();

        assert!(
            report.lr_deleted.iter().any(|d| d.contains("a1.jpg")),
            "the deleted image must be named: {:?}",
            report.lr_deleted
        );
        assert!(report.lr_deleted.iter().any(|d| d.contains("best of 2026")));
        assert!(lib.photo_by_rel_path("shoot-a/a1.jpg").unwrap().is_some(), "kept");
        assert!(lib.album_by_path("best of 2026").unwrap().is_some(), "kept");
    }

    #[test]
    fn glob_patterns_match_the_way_the_help_says() {
        assert!(matches_pattern("weddings/ana", "weddings/ana"));
        assert!(matches_pattern("Weddings/Ana", "weddings/ana"), "case-blind");
        assert!(matches_pattern("weddings/ana", "weddings/*"));
        assert!(matches_pattern("weddings/ana", "*ana"));
        assert!(matches_pattern("abab", "*ab"), "a lazy matcher gets this wrong");
        assert!(!matches_pattern("weddings/ana", "events/*"));
        assert!(!matches_pattern("weddings", "weddings/*"));
    }
}
