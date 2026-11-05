//! The unencrypted `account_id` marker file.
//!
//! The marker holds only an opaque local UUID (not a secret, not the NodeId).
//! Its presence is what `identity_status` keys off, so writes are atomic.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque local account identifier. Not a secret and not the NodeId.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountId(Uuid);

impl AccountId {
    /// Generates a fresh random (v4) account id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Returns the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from reading or writing the marker file.
#[derive(Debug)]
pub enum MarkerError {
    /// Underlying filesystem failure.
    Io(io::Error),
    /// The marker exists but is not a canonical UUID.
    Corrupt,
}

impl fmt::Display for MarkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarkerError::Io(e) => write!(f, "marker I/O error: {e}"),
            MarkerError::Corrupt => write!(f, "account marker is corrupt"),
        }
    }
}

impl std::error::Error for MarkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MarkerError::Io(e) => Some(e),
            MarkerError::Corrupt => None,
        }
    }
}

impl From<io::Error> for MarkerError {
    fn from(e: io::Error) -> Self {
        MarkerError::Io(e)
    }
}

/// Reads the marker at `path`. Absent file is `Ok(None)`; a present but
/// non-canonical file is `Err(MarkerError::Corrupt)`.
pub fn read_marker(path: &Path) -> Result<Option<AccountId>, MarkerError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(MarkerError::Io(e)),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| MarkerError::Corrupt)?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    // Canonical form only: 36 chars, lowercase hyphenated.
    if text.len() != 36 {
        return Err(MarkerError::Corrupt);
    }
    let uuid = Uuid::parse_str(text).map_err(|_| MarkerError::Corrupt)?;
    if uuid.hyphenated().to_string() != text {
        return Err(MarkerError::Corrupt);
    }
    Ok(Some(AccountId(uuid)))
}

/// Atomically writes the marker: temp file in the same directory, fsync,
/// then rename into place.
pub fn write_marker(path: &Path, id: AccountId) -> Result<(), MarkerError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let file_name = path.file_name().ok_or_else(|| {
        MarkerError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "marker path has no file name",
        ))
    })?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = dir.join(tmp_name);

    let result = (|| -> io::Result<()> {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(id.0.hyphenated().to_string().as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp_path, path)?;
        // Best-effort directory fsync so the rename is durable (unix).
        #[cfg(unix)]
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result.map_err(MarkerError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_marker_reads_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account_id");
        assert!(read_marker(&path).unwrap().is_none());
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account_id");
        let id = AccountId::new();
        write_marker(&path, id).unwrap();
        assert_eq!(read_marker(&path).unwrap(), Some(id));
        // Trailing newline tolerated.
        fs::write(&path, format!("{}\n", id.as_uuid())).unwrap();
        assert_eq!(read_marker(&path).unwrap(), Some(id));
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account_id");
        write_marker(&path, AccountId::new()).unwrap();
        write_marker(&path, AccountId::new()).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("account_id")]);
    }

    #[test]
    fn corrupt_marker_errors_not_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account_id");
        let id = AccountId::new().as_uuid();
        let bad: Vec<Vec<u8>> = vec![
            b"garbage".to_vec(),
            vec![],
            vec![0xff, 0xfe, 0x00],
            format!("{}\n\n", id).into_bytes(),
            format!(" {}", id).into_bytes(),
            id.simple().to_string().into_bytes(),
            id.hyphenated().to_string().to_uppercase().into_bytes(),
            format!("urn:uuid:{}", id).into_bytes(),
        ];
        for content in bad {
            fs::write(&path, &content).unwrap();
            assert!(
                matches!(read_marker(&path), Err(MarkerError::Corrupt)),
                "should be corrupt: {content:?}"
            );
        }
    }
}
