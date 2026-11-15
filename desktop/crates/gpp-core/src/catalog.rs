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

const SCHEMA_VERSION: i64 = 6;

/// The schema version, visible to the crate — `remotes::read_library_remotes`
/// refuses a foreign catalog from a newer build by name, same as [`migrate`].
pub(crate) const SCHEMA_VERSION_PUBLIC: i64 = SCHEMA_VERSION;

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

        // One spelling of the root, decided once. Import both builds
        // destinations under a canonicalized root (`bring_inside`) and strips
        // the stored root off scanned paths — with two spellings in play, a
        // root reached through a symlink (`/var`, `/tmp` on macOS) copied a
        // whole card in and then catalogued none of it, failing with
        // InvalidPath. The rel_paths in the catalog are unaffected: they are
        // relative, and both spellings name the same files.
        let root = root.canonicalize().unwrap_or(root);

        let conn = Connection::open(gpp.join("catalog.db"))?;
        Self::configure(&conn)?;
        migrate(&conn, &root)?;
        ensure_library_id(&conn)?;
        refresh_primary_source(&conn, &root)?;

        Ok(Self {
            root,
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory catalog rooted at `root` — used by tests.
    pub fn open_in_memory(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        migrate(&conn, &root)?;
        ensure_library_id(&conn)?;
        refresh_primary_source(&conn, &root)?;
        Ok(Self {
            root,
            conn: Mutex::new(conn),
        })
    }

    /// This library's stable identity: 16 random bytes, hex, generated once at
    /// the first open on a v5 catalog and never changed. Outcomes and manifests
    /// carry it so an aggregator (the future master catalog) can attribute
    /// state to a library without guessing from paths.
    pub fn library_id(&self) -> Result<String> {
        Ok(self
            .get_setting(SETTING_LIBRARY_ID)?
            .unwrap_or_default())
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

    /// Absolute path of the library root — the folder the photographer chose.
    ///
    /// Every `rel_path` in the catalog is relative to this, and nothing in the
    /// core ever addresses a photo outside it: that is the whole reason
    /// importing a folder from elsewhere copies it in first.
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

    /// Resolve a path relative to the **primary** source — the library root.
    ///
    /// This is where albums live: a pulled album's folder, a copy-in
    /// destination, anything the library itself lays out. A *photo* is not
    /// necessarily here — since schema v6 a catalog row carries the source it
    /// belongs to — so resolve one with [`photo_path`](Self::photo_path)
    /// rather than by handing its `rel_path` to this.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        resolve_under(&self.root, rel)
    }

    /// Resolve a path relative to one registered source.
    ///
    /// Pure path arithmetic plus the same escape refusal `resolve` applies —
    /// a `rel_path` may not climb out of *its own* source — and it answers
    /// whether the source is plugged in or not, because a path is still a
    /// path. Use [`photo_path`](Self::photo_path) when the answer is going to
    /// be opened.
    pub fn resolve_in_source(&self, source_id: i64, rel: &str) -> Result<PathBuf> {
        resolve_under(&self.source_root(source_id)?, rel)
    }

    /// Absolute path of one catalogued photo, through the source it belongs to.
    ///
    /// **The single accessor**: every part of the app that opens, copies,
    /// renders or publishes a photograph goes through this, so a referenced
    /// file on an external drive is reached exactly the way a file under the
    /// library root is. A source that is not currently reachable fails with
    /// [`Error::SourceOffline`] rather than handing back a path that is
    /// missing for a reason nobody can see.
    pub fn photo_path(&self, photo: &Photo) -> Result<PathBuf> {
        let source = self.source_row(photo.source_id)?;
        if !source.is_online() {
            return Err(Error::SourceOffline {
                name: source.name,
                path: source.path.display().to_string(),
            });
        }
        resolve_under(&source.path, &photo.rel_path)
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

    /// Every photo in the catalog, unfiltered — including the ones no album
    /// holds and the ones whose files have since vanished from disk. It counts
    /// what the index believes, which is why it can disagree with the folder
    /// until [`prune_missing`](Self::prune_missing) or another import runs.
    pub fn photo_count(&self) -> Result<i64> {
        self.with_conn(|c| {
            Ok(c.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))?)
        })
    }

    /// Fetch one photo by rowid, failing with [`Error::PhotoNotFound`] if the
    /// row is gone.
    ///
    /// An id can only have come out of this catalog, so its absence means the
    /// row was deleted underneath the caller — a stale selection in a grid, a
    /// prune between the click and the call. That is a failure worth reporting,
    /// which is why this errors where
    /// [`photo_by_rel_path`](Self::photo_by_rel_path) returns `None`.
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

    /// Look a photo up by its path under the **primary** source — the library
    /// root. `Ok(None)` is an answer, not a failure: this is how callers ask
    /// whether the library knows a file at all.
    ///
    /// The path must be exactly as stored — '/'-separated, relative to the root,
    /// no normalisation is done here — so a Windows caller holding a `\`-path has
    /// to convert before asking or it will be told, wrongly, that the photograph
    /// is not in the library.
    ///
    /// Scoped to the primary because that is what every caller means: a pulled
    /// album's folder, an album directory, a file this machine laid down. The
    /// same relative path may name a different photograph under another source,
    /// so ask for that one with
    /// [`photo_by_source_rel_path`](Self::photo_by_source_rel_path).
    pub fn photo_by_rel_path(&self, rel_path: &str) -> Result<Option<Photo>> {
        self.photo_by_source_rel_path(self.primary_source_id()?, rel_path)
    }

    /// Look a photo up by source and path — the general form of
    /// [`photo_by_rel_path`](Self::photo_by_rel_path).
    pub fn photo_by_source_rel_path(
        &self,
        source_id: i64,
        rel_path: &str,
    ) -> Result<Option<Photo>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!(
                    "SELECT {PHOTO_COLS} FROM photos WHERE source_id = ?1 AND rel_path = ?2"
                ),
                params![source_id, rel_path],
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

        let joined_an_album = filter.album_path.is_some();
        if let Some(album) = &filter.album_path {
            sql.push_str(
                " JOIN album_photos ap ON ap.photo_id = p.id \
                  JOIN albums a ON a.id = ap.album_id",
            );
            wheres.push("a.path = ?".into());
            args.push(Box::new(album.clone()));
        }
        if let Some(tag) = &filter.tag {
            // A subquery rather than a join: joining `photo_tags` would also
            // multiply rows for a photo carrying several tags.
            wheres.push(
                "p.id IN (SELECT pt.photo_id FROM photo_tags pt \
                 JOIN tags t ON t.id = pt.tag_id WHERE t.name = ?)"
                    .into(),
            );
            args.push(Box::new(tag.clone()));
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
            let like = format!("%{}%", like_literal(&text.to_lowercase()));
            wheres.push(
                "(LOWER(p.filename) LIKE ? ESCAPE '\\' OR LOWER(IFNULL(p.camera_make,'') || ' ' \
                 || IFNULL(p.camera_model,'')) LIKE ? ESCAPE '\\')"
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
            PhotoSort::AlbumOrder if joined_an_album => {
                " ORDER BY ap.position ASC, p.filename ASC"
            }
            // `ap.position` is only in the query when an album was joined in.
            // With no album to hold a position, name order is the answer;
            // naming a column that is not there was not.
            PhotoSort::AlbumOrder => " ORDER BY p.filename ASC",
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

    /// Set the pick/reject flag. Refuses an unknown photo rather than doing
    /// nothing quietly.
    ///
    /// Marking a frame [`Flag::Reject`] touches nothing on disk — culling and
    /// deleting stay separate acts — but it is not free of consequence either:
    /// the next publish leaves that photograph out of the gallery by default.
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

    /// Set or clear the colour label; `None` clears it.
    ///
    /// The string is stored as given — no vocabulary is enforced, so a label
    /// from another tool round-trips intact — which also means `"Red"` and
    /// `"red"` are two different labels and the filter will not match across
    /// them.
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

    /// Flag a whole selection in one transaction.
    ///
    /// Returns how many rows actually changed, which is the only signal that an
    /// id was stale: unlike [`set_flag`](Self::set_flag), the bulk calls do not
    /// fail on a photo that is no longer there. A caller rejecting 400 frames
    /// wants the 399 that exist applied, not the lot refused — but it should
    /// compare the count against `photo_ids.len()` before reporting success.
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
    ///
    /// **An offline source is not a missing file.** A photo on an external
    /// drive that is not plugged in is exactly as present as it was yesterday;
    /// pruning it would throw away its rating, its flags, its album
    /// memberships and its develop stack because a cable was loose. So only
    /// sources that are reachable right now are examined, and a row whose
    /// source cannot be identified at all is left alone too — this deletes,
    /// and a delete needs a reason it is sure of.
    pub fn prune_missing(&self) -> Result<usize> {
        let roots = self.source_roots()?;
        let all: Vec<(i64, i64, String)> = self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, source_id, rel_path FROM photos")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })?;

        let missing: Vec<i64> = all
            .into_iter()
            .filter(|(_, source_id, rel)| {
                let Some(root) = roots.get(source_id) else {
                    return false; // unknown source: not our row to delete
                };
                if !root.is_dir() {
                    return false; // offline: absent from this machine, not gone
                }
                match resolve_under(root, rel) {
                    Ok(abs) => !abs.exists(),
                    Err(_) => true,
                }
            })
            .map(|(id, _, _)| id)
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

    /// Read one setting, or `None` if it was never written.
    ///
    /// Settings live in the catalog, so they belong to the *library* and not to
    /// the machine: carry the drive to another computer and the publish
    /// destination and remote come with it. Keys are dotted by convention —
    /// `publish.dest`, `remote.dir`, `remote.token` — and are not validated.
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

    /// Write a setting, replacing any previous value for the key.
    ///
    /// Stored in plain text in `.gpp/catalog.db`, which matters for one key in
    /// particular: `remote.token` is the sync server's bearer credential, and a
    /// library on a shared drive is a library whose token anyone with the drive
    /// can read. There is no delete — write an empty string, or don't write it.
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

/// Join `rel` onto `root`, refusing anything that escapes it.
///
/// The refusal is the security boundary: `..`, an absolute path, a NUL or a
/// backslash never become a file operation, whichever root they were offered
/// against. Extracted from `Library::resolve` when sources arrived, so that
/// every source enforces the same rule rather than only the library root.
pub(crate) fn resolve_under(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Ok(root.to_path_buf());
    }
    if rel.contains('\0') || rel.starts_with('/') || rel.starts_with('\\') {
        return Err(Error::InvalidPath(rel.to_string()));
    }
    let mut out = root.to_path_buf();
    for segment in rel.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err(Error::InvalidPath(rel.to_string())),
            s => out.push(s),
        }
    }
    if !out.starts_with(root) {
        return Err(Error::InvalidPath(rel.to_string()));
    }
    Ok(out)
}

/// Quote the LIKE wildcards out of a free-text search term.
///
/// `_` matches any single character and `%` any run of them, and `_` is in the
/// filename of nearly every frame a Nikon writes. Pasted into the pattern as
/// typed, a search for the `DSC_0042` off one card also returns the Fuji's
/// `DSCF0042` — a different photograph, sitting at the top of a list the
/// photographer is using to find one frame. Callers must pair this with an
/// `ESCAPE '\'` clause.
fn like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// -------------------------------------------------------------------- schema

fn migrate(conn: &Connection, root: &Path) -> Result<()> {
    // Foreign keys off for the duration, and only for it.
    //
    // v6 rebuilds `photos` — the one table half the schema points at — and
    // with foreign keys on, `DROP TABLE photos` runs an implicit DELETE that
    // cascades every album membership and every edit stack into oblivion
    // before the new table is renamed into place. The pragma is a no-op inside
    // a transaction, so it is set around the whole thing here and restored
    // before anything else can touch the catalog. This is SQLite's own
    // documented procedure for altering a referenced table.
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    let result = migrate_inner(conn, root);
    conn.pragma_update(None, "foreign_keys", "ON")?;
    result
}

fn migrate_inner(conn: &Connection, root: &Path) -> Result<()> {
    // One transaction for the whole thing. SQLite makes DDL transactional, so a
    // catalog either arrives at the new schema or stays exactly where it was.
    // Run step by step in autocommit — which is what this did — a laptop closed
    // between creating a table and writing the version number down reopened,
    // saw the old version, and tried to create that table again. The open
    // failed, and it failed the same way every time after: the photographer's
    // whole library, gone behind "table album_sync already exists".
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);")?;
    let current: Option<i64> = tx
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
            r.get(0)
        })
        .optional()?;

    match current {
        None => {
            // Fresh catalog: create at the current version directly.
            tx.execute_batch(SCHEMA_V1)?;
            tx.execute_batch(SCHEMA_V2)?;
            tx.execute_batch(SCHEMA_V3)?;
            tx.execute_batch(SCHEMA_V4)?;
            apply_v5(&tx)?;
            apply_v6(&tx, root)?;
            tx.execute(
                "INSERT INTO schema_version(version) VALUES(?1)",
                params![SCHEMA_VERSION],
            )?;
        }
        Some(v) if v > SCHEMA_VERSION => {
            // A newer build has already been at this catalog. Two machines
            // share a library over a network share or a carried drive, and one
            // of them is a version behind — that one used to open this happily,
            // select the columns it knows about, and write rows the newer
            // schema's constraints were never applied to. Nothing complains
            // until the up-to-date machine reads it back. Say which build is
            // needed instead; the catalog is safe as long as nobody writes to
            // it with the wrong one.
            return Err(Error::Other(format!(
                "this library's catalog is schema v{v}, and this build understands \
                 v{SCHEMA_VERSION} — open it with a newer version of the app"
            )));
        }
        Some(v) => {
            // Stepped: a catalog two versions behind runs both migrations.
            if v < 2 {
                tx.execute_batch(SCHEMA_V2)?;
            }
            if v < 3 {
                tx.execute_batch(SCHEMA_V3)?;
            }
            if v < 4 {
                tx.execute_batch(SCHEMA_V4)?;
            }
            if v < 5 {
                apply_v5(&tx)?;
            }
            if v < 6 {
                apply_v6(&tx, root)?;
            }
            if v < SCHEMA_VERSION {
                tx.execute(
                    "UPDATE schema_version SET version = ?1",
                    params![SCHEMA_VERSION],
                )?;
            }
        }
    }
    tx.commit()?;
    // Further migrations step forward from here. Every statement in them has to
    // stay idempotent — `IF NOT EXISTS` throughout — so that a catalog already
    // wedged by the old non-atomic version is carried across rather than left
    // permanently unopenable. The catalog is a rebuildable index, but only by a
    // program that can open it first.
    Ok(())
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS photos (
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
CREATE INDEX IF NOT EXISTS idx_photos_hash     ON photos(content_hash);
CREATE INDEX IF NOT EXISTS idx_photos_rating   ON photos(rating);
CREATE INDEX IF NOT EXISTS idx_photos_captured ON photos(captured_at);
CREATE INDEX IF NOT EXISTS idx_photos_camera   ON photos(camera_model);
CREATE INDEX IF NOT EXISTS idx_photos_kind     ON photos(kind);

CREATE TABLE IF NOT EXISTS albums (
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
CREATE INDEX IF NOT EXISTS idx_albums_parent ON albums(parent_path);

CREATE TABLE IF NOT EXISTS album_photos (
  album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  position INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(album_id, photo_id)
);
CREATE INDEX IF NOT EXISTS idx_album_photos_photo ON album_photos(photo_id);

CREATE TABLE IF NOT EXISTS tags (
  id   INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS photo_tags (
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(id)   ON DELETE CASCADE,
  PRIMARY KEY(photo_id, tag_id)
);
CREATE TABLE IF NOT EXISTS album_tags (
  album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(id)   ON DELETE CASCADE,
  PRIMARY KEY(album_id, tag_id)
);

-- Reserved for the develop phase: an ordered, non-destructive operation list.
CREATE TABLE IF NOT EXISTS edits (
  photo_id   INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
  version    INTEGER NOT NULL DEFAULT 1,
  stack_json TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
  entity_kind    TEXT NOT NULL,
  entity_key     TEXT NOT NULL,
  local_hash     TEXT,
  synced_hash    TEXT,
  remote_hash    TEXT,
  last_synced_at TEXT,
  PRIMARY KEY(entity_kind, entity_key)
);

CREATE TABLE IF NOT EXISTS settings (
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
CREATE TABLE IF NOT EXISTS album_sync (
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
CREATE TABLE IF NOT EXISTS published_files (
  album_path TEXT NOT NULL,
  filename   TEXT NOT NULL,
  PRIMARY KEY (album_path, filename)
);
"#;

/// v4 — provenance links for Lightroom Classic imports.
///
/// `lrcat_id` is a stable identifier of the *source catalog* (an id read from
/// the lrcat when it offers one, else the blake3 of its canonicalized path),
/// so two different catalogs can both be imported into one library without
/// their image ids colliding. A link is what makes re-running an import a sync
/// instead of a duplication: an LR image or collection that is already linked
/// is updated in place rather than imported again.
const SCHEMA_V4: &str = r#"
CREATE TABLE IF NOT EXISTS lr_links (
  lrcat_id TEXT    NOT NULL,
  lr_image INTEGER NOT NULL,
  photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
  PRIMARY KEY (lrcat_id, lr_image)
);
CREATE INDEX IF NOT EXISTS idx_lr_links_photo ON lr_links(photo_id);

CREATE TABLE IF NOT EXISTS lr_album_links (
  lrcat_id      TEXT    NOT NULL,
  lr_collection INTEGER NOT NULL,
  album_path    TEXT    NOT NULL,
  PRIMARY KEY (lrcat_id, lr_collection)
);
"#;

/// v5 — multiple remotes and publish targets.
///
/// `remotes` and `publish_targets` are new; `album_sync`, `sync_state` and
/// `published_files` are *rebuilt* so their primary keys carry the remote or
/// target they belong to — per-(album, remote) state is what makes a second
/// remote possible at all, and what a future master catalog reads.
///
/// The data migration ([`apply_v5`]) folds today's single remote
/// (`remote.dir` / `remote.token` settings) into remotes row #1 named "Main",
/// and `publish.dest` / `publish.min_rating` into publish_targets row #1, then
/// re-keys every existing subscription, baseline and publish record to those
/// ids. A library that never configured a remote gets empty tables, and the
/// first remote added becomes the default.
const SCHEMA_V5_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS remotes (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  target     TEXT NOT NULL,      -- folder path, or http(s) URL of a sync API
  token      TEXT,               -- bearer secret for an HTTP target
  created_at TEXT
);
CREATE TABLE IF NOT EXISTS publish_targets (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  dest_root  TEXT NOT NULL,      -- the gallery's src/content/albums
  min_rating INTEGER
);
"#;

/// Does `table` have a column called `column`? The v5 rebuilds are guarded by
/// this so the migration is idempotent: a catalog wedged half-way by the old
/// non-atomic migrator is carried across rather than left unopenable.
fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn v5_has_rows(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
    Ok(n > 0)
}

fn v5_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
        .optional()?
        .filter(|v: &String| !v.is_empty()))
}

/// The v5 migration: create the new tables, then rebuild the three per-remote
/// tables around them, folding the single configured remote/destination into
/// row #1 of each. Runs inside the caller's transaction.
fn apply_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_V5_TABLES)?;

    // --- album_sync + sync_state: keyed by remote --------------------------
    let old_album_sync = !table_has_column(conn, "album_sync", "remote_id")?;
    let old_sync_state = !table_has_column(conn, "sync_state", "remote_id")?;

    if old_album_sync || old_sync_state {
        // Today's remote, if one was ever configured — or a placeholder when
        // rows exist without one, because dropping a baseline reads later as
        // "deleted on purpose" and takes files off the server.
        let need_remote = v5_setting(conn, "remote.dir")?.is_some()
            || (old_album_sync && v5_has_rows(conn, "album_sync")?)
            || (old_sync_state && v5_has_rows(conn, "sync_state")?);
        let remote_id: Option<i64> = if need_remote {
            let target = v5_setting(conn, "remote.dir")?.unwrap_or_default();
            let token: Option<String> = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'remote.token'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            conn.execute(
                "INSERT INTO remotes(name, target, token, created_at) VALUES('Main', ?1, ?2, ?3)",
                params![target, token, chrono::Utc::now().to_rfc3339()],
            )?;
            Some(conn.last_insert_rowid())
        } else {
            None
        };

        if old_album_sync {
            conn.execute_batch(
                "CREATE TABLE album_sync_v5 (
                   album_path     TEXT    NOT NULL,
                   remote_id      INTEGER NOT NULL REFERENCES remotes(id) ON DELETE CASCADE,
                   direction      TEXT    NOT NULL DEFAULT 'both',
                   scope          TEXT    NOT NULL DEFAULT 'web',
                   last_synced_at TEXT,
                   PRIMARY KEY (album_path, remote_id)
                 );",
            )?;
            if let Some(id) = remote_id {
                conn.execute(
                    "INSERT INTO album_sync_v5(album_path, remote_id, direction, scope, last_synced_at) \
                     SELECT album_path, ?1, direction, 'web', last_synced_at FROM album_sync",
                    params![id],
                )?;
            }
            conn.execute_batch(
                "DROP TABLE album_sync; ALTER TABLE album_sync_v5 RENAME TO album_sync;",
            )?;
        }

        if old_sync_state {
            conn.execute_batch(
                "CREATE TABLE sync_state_v5 (
                   remote_id      INTEGER NOT NULL REFERENCES remotes(id) ON DELETE CASCADE,
                   entity_kind    TEXT NOT NULL,
                   entity_key     TEXT NOT NULL,
                   local_hash     TEXT,
                   synced_hash    TEXT,
                   remote_hash    TEXT,
                   last_synced_at TEXT,
                   PRIMARY KEY (remote_id, entity_kind, entity_key)
                 );",
            )?;
            if let Some(id) = remote_id {
                conn.execute(
                    "INSERT INTO sync_state_v5(remote_id, entity_kind, entity_key, local_hash, \
                       synced_hash, remote_hash, last_synced_at) \
                     SELECT ?1, entity_kind, entity_key, local_hash, synced_hash, remote_hash, \
                       last_synced_at FROM sync_state",
                    params![id],
                )?;
            }
            conn.execute_batch(
                "DROP TABLE sync_state; ALTER TABLE sync_state_v5 RENAME TO sync_state;",
            )?;
        }
    }

    // --- published_files: keyed by publish target --------------------------
    if !table_has_column(conn, "published_files", "target_id")? {
        let need_target = v5_setting(conn, "publish.dest")?.is_some()
            || v5_has_rows(conn, "published_files")?;
        let target_id: Option<i64> = if need_target {
            let dest = v5_setting(conn, "publish.dest")?.unwrap_or_default();
            let min_rating: Option<i64> =
                v5_setting(conn, "publish.min_rating")?.and_then(|v| v.parse().ok());
            conn.execute(
                "INSERT INTO publish_targets(name, dest_root, min_rating) VALUES('Main', ?1, ?2)",
                params![dest, min_rating],
            )?;
            Some(conn.last_insert_rowid())
        } else {
            None
        };

        conn.execute_batch(
            "CREATE TABLE published_files_v5 (
               target_id  INTEGER NOT NULL REFERENCES publish_targets(id) ON DELETE CASCADE,
               album_path TEXT NOT NULL,
               filename   TEXT NOT NULL,
               PRIMARY KEY (target_id, album_path, filename)
             );",
        )?;
        if let Some(id) = target_id {
            conn.execute(
                "INSERT INTO published_files_v5(target_id, album_path, filename) \
                 SELECT ?1, album_path, filename FROM published_files",
                params![id],
            )?;
        }
        conn.execute_batch(
            "DROP TABLE published_files; ALTER TABLE published_files_v5 RENAME TO published_files;",
        )?;
    }

    Ok(())
}

