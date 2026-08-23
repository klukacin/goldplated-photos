//! Three-way sync between the local published tree and the server.
//!
//! The problem this exists to solve: `rsync --delete` cannot distinguish
//! *"I deleted this on purpose"* from *"this machine never had it"*. Mirroring
//! from a laptop holding a subset of the library therefore wipes the server.
//!
//! Keeping a third state — what we last agreed on with the server — makes the
//! distinction decidable. [`plan`] is a pure function over three manifests, so
//! the entire decision table is unit-tested.
//!
//! # The rules this module is here to keep
//!
//! - **An album nobody tracks is never touched, on either side.** Scope comes
//!   from an explicit subscription ([`AlbumSubscription`]); everything else is
//!   filtered out of all three manifests before a single decision is made, so
//!   it cannot be pushed, pulled or deleted even by a bug in the table.
//! - **Both sides changed since the baseline is a conflict, and a conflict is
//!   reported, never guessed.** Nothing here knows which version the
//!   photographer wants, and inventing an answer is how the wrong one wins.
//! - **Removing anything from the server needs an explicit `allow_deletes`.**
//!   Withheld deletions are named in the outcome so a UI can ask, and the run
//!   repeated once someone has said yes.
//! - **`.meta/` is the server's own** — the proofing submissions live there —
//!   and dot-names are excluded from every manifest, in both directions.
//! - **A remote manifest is a document the server writes**, so every path in it
//!   is checked before use (`accepts_remote_path`). An honest server never
//!   names a path that fails; a compromised one would only have to name
//!   `.htaccess` once for the next deploy to rsync it to the live site.
//! - **One file's failure never stops the transfer.** [`apply`] carries on and
//!   names the casualty in [`SyncOutcome::failed`], because a gallery that
//!   arrives one frame short otherwise looks exactly like a clean run.

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
///
/// Every `rel_path` is relative to the sync root and `/`-separated whatever the
/// platform underneath uses. An implementation has to refuse anything that
/// would resolve outside that root — including the Windows shapes a POSIX check
/// waves through, a backslash separator and a `C:` drive letter — because the
/// root is frequently a mounted share with the rest of someone's work beside it.
///
/// Implementations are called from several threads at once (see
/// [`TRANSFER_CONCURRENCY`]), hence the `Send + Sync` bound.
pub trait RemoteTransport: Send + Sync {
    /// Hashes of everything currently on the server, under the sync scope.
    fn manifest(&self) -> Result<Manifest>;
    /// Write one file, creating whatever folders it needs, replacing whatever
    /// is there. The plan has already decided this file should be replaced; a
    /// transport that second-guessed it would make the plan a lie.
    fn put(&self, rel_path: &str, bytes: &[u8]) -> Result<()>;
    /// Read one file whole. There is no streaming form because a gallery file
    /// is a JPEG or a few kilobytes of frontmatter — but the length is the
    /// server's claim, so an implementation still has to cap what it accepts.
    fn get(&self, rel_path: &str) -> Result<Vec<u8>>;
    /// Remove one file from the server. Only ever reached from
    /// [`Action::DeleteRemote`], which [`apply`] refuses to act on unless the
    /// caller passed `allow_deletes`.
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
    /// The chosen direction excludes this change (e.g. a local edit during a
    /// pull-only sync). Reported so the UI can say what it did not do.
    Skip,
}

/// One path, and what the plan says should happen to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedChange {
    /// Relative to the published tree, `/`-separated. The same key names the
    /// same file on both sides of the wire — that is what makes three
    /// separately-built manifests comparable at all.
    pub path: String,
    pub action: Action,
}

/// Everything a sync run is about to do, and nothing it has done.
///
/// Building a plan reads three manifests and writes nothing, which is what lets
/// the UI show a photographer the deletions *before* they agree to them.
/// [`apply`] is the only thing that acts on one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncPlan {
    /// One entry per path any of the three manifests mentioned, in path order.
    /// Entries that move no bytes are still here when the bookkeeping needs
    /// settling — a baseline out of step with reality is what later reads as a
    /// deliberate deletion.
    pub changes: Vec<PlannedChange>,
}

impl SyncPlan {
    /// The changes carrying one action — the list behind "12 files to upload",
    /// and the one a confirmation dialog reads out before a deletion.
    pub fn of(&self, action: Action) -> Vec<&PlannedChange> {
        self.changes.iter().filter(|c| c.action == action).collect()
    }

    /// How many changes carry one action.
    pub fn count(&self, action: Action) -> usize {
        self.changes.iter().filter(|c| c.action == action).count()
    }

    /// Whether anything diverged on both sides. Nothing here resolves one, so a
    /// plan that has conflicts is a plan a person still has to look at.
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

/// Which way a sync is allowed to move data.
///
/// Chosen per album, per run: a machine can push the wedding it just shot while
/// pulling last year's albums it never had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncDirection {
    /// Local is authoritative: upload changes, never overwrite local files.
    Push,
    /// Remote is authoritative: download changes, never modify the server.
    Pull,
    /// Full three-way reconciliation with conflict detection.
    Both,
}

