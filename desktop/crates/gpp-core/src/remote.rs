//! Album-level operations against a remote.
//!
//! This is what makes a second machine useful: it can *discover* what the
//! server holds, *adopt* an album it has never seen, and *contribute* one of
//! its own — all without either side mirroring the other.
//!
//! Two levels of granularity: `*_album` works on one album's own files, and
//! `*_path` works on a whole path — the folders above it, the album or
//! collection itself, and everything under it. The path form is the one the UI
//! uses, because an album without its folders is unreachable in the gallery.
//!
//! Nothing outside the requested path is read, written or deleted, which is
//! what keeps a machine holding three albums out of two hundred safe.

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
    /// A folder that holds sub-albums rather than photos. Shown as part of the
    /// tree so a path can be synced whole.
    pub is_collection: bool,
    /// How deep the path sits, for indentation.
    pub depth: usize,
}

/// What a pull produced.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PullOutcome {
    pub album_path: String,
    pub files_pulled: usize,
    pub photos_imported: usize,
    pub skipped_unchanged: usize,
    pub conflicts: Vec<String>,
    /// Every album this operation touched, shallowest first: the folders above
    /// the path, the path itself, and everything under it.
    pub albums: Vec<String>,
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
    /// Every album this operation touched, shallowest first.
    pub albums: Vec<String>,
    /// Parent folders the server already had, configured differently from this
    /// machine's copy. Left exactly as they were — push the folder itself to
    /// change it on purpose.
    pub folders_left_alone: Vec<String>,
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

        // Read straight from the remote index.md — cheap, and it means the
        // picker shows real names and real folders, not slugs and guesses.
        let parsed = transport
            .get(&format!("{path}/index.md"))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|text| parse_frontmatter(&text))
            .unwrap_or_default();

        out.push(RemoteAlbum {
            local: lib.album_by_path(&path)?.is_some(),
            remote: true,
            tracked: subs
                .iter()
                .find(|s| s.album_path == path)
                .map(|s| s.direction),
            is_collection: parsed.is_collection,
            depth: path.matches('/').count(),
            title: parsed.title,
            path,
            file_count,
        });
    }

    // Albums this machine authored that the server has never seen. Listing them
    // is what makes "add an album from anywhere" a one-click push instead of a
    // path typed by hand.
    for album in lib.albums()? {
        if out.iter().any(|a| a.path == album.path) {
            continue;
        }
        out.push(RemoteAlbum {
            tracked: subs
                .iter()
                .find(|s| s.album_path == album.path)
                .map(|s| s.direction),
            is_collection: album.is_collection,
            depth: album.path.matches('/').count(),
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
    let outcome = pull_one(lib, transport, album_path, published_root)?;
    track_default(lib, album_path, SyncDirection::Both)?;
    Ok(outcome)
}

/// Pull one album without subscribing to it.
///
/// Separate from [`pull_album`] because pulling a path also pulls the folders
/// above it, and those folders must not quietly become subscriptions — a
/// subscription on `2026` would drag in every album of the year.
fn pull_one(
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
        // This album's own files only. A sub-album's photos belong in the
        // sub-album's folder, not flattened into this one — `pull_path` walks
        // the tree and gives each album its own turn.
        if !is_direct_child(album_path, &change.path) {
            continue;
        }
        let filename = change.path.rsplit('/').next().unwrap_or_default();
        let is_metadata = filename == "index.md" || filename == "body.md";

        // A pull is an explicit "adopt the server's version". For metadata that
        // is unambiguous: index.md and body.md are *derived* from the catalog,
        // which this function has just overwritten from the server anyway, so
        // keeping the local bytes would only strand the file in a conflict it
        // could never leave. Media is different — the published copy has a
        // library original behind it, and that is never overwritten blindly.
        let action = match &change.action {
            Action::Conflict if is_metadata => &Action::Pull,
            other => other,
        };

        match action {
            Action::Pull => {
                let bytes = transport.get(&change.path)?;
                let hash = blake3::hash(&bytes).to_hex().to_string();

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
            // Nothing to fetch, but the baseline may still be out of date.
            // Settled by the same rule `sync::apply` uses on the push side —
            // a pull-only machine has no other pass to put its books right.
            Action::ForgetState => {
                let published = published_root.join(&change.path);
                match std::fs::read(&published) {
                    Ok(bytes) => {
                        let hash = blake3::hash(&bytes).to_hex().to_string();
                        lib.record_synced(&change.path, &hash)?;
                    }
                    Err(_) => lib.forget_synced(&change.path)?,
                }
                outcome.skipped_unchanged += 1;
            }
            _ => outcome.skipped_unchanged += 1,
        }
    }

    // --- 3. Catalog the pulled media -------------------------------------
    let summary = import_dir(lib, &album_dir, &ImportOptions { recursive: false, ..Default::default() }, None, None)?;
    outcome.photos_imported = summary.imported + summary.updated;

    // --- 4. Membership and order -----------------------------------------
    // Only photos sitting directly in this album's folder. A prefix match would
    // make a collection swallow every photo of every album beneath it, and
    // publishing would then copy them all into the collection's own directory.
    let photos = lib.photos(&crate::model::PhotoFilter {
        text: None,
        ..Default::default()
    })?;
    let own: Vec<&crate::model::Photo> = photos
        .iter()
        .filter(|p| is_direct_child(album_path, &p.rel_path))
        .collect();

    // A folder in the chain has no photos of its own, and asking to add none
    // of them to a collection is a question the catalog is right to refuse.
    if !own.is_empty() {
        lib.add_photos_to_album(album_path, &own.iter().map(|p| p.id).collect::<Vec<_>>())?;
    }

    if !parsed.photo_order.is_empty() {
        let ordered: Vec<i64> = parsed
            .photo_order
            .iter()
            .filter_map(|name| own.iter().find(|p| &p.filename == name).map(|p| p.id))
            .collect();
        lib.reorder_album(album_path, &ordered)?;
    }

    lib.mark_album_synced(album_path)?;
    outcome.albums.push(album_path.to_string());
    Ok(outcome)
}

// ------------------------------------------------------------- whole paths
//
// Everything above works on one album. These work on a *path*: the folders
// above it, the album or collection itself, and everything under it — because
// an album without its folders is not reachable in the gallery, and a folder
// without its albums is empty.

/// Folder paths above `path`, shallowest first. `2026/weddings/ana` yields
/// `["2026", "2026/weddings"]`.
pub fn ancestors_of(path: &str) -> Vec<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    (1..segments.len())
        .map(|depth| segments[..depth].join("/"))
        .collect()
}

/// True when `candidate` is `path` itself or lives under it.
fn at_or_under(path: &str, candidate: &str) -> bool {
    candidate == path || candidate.starts_with(&format!("{path}/"))
}

/// Sort album paths so a parent always comes before its children.
fn shallowest_first(paths: &mut [String]) {
    paths.sort_by_key(|p| (p.matches('/').count(), p.clone()));
}

/// Adopt a whole path: its folders, itself, and everything under it.
///
/// Ordering matters — a child album created before its parent collection would
/// leave the gallery unable to navigate to it, so parents are pulled first.
pub fn pull_path(
    lib: &Library,
    transport: &dyn RemoteTransport,
    path: &str,
    published_root: &Path,
) -> Result<PullOutcome> {
    let remote = transport.manifest()?;
    let on_server = sync::albums_in_manifest(&remote);

    let mut targets: Vec<String> = ancestors_of(path)
        .into_iter()
        .filter(|a| on_server.contains(a))
        .collect();
    targets.extend(on_server.iter().filter(|a| at_or_under(path, a)).cloned());
    targets.dedup();
    shallowest_first(&mut targets);

    if targets.is_empty() {
        return Err(Error::AlbumNotFound(format!("{path} (on the remote)")));
    }

    let mut total = PullOutcome {
        album_path: path.to_string(),
        ..Default::default()
    };
    for album in targets {
        let one = pull_one(lib, transport, &album, published_root)?;
        total.files_pulled += one.files_pulled;
        total.photos_imported += one.photos_imported;
        total.skipped_unchanged += one.skipped_unchanged;
        total.conflicts.extend(one.conflicts);
        total.albums.push(album);
    }

    // Only the path the caller named is subscribed. Its folders came along
    // because the gallery needs them, not because this machine wants the rest
    // of what lives under them.
    track_default(lib, path, SyncDirection::Both)?;
    Ok(total)
}

/// Contribute a whole path: its folders, itself, and everything under it.
pub fn push_path(
    lib: &Library,
    transport: &dyn RemoteTransport,
    path: &str,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
) -> Result<PushOutcome> {
    let mut total = PushOutcome {
        album_path: path.to_string(),
        ..Default::default()
    };

    // 1. The folders above the path. Only their index.md is sent, one file at a
    //    time — never a prefix scope, which would sweep in albums under the same
    //    folder that belong to other machines.
    for ancestor in ancestors_of(path) {
        if lib.album_by_path(&ancestor)?.is_none() {
            continue;
        }
        publish::publish_album(lib, &ancestor, published_root, publish_opts)?;
        match create_remote_index_if_absent(
            lib, transport, &ancestor, published_root,
        )? {
            AncestorResult::Created => {
                total.files_pushed += 1;
                total.albums.push(ancestor);
            }
            AncestorResult::AlreadyThere => total.albums.push(ancestor),
            AncestorResult::LeftAlone => total.folders_left_alone.push(ancestor),
        }
    }

    // 2. The path itself and everything under it, parents first.
    let mut subtree: Vec<String> = lib
        .albums()?
        .into_iter()
        .map(|a| a.path)
        .filter(|p| at_or_under(path, p))
        .collect();
    shallowest_first(&mut subtree);

    if subtree.is_empty() {
        return Err(Error::AlbumNotFound(path.to_string()));
    }
    for album in &subtree {
        publish::publish_album(lib, album, published_root, publish_opts)?;
    }

    // One plan for the whole subtree: the scope is the path, so nothing outside
    // it is even considered, let alone deleted.
    let local = publish::manifest_of(published_root)?;
    let synced = lib.synced_manifest()?;
    let remote = transport.manifest()?;
    let plan = sync::plan_album(path, SyncDirection::Push, &local, &synced, &remote);
    let applied = sync::apply(lib, transport, &plan, published_root, allow_deletes)?;

    for album in &subtree {
        lib.mark_album_synced(album)?;
    }
    // One subscription, for the path that was asked for — not one per album
    // underneath it.
    track_default(lib, path, SyncDirection::Push)?;

    total.files_pushed += applied.pushed;
    total.deleted_remote = applied.deleted;
    total.withheld_deletes = applied.withheld_deletes;
    total.conflicts = applied.conflicts;
    total.skipped = applied.skipped;
    total.albums.extend(subtree);
    Ok(total)
}

/// What happened to one ancestor folder during a push.
enum AncestorResult {
    /// The server had no such folder; ours now makes the album reachable.
    Created,
    /// The server's copy is byte-identical to ours. Nothing to do.
    AlreadyThere,
    /// The server has its own version. Deliberately not touched.
    LeftAlone,
}

/// Create an ancestor folder's `index.md` on the server **only if it is absent**.
///
/// Pushing `2026/weddings/ana-ivan` needs `2026/weddings` to exist, and that is
/// the entire claim it makes. It is not a claim about that folder's title,
/// password or internal token — another machine may have configured those, and
/// this one auto-generated its own when it created the album locally.
/// Overwriting would silently reset a shared folder's settings and invalidate
/// the access cookies that reference its token.
///
/// To change a folder deliberately, push that folder: `gpp push 2026/weddings`
/// puts it inside the plan's scope, where three-way reconciliation applies.
fn create_remote_index_if_absent(
    lib: &Library,
    transport: &dyn RemoteTransport,
    album_path: &str,
    published_root: &Path,
) -> Result<AncestorResult> {
    let key = format!("{album_path}/index.md");
    let path = published_root.join(&key);
    let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
    let hash = blake3::hash(&bytes).to_hex().to_string();

    match transport.manifest()?.get(&key) {
        Some(remote_hash) if *remote_hash == hash => Ok(AncestorResult::AlreadyThere),
        Some(_) => Ok(AncestorResult::LeftAlone),
        None => {
            transport.put(&key, &bytes)?;
            lib.record_synced(&key, &hash)?;
            Ok(AncestorResult::Created)
        }
    }
}

/// Sync a whole path in the requested direction.
pub fn sync_path(
    lib: &Library,
    transport: &dyn RemoteTransport,
    path: &str,
    direction: SyncDirection,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
) -> Result<sync::SyncOutcome> {
    match direction {
        SyncDirection::Pull => {
            let pulled = pull_path(lib, transport, path, published_root)?;
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                conflicts: pulled.conflicts,
                ..Default::default()
            })
        }
        SyncDirection::Push => {
            let pushed = push_path(
                lib, transport, path, published_root, publish_opts, allow_deletes,
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
            // Pull first so local edits land on top of the newest remote state.
            // A path absent from the server is not an error here: it just means
            // this machine is the one contributing it.
            let pulled = match pull_path(lib, transport, path, published_root) {
                Ok(p) => p,
                Err(Error::AlbumNotFound(_)) => PullOutcome::default(),
                Err(e) => return Err(e),
            };
            let pushed = push_path(
                lib, transport, path, published_root, publish_opts, allow_deletes,
            )?;
            lib.track_album(path, SyncDirection::Both)?;

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

/// True when `file` sits directly inside `album_path`, not in a sub-album.
fn is_direct_child(album_path: &str, file: &str) -> bool {
    file.strip_prefix(album_path)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|name| !name.contains('/'))
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
        albums: vec![album_path.to_string()],
        folders_left_alone: Vec::new(),
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
    let subs = lib.album_subscriptions()?;
    let mut out = Vec::new();

    for sub in &subs {
        // A tracked folder already carries its albums, so syncing a child that
        // sits under another subscription with the same direction would just
        // repeat the work.
        let covered_by_parent = subs.iter().any(|other| {
            other.album_path != sub.album_path
                && other.direction == sub.direction
                && sub.album_path.starts_with(&format!("{}/", other.album_path))
        });
        if covered_by_parent {
            continue;
        }

        let outcome = sync_path(
            lib,
            transport,
            &sub.album_path,
            sub.direction,
            published_root,
            publish_opts,
            allow_deletes,
        )?;
        out.push((sub.album_path.clone(), outcome));
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
