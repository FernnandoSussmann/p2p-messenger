//! String-only DTOs and the single mapping to external event names.

use std::fmt;

use serde::Serialize;
use serde_json::{json, Value};

use super::{
    neutralize_chat_text, ChatEvent, HistoryCursor, HistoryPage, MessageDirection, MessageStatus,
    StoredMessage,
};

pub const MESSAGE_RECEIVED: &str = "message_received";
pub const MESSAGE_STATUS: &str = "message_status";
/// Emitted by the app when it lagged behind the event stream; the UI reloads.
pub const CHAT_RESYNC: &str = "chat_resync";

#[derive(Serialize)]
pub struct MessageDto {
    pub node_id: String,
    pub id: String,
    pub seq: i64,
    /// Opaque history cursor token: the UI never builds cursors.
    pub cursor: String,
    pub direction: &'static str,
    pub body: String,
    pub sent_at_ms: i64,
    pub received_at_ms: Option<i64>,
    pub delivered_at_ms: Option<i64>,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct MessageStatusDto {
    pub node_id: String,
    pub id: String,
    pub status: &'static str,
    pub delivered_at_ms: Option<i64>,
}

#[derive(Serialize)]
pub struct HistoryPageDto {
    pub messages: Vec<MessageDto>,
    pub next_before: Option<String>,
}

impl From<&StoredMessage> for MessageDto {
    fn from(row: &StoredMessage) -> Self {
        Self {
            node_id: row.peer.to_string(),
            id: row.id.to_string(),
            seq: row.seq,
            cursor: HistoryCursor::of(row).to_token(),
            direction: match row.direction {
                MessageDirection::Incoming => "in",
                MessageDirection::Outgoing => "out",
            },
            body: neutralize_chat_text(&row.body),
            sent_at_ms: row.sent_at_ms,
            received_at_ms: row.received_at_ms,
            delivered_at_ms: row.delivered_at_ms,
            status: row.status.as_str(),
        }
    }
}

impl From<&HistoryPage> for HistoryPageDto {
    fn from(page: &HistoryPage) -> Self {
        Self {
            messages: page.messages.iter().map(MessageDto::from).collect(),
            next_before: page.next_before.map(|c| c.to_token()),
        }
    }
}

impl MessageStatusDto {
    pub fn new(
        peer: &crate::identity::NodeId,
        id: &protocol::MessageId,
        status: MessageStatus,
        delivered_at_ms: Option<i64>,
    ) -> Self {
        Self {
            node_id: peer.to_string(),
            id: id.to_string(),
            status: status.as_str(),
            delivered_at_ms,
        }
    }
}

impl fmt::Debug for MessageDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageDto")
            .field("node_id", &self.node_id)
            .field("id", &self.id)
            .field("seq", &self.seq)
            .field("cursor", &self.cursor)
            .field("direction", &self.direction)
            .field(
                "body",
                &format_args!("<redacted, {} bytes>", self.body.len()),
            )
            .field("sent_at_ms", &self.sent_at_ms)
            .field("received_at_ms", &self.received_at_ms)
            .field("delivered_at_ms", &self.delivered_at_ms)
            .field("status", &self.status)
            .finish()
    }
}

impl fmt::Debug for HistoryPageDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryPageDto")
            .field("messages", &self.messages)
            .field("next_before", &self.next_before)
            .finish()
    }
}