impl SyncDirection {
    /// The value stored in the catalog's `album_sync` table. Stable: it is
    /// written into every library out there, so it is not a display string to
    /// be reworded.
    pub fn as_str(self) -> &'static str {
        match self {
            SyncDirection::Push => "push",
            SyncDirection::Pull => "pull",
            SyncDirection::Both => "both",
        }
    }

    /// Read a stored direction back. Total rather than fallible: an
    /// unrecognised value becomes [`SyncDirection::Both`], so one odd row can
    /// never be the reason a whole sync refuses to start. The cost is that a
    /// direction written by some future version reads here as full
    /// reconciliation — which at least reports conflicts rather than
    /// overwriting anything.
    pub fn parse(s: &str) -> Self {
        match s {
            "push" => SyncDirection::Push,
            "pull" => SyncDirection::Pull,
            _ => SyncDirection::Both,
        }
    }
}

/// Apply a direction to the neutral three-way decision.
///
/// The one semantic addition: under `Pull` or `Both`, a file the server has and
/// this machine has never seen becomes a [`Action::Pull`] instead of
/// [`Action::LeaveAlone`] — because asking to sync *this album* is the opt-in
/// that makes adopting it safe. Albums nobody asked for are never in scope, so
/// they stay untouched.
fn decide_directional(
    local: Option<&String>,
    synced: Option<&String>,
    remote: Option<&String>,
    direction: SyncDirection,
) -> Action {
    let base = decide(local, synced, remote);
    match direction {
        SyncDirection::Both => match base {
            Action::LeaveAlone => Action::Pull,
            other => other,
        },
        SyncDirection::Push => match base {
            // Never let the server overwrite local work in push mode.
            Action::Pull => Action::Skip,
            other => other,
        },
        SyncDirection::Pull => match base {
            // Never modify the server in pull mode.
            Action::Push | Action::DeleteRemote => Action::Skip,
            Action::LeaveAlone => Action::Pull,
            other => other,
        },
    }
}

/// Plan the sync of a single album.
///
/// All three manifests are filtered to the album's subtree first, so nothing
/// outside it can possibly be affected — that is what makes per-album sync
/// safe on a machine holding a fraction of the library.
pub fn plan_album(
    album_path: &str,
    direction: SyncDirection,
    local: &Manifest,
    synced: &Manifest,
    remote: &Manifest,
) -> SyncPlan {
    let scope = SyncScope::with(vec![album_path.to_string()]);
    plan_scoped(&scope, direction, local, synced, remote)
}

/// Plan a sync restricted to a scope, in a given direction.
pub fn plan_scoped(
    scope: &SyncScope,
    direction: SyncDirection,
    local: &Manifest,
    synced: &Manifest,
    remote: &Manifest,
) -> SyncPlan {
    let (local, synced, remote) = (
        scope.filter(local),
        scope.filter(synced),
        scope.filter(remote),
    );

    let mut paths: BTreeSet<&String> = BTreeSet::new();
    paths.extend(local.keys());
    paths.extend(synced.keys());
    paths.extend(remote.keys());

    let mut changes = Vec::new();
    for path in paths {
        let action = decide_directional(
            local.get(path),
            synced.get(path),
            remote.get(path),
            direction,
        );
        // Nothing to transfer. Skip it only when the books are already right:
        // a baseline that disagrees with the local side still has to be
        // brought up to date, and dropping the change here is what left stale
        // rows behind for `apply` to misread later.
        if action == Action::ForgetState && synced.get(path) == local.get(path) {
            continue;
        }
        changes.push(PlannedChange {
            path: path.clone(),
            action,
        });
    }
    SyncPlan { changes }
}

