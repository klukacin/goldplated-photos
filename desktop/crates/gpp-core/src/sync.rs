//! Three-way sync between the local published tree and the server.
//!
//! The problem this exists to solve: `rsync --delete` cannot distinguish
//! *"I deleted this on purpose"* from *"this machine never had it"*. Mirroring
//! from a laptop holding a subset of the library therefore wipes the server.
//!
//! Keeping a third state — what we last agreed on with the server — makes the
//! distinction decidable. [`plan`] is a pure function over three manifests, so
//! the entire decision table is unit-tested.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::catalog::Library;
use crate::error::Result;

/// path → content hash.
pub type Manifest = BTreeMap<String, String>;

/// Moves files between here and the server.
///
/// A trait, not a hard-coded `rsync` call, because the core must compile for
/// iPadOS where spawning `ssh` is not an option. Desktop supplies an SFTP
/// implementation; mobile supplies an HTTP one.
pub trait RemoteTransport: Send + Sync {
    /// Hashes of everything currently on the server, under the sync scope.
    fn manifest(&self) -> Result<Manifest>;
    fn put(&self, rel_path: &str, bytes: &[u8]) -> Result<()>;
    fn get(&self, rel_path: &str) -> Result<Vec<u8>>;
    fn delete(&self, rel_path: &str) -> Result<()>;
}

