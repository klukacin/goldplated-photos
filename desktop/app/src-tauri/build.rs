fn main() {
    // The UI is embedded into the binary at compile time. Without this, editing
    // app.js/index.html/style.css and running `cargo build` produces a binary
    // that still carries the previous frontend — the build looks successful and
    // the change simply is not there.
    for asset in ["../ui/index.html", "../ui/app.js", "../ui/style.css"] {
        println!("cargo:rerun-if-changed={asset}");
    }

    tauri_build::build()
}
