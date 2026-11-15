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
//!
//! # A pull only ever adds
//!
//! The one rule to have in mind before changing anything here. The file under
//! the library root is the negative: the develop model is non-destructive
//! precisely because Reset returns to it, and there is no second copy of it
//! anywhere. So a pull writes a photo into the library only where none is
//! there yet. Where one is, the server's version goes into the published tree
//! and the original is left exactly as it was, named in
//! [`PullOutcome::kept_originals`] for a person to judge.
//!
//! This is not a hypothetical. The plan that says "pull" compares the
//! *published* copy against the remote, and the published copy holds developed
//! pixels — so the library original was never part of that comparison, and a
//! second machine developing one frame and republishing it was once enough to
//! write its JPEG over the negative here.
//!
//! Two more guarantees ride out through the outcome types rather than through
//! errors, because in both cases the rest of the work should still happen:
//! [`PullOutcome::rejected`] names paths the server asked for that this machine
//! would not write, and [`PushOutcome::failed`] names frames that never reached
//! the server. A caller that drops either has turned a partial transfer into
//! something indistinguishable from a clean one.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::albums::{AlbumUpdate, NewAlbum};
use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::import::{import_dir, ImportOptions};
use crate::publish::{self, parse_frontmatter, PublishOptions};
use crate::sync::{
    self, Action, Manifest, RemoteTransport, SyncDirection, SyncPlan, SyncScope, SyncScopeKind,
};

/// One album seen from this machine: where it exists, and how it syncs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAlbum {
    /// Gallery path, e.g. `2026/weddings/ana-ivan`. Identical on both sides —
    /// it is the only thing that identifies a local album and a remote one as
    /// the same album.
    pub path: String,
    /// Read out of the remote `index.md` where the server has one, so the
    /// picker offers the name the photographer typed rather than a folder slug.
    /// `None` only when that file could not be read or parsed.
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
    /// The path that was asked for — often not the only album touched, since
    /// pulling one brings its folders and everything under it. `albums` is the
    /// full list.
    pub album_path: String,
    /// Files taken from the server. Each lands in the published tree; a photo
    /// the library did not already hold lands there as well.
    pub files_pulled: usize,
    /// Photos the catalog took in, new plus re-hashed. Ordinarily lower than
    /// `files_pulled`: `index.md` is a file but not a photograph.
    pub photos_imported: usize,
    /// Paths with nothing to move. Includes what a pull-only run may not act on
    /// — a non-zero count here is the normal case, not a warning.
    pub skipped_unchanged: usize,
    /// Photos that moved on both sides since the baseline; left as they are on
    /// both. Metadata never appears here: `index.md` and `body.md` are derived
    /// from the catalog this pull has just overwritten from the server anyway,
    /// so holding them in a conflict would only strand them in one forever.
    pub conflicts: Vec<String>,
    /// Photos the server had a different version of, where this library already
    /// holds an original. The published copy was updated; the original was left
    /// exactly as it was. Named so the photographer can look, not resolved by
    /// guessing — the file under the library root is the negative, and there is
    /// no second copy of it anywhere.
    pub kept_originals: Vec<String>,
    /// Paths the server offered that this machine would not write — see
    /// `sync::accepts_remote_path`. Reported rather than fatal, and rather
    /// than silent: a well-behaved server never names one, so a name here is
    /// worth a photographer's attention even though the album still arrived.
    pub rejected: Vec<String>,
    /// Metadata fields a full-scope pull found changed on both sides — a
    /// rating, flag, label, tag set or develop stack that is non-default here
    /// and different there. Reported, never resolved by guessing; the web
    /// scope never produces one.
    pub metadata_conflicts: Vec<String>,
    /// Work this pull could not do, named with the reason — the read twin of
    /// [`PushOutcome::failed`]. A develop stack carrying an op this build does
    /// not know, a metadata document that would not parse, two photographs of
    /// one album that would publish the same full-scope key. Each is one item's
    /// failure and none of them stops the rest of the transfer; a caller that
    /// drops this list has turned a partial pull into something that looks
    /// exactly like a complete one.
    pub failed: Vec<(String, String)>,
    /// Every album this operation touched, shallowest first: the folders above
    /// the path, the path itself, and everything under it.
    pub albums: Vec<String>,
}

