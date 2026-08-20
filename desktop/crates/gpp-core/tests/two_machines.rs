//! Two machines, one server, partial libraries.
//!
//! This is the scenario the whole sync design exists for, exercised
//! end-to-end: machine A publishes an album, machine B — which has never seen
//! it — pulls it, contributes its own, and A adopts that in turn. At no point
//! may either machine's work be destroyed.

use std::path::Path;

use gpp_core::albums::NewAlbum;
use gpp_core::import::{import_dir, ImportOptions};
use gpp_core::publish::PublishOptions;
use gpp_core::remote;
use gpp_core::sync::{FsTransport, RemoteTransport, SyncDirection};
use gpp_core::Library;

/// A machine: its own library plus its own published tree.
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

/// Create an album on `m` with `n` photos, imported and ready to publish.
fn author_album(m: &Machine, album: &str, files: &[&str]) {
    let dir = m.lib.resolve(album).unwrap();
    for (i, name) in files.iter().enumerate() {
        write_jpeg(&dir.join(name), 120 + i as u32 * 10, 90);
    }
    import_dir(&m.lib, &dir, &ImportOptions::default(), None).unwrap();

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

#[test]
fn two_machines_exchange_albums_without_data_loss() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    let b = machine();

    // --- A shoots and publishes a wedding --------------------------------
    author_album(&a, "2026/ana-ivan", &["a1.jpg", "a2.jpg"]);
    let pushed = remote::push_album(
        &a.lib, &server, "2026/ana-ivan", a.published_root(), &opts, false,
    )
    .unwrap();
    assert!(pushed.files_pushed >= 3, "index.md + 2 photos");

    // --- B is a brand-new machine: it can see the album ------------------
    let discovered = remote::remote_albums(&b.lib, &server).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].path, "2026/ana-ivan");
    assert!(!discovered[0].local, "B does not have it yet");
    assert!(discovered[0].tracked.is_none(), "and does not sync it yet");
    assert_eq!(discovered[0].title.as_deref(), Some("Album 2026/ana-ivan"));

    // --- B pulls it and it becomes a first-class local album -------------
    let pulled = remote::pull_album(&b.lib, &server, "2026/ana-ivan", b.published_root()).unwrap();
    assert!(pulled.files_pulled >= 3);
    assert!(pulled.conflicts.is_empty());

    let adopted = b.lib.album_by_path("2026/ana-ivan").unwrap().unwrap();
    assert_eq!(adopted.title, "Album 2026/ana-ivan");
    assert_eq!(b.lib.album_photos("2026/ana-ivan").unwrap().len(), 2);
    assert_eq!(b.lib.photo_count().unwrap(), 2, "photos entered B's catalog");

    // The internal token must match, or the gallery's access cookie breaks
    // when the two machines publish the same album.
    let original = a.lib.album_by_path("2026/ana-ivan").unwrap().unwrap();
    assert_eq!(adopted.token, original.token, "album id is adopted, not regenerated");

    // --- B contributes an album of its own -------------------------------
    author_album(&b, "2026/marko", &["b1.jpg"]);
    remote::push_album(&b.lib, &server, "2026/marko", b.published_root(), &opts, false).unwrap();

    // --- A never had B's album: it must be untouched, not deleted --------
    let plan = remote::plan_album_sync(
        &a.lib, &server, "2026/marko", SyncDirection::Push, a.published_root(),
    )
    .unwrap();
    assert!(!plan.is_destructive(), "A must not delete an album it never had");

    // A syncing its OWN album must also leave B's alone.
    remote::push_album(&a.lib, &server, "2026/ana-ivan", a.published_root(), &opts, true).unwrap();
    let still_there = remote::remote_album_manifest(&server, "2026/marko").unwrap();
    assert!(!still_there.is_empty(), "B's album survived A's push");

    // --- A adopts B's album ---------------------------------------------
    remote::pull_album(&a.lib, &server, "2026/marko", a.published_root()).unwrap();
    assert_eq!(a.lib.album_photos("2026/marko").unwrap().len(), 1);
    assert_eq!(a.lib.albums().unwrap().len(), 2);

    // Both machines now agree on both albums.
    let server_albums = gpp_core::sync::albums_in_manifest(&server.manifest().unwrap());
    assert_eq!(server_albums, vec!["2026/ana-ivan", "2026/marko"]);
}

#[test]
fn pull_only_machine_never_writes_to_the_server() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/x", &["a.jpg"]);
    remote::push_album(&a.lib, &server, "2026/x", a.published_root(), &opts, false).unwrap();
    let before = server.manifest().unwrap();

    // B adopts the album, then authors a local-only file inside it.
    let b = machine();
    remote::pull_album(&b.lib, &server, "2026/x", b.published_root()).unwrap();
    b.lib.track_album("2026/x", SyncDirection::Pull).unwrap();
    write_jpeg(&b.published_root().join("2026/x/local-only.jpg"), 50, 50);

    // A pull-direction sync must not upload it.
    let outcome = remote::sync_album(
        &b.lib, &server, "2026/x", SyncDirection::Pull, b.published_root(), &opts, false,
    )
    .unwrap();
    assert_eq!(outcome.pushed, 0);
    assert_eq!(
        server.manifest().unwrap().len(),
        before.len(),
        "pull-only must leave the server byte-identical"
    );
}

#[test]
fn album_scope_isolates_sync_completely() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/keep", &["k.jpg"]);
    author_album(&a, "2026/other", &["o.jpg"]);
    remote::push_album(&a.lib, &server, "2026/keep", a.published_root(), &opts, false).unwrap();
    remote::push_album(&a.lib, &server, "2026/other", a.published_root(), &opts, false).unwrap();

    // Delete one album's files locally, then push ONLY that album with
    // deletions allowed. The other album must not be affected.
    std::fs::remove_dir_all(a.published_root().join("2026/keep")).unwrap();
    remote::push_album(&a.lib, &server, "2026/keep", a.published_root(), &opts, true).unwrap();

    let other = remote::remote_album_manifest(&server, "2026/other").unwrap();
    assert!(!other.is_empty(), "an unrelated album must survive a scoped push");
}
