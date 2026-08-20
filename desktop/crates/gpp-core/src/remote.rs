//! Album-level operations against a remote.
//!
//! This is what makes a second machine useful: it can *discover* what the
//! server holds, *adopt* an album it has never seen, and *contribute* one of
//! its own — all without either side mirroring the other.
//!
//! Everything here is scoped to a single album. Nothing outside the album's
//! subtree is read, written or deleted, which is what keeps a machine holding
//! three albums out of two hundred safe.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::albums::{AlbumUpdate, NewAlbum};
use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::import::{import_dir, ImportOptions};
use crate::publish::{self, parse_frontmatter, PublishOptions};
use crate::sync::{
    self, Action, Manifest, RemoteTransport, SyncDirection, SyncPlan, SyncScope,
};

/// One album seen from this machine: where it exists, and how it syncs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAlbum {
    pub path: String,
    pub title: Option<String>,
    /// Files under this album on the server (0 when it is local-only).
    pub file_count: usize,
    /// Whether this machine already has the album in its catalog.
    pub local: bool,
    /// Whether the server holds this album at all.
    pub remote: bool,
    /// Whether this machine syncs it, and in which direction.
    pub tracked: Option<SyncDirection>,
}

/// What a pull produced.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PullOutcome {
    pub album_path: String,
    pub files_pulled: usize,
    pub photos_imported: usize,
    pub skipped_unchanged: usize,
    pub conflicts: Vec<String>,
}

/// What a push produced.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PushOutcome {
    pub album_path: String,
    pub files_pushed: usize,
    pub deleted_remote: usize,
    /// Server files this push left in place because deletions weren't allowed.
    /// Ask the user about these, then push again with `allow_deletes`.
    pub withheld_deletes: Vec<String>,
    pub conflicts: Vec<String>,
    pub skipped: usize,
}

