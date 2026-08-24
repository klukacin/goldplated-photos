//! Full-scope sync: originals and metadata, beside the published tree.
//!
//! A `web`-scope sync moves the *published* tree — developed pixels, JPEG for
//! HEIF, RAW excluded. That is what galleries show, and its remote layout is
//! byte-for-byte what it has always been. `full` scope moves, additionally,
//! what makes the main remote usable as **library sync between devices**:
//!
//! ```text
//! __gpp_full__/<album-path>/<filename>        every catalogued original,
//!                                             byte-identical, RAW included
//! __gpp_full__/<album-path>/album.gpp.json    album fields, membership order,
//!                                             per-photo rating/flag/label/tags,
//!                                             the develop stack, captured_at
//! ```
//!
//! The namespace lives in the *same* transport as the web tree — a folder
//! remote holds it as a folder, the HTTP server stores it under a private root
//! the gallery never serves — and it is planned with the same three manifests,
//! per-remote baselines, `allow_deletes` semantics and conflict reporting as
//! the web scope.
//!
//! The prefix is reserved and **invisible to every web-scope consumer**:
//! [`crate::sync::albums_in_manifest`] filters it out, so `remote_albums()`
//! never lists it and a web pull never walks into it. Its segments are
//! ordinary names (no dots), so the same `accepts_remote_path` rules apply on
//! the way in — a hostile manifest cannot use the prefix to climb anywhere.
//!
//! The invariants do not bend here:
//!
//! - **A pull never overwrites a local original.** An original already on disk
//!   is kept and named in `kept_originals`, exactly as the web pull keeps it.
//! - **Metadata is applied with import-style caution.** A photo new to this
//!   library takes everything; an existing row only has still-default fields
//!   filled (rating 0, no flag, no label, no tags) and edits applied only
//!   where no local stack exists. Anything both sides changed is reported in
//!   `metadata_conflicts`, never resolved by guessing.
//! - **Deletes need `allow_deletes`**, and withheld ones are named.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::model::Flag;
use crate::remote::{PullOutcome, PushOutcome};
use crate::sync::{
    self, Action, Manifest, RemoteTransport, SyncDirection, SyncScope,
};

/// The reserved namespace. Nothing under a gallery's real album tree may ever
/// carry it — the server-side endpoints route it to a private root, and the
/// desktop's web-scope consumers filter it out of every manifest.
pub const FULL_PREFIX: &str = "__gpp_full__";

/// Whether a manifest key sits inside the full-scope namespace.
pub fn is_full_key(key: &str) -> bool {
    key == FULL_PREFIX || key.starts_with(&format!("{FULL_PREFIX}/"))
}

/// The key an album's file takes inside the namespace.
pub fn full_key(album_path: &str, filename: &str) -> String {
    format!("{FULL_PREFIX}/{album_path}/{filename}")
}

/// Filename of the per-album metadata document.
pub const METADATA_FILENAME: &str = "album.gpp.json";

// ------------------------------------------------------------- the document

