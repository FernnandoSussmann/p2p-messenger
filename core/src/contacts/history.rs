//! Seam for deleting a peer's stored conversation when a contact is removed.

use std::fmt;

use crate::identity::{AccountId, NodeId};

/// Removes the peer's rows and FTS postings, then reclaims pages. Idempotent
/// (erasing nothing is Ok); all-or-nothing for rows. Shared by `history_clear`
/// and "Remove contact, also delete history". Reclaim rewrites the whole file,
/// so async callers must use `spawn_blocking`.
pub trait HistoryEraser: Send + Sync + 'static {
    fn erase_peer_history(
        &self,
        account: &AccountId,
        peer: &NodeId,
    ) -> Result<(), HistoryEraseError>;
}

/// Opaque: no paths, names or counts.
#[derive(Debug)]
pub struct HistoryEraseError;

impl fmt::Display for HistoryEraseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("history erase failed")
    }
}

impl std::error::Error for HistoryEraseError {}

/// Eraser for builds that persist no messages (the headless node and tests).
/// The desktop app uses `messaging::SqlCipherMessageStore`.
pub struct NoMessageStore;

impl HistoryEraser for NoMessageStore {
    fn erase_peer_history(&self, _: &AccountId, _: &NodeId) -> Result<(), HistoryEraseError> {
        Ok(())
    }
}
