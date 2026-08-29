//! Error type for the core library.
//!
//! This enum is not private plumbing: `gpp-ffi` turns every variant into a
//! stable string tag — `"album-not-found"`, `"invalid-path"` — that a shipped
//! Swift or Kotlin client branches on, and the `Display` text is what a
//! photographer reads in a dialog. Both halves are therefore load-bearing.
//!
//! Two rules follow from that, and neither is enforced by the compiler:
//!
//! - **Adding a variant is a contract change.** The FFI's match is exhaustive,
//!   so a new variant will not compile until it is given a tag — but choosing
//!   one that no client knows still ships a failure nobody can handle. Prefer an
//!   existing variant unless callers would genuinely act differently.
//! - **The message is shown to a human.** It is the whole explanation a foreign
//!   client has, since it cannot see the source chain across the C boundary, so
//!   it must name *what* failed and, where it can, *which file*.
//!
//! Failures of one item inside a batch are deliberately **not** errors: an
//! import that cannot read one frame, a push a server refuses one upload of, a
//! pull that rejects one path — those are reported per item in the outcome
//! structs so the rest of the run still happens. A `Result::Err` here means the
//! whole operation did not.

use std::path::PathBuf;

/// The core's result alias. Every fallible public function returns this.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything the core can fail with.
///
/// See the [module docs](self) for why the variant set and the messages are a
/// published contract rather than an implementation detail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite said no: the catalog is locked beyond the busy timeout, corrupt,
    /// or a statement is wrong.
    ///
    /// Almost always a bug or a damaged file rather than anything the user did —
    /// with one exception worth telling them about, a library on a network share
    /// where SQLite's locking is unreliable. The catalog is a rebuildable index,
    /// so the recovery of last resort is to delete `.gpp/catalog.db` and
    /// re-import; the photographs are never in it.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// A filesystem operation failed, and we know which path it was on.
    ///
    /// Build it with [`Error::io`] rather than letting a `std::io::Error`
    /// propagate on its own: "permission denied" without a filename is
    /// unactionable, and by the time it reaches a dialog in the shell there is
    /// nothing left to add it from.
    #[error("i/o error at {path}: {source}")]
    Io {
        /// The file or directory the operation was on — absolute, as the core
        /// had it.
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// An I/O failure that arrived through `?` with no path attached.
    ///
    /// Same `"io"` tag as [`Error::Io`] over the FFI, so a caller cannot tell
    /// them apart and does not need to. New code should reach for
    /// [`Error::io`] instead — this exists for the places where the path is
    /// genuinely not known.
    #[error("i/o error: {0}")]
    PlainIo(#[from] std::io::Error),

    /// A decode or encode failed: the bytes are not the format the extension
    /// claims, the file is truncated, or it is a variant this build's codecs do
    /// not handle.
    ///
    /// During import this does not surface as an error at all — the frame is
    /// catalogued and listed in `ImportSummary::undecodable`, because a
    /// photograph the catalog forgets is worse than one it cannot preview.
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    /// JSON would not parse or would not encode. In practice: an edit stack
    /// stored by a newer build, or a remote manifest that is not what it claims
    /// to be.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The path escapes the library root, or is otherwise not addressable.
    ///
    /// A refusal, not a mishap — `..`, an absolute path, a NUL, a backslash — and
    /// the same check guards paths a *remote server* hands us during a pull, so
    /// this is a security boundary and not only a typo catcher. Nothing was
    /// written; there is nothing to clean up.
    #[error("invalid path: {0}")]
    InvalidPath(String),

    /// No album at that path. Carries the path asked for.
    ///
    /// Ordinary in a UI that is a moment behind the catalog — a sidebar still
    /// showing an album another window deleted — so a caller can reasonably
    /// refresh and carry on rather than treat it as a fault.
    #[error("album not found: {0}")]
    AlbumNotFound(String),

    /// No photo with that id. Carries the id, stringified.
    ///
    /// The setters raise it rather than reporting a silent no-op, so a rating
    /// applied to a photo that was pruned a second ago is visibly refused
    /// instead of quietly lost.
    #[error("photo not found: {0}")]
    PhotoNotFound(String),

    /// An album already occupies that path, on create, rename or move.
    ///
    /// Refused rather than merged: the two folders would fuse on the next
    /// publish, and a client's gallery would gain photographs from someone
    /// else's wedding. Recover by choosing another path.
    #[error("album already exists: {0}")]
    AlbumExists(String),

    /// This build cannot handle that kind of file — the RAW decoder is a hook
    /// the shell fills in, and the default one supports nothing at all.
    ///
    /// A property of the build, not of the file: the same negative opens in a
    /// shell that links a decoder, so telling the user to convert their photos
    /// would be the wrong advice.
    #[error("unsupported media type: {0}")]
    Unsupported(String),

    /// A photo's source is registered but not reachable right now — an
    /// external drive that is not plugged in, a network share that is not
    /// mounted.
    ///
    /// A distinct variant because the answer is distinct: nothing is wrong
    /// with the library, and the photographer's move is to plug the drive
    /// back in, not to re-import or repair anything. It is never raised for
    /// a file that is simply gone from an *online* source — that is an
    /// absence [`Library::prune_missing`](crate::Library::prune_missing)
    /// exists to resolve, and confusing the two is how a stack of catalog
    /// rows gets deleted because a cable was loose.
    #[error("source '{name}' is not available (expected at {path})")]
    SourceOffline {
        /// The source's display name, as `sources.name` holds it.
        name: String,
        /// Where the source is expected to be.
        path: String,
    },

    /// A folder offered as a source's new location does not hold the photos
    /// that source is catalogued with.
    ///
    /// Raised by [`Session::relocate_source`](crate::Session::relocate_source)
    /// when a sampled file is absent or its bytes differ. Naming the file is
    /// the whole point: "that is not the right folder" is unactionable, while
    /// "2026/ana/DSC_0042.jpg is not there" tells the photographer which drive
    /// they actually picked.
    #[error("{message}")]
    SourceMismatch {
        /// Human-readable explanation, naming the source and the file.
        message: String,
        /// The sampled file, relative to the source root.
        file: String,
    },

    /// Sync found divergent changes on both sides; the caller must resolve.
    ///
    /// Note that the sync engine does not currently raise this. A three-way
    /// reconciliation names conflicting files in `SyncOutcome::conflicts` and
    /// keeps going with the rest of the album, which is the more useful shape —
    /// one contested photograph must not strand the other four hundred. The
    /// variant and its `"sync-conflict"` tag are kept for a caller that wants to
    /// refuse the whole transfer instead.
    #[error("sync conflict for {entity}: changed locally and remotely")]
    SyncConflict {
        /// What diverged — an album path, or a file path within one.
        entity: String,
    },

    /// Anything without a variant of its own, tagged `"other"` over the FFI.
    ///
    /// Because it is untagged, a foreign client can only show the message, so
    /// the message has to stand alone: "no library is open", "this library's
    /// catalog is schema v4, and this build understands v3 — open it with a
    /// newer version of the app". If callers would branch on it, it wants a
    /// variant instead.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Attach a path to an I/O failure. The preferred way to build one:
    /// `std::fs::read(&p).map_err(|e| Error::io(&p, e))`.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// A message-only failure. The message is the entire diagnosis a foreign
    /// caller receives — write it for the photographer, not for the log.
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
