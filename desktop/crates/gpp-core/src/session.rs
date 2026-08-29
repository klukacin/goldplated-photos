//! Application session — the operations a UI actually performs.
//!
//! Every desktop/mobile command maps to exactly one method here. The Tauri
//! layer is a one-line wrapper per command, which keeps the untestable part of
//! the stack (the webview shell) trivial and puts all real behaviour under
//! test on any platform.
//!
//! Three clients wrap this same surface: the Tauri shell, `gpp-cli`, and the C
//! ABI in `gpp-ffi` that a native iPad or Android front end would call. So a
//! method here is not "a Rust function some GUI happens to use" — it is the
//! whole of what that platform can do, and anything it cannot express is a
//! thing no client will ever be able to do. Logic that drifts into a shell is
//! logic the other two clients silently lack.
//!
//! Two consequences worth knowing before adding a method. Arguments and returns
//! cross a JSON boundary, so they are owned, serde-derived types rather than
//! borrowed views — and a field name serde does not recognise is dropped in
//! silence, which is how a filter once filtered nothing. And ids, not paths,
//! identify photos: a path stops naming the same frame the moment a file moves.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::albums::{AlbumUpdate, NewAlbum};
use crate::catalog::Library;
use crate::error::{Error, Result};
use crate::import::{import_dir, ImportOptions};
use crate::model::{Album, Flag, ImportSummary, Photo, PhotoFilter};
use crate::publish::{publish_album_for, PublishOptions, PublishResult};
use crate::remotes::{PublishTargetInfo, PublishTargetUpdate, RemoteInfo, RemoteUpdate};
use crate::sync::SyncScopeKind;

/// Holds the currently open library. `None` until the user picks one.
#[derive(Default)]
pub struct Session {
    library: RwLock<Option<Library>>,
    /// Raised by [`Session::cancel_import`], polled by the running import.
    ///
    /// An atomic and not a channel: the import reads it from every rayon
    /// worker, and it sits outside the library lock so the UI thread can raise
    /// it while the import it is trying to stop holds that lock.
    import_cancel: AtomicBool,
}

/// What the UI shows in the sidebar for one album.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumSummary {
    /// Flattened, so the shell reads `path`, `title` and the rest at the top
    /// level of the JSON instead of nested under `album` — the sidebar and the
    /// album editor bind to one object.
    #[serde(flatten)]
    pub album: Album,
    /// Members of *this* album only. A collection reads zero however many
    /// frames sit in the albums beneath it, which is the honest number: a
    /// collection publishes an `index.md` and no photos.
    pub photo_count: usize,
    /// Password and/or share token — the padlock in the sidebar, decided by the
    /// same rule the web gallery applies when it chooses whether to ask.
    pub is_locked: bool,
}

/// Library-level status for the header bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryStatus {
    /// Absolute path of the library root, exactly as the user picked it.
    pub root: String,
    /// Rows in the catalog, not files on the disk. A folder copied in beside
    /// the others counts for nothing until an import walks it, and a row whose
    /// file has since been deleted keeps counting until [`Session::prune`].
    pub photo_count: i64,
    pub album_count: usize,
    /// Distinct camera models in the catalog. The filter bar's dropdown is
    /// built from this, so a body disappears from it when its last frame does.
    pub cameras: Vec<String>,
}

/// Settings the UI persists between launches.
///
/// Kept in the catalog rather than in the app's own preferences, so they belong
/// to the library: carry the drive to another machine and the destination comes
/// with it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublishTarget {
    /// Absolute path of the gallery's `src/content/albums`. `None` means no
    /// destination has been chosen yet, and both publish and sync refuse to run
    /// — sync moves the published tree, so it needs one too.
    pub dest: Option<String>,
    /// Stars a photo needs before it ships. `None` is "publish everything", a
    /// choice the panel actually offers rather than a missing value — see
    /// [`Session::set_publish_target`] for why that distinction cost a shoot.
    pub min_rating: Option<u8>,
}

impl Session {
    /// A session with nothing open. Every method below fails with "no library
    /// is open" until [`Session::open_library`] succeeds — which is the state
    /// the shell's folder picker exists to get out of.
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

    /// Whether a library is open — what the shell asks on start-up to choose
    /// between the picker and the grid.
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

    /// Re-read the header counts. Three queries, so the shell calls it after
    /// anything that could move them rather than trying to track deltas of its
    /// own and drifting out of step with the catalog.
    pub fn status(&self) -> Result<LibraryStatus> {
        self.with(Self::status_of)
    }

    // ------------------------------------------------------------- import

    /// Import a folder, or every registered source when `dir` is `None`.
    ///
    /// A folder inside a source is catalogued where it lies; one outside every
    /// source is copied into the library first. With no folder named, this
    /// walks the primary and every referenced source in turn — skipping, with a
    /// note in [`ImportSummary::notes`], any whose drive is not attached.
    ///
    /// Stoppable: [`Session::cancel_import`] ends the run at the next file and
    /// sets `cancelled` on the summary.
    pub fn import(
        &self,
        dir: Option<String>,
        on_progress: Option<&(dyn Fn(crate::model::ImportProgress) + Sync)>,
    ) -> Result<ImportSummary> {
        // Clear a cancel left over from the last run — including one raised
        // after that run had already finished, which would otherwise stop this
        // import before it read a single file.
        self.import_cancel.store(false, Ordering::SeqCst);

        self.with(|lib| {
            match &dir {
                Some(d) => import_dir(
                    lib,
                    &PathBuf::from(d),
                    &ImportOptions::default(),
                    on_progress,
                    Some(&|| self.import_cancel.load(Ordering::SeqCst)),
                ),
                None => crate::import::import_all_sources(
                    lib,
                    &ImportOptions::default(),
                    on_progress,
                    Some(&|| self.import_cancel.load(Ordering::SeqCst)),
                ),
            }
        })
    }

