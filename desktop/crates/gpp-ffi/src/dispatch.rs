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
use gpp_core::session::PublishTarget;
use gpp_core::sync::SyncDirection;
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
    "prune",
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
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SyncPath {
        album_path: String,
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
        "prune" => {
            let _: NoArgs = parse(args)?;
            ok(session.prune()?)
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
            }
            let a: A = parse(args)?;
            ok(session.publish(a.album_path)?)
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
            let _: NoArgs = parse(args)?;
            ok(session.remote_albums()?)
        }
        "album_subscriptions" => {
            let _: NoArgs = parse(args)?;
            ok(session.album_subscriptions()?)
        }
        "track_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                direction: SyncDirection,
            }
            let a: A = parse(args)?;
            ok(session.track_album(a.album_path, a.direction)?)
        }
        "untrack_album" => {
            let a: SyncPath = parse(args)?;
            ok(session.untrack_album(a.album_path)?)
        }
        "plan_album_sync" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                direction: SyncDirection,
            }
            let a: A = parse(args)?;
            ok(session.plan_album_sync(a.album_path, a.direction)?)
        }
        "pull_album" => {
            let a: SyncPath = parse(args)?;
            ok(session.pull_album(a.album_path)?)
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
            }
            let a: A = parse(args)?;
            ok(session.push_album(a.album_path, a.allow_deletes)?)
        }
        "sync_album" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                album_path: String,
                direction: SyncDirection,
                #[serde(default)]
                allow_deletes: bool,
            }
            let a: A = parse(args)?;
            ok(session.sync_album(a.album_path, a.direction, a.allow_deletes)?)
        }
        "sync_all_tracked" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                #[serde(default)]
                allow_deletes: bool,
            }
            let a: A = parse(args)?;
            ok(session.sync_all_tracked(a.allow_deletes)?)
        }

        other => Err(CallError::new(
            "unknown-method",
            format!("unknown method: {other}"),
        )),
    }
}