/// What a push produced.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PushOutcome {
    /// The path that was asked for. `albums` names everything it reached.
    pub album_path: String,
    /// Files that reached the server, the ancestor folders' `index.md`
    /// included.
    pub files_pushed: usize,
    /// Files removed from the server — only ever non-zero when the caller
    /// passed `allow_deletes`.
    pub deleted_remote: usize,
    /// Server files this push left in place because deletions weren't allowed.
    /// Ask the user about these, then push again with `allow_deletes`.
    pub withheld_deletes: Vec<String>,
    /// Paths that diverged. Nothing was uploaded for them: a push does not get
    /// to rule that this machine's copy is the right one.
    pub conflicts: Vec<String>,
    /// Changes the push direction excluded. A file the server holds a newer
    /// version of stays newer on the server.
    pub skipped: usize,
    /// Files that never reached the server — a dropped connection, a refused
    /// upload, a full disk. `sync::apply` deliberately carries on past one so
    /// the rest of the album still goes up; throwing its report away here made
    /// a gallery missing a frame indistinguishable from a clean push, and the
    /// photographer heard about it from the client.
    pub failed: Vec<(String, String)>,
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
    let remote_id = lib.ensure_default_remote()?;
    remote_albums_for(lib, remote_id, transport)
}

/// [`remote_albums`], against one specific remote's subscriptions.
pub fn remote_albums_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
) -> Result<Vec<RemoteAlbum>> {
    let manifest = transport.manifest()?;
    let subs = lib.album_subscriptions_for(remote_id)?;
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
    let remote_id = lib.ensure_default_remote()?;
    plan_album_sync_for(lib, remote_id, transport, album_path, direction, published_root)
}

/// [`plan_album_sync`], against one specific remote's baselines.
pub fn plan_album_sync_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    album_path: &str,
    direction: SyncDirection,
    published_root: &Path,
) -> Result<SyncPlan> {
    let local = publish::manifest_of(published_root)?;
    let synced = lib.synced_manifest_for(remote_id)?;
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
    let remote_id = lib.ensure_default_remote()?;
    pull_album_for(lib, remote_id, transport, album_path, published_root, SyncScopeKind::Web)
}

/// [`pull_album`] against one remote, in a chosen scope. `Full` additionally
/// pulls the album's originals and metadata from the `__gpp_full__/`
/// namespace — see [`crate::full`].
pub fn pull_album_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    album_path: &str,
    published_root: &Path,
    scope: SyncScopeKind,
) -> Result<PullOutcome> {
    let full_roots = lib.full_scope_albums()?;
    let mut outcome = pull_one(
        lib, remote_id, transport, album_path, published_root,
        may_adopt_media(scope, &full_roots, album_path),
    )?;
    if scope == SyncScopeKind::Full {
        crate::full::pull_full(lib, remote_id, transport, album_path, &mut outcome)?;
    }
    track_default(lib, remote_id, album_path, SyncDirection::Both, scope)?;
    Ok(outcome)
}

/// Whether a pull of `album_path` may make the published bytes this library's
/// copy of a photo it does not hold.
///
/// Only a web-scope pass ever may, and only where no full-scope subscription
/// covers the album — its own or an ancestor's (`full_roots` comes from
/// [`Library::full_scope_albums`], which spans every remote because the file
/// under the library root does too).
///
/// The answer is read from the subscription table rather than from whatever the
/// current pass happens to be doing, because the two are not the same thing and
/// the difference cost originals. `sync_tracked_albums_for` runs each
/// subscription separately: a folder tracked `web` and an album inside it
/// tracked `full` are two passes, and the subscriptions come back `ORDER BY
/// album_path`, so the folder — the shallower path — always runs first. Its web
/// pull adopted the gallery's published bytes as the library's copy of every
/// photo underneath, and the album's own full pull then found the file present
/// and quite correctly refused to overwrite it. The library's "originals" were
/// the gallery's developed, HEIC→JPEG-converted, rating-filtered pixels, the
/// RAW never landed at all, and the only thing said about it was a
/// `kept_originals` entry that reads like a safety message. Deciding per album
/// from what is stored is what makes the answer independent of which pass runs
/// first: no ordering of subscriptions can reintroduce the substitution.
fn may_adopt_media(scope: SyncScopeKind, full_roots: &[String], album_path: &str) -> bool {
    scope == SyncScopeKind::Web && !full_roots.iter().any(|root| at_or_under(root, album_path))
}

