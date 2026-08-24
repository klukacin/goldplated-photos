//! Tauri shell — one thin command per [`gpp_core::Session`] method.
//!
//! There is deliberately **no logic here**. Everything the app does lives in
//! the core, where it is unit-tested and where it also compiles for iOS. This
//! file exists to move JSON across the webview boundary.

use gpp_core::albums::{AlbumUpdate, NewAlbum};
use gpp_core::develop::{EditOp, EditStack};
use gpp_core::model::{Flag, ImportSummary, Photo, PhotoFilter};
use gpp_core::publish::PublishResult;
use gpp_core::session::{AlbumSummary, LibraryStatus, PublishTarget, Session};
use gpp_core::remote::{PullOutcome, PushOutcome, RemoteAlbum};
use gpp_core::sync::{AlbumSubscription, SyncDirection, SyncOutcome, SyncPlan};
use tauri::{Emitter, Manager, State};

/// Commands return `Result<T, String>`: the webview only needs the message.
type CmdResult<T> = std::result::Result<T, String>;

fn to_msg(e: gpp_core::Error) -> String {
    e.to_string()
}

// ------------------------------------------------------------------ library

#[tauri::command]
fn open_library(app: tauri::AppHandle, path: String) -> CmdResult<LibraryStatus> {
    let status = app.state::<Session>().open_library(&path).map_err(to_msg)?;
    allow_reading_library(&app);
    record_library_opened(&app, &status.root);
    Ok(status)
}

// ------------------------------------------------------- known libraries
//
// A small registry of every library this machine has opened, so the titlebar
// can offer them back as a menu. It is the shell's own state — about the app
// on this machine, not about any one library — so it lives in the app config
// directory, not in a catalog. Forgetting an entry edits this file and
// nothing else; the library on disk is never touched.

/// One remembered library. Written to and read from `libraries.json`; the
/// struct only ever travels *out* to the webview, so there is no unknown-field
/// trap here, and reads stay tolerant so a field added later cannot make an
/// older build throw the whole list away.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct KnownLibrary {
    /// Folder basename — what the menu shows.
    name: String,
    /// The canonical root the core reported, not whatever was typed.
    path: String,
    /// Seconds since the Unix epoch; the list is kept newest-first.
    last_opened_at: u64,
}

/// The registry file, named in exactly one place (check-shell.mjs holds it to
/// that): every reader and writer below goes through this constant.
const LIBRARIES_REGISTRY: &str = "libraries.json";

fn registry_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(LIBRARIES_REGISTRY))
}

/// A missing or unreadable registry is an empty one, never an error: the menu
/// degrades to "Open other library…" instead of blocking the app.
fn read_registry(app: &tauri::AppHandle) -> Vec<KnownLibrary> {
    let Ok(path) = registry_path(app) else { return Vec::new() };
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new() };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write through a temp file and rename, so a crash mid-write leaves the old
/// list rather than half a JSON document that then reads as an empty one.
fn write_registry(app: &tauri::AppHandle, list: &[KnownLibrary]) -> Result<(), String> {
    let path = registry_path(app)?;
    let dir = path.parent().ok_or("registry path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Called on every successful open — picker, menu, remembered, or command
/// line — so the very first open is already in the list. Failure to record is
/// only logged: the library did open, and that must not be reported as an error.
fn record_library_opened(app: &tauri::AppHandle, root: &str) {
    let name = std::path::Path::new(root)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut list = read_registry(app);
    list.retain(|e| e.path != root);
    list.insert(
        0,
        KnownLibrary { name, path: root.to_string(), last_opened_at: now },
    );
    list.sort_by(|a, b| b.last_opened_at.cmp(&a.last_opened_at));
    if let Err(e) = write_registry(app, &list) {
        eprintln!("could not record library {root} in the registry: {e}");
    }
}

/// The registry, newest-first. Read fresh on every call — the file is tiny and
/// a stale in-memory copy would show a forgotten library back in the menu.
#[tauri::command]
fn known_libraries(app: tauri::AppHandle) -> CmdResult<Vec<KnownLibrary>> {
    Ok(read_registry(&app))
}

/// Remove one entry from the registry and return what is left. Only the list
/// changes; the library on disk is never touched.
#[tauri::command]
fn forget_library(app: tauri::AppHandle, path: String) -> CmdResult<Vec<KnownLibrary>> {
    let mut list = read_registry(&app);
    list.retain(|e| e.path != path);
    write_registry(&app, &list)?;
    Ok(list)
}

/// Let the webview load images out of this library.
///
/// The asset protocol is deny-by-default and the static `$HOME/**` scope is
/// wrong twice over for a photo app: libraries live on external drives and
/// network volumes as often as under `$HOME`, and the thumbnail cache sits in a
/// dot-directory that the glob will not match. Granting the directory that was
/// actually opened is both narrower and correct.
fn allow_reading_library(app: &tauri::AppHandle) {
    use tauri::Manager;
    let dirs = match app.state::<Session>().image_dirs() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not determine image directories: {e}");
            return;
        }
    };
    for dir in dirs {
        if let Err(e) = app.asset_protocol_scope().allow_directory(&dir, true) {
            eprintln!("could not grant image access for {dir}: {e}");
        }
    }
}

