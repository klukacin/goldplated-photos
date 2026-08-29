//! Photographs that live outside the library — schema v6 end to end.
//!
//! A **source** is a root a catalogued photograph may live under. The library
//! root is one of them (the primary); any folder registered with `add_source`
//! is another, and files there are *referenced*: read, hashed, thumbnailed,
//! developed, published — never copied, never moved, never written to. That is
//! the whole of "import my Lightroom library without migrating a terabyte".
//!
//! What must not bend anywhere in here:
//!
//! - **A referenced file is never touched.** Not by import, not by publish, not
//!   by a sync. The only writes this library makes are to `.gpp/` on the
//!   primary and to the published tree.
//! - **Offline is not missing.** A drive that is not plugged in leaves the grid
//!   working (thumbnails are content-addressed and live on the primary), makes
//!   everything that must open an original fail with a *named* error, and above
//!   all does not let `prune` delete a single row.
//! - **Sources never overlap**, so a file has exactly one `(source, rel_path)`
//!   identity and every count in the app agrees with every other.

use std::path::Path;

use gpp_core::albums::NewAlbum;
use gpp_core::develop::EditOp;
use gpp_core::full::FULL_PREFIX;
use gpp_core::import::{import_dir, ImportOptions};
use gpp_core::model::PhotoFilter;
use gpp_core::publish::{publish_album, PublishOptions};
use gpp_core::sync::{FsTransport, RemoteTransport, SyncScopeKind};
use gpp_core::{Error, Library, Session, SourceKind};