/// Pull one album without subscribing to it.
///
/// Separate from [`pull_album`] because pulling a path also pulls the folders
/// above it, and those folders must not quietly become subscriptions — a
/// subscription on `2026` would drag in every album of the year.
fn pull_one(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    album_path: &str,
    published_root: &Path,
    // Whether the web tree's media may become the *library's* copy of a photo
    // the library does not hold. True for a web-scope pull — the published
    // pixels are the best available stand-in for a negative that machine will
    // never see. False under full scope, where the genuine original follows
    // through the `__gpp_full__/` namespace and writing the gallery's
    // developed pixels into the library first would block it.
    adopt_media_into_library: bool,
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
    //
    // A failed fetch of the settings must not read as "the album has none".
    // The update below writes `Some(None)` / `Some(false)` clear-values for
    // every field the parse did not find, so treating a transport error as an
    // empty frontmatter stripped password, shareToken and tags off the local
    // album — and a Both-direction sync then pushed the stripped `index.md`
    // back to the server. So: a fetch of an `index.md` the manifest names is
    // allowed to fail the album, exactly as a failed photo fetch below does —
    // the error rides out to the same per-album failure reporting
    // (`sync_tracked_albums` catches it against this album's path). An album
    // the manifest holds no `index.md` for simply has no settings to adopt,
    // and the local ones are left alone.
    let index_key = format!("{album_path}/index.md");
    let parsed = if remote.contains_key(&index_key) {
        let bytes = transport.get(&index_key)?;
        Some(parse_frontmatter(&String::from_utf8_lossy(&bytes)))
    } else {
        None
    };

    if lib.album_by_path(album_path)?.is_none() {
        lib.create_album(&NewAlbum {
            path: album_path.to_string(),
            title: parsed.as_ref().and_then(|p| p.title.clone()),
            description: parsed.as_ref().and_then(|p| p.description.clone()),
            date: parsed.as_ref().and_then(|p| p.date.clone()),
            is_collection: parsed.as_ref().is_some_and(|p| p.is_collection),
        })?;
    }
    if let Some(parsed) = &parsed {
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
    }

    // --- 2. Files ---------------------------------------------------------
    // Photos land at their album path inside the library, so a pulled album has
    // a predictable home. Locally-imported albums keep membership in the DB and
    // can live anywhere.
    let album_dir = lib.resolve(album_path)?;
    std::fs::create_dir_all(&album_dir).map_err(|e| Error::io(&album_dir, e))?;

    let synced = lib.synced_manifest_for(remote_id)?;
    let local = publish::manifest_of(published_root)?;
    let plan = sync::plan_album(album_path, SyncDirection::Pull, &local, &synced, &remote);

    for change in &plan.changes {
        // This album's own files only. A sub-album's photos belong in the
        // sub-album's folder, not flattened into this one — `pull_path` walks
        // the tree and gives each album its own turn.
        if !is_direct_child(album_path, &change.path) {
            continue;
        }
        // The only place a server's own string reaches this machine's disk
        // without `sync::apply` in front of it. Checked before anything is
        // fetched, because the two writes below go to two different roots and
        // the last segment is also taken as a filename.
        if !sync::accepts_remote_path(&change.path) {
            outcome.rejected.push(change.path.clone());
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

                if !is_metadata && adopt_media_into_library {
                    let dest = album_dir.join(filename);
                    // Only ever *add* a photo to the library. Bringing an album
                    // this machine has never seen is the point of a pull, and
                    // for that the file is simply not there yet.
                    //
                    // When it is there, it is the photographer's original, and
                    // the plan that said "pull" never looked at it: the local
                    // side of that comparison is the published tree, which
                    // holds developed pixels. So a second machine developing
                    // one frame and republishing it was enough to write its
                    // JPEG over the negative here — with the whole develop
                    // model resting on that negative being the thing Reset
                    // returns to, and no other copy of it in existence.
                    if dest.exists() {
                        outcome.kept_originals.push(change.path.clone());
                    } else {
                        std::fs::write(&dest, &bytes).map_err(|e| Error::io(&dest, e))?;
                    }
                }

                let published = published_root.join(&change.path);
                if let Some(parent) = published.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }
                std::fs::write(&published, &bytes).map_err(|e| Error::io(&published, e))?;

                lib.record_synced_for(remote_id, &change.path, &hash)?;
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
                        lib.record_synced_for(remote_id, &change.path, &hash)?;
                    }
                    Err(_) => lib.forget_synced_for(remote_id, &change.path)?,
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
    // The primary source only: a pull lays its files down under the library
    // root, and a referenced photo on another drive could carry a rel_path
    // that reads like this album's folder while being an entirely different
    // shoot on an entirely different disk.
    let primary = lib.primary_source_id()?;
    let own: Vec<&crate::model::Photo> = photos
        .iter()
        .filter(|p| p.source_id == primary && is_direct_child(album_path, &p.rel_path))
        .collect();

    // A folder in the chain has no photos of its own, and asking to add none
    // of them to a collection is a question the catalog is right to refuse.
    if !own.is_empty() {
        lib.add_photos_to_album(album_path, &own.iter().map(|p| p.id).collect::<Vec<_>>())?;
    }

    if let Some(parsed) = parsed.as_ref().filter(|p| !p.photo_order.is_empty()) {
        // The server's photoOrder names *published* files, and a published
        // name is not always the library one: a HEIC ships as `.jpg`
        // (`publish::published_filename`). Matching on the library filename
        // alone missed every HEIC frame, so a pull reordered the album as if
        // those photos had not been named at all. The library filename stays
        // as a fallback for entries that predate the rename rule.
        let ordered: Vec<i64> = parsed
            .photo_order
            .iter()
            .filter_map(|name| {
                own.iter()
                    .find(|p| &publish::published_filename(p) == name || &p.filename == name)
                    .map(|p| p.id)
            })
            .collect();
        lib.reorder_album(album_path, &ordered)?;
    }

    lib.mark_album_synced_for(remote_id, album_path)?;
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
    let remote_id = lib.ensure_default_remote()?;
    pull_path_for(lib, remote_id, transport, path, published_root, SyncScopeKind::Web)
}