/// Album paths present in a manifest, derived from the `index.md` entries.
///
/// Used to answer "what is on the server that this machine does not have?".
pub fn albums_in_manifest(manifest: &Manifest) -> Vec<String> {
    let mut out: Vec<String> = manifest
        .keys()
        .filter_map(|k| k.strip_suffix("/index.md"))
        .map(|s| s.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Restrict a sync to a subtree, so a machine can own part of the library.
#[derive(Debug, Clone, Default)]
pub struct SyncScope {
    /// Path prefixes to include. Empty means the whole tree.
    pub prefixes: Vec<String>,
}

impl SyncScope {
    /// No restriction at all. Only sound on a machine that really does hold the
    /// whole library — anywhere else, this is the setting that lets a partial
    /// catalog decide the server is missing two hundred albums.
    pub fn everything() -> Self {
        Self::default()
    }

    /// Restrict to these path prefixes. In practice there is exactly one — the
    /// path the photographer asked to sync — and every caller here builds it
    /// that way; the vector is what lets a future "sync these four" not need a
    /// second scope type.
    pub fn with(prefixes: Vec<String>) -> Self {
        Self { prefixes }
    }

    /// Whether a manifest path is in scope. Matching is on segment boundaries,
    /// so `2026/ana` covers `2026/ana/one.jpg` and never `2026/anastasia` —
    /// the difference between syncing one couple's wedding and a stranger's.
    pub fn includes(&self, path: &str) -> bool {
        if self.prefixes.is_empty() {
            return true;
        }
        self.prefixes
            .iter()
            .any(|p| path == p || path.starts_with(&format!("{p}/")))
    }

    /// The manifest cut down to what is in scope. Every plan begins with all
    /// three manifests passed through here, which is the real safety property:
    /// a path that does not survive this filter cannot be pushed, pulled or
    /// deleted whatever the decision table would have said about it.
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

    /// Drop the bookkeeping row for a path.
    ///
    /// It deletes nothing but the memory of an agreement. Keeping a row past
    /// the file's life is the more dangerous mistake: the day another machine
    /// republishes those bytes, a stale baseline reads as "deleted here on
    /// purpose" and the file comes off the server again.
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
///
/// Counts for what moved, names for everything that did not. A run whose only
/// non-zero field is a count is the one clean result; every other field exists
/// so that a partial run cannot be mistaken for it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncOutcome {
    /// Files uploaded, downloaded, and removed from the server.
    pub pushed: usize,
    pub pulled: usize,
    pub deleted: usize,
    /// Files the server has that this machine has no record of. Untouched —
    /// the case the three-way design exists for.
    pub left_alone: usize,
    /// Changes the chosen direction excluded.
    pub skipped: usize,
    /// Server files this run would have removed but didn't, because
    /// `allow_deletes` was false. Naming them lets a caller ask, then re-run.
    pub withheld_deletes: Vec<String>,
    /// Paths that moved on both sides since the baseline. Nothing was
    /// transferred for these in either direction: which version the
    /// photographer wants is not a question this code can answer.
    pub conflicts: Vec<String>,
    /// Path and error for each file that did not make it. [`apply`] carries on
    /// past one so the rest of the album still moves, which only works if the
    /// casualty is named — a gallery arriving one frame short otherwise looks
    /// exactly like a clean run, and the client is the one who notices.
    pub failed: Vec<(String, String)>,
}

/// How many transfers run at once.
///
/// A single TCP flow starts at a ~14 KB congestion window and ramps
/// geometrically, so one-at-a-time leaves most of an upstream link idle no
/// matter how fast it is. Parallel flows ramp independently and fill it. Four
/// to eight is the useful band — rclone defaults to four — and above that you
/// mostly buy packet loss. See dev-docs/sync-transport.md §5.
pub const TRANSFER_CONCURRENCY: usize = 6;

/// What one change turned into. Collected in parallel, folded in order.
enum Applied {
    Pushed,
    Pulled,
    Deleted,
    WithheldDelete(String),
    Conflict(String),
    LeftAlone,
    Skipped,
    Failed(String, String),
    Nothing,
}

/// Execute a plan against a transport.
///
/// `allow_deletes` must be an explicit, confirmed decision by the caller —
/// nothing is removed from the server without it.
///
/// Transfers run concurrently. The catalog writes they trigger serialize on the
/// connection mutex, which is fine: they are microseconds against a network
/// round trip. Results are gathered and folded afterwards so the outcome is
/// deterministic regardless of the order threads happen to finish in.
pub fn apply(
    lib: &Library,
    transport: &dyn RemoteTransport,
    plan: &SyncPlan,
    local_root: &std::path::Path,
    allow_deletes: bool,
) -> Result<SyncOutcome> {
    use rayon::prelude::*;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(TRANSFER_CONCURRENCY)
        .build()
        .map_err(|e| crate::error::Error::other(format!("thread pool: {e}")))?;

    let applied: Vec<Applied> = pool.install(|| {
        plan.changes
            .par_iter()
            .map(|change| apply_one(lib, transport, change, local_root, allow_deletes))
            .collect()
    });

    let mut out = SyncOutcome::default();
    for result in applied {
        match result {
            Applied::Pushed => out.pushed += 1,
            Applied::Pulled => out.pulled += 1,
            Applied::Deleted => out.deleted += 1,
            Applied::WithheldDelete(p) => out.withheld_deletes.push(p),
            Applied::Conflict(p) => out.conflicts.push(p),
            Applied::LeftAlone => out.left_alone += 1,
            Applied::Skipped => out.skipped += 1,
            Applied::Failed(p, e) => out.failed.push((p, e)),
            Applied::Nothing => {}
        }
    }
    Ok(out)
}

/// Place a manifest path under the local root, refusing anything that would
/// land outside it.
///
/// Manifest paths come from the server, and both halves of a transfer trust
/// them against the local disk: a push reads that file and uploads it, a pull
/// overwrites it. Every caller scopes its plan to one album today, which
/// already excludes a climbing path — this makes the guarantee belong to the
/// transfer rather than to the callers who happen to precede it.
fn local_under(root: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('\\') || rel.contains('\0') {
        return None;
    }
    let mut out = root.to_path_buf();
    for segment in rel.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return None,
            s => out.push(s),
        }
    }
    // Catches what pushing a segment can still do on Windows — a drive letter
    // or a backslash-separated path replaces the root rather than extending it.
    out.starts_with(root).then_some(out)
}

/// Whether this machine is willing to write the file a remote manifest names.
///
/// Two rules, and the local side already applies both — it is only what a
/// *server* names that has never been held to them.
///
/// A path has to name a file, and name it the same way twice: no empty
/// segment, no `.` or `..`, nothing absolute, no NUL, and no backslash, which
/// is a separator on Windows where `a\..\..\x` climbs straight out of the
/// album. [`apply`] gets that from [`local_under`], but the download half of a
/// pull does not go through `apply`: it writes to the library and to the
/// published tree itself, and takes the last segment as a filename.
///
/// And no segment may begin with a dot. `.meta/` is the site's own — the
/// proofing submissions live there — and an `.htaccess` beside an album's
/// photos is configuration the web server obeys. [`crate::publish::manifest_of`]
/// and [`FsTransport::walk`] both skip dot-names, so nothing this machine
/// publishes or pushes can be one; the published tree is what the deploy
/// rsyncs to the live site, and once a dot-file is in it neither manifest ever
/// mentions it again, so nothing here could reconcile or remove it.
pub(crate) fn accepts_remote_path(rel: &str) -> bool {
    if rel.contains('\0') || rel.contains('\\') {
        return false;
    }
    !rel.is_empty()
        && rel
            .split('/')
            .all(|segment| !segment.is_empty() && !segment.starts_with('.'))
}