/// List every album this machine could sync, from either side.
///
/// Answers both halves of the question a machine actually has: "what can I
/// pull?" (on the server, `local == false`) and "what could I contribute?"
/// (in my catalog, `remote == false`). Albums present on both sides appear
/// once. Nothing here writes anything.
pub fn remote_albums(lib: &Library, transport: &dyn RemoteTransport) -> Result<Vec<RemoteAlbum>> {
    let manifest = transport.manifest()?;
    let subs = lib.album_subscriptions()?;
    let mut out = Vec::new();

    for path in sync::albums_in_manifest(&manifest) {
        let scope = SyncScope::with(vec![path.clone()]);
        let file_count = scope.filter(&manifest).len();

        // Read the title straight from the remote index.md — cheap and it means
        // the picker shows real names, not slugs.
        let title = transport
            .get(&format!("{path}/index.md"))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| parse_frontmatter(&text).title);

        out.push(RemoteAlbum {
            local: lib.album_by_path(&path)?.is_some(),
            remote: true,
            tracked: subs
                .iter()
                .find(|s| s.album_path == path)
                .map(|s| s.direction),
            path,
            title,
            file_count,
        });
    }

    // Albums this machine authored that the server has never seen. Listing them
    // is what makes "add an album from anywhere" a one-click push instead of a
    // path typed by hand.
    for album in lib.albums()? {
        if album.is_collection || out.iter().any(|a| a.path == album.path) {
            continue;
        }
        out.push(RemoteAlbum {
            tracked: subs
                .iter()
                .find(|s| s.album_path == album.path)
                .map(|s| s.direction),
            path: album.path,
            title: Some(album.title),
            file_count: 0,
            local: true,
            remote: false,
        });
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Plan a single album's sync without touching anything.
pub fn plan_album_sync(
    lib: &Library,
    transport: &dyn RemoteTransport,
    album_path: &str,
    direction: SyncDirection,
    published_root: &Path,
) -> Result<SyncPlan> {
    let local = publish::manifest_of(published_root)?;
    let synced = lib.synced_manifest()?;
    let remote = transport.manifest()?;
    Ok(sync::plan_album(album_path, direction, &local, &synced, &remote))
}

/// Adopt an album from the server into this library.
///
/// Steps: fetch `index.md` and create/update the album row (keeping the
/// server's internal token so access cookies stay valid across machines),
/// download the photos into `<library>/<album-path>/`, import them so they are
/// first-class catalog entries, then restore the album's photo order.
pub fn pull_album(
    lib: &Library,
    transport: &dyn RemoteTransport,
    album_path: &str,
    published_root: &Path,
) -> Result<PullOutcome> {
    let mut outcome = PullOutcome {
        album_path: album_path.to_string(),
        ..Default::default()
    };

    let remote = transport.manifest()?;
    let scope = SyncScope::with(vec![album_path.to_string()]);
    let remote_scoped = scope.filter(&remote);
    if remote_scoped.is_empty() {
        return Err(Error::AlbumNotFound(format!("{album_path} (on the remote)")));
    }

    // --- 1. Album settings ------------------------------------------------
    let index_key = format!("{album_path}/index.md");
    let parsed = match transport.get(&index_key) {
        Ok(bytes) => parse_frontmatter(&String::from_utf8_lossy(&bytes)),
        Err(_) => Default::default(),
    };

    if lib.album_by_path(album_path)?.is_none() {
        lib.create_album(&NewAlbum {
            path: album_path.to_string(),
            title: parsed.title.clone(),
            description: parsed.description.clone(),
            date: parsed.date.clone(),
            is_collection: parsed.is_collection,
        })?;
    }
    lib.update_album(
        album_path,
        &AlbumUpdate {
            title: parsed.title.clone(),
            // Adopt the server's id: it is what the access cookie references.
            token: parsed.token.clone(),
            description: Some(parsed.description.clone()),
            date: Some(parsed.date.clone()),
            password: Some(parsed.password.clone()),
            share_token: Some(parsed.share_token.clone()),
            sort: parsed.sort.clone(),
            style: parsed.style.clone(),
            is_collection: Some(parsed.is_collection),
            hidden: Some(parsed.hidden),
            allow_download: Some(parsed.allow_download),
            proofing: Some(parsed.proofing),
            sort_order: Some(parsed.order),
            tags: Some(parsed.tags.clone()),
            ..Default::default()
        },
    )?;

    // --- 2. Files ---------------------------------------------------------
    // Photos land at their album path inside the library, so a pulled album has
    // a predictable home. Locally-imported albums keep membership in the DB and
    // can live anywhere.
    let album_dir = lib.resolve(album_path)?;
    std::fs::create_dir_all(&album_dir).map_err(|e| Error::io(&album_dir, e))?;

    let synced = lib.synced_manifest()?;
    let local = publish::manifest_of(published_root)?;
    let plan = sync::plan_album(album_path, SyncDirection::Pull, &local, &synced, &remote);

    for change in &plan.changes {
        match change.action {
            Action::Pull => {
                let bytes = transport.get(&change.path)?;
                let hash = blake3::hash(&bytes).to_hex().to_string();

                // Media goes into the library; index.md/body.md are metadata we
                // have already folded into the catalog, so they only need to
                // exist in the published tree.
                let filename = change.path.rsplit('/').next().unwrap_or_default();
                let is_metadata = filename == "index.md" || filename == "body.md";
                if !is_metadata {
                    let dest = album_dir.join(filename);
                    std::fs::write(&dest, &bytes).map_err(|e| Error::io(&dest, e))?;
                }

                let published = published_root.join(&change.path);
                if let Some(parent) = published.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }
                std::fs::write(&published, &bytes).map_err(|e| Error::io(&published, e))?;

                lib.record_synced(&change.path, &hash)?;
                outcome.files_pulled += 1;
            }
            Action::Conflict => outcome.conflicts.push(change.path.clone()),
            _ => outcome.skipped_unchanged += 1,
        }
    }

    // --- 3. Catalog the pulled media -------------------------------------
    let summary = import_dir(lib, &album_dir, &ImportOptions { recursive: false, ..Default::default() }, None)?;
    outcome.photos_imported = summary.imported + summary.updated;

    // --- 4. Membership and order -----------------------------------------
    let photos = lib.photos(&crate::model::PhotoFilter {
        text: None,
        ..Default::default()
    })?;
    let prefix = format!("{album_path}/");
    let ids: Vec<i64> = photos
        .iter()
        .filter(|p| p.rel_path.starts_with(&prefix))
        .map(|p| p.id)
        .collect();
    lib.add_photos_to_album(album_path, &ids)?;

    if !parsed.photo_order.is_empty() {
        let ordered: Vec<i64> = parsed
            .photo_order
            .iter()
            .filter_map(|name| {
                photos
                    .iter()
                    .find(|p| p.rel_path.starts_with(&prefix) && &p.filename == name)
                    .map(|p| p.id)
            })
            .collect();
        lib.reorder_album(album_path, &ordered)?;
    }

    track_default(lib, album_path, SyncDirection::Both)?;
    lib.mark_album_synced(album_path)?;
    Ok(outcome)
}

