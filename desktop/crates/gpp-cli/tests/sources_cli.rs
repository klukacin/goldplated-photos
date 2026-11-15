//! "Import my Lightroom library without moving a terabyte", driven from the
//! command line — the composition a photographer actually types.
//!
//! Two routes to the same end, both through the built `gpp` binary: register
//! the folder and import it (`sources add` + `import`), or hand the whole thing
//! to `lr import --mode reference` and let it register the roots itself. Either
//! way the assertion that matters is the same one: **the files never move.**

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

/// Every file under a folder, so "the library stayed empty" is a measurement.
fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().map(|n| n.to_string_lossy().starts_with('.')) == Some(true) {
            continue;
        }
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path.display().to_string());
        }
    }
    out.sort();
    out
}

/// One root folder on an outside drive, so an import has to either copy it or
/// reference it.
fn write_lrcat(lrcat: &Path, shoot: &Path) {
    let conn = rusqlite::Connection::open(lrcat).unwrap();
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE AgLibraryRootFolder (id_local INTEGER, absolutePath TEXT, name TEXT);
        CREATE TABLE AgLibraryFolder (id_local INTEGER, pathFromRoot TEXT, rootFolder INTEGER);
        CREATE TABLE AgLibraryFile (id_local INTEGER, idx_filename TEXT, folder INTEGER);
        CREATE TABLE Adobe_images (id_local INTEGER, rootFile INTEGER, rating REAL,
          colorLabels TEXT, pick REAL);
        INSERT INTO AgLibraryRootFolder VALUES (1, '{shoot}/', 'shoot');
        INSERT INTO AgLibraryFolder VALUES (10, '', 1);
        INSERT INTO AgLibraryFile VALUES (100, 'one.jpg', 10);
        INSERT INTO AgLibraryFile VALUES (101, 'two.jpg', 10);
        INSERT INTO Adobe_images VALUES (1000, 100, 4.0, NULL, 0.0);
        INSERT INTO Adobe_images VALUES (1001, 101, NULL, NULL, 0.0);
        "#,
        shoot = shoot.display()
    ))
    .unwrap();
}

#[test]
fn sources_add_then_import_catalogues_an_outside_drive_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let drive = dir.path().join("archive-2019");
    write_jpeg(&drive.join("ana/a1.jpg"), 60, 40);
    write_jpeg(&drive.join("ana/a2.jpg"), 70, 40);
    let before = files(&drive);
    let root_arg = root.display().to_string();
    let drive_arg = drive.display().to_string();

    let (ok, out, err) = gpp(&["init", &root_arg]);
    assert!(ok, "init failed: {out}{err}");

    // Without a source, that folder would be copied into the library.
    let (ok, out, err) = gpp(&[
        "sources", "add", &drive_arg, "--name", "Archive", "--library", &root_arg,
    ]);
    assert!(ok, "sources add failed: {out}{err}");
    assert!(out.contains("added source"), "{out}");

    let (ok, out, err) = gpp(&["import", &drive_arg, "--library", &root_arg]);
    assert!(ok, "import failed: {out}{err}");
    assert!(out.contains("imported 2"), "{out}");
    assert!(
        !out.contains("copied"),
        "a folder inside a source must not be copied in: {out}"
    );

    // Nothing moved, and nothing landed in the library folder.
    assert_eq!(files(&drive), before, "the drive was modified");
    assert!(files(&root).is_empty(), "the library holds photographs: {:?}", files(&root));

    // The rows carry paths relative to the drive, not to the library.
    let (ok, out, _) = gpp(&["ls", "--json", "--library", &root_arg]);
    assert!(ok);
    let photos: serde_json::Value = serde_json::from_str(&out).unwrap();
    let photos = photos.as_array().unwrap();
    assert_eq!(photos.len(), 2);
    assert!(photos.iter().all(|p| p["rel_path"].as_str().unwrap().starts_with("ana/")));

    // `sources` lists both roots and says which is which.
    let (ok, out, err) = gpp(&["sources", "--library", &root_arg]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("primary"), "{out}");
    assert!(out.contains("Archive") && out.contains("online"), "{out}");

    // A source that still holds photographs needs an answer before it goes…
    let (ok, _out, err) = gpp(&["sources", "rm", "2", "--library", &root_arg]);
    assert!(!ok, "removing a source with photos must be refused");
    assert!(err.contains("Archive"), "{err}");

    // …and even then, the photographs on disk are untouched.
    let (ok, out, err) = gpp(&["sources", "rm", "2", "--drop-photos", "--library", &root_arg]);
    assert!(ok, "{out}{err}");
    assert_eq!(files(&drive), before, "removing a source deleted a photograph");
}