#[tauri::command]
fn library_status(state: State<'_, Session>) -> CmdResult<LibraryStatus> {
    state.status().map_err(to_msg)
}

#[tauri::command]
fn is_library_open(state: State<'_, Session>) -> bool {
    state.is_open()
}

// ------------------------------------------------------------------- import

#[tauri::command]
async fn import_photos(
    app: tauri::AppHandle,
    dir: Option<String>,
) -> CmdResult<ImportSummary> {
    // Import is CPU-bound and long-running; keep it off the UI thread and
    // stream progress so the window stays responsive on a 5000-photo card.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Session>();
        let emitter = app.clone();
        state
            .import(
                dir,
                Some(&move |p| {
                    let _ = emitter.emit("import-progress", p);
                }),
            )
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stop the running import. Synchronous on purpose: it only raises a flag, and
/// it must not queue behind the blocking task it is trying to interrupt.
#[tauri::command]
fn cancel_import(state: State<'_, Session>) {
    state.cancel_import();
}

#[tauri::command]
fn prune_missing(state: State<'_, Session>) -> CmdResult<usize> {
    state.prune().map_err(to_msg)
}

// ------------------------------------------------------------------- photos

#[tauri::command]
fn list_photos(state: State<'_, Session>, filter: PhotoFilter) -> CmdResult<Vec<Photo>> {
    state.photos(filter).map_err(to_msg)
}

#[tauri::command]
fn photo_path(state: State<'_, Session>, id: i64) -> CmdResult<String> {
    state.photo_path(id).map_err(to_msg)
}

#[tauri::command]
fn thumbnail_path(state: State<'_, Session>, id: i64, size: String) -> CmdResult<String> {
    state.thumbnail_path(id, &size).map_err(to_msg)
}

#[tauri::command]
fn set_rating(state: State<'_, Session>, ids: Vec<i64>, rating: u8) -> CmdResult<usize> {
    state.set_rating(ids, rating).map_err(to_msg)
}

#[tauri::command]
fn set_flag(state: State<'_, Session>, ids: Vec<i64>, flag: Flag) -> CmdResult<usize> {
    state.set_flag(ids, flag).map_err(to_msg)
}

#[tauri::command]
fn set_color_label(
    state: State<'_, Session>,
    id: i64,
    label: Option<String>,
) -> CmdResult<()> {
    state.set_color_label(id, label).map_err(to_msg)
}

// ------------------------------------------------------------------ develop

#[tauri::command]
fn photo_edits(state: State<'_, Session>, id: i64) -> CmdResult<EditStack> {
    state.photo_edits(id).map_err(to_msg)
}

#[tauri::command]
async fn set_photo_edit(
    app: tauri::AppHandle,
    ids: Vec<i64>,
    op: EditOp,
) -> CmdResult<usize> {
    // Rendering the new thumbnails is CPU-bound; a whole selection of them must
    // not freeze the slider being dragged.
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>().set_photo_edit(ids, op).map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Relative, unlike `set_photo_edit`: the app's rotate buttons add a quarter
/// turn to whatever each photo already carries.
#[tauri::command]
async fn rotate_photos(
    app: tauri::AppHandle,
    ids: Vec<i64>,
    quarter_turns: i32,
) -> CmdResult<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>()
            .rotate_photos(ids, quarter_turns)
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn toggle_photo_edit(
    app: tauri::AppHandle,
    ids: Vec<i64>,
    op: EditOp,
) -> CmdResult<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>()
            .toggle_photo_edit(ids, op)
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn clear_photo_edit(
    app: tauri::AppHandle,
    ids: Vec<i64>,
    kind: String,
) -> CmdResult<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>()
            .clear_photo_edit(ids, kind)
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn reset_photo_edits(app: tauri::AppHandle, ids: Vec<i64>) -> CmdResult<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>().reset_photo_edits(ids).map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ------------------------------------------------------------------- albums

#[tauri::command]
fn list_albums(state: State<'_, Session>) -> CmdResult<Vec<AlbumSummary>> {
    state.albums().map_err(to_msg)
}

#[tauri::command]
fn create_album(state: State<'_, Session>, album: NewAlbum) -> CmdResult<gpp_core::Album> {
    state.create_album(album).map_err(to_msg)
}

#[tauri::command]
fn update_album(
    state: State<'_, Session>,
    path: String,
    update: AlbumUpdate,
) -> CmdResult<gpp_core::Album> {
    state.update_album(path, update).map_err(to_msg)
}

#[tauri::command]
fn move_album(state: State<'_, Session>, from: String, to: String) -> CmdResult<gpp_core::Album> {
    state.move_album(from, to).map_err(to_msg)
}

#[tauri::command]
fn delete_album(state: State<'_, Session>, path: String) -> CmdResult<()> {
    state.delete_album(path).map_err(to_msg)
}

#[tauri::command]
fn album_photos(state: State<'_, Session>, path: String) -> CmdResult<Vec<Photo>> {
    state.album_photos(path).map_err(to_msg)
}

#[tauri::command]
fn add_to_album(state: State<'_, Session>, path: String, ids: Vec<i64>) -> CmdResult<usize> {
    state.add_to_album(path, ids).map_err(to_msg)
}

#[tauri::command]
fn remove_from_album(state: State<'_, Session>, path: String, ids: Vec<i64>) -> CmdResult<usize> {
    state.remove_from_album(path, ids).map_err(to_msg)
}

#[tauri::command]
fn reorder_album(state: State<'_, Session>, path: String, ids: Vec<i64>) -> CmdResult<()> {
    state.reorder_album(path, ids).map_err(to_msg)
}

#[tauri::command]
fn generate_share_link(state: State<'_, Session>, path: String) -> CmdResult<String> {
    state.generate_share_link(path).map_err(to_msg)
}

// ------------------------------------------------------------------ publish

#[tauri::command]
fn get_publish_target(state: State<'_, Session>) -> CmdResult<PublishTarget> {
    state.publish_target().map_err(to_msg)
}

#[tauri::command]
fn set_publish_target(state: State<'_, Session>, target: PublishTarget) -> CmdResult<()> {
    state.set_publish_target(target).map_err(to_msg)
}

#[tauri::command]
async fn publish(app: tauri::AppHandle, album: Option<String>) -> CmdResult<Vec<PublishResult>> {
    // Copying originals is I/O heavy — same treatment as import.
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>().publish(album).map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn sync_plan(state: State<'_, Session>) -> CmdResult<SyncPlan> {
    state.sync_plan().map_err(to_msg)
}

// -------------------------------------------------------------------- remote

#[tauri::command]
fn get_remote_dir(state: State<'_, Session>) -> CmdResult<Option<String>> {
    state.remote_dir().map_err(to_msg)
}

#[tauri::command]
fn set_remote_dir(state: State<'_, Session>, dir: String) -> CmdResult<()> {
    state.set_remote_dir(dir).map_err(to_msg)
}

/// Whether a token is on file — never the token itself, which stays in the
/// catalog and has no reason to travel back into a web view.
#[tauri::command]
fn has_remote_token(state: State<'_, Session>) -> CmdResult<bool> {
    Ok(state.remote_token().map_err(to_msg)?.is_some())
}

#[tauri::command]
fn set_remote_token(state: State<'_, Session>, token: String) -> CmdResult<()> {
    state.set_remote_token(token).map_err(to_msg)
}

#[tauri::command]
fn remote_albums(state: State<'_, Session>) -> CmdResult<Vec<RemoteAlbum>> {
    state.remote_albums().map_err(to_msg)
}

#[tauri::command]
fn album_subscriptions(state: State<'_, Session>) -> CmdResult<Vec<AlbumSubscription>> {
    state.album_subscriptions().map_err(to_msg)
}

#[tauri::command]
fn track_album(
    state: State<'_, Session>,
    path: String,
    direction: SyncDirection,
) -> CmdResult<()> {
    state.track_album(path, direction).map_err(to_msg)
}

#[tauri::command]
fn untrack_album(state: State<'_, Session>, path: String) -> CmdResult<()> {
    state.untrack_album(path).map_err(to_msg)
}

#[tauri::command]
fn plan_album_sync(
    state: State<'_, Session>,
    path: String,
    direction: SyncDirection,
) -> CmdResult<SyncPlan> {
    state.plan_album_sync(path, direction).map_err(to_msg)
}

#[tauri::command]
async fn pull_album(app: tauri::AppHandle, path: String) -> CmdResult<PullOutcome> {
    // Network/disk bound: keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>().pull_album(path).map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn push_album(
    app: tauri::AppHandle,
    path: String,
    allow_deletes: bool,
) -> CmdResult<PushOutcome> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>()
            .push_album(path, allow_deletes)
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sync_album(
    app: tauri::AppHandle,
    path: String,
    direction: SyncDirection,
    allow_deletes: bool,
) -> CmdResult<SyncOutcome> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>()
            .sync_album(path, direction, allow_deletes)
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sync_all_tracked(
    app: tauri::AppHandle,
    allow_deletes: bool,
) -> CmdResult<Vec<(String, SyncOutcome)>> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Session>()
            .sync_all_tracked(allow_deletes)
            .map_err(to_msg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Library path passed on the command line: `gpp-desktop ~/Photos`.
///
/// Lets the app be launched straight into a library — from a shell, a shortcut,
/// or a script — instead of always going through the folder picker.
fn library_from_args() -> Option<String> {
    std::env::args().skip(1).find(|a| !a.starts_with('-'))
}

/// Entry point shared by the desktop binary and (later) the mobile target.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Session::new())
        .setup(|app| {
            if let Some(path) = library_from_args() {
                // A bad path is not fatal: the window still opens on the
                // welcome screen, which is where the user can pick another.
                match app.state::<Session>().open_library(&path) {
                    Ok(status) => {
                        allow_reading_library(app.handle());
                        // The first open of a fresh install can be this one.
                        record_library_opened(app.handle(), &status.root);
                    }
                    Err(e) => eprintln!("could not open library {path}: {e}"),
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_library,
            known_libraries,
            forget_library,
            library_status,
            is_library_open,
            import_photos,
            cancel_import,
            prune_missing,
            list_photos,
            photo_path,
            thumbnail_path,
            set_rating,
            set_flag,
            set_color_label,
            photo_edits,
            set_photo_edit,
            rotate_photos,
            toggle_photo_edit,
            clear_photo_edit,
            reset_photo_edits,
            list_albums,
            create_album,
            update_album,
            move_album,
            delete_album,
            album_photos,
            add_to_album,
            remove_from_album,
            reorder_album,
            generate_share_link,
            get_publish_target,
            set_publish_target,
            publish,
            sync_plan,
            get_remote_dir,
            set_remote_dir,
            has_remote_token,
            set_remote_token,
            remote_albums,
            album_subscriptions,
            track_album,
            untrack_album,
            plan_album_sync,
            pull_album,
            push_album,
            sync_album,
            sync_all_tracked,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Goldplated Photos");
}
