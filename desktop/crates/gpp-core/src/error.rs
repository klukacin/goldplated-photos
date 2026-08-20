//! Error type for the core library.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("i/o error: {0}")]
    PlainIo(#[from] std::io::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The path escapes the library root, or is otherwise not addressable.
    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("album not found: {0}")]
    AlbumNotFound(String),

    #[error("photo not found: {0}")]
    PhotoNotFound(String),

    #[error("album already exists: {0}")]
    AlbumExists(String),

    #[error("unsupported media type: {0}")]
    Unsupported(String),

    /// Sync found divergent changes on both sides; the caller must resolve.
    #[error("sync conflict for {entity}: changed locally and remotely")]
    SyncConflict { entity: String },

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
