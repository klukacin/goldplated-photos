//! Multiple remotes, sync scopes, cross-library push — schema v5 end to end.
//!
//! Follows the `two_machines.rs` pattern: each machine is its own library and
//! published tree, each server a folder transport, and every scenario is one
//! the design document names. The invariants that must not bend anywhere in
//! here: originals are never overwritten or deleted without an explicit,
//! confirmed request; conflicts are reported and never guessed; untracked
//! albums are untouched on every side.

use std::path::Path;

use gpp_core::albums::NewAlbum;
use gpp_core::develop::EditOp;
use gpp_core::full::{self, FULL_PREFIX};
use gpp_core::import::{import_dir, ImportOptions};
use gpp_core::publish::PublishOptions;
use gpp_core::remote;
use gpp_core::sync::{FsTransport, RemoteTransport, SyncDirection, SyncScopeKind};
use gpp_core::Library;

struct Machine {
    lib: Library,
    published: tempfile::TempDir,
    _root: tempfile::TempDir,
}

fn machine() -> Machine {
    let root = tempfile::tempdir().unwrap();
    let lib = Library::open(root.path()).unwrap();
    Machine {
        lib,
        published: tempfile::tempdir().unwrap(),
        _root: root,
    }
}

impl Machine {
    fn published_root(&self) -> &Path {
        self.published.path()
    }
}

fn write_jpeg(path: &Path, w: u32, h: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::DynamicImage::new_rgb8(w, h)
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn author_album(m: &Machine, album: &str, files: &[&str]) {
    let dir = m.lib.resolve(album).unwrap();
    for (i, name) in files.iter().enumerate() {
        if name.ends_with(".nef") {
            // A camera negative: catalogued, never decoded, never published
            // to the web tree — exactly what full scope exists to carry.
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(name), format!("raw sensor bytes {i}")).unwrap();
        } else if name.ends_with(".heic") {
            // A genuine iPhone frame. It publishes as `.jpg`, so its published
            // name is never its library name — the case that defeats matching
            // the server's `photoOrder` against filenames.
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::copy(fixture("hevc.heic"), dir.join(name)).unwrap();
        } else {
            write_jpeg(&dir.join(name), 120 + i as u32 * 10, 90);
        }
    }
    import_dir(&m.lib, &dir, &ImportOptions::default(), None, None).unwrap();

    m.lib
        .create_album(&NewAlbum {
            path: album.to_string(),
            title: Some(format!("Album {album}")),
            ..Default::default()
        })
        .unwrap();

    let prefix = format!("{album}/");
    let ids: Vec<i64> = m
        .lib
        .photos(&Default::default())
        .unwrap()
        .into_iter()
        .filter(|p| p.rel_path.starts_with(&prefix))
        .map(|p| p.id)
        .collect();
    m.lib.add_photos_to_album(album, &ids).unwrap();
}

// ------------------------------------------------------------- two remotes

/// Two remotes, independent everything: album A tracked push on remote 1
/// only, album B on remote 2 only, and the same album on both with baselines
/// of its own on each — a conflict against one remote is not a conflict
/// against the other.
#[test]
fn two_remotes_keep_independent_subscriptions_and_baselines() {
    let server1_dir = tempfile::tempdir().unwrap();
    let server2_dir = tempfile::tempdir().unwrap();
    let server1 = FsTransport::new(server1_dir.path());
    let server2 = FsTransport::new(server2_dir.path());
    let opts = PublishOptions::default();

    let m = machine();
    let r1 = m.lib.add_remote("One", server1_dir.path().to_str().unwrap(), None).unwrap();
    let r2 = m.lib.add_remote("Two", server2_dir.path().to_str().unwrap(), None).unwrap();

    author_album(&m, "2026/a", &["a1.jpg"]);
    author_album(&m, "2026/b", &["b1.jpg"]);
    author_album(&m, "2026/shared", &["s1.jpg"]);

    // A goes to remote 1 only, B to remote 2 only, shared to both.
    remote::push_path_for(
        &m.lib, r1, &server1, "2026/a", m.published_root(), &opts, false, SyncScopeKind::Web,
    )
    .unwrap();
    remote::push_path_for(
        &m.lib, r2, &server2, "2026/b", m.published_root(), &opts, false, SyncScopeKind::Web,
    )
    .unwrap();
    for (rid, server) in [(r1, &server1), (r2, &server2)] {
        remote::push_path_for(
            &m.lib, rid, server, "2026/shared", m.published_root(), &opts, false,
            SyncScopeKind::Web,
        )
        .unwrap();
    }

    // Subscriptions are per remote: remote 1 knows nothing of album B.
    let subs1: Vec<String> = m
        .lib
        .album_subscriptions_for(r1)
        .unwrap()
        .into_iter()
        .map(|s| s.album_path)
        .collect();
    let subs2: Vec<String> = m
        .lib
        .album_subscriptions_for(r2)
        .unwrap()
        .into_iter()
        .map(|s| s.album_path)
        .collect();
    assert_eq!(subs1, vec!["2026/a", "2026/shared"]);
    assert_eq!(subs2, vec!["2026/b", "2026/shared"]);

    // Album B never reached server 1 in any form, and vice versa.
    assert!(!server1.manifest().unwrap().keys().any(|k| k.contains("2026/b")));
    assert!(!server2.manifest().unwrap().keys().any(|k| k.contains("2026/a")));

    // The shared album diverges ON SERVER 2 ONLY, while the local copy also
    // moves: that is a conflict against remote 2 and a plain push against
    // remote 1 — the baselines are not shared.
    server2.put("2026/shared/s1.jpg", b"changed on server two").unwrap();
    write_jpeg(&m.published_root().join("2026/shared/s1.jpg"), 300, 200);

    let plan1 = remote::plan_album_sync_for(
        &m.lib, r1, &server1, "2026/shared", SyncDirection::Both, m.published_root(),
    )
    .unwrap();
    let plan2 = remote::plan_album_sync_for(
        &m.lib, r2, &server2, "2026/shared", SyncDirection::Both, m.published_root(),
    )
    .unwrap();
    assert!(!plan1.has_conflicts(), "remote 1 never diverged: {:?}", plan1.changes);
    assert!(plan2.has_conflicts(), "remote 2 diverged on both sides");
}

