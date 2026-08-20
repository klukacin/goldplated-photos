//! Tauri shell — one thin command per [`gpp_core::Session`] method.
//!
//! There is deliberately **no logic here**. Everything the app does lives in
//! the core, where it is unit-tested and where it also compiles for iOS. This
//! file exists to move JSON across the webview boundary.

use gpp_core::albums::{AlbumUpdate, NewAlbum};
use gpp_core::model::{Flag, ImportSummary, Photo, PhotoFilter};
use gpp_core::publish::PublishResult;
use gpp_core::session::{AlbumSummary, LibraryStatus, PublishTarget, Session};
use gpp_core::sync::SyncPlan;
use tauri::{Emitter, Manager, State};

/// Commands return `Result<T, String>`: the webview only needs the message.
type CmdResult<T> = std::result::Result<T, String>;

fn to_msg(e: gpp_core::Error) -> String {
    e.to_string()
}

// ------------------------------------------------------------------ library

#[tauri::command]
fn open_library(state: State<'_, Session>, path: String) -> CmdResult<LibraryStatus> {
    state.open_library(path).map_err(to_msg)
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

/// Entry point shared by the desktop binary and (later) the mobile target.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Session::new())
        .invoke_handler(tauri::generate_handler![
            open_library,
            library_status,
            is_library_open,
            import_photos,
            prune_missing,
            list_photos,
            photo_path,
            thumbnail_path,
            set_rating,
            set_flag,
            set_color_label,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Goldplated Photos");
}
