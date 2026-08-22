//! Develop, end to end: adjust a photo and check that every derived thing
//! follows — and that the original never moves.
//!
//! The unit tests in `develop.rs` prove the pixel maths. This proves the wiring:
//! catalog, thumbnail cache, publish, and the promise that says a photo you
//! adjusted and then reverted is byte-for-byte what it was.

use std::path::Path;

use gpp_core::albums::NewAlbum;
use gpp_core::develop::{self, EditOp, EditStack};
use gpp_core::publish::{publish_album, PublishOptions};
use gpp_core::session::{PublishTarget, Session};
use gpp_core::{Library, PhotoFilter};

/// A recognisable mid-grey frame: bright enough to see darkened, dark enough to
/// see lifted.
fn write_grey_jpeg(path: &Path, w: u32, h: u32, level: u8) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut img = image::RgbImage::new(w, h);
    for px in img.pixels_mut() {
        *px = image::Rgb([level, level, level]);
    }
    image::DynamicImage::ImageRgb8(img)
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

fn mean_luma(path: &Path) -> f64 {
    let img = image::open(path).unwrap().to_rgb8();
    let total: u64 = img.pixels().map(|p| p[0] as u64).sum();
    total as f64 / img.pixels().len() as f64
}

struct Fixture {
    session: Session,
    _root: tempfile::TempDir,
    dest: tempfile::TempDir,
    photo_id: i64,
    original: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let original = root.path().join("2026/ana/one.jpg");
    write_grey_jpeg(&original, 120, 80, 128);

    let session = Session::new();
    session.open_library(root.path()).unwrap();
    session.import(None, None).unwrap();
    session
        .create_album(NewAlbum {
            path: "2026/ana".into(),
            ..Default::default()
        })
        .unwrap();
    let photo_id = session.photos(PhotoFilter::default()).unwrap()[0].id;
    session.add_to_album("2026/ana".into(), vec![photo_id]).unwrap();
    session
        .set_publish_target(PublishTarget {
            dest: Some(dest.path().display().to_string()),
            min_rating: None,
        })
        .unwrap();

    Fixture {
        session,
        _root: root,
        dest,
        photo_id,
        original,
    }
}

#[test]
fn adjusting_a_photo_never_touches_the_original() {
    let f = fixture();
    let before = std::fs::read(&f.original).unwrap();

    f.session
        .set_photo_edit(vec![f.photo_id], EditOp::Exposure { ev: -1.0 })
        .unwrap();
    f.session
        .set_photo_edit(vec![f.photo_id], EditOp::BlackAndWhite)
        .unwrap();

    assert_eq!(
        std::fs::read(&f.original).unwrap(),
        before,
        "the original file is never written to"
    );
}

#[test]
fn the_thumbnail_the_grid_asks_for_shows_the_adjustment() {
    let f = fixture();

    let plain = f.session.thumbnail_path(f.photo_id, "small").unwrap();
    assert!(Path::new(&plain).exists(), "import built the untouched thumbnail");
    let plain_luma = mean_luma(Path::new(&plain));

    f.session
        .set_photo_edit(vec![f.photo_id], EditOp::Exposure { ev: -1.0 })
        .unwrap();

    let edited = f.session.thumbnail_path(f.photo_id, "small").unwrap();
    assert_ne!(edited, plain, "an edit moves the photo to a new render key");
    assert!(Path::new(&edited).exists(), "and that key has thumbnails");
    assert!(
        mean_luma(Path::new(&edited)) < plain_luma - 30.0,
        "one stop down is visible in the thumbnail"
    );

    // The untouched thumbnail is still there — reverting is a cache hit, not a
    // re-render.
    assert!(Path::new(&plain).exists());
}

#[test]
fn reverting_returns_the_photo_exactly_where_it_started() {
    let f = fixture();
    let plain = f.session.thumbnail_path(f.photo_id, "small").unwrap();

    f.session
        .set_photo_edit(vec![f.photo_id], EditOp::Exposure { ev: 1.5 })
        .unwrap();
    assert_ne!(f.session.thumbnail_path(f.photo_id, "small").unwrap(), plain);

    f.session.reset_photo_edits(vec![f.photo_id]).unwrap();

    assert!(f.session.photo_edits(f.photo_id).unwrap().is_empty());
    assert_eq!(
        f.session.thumbnail_path(f.photo_id, "small").unwrap(),
        plain,
        "back on the original render key"
    );
}