fn write_jpeg(path: &Path, w: u32, h: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::DynamicImage::new_rgb8(w, h)
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

/// Count every file under a folder, so "nothing was copied in" can be asserted
/// rather than hoped for.
fn file_count(dir: &Path) -> usize {
    walk(dir).len()
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `.gpp` is the library's own derived data, not a photograph.
        if path.file_name().map(|n| n.to_string_lossy().starts_with('.')) == Some(true) {
            continue;
        }
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

// ----------------------------------------------------------- import in place

/// A folder inside a registered source is catalogued where it lies — no copy,
/// not one byte.
///
/// Before sources, this was the one thing the importer could not do: a folder
/// outside the library root was copied in, because a catalog row had no way to
/// name a file anywhere else. That copy is exactly what a photographer with a
/// full drive cannot afford.
#[test]
fn importing_a_folder_inside_a_registered_source_copies_nothing() {
    let lib_dir = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    write_jpeg(&drive.path().join("2019/ana/a1.jpg"), 120, 90);
    write_jpeg(&drive.path().join("2019/ana/a2.jpg"), 130, 90);
    write_jpeg(&drive.path().join("2020/mia/m1.jpg"), 140, 90);
    let before = file_count(drive.path());

    let lib = Library::open(lib_dir.path()).unwrap();
    let card = lib
        .add_source(drive.path(), Some("Archive"), Some(SourceKind::External))
        .unwrap();
    // Registering catalogues nothing on its own.
    assert_eq!(lib.photo_count().unwrap(), 0);

    // Import one folder *inside* the source, not the whole of it.
    let summary = import_dir(
        &lib,
        &drive.path().join("2019/ana"),
        &ImportOptions::default(),
        None,
        None,
    )
    .unwrap();

    assert_eq!(summary.imported, 2);
    assert_eq!(summary.copied_in, 0, "a source is a place photos are, not a destination");
    assert_eq!(summary.copied_into, None);
    assert_eq!(
        file_count(lib_dir.path()),
        0,
        "the library folder must hold no photographs at all"
    );
    assert_eq!(file_count(drive.path()), before, "the drive was modified");

    // The rows point at the drive, with paths relative to *it*.
    let photos = lib.photos(&PhotoFilter::default()).unwrap();
    assert_eq!(photos.len(), 2);
    for photo in &photos {
        assert_eq!(photo.source_id, card);
        assert!(photo.rel_path.starts_with("2019/ana/"), "{}", photo.rel_path);
        assert_eq!(
            lib.photo_path(photo).unwrap(),
            drive.path().canonicalize().unwrap().join(&photo.rel_path)
        );
        assert!(lib.photo_path(photo).unwrap().is_file());
    }

    // Thumbnails were still generated — they live on the primary, which is the
    // point of keeping `.gpp` there.
    let key = &photos[0].content_hash;
    assert!(gpp_core::media::thumb_path(&lib.thumb_dir(), key, "small").is_file());

    // Re-importing the whole source is incremental, as ever.
    let again = import_dir(&lib, drive.path(), &ImportOptions::default(), None, None).unwrap();
    assert_eq!(again.skipped, 2, "the two already catalogued");
    assert_eq!(again.imported, 1, "and the folder that had not been scanned");
    assert_eq!(again.copied_in, 0);
    assert_eq!(lib.photo_count().unwrap(), 3);
}

/// Two cards, two sources, one relative path. They are two photographs, and the
/// catalog holds both — which is what `UNIQUE(source_id, rel_path)` is for.
#[test]
fn two_sources_may_hold_the_same_relative_path() {
    let lib_dir = tempfile::tempdir().unwrap();
    let card_a = tempfile::tempdir().unwrap();
    let card_b = tempfile::tempdir().unwrap();
    write_jpeg(&card_a.path().join("DCIM/DSC_0001.jpg"), 100, 70);
    write_jpeg(&card_b.path().join("DCIM/DSC_0001.jpg"), 200, 140);

    let lib = Library::open(lib_dir.path()).unwrap();
    let a = lib.add_source(card_a.path(), Some("Card A"), None).unwrap();
    let b = lib.add_source(card_b.path(), Some("Card B"), None).unwrap();
    let opts = ImportOptions::default();
    import_dir(&lib, card_a.path(), &opts, None, None).unwrap();
    import_dir(&lib, card_b.path(), &opts, None, None).unwrap();

    assert_eq!(lib.photo_count().unwrap(), 2, "one path, two photographs");
    let from_a = lib.photo_by_source_rel_path(a, "DCIM/DSC_0001.jpg").unwrap().unwrap();
    let from_b = lib.photo_by_source_rel_path(b, "DCIM/DSC_0001.jpg").unwrap().unwrap();
    assert_ne!(from_a.id, from_b.id);
    assert_ne!(from_a.content_hash, from_b.content_hash);
    assert_eq!(from_a.width, Some(100));
    assert_eq!(from_b.width, Some(200));
    assert!(lib.photo_path(&from_a).unwrap().starts_with(card_a.path().canonicalize().unwrap()));
    assert!(lib.photo_path(&from_b).unwrap().starts_with(card_b.path().canonicalize().unwrap()));
}

// --------------------------------------------------------------- going dark

/// The drive is unplugged. The library keeps working, says so where it must,
/// and — the part that matters — deletes nothing.
///
/// Pruning an offline source would throw away every rating, flag, membership
/// and develop stack on those photographs because a cable was loose. There is
/// no undo for that: the catalog is the only place those facts live.
#[test]
fn an_offline_source_keeps_working_reports_itself_and_is_never_pruned() {
    let lib_dir = tempfile::tempdir().unwrap();
    let published = tempfile::tempdir().unwrap();
    let holder = tempfile::tempdir().unwrap();
    let drive = holder.path().join("archive-2019");
    write_jpeg(&drive.join("ana/a1.jpg"), 120, 90);
    write_jpeg(&drive.join("ana/a2.jpg"), 130, 90);

    let lib = Library::open(lib_dir.path()).unwrap();
    let card = lib.add_source(&drive, Some("Archive 2019"), None).unwrap();
    import_dir(&lib, &drive, &ImportOptions::default(), None, None).unwrap();

    // One photo is developed and one is rated, so there is something real to
    // lose if a prune goes wrong.
    let ids: Vec<i64> = lib
        .photos(&PhotoFilter::default())
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    lib.set_rating(ids[0], 5).unwrap();
    let session = Session::new();
    session.open_library(lib_dir.path()).unwrap();
    session
        .set_photo_edit(vec![ids[0]], EditOp::Exposure { ev: 0.5 })
        .unwrap();
    let developed_thumb = session.thumbnail_path(ids[0], "medium").unwrap();
    assert!(Path::new(&developed_thumb).exists());

    lib.create_album(&NewAlbum {
        path: "2019/ana".into(),
        title: Some("Ana".into()),
        ..Default::default()
    })
    .unwrap();
    lib.add_photos_to_album("2019/ana", &ids).unwrap();

    // --- the drive leaves the room ------------------------------------
    std::fs::rename(&drive, holder.path().join("archive-2019-elsewhere")).unwrap();

    // Listing still works, and says which source is not here.
    let listed = lib.sources().unwrap();
    let offline = listed.iter().find(|s| s.id == card).unwrap();
    assert!(!offline.online);
    assert_eq!(offline.photo_count, 2, "the catalog still knows them");
    assert!(listed.iter().find(|s| s.is_primary).unwrap().online);

    // The grid is unaffected: rows, ratings and thumbnails are all on the
    // primary and content-addressed.
    assert_eq!(lib.photo_count().unwrap(), 2);
    assert_eq!(lib.photo_by_id(ids[0]).unwrap().rating, 5);
    assert_eq!(lib.album_photos("2019/ana").unwrap().len(), 2);
    assert!(
        Path::new(&session.thumbnail_path(ids[0], "medium").unwrap()).exists(),
        "a cached render does not need the drive"
    );

    // Anything that must open the original fails, by name.
    let err = session.photo_path(ids[0]).unwrap_err();
    assert!(
        matches!(&err, Error::SourceOffline { name, .. } if name == "Archive 2019"),
        "the failure must name the source: {err}"
    );
    assert!(
        err.to_string().contains("Archive 2019"),
        "and say so to a human: {err}"
    );

    // Publish names them and ships the rest of the album — one unplugged drive
    // must not fail an album, and it must not be reported as "missing" either.
    let result = publish_album(&lib, "2019/ana", published.path(), &PublishOptions::default())
        .unwrap();
    assert_eq!(result.photos_copied, 0);
    assert_eq!(result.offline.len(), 2, "{:?}", result.offline);
    assert!(result.offline.iter().all(|o| o.contains("Archive 2019")));
    assert!(result.missing.is_empty(), "not missing — elsewhere: {:?}", result.missing);
    assert!(published.path().join("2019/ana/index.md").is_file(), "the album still published");

    // An edit still records, it simply cannot re-render yet.
    assert_eq!(
        session.set_photo_edit(vec![ids[1]], EditOp::Contrast { amount: 10.0 }).unwrap(),
        1
    );
    assert!(!session.photo_edits(ids[1]).unwrap().is_empty());

    // XMP export names them too, and writes nothing.
    let xmp = lib.export_xmp(Some("2019/ana")).unwrap();
    assert_eq!(xmp.written, 0);
    assert_eq!(xmp.offline.len(), 2);
    assert!(xmp.missing.is_empty());

    // --- and the one that must never happen ---------------------------
    assert_eq!(
        lib.prune_missing().unwrap(),
        0,
        "an unplugged drive is not a deleted photograph"
    );
    assert_eq!(lib.photo_count().unwrap(), 2);
    assert_eq!(lib.album_photos("2019/ana").unwrap().len(), 2);

    // An import of the whole library skips the source with a note rather than
    // failing the run.
    let summary = session.import(None, None).unwrap();
    assert!(!summary.cancelled);
    assert!(
        summary.notes.iter().any(|n| n.contains("Archive 2019")),
        "a skipped source has to be named: {:?}",
        summary.notes
    );

    // --- the drive comes back -----------------------------------------
    std::fs::rename(holder.path().join("archive-2019-elsewhere"), &drive).unwrap();

    assert!(lib.sources().unwrap().iter().find(|s| s.id == card).unwrap().online);
    assert!(Path::new(&session.photo_path(ids[0]).unwrap()).is_file());
    let after = publish_album(&lib, "2019/ana", published.path(), &PublishOptions::default())
        .unwrap();
    assert_eq!(after.photos_copied, 2, "{:?}", after.offline);
    assert!(after.offline.is_empty());
    assert_eq!(lib.export_xmp(Some("2019/ana")).unwrap().written, 2);
    assert_eq!(lib.prune_missing().unwrap(), 0);
}

/// A file really deleted from an *online* source is still pruned. Offline is
/// the exception; it is not an excuse to stop pruning.
#[test]
fn a_deleted_file_on_an_online_source_is_still_pruned() {
    let lib_dir = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    write_jpeg(&drive.path().join("a.jpg"), 60, 40);
    write_jpeg(&drive.path().join("b.jpg"), 60, 40);
    write_jpeg(&lib_dir.path().join("inside.jpg"), 60, 40);

    let lib = Library::open(lib_dir.path()).unwrap();
    lib.add_source(drive.path(), Some("Card"), None).unwrap();
    let opts = ImportOptions::default();
    import_dir(&lib, lib_dir.path(), &opts, None, None).unwrap();
    import_dir(&lib, drive.path(), &opts, None, None).unwrap();
    assert_eq!(lib.photo_count().unwrap(), 3);

    std::fs::remove_file(drive.path().join("b.jpg")).unwrap();
    assert_eq!(lib.prune_missing().unwrap(), 1);
    assert_eq!(lib.photo_count().unwrap(), 2);
    assert!(lib.photo_by_rel_path("inside.jpg").unwrap().is_some());
}

// ------------------------------------------------------- publish and sync

/// One album, photographs on two different roots. Publishing and a full-scope
/// push both have to reach every one of them.
///
/// An album is a view over photographs, not a folder of them — so nothing stops
/// half a wedding sitting on the laptop and half on the drive it was offloaded
/// to, and a gallery that shipped only one half would be a silent, invisible
/// failure.
#[test]
fn an_album_spanning_two_sources_publishes_and_pushes_whole() {
    let lib_dir = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    let published = tempfile::tempdir().unwrap();
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());

    // Half the shoot under the library, half on the drive.
    write_jpeg(&lib_dir.path().join("2026/ana/inside.jpg"), 120, 90);
    write_jpeg(&drive.path().join("ana/outside.jpg"), 130, 90);
    // A negative, which only full scope ever carries.
    std::fs::write(drive.path().join("ana/neg.nef"), b"raw sensor bytes").unwrap();
    let drive_before = file_count(drive.path());

    let lib = Library::open(lib_dir.path()).unwrap();
    lib.add_source(drive.path(), Some("Offload"), None).unwrap();
    let opts = ImportOptions::default();
    import_dir(&lib, lib_dir.path(), &opts, None, None).unwrap();
    import_dir(&lib, drive.path(), &opts, None, None).unwrap();

    lib.create_album(&NewAlbum {
        path: "2026/ana".into(),
        title: Some("Ana & Ivan".into()),
        ..Default::default()
    })
    .unwrap();
    let ids: Vec<i64> = lib
        .photos(&PhotoFilter::default())
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids.len(), 3);
    lib.add_photos_to_album("2026/ana", &ids).unwrap();

    // --- publish -------------------------------------------------------
    let result =
        publish_album(&lib, "2026/ana", published.path(), &PublishOptions::default()).unwrap();
    assert!(result.offline.is_empty());
    assert_eq!(result.photos_copied, 2, "both stills, wherever they live");
    assert!(published.path().join("2026/ana/inside.jpg").is_file());
    assert!(published.path().join("2026/ana/outside.jpg").is_file());
    assert!(
        !published.path().join("2026/ana/neg.nef").exists(),
        "a RAW is a negative, not something to hand a client"
    );
    assert_eq!(
        file_count(drive.path()),
        drive_before,
        "publishing must not write to a referenced source"
    );

    // --- full-scope push ----------------------------------------------
    let remote_id = lib
        .add_remote("Studio", server_dir.path().to_str().unwrap(), None)
        .unwrap();
    let outcome = gpp_core::remote::push_path_for(
        &lib,
        remote_id,
        &server,
        "2026/ana",
        published.path(),
        &PublishOptions::default(),
        false,
        SyncScopeKind::Full,
    )
    .unwrap();
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

    let manifest = server.manifest().unwrap();
    for name in ["inside.jpg", "outside.jpg", "neg.nef"] {
        let key = format!("{FULL_PREFIX}/2026/ana/{name}");
        assert!(manifest.contains_key(&key), "{key} never reached the server");
    }
    // Byte-identical, including the one that never left the drive.
    assert_eq!(
        server.get(&format!("{FULL_PREFIX}/2026/ana/outside.jpg")).unwrap(),
        std::fs::read(drive.path().join("ana/outside.jpg")).unwrap()
    );
    assert_eq!(
        server.get(&format!("{FULL_PREFIX}/2026/ana/neg.nef")).unwrap(),
        b"raw sensor bytes"
    );
    assert_eq!(
        file_count(drive.path()),
        drive_before,
        "a push must not write to a referenced source either"
    );
}

