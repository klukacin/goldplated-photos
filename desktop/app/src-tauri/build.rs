fn main() {
    // The UI is embedded into the binary at compile time. Without this, editing
    // app.js/index.html/style.css and running `cargo build` produces a binary
    // that still carries the previous frontend — the build looks successful and
    // the change simply is not there.
    for asset in [
        "../ui/index.html",
        "../ui/app.js",
        "../ui/develop-queue.js",
        "../ui/features.js",
        "../ui/prefs.js",
        "../ui/style.css",
    ] {
        println!("cargo:rerun-if-changed={asset}");
    }

    copy_titlebar_icon();

    tauri_build::build()
}

/// Put the app's own mark where the webview can load it.
///
/// The titlebar draws the bundle icon, but `frontendDist` is `../ui` and the
/// webview cannot reach `icons/`. Copying at build time rather than checking in
/// a second file: two copies of one image drift, and the repository has already
/// been bitten once by `.gitignore`'s blanket `*.png` swallowing the app icons —
/// a tracked duplicate here would need its own negation and its own memory of
/// why. The generated copy stays ignored; the icon beside this file is the one
/// source of truth.
fn copy_titlebar_icon() {
    const SOURCE: &str = "icons/128x128.png";
    const DEST: &str = "../ui/app-icon.png";
    println!("cargo:rerun-if-changed={SOURCE}");

    let icon = std::fs::read(SOURCE).expect("the bundle icon is missing");
    // Only when it differs. Writing on every build would change the mtime of a
    // file inside the watched UI directory, and rebuild the frontend forever.
    if std::fs::read(DEST).map(|old| old != icon).unwrap_or(true) {
        std::fs::write(DEST, &icon).expect("could not write the titlebar icon");
    }
}
