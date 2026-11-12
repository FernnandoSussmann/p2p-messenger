//! Seam for deleting a peer's stored conversation when a contact is removed.

use std::fmt;

use crate::identity::{AccountId, NodeId};

/// Deletes a peer's stored conversation. DD-05 owns the real implementation
/// (same path as `history_clear`, with page reclaim). Contract: idempotent
/// (erasing nothing is Ok) and all-or-nothing.
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
