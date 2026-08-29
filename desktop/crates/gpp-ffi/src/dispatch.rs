//! The method table: one match arm per [`Session`] call.
//!
//! Nothing in this module touches a raw pointer. It takes two `&str` and gives
//! back a `serde_json::Value`, which makes the interesting half of the FFI —
//! "does this method name do the right thing with these arguments" — testable
//! without a single `unsafe` block. [`crate::gpp_call`] is then only concerned
//! with the C boundary itself.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use gpp_core::albums::{AlbumUpdate, NewAlbum};
use gpp_core::develop::EditOp;
use gpp_core::model::{Flag, PhotoFilter};
use gpp_core::remotes::{PublishTargetUpdate, RemoteUpdate};
use gpp_core::session::PublishTarget;
use gpp_core::sources::SourceKind;
use gpp_core::sync::{SyncDirection, SyncScopeKind};
use gpp_core::Session;

/// Everything that can go wrong on this side of the boundary, carrying a stable
/// tag as well as a message.
///
/// A foreign caller cannot pattern-match a Rust enum and should not be parsing
/// English prose to find out whether an album was missing or the disk was: the
/// tag is the part it is allowed to branch on, the message the part it shows a
/// human.
pub(crate) struct CallError {
    pub kind: &'static str,
    pub message: String,
}

