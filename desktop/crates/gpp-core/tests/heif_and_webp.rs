//! HEIC and WebP have to be first-class, because a wedding arrives in them.
//!
//! An iPhone shoots HEIC by default and guests send HEIC; anything exported for
//! the web is likely WebP. A format the catalog does not recognise is not
//! merely unsupported — `classify` returns `None`, the scan skips it, and the
//! photograph is invisible: not imported, not flagged, not counted. The
//! photographer is told nothing at all.
//!
//! The HEIC fixture is genuine HEVC in a `heic`-branded container, the codec an
//! iPhone actually writes, produced with `heif-enc`. AV1-in-HEIF would exercise
//! a different decoder entirely and prove nothing about iPhone files.

use gpp_core::media::{self, classify};
use gpp_core::model::PhotoKind;
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn heic_and_webp_are_recognised_as_photographs() {
    for name in ["IMG_4021.HEIC", "IMG_4021.heic", "scan.heif", "export.webp"] {
        assert_eq!(
            classify(Path::new(name)),
            Some(PhotoKind::Photo),
            "{name} would be skipped by the scan and never reach the catalog"
        );
    }
}

#[test]
fn a_heic_off_an_iphone_decodes() {
    let img = media::load_oriented(&fixture("hevc.heic"), None)
        .expect("an iPhone photograph must be decodable, or it gets no preview at all");
    assert_eq!((img.width(), img.height()), (1024, 768));
}

#[test]
fn a_webp_decodes() {
    let img = media::load_oriented(&fixture("sample.webp"), None).expect("webp must decode");
    assert_eq!((img.width(), img.height()), (64, 48));
}

/// The fast metadata-only import path reads dimensions without decoding the
/// whole frame. It must still answer for a HEIC, or a metadata-only import
/// flags every iPhone photo as broken.
#[test]
fn dimensions_are_readable_without_a_full_decode() {
    assert_eq!(media::read_dimensions(&fixture("hevc.heic")).unwrap(), (1024, 768));
    assert_eq!(media::read_dimensions(&fixture("sample.webp")).unwrap(), (64, 48));
}

/// What reaches a client has to be a file their browser can open.
///
/// Chrome and Firefox cannot display HEIC at all — only Safari can — so a HEIC
/// in the published tree is a broken image for most of the people the gallery
/// exists for. And once a frame carries an adjustment the published bytes are a
/// JPEG regardless, because that is what the renderer emits; shipping those
/// under a `.HEIC` name is simply a lie about the file, and the client who
/// downloads it gets something their photo viewer refuses.
///
/// So a HEIF original is published as JPEG either way. The name is then the
/// same whether the photograph has been developed or not, which matters because
/// the published filename is the URL — adding an adjustment must not silently
/// move a client's photograph to a different address.
#[test]
fn a_heic_is_published_as_a_jpeg_a_browser_can_open() {
    use gpp_core::albums::NewAlbum;
    use gpp_core::import::{import_dir, ImportOptions};
    use gpp_core::publish::{publish_album, PublishOptions};

    let lib_dir = tempfile::tempdir().unwrap();
    let lib = gpp_core::Library::open(lib_dir.path()).unwrap();
    let shoot = lib_dir.path().join("shoot");
    std::fs::create_dir_all(&shoot).unwrap();
    std::fs::copy(fixture("hevc.heic"), shoot.join("IMG_4021.HEIC")).unwrap();

    import_dir(&lib, &shoot, &ImportOptions::default(), None, None).unwrap();
    lib.create_album(&NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
    let ids: Vec<i64> = lib.photos(&Default::default()).unwrap().iter().map(|p| p.id).collect();
    lib.add_photos_to_album("a", &ids).unwrap();

    let dest = tempfile::tempdir().unwrap();
    let out = publish_album(&lib, "a", dest.path(), &PublishOptions::default()).unwrap();
    assert!(out.unrenderable.is_empty(), "the frame did not publish: {out:?}");

    let published = dest.path().join("a/IMG_4021.jpg");
    assert!(
        published.exists(),
        "expected a .jpg; the tree holds {:?}",
        std::fs::read_dir(dest.path().join("a")).unwrap()
            .map(|e| e.unwrap().file_name()).collect::<Vec<_>>()
    );

    // And the bytes must really be a JPEG, not HEIC wearing a new extension.
    let bytes = std::fs::read(&published).unwrap();
    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "not JPEG bytes");

    // photoOrder is what the site sorts by, so it has to name the file that is
    // actually there or the order silently stops applying.
    let index = std::fs::read_to_string(dest.path().join("a/index.md")).unwrap();
    assert!(
        !index.contains("IMG_4021.HEIC"),
        "index.md still names a file that was never written:\n{index}"
    );
}