/// One change, in isolation. Never returns `Err`: a single file failing is
/// reported and the rest of the transfer continues.
fn apply_one(
    lib: &Library,
    transport: &dyn RemoteTransport,
    change: &PlannedChange,
    local_root: &std::path::Path,
    allow_deletes: bool,
) -> Applied {
    let failed = |e: crate::error::Error| Applied::Failed(change.path.clone(), e.to_string());
    let Some(local_path) = local_under(local_root, &change.path) else {
        return failed(crate::error::Error::InvalidPath(change.path.clone()));
    };

    match change.action {
        Action::Push => {
            let bytes = match std::fs::read(&local_path) {
                Ok(b) => b,
                Err(e) => return Applied::Failed(change.path.clone(), e.to_string()),
            };
            if let Err(e) = transport.put(&change.path, &bytes) {
                return failed(e);
            }
            let hash = blake3::hash(&bytes).to_hex().to_string();
            match lib.record_synced(&change.path, &hash) {
                Ok(()) => Applied::Pushed,
                Err(e) => failed(e),
            }
        }
        Action::Pull => {
            let bytes = match transport.get(&change.path) {
                Ok(b) => b,
                Err(e) => return failed(e),
            };
            if let Some(parent) = local_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Err(e) = std::fs::write(&local_path, &bytes) {
                return Applied::Failed(change.path.clone(), e.to_string());
            }
            let hash = blake3::hash(&bytes).to_hex().to_string();
            match lib.record_synced(&change.path, &hash) {
                Ok(()) => Applied::Pulled,
                Err(e) => failed(e),
            }
        }
        Action::DeleteRemote => {
            if !allow_deletes {
                return Applied::WithheldDelete(change.path.clone());
            }
            if let Err(e) = transport.delete(&change.path) {
                return failed(e);
            }
            match lib.forget_synced(&change.path) {
                Ok(()) => Applied::Deleted,
                Err(e) => failed(e),
            }
        }
        Action::Conflict => Applied::Conflict(change.path.clone()),
        Action::LeaveAlone => Applied::LeftAlone,
        Action::Skip => Applied::Skipped,
        // Nothing moves, but the bookkeeping still has to be settled, and which
        // way depends on what is actually here. A file gone from both sides
        // must stop being remembered: a stale baseline reads as "I deleted this
        // on purpose" the day another machine republishes those bytes, and the
        // file is taken off the server again. A file both sides already hold
        // needs that agreement written down, or the next ordinary remote edit
        // is measured against a baseline neither side holds and reported as a
        // conflict nothing can clear.
        Action::ForgetState => match std::fs::read(&local_path) {
            Ok(bytes) => {
                let hash = blake3::hash(&bytes).to_hex().to_string();
                match lib.record_synced(&change.path, &hash) {
                    Ok(()) => Applied::Nothing,
                    Err(e) => failed(e),
                }
            }
            Err(_) => match lib.forget_synced(&change.path) {
                Ok(()) => Applied::Nothing,
                Err(e) => failed(e),
            },
        },
    }
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

        // Not confirmed → nothing is removed, but the caller is told what it
        // would have removed, so it can ask and re-run.
        let out = apply(&lib, &transport, &p, dir.path(), false).unwrap();
        assert_eq!(out.deleted, 0);
        assert_eq!(out.withheld_deletes, vec!["gone.jpg".to_string()]);
        assert_eq!(transport.manifest().unwrap().len(), 1);

        // Confirmed → removed.
        let out = apply(&lib, &transport, &p, dir.path(), true).unwrap();
        assert_eq!(out.deleted, 1);
        assert!(transport.manifest().unwrap().is_empty());
    }

    fn fake_transport() -> FakeTransport {
        FakeTransport {
            files: std::sync::Mutex::new(Manifest::new()),
            blobs: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Every path in a plan came from the server's manifest, and both halves of
    /// a transfer trust it against the local disk: a push reads that file, a
    /// pull overwrites it. One that climbs out of the tree must be refused
    /// rather than followed.
    #[test]
    fn a_path_that_climbs_out_of_the_local_tree_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let published = dir.path().join("published");
        std::fs::create_dir_all(&published).unwrap();
        let outside = dir.path().join("precious.txt");
        std::fs::write(&outside, b"not part of any gallery").unwrap();

        let lib = Library::open_in_memory(&published).unwrap();
        let transport = fake_transport();
        transport.put("../precious.txt", b"overwritten").unwrap();

        let plan = SyncPlan {
            changes: vec![PlannedChange {
                path: "../precious.txt".to_string(),
                action: Action::Pull,
            }],
        };
        let out = apply(&lib, &transport, &plan, &published, false).unwrap();

        assert_eq!(out.pulled, 0);
        assert_eq!(out.failed.len(), 1, "the path should be reported, not followed");
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"not part of any gallery",
            "a file outside the published tree was overwritten"
        );
    }

    /// The rule the download half of a pull applies to a server's manifest.
    ///
    /// Stated here as well as exercised through `remote::pull_one`, because two
    /// of the names below cannot be demonstrated end to end on a Unix host: a
    /// backslash is an ordinary character in a Linux filename and a separator
    /// on Windows, so `a\..\..\x` is contained here and climbs out of the
    /// library there.
    #[test]
    fn a_remote_manifest_path_has_to_name_a_file_and_not_a_dot_file() {
        for ok in ["2026/ana/a1.jpg", "index.md", "2026/ana-i-ivan/index.md"] {
            assert!(accepts_remote_path(ok), "{ok:?} is an ordinary album file");
        }
        for refused in [
            "",
            "/etc/passwd",
            "2026/../../escaped.jpg",
            "2026/ana/..",
            "2026/ana/.",
            "2026/ana/",
            "2026//ana/a.jpg",
            "2026/ana/a\0.jpg",
            // Windows separators, which `PathBuf::join` follows there.
            "2026/ana/..\\..\\..\\startup.exe",
            "C:\\Windows\\system32\\x.dll",
            // The site's own: proofing submissions and web-server config.
            "2026/ana/.htaccess",
            "2026/ana/.meta/proofing/x.json",
        ] {
            assert!(
                !accepts_remote_path(refused),
                "{refused:?} must not be written on any platform"
            );
        }
    }

    /// The folder transport is the reference the other transports copy, and it
    /// is the one pointed at a mounted network share — so its idea of "inside
    /// the remote root" is the one that decides whether a manifest can reach
    /// the rest of that share.
    ///
    /// [`local_under`] already refuses a leading backslash and re-checks
    /// containment at the end; this had neither. Both matter only on Windows,
    /// where a backslash separates path segments and `C:` is a root of its own,
    /// so `PathBuf::push` throws the remote root away rather than extending it
    /// — and this is a unit test on the path logic precisely because a Linux
    /// run cannot show that.
    #[test]
    fn the_folder_transport_refuses_a_path_that_would_leave_its_root() {
        let t = FsTransport::new("/srv/gallery/albums");
        for ok in ["2026/ana/a1.jpg", "index.md", "./2026/ana/index.md"] {
            assert!(t.resolve(ok).is_ok(), "{ok:?} is an ordinary album file");
        }
        for refused in [
            "",
            "/etc/passwd",
            "2026/../../escaped.jpg",
            "2026/ana/a\0.jpg",
            // Windows separators and roots. `push` treats each as absolute
            // there, and the remote root silently stops applying.
            "\\\\fileserver\\share\\x.jpg",
            "\\Windows\\system32\\x.dll",
        ] {
            let Err(e) = t.resolve(refused) else {
                panic!("{refused:?} must not resolve inside the remote root");
            };
            assert!(matches!(e, crate::error::Error::InvalidPath(_)), "{e}");
        }
    }

    /// A file gone from both sides has to stop being remembered. Keeping the
    /// row means that the day another machine republishes that photo, this one
    /// reads its own stale baseline as "I deleted this on purpose" and takes it
    /// off the server again — the exact loss the third state exists to prevent.
    #[test]
    fn a_file_gone_from_both_sides_stops_being_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open_in_memory(dir.path()).unwrap();
        let transport = fake_transport();

        let bytes = b"the photograph";
        let hash = blake3::hash(bytes).to_hex().to_string();
        lib.record_synced("a/x.jpg", &hash).unwrap();

        // It is gone here and gone there — someone removed it on the server.
        let p = plan_album(
            "a",
            SyncDirection::Both,
            &Manifest::new(),
            &lib.synced_manifest().unwrap(),
            &Manifest::new(),
        );
        apply(&lib, &transport, &p, dir.path(), true).unwrap();
        assert!(
            lib.synced_manifest().unwrap().is_empty(),
            "a file gone from both sides is still on the books"
        );

        // Another machine now republishes exactly those bytes.
        transport.put("a/x.jpg", bytes).unwrap();
        let p2 = plan_album(
            "a",
            SyncDirection::Both,
            &Manifest::new(),
            &lib.synced_manifest().unwrap(),
            &transport.manifest().unwrap(),
        );
        assert_eq!(
            p2.count(Action::DeleteRemote),
            0,
            "a file this machine never had must not be deleted from the server"
        );
    }

    /// Both sides arrived at the same bytes without either knowing. There is
    /// nothing to transfer — but they *do* now agree, and that has to be
    /// written down. Left at the older baseline, the next ordinary remote edit
    /// is read as a divergence and reported as a conflict that no number of
    /// syncs can clear.
    #[test]
    fn two_sides_that_converged_on_their_own_end_up_agreeing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("a/x.jpg"), b"both landed here").unwrap();
        let both = blake3::hash(b"both landed here").to_hex().to_string();

        let lib = Library::open_in_memory(dir.path()).unwrap();
        let transport = fake_transport();
        transport.put("a/x.jpg", b"both landed here").unwrap();
        lib.record_synced("a/x.jpg", "the-older-baseline").unwrap();

        let local = m(&[("a/x.jpg", both.as_str())]);
        let p = plan_album(
            "a",
            SyncDirection::Both,
            &local,
            &lib.synced_manifest().unwrap(),
            &transport.manifest().unwrap(),
        );
        apply(&lib, &transport, &p, dir.path(), false).unwrap();
        assert_eq!(
            lib.synced_manifest().unwrap().get("a/x.jpg").map(String::as_str),
            Some(both.as_str()),
            "the two sides agree, so that is what the baseline must say"
        );

        // The server alone moves on. That is a plain pull, not a conflict.
        transport.put("a/x.jpg", b"the server moved on").unwrap();
        let p2 = plan_album(
            "a",
            SyncDirection::Both,
            &local,
            &lib.synced_manifest().unwrap(),
            &transport.manifest().unwrap(),
        );
        assert_eq!(p2.count(Action::Conflict), 0, "nothing here diverged");
        assert_eq!(p2.count(Action::Pull), 1);
    }
}