// -------------------------------------------------------------- full scope

/// The device-to-device story: A pushes `full` (RAW and edits included), a
/// fresh B pulls `full` and can continue culling and developing the same
/// frames — originals byte-identical, ratings and stacks carried. And where B
/// already holds a *different* original, B's file survives and is named.
#[test]
fn full_scope_round_trips_originals_ratings_and_edit_stacks() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    // --- A authors: a JPEG with an edit and a rating, plus a RAW -----------
    let a = machine();
    author_album(&a, "2026/x", &["one.jpg", "neg.nef"]);
    let one = a.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    let neg = a.lib.photo_by_rel_path("2026/x/neg.nef").unwrap().unwrap();
    a.lib.set_rating(one.id, 4).unwrap();
    a.lib.set_flag(neg.id, gpp_core::Flag::Pick).unwrap();
    a.lib.set_photo_tags(one.id, &["wedding".to_string()]).unwrap();
    let mut stack = a.lib.edits(one.id).unwrap();
    stack.set(EditOp::Exposure { ev: 0.4 });
    a.lib.set_edits(one.id, &stack).unwrap();

    let pushed = remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert!(pushed.failed.is_empty(), "{:?}", pushed.failed);

    // The namespace holds the originals byte-identically — the RAW the web
    // tree never carries, and the *undeveloped* JPEG (the web copy holds the
    // developed pixels; the negative rides only here).
    let raw_bytes = std::fs::read(a.lib.resolve("2026/x/neg.nef").unwrap()).unwrap();
    assert_eq!(
        server.get(&format!("{FULL_PREFIX}/2026/x/neg.nef")).unwrap(),
        raw_bytes
    );
    let original_jpeg = std::fs::read(a.lib.resolve("2026/x/one.jpg").unwrap()).unwrap();
    assert_eq!(
        server.get(&format!("{FULL_PREFIX}/2026/x/one.jpg")).unwrap(),
        original_jpeg
    );
    let doc = full::doc_from_bytes(
        &server
            .get(&format!("{FULL_PREFIX}/2026/x/album.gpp.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(doc.membership, vec!["neg.nef", "one.jpg"]);

    // --- a fresh B pulls full ---------------------------------------------
    let b = machine();
    let pulled = remote::pull_path_for(
        &b.lib,
        b.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        b.published_root(),
        SyncScopeKind::Full,
    )
    .unwrap();
    assert!(pulled.metadata_conflicts.is_empty(), "{:?}", pulled.metadata_conflicts);

    // Originals arrived byte-identical, RAW included…
    assert_eq!(
        std::fs::read(b.lib.resolve("2026/x/neg.nef").unwrap()).unwrap(),
        raw_bytes
    );
    assert_eq!(
        std::fs::read(b.lib.resolve("2026/x/one.jpg").unwrap()).unwrap(),
        original_jpeg
    );
    // …the RAW is catalogued and a member of the album…
    let b_photos = b.lib.album_photos("2026/x").unwrap();
    assert!(b_photos.iter().any(|p| p.filename == "neg.nef"), "{b_photos:?}");
    // …and ratings, flags, tags and the develop stack carried.
    let b_one = b.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    let b_neg = b.lib.photo_by_rel_path("2026/x/neg.nef").unwrap().unwrap();
    assert_eq!(b_one.rating, 4);
    assert_eq!(b_neg.flag, gpp_core::Flag::Pick);
    assert_eq!(b.lib.photo_tags(b_one.id).unwrap(), vec!["wedding"]);
    assert_eq!(
        b.lib.edits(b_one.id).unwrap(),
        a.lib.edits(one.id).unwrap(),
        "B can continue developing the same frame"
    );

    // The subscription remembers the scope, so plain sync keeps moving full.
    let sub = b.lib.album_subscription("2026/x").unwrap().unwrap();
    assert_eq!(sub.scope, SyncScopeKind::Full);
}

/// A full pull into a library that already holds a *different* original for
/// one frame: the local negative is kept, byte for byte, and named — the same
/// rule the web pull has always enforced. Local metadata that diverged is
/// reported, not overwritten.
#[test]
fn a_full_pull_keeps_differing_local_originals_and_reports_metadata_divergence() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg"]);
    let a_one = a.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    a.lib.set_rating(a_one.id, 5).unwrap();
    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();

    // B holds its own, different one.jpg — its negative — already rated 2.
    // Written at another size so the two machines genuinely disagree.
    let b = machine();
    write_jpeg(&b.lib.resolve("2026/x").unwrap().join("one.jpg"), 300, 200);
    import_dir(
        &b.lib,
        &b.lib.resolve("2026/x").unwrap(),
        &ImportOptions::default(),
        None,
        None,
    )
    .unwrap();
    b.lib
        .create_album(&NewAlbum { path: "2026/x".into(), ..Default::default() })
        .unwrap();
    let b_ids: Vec<i64> = b
        .lib
        .photos(&Default::default())
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    b.lib.add_photos_to_album("2026/x", &b_ids).unwrap();
    let b_bytes = std::fs::read(b.lib.resolve("2026/x/one.jpg").unwrap()).unwrap();
    assert_ne!(
        b_bytes,
        std::fs::read(a.lib.resolve("2026/x/one.jpg").unwrap()).unwrap(),
        "the two machines must disagree for this test to prove anything"
    );
    let b_one = b.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    b.lib.set_rating(b_one.id, 2).unwrap();

    let pulled = remote::pull_path_for(
        &b.lib,
        b.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        b.published_root(),
        SyncScopeKind::Full,
    )
    .unwrap();

    // B's negative survived, byte for byte, and the disagreement is named.
    assert_eq!(
        std::fs::read(b.lib.resolve("2026/x/one.jpg").unwrap()).unwrap(),
        b_bytes,
        "a pull wrote over the photographer's original"
    );
    assert!(
        pulled
            .kept_originals
            .contains(&format!("{FULL_PREFIX}/2026/x/one.jpg")),
        "the kept original must be named: {:?}",
        pulled.kept_originals
    );
    // The rating both sides set differently: reported, unchanged.
    assert_eq!(
        b.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap().rating,
        2,
        "a metadata divergence must never be resolved by guessing"
    );
    assert!(
        pulled.metadata_conflicts.iter().any(|c| c.contains("rating")),
        "the divergence must be reported: {:?}",
        pulled.metadata_conflicts
    );
}

/// A web-scope client — every existing remote consumer — must never see the
/// namespace: not as an album, not in a web pull, not in a web push's
/// deletions.
#[test]
fn web_scope_clients_never_see_the_full_namespace() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg", "neg.nef"]);
    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert!(
        server
            .manifest()
            .unwrap()
            .keys()
            .any(|k| k.starts_with(FULL_PREFIX)),
        "the namespace has to exist for this test to prove anything"
    );

    // A web machine listing albums sees exactly the real one.
    let b = machine();
    let rid = b.lib.ensure_default_remote().unwrap();
    let listed = remote::remote_albums_for(&b.lib, rid, &server).unwrap();
    let paths: Vec<&str> = listed.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(paths, vec!["2026", "2026/x"], "no __gpp_full__ album anywhere");

    // A web pull of the album brings the published files only.
    let pulled = remote::pull_path_for(
        &b.lib, rid, &server, "2026/x", b.published_root(), SyncScopeKind::Web,
    )
    .unwrap();
    assert!(!b.lib.resolve("2026/x/neg.nef").unwrap().exists(), "RAW is full-scope only");
    assert!(pulled.rejected.is_empty());

    // And a web push with deletions allowed cannot touch the namespace: it is
    // outside every web plan's scope.
    let before = server.manifest().unwrap();
    remote::push_path_for(
        &b.lib, rid, &server, "2026/x", b.published_root(), &opts, true, SyncScopeKind::Web,
    )
    .unwrap();
    let after = server.manifest().unwrap();
    assert_eq!(
        before.keys().filter(|k| k.starts_with(FULL_PREFIX)).count(),
        after.keys().filter(|k| k.starts_with(FULL_PREFIX)).count(),
        "a web push deleted from the full namespace"
    );
}

