//! A remote that does not play by the rules.
//!
//! Every other sync test uses [`gpp_core::sync::FsTransport`], which is a
//! folder on disk and therefore can only ever describe files that a folder can
//! hold. An HTTP remote is not that: its manifest is a JSON document the
//! *server* writes, and a pull trusts it against this machine's disk. So the
//! interesting question — what happens when the server names something a
//! well-behaved one never would — needs a transport that can say anything.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use gpp_core::remote;
use gpp_core::sync::{Manifest, RemoteTransport};
use gpp_core::Library;

/// A server whose manifest is whatever the test says it is.
struct Hostile {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl Hostile {
    fn serving(pairs: &[(&str, &[u8])]) -> Self {
        Self {
            files: Mutex::new(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_vec()))
                    .collect(),
            ),
        }
    }
}

impl RemoteTransport for Hostile {
    fn manifest(&self) -> gpp_core::Result<Manifest> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), blake3::hash(v).to_hex().to_string()))
            .collect())
    }
    fn put(&self, rel: &str, bytes: &[u8]) -> gpp_core::Result<()> {
        self.files
            .lock()
            .unwrap()
            .insert(rel.to_string(), bytes.to_vec());
        Ok(())
    }
    fn get(&self, rel: &str) -> gpp_core::Result<Vec<u8>> {
        self.files
            .lock()
            .unwrap()
            .get(rel)
            .cloned()
            .ok_or_else(|| gpp_core::Error::other(format!("no such file: {rel}")))
    }
    fn delete(&self, rel: &str) -> gpp_core::Result<()> {
        self.files.lock().unwrap().remove(rel);
        Ok(())
    }
}

/// One machine's two trees, side by side under a sandbox we can inspect whole.
struct Machine {
    lib: Library,
    library_root: std::path::PathBuf,
    published: std::path::PathBuf,
    _sandbox: tempfile::TempDir,
}

fn machine() -> Machine {
    let sandbox = tempfile::tempdir().unwrap();
    let library_root = sandbox.path().join("library");
    let published = sandbox.path().join("published");
    std::fs::create_dir_all(&library_root).unwrap();
    std::fs::create_dir_all(&published).unwrap();
    let lib = Library::open(&library_root).unwrap();
    Machine {
        lib,
        library_root,
        published,
        _sandbox: sandbox,
    }
}

/// Every file under `root`, as '/'-separated relative paths.
fn files_under(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// A minimal album index the parser accepts.
const INDEX: &[u8] = b"---\ntitle: \"Ana\"\nsort: \"date-desc\"\nstyle: \"grid\"\n---\n";

/// Dot-named files are the gallery's own: `.meta/` holds the proofing
/// submissions the site writes, and `.htaccess` is configuration the web server
/// obeys. Both trees this machine keeps already refuse to look at them —
/// `publish::manifest_of` and `FsTransport::walk` skip anything starting with a
/// dot — so nothing this machine publishes or pushes can ever be one.
///
/// A pull did not apply the same rule to what the server named, and the
/// published tree is what `npm run deploy` rsyncs to the live site. So a
/// compromised server could put an `.htaccess` on the photographer's web host,
/// and neither manifest would ever mention the file again: it cannot be
/// reconciled, pruned or even listed by anything in the app.
#[test]
fn a_pull_refuses_the_dot_named_files_every_manifest_already_skips() {
    let m = machine();
    let server = Hostile::serving(&[
        ("2026/ana/index.md", INDEX),
        ("2026/ana/.htaccess", b"Options +ExecCGI\n"),
        ("2026/ana/.meta", b"not the server's proofing folder any more"),
    ]);

    let outcome = remote::pull_album(&m.lib, &server, "2026/ana", &m.published).unwrap();

    let published = files_under(&m.published);
    let library = files_under(&m.library_root);
    assert!(
        !published.iter().any(|f| f.contains("/.")),
        "the server's dot-files reached the tree that gets deployed: {published:?}"
    );
    assert!(
        !library.iter().any(|f| f.contains("/.")),
        "the server's dot-files reached the library: {library:?}"
    );

    // Refused, not silently dropped: the photographer has to be able to see
    // that the server asked for something this machine would not do.
    assert_eq!(
        outcome.rejected,
        vec![
            "2026/ana/.htaccess".to_string(),
            "2026/ana/.meta".to_string()
        ],
        "a refused path is named in the outcome"
    );
    // And the album itself still arrived.
    assert!(published.contains(&"2026/ana/index.md".to_string()));
}

/// `sync::apply` treats one failing file as one failing file and carries on —
/// "a single file failing is reported and the rest of the transfer continues".
/// The download half of a pull does not go through `apply`, and it took the
/// opposite line: one path the local filesystem cannot accept aborted the whole
/// album, so a server naming `..` once was enough to stop that album syncing
/// for good, with the photos that had been fetched already left uncatalogued.
#[test]
fn one_impossible_path_does_not_abort_the_whole_album() {
    let m = machine();
    let server = Hostile::serving(&[
        ("2026/ana/index.md", INDEX),
        ("2026/ana/..", b"climb"),
        ("2026/ana/", b"the album's own directory"),
        ("2026/ana/real.jpg", &jpeg_bytes()),
    ]);

    let outcome = remote::pull_album(&m.lib, &server, "2026/ana", &m.published)
        .expect("one bad name must not fail the album");

    assert!(
        outcome.rejected.len() == 2,
        "both impossible names are reported: {:?}",
        outcome.rejected
    );
    assert!(
        m.library_root.join("2026/ana/real.jpg").is_file(),
        "the real photo still arrived: {:?}",
        files_under(&m.library_root)
    );
    assert_eq!(outcome.photos_imported, 1);
}

fn jpeg_bytes() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(24, 16)
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .unwrap();
    buf.into_inner()
}
