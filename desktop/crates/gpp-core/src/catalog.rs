//! The SQLite catalog and the [`Library`] handle.
//!
//! The filesystem stays the source of truth for pixels; this database is a
//! derived index. Losing it is an inconvenience (re-import rebuilds it), never
//! data loss — that property is deliberate.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Error, Result};
use crate::model::{Album, Flag, Photo, PhotoFilter, PhotoKind, PhotoSort};

const SCHEMA_VERSION: i64 = 3;

/// Directory (relative to the library root) holding all derived data.
pub const GPP_DIR: &str = ".gpp";

/// An open photo library: a root folder plus its catalog.
pub struct Library {
    root: PathBuf,
    conn: Mutex<Connection>,
}

impl Library {
    /// Open (creating if needed) the library rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let gpp = root.join(GPP_DIR);
        std::fs::create_dir_all(&gpp).map_err(|e| Error::io(&gpp, e))?;

        let conn = Connection::open(gpp.join("catalog.db"))?;
        Self::configure(&conn)?;
        migrate(&conn)?;

        Ok(Self {
            root,
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory catalog rooted at `root` — used by tests.
    pub fn open_in_memory(root: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        migrate(&conn)?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
            conn: Mutex::new(conn),
        })
    }

    fn configure(conn: &Connection) -> Result<()> {
        // WAL keeps readers unblocked while the import writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // A library is meant to be reachable from more than one program at a
        // time — the desktop app open while the CLI publishes. WAL lets
        // readers through; two writers still meet, and the second must wait
        // rather than fail with "database is locked". rusqlite already
        // defaults to waiting 5s; this states the intent and pins it, since
        // bare SQLite's own default is to fail instantly.
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute path of the derived-data directory.
    pub fn gpp_dir(&self) -> PathBuf {
        self.root.join(GPP_DIR)
    }

    /// Where content-addressed thumbnails live.
    pub fn thumb_dir(&self) -> PathBuf {
        self.gpp_dir().join("thumbs")
    }

    /// Resolve a library-relative path, refusing anything that escapes the root.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        if rel.is_empty() {
            return Ok(self.root.clone());
        }
        if rel.contains('\0') || rel.starts_with('/') || rel.starts_with('\\') {
            return Err(Error::InvalidPath(rel.to_string()));
        }
        let mut out = self.root.clone();
        for segment in rel.split('/') {
            match segment {
                "" | "." => continue,
                ".." => return Err(Error::InvalidPath(rel.to_string())),
                s => out.push(s),
            }
        }
        if !out.starts_with(&self.root) {
            return Err(Error::InvalidPath(rel.to_string()));
        }
        Ok(out)
    }

    /// Run a closure with the connection. Kept crate-visible so other modules
    /// compose queries without exposing SQLite to the shell.
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("catalog mutex poisoned");
        f(&conn)
    }

    /// Run a closure inside a transaction.
    pub(crate) fn with_tx<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut conn = self.conn.lock().expect("catalog mutex poisoned");
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    // ---------------------------------------------------------------- photos

    pub fn photo_count(&self) -> Result<i64> {
        self.with_conn(|c| {
            Ok(c.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))?)
        })
    }

    pub fn photo_by_id(&self, id: i64) -> Result<Photo> {
        self.with_conn(|c| {
            c.query_row(
                &format!("SELECT {PHOTO_COLS} FROM photos WHERE id = ?1"),
                params![id],
                photo_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::PhotoNotFound(id.to_string()))
        })
    }

    pub fn photo_by_rel_path(&self, rel_path: &str) -> Result<Option<Photo>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {PHOTO_COLS} FROM photos WHERE rel_path = ?1"),
                params![rel_path],
                photo_from_row,
            )
            .optional()?)
        })
    }

    /// The main grid query.
    pub fn photos(&self, filter: &PhotoFilter) -> Result<Vec<Photo>> {
        let mut sql = format!("SELECT {} FROM photos p", prefixed_photo_cols("p"));
        let mut wheres: Vec<String> = Vec::new();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(album) = &filter.album_path {
            sql.push_str(
                " JOIN album_photos ap ON ap.photo_id = p.id \
                  JOIN albums a ON a.id = ap.album_id",
            );
            wheres.push("a.path = ?".into());
            args.push(Box::new(album.clone()));
        }
        if let Some(min) = filter.min_rating {
            wheres.push("p.rating >= ?".into());
            args.push(Box::new(min as i64));
        }
        if let Some(flag) = filter.flag {
            wheres.push("p.flag = ?".into());
            args.push(Box::new(flag.as_str().to_string()));
        }
        if let Some(label) = &filter.color_label {
            wheres.push("p.color_label = ?".into());
            args.push(Box::new(label.clone()));
        }
        if let Some(kind) = filter.kind {
            wheres.push("p.kind = ?".into());
            args.push(Box::new(kind.as_str().to_string()));
        }
        if let Some(model) = &filter.camera_model {
            wheres.push("p.camera_model = ?".into());
            args.push(Box::new(model.clone()));
        }
        if let Some(text) = &filter.text {
            let like = format!("%{}%", text.to_lowercase());
            wheres.push(
                "(LOWER(p.filename) LIKE ? OR LOWER(IFNULL(p.camera_make,'') || ' ' \
                 || IFNULL(p.camera_model,'')) LIKE ?)"
                    .into(),
            );
            args.push(Box::new(like.clone()));
            args.push(Box::new(like));
        }
        if let Some(from) = &filter.captured_from {
            wheres.push("p.captured_at >= ?".into());
            args.push(Box::new(from.clone()));
        }
        if let Some(to) = &filter.captured_to {
            // Inclusive on the whole day when a bare date is given.
            wheres.push("p.captured_at <= ?".into());
            args.push(Box::new(format!("{to}T23:59:59Z")));
        }

        if !wheres.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&wheres.join(" AND "));
        }

        sql.push_str(match filter.sort {
            PhotoSort::CapturedAsc => " ORDER BY p.captured_at IS NULL, p.captured_at ASC, p.filename ASC",
            PhotoSort::CapturedDesc => " ORDER BY p.captured_at IS NULL, p.captured_at DESC, p.filename ASC",
            PhotoSort::NameAsc => " ORDER BY p.filename ASC",
            PhotoSort::NameDesc => " ORDER BY p.filename DESC",
            PhotoSort::RatingDesc => " ORDER BY p.rating DESC, p.filename ASC",
            PhotoSort::AlbumOrder => " ORDER BY ap.position ASC, p.filename ASC",
        });

        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
            if let Some(offset) = filter.offset {
                sql.push_str(&format!(" OFFSET {offset}"));
            }
        }

        self.with_conn(|c| {
            let mut stmt = c.prepare(&sql)?;
            let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
            let rows = stmt.query_map(refs.as_slice(), photo_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Set the star rating (clamped to 0..=5).
    pub fn set_rating(&self, photo_id: i64, rating: u8) -> Result<()> {
        let rating = rating.min(5);
        let changed = self.with_conn(|c| {
            Ok(c.execute(
                "UPDATE photos SET rating = ?1 WHERE id = ?2",
                params![rating as i64, photo_id],
            )?)
        })?;
        if changed == 0 {
            return Err(Error::PhotoNotFound(photo_id.to_string()));
        }
        Ok(())
    }

    pub fn set_flag(&self, photo_id: i64, flag: Flag) -> Result<()> {
        let changed = self.with_conn(|c| {
            Ok(c.execute(
                "UPDATE photos SET flag = ?1 WHERE id = ?2",
                params![flag.as_str(), photo_id],
            )?)
        })?;
        if changed == 0 {
            return Err(Error::PhotoNotFound(photo_id.to_string()));
        }
        Ok(())
    }

    pub fn set_color_label(&self, photo_id: i64, label: Option<&str>) -> Result<()> {
        let changed = self.with_conn(|c| {
            Ok(c.execute(
                "UPDATE photos SET color_label = ?1 WHERE id = ?2",
                params![label, photo_id],
            )?)
        })?;
        if changed == 0 {
            return Err(Error::PhotoNotFound(photo_id.to_string()));
        }
        Ok(())
    }

    /// Bulk rating — one transaction for the whole selection.
    pub fn set_rating_bulk(&self, photo_ids: &[i64], rating: u8) -> Result<usize> {
        let rating = rating.min(5);
        self.with_tx(|tx| {
            let mut stmt = tx.prepare("UPDATE photos SET rating = ?1 WHERE id = ?2")?;
            let mut n = 0;
            for id in photo_ids {
                n += stmt.execute(params![rating as i64, id])?;
            }
            Ok(n)
        })
    }

    pub fn set_flag_bulk(&self, photo_ids: &[i64], flag: Flag) -> Result<usize> {
        self.with_tx(|tx| {
            let mut stmt = tx.prepare("UPDATE photos SET flag = ?1 WHERE id = ?2")?;
            let mut n = 0;
            for id in photo_ids {
                n += stmt.execute(params![flag.as_str(), id])?;
            }
            Ok(n)
        })
    }

    /// Remove catalog entries whose files no longer exist on disk.
    pub fn prune_missing(&self) -> Result<usize> {
        let all: Vec<(i64, String)> = self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, rel_path FROM photos")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })?;

        let missing: Vec<i64> = all
            .into_iter()
            .filter(|(_, rel)| match self.resolve(rel) {
                Ok(abs) => !abs.exists(),
                Err(_) => true,
            })
            .map(|(id, _)| id)
            .collect();

        if missing.is_empty() {
            return Ok(0);
        }
        self.with_tx(|tx| {
            let mut stmt = tx.prepare("DELETE FROM photos WHERE id = ?1")?;
            let mut n = 0;
            for id in &missing {
                n += stmt.execute(params![id])?;
            }
            Ok(n)
        })
    }

    /// Distinct camera models present in the library (for filter UI).
    pub fn camera_models(&self) -> Result<Vec<String>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT DISTINCT camera_model FROM photos \
                 WHERE camera_model IS NOT NULL AND camera_model <> '' \
                 ORDER BY camera_model",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    // -------------------------------------------------------------- settings

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO settings(key, value) VALUES(?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }
}

// -------------------------------------------------------------------- schema

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);",
    )?;
    let current: Option<i64> = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
            r.get(0)
        })
        .optional()?;

    match current {
        None => {
            // Fresh catalog: create at the current version directly.
            conn.execute_batch(SCHEMA_V1)?;
            conn.execute_batch(SCHEMA_V2)?;
            conn.execute_batch(SCHEMA_V3)?;
            conn.execute(
                "INSERT INTO schema_version(version) VALUES(?1)",
                params![SCHEMA_VERSION],
            )?;
        }
        Some(v) => {
            // Stepped: a catalog two versions behind runs both migrations.
            if v < 2 {
                conn.execute_batch(SCHEMA_V2)?;
            }
            if v < 3 {
                conn.execute_batch(SCHEMA_V3)?;
            }
            if v < SCHEMA_VERSION {
                conn.execute(
                    "UPDATE schema_version SET version = ?1",
                    params![SCHEMA_VERSION],
                )?;
            }
        }
    }
    // Further migrations step forward from here. The catalog is a rebuildable
    // index, so a failed migration is recoverable by re-importing.
    Ok(())
}