    // ------------------------------------------------------------- sources

    /// Every root photographs may live under: the library itself, plus any
    /// folder registered with [`add_source`](Self::add_source).
    ///
    /// `online` is probed at the moment of the call — that is the whole
    /// question a drive that comes and goes asks — and `photo_count` is what
    /// the catalog holds on each, so a UI can say "1 842 photographs on
    /// Archive 2019, not available" rather than showing an empty grid.
    pub fn sources(&self) -> Result<Vec<crate::sources::SourceInfo>> {
        self.with(|lib| lib.sources())
    }

    /// Register a folder as a source: photographs there are catalogued where
    /// they lie, never copied and never moved.
    ///
    /// This is what makes a Lightroom library importable without migrating a
    /// terabyte. Nothing is catalogued by registering — run an import on the
    /// folder afterwards, and it will be taken in place.
    ///
    /// Refused when the folder overlaps a source that already exists, in either
    /// direction: two roots over one file give it two identities, and then a
    /// prune, a publish and a sync each disagree about what the library holds.
    pub fn add_source(
        &self,
        path: String,
        name: Option<String>,
        kind: Option<crate::sources::SourceKind>,
    ) -> Result<i64> {
        self.with(|lib| lib.add_source(Path::new(&path), name.as_deref(), kind))
    }

    /// Forget a source. **Never deletes a file.**
    ///
    /// `drop_photos` has to be answered: with `false`, a source that still
    /// holds catalog rows is refused rather than silently taking their ratings,
    /// flags, album memberships and adjustments with it. Returns how many rows
    /// were dropped.
    pub fn remove_source(&self, id: i64, drop_photos: bool) -> Result<usize> {
        self.with(|lib| lib.remove_source(id, drop_photos))
    }

    /// Point a source at the folder it lives in now — a drive that mounted
    /// somewhere else, a share that moved.
    ///
    /// Validated by re-hashing a handful of that source's own catalogued files
    /// at the new path: a folder that does not hold them is refused by name,
    /// and nothing changes. Pointing a source at last year's backup would
    /// otherwise re-attach every row to the wrong negatives, quietly.
    pub fn relocate_source(&self, id: i64, new_path: String) -> Result<()> {
        self.with(|lib| lib.relocate_source(id, Path::new(&new_path)))
    }

    /// Ask the running import to stop. Harmless when none is running: the next
    /// [`Session::import`] clears the flag before it starts.
    pub fn cancel_import(&self) {
        self.import_cancel.store(true, Ordering::SeqCst);
    }

    /// Drop catalog rows whose files are gone.
    pub fn prune(&self) -> Result<usize> {
        self.with(|lib| lib.prune_missing())
    }

    // ---------------------------------------------------------- lightroom

    /// Look inside a Lightroom Classic catalog without changing anything:
    /// root folders (and whether each could import in place), collections,
    /// keywords, missing files. The `.lrcat` itself is only ever copied and
    /// read — never opened in place, never written.
    pub fn lr_scan(&self, lrcat_path: String) -> Result<crate::lightroom::LrScanReport> {
        self.with(|lib| crate::lightroom::scan(lib, Path::new(&lrcat_path)))
    }

    /// Import a Lightroom Classic catalog: photos copied in (or catalogued in
    /// place when they already live under the library root), collections
    /// mapped to albums, keywords to tags. Idempotent — re-running with the
    /// same catalog syncs rather than duplicates — and stoppable the same way
    /// an ordinary import is, via [`Session::cancel_import`].
    pub fn lr_import(
        &self,
        lrcat_path: String,
        options: crate::lightroom::LrImportOptions,
        on_progress: Option<&(dyn Fn(crate::model::ImportProgress) + Sync)>,
    ) -> Result<crate::lightroom::LrImportReport> {
        // Same latch as `import`: clear a cancel left over from the last run.
        self.import_cancel.store(false, Ordering::SeqCst);
        self.with(|lib| {
            crate::lightroom::lr_import(
                lib,
                Path::new(&lrcat_path),
                &options,
                on_progress,
                Some(&|| self.import_cancel.load(Ordering::SeqCst)),
            )
        })
    }

    // -------------------------------------------------------------- photos

    /// The grid's contents: everything matching the filter, ordered by the
    /// catalog. Filtering is done here and not in the client, so the shell and
    /// a future iPad app agree about what "3 stars and up, Leica only" means.
    pub fn photos(&self, filter: PhotoFilter) -> Result<Vec<Photo>> {
        self.with(|lib| lib.photos(&filter))
    }

    /// One photo, by catalog id — the identifier every other method takes, and
    /// the only one that survives a file being renamed or moved.
    pub fn photo(&self, id: i64) -> Result<Photo> {
        self.with(|lib| lib.photo_by_id(id))
    }

    /// Absolute path of a photo — the shell turns this into an asset URL.
    ///
    /// Resolved through the photo's source, so a referenced frame answers with
    /// its own drive's path. A source that is not attached fails with
    /// [`Error::SourceOffline`] naming it, rather than handing back a path that
    /// is not there for a reason nobody can see.
    pub fn photo_path(&self, id: i64) -> Result<String> {
        self.with(|lib| {
            let photo = lib.photo_by_id(id)?;
            Ok(lib.photo_path(&photo)?.display().to_string())
        })
    }

