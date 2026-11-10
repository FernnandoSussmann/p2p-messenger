//! The trust seam: which NodeIds may connect without an Accept/Decline prompt.

use std::fmt;

use crate::identity::{DisplayName, NodeId};

/// Persistent set of peers the user has already accepted or deliberately
/// dialed.
pub trait KnownPeers: Send + Sync + 'static {
    /// Trust check, keyed on NodeId only. An `Err` must be treated as
    /// "unknown" (fail closed): the caller prompts instead of connecting
    /// silently.
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError>;

    /// Records an accepted or dialed peer. `display_name` is the last-seen
    /// self-asserted name: advisory only, never a key, never used for lookup
    /// or dedup.
    fn add(&self, id: NodeId, display_name: Option<DisplayName>) -> Result<(), KnownPeersError>;
}

/// Why a [`KnownPeers`] operation failed.
#[derive(Debug)]
pub enum KnownPeersError {
    /// The store is full.
    Full,
    /// The stored data is unreadable, corrupt or of an unknown version. It is
    /// never treated as empty.
    Corrupt,
    /// The backing storage could not be read or written.
    Io(std::io::Error),
}

impl fmt::Display for KnownPeersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("known peers store is full"),
            Self::Corrupt => f.write_str("known peers store is corrupt"),
            Self::Io(e) => write!(f, "known peers store I/O error: {e}"),
        }
    }
}

impl std::error::Error for KnownPeersError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