const SCHEMA_V1: &str = r#"
CREATE TABLE photos (
  id            INTEGER PRIMARY KEY,
  rel_path      TEXT    NOT NULL UNIQUE,
  filename      TEXT    NOT NULL,
  content_hash  TEXT    NOT NULL,
  file_size     INTEGER NOT NULL,
  mtime_ms      INTEGER NOT NULL,
  kind          TEXT    NOT NULL DEFAULT 'photo',
  width         INTEGER,
  height        INTEGER,
  orientation   INTEGER,
  captured_at   TEXT,
  camera_make   TEXT,
  camera_model  TEXT,
  lens          TEXT,
  iso           INTEGER,
  aperture      REAL,
  shutter       REAL,
  focal_length  REAL,
  rating        INTEGER NOT NULL DEFAULT 0,
  flag          TEXT    NOT NULL DEFAULT 'none',
  color_label   TEXT,
  blur_lqip     TEXT,
  imported_at   TEXT    NOT NULL
);
CREATE INDEX idx_photos_hash     ON photos(content_hash);
CREATE INDEX idx_photos_rating   ON photos(rating);
CREATE INDEX idx_photos_captured ON photos(captured_at);
CREATE INDEX idx_photos_camera   ON photos(camera_model);
CREATE INDEX idx_photos_kind     ON photos(kind);