/// [`pull_path`] against one remote, in a chosen scope.
pub fn pull_path_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    path: &str,
    published_root: &Path,
    scope: SyncScopeKind,
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
    let full_roots = lib.full_scope_albums()?;
    for album in targets {
        let one = pull_one(
            lib, remote_id, transport, &album, published_root,
            may_adopt_media(scope, &full_roots, &album),
        )?;
        total.files_pulled += one.files_pulled;
        total.photos_imported += one.photos_imported;
        total.skipped_unchanged += one.skipped_unchanged;
        total.conflicts.extend(one.conflicts);
        // All three of these are warnings, and this is the form the UI calls,
        // so dropping them here is the same as never producing them: a kept
        // original said the server disagrees about a negative there is only one
        // copy of, a rejected path said the server asked for something no
        // honest one asks for, and a failure said part of the album is not
        // here. None of them reached a screen.
        total.kept_originals.extend(one.kept_originals);
        total.rejected.extend(one.rejected);
        total.failed.extend(one.failed);
        total.albums.push(album);
    }

    // Full scope: the originals and metadata ride in after the web tree, so
    // album rows and memberships already exist for them to land on.
    if scope == SyncScopeKind::Full {
        crate::full::pull_full(lib, remote_id, transport, path, &mut total)?;
    }

    // Only the path the caller named is subscribed. Its folders came along
    // because the gallery needs them, not because this machine wants the rest
    // of what lives under them.
    track_default(lib, remote_id, path, SyncDirection::Both, scope)?;
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
    let remote_id = lib.ensure_default_remote()?;
    push_path_for(
        lib, remote_id, transport, path, published_root, publish_opts, allow_deletes,
        SyncScopeKind::Web,
    )
}