/// A full push whose referenced drive is unplugged names the frames it could
/// not read, uploads the rest — and does **not** propose deleting the missing
/// ones from the server.
///
/// That last part is the trap: the local manifest is what the planner compares
/// against, and a photograph dropped out of it reads as "deleted here". With
/// deletions allowed, a loose cable would have taken the client's originals off
/// the server.
#[test]
fn a_full_push_with_an_unplugged_drive_reports_and_deletes_nothing() {
    let lib_dir = tempfile::tempdir().unwrap();
    let holder = tempfile::tempdir().unwrap();
    let drive = holder.path().join("offload");
    let published = tempfile::tempdir().unwrap();
    let server_dir = tempfile::tempdir().unwrap();
    let server = FsTransport::new(server_dir.path());

    write_jpeg(&lib_dir.path().join("2026/ana/inside.jpg"), 120, 90);
    write_jpeg(&drive.join("ana/outside.jpg"), 130, 90);

    let lib = Library::open(lib_dir.path()).unwrap();
    lib.add_source(&drive, Some("Offload"), None).unwrap();
    let opts = ImportOptions::default();
    import_dir(&lib, lib_dir.path(), &opts, None, None).unwrap();
    import_dir(&lib, &drive, &opts, None, None).unwrap();

    lib.create_album(&NewAlbum { path: "2026/ana".into(), ..Default::default() }).unwrap();
    let ids: Vec<i64> = lib
        .photos(&PhotoFilter::default())
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    lib.add_photos_to_album("2026/ana", &ids).unwrap();

    let remote_id = lib
        .add_remote("Studio", server_dir.path().to_str().unwrap(), None)
        .unwrap();
    let push = |lib: &Library, allow_deletes: bool| {
        gpp_core::remote::push_path_for(
            lib,
            remote_id,
            &server,
            "2026/ana",
            published.path(),
            &PublishOptions::default(),
            allow_deletes,
            SyncScopeKind::Full,
        )
        .unwrap()
    };

    let outside_key = format!("{FULL_PREFIX}/2026/ana/outside.jpg");
    let inside_key = format!("{FULL_PREFIX}/2026/ana/inside.jpg");

    // --- the first push happens with the drive already gone ------------
    // Nothing of it is on the server yet, so the planner really does try to
    // upload it, and the read is where the absence shows up.
    std::fs::rename(&drive, holder.path().join("offload-elsewhere")).unwrap();
    let first = push(&lib, false);
    assert!(
        first
            .failed
            .iter()
            .any(|(k, why)| k.contains("outside.jpg") && why.contains("Offload")),
        "the unreadable frame must be named, or the gallery is one photograph \
         short and nothing said so: {:?}",
        first.failed
    );
    assert!(
        server.manifest().unwrap().contains_key(&inside_key),
        "the frames that were reachable still went up"
    );
    assert!(!server.manifest().unwrap().contains_key(&outside_key));

    // --- the drive comes back, everything goes up ----------------------
    std::fs::rename(holder.path().join("offload-elsewhere"), &drive).unwrap();
    assert!(push(&lib, false).failed.is_empty());
    assert!(server.manifest().unwrap().contains_key(&outside_key));

    // --- and now it leaves again, with deletions allowed ---------------
    // This is the trap: the local manifest is what the planner compares
    // against, so a photograph dropped out of it because its drive is
    // elsewhere reads as "deleted here". It stays in the manifest with the
    // hash the catalog already holds, and the absence surfaces at the read
    // instead — because a loose cable must never take a client's original off
    // the server.
    std::fs::rename(&drive, holder.path().join("offload-elsewhere")).unwrap();
    let outcome = push(&lib, true);
    assert_eq!(outcome.deleted_remote, 0, "a loose cable deleted from the server");
    assert!(outcome.withheld_deletes.is_empty(), "{:?}", outcome.withheld_deletes);
    assert!(
        server.manifest().unwrap().contains_key(&outside_key),
        "the client's original is gone from the server"
    );
    assert_eq!(
        server.get(&outside_key).unwrap().len(),
        std::fs::read(holder.path().join("offload-elsewhere/ana/outside.jpg"))
            .unwrap()
            .len(),
        "and it is still the photograph, untouched"
    );
}

