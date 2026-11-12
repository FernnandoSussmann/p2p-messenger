//! The `messaging` slice: the durable message store and the delivery engine.
//!
//! - **Engine.** One `Envelope` per QUIC stream over an authorized discovery
//!   channel; the `Ack` returns on the same stream. Inbound frames are read
//!   concurrently but persisted in accept order; outgoing messages go
//!   Sending, then Delivered or Pending, then Failed. Messaging never dials:
//!   re-establishing a channel is discovery's job.
//! - **Storage.** [`MessageStore`] is backed by one encrypted `messages` table;
//!   the table lives in `storage`'s single DB as migration 1.
//! - **Bodies live only inside the SQLCipher file**, and every row and every
//!   query is scoped by `account_id`.
//! - **Dedupe is a storage invariant**: `UNIQUE(account_id, peer, message_id)`.
//! - **History order is `(sent_at_ms, seq)`.** `sent_at_ms` is sender-controlled
//!   and is used for display and ordering only, never for a unique key or a
//!   security decision; `seq` (local arrival order) breaks ties.

mod codec;
mod config;
#[cfg(test)]
mod contract;
mod display;
mod driver;
mod dto;
mod engine;
mod events;
mod inbound;
mod model;
mod outbox;
mod service;
mod sqlcipher;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use codec::{decode_ack, decode_frame, encode_ack, encode_chat, Frame, FrameError};
pub use config::{
    MessagingConfig, ACK_TIMEOUT, BACKOFF_BASE, BACKOFF_MAX, EVENT_CAPACITY, FRAME_READ_TIMEOUT,
    MAX_ATTEMPTS, MAX_INBOUND_PER_PEER, MAX_IN_FLIGHT_PER_PEER, MAX_OUTBOX,
};
pub use display::neutralize_chat_text;
pub use dto::{
    chat_event_json, HistoryPageDto, MessageDto, MessageStatusDto, CHAT_RESYNC, MESSAGE_RECEIVED,
    MESSAGE_STATUS,
};
pub use events::ChatEvent;
pub use model::{
    validate_body, HistoryCursor, Insert, MessageDirection, MessageStatus, MessageStoreError,
    StoredMessage,
};
pub use protocol::{MessageId, MAX_BODY_BYTES};
pub use service::{Messaging, MessagingDeps, SendError};
pub use sqlcipher::SqlCipherMessageStore;
pub(crate) use sqlcipher::MIGRATION_MESSAGES_V1;
pub use store::{
    history_page, HistoryPage, MessageStore, DEFAULT_PAGE, DEFAULT_SEARCH_RESULTS, MAX_PAGE,
    MAX_SEARCH_QUERY_CHARS, MAX_SEARCH_RESULTS,
};

/// Parses a message id from external input: at most 64 bytes, then a UUID.
pub fn parse_message_id(s: &str) -> Option<MessageId> {
    if s.len() > 64 {
        return None;
    }
    MessageId::parse_str(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_id_bounds_input() {
        let id = protocol::new_message_id();
        assert_eq!(parse_message_id(&id.to_string()), Some(id));
        assert_eq!(parse_message_id("garbage"), None);
        assert_eq!(parse_message_id(&"0".repeat(65)), None);
    }
}