CREATE TABLE albums (
  id             INTEGER PRIMARY KEY,
  path           TEXT    NOT NULL UNIQUE,
  parent_path    TEXT,
  title          TEXT    NOT NULL,
  description    TEXT,
  date           TEXT,
  token          TEXT    NOT NULL,
  password       TEXT,
  share_token    TEXT,
  sort           TEXT    NOT NULL DEFAULT 'date-desc',
  style          TEXT    NOT NULL DEFAULT 'single-column',
  cover_photo_id INTEGER REFERENCES photos(id) ON DELETE SET NULL,
  is_collection  INTEGER NOT NULL DEFAULT 0,
  hidden         INTEGER NOT NULL DEFAULT 0,
  allow_download INTEGER NOT NULL DEFAULT 0,
  proofing       INTEGER NOT NULL DEFAULT 0,
  sort_order     INTEGER,
  body           TEXT
);
CREATE INDEX idx_albums_parent ON albums(parent_path);

CREATE TABLE album_photos (
  album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  position INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(album_id, photo_id)
);
CREATE INDEX idx_album_photos_photo ON album_photos(photo_id);

CREATE TABLE tags (
  id   INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);
CREATE TABLE photo_tags (
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(id)   ON DELETE CASCADE,
  PRIMARY KEY(photo_id, tag_id)
);
CREATE TABLE album_tags (
  album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(id)   ON DELETE CASCADE,
  PRIMARY KEY(album_id, tag_id)
);