/// v6 — sources: photos that live outside the library root.
///
/// Until now a `rel_path` was relative to the one folder the photographer
/// chose, and importing anything from elsewhere had to copy it in. A **source**
/// is a root a photo may live under; the library root becomes source #1, the
/// *primary*, and `photos.rel_path` becomes relative to whichever source its
/// row names. That is what makes a Lightroom import without file migration
/// possible: the LR root folder is registered as a source and its files are
/// catalogued where they lie.
///
/// `.gpp/` — catalog, thumbnails, render cache — stays on the primary, and
/// only the primary receives copy-in imports and pulls: a referenced source is
/// a place photographs *are*, never a place this app puts things.
const SCHEMA_V6_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS sources (
  id          INTEGER PRIMARY KEY,
  name        TEXT    NOT NULL,
  path        TEXT    NOT NULL UNIQUE,
  kind        TEXT    NOT NULL
              CHECK (kind IN ('primary','internal','external','network')),
  volume_hint TEXT,
  is_primary  INTEGER NOT NULL DEFAULT 0,
  added_at    TEXT
);
-- Exactly one primary, enforced by the schema rather than by everyone
-- remembering: the primary is where .gpp lives, and two of them is a library
-- with two catalogs' worth of derived data and no way to say which is real.
CREATE UNIQUE INDEX IF NOT EXISTS idx_sources_one_primary
  ON sources(is_primary) WHERE is_primary = 1;