    /// Absolute path of a cached thumbnail, generating nothing — the import
    /// already produced these.
    pub fn thumbnail_path(&self, id: i64, size: &str) -> Result<String> {
        self.with(|lib| {
            let photo = lib.photo_by_id(id)?;
            let key = crate::develop::render_key(&photo.content_hash, &lib.edits(photo.id)?);
            let p = crate::media::thumb_path(&lib.thumb_dir(), &key, size);
            Ok(p.display().to_string())
        })
    }

    /// Directories the shell must let the webview read images from: every
    /// source's root and the thumbnail cache. The cache is separate because it
    /// lives in a dot-directory, which path globs skip.
    ///
    /// Referenced sources are in the list because their originals are real
    /// files the lightbox opens at full size; leaving one out shows a
    /// photographer a blank frame with no error anywhere.
    pub fn image_dirs(&self) -> Result<Vec<String>> {
        self.with(|lib| {
            let mut dirs: Vec<String> =
                lib.sources()?.into_iter().map(|s| s.path).collect();
            dirs.push(lib.thumb_dir().display().to_string());
            Ok(dirs)
        })
    }

    /// Stars for a whole selection; `0` is unrated, anything above five is
    /// clamped. Returns how many photos it reached, which is what the UI
    /// reports rather than assuming every id took — an id whose row has since
    /// gone is not counted, and over a hundred-frame cull that gap is otherwise
    /// invisible.
    pub fn set_rating(&self, ids: Vec<i64>, rating: u8) -> Result<usize> {
        self.with(|lib| lib.set_rating_bulk(&ids, rating))
    }

    /// Pick or reject a selection. A reject is a mark on a row, never a
    /// deletion: the negative stays on disk, and publishing is what leaves it
    /// out (`PublishOptions::exclude_rejected`, on by default).
    pub fn set_flag(&self, ids: Vec<i64>, flag: Flag) -> Result<usize> {
        self.with(|lib| lib.set_flag_bulk(&ids, flag))
    }

    /// One photo's colour label, `None` to clear it. Single-photo on purpose:
    /// a label marks one frame out of a run, so there is no bulk form to
    /// mis-click.
    pub fn set_color_label(&self, id: i64, label: Option<String>) -> Result<()> {
        self.with(|lib| lib.set_color_label(id, label.as_deref()))
    }

    // ------------------------------------------------------------- develop

    /// The adjustments on one photo. Never absent — an untouched photo has an
    /// empty stack, so a develop panel has one shape to render.
    pub fn photo_edits(&self, id: i64) -> Result<crate::develop::EditStack> {
        self.with(|lib| lib.edits(id))
    }

    /// Set one adjustment on every selected photo.
    ///
    /// Bulk because that is how the panel is used: pick twenty frames from the
    /// same light and pull them all down half a stop. Returns how many photos
    /// changed.
    pub fn set_photo_edit(
        &self,
        ids: Vec<i64>,
        op: crate::develop::EditOp,
    ) -> Result<usize> {
        self.edit_each(ids, |stack| stack.set(op.clone()))
    }

    /// Turn each selected photo a further quarter — positive clockwise.
    ///
    /// Relative, because the button is: a selection can hold photos at
    /// different angles, and "rotate right" has to mean the same thing to each
    /// of them.
    pub fn rotate_photos(&self, ids: Vec<i64>, quarter_turns: i32) -> Result<usize> {
        self.edit_each(ids, |stack| stack.rotate_by(quarter_turns))
    }

    /// Switch a valueless adjustment — a flip — on, or off again, per photo.
    pub fn toggle_photo_edit(&self, ids: Vec<i64>, op: crate::develop::EditOp) -> Result<usize> {
        self.edit_each(ids, |stack| stack.toggle(op.clone()))
    }

    /// Drop one adjustment, leaving the rest.
    pub fn clear_photo_edit(&self, ids: Vec<i64>, kind: String) -> Result<usize> {
        self.edit_each(ids, |stack| stack.remove(&kind))
    }

    /// Back to the original, for every selected photo.
    pub fn reset_photo_edits(&self, ids: Vec<i64>) -> Result<usize> {
        self.edit_each(ids, |stack| *stack = crate::develop::EditStack::new())
    }

    /// Apply a change to each photo's stack, then rebuild what it derives.
    ///
    /// One photo failing to render — an unplugged drive, a corrupt file — must
    /// not abandon the rest of the selection, so the stack is saved first and
    /// rendering failures are skipped.
    fn edit_each(
        &self,
        ids: Vec<i64>,
        change: impl Fn(&mut crate::develop::EditStack),
    ) -> Result<usize> {
        self.with(|lib| {
            // Record every stack first — that is the part the catalog has to be
            // consistent about, and it is cheap. Rendering is the expensive
            // part, so it happens afterwards and in parallel: a bulk edit over
            // a selected shoot is otherwise one CPU core doing hundreds of
            // full-size renders in a row while the app looks frozen.
            let mut pending = Vec::new();
            for id in ids {
                let photo = lib.photo_by_id(id)?;
                let mut stack = lib.edits(id)?;
                let before = stack.clone();
                change(&mut stack);
                if stack == before {
                    continue;
                }
                lib.set_edits(id, &stack)?;
                pending.push((photo, stack));
            }

            use rayon::prelude::*;
            pending.par_iter().for_each(|(photo, stack)| {
                // A render that fails leaves the edit recorded and the old
                // thumbnail in place; the next request renders it again.
                let _ = crate::develop::render_derived(lib, photo, stack);
            });

            Ok(pending.len())
        })
    }