/// Maps a [`ChatEvent`] to its external event name and JSON payload.
pub fn chat_event_json(ev: &ChatEvent) -> (&'static str, Value) {
    match ev {
        ChatEvent::Received(row) => (
            MESSAGE_RECEIVED,
            serde_json::to_value(MessageDto::from(row)).expect("dto serializes"),
        ),
        ChatEvent::Status {
            peer,
            id,
            status,
            delivered_at_ms,
        } => (
            MESSAGE_STATUS,
            json!(MessageStatusDto::new(peer, id, *status, *delivered_at_ms)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use protocol::new_message_id;

    use super::*;

    fn row(body: &str) -> StoredMessage {
        StoredMessage {
            seq: 3,
            peer: iroh_base::SecretKey::generate().public(),
            id: new_message_id(),
            direction: MessageDirection::Incoming,
            sent_at_ms: 1_000,
            received_at_ms: Some(2_000),
            body: body.to_owned(),
            status: MessageStatus::Received,
            delivered_at_ms: None,
        }
    }

    #[test]
    fn message_dto_json_shape() {
        let r = row("hi");
        let v = serde_json::to_value(MessageDto::from(&r)).unwrap();
        assert_eq!(
            v,
            json!({
                "node_id": r.peer.to_string(), "id": r.id.to_string(), "seq": 3,
                "cursor": "1000:3", "direction": "in", "body": "hi",
                "sent_at_ms": 1000, "received_at_ms": 2000, "delivered_at_ms": null,
                "status": "received",
            })
        );
        let mut out = r;
        out.direction = MessageDirection::Outgoing;
        out.received_at_ms = None;
        out.status = MessageStatus::Pending;
        let v = serde_json::to_value(MessageDto::from(&out)).unwrap();
        assert_eq!(v["direction"], "out");
        assert_eq!(v["received_at_ms"], Value::Null);
        assert_eq!(v["delivered_at_ms"], Value::Null);
        assert_eq!(v["status"], "pending");
        out.status = MessageStatus::Delivered;
        out.delivered_at_ms = Some(3_000);
        let v = serde_json::to_value(MessageDto::from(&out)).unwrap();
        assert_eq!(v["delivered_at_ms"], 3000);
    }

    #[test]
    fn history_page_dto_carries_cursor_token() {
        let r = row("a");
        let page = HistoryPage {
            messages: vec![r.clone()],
            next_before: Some(HistoryCursor::of(&r)),
        };
        let dto = HistoryPageDto::from(&page);
        assert_eq!(dto.messages.len(), 1);
        assert_eq!(dto.next_before.as_deref(), Some("1000:3"));
        let end = HistoryPage {
            messages: vec![],
            next_before: None,
        };
        assert_eq!(HistoryPageDto::from(&end).next_before, None);
    }

    #[test]
    fn message_status_dto_json_shape() {
        let r = row("a");
        let dto = MessageStatusDto::new(&r.peer, &r.id, MessageStatus::Delivered, Some(5));
        let none = MessageStatusDto::new(&r.peer, &r.id, MessageStatus::Sending, None);
        assert_eq!(
            serde_json::to_value(&none).unwrap()["delivered_at_ms"],
            Value::Null
        );
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"node_id": r.peer.to_string(), "id": r.id.to_string(), "status": "delivered", "delivered_at_ms": 5})
        );
    }

    #[test]
    fn chat_event_json_names() {
        let r = row("a");
        let (name, v) = chat_event_json(&ChatEvent::Received(r.clone()));
        assert_eq!(name, "message_received");
        assert_eq!(v["id"], r.id.to_string());
        let (name, v) = chat_event_json(&ChatEvent::Status {
            peer: r.peer,
            id: r.id,
            status: MessageStatus::Failed,
            delivered_at_ms: None,
        });
        assert_eq!(name, "message_status");
        assert_eq!(v["status"], "failed");
        assert_eq!(v["delivered_at_ms"], Value::Null);
        let (_, v) = chat_event_json(&ChatEvent::Status {
            peer: r.peer,
            id: r.id,
            status: MessageStatus::Delivered,
            delivered_at_ms: Some(9),
        });
        assert_eq!(v["delivered_at_ms"], 9);
        assert_eq!(CHAT_RESYNC, "chat_resync");
    }

    #[test]
    fn message_dto_body_is_neutralized() {
        let dto = MessageDto::from(&row("a\u{202E}b\u{200B}c"));
        assert_eq!(dto.body, "a\u{FFFD}b\u{FFFD}c");
    }

    #[test]
    fn message_dto_debug_redacts_body() {
        let page = HistoryPage {
            messages: vec![row("top secret text")],
            next_before: None,
        };
        let dto = HistoryPageDto::from(&page);
        let shown = format!("{:?} {:?}", dto, dto.messages[0]);
        assert!(!shown.contains("top secret"));
        assert!(shown.contains("redacted"));
        assert!(shown.contains("delivered_at_ms"));
    }
}