-- Reserved for the develop phase: an ordered, non-destructive operation list.
CREATE TABLE edits (
  photo_id   INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
  version    INTEGER NOT NULL DEFAULT 1,
  stack_json TEXT    NOT NULL
);

CREATE TABLE sync_state (
  entity_kind    TEXT NOT NULL,
  entity_key     TEXT NOT NULL,
  local_hash     TEXT,
  synced_hash    TEXT,
  remote_hash    TEXT,
  last_synced_at TEXT,
  PRIMARY KEY(entity_kind, entity_key)
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

/// v2 — per-album sync subscriptions.
///
/// A machine only touches albums listed here. An album absent from this table
/// is invisible to sync: never pushed, never pulled, never deleted. That is
/// what lets one machine hold three albums out of two hundred safely.
const SCHEMA_V2: &str = r#"
CREATE TABLE album_sync (
  album_path     TEXT PRIMARY KEY,
  direction      TEXT NOT NULL DEFAULT 'both',   -- push | pull | both
  last_synced_at TEXT
);
"#;

const SCHEMA_V3: &str = r#"
-- What this library last wrote into the published tree, per album.
--
-- Publishing prunes a file only if it appears here: that is how a photo removed
-- from an album disappears from the gallery, while anything another tool put in
-- the same folder is left strictly alone.
CREATE TABLE published_files (
  album_path TEXT NOT NULL,
  filename   TEXT NOT NULL,
  PRIMARY KEY (album_path, filename)
);
"#;

// ------------------------------------------------------------------ row glue

const PHOTO_COLS: &str = "id, rel_path, filename, content_hash, file_size, mtime_ms, kind, \
     width, height, orientation, captured_at, camera_make, camera_model, lens, iso, \
     aperture, shutter, focal_length, rating, flag, color_label, blur_lqip, imported_at";

fn prefixed_photo_cols(prefix: &str) -> String {
    PHOTO_COLS
        .split(',')
        .map(|c| format!("{prefix}.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn photo_from_row(row: &Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: row.get(0)?,
        rel_path: row.get(1)?,
        filename: row.get(2)?,
        content_hash: row.get(3)?,
        file_size: row.get(4)?,
        mtime_ms: row.get(5)?,
        kind: PhotoKind::parse(&row.get::<_, String>(6)?),
        width: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
        height: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
        orientation: row.get::<_, Option<i64>>(9)?.map(|v| v as u16),
        captured_at: row.get(10)?,
        camera_make: row.get(11)?,
        camera_model: row.get(12)?,
        lens: row.get(13)?,
        iso: row.get(14)?,
        aperture: row.get(15)?,
        shutter: row.get(16)?,
        focal_length: row.get(17)?,
        rating: row.get::<_, i64>(18)? as u8,
        flag: Flag::parse(&row.get::<_, String>(19)?),
        color_label: row.get(20)?,
        blur_lqip: row.get(21)?,
        imported_at: row.get(22)?,
    })
}

/// Album row mapping lives here so `albums.rs` stays about behaviour.
pub(crate) const ALBUM_COLS: &str =
    "id, path, parent_path, title, description, date, token, password, share_token, \
     sort, style, cover_photo_id, is_collection, hidden, allow_download, proofing, \
     sort_order, body";

pub(crate) fn album_from_row(row: &Row<'_>) -> rusqlite::Result<(Album, Option<i64>)> {
    let cover_photo_id: Option<i64> = row.get(11)?;
    let album = Album {
        id: row.get(0)?,
        path: row.get(1)?,
        parent_path: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        date: row.get(5)?,
        token: row.get(6)?,
        password: row.get(7)?,
        share_token: row.get(8)?,
        sort: row.get(9)?,
        style: row.get(10)?,
        cover_filename: None, // resolved by albums::hydrate_cover
        is_collection: row.get::<_, i64>(12)? != 0,
        hidden: row.get::<_, i64>(13)? != 0,
        allow_download: row.get::<_, i64>(14)? != 0,
        proofing: row.get::<_, i64>(15)? != 0,
        sort_order: row.get(16)?,
        tags: Vec::new(), // filled by albums::hydrate_tags
        body: row.get(17)?,
    };
    Ok((album, cover_photo_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        assert_eq!(lib.photo_count().unwrap(), 0);
    }

    /// Two programs may hold the same library — the app open while the CLI
    /// publishes. WAL lets readers through; a second writer has to wait rather
    /// than fail instantly with "database is locked". This locks that in: it
    /// is currently true because of a default, and a default is not a promise.
    #[test]
    fn a_second_writer_waits_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let first = Library::open(dir.path()).unwrap();
        let second = Library::open(dir.path()).unwrap();

        let holding = std::sync::Arc::new(std::sync::Barrier::new(2));
        let done = std::sync::Arc::new(std::sync::Barrier::new(2));

        let writer = {
            let (holding, done) = (holding.clone(), done.clone());
            std::thread::spawn(move || {
                first
                    .with_tx(|tx| {
                        // Take the write lock, then hold it while the other
                        // side tries.
                        tx.execute("INSERT INTO settings(key, value) VALUES('a','1')", [])?;
                        holding.wait();
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        Ok(())
                    })
                    .unwrap();
                done.wait();
            })
        };

        holding.wait();
        // Without a busy timeout this returns SQLITE_BUSY immediately.
        second
            .with_tx(|tx| {
                tx.execute("INSERT INTO settings(key, value) VALUES('b','2')", [])?;
                Ok(())
            })
            .expect("second writer should wait for the lock, not fail");
        done.wait();
        writer.join().unwrap();
    }

    #[test]
    fn resolve_rejects_traversal() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        assert!(lib.resolve("a/b.jpg").is_ok());
        assert!(lib.resolve("../etc/passwd").is_err());
        assert!(lib.resolve("a/../../etc").is_err());
        assert!(lib.resolve("/etc/passwd").is_err());
        assert!(lib.resolve("a\0b").is_err());
    }

    #[test]
    fn settings_round_trip() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        assert_eq!(lib.get_setting("remote").unwrap(), None);
        lib.set_setting("remote", "example.com").unwrap();
        assert_eq!(
            lib.get_setting("remote").unwrap().as_deref(),
            Some("example.com")
        );
        lib.set_setting("remote", "other.com").unwrap();
        assert_eq!(
            lib.get_setting("remote").unwrap().as_deref(),
            Some("other.com")
        );
    }
}