    // -------------------------------------------------------------- albums

    /// Every album, flat and already sorted so a parent precedes its children.
    /// The sidebar indents from `path` rather than the catalog handing back a
    /// nested shape — one list is far easier to keep in step across the wire
    /// than a tree the client has to rebuild after every edit.
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

    /// Create an album at a gallery path, and any folders above it that do not
    /// exist yet — the gallery cannot navigate to an album whose parents are
    /// missing, so asking for `2026/weddings/ana-ivan` in an empty library
    /// yields three rows, not one.
    pub fn create_album(&self, new: NewAlbum) -> Result<Album> {
        self.with(|lib| lib.create_album(&new))
    }

    /// Change album settings. Every field of the update is optional and an
    /// absent one is left alone, so a panel that edits the title cannot blank
    /// the password it never showed.
    pub fn update_album(&self, path: String, update: AlbumUpdate) -> Result<Album> {
        self.with(|lib| lib.update_album(&path, &update))
    }

    /// Rename or move an album, carrying its sub-albums and every sync
    /// subscription among them.
    ///
    /// Nothing moves on disk and nothing moves on the server. Photos are
    /// members, not contents, so they stay where they were imported; and a
    /// tracked album that was already pushed keeps its old copy up there while
    /// the next sync publishes the new path, so the server holds both until
    /// someone removes one deliberately with `allow_deletes`.
    pub fn move_album(&self, from: String, to: String) -> Result<Album> {
        self.with(|lib| lib.move_album(&from, &to))
    }

    /// Delete the album and its subscription. Not the photos — an album is a
    /// view over them, and every frame it held is still in the library
    /// afterwards. The published copy and the server's copy also survive;
    /// removing those is a sync with deletions allowed.
    pub fn delete_album(&self, path: String) -> Result<()> {
        self.with(|lib| lib.delete_album(&path))
    }

    /// The album's members, in the order the gallery will show them.
    pub fn album_photos(&self, path: String) -> Result<Vec<Photo>> {
        self.with(|lib| lib.album_photos(&path))
    }

    /// Add photos to an album, appended in the order given. A photo may belong
    /// to several albums at once; a collection refuses them, because the
    /// gallery renders one as a grid of sub-albums and would never draw them.
    pub fn add_to_album(&self, path: String, photo_ids: Vec<i64>) -> Result<usize> {
        self.with(|lib| lib.add_photos_to_album(&path, &photo_ids))
    }

    /// Drop photos from an album. A membership change and nothing more: the
    /// files stay in the library and in every other album that holds them.
    pub fn remove_from_album(&self, path: String, photo_ids: Vec<i64>) -> Result<usize> {
        self.with(|lib| lib.remove_photos_from_album(&path, &photo_ids))
    }

    /// Set the album's running order — what the drag-and-drop grid saves, and
    /// what publishing writes out as `photoOrder` for the gallery to obey.
    /// Members left out of the list follow the listed ones, keeping the order
    /// they already had.
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

    /// Read back where publishing writes, and how much of the album it ships.
    ///
    /// A thin compatibility view over the **default** publish target row
    /// (schema v5) — the shape every existing caller binds to.
    pub fn publish_target(&self) -> Result<PublishTarget> {
        self.with(|lib| {
            Ok(match lib.default_target_id()? {
                Some(id) => {
                    let t = lib
                        .publish_target_by_id(id)?
                        .ok_or_else(|| Error::other("the default publish target vanished"))?;
                    PublishTarget {
                        dest: Some(t.dest_root).filter(|d| !d.is_empty()),
                        min_rating: t.min_rating,
                    }
                }
                None => PublishTarget::default(),
            })
        })
    }

    /// Save both settings, onto the default publish target row. The two
    /// halves are treated differently on purpose: an absent destination
    /// leaves the stored one alone, while an absent minimum rating really
    /// does clear it — "no minimum" is one of the choices the panel offers.
    pub fn set_publish_target(&self, target: PublishTarget) -> Result<()> {
        self.with(|lib| {
            let id = lib.ensure_default_target()?;
            lib.update_publish_target(
                id,
                &PublishTargetUpdate {
                    dest_root: target.dest.clone(),
                    min_rating: Some(target.min_rating),
                    ..Default::default()
                },
            )
        })
    }

    /// Every publish target this library knows.
    pub fn publish_targets(&self) -> Result<Vec<PublishTargetInfo>> {
        self.with(|lib| lib.publish_targets())
    }

    /// Add a publish target; returns its id. The first one becomes default.
    pub fn add_publish_target(
        &self,
        name: String,
        dest_root: String,
        min_rating: Option<u8>,
    ) -> Result<i64> {
        self.with(|lib| lib.add_publish_target(&name, &dest_root, min_rating))
    }

    /// Change a publish target; absent fields are left alone.
    pub fn update_publish_target(&self, id: i64, update: PublishTargetUpdate) -> Result<()> {
        self.with(|lib| lib.update_publish_target(id, &update))
    }

    /// Remove a publish target and its publish records. The files in its
    /// tree stay where they are.
    pub fn remove_publish_target(&self, id: i64) -> Result<()> {
        self.with(|lib| lib.remove_publish_target(id))
    }

    /// Choose which publish target the un-suffixed calls act on.
    pub fn set_default_publish_target(&self, id: i64) -> Result<()> {
        self.with(|lib| lib.set_default_publish_target(id))
    }