// ------------------------------------------------------------------- xmp

/// A sidecar goes beside its own original — on the drive the photograph lives
/// on, never gathered into the library.
///
/// That is what makes the sidecars useful at all: they are how triage survives
/// a move to another tool, and a tool pointed at that drive has to find them
/// next to the frames they describe.
#[test]
fn xmp_sidecars_are_written_beside_the_originals_in_their_own_sources() {
    let lib_dir = tempfile::tempdir().unwrap();
    let drive = tempfile::tempdir().unwrap();
    write_jpeg(&lib_dir.path().join("2026/ana/inside.jpg"), 120, 90);
    write_jpeg(&drive.path().join("ana/outside.jpg"), 130, 90);

    let lib = Library::open(lib_dir.path()).unwrap();
    lib.add_source(drive.path(), Some("Offload"), None).unwrap();
    let opts = ImportOptions::default();
    import_dir(&lib, lib_dir.path(), &opts, None, None).unwrap();
    import_dir(&lib, drive.path(), &opts, None, None).unwrap();

    lib.create_album(&NewAlbum { path: "2026/ana".into(), ..Default::default() }).unwrap();
    let photos = lib.photos(&PhotoFilter::default()).unwrap();
    lib.add_photos_to_album("2026/ana", &photos.iter().map(|p| p.id).collect::<Vec<_>>())
        .unwrap();

    let outside = photos.iter().find(|p| p.filename == "outside.jpg").unwrap();
    lib.set_rating(outside.id, 4).unwrap();
    lib.set_photo_tags(outside.id, &["wedding".to_string()]).unwrap();

    let outcome = lib.export_xmp(Some("2026/ana")).unwrap();
    assert_eq!(outcome.written, 2);
    assert!(outcome.offline.is_empty());
    assert!(outcome.missing.is_empty());

    // Each sidecar sits next to its own frame, on its own root.
    let far = drive.path().join("ana/outside.xmp");
    assert!(far.is_file(), "the referenced photo's sidecar went somewhere else");
    assert!(lib_dir.path().join("2026/ana/inside.xmp").is_file());
    assert!(
        !lib_dir.path().join("ana/outside.xmp").exists(),
        "a sidecar must not be gathered into the library"
    );

    let packet = std::fs::read_to_string(&far).unwrap();
    assert!(packet.contains("xmp:Rating=\"4\""), "{packet}");
    assert!(packet.contains("wedding"), "{packet}");
}
