//! The [`MessageStore`] trait and the paging helper.

use protocol::ChatMessage;

use super::model::{HistoryCursor, Insert, MessageStatus, MessageStoreError, StoredMessage};
use crate::contacts::HistoryEraser;
use crate::identity::{AccountId, NodeId};

pub const DEFAULT_PAGE: usize = 50;
pub const MAX_PAGE: usize = 200;
pub const DEFAULT_SEARCH_RESULTS: usize = 50;
pub const MAX_SEARCH_RESULTS: usize = 100;
pub const MAX_SEARCH_QUERY_CHARS: usize = 128;

/// Blocking API; async callers use `spawn_blocking`, like contacts.
/// `erase_peer_history` removes the peer's rows and FTS postings, then reclaims
/// pages. Idempotent. All-or-nothing for rows.
pub trait MessageStore: HistoryEraser {
    /// New outgoing row (status Sending or Pending). Duplicate id for the peer
    /// is `AlreadyExists`.
    fn insert_outgoing(
        &self,
        account: &AccountId,
        peer: &NodeId,
        msg: &ChatMessage,
        status: MessageStatus,
    ) -> Result<StoredMessage, MessageStoreError>;

    /// New incoming row (status Received). An existing `(account, peer, id)` is
    /// `Ok(Insert::Duplicate)` and the row is untouched.
    fn insert_incoming(
        &self,
        account: &AccountId,
        peer: &NodeId,
        msg: &ChatMessage,
        received_at_ms: i64,
    ) -> Result<Insert, MessageStoreError>;

    /// Outgoing rows only. Delivered is terminal (never overwritten). `Received`
    /// is refused with `InvalidQuery`. `Ok(false)` if there is no such row, it is
    /// not outgoing, or it is already Delivered.
    fn set_status(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &protocol::MessageId,
        status: MessageStatus,
    ) -> Result<bool, MessageStoreError>;

    fn get(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &protocol::MessageId,
    ) -> Result<Option<StoredMessage>, MessageStoreError>;

    /// Rows strictly before `before` in `(sent_at_ms, seq)` order: the newest
    /// `limit` such rows, returned oldest to newest.
    fn page(
        &self,
        account: &AccountId,
        peer: &NodeId,
        before: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError>;

    /// Substring search over bodies, newest first by `(sent_at_ms, seq)`, scoped
    /// to the account (and the peer if given).
    ///
    /// Case folding: ASCII case-insensitive is guaranteed by every
    /// implementation. Non-ASCII case folding is not: the trigram path may fold
    /// some non-ASCII letters, while the `LIKE` path and `MemoryMessageStore` do
    /// not. Non-ASCII text is always found when the query has the same case.
    fn search(
        &self,
        account: &AccountId,
        peer: Option<&NodeId>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError>;

    /// Outgoing rows with status Sending or Pending, oldest first by `seq`, at
    /// most `limit`.
    fn undelivered(
        &self,
        account: &AccountId,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    pub messages: Vec<StoredMessage>,
    pub next_before: Option<HistoryCursor>,
}

/// Clamps `limit` to `1..=MAX_PAGE` (default `DEFAULT_PAGE`). `next_before` is
/// the cursor of the oldest returned row when the page is full, else `None`.
pub fn history_page(
    store: &dyn MessageStore,
    account: &AccountId,
    peer: &NodeId,
    before: Option<HistoryCursor>,
    limit: Option<usize>,
) -> Result<HistoryPage, MessageStoreError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let messages = store.page(account, peer, before, limit)?;
    let next_before = if messages.len() == limit {
        messages.first().map(HistoryCursor::of)
    } else {
        None
    };
    Ok(HistoryPage {
        messages,
        next_before,
    })
}

/// Normalises a search query: trimmed, non-empty, at most
/// `MAX_SEARCH_QUERY_CHARS` code points.
pub(crate) fn clean_query(query: &str) -> Result<&str, MessageStoreError> {
    let q = query.trim();
    if q.is_empty() || q.chars().nth(MAX_SEARCH_QUERY_CHARS).is_some() {
        return Err(MessageStoreError::InvalidQuery);
    }
    Ok(q)
}