/// One decision for one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Local is authoritative — upload.
    Push,
    /// Server is authoritative — download.
    Pull,
    /// Deleted locally on purpose; remove it on the server too.
    DeleteRemote,
    /// Both sides changed independently. Never resolved automatically.
    Conflict,
    /// Present on the server, unknown to this machine. **Do nothing** — this
    /// is the case that made mirroring unsafe.
    LeaveAlone,
    /// Gone on both sides; just drop the bookkeeping row.
    ForgetState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedChange {
    pub path: String,
    pub action: Action,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncPlan {
    pub changes: Vec<PlannedChange>,
}

impl SyncPlan {
    pub fn of(&self, action: Action) -> Vec<&PlannedChange> {
        self.changes.iter().filter(|c| c.action == action).collect()
    }

    pub fn count(&self, action: Action) -> usize {
        self.changes.iter().filter(|c| c.action == action).count()
    }

    pub fn has_conflicts(&self) -> bool {
        self.count(Action::Conflict) > 0
    }

    /// True when applying this plan would remove anything from the server.
    /// The UI must confirm before proceeding.
    pub fn is_destructive(&self) -> bool {
        self.count(Action::DeleteRemote) > 0
    }
}

/// Decide what to do for every path across the three manifests.
///
/// `local` and `remote` are current states; `synced` is what the two sides last
/// agreed on.
pub fn plan(local: &Manifest, synced: &Manifest, remote: &Manifest) -> SyncPlan {
    let mut paths: BTreeSet<&String> = BTreeSet::new();
    paths.extend(local.keys());
    paths.extend(synced.keys());
    paths.extend(remote.keys());

    let mut changes = Vec::new();
    for path in paths {
        let action = decide(local.get(path), synced.get(path), remote.get(path));
        if action != Action::LeaveAlone || remote.contains_key(path) {
            changes.push(PlannedChange {
                path: path.clone(),
                action,
            });
        }
    }
    SyncPlan { changes }
}

/// The decision table. Kept separate and total so every branch is testable.
fn decide(local: Option<&String>, synced: Option<&String>, remote: Option<&String>) -> Action {
    match (local, synced, remote) {
        // Present everywhere.
        (Some(l), Some(s), Some(r)) => {
            let local_changed = l != s;
            let remote_changed = r != s;
            match (local_changed, remote_changed) {
                (false, false) => Action::ForgetState, // already in sync
                (true, false) => Action::Push,
                (false, true) => Action::Pull,
                // Both moved. If they happen to agree, there is nothing to do.
                (true, true) if l == r => Action::ForgetState,
                (true, true) => Action::Conflict,
            }
        }

        // Server lost a file we previously published — re-publish rather than
        // delete locally. Non-destructive is the right default here.
        (Some(_), Some(_), None) => Action::Push,

        // No bookkeeping, but both sides have it: adopt if identical,
        // otherwise we cannot know who is newer.
        (Some(l), None, Some(r)) => {
            if l == r {
                Action::ForgetState
            } else {
                Action::Conflict
            }
        }

        // Brand new locally.
        (Some(_), None, None) => Action::Push,

        // We had it, we deleted it. Safe to remove remotely only if the server
        // copy is the one we agreed on; if it changed, ask.
        (None, Some(s), Some(r)) => {
            if s == r {
                Action::DeleteRemote
            } else {
                Action::Conflict
            }
        }

        // Gone on both sides.
        (None, Some(_), None) => Action::ForgetState,

        // *** The case that makes partial libraries safe. ***
        // The server has it, this machine has no record of it: another machine
        // published it. Touching it would be data loss.
        (None, None, Some(_)) => Action::LeaveAlone,

        (None, None, None) => Action::ForgetState,
    }
}

/// Restrict a sync to a subtree, so a machine can own part of the library.
#[derive(Debug, Clone, Default)]
pub struct SyncScope {
    /// Path prefixes to include. Empty means the whole tree.
    pub prefixes: Vec<String>,
}

impl SyncScope {
    pub fn everything() -> Self {
        Self::default()
    }

    pub fn with(prefixes: Vec<String>) -> Self {
        Self { prefixes }
    }

    pub fn includes(&self, path: &str) -> bool {
        if self.prefixes.is_empty() {
            return true;
        }
        self.prefixes
            .iter()
            .any(|p| path == p || path.starts_with(&format!("{p}/")))
    }

    pub fn filter(&self, manifest: &Manifest) -> Manifest {
        manifest
            .iter()
            .filter(|(k, _)| self.includes(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

// ------------------------------------------------------- persisted sync state

const ENTITY_FILE: &str = "file";

impl Library {
    /// The manifest of what we last agreed on with the server.
    pub fn synced_manifest(&self) -> Result<Manifest> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT entity_key, synced_hash FROM sync_state \
                 WHERE entity_kind = ?1 AND synced_hash IS NOT NULL",
            )?;
            let rows = stmt.query_map(params![ENTITY_FILE], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut out = Manifest::new();
            for row in rows {
                let (k, v) = row?;
                out.insert(k, v);
            }
            Ok(out)
        })
    }

    /// Record agreement on a path at a given hash.
    pub fn record_synced(&self, path: &str, hash: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO sync_state(entity_kind, entity_key, synced_hash, last_synced_at) \
                 VALUES(?1, ?2, ?3, ?4) \
                 ON CONFLICT(entity_kind, entity_key) DO UPDATE SET \
                   synced_hash = excluded.synced_hash, \
                   last_synced_at = excluded.last_synced_at",
                params![ENTITY_FILE, path, hash, now],
            )?;
            Ok(())
        })
    }

    pub fn forget_synced(&self, path: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "DELETE FROM sync_state WHERE entity_kind = ?1 AND entity_key = ?2",
                params![ENTITY_FILE, path],
            )?;
            Ok(())
        })
    }

    /// Persist an entire agreed manifest in one transaction.
    pub fn record_synced_all(&self, manifest: &Manifest) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.with_tx(|tx| {
            let mut stmt = tx.prepare(
                "INSERT INTO sync_state(entity_kind, entity_key, synced_hash, last_synced_at) \
                 VALUES(?1, ?2, ?3, ?4) \
                 ON CONFLICT(entity_kind, entity_key) DO UPDATE SET \
                   synced_hash = excluded.synced_hash, \
                   last_synced_at = excluded.last_synced_at",
            )?;
            for (path, hash) in manifest {
                stmt.execute(params![ENTITY_FILE, path, hash, now])?;
            }
            Ok(())
        })
    }
}

/// Outcome of applying a plan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncOutcome {
    pub pushed: usize,
    pub pulled: usize,
    pub deleted: usize,
    pub left_alone: usize,
    pub conflicts: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Execute a plan against a transport.