"#;

/// The `photos` table as v6 shapes it: a `source_id`, and a `rel_path` that is
/// unique only *within* its source. Two cards can hold `DCIM/DSC_0001.jpg`
/// under two different roots and both belong in the catalog.
const SCHEMA_V6_PHOTOS: &str = r#"
CREATE TABLE photos_v6 (
  id            INTEGER PRIMARY KEY,
  source_id     INTEGER NOT NULL DEFAULT 1 REFERENCES sources(id),
  rel_path      TEXT    NOT NULL,
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
  imported_at   TEXT    NOT NULL,
  UNIQUE(source_id, rel_path)
);
"#;

/// Rebuild the photo indexes the v1 schema declared inline.
const SCHEMA_V6_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_photos_hash     ON photos(content_hash);
CREATE INDEX IF NOT EXISTS idx_photos_rating   ON photos(rating);
CREATE INDEX IF NOT EXISTS idx_photos_captured ON photos(captured_at);
CREATE INDEX IF NOT EXISTS idx_photos_camera   ON photos(camera_model);
CREATE INDEX IF NOT EXISTS idx_photos_kind     ON photos(kind);
CREATE INDEX IF NOT EXISTS idx_photos_source   ON photos(source_id);
"#;

/// The v6 migration: register the library root as the primary source, then
/// re-key every existing photo onto it.
///
/// Every row keeps its `rel_path` **verbatim** — the primary source's root is
/// the library root, so the paths already mean what they meant. A single-source
/// library therefore behaves after this migration exactly as it did before,
/// which is what the existing test suite is the proof of.
///
/// Runs inside the caller's transaction, with foreign keys off (see
/// [`migrate`]): `photos` is rebuilt to drop the old `UNIQUE(rel_path)`, and
/// half the schema references it.
fn apply_v6(conn: &Connection, root: &Path) -> Result<()> {
    conn.execute_batch(SCHEMA_V6_TABLES)?;

    // The library root becomes source #1. Named after its folder, because that
    // is the word the photographer already uses for it.
    let primary: Option<i64> = conn
        .query_row("SELECT id FROM sources WHERE is_primary = 1", [], |r| r.get(0))
        .optional()?;
    let primary = match primary {
        Some(id) => id,
        None => {
            conn.execute(
                "INSERT INTO sources(name, path, kind, is_primary, added_at) \
                 VALUES(?1, ?2, 'primary', 1, ?3)",
                params![
                    root_display_name(root),
                    root.display().to_string(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )?;
            conn.last_insert_rowid()
        }
    };

    if table_has_column(conn, "photos", "source_id")? {
        return Ok(());
    }

    conn.execute_batch(SCHEMA_V6_PHOTOS)?;
    conn.execute(
        &format!(
            "INSERT INTO photos_v6(source_id, {V5_PHOTO_COLS}) SELECT ?1, {V5_PHOTO_COLS} FROM photos"
        ),
        params![primary],
    )?;
    conn.execute_batch("DROP TABLE photos; ALTER TABLE photos_v6 RENAME TO photos;")?;
    conn.execute_batch(SCHEMA_V6_INDEXES)?;
    Ok(())
}

/// The v5 photo columns, in order — what the v6 rebuild carries across. Spelled
/// out rather than derived from [`PHOTO_COLS`], which now has `source_id` in it
/// and would make the copy select a column the old table does not have.
const V5_PHOTO_COLS: &str = "id, rel_path, filename, content_hash, file_size, mtime_ms, kind, \
     width, height, orientation, captured_at, camera_make, camera_model, lens, iso, \
     aperture, shutter, focal_length, rating, flag, color_label, blur_lqip, imported_at";

/// A display name for a root folder: its last segment, or the whole path when
/// it has none (a drive root, `/`).
pub(crate) fn root_display_name(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| root.display().to_string())
}

/// Keep the primary source row pointing at the root this library was opened
/// from.
///
/// The stored path is what `sources()` shows and what a person recognises; the
/// root itself is whatever the caller opened, canonicalized. A library carried
/// to another machine — a different mount point, a different drive letter — has
/// a stale spelling written in it, and resolution deliberately does not consult
/// it (the primary always resolves against `Library::root`), so refreshing it
/// costs one write and keeps the listing honest.
fn refresh_primary_source(conn: &Connection, root: &Path) -> Result<()> {
    let stored: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT path, volume_hint FROM sources WHERE is_primary = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let now = root.display().to_string();
    match stored {
        // Nothing to say: the spelling is current and the volume is known.
        Some((p, Some(_))) if p == now => Ok(()),
        Some(_) => {
            conn.execute(
                "UPDATE sources SET path = ?1, volume_hint = ?2 WHERE is_primary = 1",
                params![now, crate::sources::volume_hint(root)],
            )?;
            Ok(())
        }
        // No primary row at all is only reachable if something deleted it out
        // from under the schema's index; put one back rather than fail to open.
        None => {
            conn.execute(
                "INSERT INTO sources(name, path, kind, volume_hint, is_primary, added_at) \
                 VALUES(?1, ?2, 'primary', ?3, 1, ?4)",
                params![
                    root_display_name(root),
                    now,
                    crate::sources::volume_hint(root),
                    chrono::Utc::now().to_rfc3339()
                ],
            )?;
            Ok(())
        }
    }
}

/// Settings key holding this library's stable identity.
pub(crate) const SETTING_LIBRARY_ID: &str = "library_id";

/// Generate the library id once, at open, if it is absent.
fn ensure_library_id(conn: &Connection) -> Result<()> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SETTING_LIBRARY_ID],
            |r| r.get(0),
        )
        .optional()?;
    if existing.map(|v| !v.is_empty()).unwrap_or(false) {
        return Ok(());
    }
    use rand::Rng;
    let bytes: [u8; 16] = rand::thread_rng().gen();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SETTING_LIBRARY_ID, hex],
    )?;
    Ok(())
}