/// The per-album metadata document, versioned. The bytes are what get hashed
/// into the manifest, so serialization has to be deterministic: struct field
/// order, album order for photos, sorted tags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FullAlbumDoc {
    /// Format version of this document, not of anything it describes.
    pub version: u32,
    /// The library that wrote it — provenance for a future master catalog.
    #[serde(default)]
    pub library_id: String,
    /// Album fields as the catalog holds them, for reconstruction. The web
    /// tree's `index.md` stays authoritative for the gallery; this is the
    /// catalog's own richer view.
    #[serde(default)]
    pub album: FullAlbumFields,
    /// Membership, in album order, by library filename.
    #[serde(default)]
    pub membership: Vec<String>,
    /// One entry per member photo, in album order.
    #[serde(default)]
    pub photos: Vec<FullPhotoDoc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FullAlbumFields {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub is_collection: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FullPhotoDoc {
    pub filename: String,
    /// BLAKE3 of the original's bytes — the identity the receiving side
    /// verifies its pulled copy against.
    pub content_hash: String,
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub flag: String,
    #[serde(default)]
    pub color_label: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The develop stack, verbatim as `edits.stack_json` holds it (a JSON
    /// value, not a re-encoded string) — readable by us on any machine,
    /// opaque to everyone else. `null` for an untouched frame.
    #[serde(default)]
    pub edits: Option<serde_json::Value>,
    #[serde(default)]
    pub captured_at: Option<String>,
}

const DOC_VERSION: u32 = 1;

/// Build the metadata document for one album, as bytes ready to upload.
pub fn album_metadata_bytes(lib: &Library, album_path: &str) -> Result<Vec<u8>> {
    let album = lib
        .album_by_path(album_path)?
        .ok_or_else(|| Error::AlbumNotFound(album_path.to_string()))?;
    let photos = lib.album_photos(album_path)?;

    let mut doc = FullAlbumDoc {
        version: DOC_VERSION,
        library_id: lib.library_id()?,
        album: FullAlbumFields {
            title: album.title.clone(),
            description: album.description.clone(),
            date: album.date.clone(),
            sort: album.sort.clone(),
            style: album.style.clone(),
            is_collection: album.is_collection,
            tags: album.tags.clone(),
        },
        membership: photos.iter().map(|p| p.filename.clone()).collect(),
        photos: Vec::with_capacity(photos.len()),
    };
    for p in &photos {
        let stack = lib.edits(p.id)?;
        let edits = if stack.is_empty() {
            None
        } else {
            Some(serde_json::from_str(&stack.to_json()?)?)
        };
        doc.photos.push(FullPhotoDoc {
            filename: p.filename.clone(),
            content_hash: p.content_hash.clone(),
            rating: p.rating,
            flag: p.flag.as_str().to_string(),
            color_label: p.color_label.clone(),
            tags: lib.photo_tags(p.id)?,
            edits,
            captured_at: p.captured_at.clone(),
        });
    }
    Ok(serde_json::to_vec_pretty(&doc)?)
}

// --------------------------------------------------------------- local side

/// What backs one full-namespace key on this machine.
enum Source {
    /// A catalogued original — absolute path of the file to read.
    Original(std::path::PathBuf),
    /// The metadata document, generated in memory.
    Metadata(Vec<u8>),
}

/// The local manifest of the full namespace for a set of albums, plus where
/// each key's bytes come from. Membership order decides duplicate filenames:
/// the first photo keeps the name, later ones are reported by the caller.
fn local_full_side(
    lib: &Library,
    albums: &[String],
    duplicates: &mut Vec<(String, String)>,
) -> Result<(Manifest, BTreeMap<String, Source>)> {
    let mut manifest = Manifest::new();
    let mut sources: BTreeMap<String, Source> = BTreeMap::new();

    for album in albums {
        for photo in lib.album_photos(album)? {
            let key = full_key(album, &photo.filename);
            if photo.filename == METADATA_FILENAME {
                duplicates.push((
                    photo.rel_path.clone(),
                    format!("'{METADATA_FILENAME}' is reserved for the metadata document"),
                ));
                continue;
            }
            if manifest.contains_key(&key) {
                duplicates.push((
                    photo.rel_path.clone(),
                    format!("another photo already publishes {key} in full scope"),
                ));
                continue;
            }
            manifest.insert(key.clone(), photo.content_hash.clone());
            sources.insert(key, Source::Original(lib.resolve(&photo.rel_path)?));
        }
        let bytes = album_metadata_bytes(lib, album)?;
        let key = full_key(album, METADATA_FILENAME);
        manifest.insert(key.clone(), blake3::hash(&bytes).to_hex().to_string());
        sources.insert(key, Source::Metadata(bytes));
    }
    Ok((manifest, sources))
}

// --------------------------------------------------------------------- push

/// Push the full namespace of `path`'s subtree, folding the result into an
/// existing [`PushOutcome`]. Same rules as the web push: per-remote baselines,
/// conflicts reported, deletes withheld without `allow_deletes`, one file's
/// failure never stopping the rest.
pub(crate) fn push_full(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    path: &str,
    subtree: &[String],
    allow_deletes: bool,
    outcome: &mut PushOutcome,
) -> Result<()> {
    let mut duplicates = Vec::new();
    let (local, sources) = local_full_side(lib, subtree, &mut duplicates)?;
    outcome.failed.extend(duplicates);

    let synced = lib.synced_manifest_for(remote_id)?;
    let remote = transport.manifest()?;
    let scope = SyncScope::with(vec![format!("{FULL_PREFIX}/{path}")]);
    let plan = sync::plan_scoped(&scope, SyncDirection::Push, &local, &synced, &remote);

    for change in &plan.changes {
        let key = &change.path;
        match &change.action {
            Action::Push => {
                let bytes = match sources.get(key) {
                    Some(Source::Original(p)) => match std::fs::read(p) {
                        Ok(b) => b,
                        Err(e) => {
                            outcome.failed.push((key.clone(), e.to_string()));
                            continue;
                        }
                    },
                    Some(Source::Metadata(b)) => b.clone(),
                    // A plan can only say Push for a key the local manifest
                    // holds, and every local key has a source.
                    None => continue,
                };
                match transport.put(key, &bytes) {
                    Ok(()) => {
                        let hash = blake3::hash(&bytes).to_hex().to_string();
                        lib.record_synced_for(remote_id, key, &hash)?;
                        outcome.files_pushed += 1;
                    }
                    Err(e) => outcome.failed.push((key.clone(), e.to_string())),
                }
            }
            Action::DeleteRemote => {
                if !allow_deletes {
                    outcome.withheld_deletes.push(key.clone());
                    continue;
                }
                match transport.delete(key) {
                    Ok(()) => {
                        lib.forget_synced_for(remote_id, key)?;
                        outcome.deleted_remote += 1;
                    }
                    Err(e) => outcome.failed.push((key.clone(), e.to_string())),
                }
            }
            Action::Conflict => outcome.conflicts.push(key.clone()),
            Action::Skip | Action::LeaveAlone => outcome.skipped += 1,
            Action::Pull => outcome.skipped += 1, // cannot occur under Push
            Action::ForgetState => {
                // Settle the books the same way `sync::apply` does.
                match local.get(key) {
                    Some(hash) => lib.record_synced_for(remote_id, key, hash)?,
                    None => lib.forget_synced_for(remote_id, key)?,
                }
            }
        }
    }
    Ok(())
}

/// The full-namespace half of a stateless foreign push: upload what is new or
/// different, name every overwrite, delete nothing, record no baselines.
pub(crate) fn foreign_push_full(
    lib: &Library,
    transport: &dyn RemoteTransport,
    path: &str,
    subtree: &[String],
    remote: &Manifest,
    out: &mut crate::remote::ForeignPushOutcome,
) -> Result<()> {
    let mut duplicates = Vec::new();
    let (local, sources) = local_full_side(lib, subtree, &mut duplicates)?;
    out.failed.extend(duplicates);

    let scope = SyncScope::with(vec![format!("{FULL_PREFIX}/{path}")]);
    for (key, hash) in scope.filter(&local) {
        match remote.get(&key) {
            Some(theirs) if *theirs == hash => {
                out.skipped_unchanged += 1;
            }
            other => {
                let bytes = match sources.get(&key) {
                    Some(Source::Original(p)) => match std::fs::read(p) {
                        Ok(b) => b,
                        Err(e) => {
                            out.failed.push((key.clone(), e.to_string()));
                            continue;
                        }
                    },
                    Some(Source::Metadata(b)) => b.clone(),
                    None => continue,
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
    Ok(())
}

// --------------------------------------------------------------------- pull

/// Pull the full namespace of `path`'s subtree, folding the result into an
/// existing [`PullOutcome`]. Originals land in the library at their album
/// path; **an original already on disk is never overwritten** — it is named
/// in `kept_originals`. Metadata is applied afterwards, with import-style
/// caution; divergences land in `metadata_conflicts`.
pub(crate) fn pull_full(
    lib: &Library,
    remote_id: i64,
    transport: &dyn RemoteTransport,
    path: &str,
    outcome: &mut PullOutcome,
) -> Result<()> {
    let remote = transport.manifest()?;
    let scope = SyncScope::with(vec![format!("{FULL_PREFIX}/{path}")]);
    let remote_scoped = scope.filter(&remote);
    if remote_scoped.is_empty() {
        // Nothing was ever pushed full-scope for this path — a web-only remote
        // is not an error, the web pull already did its work.
        return Ok(());
    }

    // The albums the namespace mentions under this path.
    let mut albums: Vec<String> = Vec::new();
    // Metadata documents to apply once every file has landed.
    let mut metadata: Vec<(String, Vec<u8>)> = Vec::new();

    let mut dup_guard = Vec::new();
    let known_albums: Vec<String> = remote_scoped
        .keys()
        .filter_map(|k| split_full_key(k).map(|(album, _)| album.to_string()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let (local, _) = local_full_side(
        lib,
        &known_albums
            .iter()
            .filter(|a| lib.album_by_path(a).ok().flatten().is_some())
            .cloned()
            .collect::<Vec<_>>(),
        &mut dup_guard,
    )?;

    let synced = lib.synced_manifest_for(remote_id)?;
    let plan = sync::plan_scoped(&scope, SyncDirection::Pull, &local, &synced, &remote);

    for change in &plan.changes {
        let key = &change.path;
        // A remote manifest is a document the server writes: every path in it
        // is checked before either write below — including that the prefix
        // cannot be used to escape (no `..`, no dot-segments, no backslash).
        if !sync::accepts_remote_path(key) {
            outcome.rejected.push(key.clone());
            continue;
        }
        let Some((album, filename)) = split_full_key(key) else {
            outcome.rejected.push(key.clone());
            continue;
        };

        // Two deliberate reinterpretations, mirroring the web pull:
        //
        // The metadata document is *derived* from a catalog, the way an
        // `index.md` is — holding it in a conflict would strand it there
        // forever, and adopting it is safe because `apply_metadata` is itself
        // cautious per field, never overwriting a local non-default value.
        //
        // A divergent original is the kept-negative case, not a file
        // conflict: the local file under the library root is the negative,
        // the resolution is always "keep ours", and the photographer is told
        // through `kept_originals` exactly as the web pull tells them.
        let action = match &change.action {
            Action::Conflict if filename == METADATA_FILENAME => &Action::Pull,
            Action::Conflict => {
                outcome.kept_originals.push(key.clone());
                continue;
            }
            other => other,
        };

        match action {
            Action::Pull => {
                if filename == METADATA_FILENAME {
                    match transport.get(key) {
                        Ok(bytes) => {
                            let hash = blake3::hash(&bytes).to_hex().to_string();
                            lib.record_synced_for(remote_id, key, &hash)?;
                            metadata.push((album.to_string(), bytes));
                        }
                        Err(e) => outcome
                            .rejected
                            .push(format!("{key} (fetch failed: {e})")),
                    }
                    if !albums.contains(&album.to_string()) {
                        albums.push(album.to_string());
                    }
                    continue;
                }

                let album_dir = lib.resolve(album)?;
                let dest = album_dir.join(filename);
                if dest.exists() {
                    // The photographer's original. Never overwritten; the
                    // server's disagreement is a fact worth a look, not an
                    // instruction.
                    outcome.kept_originals.push(key.clone());
                    continue;
                }
                match transport.get(key) {
                    Ok(bytes) => {
                        std::fs::create_dir_all(&album_dir)
                            .map_err(|e| Error::io(&album_dir, e))?;
                        std::fs::write(&dest, &bytes).map_err(|e| Error::io(&dest, e))?;
                        let hash = blake3::hash(&bytes).to_hex().to_string();
                        lib.record_synced_for(remote_id, key, &hash)?;
                        outcome.files_pulled += 1;
                        if !albums.contains(&album.to_string()) {
                            albums.push(album.to_string());
                        }
                    }
                    Err(e) => outcome
                        .rejected
                        .push(format!("{key} (fetch failed: {e})")),
                }
            }
            Action::Conflict => outcome.conflicts.push(key.clone()),
            Action::ForgetState => {
                match local.get(key) {
                    Some(hash) => lib.record_synced_for(remote_id, key, hash)?,
                    None => lib.forget_synced_for(remote_id, key)?,
                }
                outcome.skipped_unchanged += 1;
            }
            _ => outcome.skipped_unchanged += 1,
        }
    }

    // Catalog what arrived, then apply the metadata on top.
    for album in &albums {
        if lib.album_by_path(album)?.is_none() {
            // The web pull creates album rows; a full namespace naming an
            // album the web tree does not hold is left as files on disk.
            continue;
        }
        let album_dir = lib.resolve(album)?;
        if album_dir.is_dir() {
            crate::import::import_dir(
                lib,
                &album_dir,
                &crate::import::ImportOptions { recursive: false, ..Default::default() },
                None,
                None,
            )?;
        }
        // Newly catalogued originals (RAW above all — the web tree never
        // carried them) become members of their album.
        let members: std::collections::BTreeSet<String> = lib
            .album_photos(album)?
            .into_iter()
            .map(|p| p.filename)
            .collect();
        let missing: Vec<i64> = lib
            .photos(&crate::model::PhotoFilter::default())?
            .into_iter()
            .filter(|p| is_direct_child_of(album, &p.rel_path))
            .filter(|p| !members.contains(&p.filename))
            .map(|p| p.id)
            .collect();
        if !missing.is_empty() {
            lib.add_photos_to_album(album, &missing)?;
        }
    }

    for (album, bytes) in metadata {
        match serde_json::from_slice::<FullAlbumDoc>(&bytes) {
            Ok(doc) => apply_metadata(lib, &album, &doc, outcome)?,
            Err(e) => outcome
                .rejected
                .push(format!("{}/{album}/{METADATA_FILENAME} (unreadable: {e})", FULL_PREFIX)),
        }
    }
    Ok(())
}

/// `__gpp_full__/<album>/<file>` → `(<album>, <file>)`. `None` for the bare
/// prefix, a key with no filename, or one outside the namespace.
fn split_full_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix(FULL_PREFIX)?.strip_prefix('/')?;
    let (album, filename) = rest.rsplit_once('/')?;
    (!album.is_empty() && !filename.is_empty()).then_some((album, filename))
}

fn is_direct_child_of(album_path: &str, rel_path: &str) -> bool {
    rel_path
        .strip_prefix(album_path)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|name| !name.contains('/'))
}

/// Apply a pulled metadata document with import-style caution.
///
/// Full apply on photos new to this library (their fields are still at the
/// defaults an import leaves); on existing rows only still-default fields are
/// filled, and edits land only where no local stack exists. A field both
/// sides changed — local non-default, remote different — is named in
/// `metadata_conflicts` and left exactly as it is.
fn apply_metadata(
    lib: &Library,
    album: &str,
    doc: &FullAlbumDoc,
    outcome: &mut PullOutcome,
) -> Result<()> {
    for entry in &doc.photos {
        let rel = format!("{album}/{}", entry.filename);
        let Some(photo) = lib.photo_by_rel_path(&rel)? else {
            continue; // not pulled (rejected, kept elsewhere) — nothing to annotate
        };
        let mut conflict = |what: &str| {
            outcome
                .metadata_conflicts
                .push(format!("{rel}: {what} differs on both sides"));
        };

        if entry.rating > 0 {
            if photo.rating == 0 {
                lib.set_rating(photo.id, entry.rating)?;
            } else if photo.rating != entry.rating {
                conflict("rating");
            }
        }
        let flag = Flag::parse(&entry.flag);
        if flag != Flag::None {
            if photo.flag == Flag::None {
                lib.set_flag(photo.id, flag)?;
            } else if photo.flag != flag {
                conflict("flag");
            }
        }
        if let Some(label) = &entry.color_label {
            match &photo.color_label {
                None => lib.set_color_label(photo.id, Some(label))?,
                Some(local) if local != label => conflict("color label"),
                Some(_) => {}
            }
        }
        if !entry.tags.is_empty() {
            let local_tags = lib.photo_tags(photo.id)?;
            if local_tags.is_empty() {
                lib.set_photo_tags(photo.id, &entry.tags)?;
            } else if local_tags != entry.tags {
                conflict("tags");
            }
        }
        if let Some(edits) = &entry.edits {
            let local_stack = lib.edits(photo.id)?;
            let incoming = crate::develop::EditStack::from_json(&edits.to_string())?;
            if local_stack.is_empty() {
                if !incoming.is_empty() {
                    lib.set_edits(photo.id, &incoming)?;
                }
            } else if local_stack != incoming {
                conflict("develop stack");
            }
        }
    }
    Ok(())
}

/// One photo's presence check against the doc — used by tests.
pub fn doc_from_bytes(bytes: &[u8]) -> Result<FullAlbumDoc> {
    Ok(serde_json::from_slice(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_is_recognised_only_on_segment_boundaries() {
        assert!(is_full_key("__gpp_full__"));
        assert!(is_full_key("__gpp_full__/2026/ana/a.raw"));
        assert!(!is_full_key("__gpp_full__extra/x"));
        assert!(!is_full_key("2026/__gpp_full__/x"), "only a leading prefix is reserved");
    }

    #[test]
    fn full_keys_split_into_album_and_filename() {
        assert_eq!(
            split_full_key("__gpp_full__/2026/ana/a.raw"),
            Some(("2026/ana", "a.raw"))
        );
        assert_eq!(split_full_key("__gpp_full__/a"), None, "no album/file split");
        assert_eq!(split_full_key("__gpp_full__"), None);
        assert_eq!(split_full_key("2026/ana/a.raw"), None);
    }

    /// A hostile manifest cannot ride the reserved prefix out of the tree:
    /// the same `accepts_remote_path` rules apply inside the namespace.
    #[test]
    fn the_prefix_grants_no_escape() {
        for hostile in [
            "__gpp_full__/../x",
            "__gpp_full__/2026/../../etc/passwd",
            "__gpp_full__/2026/ana/.htaccess",
            "__gpp_full__/2026/ana/..\\..\\x",
            "__gpp_full__//x",
        ] {
            assert!(
                !sync::accepts_remote_path(hostile),
                "{hostile:?} must be refused"
            );
        }
        // …while ordinary keys inside it pass, because the segments are
        // ordinary names.
        assert!(sync::accepts_remote_path("__gpp_full__/2026/ana/a.raw"));
        assert!(sync::accepts_remote_path("__gpp_full__/2026/ana/album.gpp.json"));
    }

    #[test]
    fn the_web_album_walk_never_sees_the_namespace() {
        let manifest: Manifest = [
            ("2026/ana/index.md", "h"),
            ("__gpp_full__/2026/ana/index.md", "h"),
            ("__gpp_full__/2026/ana/album.gpp.json", "h"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(sync::albums_in_manifest(&manifest), vec!["2026/ana"]);
    }
}
