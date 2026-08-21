//! Application session — the operations a UI actually performs.
//!
//! Every desktop/mobile command maps to exactly one method here. The Tauri
//! layer is a one-line wrapper per command, which keeps the untestable part of
//! the stack (the webview shell) trivial and puts all real behaviour under
//! test on any platform.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::albums::{AlbumUpdate, NewAlbum};
use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::import::{import_dir, ImportOptions};
use crate::model::{Album, Flag, ImportSummary, Photo, PhotoFilter};
use crate::publish::{publish_album, PublishOptions, PublishResult};

/// Holds the currently open library. `None` until the user picks one.
#[derive(Default)]
pub struct Session {
    library: RwLock<Option<Library>>,
}

/// What the UI shows in the sidebar for one album.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumSummary {
    #[serde(flatten)]
    pub album: Album,
    pub photo_count: usize,
    pub is_locked: bool,
}

/// Library-level status for the header bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryStatus {
    pub root: String,
    pub photo_count: i64,
    pub album_count: usize,
    pub cameras: Vec<String>,
}

/// Settings the UI persists between launches.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublishTarget {
    /// Absolute path of the gallery's `src/content/albums`.
    pub dest: Option<String>,
    pub min_rating: Option<u8>,
}

const SETTING_PUBLISH_DEST: &str = "publish.dest";
const SETTING_PUBLISH_MIN_RATING: &str = "publish.min_rating";
const SETTING_REMOTE_DIR: &str = "remote.dir";

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open (or create) a library and make it current.
    pub fn open_library(&self, root: impl AsRef<Path>) -> Result<LibraryStatus> {
        let lib = Library::open(root)?;
        let status = Self::status_of(&lib)?;
        *self.library.write().expect("session lock poisoned") = Some(lib);
        Ok(status)
    }

    pub fn is_open(&self) -> bool {
        self.library.read().expect("session lock poisoned").is_some()
    }

    /// Run `f` against the open library, or fail with a clear message.
    fn with<T>(&self, f: impl FnOnce(&Library) -> Result<T>) -> Result<T> {
        let guard = self.library.read().expect("session lock poisoned");
        let lib = guard
            .as_ref()
            .ok_or_else(|| Error::other("no library is open"))?;
        f(lib)
    }

    fn status_of(lib: &Library) -> Result<LibraryStatus> {
        Ok(LibraryStatus {
            root: lib.root().display().to_string(),
            photo_count: lib.photo_count()?,
            album_count: lib.albums()?.len(),
            cameras: lib.camera_models()?,
        })
    }

    pub fn status(&self) -> Result<LibraryStatus> {
        self.with(Self::status_of)
    }

    // ------------------------------------------------------------- import

    pub fn import(
        &self,
        dir: Option<String>,
        on_progress: Option<&(dyn Fn(crate::model::ImportProgress) + Sync)>,
    ) -> Result<ImportSummary> {
        self.with(|lib| {
            let target = match &dir {
                Some(d) => PathBuf::from(d),
                None => lib.root().to_path_buf(),
            };
            import_dir(lib, &target, &ImportOptions::default(), on_progress)
        })
    }

    /// Drop catalog rows whose files are gone.
    pub fn prune(&self) -> Result<usize> {
        self.with(|lib| lib.prune_missing())
    }

    // -------------------------------------------------------------- photos

    pub fn photos(&self, filter: PhotoFilter) -> Result<Vec<Photo>> {
        self.with(|lib| lib.photos(&filter))
    }

    pub fn photo(&self, id: i64) -> Result<Photo> {
        self.with(|lib| lib.photo_by_id(id))
    }

    /// Absolute path of a photo — the shell turns this into an asset URL.
    pub fn photo_path(&self, id: i64) -> Result<String> {
        self.with(|lib| {
            let photo = lib.photo_by_id(id)?;
            Ok(lib.resolve(&photo.rel_path)?.display().to_string())
        })
    }

    /// Absolute path of a cached thumbnail, generating nothing — the import
    /// already produced these.
    pub fn thumbnail_path(&self, id: i64, size: &str) -> Result<String> {
        self.with(|lib| {
            let photo = lib.photo_by_id(id)?;
            let p = crate::media::thumb_path(&lib.thumb_dir(), &photo.content_hash, size);
            Ok(p.display().to_string())
        })
    }

    /// Directories the shell must let the webview read images from: the library
    /// itself and its thumbnail cache. The cache is separate because it lives in
    /// a dot-directory, which path globs skip.
    pub fn image_dirs(&self) -> Result<Vec<String>> {
        self.with(|lib| {
            Ok(vec![
                lib.root().display().to_string(),
                lib.thumb_dir().display().to_string(),
            ])
        })
    }

    pub fn set_rating(&self, ids: Vec<i64>, rating: u8) -> Result<usize> {
        self.with(|lib| lib.set_rating_bulk(&ids, rating))
    }

    pub fn set_flag(&self, ids: Vec<i64>, flag: Flag) -> Result<usize> {
        self.with(|lib| lib.set_flag_bulk(&ids, flag))
    }

    pub fn set_color_label(&self, id: i64, label: Option<String>) -> Result<()> {
        self.with(|lib| lib.set_color_label(id, label.as_deref()))
    }

    // -------------------------------------------------------------- albums

    pub fn albums(&self) -> Result<Vec<AlbumSummary>> {
        self.with(|lib| {
            let mut out = Vec::new();
            for album in lib.albums()? {
                let photo_count = lib.album_photos(&album.path)?.len();
                out.push(AlbumSummary {
                    is_locked: album.is_locked(),
                    album,
                    photo_count,
                });
            }
            Ok(out)
        })
    }

    pub fn create_album(&self, new: NewAlbum) -> Result<Album> {
        self.with(|lib| lib.create_album(&new))
    }

    pub fn update_album(&self, path: String, update: AlbumUpdate) -> Result<Album> {
        self.with(|lib| lib.update_album(&path, &update))
    }

    pub fn move_album(&self, from: String, to: String) -> Result<Album> {
        self.with(|lib| lib.move_album(&from, &to))
    }

    pub fn delete_album(&self, path: String) -> Result<()> {
        self.with(|lib| lib.delete_album(&path))
    }

    pub fn album_photos(&self, path: String) -> Result<Vec<Photo>> {
        self.with(|lib| lib.album_photos(&path))
    }

    pub fn add_to_album(&self, path: String, photo_ids: Vec<i64>) -> Result<usize> {
        self.with(|lib| lib.add_photos_to_album(&path, &photo_ids))
    }

    pub fn remove_from_album(&self, path: String, photo_ids: Vec<i64>) -> Result<usize> {
        self.with(|lib| lib.remove_photos_from_album(&path, &photo_ids))
    }

    pub fn reorder_album(&self, path: String, photo_ids: Vec<i64>) -> Result<()> {
        self.with(|lib| lib.reorder_album(&path, &photo_ids))
    }

    /// Generate a fresh share link secret for an album.
    pub fn generate_share_link(&self, path: String) -> Result<String> {
        self.with(|lib| {
            let token = crate::albums::generate_share_token();
            lib.update_album(
                &path,
                &AlbumUpdate {
                    share_token: Some(Some(token.clone())),
                    ..Default::default()
                },
            )?;
            Ok(token)
        })
    }

    // ------------------------------------------------------------- publish

    pub fn publish_target(&self) -> Result<PublishTarget> {
        self.with(|lib| {
            Ok(PublishTarget {
                dest: lib.get_setting(SETTING_PUBLISH_DEST)?,
                min_rating: lib
                    .get_setting(SETTING_PUBLISH_MIN_RATING)?
                    .and_then(|v| v.parse().ok()),
            })
        })
    }

    pub fn set_publish_target(&self, target: PublishTarget) -> Result<()> {
        self.with(|lib| {
            if let Some(dest) = &target.dest {
                lib.set_setting(SETTING_PUBLISH_DEST, dest)?;
            }
            if let Some(min) = target.min_rating {
                lib.set_setting(SETTING_PUBLISH_MIN_RATING, &min.to_string())?;
            }
            Ok(())
        })
    }

    /// Publish one album, or every album when `album_path` is `None`.
    pub fn publish(&self, album_path: Option<String>) -> Result<Vec<PublishResult>> {
        let target = self.publish_target()?;
        let dest = target
            .dest
            .ok_or_else(|| Error::other("no publish destination configured"))?;
        let dest = PathBuf::from(dest);

        self.with(|lib| {
            let opts = PublishOptions {
                min_rating: target.min_rating,
                ..Default::default()
            };
            let targets: Vec<String> = match &album_path {
                Some(p) => vec![p.clone()],
                None => lib.albums()?.into_iter().map(|a| a.path).collect(),
            };
            let mut out = Vec::new();
            for path in targets {
                out.push(publish_album(lib, &path, &dest, &opts)?);
            }
            Ok(out)
        })
    }

    // -------------------------------------------------------------- remote

    /// Where the remote lives. A directory today (network share, external
    /// drive, or a folder another tool keeps in sync); SFTP/HTTP transports
    /// slot in behind the same trait later.
    pub fn remote_dir(&self) -> Result<Option<String>> {
        self.with(|lib| lib.get_setting(SETTING_REMOTE_DIR))
    }

    pub fn set_remote_dir(&self, dir: String) -> Result<()> {
        self.with(|lib| lib.set_setting(SETTING_REMOTE_DIR, &dir))
    }

    fn transport(&self) -> Result<crate::sync::FsTransport> {
        let dir = self
            .remote_dir()?
            .ok_or_else(|| Error::other("no remote configured"))?;
        Ok(crate::sync::FsTransport::new(dir))
    }

    fn published_root(&self) -> Result<PathBuf> {
        let target = self.publish_target()?;
        target.dest.map(PathBuf::from).ok_or_else(|| {
            // Sync goes through the published tree, so this error shows up in
            // the sync panel too — say where to fix it.
            Error::other(
                "no publish destination configured — choose the gallery content folder \
                 under Publish first",
            )
        })
    }

    /// Albums on the remote, annotated with what this machine knows about them.
    pub fn remote_albums(&self) -> Result<Vec<crate::remote::RemoteAlbum>> {
        let transport = self.transport()?;
        self.with(|lib| crate::remote::remote_albums(lib, &transport))
    }

    /// Which albums this machine syncs, and in which direction.
    pub fn album_subscriptions(&self) -> Result<Vec<crate::sync::AlbumSubscription>> {
        self.with(|lib| lib.album_subscriptions())
    }

    pub fn track_album(&self, album_path: String, direction: crate::sync::SyncDirection) -> Result<()> {
        self.with(|lib| lib.track_album(&album_path, direction))
    }

    pub fn untrack_album(&self, album_path: String) -> Result<()> {
        self.with(|lib| lib.untrack_album(&album_path))
    }

    /// Preview one album's sync without moving anything.
    pub fn plan_album_sync(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
    ) -> Result<crate::sync::SyncPlan> {
        let transport = self.transport()?;
        let root = self.published_root()?;
        self.with(|lib| crate::remote::plan_album_sync(lib, &transport, &album_path, direction, &root))
    }

    /// Adopt an album from the remote into this library.
    /// Adopt a path from the remote: the folders above it, the album or
    /// collection itself, and everything under it.
    pub fn pull_album(&self, album_path: String) -> Result<crate::remote::PullOutcome> {
        let transport = self.transport()?;
        let root = self.published_root()?;
        self.with(|lib| crate::remote::pull_path(lib, &transport, &album_path, &root))
    }

    /// Publish one album and upload it.
    /// Contribute a path to the remote: its folders, itself, everything under it.
    pub fn push_album(&self, album_path: String, allow_deletes: bool) -> Result<crate::remote::PushOutcome> {
        let transport = self.transport()?;
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        self.with(|lib| {
            crate::remote::push_path(lib, &transport, &album_path, &root, &opts, allow_deletes)
        })
    }

    /// Sync one album in the requested direction.
    pub fn sync_album(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
        allow_deletes: bool,
    ) -> Result<crate::sync::SyncOutcome> {
        let transport = self.transport()?;
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        self.with(|lib| {
            crate::remote::sync_path(
                lib, &transport, &album_path, direction, &root, &opts, allow_deletes,
            )
        })
    }

    /// Sync every subscribed album, each in its own direction.
    pub fn sync_all_tracked(&self, allow_deletes: bool) -> Result<Vec<(String, crate::sync::SyncOutcome)>> {
        let transport = self.transport()?;
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        self.with(|lib| {
            crate::remote::sync_tracked_albums(lib, &transport, &root, &opts, allow_deletes)
        })
    }

    fn publish_options(&self) -> Result<PublishOptions> {
        Ok(PublishOptions {
            min_rating: self.publish_target()?.min_rating,
            ..Default::default()
        })
    }

    /// What a sync would do, without doing it.
    pub fn sync_plan(&self) -> Result<crate::sync::SyncPlan> {
        let target = self.publish_target()?;
        let dest = target
            .dest
            .ok_or_else(|| Error::other("no publish destination configured"))?;
        let local = crate::publish::manifest_of(Path::new(&dest))?;
        self.with(|lib| {
            let synced = lib.synced_manifest()?;
            // Without a configured transport the server side is unknown; the
            // last agreement is the best available stand-in and keeps the plan
            // non-destructive.
            Ok(crate::sync::plan(&local, &synced, &synced))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_jpeg(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgb8(w, h)
            .save_with_format(path, image::ImageFormat::Jpeg)
            .unwrap();
    }

    #[test]
    fn refuses_work_before_a_library_is_open() {
        let s = Session::new();
        assert!(!s.is_open());
        assert!(s.status().is_err());
        assert!(s.photos(PhotoFilter::default()).is_err());
    }

    #[test]
    fn full_ui_flow_import_rate_album_publish() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 200, 150);
        write_jpeg(&src.path().join("b.jpg"), 200, 150);

        let s = Session::new();
        let status = s.open_library(src.path()).unwrap();
        assert_eq!(status.photo_count, 0);

        // Import
        let summary = s.import(None, None).unwrap();
        assert_eq!(summary.imported, 2);
        assert_eq!(s.status().unwrap().photo_count, 2);

        // Rate
        let photos = s.photos(PhotoFilter::default()).unwrap();
        s.set_rating(vec![photos[0].id], 5).unwrap();
        s.set_flag(vec![photos[1].id], Flag::Reject).unwrap();

        // Album
        s.create_album(NewAlbum {
            path: "2026/test".into(),
            title: Some("Test".into()),
            ..Default::default()
        })
        .unwrap();
        s.add_to_album("2026/test".into(), photos.iter().map(|p| p.id).collect())
            .unwrap();

        // The album, plus the `2026` folder created to hold it.
        let albums = s.albums().unwrap();
        assert_eq!(albums.len(), 2);
        let album = albums.iter().find(|a| a.album.path == "2026/test").unwrap();
        assert_eq!(album.photo_count, 2);
        assert!(!album.is_locked);

        // Share link makes it locked
        let token = s.generate_share_link("2026/test".into()).unwrap();
        assert!(!token.is_empty());
        assert!(s.albums().unwrap().iter().any(|a| a.album.path == "2026/test" && a.is_locked));

        // Publish, filtered to 4★+ — the rejected photo must not ship
        s.set_publish_target(PublishTarget {
            dest: Some(dest.path().display().to_string()),
            min_rating: Some(4),
        })
        .unwrap();
        let results = s.publish(Some("2026/test".into())).unwrap();
        assert_eq!(results[0].photos_copied, 1);
        assert!(dest.path().join("2026/test/index.md").exists());

        let index = std::fs::read_to_string(dest.path().join("2026/test/index.md")).unwrap();
        assert!(index.contains(&format!("shareToken: \"{token}\"")));
    }

    #[test]
    fn publish_requires_a_destination() {
        let src = tempfile::tempdir().unwrap();
        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.create_album(NewAlbum { path: "a".into(), ..Default::default() }).unwrap();
        assert!(s.publish(Some("a".into())).is_err());
    }

    #[test]
    fn publish_target_persists_in_the_catalog() {
        let src = tempfile::tempdir().unwrap();
        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.set_publish_target(PublishTarget {
            dest: Some("/tmp/gallery".into()),
            min_rating: Some(3),
        })
        .unwrap();

        // Re-opening the same library restores it.
        let s2 = Session::new();
        s2.open_library(src.path()).unwrap();
        let t = s2.publish_target().unwrap();
        assert_eq!(t.dest.as_deref(), Some("/tmp/gallery"));
        assert_eq!(t.min_rating, Some(3));
    }

    #[test]
    fn thumbnail_and_photo_paths_resolve() {
        let src = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 100, 100);

        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.import(None, None).unwrap();
        let id = s.photos(PhotoFilter::default()).unwrap()[0].id;

        assert!(Path::new(&s.photo_path(id).unwrap()).exists());
        assert!(Path::new(&s.thumbnail_path(id, "small").unwrap()).exists());
    }

    #[test]
    fn sync_plan_from_a_partial_library_is_never_destructive() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.set_publish_target(PublishTarget {
            dest: Some(dest.path().display().to_string()),
            min_rating: None,
        })
        .unwrap();

        let plan = s.sync_plan().unwrap();
        assert!(!plan.is_destructive());
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    use crate::sync::SyncDirection;

    fn write_jpeg(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgb8(w, h)
            .save_with_format(path, image::ImageFormat::Jpeg)
            .unwrap();
    }

    /// A session wired to its own library, published tree and shared remote.
    fn session_with(remote: &Path) -> (Session, tempfile::TempDir, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let published = tempfile::tempdir().unwrap();
        let s = Session::new();
        s.open_library(root.path()).unwrap();
        s.set_publish_target(PublishTarget {
            dest: Some(published.path().display().to_string()),
            min_rating: None,
        })
        .unwrap();
        s.set_remote_dir(remote.display().to_string()).unwrap();
        (s, root, published)
    }

    #[test]
    fn remote_operations_require_configuration() {
        let root = tempfile::tempdir().unwrap();
        let s = Session::new();
        s.open_library(root.path()).unwrap();
        // No remote set yet.
        assert!(s.remote_albums().is_err());
        assert!(s.pull_album("a".into()).is_err());
    }

    #[test]
    fn session_drives_a_full_two_machine_exchange() {
        let remote = tempfile::tempdir().unwrap();

        // --- Machine A authors and pushes ---------------------------------
        let (a, a_root, _a_pub) = session_with(remote.path());
        write_jpeg(&a_root.path().join("2026/x/one.jpg"), 90, 60);
        a.import(None, None).unwrap();
        a.create_album(NewAlbum { path: "2026/x".into(), title: Some("Ex".into()), ..Default::default() })
            .unwrap();
        let ids: Vec<i64> = a.photos(PhotoFilter::default()).unwrap().iter().map(|p| p.id).collect();
        a.add_to_album("2026/x".into(), ids).unwrap();
        a.push_album("2026/x".into(), false).unwrap();

        // A is now subscribed to the album it pushed.
        let subs = a.album_subscriptions().unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].direction, SyncDirection::Push);

        // --- Machine B discovers and pulls --------------------------------
        let (b, _b_root, _b_pub) = session_with(remote.path());
        let found = b.remote_albums().unwrap();
        let ex = found.iter().find(|a| a.path == "2026/x").unwrap();
        assert!(!ex.local);
        assert_eq!(ex.title.as_deref(), Some("Ex"));

        let pulled = b.pull_album("2026/x".into()).unwrap();
        assert!(pulled.files_pulled >= 2);
        // The album plus the `2026` folder it needs to be reachable.
        assert_eq!(b.albums().unwrap().len(), 2);
        assert_eq!(b.album_photos("2026/x".into()).unwrap().len(), 1);

        // After pulling, B tracks it bidirectionally and sees it as local.
        let after = b.remote_albums().unwrap();
        let ex = after.iter().find(|a| a.path == "2026/x").unwrap();
        assert!(ex.local);
        assert_eq!(ex.tracked, Some(SyncDirection::Both));
    }

    #[test]
    fn tracking_can_be_changed_and_removed() {
        let remote = tempfile::tempdir().unwrap();
        let (s, _root, _pub) = session_with(remote.path());

        s.track_album("a".into(), SyncDirection::Pull).unwrap();
        assert_eq!(s.album_subscriptions().unwrap()[0].direction, SyncDirection::Pull);

        s.track_album("a".into(), SyncDirection::Push).unwrap();
        assert_eq!(s.album_subscriptions().unwrap()[0].direction, SyncDirection::Push);

        s.untrack_album("a".into()).unwrap();
        assert!(s.album_subscriptions().unwrap().is_empty());
    }

    /// A chosen direction is the user's, not something an operation rewrites:
    /// a pull-only machine must stay pull-only after it pulls.
    #[test]
    fn a_one_off_operation_never_overrules_the_chosen_direction() {
        let remote = tempfile::tempdir().unwrap();

        let (a, a_root, _a_pub) = session_with(remote.path());
        write_jpeg(&a_root.path().join("2026/x/one.jpg"), 90, 60);
        a.import(None, None).unwrap();
        a.create_album(NewAlbum { path: "2026/x".into(), ..Default::default() }).unwrap();
        let ids: Vec<i64> = a.photos(PhotoFilter::default()).unwrap().iter().map(|p| p.id).collect();
        a.add_to_album("2026/x".into(), ids).unwrap();

        // A tracks it both ways, then does a one-off push.
        a.track_album("2026/x".into(), SyncDirection::Both).unwrap();
        a.push_album("2026/x".into(), false).unwrap();
        assert_eq!(
            a.album_subscriptions().unwrap()[0].direction,
            SyncDirection::Both,
            "a push must not downgrade a both-ways album"
        );

        // B declares itself read-only, then pulls.
        let (b, _b_root, _b_pub) = session_with(remote.path());
        b.track_album("2026/x".into(), SyncDirection::Pull).unwrap();
        b.pull_album("2026/x".into()).unwrap();
        let subs = b.album_subscriptions().unwrap();
        assert_eq!(
            subs.iter().find(|s| s.album_path == "2026/x").map(|s| s.direction),
            Some(SyncDirection::Pull),
            "a pull must not turn a read-only machine into one that pushes"
        );
        // The `2026` folder came along for navigation, but was not subscribed:
        // a subscription there would drag in every album of the year.
        assert!(subs.iter().all(|s| s.album_path != "2026"));
    }

    /// A local album the server has never seen must still be offered, or
    /// "add an album from anywhere" would mean typing paths by hand.
    #[test]
    fn local_only_albums_are_listed_as_pushable() {
        let remote = tempfile::tempdir().unwrap();
        let (s, root, _pub) = session_with(remote.path());

        write_jpeg(&root.path().join("2026/new/one.jpg"), 60, 40);
        s.import(None, None).unwrap();
        s.create_album(NewAlbum {
            path: "2026/new".into(),
            title: Some("Fresh".into()),
            ..Default::default()
        })
        .unwrap();

        let listed = s.remote_albums().unwrap();
        // The album and the folder above it — the folder is part of the path,
        // so it is offered too.
        assert_eq!(
            listed.iter().map(|a| a.path.as_str()).collect::<Vec<_>>(),
            vec!["2026", "2026/new"]
        );
        let new = listed.iter().find(|a| a.path == "2026/new").unwrap();
        assert!(new.local, "it is in this catalog");
        assert!(!new.remote, "the server has never seen it");
        assert_eq!(new.file_count, 0);
        assert!(listed[0].is_collection, "2026 is a folder");
        // Listing is read-only: nothing was tracked or uploaded by looking.
        assert!(s.album_subscriptions().unwrap().is_empty());
        assert!(std::fs::read_dir(remote.path()).unwrap().next().is_none());
    }
}