    /// Publish one album, or every album when `album_path` is `None`, to the
    /// default publish target.
    pub fn publish(&self, album_path: Option<String>) -> Result<Vec<PublishResult>> {
        self.publish_on(album_path, None)
    }

    /// [`Self::publish`], to a chosen publish target (`None` = default).
    pub fn publish_on(
        &self,
        album_path: Option<String>,
        target_id: Option<i64>,
    ) -> Result<Vec<PublishResult>> {
        let info = self.resolve_publish_target(target_id)?;
        let dest = PathBuf::from(&info.dest_root);

        self.with(|lib| {
            let opts = PublishOptions {
                min_rating: info.min_rating,
                ..Default::default()
            };
            let targets: Vec<String> = match &album_path {
                Some(p) => vec![p.clone()],
                None => lib.albums()?.into_iter().map(|a| a.path).collect(),
            };
            let mut out = Vec::new();
            for path in targets {
                out.push(publish_album_for(lib, info.id, &path, &dest, &opts)?);
            }
            Ok(out)
        })
    }

    /// One publish target row, default when `None`, refusing an unconfigured
    /// or empty destination with the message callers have always seen.
    fn resolve_publish_target(&self, target_id: Option<i64>) -> Result<PublishTargetInfo> {
        self.with(|lib| {
            let id = match target_id {
                Some(id) => id,
                None => lib
                    .default_target_id()?
                    .ok_or_else(|| Error::other("no publish destination configured"))?,
            };
            let info = lib
                .publish_target_by_id(id)?
                .ok_or_else(|| Error::other(format!("no publish target with id {id}")))?;
            if info.dest_root.is_empty() {
                return Err(Error::other("no publish destination configured"));
            }
            Ok(info)
        })
    }

    // -------------------------------------------------------------- remote

    /// Where the **default** remote lives — a folder path, or the `http(s)`
    /// URL of a gallery's sync API. A thin compatibility view over remotes
    /// row #1 (schema v5); `None` while nothing is configured.
    pub fn remote_dir(&self) -> Result<Option<String>> {
        self.with(|lib| {
            Ok(match lib.default_remote_id()? {
                Some(id) => lib
                    .remote_by_id(id)?
                    .map(|r| r.target)
                    .filter(|t| !t.is_empty()),
                None => None,
            })
        })
    }

    /// Point the default remote at a target: a folder path, or the `http(s)`
    /// URL of a gallery's sync API. One text box in the UI — which kind it is
    /// gets read off the string, so there is no type to pick and no way to
    /// pick it wrong.
    pub fn set_remote_dir(&self, dir: String) -> Result<()> {
        self.with(|lib| {
            let id = lib.ensure_default_remote()?;
            lib.update_remote(id, &RemoteUpdate { target: Some(dir.clone()), ..Default::default() })
        })
    }

    /// Every remote this library knows.
    pub fn remotes(&self) -> Result<Vec<RemoteInfo>> {
        self.with(|lib| lib.remotes())
    }

    /// Add a remote; returns its id. The first one added becomes the default.
    pub fn add_remote(&self, name: String, target: String, token: Option<String>) -> Result<i64> {
        self.with(|lib| lib.add_remote(&name, &target, token.as_deref()))
    }

    /// Change a remote; absent fields are left alone, `token: Some(None)`
    /// clears the stored secret.
    pub fn update_remote(&self, id: i64, update: RemoteUpdate) -> Result<()> {
        self.with(|lib| lib.update_remote(id, &update))
    }

    /// Remove a remote. Cascades this library's subscriptions and baselines
    /// for it — **locally**. The server it pointed at is never touched.
    pub fn remove_remote(&self, id: i64) -> Result<()> {
        self.with(|lib| lib.remove_remote(id))
    }

    /// Choose which remote the un-suffixed calls act on.
    pub fn set_default_remote(&self, id: i64) -> Result<()> {
        self.with(|lib| lib.set_default_remote(id))
    }

    /// The remotes of a library that is **not** open here, read from its own
    /// catalog without opening a session on it (and without migrating it).
    pub fn read_library_remotes(&self, library_root: String) -> Result<Vec<RemoteInfo>> {
        crate::remotes::read_library_remotes(library_root)
    }

    /// One remote row plus its transport — default when `None`, with the
    /// errors callers have always seen for an unconfigured one.
    fn transport_on(
        &self,
        remote_id: Option<i64>,
    ) -> Result<(i64, Box<dyn crate::sync::RemoteTransport>)> {
        // `remote_by_id` never answers with an unconfigured anchor row (see
        // `Library::remotes`), so a library that has never been pointed
        // anywhere reaches "no remote configured" whether it carries an anchor
        // or no rows at all — rather than naming an id the user never chose.
        let info = self.with(|lib| match remote_id {
            Some(id) => lib
                .remote_by_id(id)?
                .ok_or_else(|| Error::other(format!("no remote with id {id}"))),
            None => lib
                .default_remote_id()?
                .and_then(|id| lib.remote_by_id(id).transpose())
                .transpose()?
                .ok_or_else(|| Error::other("no remote configured")),
        })?;
        Ok((info.id, Self::build_transport(&info.target, info.token)?))
    }