/// A folder tracked `web` must never put the gallery's pixels where a
/// `full`-tracked album's negatives go.
///
/// The two are separate subscriptions, and `sync_tracked_albums_for` runs them
/// separately — `ORDER BY album_path`, so the folder, being the shorter path,
/// always runs first. Its web pull adopted the published bytes as the library's
/// copy of every photo underneath; the album's own full pull then found the
/// file present and, quite correctly, refused to overwrite it. The result was a
/// library whose "originals" were the gallery's developed, HEIC→JPEG-converted,
/// rating-filtered pixels, whose RAW never arrived at all, and whose only sign
/// of any of it was a `kept_originals` line that reads like a safety message.
#[test]
fn a_web_tracked_folder_never_substitutes_gallery_pixels_for_full_scope_originals() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    // A develops the frame, so the published pixels genuinely differ from the
    // negative — without that this test proves nothing.
    let a = machine();
    author_album(&a, "2026/x", &["one.jpg", "neg.nef"]);
    let one = a.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    let mut stack = a.lib.edits(one.id).unwrap();
    stack.set(EditOp::Exposure { ev: 0.8 });
    a.lib.set_edits(one.id, &stack).unwrap();
    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();

    let negative = std::fs::read(a.lib.resolve("2026/x/one.jpg").unwrap()).unwrap();
    let published = server.get("2026/x/one.jpg").unwrap();
    assert_ne!(negative, published, "the develop has to change the delivered file");
    let raw = std::fs::read(a.lib.resolve("2026/x/neg.nef").unwrap()).unwrap();

    // B wants the year's galleries, and this one wedding's negatives.
    let b = machine();
    let rid = b.lib.ensure_default_remote().unwrap();
    b.lib
        .track_album_for("2026", SyncDirection::Both, Some(SyncScopeKind::Web), rid)
        .unwrap();
    b.lib
        .track_album_for("2026/x", SyncDirection::Both, Some(SyncScopeKind::Full), rid)
        .unwrap();

    let results =
        remote::sync_tracked_albums_for(&b.lib, rid, &server, b.published_root(), &opts, false)
            .unwrap();
    assert_eq!(
        results.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        vec!["2026", "2026/x"],
        "the folder runs first — that ordering is the whole trap"
    );
    for (path, outcome) in &results {
        assert!(outcome.failed.is_empty(), "{path}: {:?}", outcome.failed);
    }

    assert_eq!(
        std::fs::read(b.lib.resolve("2026/x/one.jpg").unwrap()).unwrap(),
        negative,
        "B's negative is the gallery's developed copy, and the real one can never land"
    );
    assert_eq!(
        std::fs::read(b.lib.resolve("2026/x/neg.nef").unwrap()).unwrap(),
        raw,
        "the RAW the web scope never publishes"
    );

    // And nothing was withheld on the way: the same two passes on a fresh
    // machine, in the same order, through the form that carries the report —
    // and split across two remotes, because the negative under the library root
    // is one file no matter how many servers have an opinion about it.
    let c = machine();
    let studio = c.lib.add_remote("Studio", server_dir.path().to_str().unwrap(), None).unwrap();
    let backup = c.lib.add_remote("Backup", server_dir.path().to_str().unwrap(), None).unwrap();
    c.lib
        .track_album_for("2026", SyncDirection::Both, Some(SyncScopeKind::Web), studio)
        .unwrap();
    c.lib
        .track_album_for("2026/x", SyncDirection::Both, Some(SyncScopeKind::Full), backup)
        .unwrap();

    let web = remote::pull_path_for(
        &c.lib,
        studio,
        &server,
        "2026",
        c.published_root(),
        SyncScopeKind::Web,
    )
    .unwrap();
    assert!(web.kept_originals.is_empty(), "{:?}", web.kept_originals);
    assert!(
        !c.lib.resolve("2026/x/one.jpg").unwrap().exists(),
        "the web pass wrote the gallery's pixels into the library"
    );

    let full = remote::pull_path_for(
        &c.lib,
        backup,
        &server,
        "2026/x",
        c.published_root(),
        SyncScopeKind::Full,
    )
    .unwrap();
    assert!(
        full.kept_originals.is_empty(),
        "an original was 'kept' that was never the photographer's: {:?}",
        full.kept_originals
    );
    assert_eq!(
        std::fs::read(c.lib.resolve("2026/x/one.jpg").unwrap()).unwrap(),
        negative
    );
}