// ------------------------------------------------------------------ row glue

/// `source_id` is appended rather than slotted in beside `rel_path` so the
/// column indexes [`photo_from_row`] reads by did not all shift when v6 landed.
const PHOTO_COLS: &str = "id, rel_path, filename, content_hash, file_size, mtime_ms, kind, \
     width, height, orientation, captured_at, camera_make, camera_model, lens, iso, \
     aperture, shutter, focal_length, rating, flag, color_label, blur_lqip, imported_at, \
     source_id";

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
        source_id: row.get(23)?,
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

    /// A catalog caught half-way through a migration must still open.
    ///
    /// The steps used to run in autocommit, so a laptop closed between creating
    /// a table and writing the new version number down left a catalog claiming
    /// to be older than it is. Every open after that tried to create a table
    /// that was already there and failed — the same way, for ever. The library
    /// is a rebuildable index, but only by a program that can open it.
    #[test]
    fn a_half_applied_migration_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        {
            let lib = Library::open(dir.path()).unwrap();
            drop(lib);
            // Wind the books back to where the crash left them: v2's table on
            // disk, v3's not, and the version still saying 1.
            let conn = Connection::open(dir.path().join(GPP_DIR).join("catalog.db")).unwrap();
            conn.execute("UPDATE schema_version SET version = 1", []).unwrap();
            conn.execute("DROP TABLE published_files", []).unwrap();
        }

        let lib = Library::open(dir.path()).expect("the library can no longer be opened");
        // …and it comes back at the current schema, not stuck one behind.
        lib.record_published_files("a", &Default::default()).unwrap();
        assert!(lib.album_subscriptions().unwrap().is_empty());
    }

    /// A catalog written by a newer build must be refused, not run against.
    ///
    /// Two machines share a library over a network share or a carried drive,
    /// and one of them is a version behind. The older build used to open the
    /// newer catalog and work happily: it selects the columns it knows, writes
    /// rows the new schema's constraints were never applied to, and the damage
    /// only shows up later, on the machine that is up to date.
    #[test]
    fn a_catalog_from_a_newer_build_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        drop(Library::open(dir.path()).unwrap());
        {
            let conn = Connection::open(dir.path().join(GPP_DIR).join("catalog.db")).unwrap();
            conn.execute(
                "UPDATE schema_version SET version = ?1",
                params![SCHEMA_VERSION + 1],
            )
            .unwrap();
        }

        let Err(err) = Library::open(dir.path()) else {
            panic!("an older build must not open a newer catalog");
        };
        let msg = err.to_string();
        assert!(
            msg.contains(&format!("v{}", SCHEMA_VERSION + 1))
                && msg.contains(&format!("v{SCHEMA_VERSION}")),
            "the refusal has to name both versions, or nobody knows which build to reach for: {msg}"
        );
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

    /// `ap.position` only exists in the query when an album was joined in.
    /// Asking for album order without an album handed SQLite a column that was
    /// not there — and the first caller to write the filter JSON by hand, a
    /// CLI or a client over the C door, reaches that before the UI ever does.
    #[test]
    fn album_order_without_an_album_still_answers() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        let listed = lib.photos(&PhotoFilter {
            sort: PhotoSort::AlbumOrder,
            ..Default::default()
        });
        assert!(listed.is_ok(), "{:?}", listed.err());
    }

    fn insert_photo(lib: &Library, rel_path: &str) {
        let filename = rel_path.rsplit('/').next().unwrap();
        lib.with_conn(|c| {
            c.execute(
                "INSERT INTO photos(rel_path, filename, content_hash, file_size, \
                 mtime_ms, imported_at) VALUES(?1, ?2, 'h', 0, 0, '')",
                params![rel_path, filename],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn search(lib: &Library, text: &str) -> Vec<String> {
        lib.photos(&PhotoFilter {
            text: Some(text.to_string()),
            sort: PhotoSort::NameAsc,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .map(|p| p.filename)
        .collect()
    }

    /// `_` matches any single character in SQL LIKE, and it is in the filename
    /// of nearly every frame a Nikon writes. Pasted straight into the pattern,
    /// searching for the `DSC_0042` off one card also turned up the Fuji's
    /// `DSCF0042` — a different photograph, from a different camera, at the top
    /// of a list the photographer is using to find one frame.
    #[test]
    fn a_wildcard_in_a_filename_is_searched_for_literally() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        for name in ["DSC_0042.jpg", "DSCF0042.jpg", "DSC-0042.jpg"] {
            insert_photo(&lib, name);
        }
        assert_eq!(search(&lib, "DSC_0042"), vec!["DSC_0042.jpg"]);

        // `%` stands for any run of characters, so a bare one used to select
        // the whole library instead of the files actually named with it.
        insert_photo(&lib, "100% crop.jpg");
        assert_eq!(search(&lib, "%"), vec!["100% crop.jpg"]);

        // And the escape character itself is a legal byte in a Unix filename.
        insert_photo(&lib, "back\\slash.jpg");
        assert_eq!(search(&lib, "back\\slash"), vec!["back\\slash.jpg"]);
    }

    /// Build a genuine v4 catalog on disk — the schema constants are the ones
    /// v4 builds ran, so this is the real thing, not a simulation.
    fn write_v4_catalog(dir: &Path) -> Connection {
        let gpp = dir.join(GPP_DIR);
        std::fs::create_dir_all(&gpp).unwrap();
        let conn = Connection::open(gpp.join("catalog.db")).unwrap();
        conn.execute_batch("CREATE TABLE schema_version(version INTEGER NOT NULL);").unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(SCHEMA_V2).unwrap();
        conn.execute_batch(SCHEMA_V3).unwrap();
        conn.execute_batch(SCHEMA_V4).unwrap();
        conn.execute("INSERT INTO schema_version(version) VALUES(4)", []).unwrap();
        conn
    }

    /// …and a genuine v5 one: v4 plus the remotes/targets rebuild, stamped 5.
    fn write_v5_catalog(dir: &Path) -> Connection {
        let conn = write_v4_catalog(dir);
        apply_v5(&conn).unwrap();
        conn.execute("UPDATE schema_version SET version = 5", []).unwrap();
        conn
    }

    /// The v6 migration puts every existing photo on source #1 — the library
    /// root — with its `rel_path` untouched, and everything that hangs off a
    /// photo comes across with it.
    ///
    /// `photos` is the one table half the schema points at, and v6 rebuilds it
    /// to drop the old `UNIQUE(rel_path)`. With foreign keys left on, the
    /// `DROP TABLE` in the middle of that runs an implicit DELETE and cascades
    /// every album membership and every edit stack into oblivion — a migration
    /// that opens cleanly and quietly empties the library. This is the test
    /// that would have caught it.
    #[test]
    fn v5_photos_land_on_the_primary_source_with_everything_attached_to_them() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = write_v5_catalog(dir.path());
            conn.execute(
                "INSERT INTO photos(id, rel_path, filename, content_hash, file_size, \
                 mtime_ms, rating, flag, color_label, imported_at) \
                 VALUES(7, '2026/ana/a1.jpg', 'a1.jpg', 'hash-a1', 100, 0, 5, 'pick', \
                 'Red', '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO photos(id, rel_path, filename, content_hash, file_size, \
                 mtime_ms, imported_at) \
                 VALUES(8, '2026/ana/a2.jpg', 'a2.jpg', 'hash-a2', 100, 0, '')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO albums(id, path, title, token) VALUES(1, '2026/ana', 'Ana', 'tok')",
                [],
            )
            .unwrap();
            conn.execute("UPDATE albums SET cover_photo_id = 7 WHERE id = 1", []).unwrap();
            conn.execute(
                "INSERT INTO album_photos(album_id, photo_id, position) VALUES(1, 7, 0), (1, 8, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO edits(photo_id, version, stack_json) \
                 VALUES(7, 1, '{\"version\":1,\"ops\":[{\"op\":\"exposure\",\"ev\":0.5}]}')",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO tags(id, name) VALUES(1, 'wedding')", []).unwrap();
            conn.execute("INSERT INTO photo_tags(photo_id, tag_id) VALUES(7, 1)", []).unwrap();
            conn.execute(
                "INSERT INTO lr_links(lrcat_id, lr_image, photo_id) VALUES('cat', 1000, 7)",
                [],
            )
            .unwrap();
        }

        let lib = Library::open(dir.path()).expect("the migration must open a v5 catalog");

        // One source: the library itself, named after its folder.
        let sources = lib.sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].is_primary);
        assert_eq!(sources[0].path, lib.root().display().to_string());
        assert_eq!(sources[0].photo_count, 2);

        // Both photos are on it, at exactly the paths they had.
        let primary = lib.primary_source_id().unwrap();
        let a1 = lib.photo_by_rel_path("2026/ana/a1.jpg").unwrap().unwrap();
        assert_eq!(a1.id, 7, "rowids are stable across the rebuild");
        assert_eq!(a1.source_id, primary);
        assert_eq!(a1.rating, 5);
        assert_eq!(a1.flag, Flag::Pick);
        assert_eq!(a1.color_label.as_deref(), Some("Red"));
        assert_eq!(a1.content_hash, "hash-a1");
        assert!(lib.photo_by_rel_path("2026/ana/a2.jpg").unwrap().is_some());
        assert_eq!(lib.photo_count().unwrap(), 2);

        // …and everything that hangs off a photo survived the table rebuild.
        assert_eq!(
            lib.album_photos("2026/ana")
                .unwrap()
                .into_iter()
                .map(|p| p.filename)
                .collect::<Vec<_>>(),
            vec!["a1.jpg", "a2.jpg"],
            "album memberships were cascaded away by the rebuild"
        );
        assert_eq!(
            lib.album_by_path("2026/ana").unwrap().unwrap().cover_filename.as_deref(),
            Some("a1.jpg")
        );
        assert!(!lib.edits(7).unwrap().is_empty(), "the develop stack was dropped");
        assert_eq!(lib.photo_tags(7).unwrap(), vec!["wedding"]);
        assert_eq!(lib.lr_photo_links("cat").unwrap().get(&1000), Some(&7));

        // And re-opening changes nothing — the migration is not re-applied.
        drop(lib);
        let again = Library::open(dir.path()).unwrap();
        assert_eq!(again.photo_count().unwrap(), 2);
        assert_eq!(again.sources().unwrap().len(), 1);
        assert_eq!(again.album_photos("2026/ana").unwrap().len(), 2);
    }

    /// The old uniqueness was on `rel_path` alone. It becomes
    /// `(source_id, rel_path)`, which is the whole point: two registered cards
    /// may each hold `DCIM/DSC_0001.jpg`, and they are two photographs.
    #[test]
    fn a_relative_path_is_unique_within_a_source_not_across_the_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let drive = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let other = lib.add_source(drive.path(), Some("Card"), None).unwrap();
        let primary = lib.primary_source_id().unwrap();

        for source in [primary, other] {
            lib.with_conn(|c| {
                c.execute(
                    "INSERT INTO photos(source_id, rel_path, filename, content_hash, \
                     file_size, mtime_ms, imported_at) \
                     VALUES(?1, 'DCIM/DSC_0001.jpg', 'DSC_0001.jpg', 'h', 0, 0, '')",
                    params![source],
                )?;
                Ok(())
            })
            .unwrap();
        }
        assert_eq!(lib.photo_count().unwrap(), 2);

        // The same path twice under one source is still refused.
        let again = lib.with_conn(|c| {
            Ok(c.execute(
                "INSERT INTO photos(source_id, rel_path, filename, content_hash, \
                 file_size, mtime_ms, imported_at) \
                 VALUES(?1, 'DCIM/DSC_0001.jpg', 'DSC_0001.jpg', 'h', 0, 0, '')",
                params![primary],
            )?)
        });
        assert!(again.is_err(), "one source cannot hold one path twice");

        // And each lookup finds its own.
        assert_eq!(
            lib.photo_by_rel_path("DCIM/DSC_0001.jpg").unwrap().unwrap().source_id,
            primary
        );
        assert_eq!(
            lib.photo_by_source_rel_path(other, "DCIM/DSC_0001.jpg")
                .unwrap()
                .unwrap()
                .source_id,
            other
        );
    }

    /// The v5 migration folds today's single remote and destination into row
    /// #1 of the new tables, carries every subscription, baseline and publish
    /// record onto those ids — and the un-suffixed APIs then behave exactly
    /// as they did on v4.
    #[test]
    fn v4_with_a_configured_remote_migrates_to_row_one_of_each() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = write_v4_catalog(dir.path());
            for (k, v) in [
                ("remote.dir", "https://gallery.example/api/sync"),
                ("remote.token", "sync-token-16-chars!"),
                ("publish.dest", "/srv/gallery/albums"),
                ("publish.min_rating", "3"),
            ] {
                conn.execute("INSERT INTO settings(key, value) VALUES(?1, ?2)", params![k, v])
                    .unwrap();
            }
            conn.execute(
                "INSERT INTO album_sync(album_path, direction, last_synced_at) \
                 VALUES('2026/ana', 'push', '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sync_state(entity_kind, entity_key, synced_hash) \
                 VALUES('file', '2026/ana/a.jpg', 'hash-a')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO published_files(album_path, filename) VALUES('2026/ana', 'a.jpg')",
                [],
            )
            .unwrap();
        }

        let lib = Library::open(dir.path()).expect("the migration must open a v4 catalog");

        // The single remote became remotes row #1 named "Main", and it is the
        // default the compatibility APIs act on.
        let remotes = lib.remotes().unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "Main");
        assert_eq!(remotes[0].target, "https://gallery.example/api/sync");
        assert_eq!(remotes[0].token.as_deref(), Some("sync-token-16-chars!"));
        assert!(remotes[0].is_default);

        let targets = lib.publish_targets().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "Main");
        assert_eq!(targets[0].dest_root, "/srv/gallery/albums");
        assert_eq!(targets[0].min_rating, Some(3));

        // Subscriptions, baselines and publish records survived, re-keyed —
        // and read back identically through the old entry points.
        let subs = lib.album_subscriptions().unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].album_path, "2026/ana");
        assert_eq!(subs[0].direction, crate::sync::SyncDirection::Push);
        assert_eq!(subs[0].scope, crate::sync::SyncScopeKind::Web);
        assert_eq!(subs[0].last_synced_at.as_deref(), Some("2026-01-01T00:00:00Z"));

        let synced = lib.synced_manifest().unwrap();
        assert_eq!(synced.get("2026/ana/a.jpg").map(String::as_str), Some("hash-a"));

        assert_eq!(
            lib.published_files("2026/ana").unwrap(),
            ["a.jpg".to_string()].into_iter().collect()
        );

        // The catalog now carries a stable identity.
        let id = lib.library_id().unwrap();
        assert_eq!(id.len(), 32, "16 random bytes, hex: {id:?}");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));

        // …and it survives a re-open unchanged, as does everything above.
        drop(lib);
        let again = Library::open(dir.path()).unwrap();
        assert_eq!(again.library_id().unwrap(), id);
        assert_eq!(again.remotes().unwrap().len(), 1);
    }

    /// A library that never configured a remote migrates to empty tables, and
    /// the first remote added becomes the default.
    #[test]
    fn v4_without_a_remote_migrates_to_empty_tables() {
        let dir = tempfile::tempdir().unwrap();
        drop(write_v4_catalog(dir.path()));

        let lib = Library::open(dir.path()).unwrap();
        assert!(lib.remotes().unwrap().is_empty());
        assert!(lib.publish_targets().unwrap().is_empty());
        assert_eq!(lib.default_remote_id().unwrap(), None);

        let id = lib.add_remote("Studio", "/srv/studio", None).unwrap();
        assert_eq!(lib.default_remote_id().unwrap(), Some(id), "the first add is the default");
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
