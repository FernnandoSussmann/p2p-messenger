//! Diagnostic logging: a `tracing-subscriber` pipeline that writes to a
//! size-bounded rotating file in the OS app log directory. There is no
//! network sink.
//!
//! **Logged:** connection lifecycle, hole-punch and path events, relay
//! status, discovery events and errors, identified by short NodeId and
//! address.
//!
//! **Never logged:** message bodies, search queries, display names and
//! nicknames, the identity key, the database key and ticket strings.
//! `core/tests/log_redaction_guard.rs` enforces this over every log call in
//! the workspace; log calls use [`fields`] helpers, enums and counters only.
//!
//! Log files reveal who you connected to and when. Deleting the log folder is
//! safe at any time.

mod config;
pub mod fields;
#[cfg(test)]
mod tests;
mod writer;

use std::fmt;
use std::io;
use std::sync::Mutex;

use tracing::level_filters::LevelFilter;
use tracing::Subscriber;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{fmt as tracing_fmt, Layer};

pub use config::{
    level_from_env, LogConfig, LogConfigError, DEFAULT_LEVEL, DEFAULT_MAX_FILES,
    DEFAULT_MAX_FILE_BYTES, MAX_LINE_BYTES,
};
use writer::RollingFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogInitError {
    /// The directory or file could not be created or was refused (symlink).
    Io,
    AlreadyInstalled,
}

impl fmt::Display for LogInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "the log file could not be opened",
            Self::AlreadyInstalled => "a global log subscriber is already installed",
        })
    }
}

impl std::error::Error for LogInitError {}

/// Our crates at `level`, everything else capped at WARN. `core::` and
/// `app_lib::` carry the separator because targets match by prefix and a bare
/// `core` would also match `core_foundation`.
pub fn filter(level: LevelFilter) -> Targets {
    let third_party = level.min(LevelFilter::WARN);
    Targets::new()
        .with_default(third_party)
        .with_target("core::", level)
        .with_target("app_lib::", level)
}

/// Builds the file-only subscriber (ANSI off) without installing it.
pub fn subscriber(
    config: &LogConfig,
) -> Result<impl Subscriber + Send + Sync + 'static, LogInitError> {
    let file = RollingFile::open(&config.dir, config.max_file_bytes, config.max_files)
        .map_err(|_: io::Error| LogInitError::Io)?;
    let layer = tracing_fmt::layer()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(true)
        .with_filter(filter(config.level));
    Ok(tracing_subscriber::registry().with(layer))
}

/// Installs the subscriber as the global default. `OFF` installs nothing and
/// creates no file.
pub fn init(config: &LogConfig) -> Result<(), LogInitError> {
    if config.level == LevelFilter::OFF {
        return Ok(());
    }
    tracing::subscriber::set_global_default(subscriber(config)?)
        .map_err(|_| LogInitError::AlreadyInstalled)
}