/// Two libraries holding the same full-scope album must reach a state where
/// neither has anything to say.
///
/// The metadata document's bytes are its manifest hash, and it carried the
/// writing library's random id — so after A pushed and B pulled, each side
/// regenerated a document the other could never match. `album.gpp.json` went up
/// again on every sync in both directions and `apply_metadata` re-ran over the
/// whole album each round: a full-scope album never reached a clean state, and
/// no test ever ran a second round to notice.
#[test]
fn a_full_scope_album_settles_after_one_round_trip() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    let arid = a.lib.ensure_default_remote().unwrap();
    author_album(&a, "2026/x", &["one.jpg", "neg.nef"]);
    let one = a.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    a.lib.set_rating(one.id, 4).unwrap();
    a.lib.set_photo_tags(one.id, &["wedding".to_string(), "bride".to_string()]).unwrap();
    let mut stack = a.lib.edits(one.id).unwrap();
    stack.set(EditOp::Exposure { ev: 0.4 });
    a.lib.set_edits(one.id, &stack).unwrap();

    remote::push_path_for(
        &a.lib, arid, &server, "2026/x", a.published_root(), &opts, false, SyncScopeKind::Full,
    )
    .unwrap();

    let b = machine();
    let brid = b.lib.ensure_default_remote().unwrap();
    remote::pull_path_for(&b.lib, brid, &server, "2026/x", b.published_root(), SyncScopeKind::Full)
        .unwrap();

    // The document B would write is the document A wrote, byte for byte. That
    // is the whole property: identical content, identical bytes, on any machine.
    let key = format!("{FULL_PREFIX}/2026/x/{}", full::METADATA_FILENAME);
    assert_eq!(
        full::album_metadata_bytes(&b.lib, "2026/x").unwrap(),
        server.get(&key).unwrap(),
        "B regenerates a document A can never match, so it re-uploads for ever"
    );

    // Round two, on each side, in the direction a subscription syncs.
    let before = server.manifest().unwrap();
    let b_again = remote::sync_path_for(
        &b.lib, brid, &server, "2026/x", SyncDirection::Both, b.published_root(), &opts, false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert_eq!((b_again.pushed, b_again.pulled), (0, 0), "B: {b_again:?}");
    assert!(b_again.conflicts.is_empty(), "B: {:?}", b_again.conflicts);

    let a_again = remote::sync_path_for(
        &a.lib, arid, &server, "2026/x", SyncDirection::Both, a.published_root(), &opts, false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert_eq!((a_again.pushed, a_again.pulled), (0, 0), "A: {a_again:?}");
    assert!(a_again.conflicts.is_empty(), "A: {:?}", a_again.conflicts);

    assert_eq!(server.manifest().unwrap(), before, "the server moved on a settled album");
}

/// One develop op this build cannot read must cost one frame's metadata, not
/// the pull — and never the other library's document.
///
/// `EditStack::from_json` is strict by design (an unknown op cannot be stored
/// verbatim: the render key hashes the stack, so a build that skipped an op
/// would still cache pixels under a key claiming it had applied it). That
/// strictness rode out of `apply_metadata` as a hard error, taking the whole
/// pull down *after* the originals had landed — and after the document's
/// baseline had been written at fetch time, which made the retry read our
/// document as the newer one and push it over theirs. Their metadata was then
/// never applied and never mentioned again.
#[test]
fn an_unknown_develop_op_costs_one_frame_and_never_the_pull() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg", "two.jpg", "neg.nef"]);
    let two = a.lib.photo_by_rel_path("2026/x/two.jpg").unwrap().unwrap();
    a.lib.set_rating(two.id, 3).unwrap();
    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();

    // A newer build wrote one frame's stack with an op this one has never
    // heard of.
    let key = format!("{FULL_PREFIX}/2026/x/{}", full::METADATA_FILENAME);
    let theirs = {
        let mut doc: serde_json::Value =
            serde_json::from_slice(&server.get(&key).unwrap()).unwrap();
        for photo in doc["photos"].as_array_mut().unwrap() {
            if photo["filename"] == "one.jpg" {
                photo["edits"] = serde_json::json!({
                    "version": 1,
                    "ops": [{ "op": "tone-curve", "points": [0, 90, 255] }]
                });
            }
        }
        serde_json::to_vec_pretty(&doc).unwrap()
    };
    server.put(&key, &theirs).unwrap();

    let b = machine();
    let brid = b.lib.ensure_default_remote().unwrap();
    let pulled = remote::pull_path_for(
        &b.lib, brid, &server, "2026/x", b.published_root(), SyncScopeKind::Full,
    )
    .expect("one unreadable op must not fail the pull");

    // Everything else arrived: both JPEGs, the RAW, and the other frame's
    // metadata.
    for name in ["one.jpg", "two.jpg", "neg.nef"] {
        assert!(
            b.lib.resolve(&format!("2026/x/{name}")).unwrap().exists(),
            "{name} never landed"
        );
    }
    assert_eq!(
        b.lib.photo_by_rel_path("2026/x/two.jpg").unwrap().unwrap().rating,
        3,
        "the rest of the document was not applied"
    );
    // …and the frame that could not be read is named, not swallowed.
    assert!(
        pulled
            .failed
            .iter()
            .any(|(what, why)| what == "2026/x/one.jpg" && why.contains("develop stack")),
        "the unreadable stack must be named: {:?}",
        pulled.failed
    );
    assert!(b.lib.edits(
        b.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap().id
    ).unwrap().is_empty(), "half an unknown stack must never be stored");

    // A second sync must not decide our document is the newer one.
    let again = remote::sync_path_for(
        &b.lib, brid, &server, "2026/x", SyncDirection::Both, b.published_root(), &opts, false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert_eq!(
        server.get(&key).unwrap(),
        theirs,
        "our document was pushed over theirs, and their metadata is gone for good"
    );
    assert!(
        again.conflicts.contains(&key),
        "a document neither side can settle is a conflict to report: {:?}",
        again.conflicts
    );
}

/// A full-scope pull of a new album must arrive in the order the album was in.
///
/// The web half applies the server's `photoOrder`, but it runs before the
/// full-scope originals do — on a fresh library it reordered an empty album and
/// `sort: custom` was silently dropped on the receiving machine. Two of the
/// frames here are ones `photoOrder` could never place anyway: a HEIC, which is
/// published under a different name, and a RAW, which is not published at all.
#[test]
fn a_full_scope_pull_keeps_the_albums_own_photo_order() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    #[cfg(feature = "heif")]
    author_album(&a, "2026/x", &["a.jpg", "b.jpg", "neg.nef", "shot.heic"]);
    #[cfg(not(feature = "heif"))]
    author_album(&a, "2026/x", &["a.jpg", "b.jpg", "neg.nef"]);

    // A custom order, deliberately not the alphabetical one the grid falls
    // back to, so an order that was never applied cannot look like success.
    let mut ids: Vec<i64> = a.lib.album_photos("2026/x").unwrap().iter().map(|p| p.id).collect();
    ids.reverse();
    a.lib.reorder_album("2026/x", &ids).unwrap();
    a.lib
        .update_album(
            "2026/x",
            &gpp_core::albums::AlbumUpdate { sort: Some("custom".into()), ..Default::default() },
        )
        .unwrap();
    let expected: Vec<String> = a
        .lib
        .album_photos("2026/x")
        .unwrap()
        .into_iter()
        .map(|p| p.filename)
        .collect();
    let alphabetical = {
        let mut sorted = expected.clone();
        sorted.sort();
        sorted
    };
    assert_ne!(expected, alphabetical, "the order has to be a choice, not the default");

    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();

    let b = machine();
    remote::pull_path_for(
        &b.lib,
        b.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        b.published_root(),
        SyncScopeKind::Full,
    )
    .unwrap();

    let arrived: Vec<String> = b
        .lib
        .album_photos("2026/x")
        .unwrap()
        .into_iter()
        .map(|p| p.filename)
        .collect();
    assert_eq!(arrived, expected, "the album arrived in an order nobody chose");
    assert_eq!(
        b.lib.album_by_path("2026/x").unwrap().unwrap().sort,
        "custom",
        "an order the gallery is told to ignore is no order at all"
    );
}

