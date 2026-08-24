//! Remotes and publish targets — the rows behind multi-destination sync.
//!
//! A **remote** is a sync destination this library knows: a folder (network
//! share, external drive, synced directory) or the `http(s)` URL of a
//! gallery's sync API, with the bearer token that URL needs. A **publish
//! target** is a local rendered-tree destination (`dest_root`). Both live in
//! the catalog, so they belong to the *library* and travel with the drive.
//!
//! Every subscription, sync baseline and publish record is keyed by one of
//! these ids (schema v5), which is what lets one album sync push-only to the
//! studio server and pull-only from a client's, with independent baselines —
//! a conflict on one remote is not a conflict on the other.
//!
//! One of each may be the **default**: the row the un-suffixed compatibility
//! APIs (`remote_dir`, `publish_target`, …) operate on. The default is a
//! settings row; with none written it is the lowest id. When nothing is
//! configured at all, the internals lazily create a placeholder row named
//! "Main" with an empty target, so that baselines recorded before a remote is
//! chosen (tests drive transports directly this way) still have a row to hang
//! off — an empty target reads back as "no remote configured".

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::catalog::{Library, GPP_DIR};
use crate::error::{Error, Result};

/// One sync destination, as stored.
///
/// `token` is included: the catalog stores it in plain text and the reason a
/// caller lists remotes — its own or another library's, for a cross-library
/// push — is to construct a transport, which needs the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInfo {
    pub id: i64,
    pub name: String,
    /// Folder path, or `http(s)` URL of a sync API. Which kind it is gets read
    /// off the string, same as the single-remote days.
    pub target: String,
    pub token: Option<String>,
    pub created_at: Option<String>,
    /// Whether this is the library's default remote.
    pub is_default: bool,
}

/// One local publish destination, as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishTargetInfo {
    pub id: i64,
    pub name: String,
    /// Absolute path of the gallery's `src/content/albums`.
    pub dest_root: String,
    /// Stars a photo needs before it ships; `None` publishes everything.
    pub min_rating: Option<u8>,
    /// Whether this is the library's default target.
    pub is_default: bool,
}

/// Fields of a remote to change; absent fields are left alone. `token` uses
/// the double option so `Some(None)` clears a stored secret.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub token: Option<Option<String>>,
}

/// Fields of a publish target to change; absent fields are left alone.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishTargetUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub dest_root: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub min_rating: Option<Option<u8>>,
}

/// Distinguish "field absent" from "field explicitly null" — same helper the
/// album update uses, restated here because it is private there.
fn double_option<'de, T, D>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

const SETTING_DEFAULT_REMOTE: &str = "remote.default";
const SETTING_DEFAULT_TARGET: &str = "publish.default";

impl Library {
    // ------------------------------------------------------------- remotes

