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
    // Two albums plus the `2026` folder they both live in.
    assert_eq!(a.lib.albums().unwrap().len(), 3);

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

/// Deep paths must survive the trip. An album at `2026/weddings/ana-ivan`
/// is only reachable in the gallery if `2026` and `2026/weddings` exist as
/// collections too — on the server, and on every machine that adopts it.
#[test]
fn a_nested_path_arrives_whole_on_the_other_machine() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/weddings/ana-ivan", &["a1.jpg", "a2.jpg"]);

    // Authoring a deep album creates the folders above it.
    let folders: Vec<String> = a
        .lib
        .albums()
        .unwrap()
        .into_iter()
        .filter(|x| x.is_collection)
        .map(|x| x.path)
        .collect();
    assert_eq!(folders, vec!["2026".to_string(), "2026/weddings".to_string()]);

    remote::push_path(
        &a.lib, &server, "2026/weddings/ana-ivan", a.published_root(), &opts, false,
    )
    .unwrap();

    // The server has the whole chain, not just the leaf.
    for key in ["2026/index.md", "2026/weddings/index.md", "2026/weddings/ana-ivan/index.md"] {
        assert!(server.get(key).is_ok(), "server is missing {key}");
    }

    // --- B adopts the leaf and gets the folders with it -------------------
    let b = machine();
    let pulled = remote::pull_path(
        &b.lib, &server, "2026/weddings/ana-ivan", b.published_root(),
    )
    .unwrap();

    assert_eq!(
        pulled.albums,
        vec![
            "2026".to_string(),
            "2026/weddings".to_string(),
            "2026/weddings/ana-ivan".to_string()
        ],
        "parents first, then the album"
    );
    assert!(b.lib.album_by_path("2026").unwrap().unwrap().is_collection);
    assert!(b.lib.album_by_path("2026/weddings").unwrap().unwrap().is_collection);
    assert_eq!(b.lib.album_photos("2026/weddings/ana-ivan").unwrap().len(), 2);

    // Photos landed under the full path, not flattened into the library root.
    assert!(b.lib.resolve("2026/weddings/ana-ivan/a1.jpg").unwrap().exists());
}

/// Pulling a folder brings every album under it, each into its own folder.
#[test]
fn pulling_a_folder_brings_its_albums_and_keeps_them_apart() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    let a = machine();
    author_album(&a, "2026/weddings/ana-ivan", &["a1.jpg"]);
    author_album(&a, "2026/weddings/mia-luka", &["m1.jpg"]);
    author_album(&a, "2026/events/konferencija", &["k1.jpg"]);
    remote::push_path(&a.lib, &server, "2026", a.published_root(), &opts, false).unwrap();

    // B wants the weddings only.
    let b = machine();
    let pulled =
        remote::pull_path(&b.lib, &server, "2026/weddings", b.published_root()).unwrap();

    assert_eq!(
        pulled.albums,
        vec![
            "2026".to_string(),
            "2026/weddings".to_string(),
            "2026/weddings/ana-ivan".to_string(),
            "2026/weddings/mia-luka".to_string(),
        ]
    );
    // Each album's photo is in its own folder — nothing flattened.
    assert!(b.lib.resolve("2026/weddings/ana-ivan/a1.jpg").unwrap().exists());
    assert!(b.lib.resolve("2026/weddings/mia-luka/m1.jpg").unwrap().exists());
    assert_eq!(b.lib.album_photos("2026/weddings/ana-ivan").unwrap().len(), 1);
    assert_eq!(b.lib.album_photos("2026/weddings/mia-luka").unwrap().len(), 1);

    // A collection holds sub-albums, not photos. A prefix match here would make
    // `2026/weddings` claim every photo beneath it, and publishing would then
    // copy them all into the collection's own folder.
    assert_eq!(b.lib.album_photos("2026/weddings").unwrap().len(), 0);
    assert_eq!(b.lib.album_photos("2026").unwrap().len(), 0);

    // The events branch was outside the requested path: not adopted at all.
    assert!(b.lib.album_by_path("2026/events").unwrap().is_none());
    assert!(b.lib.album_by_path("2026/events/konferencija").unwrap().is_none());

    // And B pushing its branch back must not disturb the events branch.
    remote::push_path(
        &b.lib, &server, "2026/weddings", b.published_root(), &opts, true,
    )
    .unwrap();
    assert!(server.get("2026/events/konferencija/k1.jpg").is_ok(), "other branch survived");
}