/// Pushing one album at full scope must not delete its sub-albums' originals.
///
/// `push_full` plans over `__gpp_full__/<path>` — the whole subtree — so handing
/// it one album's keys as the local side made every original a sub-album had
/// pushed earlier look locally absent: a delete with `allow_deletes`, and a
/// withheld-delete report naming files nobody had deleted without it.
#[test]
fn pushing_one_album_at_full_scope_leaves_its_sub_albums_originals_alone() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    let rid = a.lib.ensure_default_remote().unwrap();
    author_album(&a, "2026/weddings/ana", &["a1.jpg", "a1.nef"]);
    remote::push_path_for(
        &a.lib, rid, &server, "2026/weddings", a.published_root(), &opts, false,
        SyncScopeKind::Full,
    )
    .unwrap();
    let raw = format!("{FULL_PREFIX}/2026/weddings/ana/a1.nef");
    assert!(server.get(&raw).is_ok(), "the sub-album's negative has to be up there first");

    // Pushing the folder alone, deletions withheld: nothing to withhold.
    let careful = remote::push_album_for(
        &a.lib, rid, &server, "2026/weddings", a.published_root(), &opts, false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert!(
        careful.withheld_deletes.is_empty(),
        "files nobody deleted were reported as withheld deletions: {:?}",
        careful.withheld_deletes
    );

    // And with deletions allowed, the sub-album's originals survive.
    let allowed = remote::push_album_for(
        &a.lib, rid, &server, "2026/weddings", a.published_root(), &opts, true,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert_eq!(allowed.deleted_remote, 0, "a push of the folder deleted from inside it");
    assert!(
        server.get(&raw).is_ok(),
        "the sub-album's only copy of a negative was deleted from the server"
    );
    assert!(server
        .get(&format!("{FULL_PREFIX}/2026/weddings/ana/{}", full::METADATA_FILENAME))
        .is_ok());
}

/// A photograph the namespace cannot carry has to be named on the way in too.
///
/// Two members of one album sharing a filename publish the same full-scope key,
/// and only the first can have it. The push side has always reported the loser;
/// the pull side computed exactly the same finding into a throwaway and dropped
/// it, so the photographer was never told which frame a full sync leaves behind.
#[test]
fn a_full_pull_names_the_photographs_the_namespace_cannot_carry() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg"]);
    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &opts,
        false,
        SyncScopeKind::Full,
    )
    .unwrap();

    // B has the album, and a second frame from another shoot that happens to
    // carry the same filename, put into the same album.
    let b = machine();
    let brid = b.lib.ensure_default_remote().unwrap();
    author_album(&b, "2026/x", &["one.jpg"]);
    write_jpeg(&b.lib.resolve("spare").unwrap().join("one.jpg"), 320, 240);
    import_dir(
        &b.lib,
        &b.lib.resolve("spare").unwrap(),
        &ImportOptions::default(),
        None,
        None,
    )
    .unwrap();
    let spare = b.lib.photo_by_rel_path("spare/one.jpg").unwrap().unwrap();
    b.lib.add_photos_to_album("2026/x", &[spare.id]).unwrap();

    let pulled = remote::pull_path_for(
        &b.lib, brid, &server, "2026/x", b.published_root(), SyncScopeKind::Full,
    )
    .unwrap();
    assert_eq!(
        pulled.failed.len(),
        1,
        "the frame the namespace leaves behind was computed and thrown away: {:?}",
        pulled.failed
    );
    assert!(
        pulled.failed[0].1.contains(&format!("{FULL_PREFIX}/2026/x/one.jpg")),
        "{:?}",
        pulled.failed
    );

    // And it rides out through the form the sync panel calls.
    let synced = remote::sync_path_for(
        &b.lib, brid, &server, "2026/x", SyncDirection::Pull, b.published_root(), &opts, false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert_eq!(synced.failed.len(), 1, "{:?}", synced.failed);
}