#[test]
fn lr_import_in_reference_mode_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let shoot = dir.path().join("shoot");
    write_jpeg(&shoot.join("one.jpg"), 60, 40);
    write_jpeg(&shoot.join("two.jpg"), 60, 40);
    let before = files(&shoot);
    let lrcat = dir.path().join("photos.lrcat");
    write_lrcat(&lrcat, &shoot);
    let root_arg = root.display().to_string();
    let lrcat_arg = lrcat.display().to_string();

    let (ok, out, err) = gpp(&["init", &root_arg]);
    assert!(ok, "{out}{err}");

    // The scan offers the choice rather than only announcing the copy.
    let (ok, out, err) = gpp(&["lr", "scan", &lrcat_arg, "--library", &root_arg]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("--mode reference"), "the scan must offer it: {out}");

    let (ok, out, err) = gpp(&[
        "lr", "import", &lrcat_arg, "--mode", "reference", "--library", &root_arg,
    ]);
    assert!(ok, "{out}{err}");
    assert!(out.contains("0 copied"), "{out}");
    assert!(out.contains("2 referenced"), "{out}");
    assert!(out.contains("registered as a source"), "{out}");

    assert_eq!(files(&shoot), before, "reference mode moved or changed a file");
    assert!(files(&root).is_empty(), "reference mode copied into the library");
    assert!(!root.join("lr").exists());

    let (ok, out, _) = gpp(&["sources", "--library", &root_arg]);
    assert!(ok);
    assert!(out.contains("shoot"), "the LR root became a named source: {out}");

    // The photographs are catalogued, with Lightroom's rating, where they are.
    let (_, out, _) = gpp(&["ls", "--json", "--library", &root_arg]);
    let photos: serde_json::Value = serde_json::from_str(&out).unwrap();
    let one = photos
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["filename"] == "one.jpg")
        .expect("one.jpg catalogued");
    assert_eq!(one["rel_path"], "one.jpg", "relative to its own source");
    assert_eq!(one["rating"], 4);
}

/// The drive moved. `sources relocate` re-points it — but only at a folder that
/// really holds these photographs.
#[test]
fn sources_relocate_refuses_the_wrong_folder_and_accepts_the_right_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let first = dir.path().join("mount-a");
    let second = dir.path().join("mount-b");
    let decoy = dir.path().join("decoy");
    write_jpeg(&first.join("ana/a1.jpg"), 60, 40);
    write_jpeg(&decoy.join("ana/a1.jpg"), 90, 60);
    let root_arg = root.display().to_string();

    gpp(&["init", &root_arg]);
    gpp(&[
        "sources",
        "add",
        &first.display().to_string(),
        "--name",
        "Card",
        "--library",
        &root_arg,
    ]);
    let (ok, out, err) = gpp(&["import", &first.display().to_string(), "--library", &root_arg]);
    assert!(ok, "{out}{err}");

    // Same layout, different photograph: refused, and the file is named.
    let (ok, _out, err) = gpp(&[
        "sources", "relocate", "2", "--to", &decoy.display().to_string(),
        "--library", &root_arg,
    ]);
    assert!(!ok, "a folder holding different photographs must be refused");
    assert!(err.contains("ana/a1.jpg"), "the refusal must name the file: {err}");

    // The real drive, remounted elsewhere: accepted.
    std::fs::create_dir_all(second.join("ana")).unwrap();
    std::fs::copy(first.join("ana/a1.jpg"), second.join("ana/a1.jpg")).unwrap();
    let (ok, out, err) = gpp(&[
        "sources", "relocate", "2", "--to", &second.display().to_string(),
        "--library", &root_arg,
    ]);
    assert!(ok, "{out}{err}");

    let (_, out, _) = gpp(&["sources", "--library", &root_arg]);
    assert!(out.contains("mount-b"), "{out}");
    assert!(out.contains("online"), "{out}");
}