///
/// `allow_deletes` must be an explicit, confirmed decision by the caller —
/// nothing is removed from the server without it.
pub fn apply(
    lib: &Library,
    transport: &dyn RemoteTransport,
    plan: &SyncPlan,
    local_root: &std::path::Path,
    allow_deletes: bool,
) -> Result<SyncOutcome> {
    let mut out = SyncOutcome::default();

    for change in &plan.changes {
        let local_path = local_root.join(&change.path);
        match change.action {
            Action::Push => {
                match std::fs::read(&local_path) {
                    Ok(bytes) => match transport.put(&change.path, &bytes) {
                        Ok(()) => {
                            let hash = blake3::hash(&bytes).to_hex().to_string();
                            lib.record_synced(&change.path, &hash)?;
                            out.pushed += 1;
                        }
                        Err(e) => out.failed.push((change.path.clone(), e.to_string())),
                    },
                    Err(e) => out.failed.push((change.path.clone(), e.to_string())),
                }
            }
            Action::Pull => match transport.get(&change.path) {
                Ok(bytes) => {
                    if let Some(parent) = local_path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    match std::fs::write(&local_path, &bytes) {
                        Ok(()) => {
                            let hash = blake3::hash(&bytes).to_hex().to_string();
                            lib.record_synced(&change.path, &hash)?;
                            out.pulled += 1;
                        }
                        Err(e) => out.failed.push((change.path.clone(), e.to_string())),
                    }
                }
                Err(e) => out.failed.push((change.path.clone(), e.to_string())),
            },
            Action::DeleteRemote => {
                if !allow_deletes {
                    continue;
                }
                match transport.delete(&change.path) {
                    Ok(()) => {
                        lib.forget_synced(&change.path)?;
                        out.deleted += 1;
                    }
                    Err(e) => out.failed.push((change.path.clone(), e.to_string())),
                }
            }
            Action::Conflict => out.conflicts.push(change.path.clone()),
            Action::LeaveAlone => out.left_alone += 1,
            Action::ForgetState => {}
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(&str, &str)]) -> Manifest {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn action_for(local: Option<&str>, synced: Option<&str>, remote: Option<&str>) -> Action {
        decide(
            local.map(String::from).as_ref(),
            synced.map(String::from).as_ref(),
            remote.map(String::from).as_ref(),
        )
    }

    #[test]
    fn in_sync_does_nothing() {
        assert_eq!(action_for(Some("A"), Some("A"), Some("A")), Action::ForgetState);
    }

    #[test]
    fn local_change_pushes() {
        assert_eq!(action_for(Some("B"), Some("A"), Some("A")), Action::Push);
    }

    #[test]
    fn remote_change_pulls() {
        assert_eq!(action_for(Some("A"), Some("A"), Some("B")), Action::Pull);
    }

    #[test]
    fn divergent_change_is_a_conflict() {
        assert_eq!(action_for(Some("B"), Some("A"), Some("C")), Action::Conflict);
    }

    #[test]
    fn identical_independent_change_is_not_a_conflict() {
        assert_eq!(action_for(Some("B"), Some("A"), Some("B")), Action::ForgetState);
    }

    #[test]
    fn deliberate_local_delete_removes_remote() {
        assert_eq!(action_for(None, Some("A"), Some("A")), Action::DeleteRemote);
    }

    #[test]
    fn delete_of_a_remotely_changed_file_is_a_conflict() {
        assert_eq!(action_for(None, Some("A"), Some("B")), Action::Conflict);
    }

    /// The whole reason this module exists.
    #[test]
    fn file_this_machine_never_had_is_left_alone() {
        assert_eq!(action_for(None, None, Some("A")), Action::LeaveAlone);
    }

    #[test]
    fn new_local_file_is_pushed() {
        assert_eq!(action_for(Some("A"), None, None), Action::Push);
    }

    #[test]
    fn missing_remote_of_synced_file_is_republished() {
        assert_eq!(action_for(Some("A"), Some("A"), None), Action::Push);
    }

    #[test]
    fn unknown_but_identical_is_adopted() {
        assert_eq!(action_for(Some("A"), None, Some("A")), Action::ForgetState);
    }

    #[test]
    fn unknown_and_different_is_a_conflict() {
        assert_eq!(action_for(Some("A"), None, Some("B")), Action::Conflict);
    }

    /// A laptop holding only part of the library must not endanger the rest.
    #[test]
    fn partial_library_never_deletes_other_machines_work() {
        let local = m(&[("2026/a/1.jpg", "h1")]);
        let synced = m(&[("2026/a/1.jpg", "h1")]);
        let remote = m(&[
            ("2026/a/1.jpg", "h1"),
            ("2019/old/x.jpg", "h9"),
            ("2020/other/y.jpg", "h8"),
        ]);

        let p = plan(&local, &synced, &remote);
        assert_eq!(p.count(Action::DeleteRemote), 0, "must not delete anything");
        assert_eq!(p.count(Action::LeaveAlone), 2, "the two unknown albums are untouched");
        assert!(!p.is_destructive());
    }

    #[test]
    fn plan_reports_destructive_and_conflicting() {
        let local = m(&[("keep.jpg", "A"), ("changed.jpg", "LOCAL")]);
        let synced = m(&[("keep.jpg", "A"), ("changed.jpg", "BASE"), ("gone.jpg", "G")]);
        let remote = m(&[
            ("keep.jpg", "A"),
            ("changed.jpg", "REMOTE"),
            ("gone.jpg", "G"),
        ]);

        let p = plan(&local, &synced, &remote);
        assert!(p.has_conflicts());
        assert!(p.is_destructive());
        assert_eq!(p.count(Action::Conflict), 1);
        assert_eq!(p.count(Action::DeleteRemote), 1);
    }

    #[test]
    fn scope_limits_to_a_subtree() {
        let scope = SyncScope::with(vec!["2026".into()]);
        assert!(scope.includes("2026/a/1.jpg"));
        assert!(scope.includes("2026"));
        assert!(!scope.includes("2019/a/1.jpg"));
        // A prefix must match a whole segment, not a partial name.
        assert!(!scope.includes("2026extra/a.jpg"));

        let full = m(&[("2026/a.jpg", "h"), ("2019/b.jpg", "h")]);
        assert_eq!(scope.filter(&full).len(), 1);
        assert_eq!(SyncScope::everything().filter(&full).len(), 2);
    }

    #[test]
    fn synced_state_round_trips() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        assert!(lib.synced_manifest().unwrap().is_empty());

        lib.record_synced("a/1.jpg", "hash1").unwrap();
        lib.record_synced("a/2.jpg", "hash2").unwrap();
        let m1 = lib.synced_manifest().unwrap();
        assert_eq!(m1.len(), 2);
        assert_eq!(m1.get("a/1.jpg").unwrap(), "hash1");

        lib.record_synced("a/1.jpg", "hash1b").unwrap();
        assert_eq!(lib.synced_manifest().unwrap().get("a/1.jpg").unwrap(), "hash1b");

        lib.forget_synced("a/1.jpg").unwrap();
        assert_eq!(lib.synced_manifest().unwrap().len(), 1);
    }

    /// In-memory transport so the apply path is exercised without a server.
    struct FakeTransport {
        files: std::sync::Mutex<Manifest>,
        blobs: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl RemoteTransport for FakeTransport {
        fn manifest(&self) -> Result<Manifest> {
            Ok(self.files.lock().unwrap().clone())
        }
        fn put(&self, rel: &str, bytes: &[u8]) -> Result<()> {
            self.files
                .lock()
                .unwrap()
                .insert(rel.to_string(), blake3::hash(bytes).to_hex().to_string());
            self.blobs.lock().unwrap().insert(rel.to_string(), bytes.to_vec());
            Ok(())
        }
        fn get(&self, rel: &str) -> Result<Vec<u8>> {
            self.blobs
                .lock()
                .unwrap()
                .get(rel)
                .cloned()
                .ok_or_else(|| crate::error::Error::other("not found"))
        }
        fn delete(&self, rel: &str) -> Result<()> {
            self.files.lock().unwrap().remove(rel);
            self.blobs.lock().unwrap().remove(rel);
            Ok(())
        }
    }

    #[test]
    fn apply_pushes_and_records_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"content").unwrap();

        let lib = Library::open_in_memory(dir.path()).unwrap();
        let transport = FakeTransport {
            files: std::sync::Mutex::new(Manifest::new()),
            blobs: std::sync::Mutex::new(BTreeMap::new()),
        };

        let local = m(&[("a.jpg", blake3::hash(b"content").to_hex().as_ref())]);
        let p = plan(&local, &Manifest::new(), &Manifest::new());

        let out = apply(&lib, &transport, &p, dir.path(), false).unwrap();
        assert_eq!(out.pushed, 1);
        assert_eq!(transport.manifest().unwrap().len(), 1);
        // The agreement is now recorded, so a re-plan is a no-op.
        let p2 = plan(&local, &lib.synced_manifest().unwrap(), &transport.manifest().unwrap());
        assert_eq!(p2.count(Action::Push), 0);
    }

    #[test]
    fn apply_refuses_deletes_unless_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open_in_memory(dir.path()).unwrap();
        let transport = FakeTransport {
            files: std::sync::Mutex::new(m(&[("gone.jpg", "H")])),
            blobs: std::sync::Mutex::new(BTreeMap::new()),
        };
        lib.record_synced("gone.jpg", "H").unwrap();

        let p = plan(&Manifest::new(), &lib.synced_manifest().unwrap(), &transport.manifest().unwrap());
        assert!(p.is_destructive());

        // Not confirmed → nothing is removed.
        let out = apply(&lib, &transport, &p, dir.path(), false).unwrap();
        assert_eq!(out.deleted, 0);
        assert_eq!(transport.manifest().unwrap().len(), 1);

        // Confirmed → removed.
        let out = apply(&lib, &transport, &p, dir.path(), true).unwrap();
        assert_eq!(out.deleted, 1);
        assert!(transport.manifest().unwrap().is_empty());
    }
}