/// Tags listed in another order are the same tags.
///
/// `photo_tags` answers `ORDER BY name`, and so does every document this build
/// writes — but a document from an older writer may list them in any order, and
/// comparing the two as ordered vectors turned that into "changed on both
/// sides": a conflict reported about a photograph nobody had touched.
#[test]
fn a_documents_tag_order_is_not_a_disagreement() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg"]);
    let one = a.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap();
    a.lib
        .set_photo_tags(one.id, &["wedding".to_string(), "bride".to_string()])
        .unwrap();
    remote::push_path_for(
        &a.lib,
        a.lib.ensure_default_remote().unwrap(),
        &server,
        "2026/x",
        a.published_root(),
        &PublishOptions::default(),
        false,
        SyncScopeKind::Full,
    )
    .unwrap();

    let b = machine();
    let brid = b.lib.ensure_default_remote().unwrap();
    remote::pull_path_for(&b.lib, brid, &server, "2026/x", b.published_root(), SyncScopeKind::Full)
        .unwrap();
    assert_eq!(
        b.lib
            .photo_tags(b.lib.photo_by_rel_path("2026/x/one.jpg").unwrap().unwrap().id)
            .unwrap(),
        vec!["bride", "wedding"]
    );

    // The same document, rewritten by a writer that did not sort its tags.
    let key = format!("{FULL_PREFIX}/2026/x/{}", full::METADATA_FILENAME);
    let mut doc: serde_json::Value = serde_json::from_slice(&server.get(&key).unwrap()).unwrap();
    for photo in doc["photos"].as_array_mut().unwrap() {
        photo["tags"] = serde_json::json!(["wedding", "bride"]);
    }
    server.put(&key, &serde_json::to_vec_pretty(&doc).unwrap()).unwrap();

    let again = remote::pull_path_for(
        &b.lib, brid, &server, "2026/x", b.published_root(), SyncScopeKind::Full,
    )
    .unwrap();
    assert!(
        again.metadata_conflicts.is_empty(),
        "the same two tags in another order were reported as a divergence: {:?}",
        again.metadata_conflicts
    );
}