// -------------------------------------------------------- album subscriptions

/// A machine's interest in one album.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumSubscription {
    /// The album's gallery path. It follows the album: renaming or moving one
    /// carries its subscription, and its sub-albums', to the new path — left
    /// behind, a subscription silently stops syncing the album, with no error
    /// and only a client's gallery that never updates again to show for it.
    pub album_path: String,
    /// The direction the *user* chose. One-off pushes and pulls never rewrite
    /// it: a pull-only machine stays pull-only after it pulls.
    pub direction: SyncDirection,
    /// When a sync of this album last completed, RFC 3339. `None` until one
    /// does — subscribing records an interest, not a transfer.
    pub last_synced_at: Option<String>,
}

impl Library {
    /// Albums this machine syncs. Anything not listed is out of scope: never
    /// pushed, never pulled, never deleted.
    pub fn album_subscriptions(&self) -> Result<Vec<AlbumSubscription>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT album_path, direction, last_synced_at FROM album_sync \
                 ORDER BY album_path",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(AlbumSubscription {
                    album_path: r.get(0)?,
                    direction: SyncDirection::parse(&r.get::<_, String>(1)?),
                    last_synced_at: r.get(2)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// One album's subscription, or `None` when this machine does not track it.
    /// `None` is the answer for most of the library on most machines, and it is
    /// what keeps the rest of it out of every plan.
    pub fn album_subscription(&self, album_path: &str) -> Result<Option<AlbumSubscription>> {
        Ok(self
            .album_subscriptions()?
            .into_iter()
            .find(|s| s.album_path == album_path))
    }

