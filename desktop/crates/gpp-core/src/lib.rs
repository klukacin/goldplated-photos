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
pub mod error;
pub mod import;
pub mod media;
pub mod model;
pub mod publish;
pub mod sync;

pub use error::{Error, Result};
pub use model::{Album, Flag, Photo, PhotoFilter, PhotoKind};

/// Library handle — the entry point for everything.
pub use catalog::Library;