/// Start tracking an album, without overruling a direction already chosen.
///
/// A one-off pull must not quietly turn a pull-only machine into one that
/// pushes, and a one-off push must not stop a both-ways album from pulling.
fn track_default(lib: &Library, album_path: &str, direction: SyncDirection) -> Result<()> {
    if lib.album_subscription(album_path)?.is_none() {
        lib.track_album(album_path, direction)?;
    }
    Ok(())
}

/// Publish one album locally, then upload it.
///
/// Deletions on the server require `allow_deletes`; without it a plan that
/// would remove files reports them and moves on.
pub fn push_album(
    lib: &Library,
    transport: &dyn RemoteTransport,
    album_path: &str,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
) -> Result<PushOutcome> {
    // Materialize first, so what we upload is exactly what the gallery reads.
    publish::publish_album(lib, album_path, published_root, publish_opts)?;

    let local = publish::manifest_of(published_root)?;
    let synced = lib.synced_manifest()?;
    let remote = transport.manifest()?;
    let plan = sync::plan_album(album_path, SyncDirection::Push, &local, &synced, &remote);

    let outcome_inner = sync::apply(lib, transport, &plan, published_root, allow_deletes)?;
    track_default(lib, album_path, SyncDirection::Push)?;
    lib.mark_album_synced(album_path)?;

    Ok(PushOutcome {
        album_path: album_path.to_string(),
        files_pushed: outcome_inner.pushed,
        deleted_remote: outcome_inner.deleted,
        withheld_deletes: outcome_inner.withheld_deletes,
        conflicts: outcome_inner.conflicts,
        skipped: outcome_inner.skipped,
    })
}

/// Sync one album in the requested direction.
///
/// `Pull` adopts, `Push` contributes, `Both` reconciles.
pub fn sync_album(
    lib: &Library,
    transport: &dyn RemoteTransport,
    album_path: &str,
    direction: SyncDirection,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
) -> Result<sync::SyncOutcome> {
    match direction {
        SyncDirection::Pull => {
            let pulled = pull_album(lib, transport, album_path, published_root)?;
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                conflicts: pulled.conflicts,
                ..Default::default()
            })
        }
        SyncDirection::Push => {
            let pushed = push_album(
                lib, transport, album_path, published_root, publish_opts, allow_deletes,
            )?;
            Ok(sync::SyncOutcome {
                pushed: pushed.files_pushed,
                deleted: pushed.deleted_remote,
                withheld_deletes: pushed.withheld_deletes,
                conflicts: pushed.conflicts,
                skipped: pushed.skipped,
                ..Default::default()
            })
        }
        SyncDirection::Both => {
            // Pull first so local edits are applied on top of the newest
            // remote state, then push the result.
            let pulled = pull_album(lib, transport, album_path, published_root)?;
            let pushed = push_album(
                lib, transport, album_path, published_root, publish_opts, allow_deletes,
            )?;
            lib.track_album(album_path, SyncDirection::Both)?;

            let mut conflicts = pulled.conflicts;
            conflicts.extend(pushed.conflicts);
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                pushed: pushed.files_pushed,
                deleted: pushed.deleted_remote,
                withheld_deletes: pushed.withheld_deletes,
                conflicts,
                skipped: pushed.skipped,
                ..Default::default()
            })
        }
    }
}

/// Sync every album this machine has subscribed to, each in its own direction.
pub fn sync_tracked_albums(
    lib: &Library,
    transport: &dyn RemoteTransport,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
) -> Result<Vec<(String, sync::SyncOutcome)>> {
    let mut out = Vec::new();
    for sub in lib.album_subscriptions()? {
        let outcome = sync_album(
            lib,
            transport,
            &sub.album_path,
            sub.direction,
            published_root,
            publish_opts,
            allow_deletes,
        )?;
        out.push((sub.album_path, outcome));
    }
    Ok(out)
}

/// Manifest of a single album on the remote — handy for status displays.
pub fn remote_album_manifest(
    transport: &dyn RemoteTransport,
    album_path: &str,
) -> Result<Manifest> {
    let scope = SyncScope::with(vec![album_path.to_string()]);
    Ok(scope.filter(&transport.manifest()?))
}