/// A machine pushing a deep album needs its parent folders to exist. That is
/// the whole claim — not a claim about how those folders are configured. Two
/// machines each auto-create their own `2026`, with different tokens; the one
/// that pushes second must not reset the first one's folder.
#[test]
fn pushing_an_album_never_rewrites_a_shared_parent_folder() {
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());
    let opts = PublishOptions::default();

    // A configures the shared folder deliberately and pushes it.
    let a = machine();
    author_album(&a, "2026/weddings/ana-ivan", &["a1.jpg"]);
    a.lib
        .update_album(
            "2026/weddings",
            &gpp_core::albums::AlbumUpdate {
                title: Some("Vjenčanja 2026".into()),
                password: Some(Some("tajna".into())),
                ..Default::default()
            },
        )
        .unwrap();
    remote::push_path(&a.lib, &server, "2026/weddings", a.published_root(), &opts, false).unwrap();

    let before = String::from_utf8(server.get("2026/weddings/index.md").unwrap()).unwrap();
    assert!(before.contains("Vjenčanja 2026"));
    assert!(before.contains("tajna"));

    // B has never pulled. It authored its own album in the same tree, so it has
    // its own `2026/weddings` row with its own token, title and no password.
    let b = machine();
    author_album(&b, "2026/weddings/mia-luka", &["m1.jpg"]);
    let pushed = remote::push_path(
        &b.lib, &server, "2026/weddings/mia-luka", b.published_root(), &opts, false,
    )
    .unwrap();

    // B's album is there…
    assert!(server.get("2026/weddings/mia-luka/m1.jpg").is_ok());
    // …and A's folder settings are untouched, byte for byte.
    let after = String::from_utf8(server.get("2026/weddings/index.md").unwrap()).unwrap();
    assert_eq!(before, after, "a leaf push must not reconfigure the folder");
    // Both shared folders are reported: B generated its own token for each of
    // them locally, so neither matches what the server already holds.
    assert_eq!(
        pushed.folders_left_alone,
        vec!["2026".to_string(), "2026/weddings".to_string()]
    );
    // The album itself is not a "folder left alone" — it was pushed.
    assert!(pushed.albums.contains(&"2026/weddings/mia-luka".to_string()));

    // Pushing the folder itself puts it inside the plan's scope, where the two
    // divergent versions are a genuine conflict. The engine refuses to pick a
    // winner rather than overwriting A's settings.
    let direct = remote::push_path(
        &b.lib, &server, "2026/weddings", b.published_root(), &opts, false,
    )
    .unwrap();
    assert_eq!(direct.conflicts, vec!["2026/weddings/index.md".to_string()]);
    assert_eq!(
        before,
        String::from_utf8(server.get("2026/weddings/index.md").unwrap()).unwrap(),
        "a conflict resolves to leaving the server alone"
    );

    // The way to take over a folder is to adopt it first, then edit and push —
    // so the change is made on top of what is actually online.
    remote::pull_path(&b.lib, &server, "2026/weddings", b.published_root()).unwrap();
    assert_eq!(
        b.lib.album_by_path("2026/weddings").unwrap().unwrap().title,
        "Vjenčanja 2026",
        "B adopted A's folder settings"
    );
    b.lib
        .update_album(
            "2026/weddings",
            &gpp_core::albums::AlbumUpdate {
                title: Some("Vjenčanja".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let edited = remote::push_path(
        &b.lib, &server, "2026/weddings", b.published_root(), &opts, false,
    )
    .unwrap();
    assert!(edited.conflicts.is_empty(), "no conflict once B is up to date");
    let now = String::from_utf8(server.get("2026/weddings/index.md").unwrap()).unwrap();
    assert!(now.contains("title: \"Vjenčanja\""));
    assert!(now.contains("tajna"), "A's password survived B's rename");
}