    /// Start syncing an album (or change its direction).
    pub fn track_album(&self, album_path: &str, direction: SyncDirection) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO album_sync(album_path, direction) VALUES(?1, ?2) \
                 ON CONFLICT(album_path) DO UPDATE SET direction = excluded.direction",
                params![album_path, direction.as_str()],
            )?;
            Ok(())
        })
    }

    /// Stop syncing an album. Files stay where they are on both sides — this
    /// only removes it from scope.
    pub fn untrack_album(&self, album_path: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "DELETE FROM album_sync WHERE album_path = ?1",
                params![album_path],
            )?;
            Ok(())
        })
    }

    pub(crate) fn mark_album_synced(&self, album_path: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.with_conn(|c| {
            c.execute(
                "UPDATE album_sync SET last_synced_at = ?1 WHERE album_path = ?2",
                params![now, album_path],
            )?;
            Ok(())
        })
    }
}

// ------------------------------------------------------------- fs transport

/// A directory as the remote.
///
/// Useful in three real situations, not just tests: a mounted network share, an
/// external drive carried between machines, and a folder kept in sync by
/// something else (Dropbox, Syncthing). It is also the reference implementation
/// the SFTP and HTTP transports must behave like.
pub struct FsTransport {
    root: std::path::PathBuf,
}

impl FsTransport {
    /// `root` is the folder standing in for the server — the mount point or the
    /// drive's sync directory, not a path inside it. Every transferred path is
    /// resolved beneath it and refused if it would land anywhere else.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Reject anything that would escape the remote root.
    ///
    /// The same two rules as [`local_under`], and for the same reason: a
    /// backslash separates segments on Windows and `C:` is a root of its own
    /// there, so `push` throws the remote root away instead of extending it.
    /// The remote root is often a mounted share, and everything else on that
    /// share is one such path away.
    fn resolve(&self, rel: &str) -> Result<std::path::PathBuf> {
        if rel.is_empty()
            || rel.starts_with('/')
            || rel.starts_with('\\')
            || rel.contains('\0')
        {
            return Err(crate::error::Error::InvalidPath(rel.to_string()));
        }
        let mut out = self.root.clone();
        for seg in rel.split('/') {
            match seg {
                "" | "." => continue,
                ".." => return Err(crate::error::Error::InvalidPath(rel.to_string())),
                s => out.push(s),
            }
        }
        if !out.starts_with(&self.root) {
            return Err(crate::error::Error::InvalidPath(rel.to_string()));
        }
        Ok(out)
    }

    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Manifest) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir).map_err(|e| crate::error::Error::io(dir, e))? {
            let entry = entry.map_err(|e| crate::error::Error::io(dir, e))?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // `.meta/` is server-owned (thumbnail cache, proofing submissions).
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                Self::walk(&path, root, out)?;
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
}

impl RemoteTransport for FsTransport {
    fn manifest(&self) -> Result<Manifest> {
        let mut out = Manifest::new();
        Self::walk(&self.root, &self.root, &mut out)?;
        Ok(out)
    }