    /// A transport for a bare target/token pair — what a cross-library push
    /// uses, since a foreign remote has no row in this catalog.
    ///
    /// Chosen by what the target looks like rather than by a type field, so
    /// the UI keeps one text box: a path is a directory, an `http(s)` URL is
    /// the sync API. Adding SFTP later is another arm here and no UI change.
    fn build_transport(
        target: &str,
        token: Option<String>,
    ) -> Result<Box<dyn crate::sync::RemoteTransport>> {
        if target.starts_with("http://") || target.starts_with("https://") {
            let token = token.filter(|t| !t.is_empty()).ok_or_else(|| {
                Error::other(
                    "this remote needs an access token — set it in the sync panel, \
                     or with `gpp sync --remote-token <token>`",
                )
            })?;
            return Ok(Box::new(crate::sync::HttpTransport::new(target, token)));
        }
        Ok(Box::new(crate::sync::FsTransport::new(target)))
    }

    /// Shared secret of the default remote. Stored in the catalog beside the
    /// URL.
    pub fn remote_token(&self) -> Result<Option<String>> {
        self.with(|lib| {
            Ok(match lib.default_remote_id()? {
                Some(id) => lib.remote_by_id(id)?.and_then(|r| r.token),
                None => None,
            })
        })
    }

    /// Store the secret an HTTP remote requires, on the default remote. It
    /// has to equal the gallery's `SYNC_TOKEN`; a gallery with none configured
    /// answers 503 and stays shut rather than open, so a blank on either side
    /// never quietly works.
    pub fn set_remote_token(&self, token: String) -> Result<()> {
        self.with(|lib| {
            let id = lib.ensure_default_remote()?;
            lib.update_remote(
                id,
                &RemoteUpdate { token: Some(Some(token.clone())), ..Default::default() },
            )
        })
    }