impl CallError {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<gpp_core::Error> for CallError {
    fn from(e: gpp_core::Error) -> Self {
        use gpp_core::Error as E;
        // The tags are part of the published contract, so they are written out
        // rather than derived from the variant name: renaming a Rust variant
        // must not silently change what a shipped Swift app switches on.
        let kind = match &e {
            E::Database(_) => "database",
            E::Io { .. } | E::PlainIo(_) => "io",
            E::Image(_) => "image",
            E::Serde(_) => "serde",
            E::InvalidPath(_) => "invalid-path",
            E::AlbumNotFound(_) => "album-not-found",
            E::PhotoNotFound(_) => "photo-not-found",
            E::AlbumExists(_) => "album-exists",
            E::Unsupported(_) => "unsupported",
            E::SourceOffline { .. } => "source-offline",
            E::SourceMismatch { .. } => "source-mismatch",
            E::SyncConflict { .. } => "sync-conflict",
            E::Other(_) => "other",
        };
        CallError::new(kind, e.to_string())
    }
}

/// Every method name [`dispatch`] answers to.
///
/// Exported so a Rust caller can enumerate the door, and so the test suite can
/// assert that each listed name really resolves — a name in this list that the
/// match forgot would otherwise look like a documented method and behave like a
/// typo.
pub const METHODS: &[&str] = &[
    // library
    "open_library",
    "is_open",
    "status",
    "image_dirs",
    // import
    "import",
    "cancel_import",
    "prune",
    // sources (schema v6)
    "sources",
    "add_source",
    "remove_source",
    "relocate_source",
    // lightroom
    "lr_scan",
    "lr_import",
    // photos
    "photos",
    "photo",
    "photo_path",
    "thumbnail_path",
    "set_rating",
    "set_flag",
    "set_color_label",
    // develop
    "photo_edits",
    "set_photo_edit",
    "rotate_photos",
    "toggle_photo_edit",
    "clear_photo_edit",
    "reset_photo_edits",
    // albums
    "albums",
    "create_album",
    "update_album",
    "move_album",
    "delete_album",
    "album_photos",
    "add_to_album",
    "remove_from_album",
    "reorder_album",
    "generate_share_link",
    // publish
    "publish_target",
    "set_publish_target",
    "publish",
    "sync_plan",
    // remote
    "remote_dir",
    "set_remote_dir",
    "has_remote_token",
    "set_remote_token",
    "remote_albums",
    "album_subscriptions",
    "track_album",
    "untrack_album",
    "plan_album_sync",
    "pull_album",
    "push_album",
    "sync_album",
    "sync_all_tracked",
    // remotes & publish targets (schema v5)
    "remotes",
    "add_remote",
    "update_remote",
    "remove_remote",
    "set_default_remote",
    "publish_targets",
    "add_publish_target",
    "update_publish_target",
    "remove_publish_target",
    "set_default_publish_target",
    "read_library_remotes",
    "push_album_to",
    // interchange
    "export_xmp",
];

/// A call that takes no arguments.
///
/// It still goes through serde so that `{"typo": 1}` is refused rather than
/// ignored: silently accepting arguments a method does not read is how a caller
/// comes to believe it passed something.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// Parse the argument object.
///
/// A null or blank `args_json` means `{}`, so a no-argument call from C can
/// pass `NULL` instead of the string `"{}"`.
fn parse<T: DeserializeOwned>(args: &str) -> Result<T, CallError> {
    let text = if args.trim().is_empty() { "{}" } else { args };
    serde_json::from_str(text)
        .map_err(|e| CallError::new("bad-arguments", format!("invalid arguments: {e}")))
}

/// Serialize a return value, or say so if it cannot be.
fn ok<T: serde::Serialize>(value: T) -> Result<Value, CallError> {
    serde_json::to_value(value)
        .map_err(|e| CallError::new("serde", format!("could not encode the result: {e}")))
}

/// Route one call.
///
/// The method names are [`Session`]'s own method names, unchanged. That is the
/// whole naming rule: there is no second vocabulary to keep in step, and a
/// reader of `session.rs` already knows what to type here.
///
/// The same rule extends to arguments: a key is the Rust parameter name,
/// spelled exactly as `session.rs` spells it. Nested objects are the core's own
/// serde shapes, so most of them are `snake_case` too — the exceptions are
/// [`PhotoFilter`], [`NewAlbum`] and [`AlbumUpdate`], which the web UI made
/// `camelCase` before this door existed. Renaming them here would fix the seam
/// for foreign callers and break it for the Tauri shell, so the inconsistency
/// is documented in the README rather than papered over.
///
/// Every argument struct denies unknown fields.
pub(crate) fn dispatch(session: &Session, method: &str, args: &str) -> Result<Value, CallError> {
    // Shorthands for the argument shapes that repeat.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Id {
        id: i64,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Ids {
        ids: Vec<i64>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AlbumPath {
        path: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PathAndPhotos {
        path: String,
        photo_ids: Vec<i64>,
    }
    /// The remote calls name their album `album_path`, matching `Session`.
    /// `remote_id` is optional everywhere it appears: omitted means the
    /// default remote, so every pre-v5 caller keeps working unchanged.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SyncPath {
        album_path: String,
        #[serde(default)]
        remote_id: Option<i64>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RemoteId {
        id: i64,
    }

    match method {
        // ------------------------------------------------------------ library
        "open_library" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                path: String,
            }
            let a: A = parse(args)?;
            ok(session.open_library(&a.path)?)
        }
        "is_open" => {
            let _: NoArgs = parse(args)?;
            ok(session.is_open())
        }
        "status" => {
            let _: NoArgs = parse(args)?;
            ok(session.status()?)
        }
        "image_dirs" => {
            let _: NoArgs = parse(args)?;
            ok(session.image_dirs()?)
        }

        // ------------------------------------------------------------- import
        //
        // `Session::import` also takes a progress callback. There is no honest
        // way to hand a Rust closure to C, and inventing a function-pointer
        // channel here would be a second, unrelated door. Through this one an
        // import is synchronous and silent: run it on a background thread and
        // show an indeterminate spinner. See the README.
        "import" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                #[serde(default)]
                dir: Option<String>,
            }
            let a: A = parse(args)?;
            ok(session.import(a.dir, None)?)
        }
        // The one call that is useful *while* another is running, and the
        // reason it works is that it never reaches the library: it raises an
        // atomic flag the import reads between files. A foreign client that
        // started a 2000-frame card on a background thread otherwise has
        // nothing to do but wait out the tens of minutes it takes.
        "cancel_import" => {
            let _: NoArgs = parse(args)?;
            session.cancel_import();
            ok(())
        }
        "prune" => {
            let _: NoArgs = parse(args)?;
            ok(session.prune()?)
        }

