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

        let albums = s.albums().unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].photo_count, 2);
        assert!(!albums[0].is_locked);

        // Share link makes it locked
        let token = s.generate_share_link("2026/test".into()).unwrap();
        assert!(!token.is_empty());
        assert!(s.albums().unwrap()[0].is_locked);

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
