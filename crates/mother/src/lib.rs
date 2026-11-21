#[cfg(not(unix))]
compile_error!("Mother currently requires macOS or Linux Unix domain sockets");

pub mod config;
pub mod continuity;
mod evidence;
pub mod mcp;
pub mod personas;
pub mod protocol;
pub mod runtime;
pub mod store;
pub mod tasks;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Integrity(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Config(#[from] toml::de::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