// ------------------------------------------------------ cross-library push

/// A stateless push against a foreign remote: uploads what is new or
/// different, refuses to delete anything (there is no allow_deletes to even
/// pass), names every overwrite, and carries the sender's identity.
#[test]
fn a_foreign_push_never_deletes_and_names_its_overwrites() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    // The receiving library's own content, which the sender must not disturb.
    server.put("2026/theirs/index.md", b"---\ntitle: \"Theirs\"\n---\n").unwrap();
    server.put("2026/theirs/t1.jpg", b"their photo").unwrap();
    // And a file inside the very album the sender will push, differing.
    server.put("2026/x/one.jpg", b"their version of one.jpg").unwrap();

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg", "two.jpg"]);

    let outcome = remote::push_album_to(
        &a.lib, &server, "2026/x", a.published_root(), &opts, SyncScopeKind::Web,
    )
    .unwrap();

    // Provenance travels with the outcome.
    assert_eq!(outcome.library_id, a.lib.library_id().unwrap());
    assert!(!outcome.library_name.is_empty());

    // The differing file was replaced — and named, so a UI can warn.
    assert_eq!(outcome.overwritten, vec!["2026/x/one.jpg".to_string()]);
    assert!(outcome.files_pushed >= 3, "index.md + two photos");

    // Nothing was deleted anywhere: the receiver's album is intact.
    assert_eq!(server.get("2026/theirs/t1.jpg").unwrap(), b"their photo");

    // Stateless: no subscription, no baselines were recorded for this remote.
    assert!(a.lib.album_subscriptions().unwrap().is_empty());
    assert!(a.lib.synced_manifest().unwrap().is_empty());

    // Now the sender drops a photo locally and pushes again: a stateful push
    // would want a deletion — a foreign push simply cannot express one.
    let two = a.lib.photo_by_rel_path("2026/x/two.jpg").unwrap().unwrap();
    a.lib.remove_photos_from_album("2026/x", &[two.id]).unwrap();
    remote::push_album_to(&a.lib, &server, "2026/x", a.published_root(), &opts, SyncScopeKind::Web)
        .unwrap();
    assert!(
        server.get("2026/x/two.jpg").is_ok(),
        "a foreign push deleted from someone else's remote"
    );
}