        // ------------------------------------------------ sources (v6)
        //
        // A source is a root photographs may live under. Registering one is
        // what makes an import catalogue files where they are instead of
        // copying them in — the whole of "import my Lightroom library without
        // moving a terabyte". Nothing here writes to, moves or deletes a
        // photograph.
        "sources" => {
            let _: NoArgs = parse(args)?;
            ok(session.sources()?)
        }
        "add_source" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                path: String,
                /// Omit to take the folder's own name.
                #[serde(default)]
                name: Option<String>,
                /// `internal`, `external` (the default) or `network`.
                #[serde(default)]
                kind: Option<SourceKind>,
            }
            let a: A = parse(args)?;
            ok(session.add_source(a.path, a.name, a.kind)?)
        }
        // `drop_photos` is deliberately not defaulted away: a source holding
        // catalog rows is refused without it, because removing it takes every
        // rating, flag, membership and adjustment on those photographs with it.
        // The files themselves are never touched either way.
        "remove_source" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: i64,
                #[serde(default)]
                drop_photos: bool,
            }
            let a: A = parse(args)?;
            ok(session.remove_source(a.id, a.drop_photos)?)
        }
        "relocate_source" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: i64,
                new_path: String,
            }
            let a: A = parse(args)?;
            ok(session.relocate_source(a.id, a.new_path)?)
        }

        // ---------------------------------------------------------- lightroom
        "lr_scan" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                lrcat_path: String,
            }
            let a: A = parse(args)?;
            ok(session.lr_scan(a.lrcat_path)?)
        }
        // Synchronous and silent through this door, like `import` — run it on
        // a background thread; `cancel_import` stops it too.
        "lr_import" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                lrcat_path: String,
                /// Omit for the defaults: copy under `lr/`, all collections,
                /// no prefix, auto collision handling, a real (non-dry) run.
                #[serde(default)]
                options: gpp_core::lightroom::LrImportOptions,
            }
            let a: A = parse(args)?;
            ok(session.lr_import(a.lrcat_path, a.options, None)?)
        }

        // ------------------------------------------------------------- photos
        "photos" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                #[serde(default)]
                filter: PhotoFilter,
            }
            let a: A = parse(args)?;
            ok(session.photos(a.filter)?)
        }
        "photo" => {
            let a: Id = parse(args)?;
            ok(session.photo(a.id)?)
        }
        "photo_path" => {
            let a: Id = parse(args)?;
            ok(session.photo_path(a.id)?)
        }
        "thumbnail_path" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: i64,
                size: String,
            }
            let a: A = parse(args)?;
            ok(session.thumbnail_path(a.id, &a.size)?)
        }
        "set_rating" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                ids: Vec<i64>,
                rating: u8,
            }
            let a: A = parse(args)?;
            ok(session.set_rating(a.ids, a.rating)?)
        }
        "set_flag" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                ids: Vec<i64>,
                flag: Flag,
            }
            let a: A = parse(args)?;
            ok(session.set_flag(a.ids, a.flag)?)
        }
        "set_color_label" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: i64,
                /// Absent and `null` both mean "no label", because a colour
                /// picker set back to neutral sends one or the other.
                #[serde(default)]
                label: Option<String>,
            }
            let a: A = parse(args)?;
            ok(session.set_color_label(a.id, a.label)?)
        }

        // ------------------------------------------------------------ develop
        "photo_edits" => {
            let a: Id = parse(args)?;
            ok(session.photo_edits(a.id)?)
        }
        "set_photo_edit" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                ids: Vec<i64>,
                op: EditOp,
            }
            let a: A = parse(args)?;
            ok(session.set_photo_edit(a.ids, a.op)?)
        }
        // Relative, like the button: `quarter_turns` is how much further to
        // turn, not where to end up. A selection can hold photos at different
        // angles, and setting a `rotate` op instead would flatten them all to
        // the same one.
        "rotate_photos" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                ids: Vec<i64>,
                /// Positive is clockwise; negative turns the other way.
                quarter_turns: i32,
            }
            let a: A = parse(args)?;
            ok(session.rotate_photos(a.ids, a.quarter_turns)?)
        }
        // For the flips, which have no zero to set: `set_photo_edit` would drop
        // the existing mirror and push an identical one back, so pressing the
        // button twice would never undo it.
        "toggle_photo_edit" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                ids: Vec<i64>,
                op: EditOp,
            }
            let a: A = parse(args)?;
            ok(session.toggle_photo_edit(a.ids, a.op)?)
        }
        "clear_photo_edit" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                ids: Vec<i64>,
                kind: String,
            }
            let a: A = parse(args)?;
            ok(session.clear_photo_edit(a.ids, a.kind)?)
        }
        "reset_photo_edits" => {
            let a: Ids = parse(args)?;
            ok(session.reset_photo_edits(a.ids)?)
        }

        // ------------------------------------------------------------- albums
        "albums" => {
            let _: NoArgs = parse(args)?;
            ok(session.albums()?)
        }
        "create_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album: NewAlbum,
            }
            let a: A = parse(args)?;
            ok(session.create_album(a.album)?)
        }
        "update_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                path: String,
                update: AlbumUpdate,
            }
            let a: A = parse(args)?;
            ok(session.update_album(a.path, a.update)?)
        }
        "move_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                from: String,
                to: String,
            }
            let a: A = parse(args)?;
            ok(session.move_album(a.from, a.to)?)
        }
        "delete_album" => {
            let a: AlbumPath = parse(args)?;
            ok(session.delete_album(a.path)?)
        }
        "album_photos" => {
            let a: AlbumPath = parse(args)?;
            ok(session.album_photos(a.path)?)
        }
        "add_to_album" => {
            let a: PathAndPhotos = parse(args)?;
            ok(session.add_to_album(a.path, a.photo_ids)?)
        }
        "remove_from_album" => {
            let a: PathAndPhotos = parse(args)?;
            ok(session.remove_from_album(a.path, a.photo_ids)?)
        }
        "reorder_album" => {
            let a: PathAndPhotos = parse(args)?;
            ok(session.reorder_album(a.path, a.photo_ids)?)
        }
        "generate_share_link" => {
            let a: AlbumPath = parse(args)?;
            ok(session.generate_share_link(a.path)?)
        }

        // ------------------------------------------------------------ publish
        "publish_target" => {
            let _: NoArgs = parse(args)?;
            ok(session.publish_target()?)
        }
        "set_publish_target" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                target: PublishTarget,
            }
            let a: A = parse(args)?;
            ok(session.set_publish_target(a.target)?)
        }
        "publish" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                /// Omit to publish every album.
                #[serde(default)]
                album_path: Option<String>,
                /// Omit for the default publish target.
                #[serde(default)]
                target_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.publish_on(a.album_path, a.target_id)?)
        }
        "sync_plan" => {
            let _: NoArgs = parse(args)?;
            ok(session.sync_plan()?)
        }

        // ------------------------------------------------------------- remote
        "remote_dir" => {
            let _: NoArgs = parse(args)?;
            ok(session.remote_dir()?)
        }
        "set_remote_dir" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                dir: String,
            }
            let a: A = parse(args)?;
            ok(session.set_remote_dir(a.dir)?)
        }
        // Whether a token is on file, never the token — the same line the
        // Tauri shell draws. A secret that has been written into the catalog
        // has no reason to travel back out to a caller that already had it.
        "has_remote_token" => {
            let _: NoArgs = parse(args)?;
            ok(session.remote_token()?.is_some())
        }
        "set_remote_token" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                token: String,
            }
            let a: A = parse(args)?;
            ok(session.set_remote_token(a.token)?)
        }
        "remote_albums" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.remote_albums_on(a.remote_id)?)
        }
        "album_subscriptions" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.album_subscriptions_on(a.remote_id)?)
        }
        "track_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                direction: SyncDirection,
                /// `web` (default) or `full`; omitted keeps what the
                /// subscription already carries.
                #[serde(default)]
                scope: Option<SyncScopeKind>,
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.track_album_on(a.album_path, a.direction, a.scope, a.remote_id)?)
        }
        "untrack_album" => {
            let a: SyncPath = parse(args)?;
            ok(session.untrack_album_on(a.album_path, a.remote_id)?)
        }
        "plan_album_sync" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                direction: SyncDirection,
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.plan_album_sync_on(a.album_path, a.direction, a.remote_id)?)
        }
        "pull_album" => {
            let a: SyncPath = parse(args)?;
            ok(session.pull_album_on(a.album_path, a.remote_id)?)
        }
        "push_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                /// Withheld by default: deleting on the server is the one
                /// action a caller must ask for in so many words.
                #[serde(default)]
                allow_deletes: bool,
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.push_album_on(a.album_path, a.allow_deletes, a.remote_id)?)
        }
        "sync_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                direction: SyncDirection,
                #[serde(default)]
                allow_deletes: bool,
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.sync_album_on(a.album_path, a.direction, a.allow_deletes, a.remote_id)?)
        }
        "sync_all_tracked" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                #[serde(default)]
                allow_deletes: bool,
                #[serde(default)]
                remote_id: Option<i64>,
            }
            let a: A = parse(args)?;
            ok(session.sync_all_tracked_on(a.allow_deletes, a.remote_id)?)
        }

        // ---------------------------------- remotes & publish targets (v5)
        "remotes" => {
            let _: NoArgs = parse(args)?;
            ok(session.remotes()?)
        }
        "add_remote" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                name: String,
                target: String,
                #[serde(default)]
                token: Option<String>,
            }
            let a: A = parse(args)?;
            ok(session.add_remote(a.name, a.target, a.token)?)
        }
        "update_remote" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: i64,
                update: RemoteUpdate,
            }
            let a: A = parse(args)?;
            ok(session.update_remote(a.id, a.update)?)
        }
        "remove_remote" => {
            let a: RemoteId = parse(args)?;
            ok(session.remove_remote(a.id)?)
        }
        "set_default_remote" => {
            let a: RemoteId = parse(args)?;
            ok(session.set_default_remote(a.id)?)
        }
        "publish_targets" => {
            let _: NoArgs = parse(args)?;
            ok(session.publish_targets()?)
        }
        "add_publish_target" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                name: String,
                dest_root: String,
                #[serde(default)]
                min_rating: Option<u8>,
            }
            let a: A = parse(args)?;
            ok(session.add_publish_target(a.name, a.dest_root, a.min_rating)?)
        }
        "update_publish_target" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                id: i64,
                update: PublishTargetUpdate,
            }
            let a: A = parse(args)?;
            ok(session.update_publish_target(a.id, a.update)?)
        }
        "remove_publish_target" => {
            let a: RemoteId = parse(args)?;
            ok(session.remove_publish_target(a.id)?)
        }
        "set_default_publish_target" => {
            let a: RemoteId = parse(args)?;
            ok(session.set_default_publish_target(a.id)?)
        }
        "read_library_remotes" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                library_root: String,
            }
            let a: A = parse(args)?;
            ok(session.read_library_remotes(a.library_root)?)
        }
        "push_album_to" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                target: String,
                #[serde(default)]
                token: Option<String>,
                #[serde(default)]
                scope: Option<SyncScopeKind>,
            }
            let a: A = parse(args)?;
            ok(session.push_album_to(a.album_path, a.target, a.token, a.scope)?)
        }

        // ------------------------------------------------------ interchange
        "export_xmp" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                /// Omit to export sidecars for the whole catalog.
                #[serde(default)]
                album_path: Option<String>,
            }
            let a: A = parse(args)?;
            ok(session.export_xmp(a.album_path)?)
        }

        other => Err(CallError::new(
            "unknown-method",
            format!("unknown method: {other}"),
        )),
    }
}
