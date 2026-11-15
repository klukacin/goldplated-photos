//! "Add library" is not a core method — it is `init` + `lr import` composed,
//! which is exactly what the CLI (and the shell) do. This drives the built
//! `gpp` binary end to end over a generated Lightroom fixture: a fresh
//! library whose root already CONTAINS the catalog's photo tree, so the
//! import must catalogue in place and copy nothing.

use std::path::Path;
use std::process::Command;

fn gpp(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_gpp"))
        .args(args)
        .output()
        .expect("the gpp binary should run");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn write_jpeg(path: &Path, w: u32, h: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::DynamicImage::new_rgb8(w, h)
        .save_with_format(path, image::ImageFormat::Jpeg)
        .unwrap();
}

/// A one-root catalog whose root folder is `shoot` under the library root.
fn write_lrcat(lrcat: &Path, shoot: &Path) {
    let conn = rusqlite::Connection::open(lrcat).unwrap();
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE AgLibraryRootFolder (id_local INTEGER, absolutePath TEXT, name TEXT);
        CREATE TABLE AgLibraryFolder (id_local INTEGER, pathFromRoot TEXT, rootFolder INTEGER);
        CREATE TABLE AgLibraryFile (id_local INTEGER, idx_filename TEXT, folder INTEGER);
        CREATE TABLE Adobe_images (id_local INTEGER, rootFile INTEGER, rating REAL,
          colorLabels TEXT, pick REAL);
        CREATE TABLE AgLibraryCollection (id_local INTEGER, name TEXT, parent INTEGER,
          creationId TEXT);
        CREATE TABLE AgLibraryCollectionimage (id_local INTEGER, collection INTEGER,
          image INTEGER, positionInCollection REAL);
        INSERT INTO AgLibraryRootFolder VALUES (1, '{shoot}/', 'shoot');
        INSERT INTO AgLibraryFolder VALUES (10, '', 1);
        INSERT INTO AgLibraryFile VALUES (100, 'one.jpg', 10);
        INSERT INTO AgLibraryFile VALUES (101, 'two.jpg', 10);
        INSERT INTO Adobe_images VALUES (1000, 100, 4.0, NULL, 0.0);
        INSERT INTO Adobe_images VALUES (1001, 101, NULL, NULL, 0.0);
        INSERT INTO AgLibraryCollection VALUES (1, 'Picks', NULL,
          'com.adobe.ag.library.collection');
        INSERT INTO AgLibraryCollectionimage VALUES (1, 1, 1000, 1.0);
        "#,
        shoot = shoot.display()
    ))
    .unwrap();
}

#[test]
fn init_then_lr_import_catalogues_an_inside_tree_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let shoot = root.join("shoot");
    write_jpeg(&shoot.join("one.jpg"), 60, 40);
    write_jpeg(&shoot.join("two.jpg"), 60, 40);
    let lrcat = dir.path().join("photos.lrcat");
    write_lrcat(&lrcat, &shoot);
    let root_arg = root.display().to_string();
    let lrcat_arg = lrcat.display().to_string();

    // 1. Create the library at the chosen root.
    let (ok, out, err) = gpp(&["init", &root_arg]);
    assert!(ok, "init failed: {out}{err}");

    // 2. The scan says the root folder can import in place.
    let (ok, out, err) = gpp(&["lr", "scan", &lrcat_arg, "--library", &root_arg]);
    assert!(ok, "scan failed: {out}{err}");
    assert!(out.contains("in place"), "the scan must say in-place is possible: {out}");

    // 3. Import: everything catalogued where it lies, nothing copied.
    let (ok, out, err) = gpp(&["lr", "import", &lrcat_arg, "--library", &root_arg]);
    assert!(ok, "import failed: {out}{err}");
    assert!(out.contains("0 copied"), "nothing should have been copied: {out}");
    assert!(out.contains("2 in place"), "{out}");
    assert!(!root.join("lr").exists(), "no copy tree should appear");

    // The photos are catalogued at their own paths, with LR's rating.
    let (ok, out, _) = gpp(&["ls", "--json", "--library", &root_arg]);
    assert!(ok);
    let photos: serde_json::Value = serde_json::from_str(&out).unwrap();
    let photos = photos.as_array().unwrap();
    assert_eq!(photos.len(), 2);
    let one = photos
        .iter()
        .find(|p| p["filename"] == "one.jpg")
        .expect("one.jpg catalogued");
    assert_eq!(one["rel_path"], "shoot/one.jpg", "in place, not under lr/");
    assert_eq!(one["rating"], 4);

    // The collection became an album holding its member.
    let (ok, out, _) = gpp(&["album", "show", "picks", "--library", &root_arg]);
    assert!(ok, "the Picks collection should be an album now: {out}");
    assert!(out.contains("one.jpg"), "{out}");

    // 4. Run it again: a sync, not a duplication.
    let (ok, out, err) = gpp(&["lr", "import", &lrcat_arg, "--library", &root_arg]);
    assert!(ok, "{out}{err}");
    let (_, out, _) = gpp(&["ls", "--json", "--library", &root_arg]);
    let photos: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(photos.as_array().unwrap().len(), 2, "the second run duplicated rows");
}

#[test]
fn a_dry_run_from_the_cli_reports_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let shoot = root.join("shoot");
    write_jpeg(&shoot.join("one.jpg"), 60, 40);
    write_jpeg(&shoot.join("two.jpg"), 60, 40);
    let lrcat = dir.path().join("photos.lrcat");
    write_lrcat(&lrcat, &shoot);
    let root_arg = root.display().to_string();

    let (ok, out, err) = gpp(&["init", &root_arg]);
    assert!(ok, "{out}{err}");
    let (ok, out, err) = gpp(&[
        "lr",
        "import",
        &lrcat.display().to_string(),
        "--dry-run",
        "--library",
        &root_arg,
    ]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("dry run"), "{out}");

    let (_, out, _) = gpp(&["ls", "--json", "--library", &root_arg]);
    let photos: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(photos.as_array().unwrap().len(), 0, "a dry run catalogued something");
}
