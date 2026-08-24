//! Goldplated Photos — core library.
//!
//! All application logic lives here: the SQLite catalog, the import pipeline,
//! album management, publishing to the Astro gallery, and the sync engine.
//!
//! # Portability contract
//!
//! This crate compiles unchanged for macOS, iPadOS, Windows and Linux. To keep
//! that true:
//!
//! - no GUI dependencies,
//! - no spawning external processes (no shelling out to `rsync`/`ssh`),
//! - anything platform-specific sits behind a trait implemented by the shell
//!   (see [`sync::RemoteTransport`] and [`media::RawDecoder`]).

pub mod albums;
pub mod catalog;
pub mod develop;
pub mod error;
pub mod import;
pub mod lightroom;
pub mod media;
pub mod model;
pub mod publish;
pub mod remote;
pub mod session;
pub mod sync;
pub mod xmp;

pub use error::{Error, Result};
pub use model::{Album, Flag, Photo, PhotoFilter, PhotoKind};

/// Library handle — the entry point for everything.
pub use catalog::Library;

/// Application session — one method per UI command. The desktop and mobile
/// shells are thin wrappers over this.
pub use session::Session;

#[cfg(test)]
mod contract {
    /// Anything holding a library from more than one thread — a server, a
    /// background import, a scheduled publish — needs these. They hold today
    /// because of what `Session` and `Library` are made of; this makes the
    /// next change that would take them away a compile error instead of a
    /// discovery in someone else's project.
    #[test]
    fn the_public_handles_cross_threads() {
        fn require<T: Send + Sync + 'static>() {}
        require::<crate::Session>();
        require::<crate::Library>();
        require::<crate::Error>();
    }
}