    fn put(&self, rel_path: &str, bytes: &[u8]) -> Result<()> {
        let dest = self.resolve(rel_path)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| crate::error::Error::io(parent, e))?;
        }
        std::fs::write(&dest, bytes).map_err(|e| crate::error::Error::io(&dest, e))
    }

    fn get(&self, rel_path: &str) -> Result<Vec<u8>> {
        let src = self.resolve(rel_path)?;
        std::fs::read(&src).map_err(|e| crate::error::Error::io(&src, e))
    }

    fn delete(&self, rel_path: &str) -> Result<()> {
        let target = self.resolve(rel_path)?;
        if target.exists() {
            std::fs::remove_file(&target).map_err(|e| crate::error::Error::io(&target, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod direction_tests {
    use super::*;

    fn d(local: Option<&str>, synced: Option<&str>, remote: Option<&str>, dir: SyncDirection) -> Action {
        decide_directional(
            local.map(String::from).as_ref(),
            synced.map(String::from).as_ref(),
            remote.map(String::from).as_ref(),
            dir,
        )
    }

    /// The headline case: a brand-new machine adopting an album.
    #[test]
    fn new_machine_pulls_everything_it_opted_into() {
        assert_eq!(d(None, None, Some("A"), SyncDirection::Pull), Action::Pull);
        assert_eq!(d(None, None, Some("A"), SyncDirection::Both), Action::Pull);
        // But push-only must not invent local files.
        assert_eq!(d(None, None, Some("A"), SyncDirection::Push), Action::LeaveAlone);
    }

    #[test]
    fn pull_never_writes_to_the_server() {
        assert_eq!(d(Some("A"), None, None, SyncDirection::Pull), Action::Skip);
        assert_eq!(d(None, Some("A"), Some("A"), SyncDirection::Pull), Action::Skip);
    }

    #[test]
    fn push_never_overwrites_local_files() {
        assert_eq!(d(Some("A"), Some("A"), Some("B"), SyncDirection::Push), Action::Skip);
        assert_eq!(d(Some("B"), Some("A"), Some("A"), SyncDirection::Push), Action::Push);
    }

    #[test]
    fn conflicts_survive_every_direction() {
        for dir in [SyncDirection::Push, SyncDirection::Pull, SyncDirection::Both] {
            assert_eq!(
                d(Some("B"), Some("A"), Some("C"), dir),
                Action::Conflict,
                "direction {dir:?} must still surface divergence"
            );
        }
    }

    #[test]
    fn album_plan_ignores_everything_outside_the_album() {
        let local: Manifest = Manifest::new();
        let synced = Manifest::new();
        let remote: Manifest = [
            ("2026/ana/index.md", "h1"),
            ("2026/ana/a.jpg", "h2"),
            ("2019/old/index.md", "h3"),
            ("2019/old/x.jpg", "h4"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let p = plan_album("2026/ana", SyncDirection::Pull, &local, &synced, &remote);
        assert_eq!(p.count(Action::Pull), 2, "only the requested album");
        assert!(
            p.changes.iter().all(|c| c.path.starts_with("2026/ana/")),
            "nothing outside the album may appear in the plan"
        );
    }

    #[test]
    fn discovers_albums_from_a_manifest() {
        let m: Manifest = [
            ("2026/ana/index.md", "h"),
            ("2026/ana/a.jpg", "h"),
            ("2026/ana/day2/index.md", "h"),
            ("stray.jpg", "h"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(albums_in_manifest(&m), vec!["2026/ana", "2026/ana/day2"]);
    }

    #[test]
    fn subscriptions_scope_and_persist() {
        let lib = Library::open_in_memory("/tmp/lib").unwrap();
        assert!(lib.album_subscriptions().unwrap().is_empty());

        lib.track_album("2026/ana", SyncDirection::Pull).unwrap();
        lib.track_album("2026/ivo", SyncDirection::Push).unwrap();
        let subs = lib.album_subscriptions().unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].direction, SyncDirection::Pull);

        // Re-tracking changes the direction rather than duplicating.
        lib.track_album("2026/ana", SyncDirection::Both).unwrap();
        assert_eq!(lib.album_subscriptions().unwrap().len(), 2);
        assert_eq!(
            lib.album_subscription("2026/ana").unwrap().unwrap().direction,
            SyncDirection::Both
        );

        lib.untrack_album("2026/ana").unwrap();
        assert_eq!(lib.album_subscriptions().unwrap().len(), 1);
    }

    #[test]
    fn fs_transport_round_trips_and_blocks_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let t = FsTransport::new(dir.path());

        t.put("2026/ana/index.md", b"---\ntitle: \"X\"\n---\n").unwrap();
        t.put("2026/ana/a.jpg", b"bytes").unwrap();

        let m = t.manifest().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(t.get("2026/ana/a.jpg").unwrap(), b"bytes");

        t.delete("2026/ana/a.jpg").unwrap();
        assert_eq!(t.manifest().unwrap().len(), 1);

        assert!(t.put("../escape.txt", b"x").is_err());
        assert!(t.get("/etc/passwd").is_err());
    }

    #[test]
    fn fs_transport_hides_server_owned_meta() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/.meta/proofing")).unwrap();
        std::fs::write(dir.path().join("a/.meta/proofing/x.json"), "{}").unwrap();
        std::fs::write(dir.path().join("a/index.md"), "---\n---\n").unwrap();

        let m = FsTransport::new(dir.path()).manifest().unwrap();
        assert_eq!(m.len(), 1);
        assert!(!m.keys().any(|k| k.contains(".meta")));
    }
}

// ------------------------------------------------------------ http transport

/// The server half of a sync, over plain HTTP.
///
/// Three deliberate choices, all argued in `dev-docs/sync-transport.md`:
///
/// **HTTP/1.1, pinned.** Stock Apache and nginx both cap an HTTP/2 request body
/// at a 64 KB flow-control window, which puts a per-stream ceiling of
/// `window ÷ RTT` on uploads — about an eighth of a 100 Mbit/s line at 40 ms.
/// HTTP/2's multiplexing is a download win; for uploads it trades N congestion
/// windows for one. `ureq` speaks only HTTP/1.1, so this is free.
///
/// **One manifest request for the whole scope.** The client then diffs locally
/// and sends only what is missing, which is the entire benefit rsync ever gave
/// us here — a JPEG is new or unchanged, never byte-edited, so block-level
/// deltas were always dead weight.
///
/// **A connection pool, shared across threads.** `sync::apply` runs transfers
/// concurrently; a pooled agent amortises the TLS handshake to nothing.
pub struct HttpTransport {
    agent: ureq::Agent,
    /// Base URL of the sync endpoints, without a trailing slash.
    base: String,
    /// Shared secret proving this client may write. Sent as a bearer token.
    token: String,
    /// Restricts every request to one subtree, so a misconfigured client cannot
    /// enumerate or overwrite the whole gallery.
    scope: Option<String>,
}

/// One entry of the manifest the server returns.
#[derive(Debug, Deserialize)]
struct RemoteEntry {
    path: String,
    hash: String,
}

#[derive(Debug, Deserialize)]
struct ManifestResponse {
    files: Vec<RemoteEntry>,
}

impl HttpTransport {
    /// `base` is the URL of the sync API, e.g. `https://example.com/api/sync`.
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            // Long enough for a 50 MB RAW on a slow uplink; short enough that a
            // dead server does not hang the app forever.
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build();
        Self {
            agent: config.into(),
            base: base.into().trim_end_matches('/').to_string(),
            token: token.into(),
            scope: None,
        }
    }

    /// Limit every request to one album subtree.
    pub fn scoped(mut self, prefix: impl Into<String>) -> Self {
        self.scope = Some(prefix.into());
        self
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/{suffix}", self.base)
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

impl RemoteTransport for HttpTransport {
    fn manifest(&self) -> Result<Manifest> {
        let mut url = self.url("manifest");
        if let Some(scope) = &self.scope {
            url = format!("{url}?scope={}", urlencode(scope));
        }
        let body: ManifestResponse = self
            .agent
            .get(&url)
            .header("Authorization", self.auth())
            .call()
            .map_err(http_error)?
            .body_mut()
            .read_json()
            .map_err(http_error)?;

        Ok(body
            .files
            .into_iter()
            .map(|e| (e.path, e.hash))
            .collect())
    }

    fn put(&self, rel_path: &str, bytes: &[u8]) -> Result<()> {
        self.agent
            .put(&self.url(&format!("file?path={}", urlencode(rel_path))))
            .header("Authorization", self.auth())
            .header("Content-Type", "application/octet-stream")
            // The server verifies this before committing the file, so a
            // truncated or corrupted upload is rejected rather than published.
            .header("X-Content-Blake3", blake3::hash(bytes).to_hex().as_str())
            .send(bytes)
            .map_err(http_error)?;
        Ok(())
    }

    fn get(&self, rel_path: &str) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(&self.url(&format!("file?path={}", urlencode(rel_path))))
            .header("Authorization", self.auth())
            .call()
            .map_err(http_error)?;
        response
            .body_mut()
            .with_config()
            // A single gallery file; the cap stops a hostile or broken server
            // from exhausting memory.
            .limit(512 * 1024 * 1024)
            .read_to_vec()
            .map_err(http_error)
    }

    fn delete(&self, rel_path: &str) -> Result<()> {
        self.agent
            .delete(&self.url(&format!("file?path={}", urlencode(rel_path))))
            .header("Authorization", self.auth())
            .call()
            .map_err(http_error)?;
        Ok(())
    }
}

/// Turn a transport failure into something that names the fix.
///
/// ureq renders a rejected request as bare `http status: 401`, which in a sync
/// panel reads as "broken" rather than "your token is wrong" — and those are
/// the two failures a new remote actually hits.
fn http_error(e: impl std::fmt::Display) -> crate::error::Error {
    let raw = e.to_string();
    let hint = if raw.contains("401") || raw.contains("403") {
        Some("the server rejected the access token — check it matches SYNC_TOKEN")
    } else if raw.contains("503") {
        Some("the server has no SYNC_TOKEN configured, so syncing is turned off there")
    } else if raw.contains("404") {
        Some("no sync endpoint at this address — the URL should end in /api/sync")
    } else {
        None
    };
    match hint {
        Some(hint) => crate::error::Error::other(format!("{raw} — {hint}")),
        None => crate::error::Error::other(raw),
    }
}

/// Percent-encode a query-string value.
///
/// Hand-rolled to avoid pulling a URL crate for one job: everything outside the
/// unreserved set becomes `%XX`, which is what a path or a scope prefix needs.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