/// Full scope works through the foreign door too: the namespace rides along.
#[test]
fn a_foreign_push_can_carry_full_scope() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/x", &["one.jpg", "neg.nef"]);
    let outcome = remote::push_album_to(
        &a.lib, &server, "2026/x", a.published_root(), &opts, SyncScopeKind::Full,
    )
    .unwrap();
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert!(server.get(&format!("{FULL_PREFIX}/2026/x/neg.nef")).is_ok());
    assert!(server.get(&format!("{FULL_PREFIX}/2026/x/album.gpp.json")).is_ok());
}

// ------------------------------------------------- reading foreign catalogs

#[test]
fn read_library_remotes_reads_v5_and_legacy_catalogs_without_migrating_them() {
    // --- a v5 library ------------------------------------------------------
    let v5 = machine();
    v5.lib.add_remote("Studio", "https://studio.example/api/sync", Some("tok-tok-tok-tok!"))
        .unwrap();
    v5.lib.add_remote("Drive", "/mnt/backup", None).unwrap();
    let listed = gpp_core::remotes::read_library_remotes(v5._root.path()).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "Studio");
    assert_eq!(listed[0].token.as_deref(), Some("tok-tok-tok-tok!"));
    assert!(listed[0].is_default);

    // --- a legacy (pre-v5) library: settings keys only ---------------------
    let legacy_root = tempfile::tempdir().unwrap();
    let gpp = legacy_root.path().join(".gpp");
    std::fs::create_dir_all(&gpp).unwrap();
    {
        let conn = rusqlite::Connection::open(gpp.join("catalog.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL);
             INSERT INTO schema_version(version) VALUES(4);
             CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO settings(key, value) VALUES('remote.dir', '/srv/old-remote');
             INSERT INTO settings(key, value) VALUES('remote.token', 'legacy-token-16ch!');",
        )
        .unwrap();
    }
    let listed = gpp_core::remotes::read_library_remotes(legacy_root.path()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Main");
    assert_eq!(listed[0].target, "/srv/old-remote");
    assert_eq!(listed[0].token.as_deref(), Some("legacy-token-16ch!"));

    // Reading did NOT migrate the foreign catalog: still v4, no remotes table.
    {
        let conn = rusqlite::Connection::open(gpp.join("catalog.db")).unwrap();
        let v: i64 = conn.query_row("SELECT version FROM schema_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 4, "a foreign catalog belongs to whatever build manages it");
        let has_remotes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='remotes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_remotes, 0);
    }

    // --- a catalog from a newer build is refused by name --------------------
    {
        let conn = rusqlite::Connection::open(gpp.join("catalog.db")).unwrap();
        conn.execute("UPDATE schema_version SET version = 99", []).unwrap();
    }
    let err = gpp_core::remotes::read_library_remotes(legacy_root.path()).unwrap_err();
    assert!(err.to_string().contains("v99"), "{err}");

    // --- not a library at all ----------------------------------------------
    let empty = tempfile::tempdir().unwrap();
    assert!(gpp_core::remotes::read_library_remotes(empty.path()).is_err());
}