#[test]
fn publish_ships_the_adjusted_pixels() {
    let f = fixture();
    let published = f.dest.path().join("2026/ana/one.jpg");

    f.session.publish(Some("2026/ana".into())).unwrap();
    let plain_luma = mean_luma(&published);
    assert!(
        (plain_luma - 128.0).abs() < 3.0,
        "untouched publish is the original, got {plain_luma}"
    );

    f.session
        .set_photo_edit(vec![f.photo_id], EditOp::Exposure { ev: -1.0 })
        .unwrap();
    f.session.publish(Some("2026/ana".into())).unwrap();

    let edited_luma = mean_luma(&published);
    assert!(
        edited_luma < 80.0,
        "publish ships the adjustment, got {edited_luma}"
    );

    // And reverting re-publishes the original, rather than leaving the darkened
    // copy online.
    f.session.reset_photo_edits(vec![f.photo_id]).unwrap();
    f.session.publish(Some("2026/ana".into())).unwrap();
    assert!(
        (mean_luma(&published) - 128.0).abs() < 3.0,
        "reverting reaches the published gallery too"
    );
}

#[test]
fn one_adjustment_applies_to_a_whole_selection() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        write_grey_jpeg(&root.path().join(name), 60, 40, 200);
    }
    let session = Session::new();
    session.open_library(root.path()).unwrap();
    session.import(None, None).unwrap();

    let ids: Vec<i64> = session
        .photos(PhotoFilter::default())
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids.len(), 3);

    let changed = session
        .set_photo_edit(ids.clone(), EditOp::Exposure { ev: -2.0 })
        .unwrap();
    assert_eq!(changed, 3, "every selected photo took the adjustment");

    for id in &ids {
        let thumb = session.thumbnail_path(*id, "small").unwrap();
        assert!(mean_luma(Path::new(&thumb)) < 100.0, "photo {id} was darkened");
    }

    // Setting the same value again reports no change, so a UI can tell the
    // difference between "applied" and "already like that".
    assert_eq!(
        session.set_photo_edit(ids, EditOp::Exposure { ev: -2.0 }).unwrap(),
        0
    );
}

#[test]
fn a_full_size_render_is_cached_once_and_reused() {
    let f = fixture();
    let lib = Library::open(f._root.path()).unwrap();
    let photo = lib.photo_by_id(f.photo_id).unwrap();

    let mut stack = EditStack::new();
    stack.set(EditOp::Contrast { amount: 40.0 });
    let key = develop::render_key(&photo.content_hash, &stack);
    let cached = develop::render_cache_path(&lib.thumb_dir(), &key);
    assert!(!cached.exists());

    let first = develop::ensure_rendered(&f.original, &lib.thumb_dir(), &photo, &stack).unwrap();
    assert_eq!(first, cached);
    let stamp = std::fs::metadata(&cached).unwrap().len();

    let second = develop::ensure_rendered(&f.original, &lib.thumb_dir(), &photo, &stack).unwrap();
    assert_eq!(second, cached);
    assert_eq!(std::fs::metadata(&cached).unwrap().len(), stamp, "reused, not re-rendered");

    // With no edits there is no cache entry at all: the original is the answer.
    let plain = develop::ensure_rendered(
        &f.original,
        &lib.thumb_dir(),
        &photo,
        &EditStack::new(),
    )
    .unwrap();
    assert_eq!(plain, f.original);
}

#[test]
fn a_rejected_photo_is_still_excluded_when_it_has_edits() {
    // Develop and selection are independent: adjusting a frame does not sneak it
    // past the publish filters.
    let f = fixture();
    let lib = Library::open(f._root.path()).unwrap();

    f.session
        .set_photo_edit(vec![f.photo_id], EditOp::Exposure { ev: 1.0 })
        .unwrap();
    lib.set_flag(f.photo_id, gpp_core::Flag::Reject).unwrap();

    let r = publish_album(&lib, "2026/ana", f.dest.path(), &PublishOptions::default()).unwrap();
    assert_eq!(r.photos_copied, 0);
    assert!(!f.dest.path().join("2026/ana/one.jpg").exists());
}
