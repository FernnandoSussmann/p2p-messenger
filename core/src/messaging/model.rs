//! Stored-message model, the cursor and body validation.

use std::fmt;

use protocol::{MessageId, MAX_BODY_BYTES};

use crate::identity::NodeId;

/// Stored as `'in'` / `'out'`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

impl MessageDirection {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "in" => Some(Self::Incoming),
            "out" => Some(Self::Outgoing),
            _ => None,
        }
    }
}

/// Mapped to/from TEXT in Rust only (`as_str` / `parse`); the table has no CHECK
/// on it, so a future status (e.g. store-and-forward "queued") needs no table
/// rebuild. An unknown value read back is treated as corruption (`Unavailable`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageStatus {
    Sending,
    Pending,
    Delivered,
    Failed,
    Received,
}

impl MessageStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Sending => "sending",
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::Received => "received",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "sending" => Some(Self::Sending),
            "pending" => Some(Self::Pending),
            "delivered" => Some(Self::Delivered),
            "failed" => Some(Self::Failed),
            "received" => Some(Self::Received),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StoredMessage {
    /// Local arrival/insertion order: the tie-breaker.
    pub seq: i64,
    pub peer: NodeId,
    pub id: MessageId,
    pub direction: MessageDirection,
    /// Sender-controlled: display and primary sort key only.
    pub sent_at_ms: i64,
    /// `Some` for incoming rows only.
    pub received_at_ms: Option<i64>,
    pub body: String,
    /// `Received` iff incoming (enforced in Rust).
    pub status: MessageStatus,
    /// This device's clock when the matching `Ack` was processed. `Some` only for
    /// outgoing rows that became Delivered after DD-08; never peer-supplied.
    pub delivered_at_ms: Option<i64>,
}

impl fmt::Debug for StoredMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredMessage")
            .field("seq", &self.seq)
            .field("peer", &self.peer)
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("sent_at_ms", &self.sent_at_ms)
            .field("received_at_ms", &self.received_at_ms)
            .field(
                "body",
                &format_args!("<redacted, {} bytes>", self.body.len()),
            )
            .field("status", &self.status)
            .field("delivered_at_ms", &self.delivered_at_ms)
            .finish()
    }
}

/// Keyset cursor over `(sent_at_ms, seq)`. Crosses the IPC boundary only as an
/// opaque token `"<sent_at_ms>:<seq>"` (decimal), because a JS number cannot
/// carry every i64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryCursor {
    pub sent_at_ms: i64,
    pub seq: i64,
}

const MAX_TOKEN_BYTES: usize = 48;

impl HistoryCursor {
    pub fn of(row: &StoredMessage) -> Self {
        Self {
            sent_at_ms: row.sent_at_ms,
            seq: row.seq,
        }
    }

    pub fn to_token(&self) -> String {
        format!("{}:{}", self.sent_at_ms, self.seq)
    }

    /// At most 48 bytes, exactly one ':', both parts parse as i64; else `None`.
    pub fn parse_token(s: &str) -> Option<Self> {
        if s.len() > MAX_TOKEN_BYTES {
            return None;
        }
        let (sent, seq) = s.split_once(':')?;
        if seq.contains(':') {
            return None;
        }
        Some(Self {
            sent_at_ms: sent.parse().ok()?,
            seq: seq.parse().ok()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Insert {
    Inserted(StoredMessage),
    Duplicate,
}

/// Failure classes only. `Display` never names a path, key, id or body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageStoreError {
    Unavailable,
    AlreadyExists,
    InvalidBody,
    InvalidQuery,
}

impl fmt::Display for MessageStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "message store unavailable",
            Self::AlreadyExists => "message already exists",
            Self::InvalidBody => "invalid message body",
            Self::InvalidQuery => "invalid query",
        })
    }
}

impl std::error::Error for MessageStoreError {}

/// Non-empty after trim and at most [`MAX_BODY_BYTES`] bytes.
pub fn validate_body(body: &str) -> Result<(), MessageStoreError> {
    if body.len() > MAX_BODY_BYTES || body.trim().is_empty() {
        return Err(MessageStoreError::InvalidBody);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use protocol::new_message_id;

    use super::*;

    #[test]
    fn history_cursor_token_round_trips() {
        for (sent_at_ms, seq) in [(i64::MIN, i64::MAX), (i64::MAX, i64::MIN), (0, 1), (-5, 7)] {
            let c = HistoryCursor { sent_at_ms, seq };
            let token = c.to_token();
            assert!(token.len() <= MAX_TOKEN_BYTES);
            assert_eq!(HistoryCursor::parse_token(&token), Some(c));
        }
        for junk in [
            "",
            ":",
            "1",
            "1:",
            ":1",
            "a:b",
            "1:2:3",
            "1::2",
            " 1:2",
            "9223372036854775808:1",
            &format!("{}:1", "1".repeat(48)),
        ] {
            assert_eq!(HistoryCursor::parse_token(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn stored_message_debug_redacts_body() {
        let row = StoredMessage {
            seq: 1,
            peer: iroh_base::SecretKey::generate().public(),
            id: new_message_id(),
            direction: MessageDirection::Incoming,
            sent_at_ms: 2,
            received_at_ms: Some(3),
            body: format!("canary-{}", new_message_id()),
            status: MessageStatus::Received,
            delivered_at_ms: None,
        };
        let shown = format!("{row:?}");
        assert!(!shown.contains(&row.body));
        assert!(shown.contains(&format!("<redacted, {} bytes>", row.body.len())));
        assert!(shown.contains("delivered_at_ms"));
    }

    #[test]
    fn status_text_round_trips_and_unknown_is_rejected() {
        for s in [
            MessageStatus::Sending,
            MessageStatus::Pending,
            MessageStatus::Delivered,
            MessageStatus::Failed,
            MessageStatus::Received,
        ] {
            assert_eq!(MessageStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(MessageStatus::parse("queued"), None);
    }
}
