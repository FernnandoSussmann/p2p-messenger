use std::fmt;
use std::path::PathBuf;

use tracing::level_filters::LevelFilter;

pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;
pub const MAX_LINE_BYTES: usize = 8 * 1024;
pub const DEFAULT_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const DEFAULT_MAX_FILES: usize = 5;

#[derive(Debug, Clone)]
pub struct LogConfig {
    /// OS app log directory (profile-namespaced by the app).
    pub dir: PathBuf,
    /// Level for our crates; third-party crates are capped at WARN.
    pub level: LevelFilter,
    pub max_file_bytes: u64,
    /// Active file plus rotated ones.
    pub max_files: usize,
}

impl LogConfig {
    pub fn new(dir: impl Into<PathBuf>, level: LevelFilter) -> Self {
        Self {
            dir: dir.into(),
            level,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_files: DEFAULT_MAX_FILES,
        }
    }
}

/// The `MESSENGER_LOG` value is not a known level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogConfigError;

impl fmt::Display for LogConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid MESSENGER_LOG: expected off, error, warn, info, debug or trace")
    }
}

impl std::error::Error for LogConfigError {}

/// `MESSENGER_LOG`: off|error|warn|info|debug|trace (case-insensitive).
/// Unset is [`DEFAULT_LEVEL`]; anything else is an error.
pub fn level_from_env(raw: Option<&str>) -> Result<LevelFilter, LogConfigError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_LEVEL);
    };
    match raw.to_ascii_lowercase().as_str() {
        "off" => Ok(LevelFilter::OFF),
        "error" => Ok(LevelFilter::ERROR),
        "warn" => Ok(LevelFilter::WARN),
        "info" => Ok(LevelFilter::INFO),
        "debug" => Ok(LevelFilter::DEBUG),
        "trace" => Ok(LevelFilter::TRACE),
        _ => Err(LogConfigError),
    }
}