/// [`push_path`] against one remote, in a chosen scope. `Full` additionally
/// pushes every catalogued original of the subtree's albums (RAW included)
/// and a metadata document per album, under `__gpp_full__/` — with the same
/// three-manifest planning, per-remote baselines and `allow_deletes`
/// semantics as the web half.
#[allow(clippy::too_many_arguments)]
pub fn push_path_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    path: &str,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
    scope: SyncScopeKind,
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
            lib, remote_id, transport, &ancestor, published_root,
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
    let synced = lib.synced_manifest_for(remote_id)?;
    let remote = transport.manifest()?;
    let plan = sync::plan_album(path, SyncDirection::Push, &local, &synced, &remote);
    let applied = sync::apply_for(lib, remote_id, transport, &plan, published_root, allow_deletes)?;

    total.files_pushed += applied.pushed;
    total.deleted_remote = applied.deleted;
    total.withheld_deletes = applied.withheld_deletes;
    total.conflicts = applied.conflicts;
    total.skipped = applied.skipped;
    total.failed = applied.failed;

    // Full scope: the originals and metadata follow the web tree up.
    if scope == SyncScopeKind::Full {
        crate::full::push_full(
            lib, remote_id, transport, path, &subtree, allow_deletes, &mut total,
        )?;
    }

    for album in &subtree {
        lib.mark_album_synced_for(remote_id, album)?;
    }
    // One subscription, for the path that was asked for — not one per album
    // underneath it.
    track_default(lib, remote_id, path, SyncDirection::Push, scope)?;

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
    remote_id: i64,
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
            lib.record_synced_for(remote_id, &key, &hash)?;
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
    let remote_id = lib.ensure_default_remote()?;
    sync_path_for(
        lib, remote_id, transport, path, direction, published_root, publish_opts,
        allow_deletes, SyncScopeKind::Web,
    )
}

/// [`sync_path`] against one remote, in a chosen scope.
#[allow(clippy::too_many_arguments)]
pub fn sync_path_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    path: &str,
    direction: SyncDirection,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
    scope: SyncScopeKind,
) -> Result<sync::SyncOutcome> {
    match direction {
        SyncDirection::Pull => {
            let pulled = pull_path_for(lib, remote_id, transport, path, published_root, scope)?;
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                conflicts: merged_conflicts(pulled.conflicts, pulled.metadata_conflicts),
                failed: pulled.failed,
                ..Default::default()
            })
        }
        SyncDirection::Push => {
            let pushed = push_path_for(
                lib, remote_id, transport, path, published_root, publish_opts, allow_deletes,
                scope,
            )?;
            Ok(sync::SyncOutcome {
                pushed: pushed.files_pushed,
                deleted: pushed.deleted_remote,
                withheld_deletes: pushed.withheld_deletes,
                conflicts: pushed.conflicts,
                skipped: pushed.skipped,
                failed: pushed.failed,
                ..Default::default()
            })
        }
        SyncDirection::Both => {
            // Pull first so local edits land on top of the newest remote state.
            // A path absent from the server is not an error here: it just means
            // this machine is the one contributing it.
            let pulled = match pull_path_for(lib, remote_id, transport, path, published_root, scope)
            {
                Ok(p) => p,
                Err(Error::AlbumNotFound(_)) => PullOutcome::default(),
                Err(e) => return Err(e),
            };
            let pushed = push_path_for(
                lib, remote_id, transport, path, published_root, publish_opts, allow_deletes,
                scope,
            )?;
            lib.track_album_for(path, SyncDirection::Both, Some(scope), remote_id)?;

            let mut conflicts = merged_conflicts(pulled.conflicts, pulled.metadata_conflicts);
            conflicts.extend(pushed.conflicts);
            let mut failed = pulled.failed;
            failed.extend(pushed.failed);
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                pushed: pushed.files_pushed,
                deleted: pushed.deleted_remote,
                withheld_deletes: pushed.withheld_deletes,
                conflicts,
                skipped: pushed.skipped,
                failed,
                ..Default::default()
            })
        }
    }
}

