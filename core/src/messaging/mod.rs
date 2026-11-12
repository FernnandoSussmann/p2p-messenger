//! The `messaging` slice: the durable message store.
//!
//! - **DD-05 seam.** [`MessageStore`] is the minimal, clearly-labelled seam for
//!   DD-05: one encrypted `messages` table and nothing else. DD-05 later moves
//!   the table into its single database and migration framework.
//! - **Bodies live only inside the SQLCipher file**, and every row and every
//!   query is scoped by `account_id`.
//! - **Dedupe is a storage invariant**: `UNIQUE(account_id, peer, message_id)`.
//! - **History order is `(sent_at_ms, seq)`.** `sent_at_ms` is sender-controlled
//!   and is used for display and ordering only, never for a unique key or a
//!   security decision; `seq` (local arrival order) breaks ties.

#[cfg(test)]
mod contract;
mod model;
mod sqlcipher;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use model::{
    validate_body, HistoryCursor, Insert, MessageDirection, MessageStatus, MessageStoreError,
    StoredMessage,
};
pub use sqlcipher::{open_message_store, SqlCipherMessageStore, DB_FILE};
pub use store::{
    history_page, HistoryPage, MessageStore, DEFAULT_PAGE, DEFAULT_SEARCH_RESULTS, MAX_PAGE,
    MAX_SEARCH_QUERY_CHARS, MAX_SEARCH_RESULTS,
};