    /// Every remote this library knows, lowest id first.
    pub fn remotes(&self) -> Result<Vec<RemoteInfo>> {
        let default = self.default_remote_id()?;
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT id, name, target, token, created_at FROM remotes ORDER BY id")?;
            let rows = stmt.query_map([], |r| {
                Ok(RemoteInfo {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    target: r.get(2)?,
                    token: r.get(3)?,
                    created_at: r.get(4)?,
                    is_default: false,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                let mut info = row?;
                info.is_default = Some(info.id) == default;
                out.push(info);
            }
            Ok(out)
        })
    }

    /// One remote by id, or `None`.
    pub fn remote_by_id(&self, id: i64) -> Result<Option<RemoteInfo>> {
        Ok(self.remotes()?.into_iter().find(|r| r.id == id))
    }

    /// Add a remote. The first one added becomes the default automatically —
    /// that is what "the first add becomes the default" means for a library
    /// migrated with no remote configured.
    pub fn add_remote(&self, name: &str, target: &str, token: Option<&str>) -> Result<i64> {
        if name.trim().is_empty() {
            return Err(Error::other("a remote needs a name"));
        }
        if target.trim().is_empty() {
            return Err(Error::other("a remote needs a target — a folder path or an http(s) URL"));
        }
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO remotes(name, target, token, created_at) VALUES(?1, ?2, ?3, ?4)",
                params![name, target, token, chrono::Utc::now().to_rfc3339()],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    /// Change a remote's name, target and/or token. Absent fields stay.
    pub fn update_remote(&self, id: i64, update: &RemoteUpdate) -> Result<()> {
        let changed = self.with_conn(|c| {
            let mut n = 0;
            if let Some(name) = &update.name {
                n += c.execute("UPDATE remotes SET name = ?1 WHERE id = ?2", params![name, id])?;
            }
            if let Some(target) = &update.target {
                n += c.execute(
                    "UPDATE remotes SET target = ?1 WHERE id = ?2",
                    params![target, id],
                )?;
            }
            if let Some(token) = &update.token {
                n += c.execute(
                    "UPDATE remotes SET token = ?1 WHERE id = ?2",
                    params![token.as_deref(), id],
                )?;
            }
            // With nothing to change, still verify the row exists.
            if update.name.is_none() && update.target.is_none() && update.token.is_none() {
                n += c.execute("UPDATE remotes SET id = id WHERE id = ?1", params![id])?;
            }
            Ok(n)
        })?;
        if changed == 0 {
            return Err(Error::other(format!("no remote with id {id}")));
        }
        Ok(())
    }

    /// Remove a remote. The FK cascade drops its subscriptions and baselines
    /// **locally** — nothing on the server it pointed at is touched.
    pub fn remove_remote(&self, id: i64) -> Result<()> {
        let changed =
            self.with_conn(|c| Ok(c.execute("DELETE FROM remotes WHERE id = ?1", params![id])?))?;
        if changed == 0 {
            return Err(Error::other(format!("no remote with id {id}")));
        }
        // A default pointing at a removed row falls back to the lowest id.
        if self.get_setting(SETTING_DEFAULT_REMOTE)? == Some(id.to_string()) {
            self.set_setting(SETTING_DEFAULT_REMOTE, "")?;
        }
        Ok(())
    }

    /// The remote the un-suffixed APIs act on. A stored choice wins; otherwise
    /// the lowest id; `None` when the table is empty.
    pub fn default_remote_id(&self) -> Result<Option<i64>> {
        if let Some(stored) = self
            .get_setting(SETTING_DEFAULT_REMOTE)?
            .and_then(|v| v.parse::<i64>().ok())
        {
            let exists = self.with_conn(|c| {
                Ok(c.query_row("SELECT id FROM remotes WHERE id = ?1", params![stored], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?)
            })?;
            if exists.is_some() {
                return Ok(Some(stored));
            }
        }
        self.with_conn(|c| {
            Ok(c.query_row("SELECT MIN(id) FROM remotes", [], |r| r.get::<_, Option<i64>>(0))?)
        })
    }

    /// Choose the default remote.
    pub fn set_default_remote(&self, id: i64) -> Result<()> {
        if self.remote_by_id(id)?.is_none() {
            return Err(Error::other(format!("no remote with id {id}")));
        }
        self.set_setting(SETTING_DEFAULT_REMOTE, &id.to_string())
    }

    /// The default remote's id, creating a placeholder row when the table is
    /// empty — baselines and subscriptions need a row to reference even before
    /// a destination is chosen (which is exactly how the tests drive
    /// transports directly). Public because the CLI configures remotes
    /// through the `Library` handle directly.
    pub fn ensure_default_remote(&self) -> Result<i64> {
        if let Some(id) = self.default_remote_id()? {
            return Ok(id);
        }
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO remotes(name, target, created_at) VALUES('Main', '', ?1)",
                params![chrono::Utc::now().to_rfc3339()],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    // ----------------------------------------------------- publish targets

    /// Every publish target, lowest id first.
    pub fn publish_targets(&self) -> Result<Vec<PublishTargetInfo>> {
        let default = self.default_target_id()?;
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT id, name, dest_root, min_rating FROM publish_targets ORDER BY id")?;
            let rows = stmt.query_map([], |r| {
                Ok(PublishTargetInfo {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    dest_root: r.get(2)?,
                    min_rating: r.get::<_, Option<i64>>(3)?.map(|v| v as u8),
                    is_default: false,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                let mut info = row?;
                info.is_default = Some(info.id) == default;
                out.push(info);
            }
            Ok(out)
        })
    }

    /// One publish target by id, or `None`.
    pub fn publish_target_by_id(&self, id: i64) -> Result<Option<PublishTargetInfo>> {
        Ok(self.publish_targets()?.into_iter().find(|t| t.id == id))
    }

    /// Add a publish target.
    pub fn add_publish_target(
        &self,
        name: &str,
        dest_root: &str,
        min_rating: Option<u8>,
    ) -> Result<i64> {
        if name.trim().is_empty() {
            return Err(Error::other("a publish target needs a name"));
        }
        if dest_root.trim().is_empty() {
            return Err(Error::other("a publish target needs a destination folder"));
        }
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO publish_targets(name, dest_root, min_rating) VALUES(?1, ?2, ?3)",
                params![name, dest_root, min_rating.map(|m| m as i64)],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    /// Change a publish target; absent fields stay, `min_rating: Some(None)`
    /// clears the filter.
    pub fn update_publish_target(&self, id: i64, update: &PublishTargetUpdate) -> Result<()> {
        let changed = self.with_conn(|c| {
            let mut n = 0;
            if let Some(name) = &update.name {
                n += c.execute(
                    "UPDATE publish_targets SET name = ?1 WHERE id = ?2",
                    params![name, id],
                )?;
            }
            if let Some(dest) = &update.dest_root {
                n += c.execute(
                    "UPDATE publish_targets SET dest_root = ?1 WHERE id = ?2",
                    params![dest, id],
                )?;
            }
            if let Some(min) = &update.min_rating {
                n += c.execute(
                    "UPDATE publish_targets SET min_rating = ?1 WHERE id = ?2",
                    params![min.map(|m| m as i64), id],
                )?;
            }
            if update.name.is_none() && update.dest_root.is_none() && update.min_rating.is_none() {
                n += c.execute("UPDATE publish_targets SET id = id WHERE id = ?1", params![id])?;
            }
            Ok(n)
        })?;
        if changed == 0 {
            return Err(Error::other(format!("no publish target with id {id}")));
        }
        Ok(())
    }

    /// Remove a publish target. Cascade drops its publish records; the files
    /// in its tree are left exactly where they are.
    pub fn remove_publish_target(&self, id: i64) -> Result<()> {
        let changed = self.with_conn(|c| {
            Ok(c.execute("DELETE FROM publish_targets WHERE id = ?1", params![id])?)
        })?;
        if changed == 0 {
            return Err(Error::other(format!("no publish target with id {id}")));
        }
        if self.get_setting(SETTING_DEFAULT_TARGET)? == Some(id.to_string()) {
            self.set_setting(SETTING_DEFAULT_TARGET, "")?;
        }
        Ok(())
    }

    /// The publish target the un-suffixed APIs act on.
    pub fn default_target_id(&self) -> Result<Option<i64>> {
        if let Some(stored) = self
            .get_setting(SETTING_DEFAULT_TARGET)?
            .and_then(|v| v.parse::<i64>().ok())
        {
            let exists = self.with_conn(|c| {
                Ok(c.query_row(
                    "SELECT id FROM publish_targets WHERE id = ?1",
                    params![stored],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?)
            })?;
            if exists.is_some() {
                return Ok(Some(stored));
            }
        }
        self.with_conn(|c| {
            Ok(c.query_row("SELECT MIN(id) FROM publish_targets", [], |r| {
                r.get::<_, Option<i64>>(0)
            })?)
        })
    }

    /// Choose the default publish target.
    pub fn set_default_publish_target(&self, id: i64) -> Result<()> {
        if self.publish_target_by_id(id)?.is_none() {
            return Err(Error::other(format!("no publish target with id {id}")));
        }
        self.set_setting(SETTING_DEFAULT_TARGET, &id.to_string())
    }

    /// The default target's id, creating a placeholder row when none exists.
    pub fn ensure_default_target(&self) -> Result<i64> {
        if let Some(id) = self.default_target_id()? {
            return Ok(id);
        }
        self.with_conn(|c| {
            c.execute("INSERT INTO publish_targets(name, dest_root) VALUES('Main', '')", [])?;
            Ok(c.last_insert_rowid())
        })
    }
}

// -------------------------------------------------- reading another library

/// List the remotes of a library that is **not** open in this session.
///
/// Opens only that library's catalog, read-only, and never migrates it: a
/// foreign library belongs to whatever build manages it. A catalog still on
/// the pre-v5 schema is read through its legacy `remote.dir`/`remote.token`
/// settings instead; one from a newer build is refused by name, the same way
/// [`Library::open`] refuses one.
///
/// This is what makes "push to \<remote\> (from library X)" possible: the app
/// holds the other library's path in its registry, the credentials stay in
/// that library's catalog, and this reads them without a session swap.
pub fn read_library_remotes(library_root: impl AsRef<Path>) -> Result<Vec<RemoteInfo>> {
    let db = library_root.as_ref().join(GPP_DIR).join("catalog.db");
    if !db.is_file() {
        return Err(Error::other(format!(
            "no library catalog at {} — is that folder a gpp library?",
            db.display()
        )));
    }
    let conn = Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let version: Option<i64> = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .optional()
        .unwrap_or(None);
    let version = version.unwrap_or(0);
    if version > crate::catalog::SCHEMA_VERSION_PUBLIC {
        return Err(Error::other(format!(
            "that library's catalog is schema v{version}, and this build understands \
             v{} — open it with a newer version of the app",
            crate::catalog::SCHEMA_VERSION_PUBLIC
        )));
    }

    let setting = |key: &str| -> Result<Option<String>> {
        Ok(conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?
            .filter(|v: &String| !v.is_empty()))
    };

    if version < 5 {
        // Legacy: the single remote lives in two settings keys.
        return Ok(match setting("remote.dir")? {
            Some(target) => vec![RemoteInfo {
                id: 1,
                name: "Main".into(),
                target,
                token: setting("remote.token")?,
                created_at: None,
                is_default: true,
            }],
            None => Vec::new(),
        });
    }

    let default: Option<i64> = setting(SETTING_DEFAULT_REMOTE)?.and_then(|v| v.parse().ok());
    let mut stmt =
        conn.prepare("SELECT id, name, target, token, created_at FROM remotes ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(RemoteInfo {
            id: r.get(0)?,
            name: r.get(1)?,
            target: r.get(2)?,
            token: r.get(3)?,
            created_at: r.get(4)?,
            is_default: false,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    let default = default
        .filter(|id| out.iter().any(|r| r.id == *id))
        .or_else(|| out.first().map(|r| r.id));
    for r in &mut out {
        r.is_default = Some(r.id) == default;
    }
    // Placeholder rows (empty target) are internal bookkeeping, not remotes a
    // foreign caller could push to.
    out.retain(|r| !r.target.is_empty());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_crud_and_default() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        assert!(lib.remotes().unwrap().is_empty());
        assert_eq!(lib.default_remote_id().unwrap(), None);

        let a = lib.add_remote("Studio", "https://studio.example/api/sync", Some("tok-tok-tok-tok!")).unwrap();
        let b = lib.add_remote("Drive", "/mnt/backup", None).unwrap();

        let listed = lib.remotes().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].is_default, "the first added remote is the default");
        assert_eq!(listed[0].token.as_deref(), Some("tok-tok-tok-tok!"));

        lib.set_default_remote(b).unwrap();
        assert_eq!(lib.default_remote_id().unwrap(), Some(b));

        lib.update_remote(a, &RemoteUpdate { name: Some("Studio 2".into()), token: Some(None), ..Default::default() })
            .unwrap();
        let a_row = lib.remote_by_id(a).unwrap().unwrap();
        assert_eq!(a_row.name, "Studio 2");
        assert_eq!(a_row.token, None, "Some(None) clears the token");
        assert_eq!(a_row.target, "https://studio.example/api/sync", "absent field untouched");

        lib.remove_remote(b).unwrap();
        assert_eq!(lib.default_remote_id().unwrap(), Some(a), "default falls back");
        assert!(lib.update_remote(b, &RemoteUpdate::default()).is_err(), "gone is gone");
    }

    #[test]
    fn removing_a_remote_cascades_its_local_state_only() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        let a = lib.add_remote("A", "/srv/a", None).unwrap();
        let b = lib.add_remote("B", "/srv/b", None).unwrap();
        lib.track_album_for("2026/x", crate::sync::SyncDirection::Push, None, a).unwrap();
        lib.track_album_for("2026/x", crate::sync::SyncDirection::Pull, None, b).unwrap();
        lib.record_synced_for(a, "2026/x/a.jpg", "h1").unwrap();
        lib.record_synced_for(b, "2026/x/a.jpg", "h2").unwrap();

        lib.remove_remote(a).unwrap();
        assert!(lib.album_subscriptions_for(a).unwrap().is_empty());
        assert!(lib.synced_manifest_for(a).unwrap().is_empty());
        // The other remote's state is untouched.
        assert_eq!(lib.album_subscriptions_for(b).unwrap().len(), 1);
        assert_eq!(lib.synced_manifest_for(b).unwrap().get("2026/x/a.jpg").unwrap(), "h2");
    }

    #[test]
    fn publish_targets_crud() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        let id = lib.add_publish_target("Site", "/srv/gallery/albums", Some(3)).unwrap();
        let t = lib.publish_target_by_id(id).unwrap().unwrap();
        assert_eq!(t.min_rating, Some(3));
        assert!(t.is_default);

        lib.update_publish_target(id, &PublishTargetUpdate { min_rating: Some(None), ..Default::default() })
            .unwrap();
        assert_eq!(lib.publish_target_by_id(id).unwrap().unwrap().min_rating, None);

        lib.remove_publish_target(id).unwrap();
        assert!(lib.publish_targets().unwrap().is_empty());
    }
}