/// File conflicts and metadata conflicts, one list for the outcome shape that
/// only has one.
fn merged_conflicts(mut conflicts: Vec<String>, metadata: Vec<String>) -> Vec<String> {
    conflicts.extend(metadata);
    conflicts
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
fn track_default(
    lib: &Library,
    remote_id: i64,
    album_path: &str,
    direction: SyncDirection,
    scope: SyncScopeKind,
) -> Result<()> {
    if lib.album_subscription_for(remote_id, album_path)?.is_none() {
        lib.track_album_for(album_path, direction, Some(scope), remote_id)?;
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
    let remote_id = lib.ensure_default_remote()?;
    push_album_for(
        lib, remote_id, transport, album_path, published_root, publish_opts, allow_deletes,
        SyncScopeKind::Web,
    )
}

/// [`push_album`] against one remote, in a chosen scope.
#[allow(clippy::too_many_arguments)]
pub fn push_album_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    album_path: &str,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
    scope: SyncScopeKind,
) -> Result<PushOutcome> {
    // Materialize first, so what we upload is exactly what the gallery reads.
    publish::publish_album(lib, album_path, published_root, publish_opts)?;

    let local = publish::manifest_of(published_root)?;
    let synced = lib.synced_manifest_for(remote_id)?;
    let remote = transport.manifest()?;
    let plan = sync::plan_album(album_path, SyncDirection::Push, &local, &synced, &remote);

    let outcome_inner = sync::apply_for(lib, remote_id, transport, &plan, published_root, allow_deletes)?;

    let mut out = PushOutcome {
        album_path: album_path.to_string(),
        files_pushed: outcome_inner.pushed,
        deleted_remote: outcome_inner.deleted,
        withheld_deletes: outcome_inner.withheld_deletes,
        conflicts: outcome_inner.conflicts,
        skipped: outcome_inner.skipped,
        failed: outcome_inner.failed,
        albums: vec![album_path.to_string()],
        folders_left_alone: Vec::new(),
    };
    if scope == SyncScopeKind::Full {
        // The whole subtree, not just this album — because the plan inside
        // `push_full` scopes `__gpp_full__/<path>`, which *is* the whole
        // subtree. Handed one album's keys as the local side, every original a
        // sub-album had pushed earlier read as locally absent: DeleteRemote
        // with `allow_deletes`, and a withheld-delete report naming files
        // nobody had deleted without it. The local side has to cover exactly
        // what the plan looks at, which is what `push_path_for` already does.
        let mut subtree: Vec<String> = lib
            .albums()?
            .into_iter()
            .map(|a| a.path)
            .filter(|p| at_or_under(album_path, p))
            .collect();
        shallowest_first(&mut subtree);
        crate::full::push_full(
            lib, remote_id, transport, album_path, &subtree, allow_deletes, &mut out,
        )?;
    }
    track_default(lib, remote_id, album_path, SyncDirection::Push, scope)?;
    lib.mark_album_synced_for(remote_id, album_path)?;

    Ok(out)
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
    let remote_id = lib.ensure_default_remote()?;
    sync_album_for(
        lib, remote_id, transport, album_path, direction, published_root, publish_opts,
        allow_deletes, SyncScopeKind::Web,
    )
}

/// [`sync_album`] against one remote, in a chosen scope.
#[allow(clippy::too_many_arguments)]
pub fn sync_album_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    album_path: &str,
    direction: SyncDirection,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
    scope: SyncScopeKind,
) -> Result<sync::SyncOutcome> {
    match direction {
        SyncDirection::Pull => {
            let pulled =
                pull_album_for(lib, remote_id, transport, album_path, published_root, scope)?;
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                conflicts: merged_conflicts(pulled.conflicts, pulled.metadata_conflicts),
                failed: pulled.failed,
                ..Default::default()
            })
        }
        SyncDirection::Push => {
            let pushed = push_album_for(
                lib, remote_id, transport, album_path, published_root, publish_opts,
                allow_deletes, scope,
            )?;
            Ok(sync::SyncOutcome {
                pushed: pushed.files_pushed,
                deleted: pushed.deleted_remote,
                withheld_deletes: pushed.withheld_deletes,
                conflicts: pushed.conflicts,
                skipped: pushed.skipped,
                failed: pushed.failed,
                ..Default::default()
            })
        }
        SyncDirection::Both => {
            // Pull first so local edits are applied on top of the newest
            // remote state, then push the result.
            let pulled =
                pull_album_for(lib, remote_id, transport, album_path, published_root, scope)?;
            let pushed = push_album_for(
                lib, remote_id, transport, album_path, published_root, publish_opts,
                allow_deletes, scope,
            )?;
            lib.track_album_for(album_path, SyncDirection::Both, Some(scope), remote_id)?;

            let mut conflicts = merged_conflicts(pulled.conflicts, pulled.metadata_conflicts);
            conflicts.extend(pushed.conflicts);
            let mut failed = pulled.failed;
            failed.extend(pushed.failed);
            Ok(sync::SyncOutcome {
                pulled: pulled.files_pulled,
                pushed: pushed.files_pushed,
                deleted: pushed.deleted_remote,
                withheld_deletes: pushed.withheld_deletes,
                conflicts,
                skipped: pushed.skipped,
                failed,
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
    let remote_id = lib.ensure_default_remote()?;
    sync_tracked_albums_for(lib, remote_id, transport, published_root, publish_opts, allow_deletes)
}

/// [`sync_tracked_albums`], for one remote's subscriptions — each album in its
/// own direction *and its own scope*.
pub fn sync_tracked_albums_for(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    published_root: &Path,
    publish_opts: &PublishOptions,
    allow_deletes: bool,
) -> Result<Vec<(String, sync::SyncOutcome)>> {
    let subs = lib.album_subscriptions_for(remote_id)?;
    let mut out = Vec::new();

    for sub in &subs {
        // A tracked folder already carries its albums, so syncing a child that
        // sits under another subscription with the same direction would just
        // repeat the work.
        let covered_by_parent = subs.iter().any(|other| {
            other.album_path != sub.album_path
                && other.direction == sub.direction
                && other.scope == sub.scope
                && sub.album_path.starts_with(&format!("{}/", other.album_path))
        });
        if covered_by_parent {
            continue;
        }

        // One album's failure is reported against that album and the batch goes
        // on. Aborting here meant a single stale subscription — an album
        // renamed on Wednesday, say — stopped every other album from syncing,
        // and the photographer had no way to tell which one was at fault or
        // that the rest had never gone up at all. `apply` already treats a
        // single failing file this way; a failing album is the same shape.
        let outcome = match sync_path_for(
            lib,
            remote_id,
            transport,
            &sub.album_path,
            sub.direction,
            published_root,
            publish_opts,
            allow_deletes,
            sub.scope,
        ) {
            Ok(outcome) => outcome,
            Err(e) => sync::SyncOutcome {
                failed: vec![(sub.album_path.clone(), e.to_string())],
                ..Default::default()
            },
        };
        out.push((sub.album_path.clone(), outcome));
    }
    Ok(out)
}

// --------------------------------------------------------- cross-library push

/// What a stateless push to a *foreign* remote produced.
///
/// A foreign remote belongs to another library, so this machine holds no
/// baselines for it and never will: there is no third manifest to make a
/// deletion decidable, which is why deletes are not even an option here.
/// Overwrites of files the remote held differently are performed — that is
/// what pushing means — but each one is named so the caller's UI can warn.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ForeignPushOutcome {
    /// The path that was pushed.
    pub album_path: String,
    /// Which library did the pushing — the sender's stable id and its root
    /// folder's name, recorded so the receiving side can label foreign
    /// content ("pushed from library X") without guessing.
    pub library_id: String,
    pub library_name: String,
    /// Files uploaded: new to the remote, or replacing a differing copy.
    pub files_pushed: usize,
    /// Files the remote already held byte-identically.
    pub skipped_unchanged: usize,
    /// Files that existed on the remote with different bytes and were
    /// replaced. With no baseline there is no way to know whose is newer —
    /// the caller warns, the photographer decides whether to have done it.
    pub overwritten: Vec<String>,
    /// Files that never reached the remote, named with the reason.
    pub failed: Vec<(String, String)>,
    /// Every album the push covered.
    pub albums: Vec<String>,
}

/// Push a path to an arbitrary remote this library has no relationship with —
/// typically another library's remote, read via
/// [`crate::remotes::read_library_remotes`].
///
/// Stateless: no subscription is created, no baseline recorded, and nothing
/// is ever deleted from the remote (`allow_deletes` is not accepted here at
/// all). Ancestor folders' `index.md` are created only where absent, exactly
/// as an ordinary push treats folders it does not own. Both scopes work:
/// `Full` also uploads the originals and metadata namespace.
pub fn push_album_to(
    lib: &Library,
    transport: &dyn RemoteTransport,
    path: &str,
    published_root: &Path,
    publish_opts: &PublishOptions,
    scope: SyncScopeKind,
) -> Result<ForeignPushOutcome> {
    let mut out = ForeignPushOutcome {
        album_path: path.to_string(),
        library_id: lib.library_id()?,
        library_name: lib
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        ..Default::default()
    };

    // The subtree, published fresh so what goes up is what a gallery reads.
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

    let remote = transport.manifest()?;

    // Ancestors: reachable, never reconfigured — and never recorded, this is
    // not our remote.
    for ancestor in ancestors_of(path) {
        if lib.album_by_path(&ancestor)?.is_none() {
            continue;
        }
        publish::publish_album(lib, &ancestor, published_root, publish_opts)?;
        let key = format!("{ancestor}/index.md");
        if remote.contains_key(&key) {
            out.albums.push(ancestor);
            continue;
        }
        let file = published_root.join(&key);
        match std::fs::read(&file) {
            Ok(bytes) => match transport.put(&key, &bytes) {
                Ok(()) => {
                    out.files_pushed += 1;
                    out.albums.push(ancestor);
                }
                Err(e) => out.failed.push((key, e.to_string())),
            },
            Err(e) => out.failed.push((key, e.to_string())),
        }
    }

    // The subtree's own files: upload what is new or different, name every
    // overwrite, delete nothing.
    let local = publish::manifest_of(published_root)?;
    let scope_filter = SyncScope::with(vec![path.to_string()]);
    for (key, hash) in scope_filter.filter(&local) {
        match remote.get(&key) {
            Some(theirs) if *theirs == hash => {
                out.skipped_unchanged += 1;
                continue;
            }
            other => {
                let file = published_root.join(&key);
                let bytes = match std::fs::read(&file) {
                    Ok(b) => b,
                    Err(e) => {
                        out.failed.push((key.clone(), e.to_string()));
                        continue;
                    }
                };
                match transport.put(&key, &bytes) {
                    Ok(()) => {
                        out.files_pushed += 1;
                        if other.is_some() {
                            out.overwritten.push(key.clone());
                        }
                    }
                    Err(e) => out.failed.push((key.clone(), e.to_string())),
                }
            }
        }
    }

    if scope == SyncScopeKind::Full {
        crate::full::foreign_push_full(lib, transport, path, &subtree, &remote, &mut out)?;
    }

    out.albums.extend(subtree);
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