    /// The scope an album's subscription on `remote_id` carries — `Web` when
    /// it is not tracked there at all.
    fn scope_of(&self, remote_id: i64, album_path: &str) -> Result<SyncScopeKind> {
        self.with(|lib| {
            Ok(lib
                .album_subscription_for(remote_id, album_path)?
                .map(|s| s.scope)
                .unwrap_or_default())
        })
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

    /// Albums on the default remote, annotated with what this machine knows
    /// about them.
    pub fn remote_albums(&self) -> Result<Vec<crate::remote::RemoteAlbum>> {
        self.remote_albums_on(None)
    }

    /// [`Self::remote_albums`], against a chosen remote (`None` = default).
    pub fn remote_albums_on(&self, remote_id: Option<i64>) -> Result<Vec<crate::remote::RemoteAlbum>> {
        let (id, transport) = self.transport_on(remote_id)?;
        let transport = transport.as_ref();
        self.with(|lib| crate::remote::remote_albums_for(lib, id, transport))
    }

    /// Which albums this machine syncs with the default remote, and how.
    pub fn album_subscriptions(&self) -> Result<Vec<crate::sync::AlbumSubscription>> {
        self.with(|lib| lib.album_subscriptions())
    }

    /// [`Self::album_subscriptions`], for a chosen remote (`None` = default).
    pub fn album_subscriptions_on(
        &self,
        remote_id: Option<i64>,
    ) -> Result<Vec<crate::sync::AlbumSubscription>> {
        self.with(|lib| match remote_id {
            Some(id) => lib.album_subscriptions_for(id),
            None => lib.album_subscriptions(),
        })
    }

    /// Subscribe to an album and say which way it may move, replacing any
    /// direction already chosen for it.
    ///
    /// This is the opt-in the whole design rests on: an album nobody tracks is
    /// out of scope entirely — never pushed, never pulled, never deleted on
    /// either side — so a laptop holding three albums out of two hundred can
    /// sync without endangering the other hundred and ninety-seven.
    pub fn track_album(&self, album_path: String, direction: crate::sync::SyncDirection) -> Result<()> {
        self.track_album_on(album_path, direction, None, None)
    }

    /// [`Self::track_album`] on a chosen remote, optionally choosing how much
    /// of the album moves: `web` (the default — the published tree) or `full`
    /// (originals and metadata as well). An absent scope keeps whatever the
    /// subscription already carries.
    pub fn track_album_on(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
        scope: Option<SyncScopeKind>,
        remote_id: Option<i64>,
    ) -> Result<()> {
        self.with(|lib| {
            let id = match remote_id {
                Some(id) => id,
                None => lib.ensure_default_remote()?,
            };
            lib.track_album_for(&album_path, direction, scope, id)
        })
    }

    /// Stop syncing an album. Files stay exactly where they are on both sides;
    /// this only takes the album out of scope, so "I don't want this synced any
    /// more" can never be the click that removes a wedding from the server.
    pub fn untrack_album(&self, album_path: String) -> Result<()> {
        self.untrack_album_on(album_path, None)
    }

    /// [`Self::untrack_album`], on a chosen remote (`None` = default).
    pub fn untrack_album_on(&self, album_path: String, remote_id: Option<i64>) -> Result<()> {
        self.with(|lib| match remote_id {
            Some(id) => lib.untrack_album_for(&album_path, id),
            None => lib.untrack_album(&album_path),
        })
    }

    /// Preview one album's sync without moving anything.
    pub fn plan_album_sync(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
    ) -> Result<crate::sync::SyncPlan> {
        self.plan_album_sync_on(album_path, direction, None)
    }

    /// [`Self::plan_album_sync`], against a chosen remote (`None` = default).
    pub fn plan_album_sync_on(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
        remote_id: Option<i64>,
    ) -> Result<crate::sync::SyncPlan> {
        let (id, transport) = self.transport_on(remote_id)?;
        let transport = transport.as_ref();
        let root = self.published_root()?;
        self.with(|lib| {
            crate::remote::plan_album_sync_for(lib, id, transport, &album_path, direction, &root)
        })
    }

    /// Adopt a path from the remote: the folders above it, the album or
    /// collection itself, and everything under it.
    ///
    /// Only ever *adds* a photo to the library. Where a frame is already here,
    /// this machine's file is the negative and it is kept untouched, with the
    /// server's version named in [`crate::remote::PullOutcome::kept_originals`]
    /// for a person to look at. Paths the server named that this machine
    /// refused to write come back in `rejected`; both are warnings a UI should
    /// show, because a well-behaved server produces neither.
    pub fn pull_album(&self, album_path: String) -> Result<crate::remote::PullOutcome> {
        self.pull_album_on(album_path, None)
    }

    /// [`Self::pull_album`], from a chosen remote (`None` = default). The
    /// subscription's scope decides how much arrives: a `full`-tracked album
    /// also pulls its originals and metadata.
    pub fn pull_album_on(
        &self,
        album_path: String,
        remote_id: Option<i64>,
    ) -> Result<crate::remote::PullOutcome> {
        let (id, transport) = self.transport_on(remote_id)?;
        let transport = transport.as_ref();
        let root = self.published_root()?;
        let scope = self.scope_of(id, &album_path)?;
        self.with(|lib| crate::remote::pull_path_for(lib, id, transport, &album_path, &root, scope))
    }

    /// Contribute a path to the remote: its folders, itself, everything under
    /// it. Publishes first, so what goes up is byte-for-byte what the gallery
    /// would read.
    ///
    /// `allow_deletes` has to be an answered question, never a default: without
    /// it, files this run would have removed from the server are listed in
    /// [`crate::remote::PushOutcome::withheld_deletes`] and left alone, for the
    /// UI to name and ask about before a second run.
    pub fn push_album(&self, album_path: String, allow_deletes: bool) -> Result<crate::remote::PushOutcome> {
        self.push_album_on(album_path, allow_deletes, None)
    }

    /// [`Self::push_album`], to a chosen remote (`None` = default). The
    /// subscription's scope decides how much goes up: a `full`-tracked album
    /// also pushes its originals and metadata.
    pub fn push_album_on(
        &self,
        album_path: String,
        allow_deletes: bool,
        remote_id: Option<i64>,
    ) -> Result<crate::remote::PushOutcome> {
        let (id, transport) = self.transport_on(remote_id)?;
        let transport = transport.as_ref();
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        let scope = self.scope_of(id, &album_path)?;
        self.with(|lib| {
            crate::remote::push_path_for(
                lib, id, transport, &album_path, &root, &opts, allow_deletes, scope,
            )
        })
    }

    /// Push a path to a remote that belongs to another library (or to any bare
    /// target/token pair): a **stateless** push. No local baselines exist for
    /// a foreign remote, so files that are new or differ from the remote's
    /// manifest are uploaded, nothing is ever deleted (there is no
    /// `allow_deletes` here at all), and every overwrite of a differing remote
    /// file is named in the outcome for the caller's UI to warn about. The
    /// outcome carries this library's id and name for provenance labeling.
    pub fn push_album_to(
        &self,
        album_path: String,
        target: String,
        token: Option<String>,
        scope: Option<SyncScopeKind>,
    ) -> Result<crate::remote::ForeignPushOutcome> {
        let transport = Self::build_transport(&target, token)?;
        let transport = transport.as_ref();
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        self.with(|lib| {
            crate::remote::push_album_to(
                lib, transport, &album_path, &root, &opts, scope.unwrap_or_default(),
            )
        })
    }

    /// Sync one album in the requested direction.
    pub fn sync_album(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
        allow_deletes: bool,
    ) -> Result<crate::sync::SyncOutcome> {
        self.sync_album_on(album_path, direction, allow_deletes, None)
    }

    /// [`Self::sync_album`], against a chosen remote (`None` = default), in
    /// the subscription's scope.
    pub fn sync_album_on(
        &self,
        album_path: String,
        direction: crate::sync::SyncDirection,
        allow_deletes: bool,
        remote_id: Option<i64>,
    ) -> Result<crate::sync::SyncOutcome> {
        let (id, transport) = self.transport_on(remote_id)?;
        let transport = transport.as_ref();
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        let scope = self.scope_of(id, &album_path)?;
        self.with(|lib| {
            crate::remote::sync_path_for(
                lib, id, transport, &album_path, direction, &root, &opts, allow_deletes, scope,
            )
        })
    }

    /// Sync every subscribed album, each in its own direction.
    ///
    /// One album's failure never stops the batch: it is reported against that
    /// album in its own outcome's `failed` and the rest carry on. An album
    /// renamed on the server on Wednesday used to abort the whole run, and the
    /// photographer had no way to see which one was at fault, or that nothing
    /// else had gone up either.
    pub fn sync_all_tracked(&self, allow_deletes: bool) -> Result<Vec<(String, crate::sync::SyncOutcome)>> {
        self.sync_all_tracked_on(allow_deletes, None)
    }

    /// [`Self::sync_all_tracked`], against a chosen remote (`None` = default)
    /// — each subscribed album in its own direction and its own scope.
    pub fn sync_all_tracked_on(
        &self,
        allow_deletes: bool,
        remote_id: Option<i64>,
    ) -> Result<Vec<(String, crate::sync::SyncOutcome)>> {
        let (id, transport) = self.transport_on(remote_id)?;
        let transport = transport.as_ref();
        let root = self.published_root()?;
        let opts = self.publish_options()?;
        self.with(|lib| {
            crate::remote::sync_tracked_albums_for(lib, id, transport, &root, &opts, allow_deletes)
        })
    }

    /// Write (or refresh) XMP sidecars for one album's photos, or for the
    /// whole catalog when `album_path` is `None` — the interchange half of
    /// "the catalog is the source of truth". Standard fields (`xmp:Rating`,
    /// `xmp:Label`, `dc:subject`, `tiff:Orientation`) plus the develop stack
    /// under the versioned `gpp:` namespace. Never touches the image file;
    /// never overwrites a sidecar another tool wrote.
    pub fn export_xmp(&self, album_path: Option<String>) -> Result<crate::xmp::XmpExportOutcome> {
        self.with(|lib| lib.export_xmp(album_path.as_deref()))
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

    /// The GUI's cancel is a latch, and a latch left closed would make every
    /// import after the first one stop before it read a file.
    #[test]
    fn a_cancel_does_not_carry_over_into_the_next_import() {
        let src = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 100, 80);
        write_jpeg(&src.path().join("b.jpg"), 100, 80);

        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.cancel_import();

        let summary = s.import(None, None).unwrap();
        assert!(!summary.cancelled);
        assert_eq!(summary.imported, 2);
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

    /// "No minimum" is one of the choices the publish panel offers, so it has
    /// to take. Read as "leave it alone", a cleared field kept the old filter
    /// in place and the frames the photographer had just asked for silently
    /// did not publish.
    #[test]
    fn clearing_the_minimum_rating_really_clears_it() {
        let src = tempfile::tempdir().unwrap();
        let s = Session::new();
        s.open_library(src.path()).unwrap();

        s.set_publish_target(PublishTarget {
            dest: Some("/tmp/gallery".into()),
            min_rating: Some(4),
        })
        .unwrap();
        assert_eq!(s.publish_target().unwrap().min_rating, Some(4));

        s.set_publish_target(PublishTarget {
            dest: Some("/tmp/gallery".into()),
            min_rating: None,
        })
        .unwrap();
        assert_eq!(s.publish_target().unwrap().min_rating, None);
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

    /// Rotate is the one adjustment the UI drives relatively, and it has to
    /// survive the whole round trip: stack, catalog, and a re-render the grid
    /// can actually point at.
    #[test]
    fn rotating_twice_turns_the_photo_half_way_and_re_renders_it() {
        use crate::develop::EditOp;

        let src = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 200, 100);

        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.import(None, None).unwrap();
        let id = s.photos(PhotoFilter::default()).unwrap()[0].id;
        let upright = s.thumbnail_path(id, "medium").unwrap();

        assert_eq!(s.rotate_photos(vec![id], 1).unwrap(), 1);
        assert_eq!(s.rotate_photos(vec![id], 1).unwrap(), 1);
        assert_eq!(
            s.photo_edits(id).unwrap().get("rotate"),
            Some(&EditOp::Rotate { quarter_turns: 2 })
        );

        // A new render key, and the thumbnail behind it really exists — this is
        // what the grid asks for the moment the edit lands.
        let turned = s.thumbnail_path(id, "medium").unwrap();
        assert_ne!(turned, upright);
        assert!(Path::new(&turned).exists());

        // A quarter turn swaps the axes, so this one lands taller than wide.
        let quarter = s.rotate_photos(vec![id], 1).unwrap();
        assert_eq!(quarter, 1);
        let img = image::open(s.thumbnail_path(id, "medium").unwrap()).unwrap();
        assert!(img.height() > img.width(), "three quarters is on its side");

        // All the way round is no edit at all, back on the original thumbnails.
        s.rotate_photos(vec![id], 1).unwrap();
        assert!(s.photo_edits(id).unwrap().is_empty());
        assert_eq!(s.thumbnail_path(id, "medium").unwrap(), upright);
    }

    #[test]
    fn a_flip_comes_off_with_the_same_button() {
        use crate::develop::EditOp;

        let src = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 120, 80);
        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.import(None, None).unwrap();
        let id = s.photos(PhotoFilter::default()).unwrap()[0].id;

        s.toggle_photo_edit(vec![id], EditOp::FlipHorizontal).unwrap();
        assert!(s.photo_edits(id).unwrap().get("flip-horizontal").is_some());
        s.toggle_photo_edit(vec![id], EditOp::FlipHorizontal).unwrap();
        assert!(s.photo_edits(id).unwrap().is_empty());
    }

    /// Taking a photo out of an album is a membership change, never a deletion —
    /// the app says so, and the core has to mean it.
    #[test]
    fn removing_from_an_album_leaves_the_photo_in_the_library() {
        let src = tempfile::tempdir().unwrap();
        write_jpeg(&src.path().join("a.jpg"), 80, 60);
        write_jpeg(&src.path().join("b.jpg"), 80, 60);

        let s = Session::new();
        s.open_library(src.path()).unwrap();
        s.import(None, None).unwrap();
        let ids: Vec<i64> = s
            .photos(PhotoFilter::default())
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();

        s.create_album(NewAlbum { path: "2026/x".into(), ..Default::default() }).unwrap();
        s.add_to_album("2026/x".into(), ids.clone()).unwrap();
        assert_eq!(s.album_photos("2026/x".into()).unwrap().len(), 2);

        assert_eq!(s.remove_from_album("2026/x".into(), vec![ids[0]]).unwrap(), 1);
        assert_eq!(s.album_photos("2026/x".into()).unwrap().len(), 1);
        assert_eq!(s.status().unwrap().photo_count, 2, "the file is still catalogued");
        assert!(Path::new(&s.photo_path(ids[0]).unwrap()).exists(), "and still on disk");
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
