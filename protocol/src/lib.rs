//! Wire protocol shared across every layer of the p2p-messenger.
//!
//! Every frame on the wire is an [`Envelope`] carrying a protocol [`version`]
//! and a [`Payload`]. Frames are serialized with [`postcard`], a compact,
//! `#[no_std]`-friendly binary format. The `version` field lets a receiver
//! reject or adapt to frames from an incompatible protocol revision instead of
//! mis-decoding them.
//!
//! This crate is intentionally transport- and UI-agnostic: it knows nothing
//! about iroh, keychains, or storage. It defines *what* is sent, not *how*.
//!
//! [`version`]: Envelope::version

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current protocol revision. Bump on any breaking change to the wire format.
pub const PROTOCOL_VERSION: u16 = 1;

/// Application-layer protocol identifier negotiated on each iroh/QUIC
/// connection, so peers only speak to others running this messenger.
pub const ALPN: &[u8] = b"p2p-messenger/1";

/// Stable, globally-unique identifier for a single chat message.
///
/// Used to correlate a sent message with its [`Ack`] and to deduplicate on the
/// receiving side. A v4 (random) UUID; generate one with [`new_message_id`].
pub type MessageId = Uuid;

/// Generate a fresh random [`MessageId`].
pub fn new_message_id() -> MessageId {
    Uuid::new_v4()
}

/// The top-level unit of exchange on the wire.
///
/// Construct with [`Envelope::new`] so the current [`PROTOCOL_VERSION`] is
/// stamped automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol revision this frame was produced with.
    pub version: u16,
    /// The frame's contents.
    pub payload: Payload,
}

impl Envelope {
    /// Wrap a payload in an envelope stamped with [`PROTOCOL_VERSION`].
    pub fn new(payload: Payload) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            payload,
        }
    }

    /// Whether this frame's version is one this build knows how to interpret.
    pub fn is_supported(&self) -> bool {
        self.version == PROTOCOL_VERSION
    }

    /// Serialize to postcard bytes, ready to send on the wire.
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserialize a frame from postcard bytes received off the wire.
    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

/// Everything that can travel inside an [`Envelope`].
///
/// New variants are additive; a peer on an older protocol version will reject
/// frames it cannot decode via [`Envelope::is_supported`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Payload {
    /// First frame after a connection opens: introduces the sender.
    Hello(Hello),
    /// A plain-text chat message.
    Chat(ChatMessage),
    /// Delivery acknowledgement for a previously received [`ChatMessage`].
    Ack(Ack),
}

/// Sender introduction sent immediately after a connection is established.
///
/// Carries the self-chosen display name shown in the Accept/Decline prompt for
/// unknown peers. The peer's verified identity is its iroh public key (NodeId),
/// established by the transport — never this field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Human-readable name the sender chose for themselves. Unverified.
    pub display_name: String,
}

/// A single plain-text chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique id, echoed back in the matching [`Ack`] to mark it delivered.
    pub id: MessageId,
    /// Sender's wall-clock send time, in milliseconds since the Unix epoch.
    pub sent_at_ms: i64,
    /// Message body: plain UTF-8 text (multi-line and emoji permitted).
    pub body: String,
}

/// Acknowledges receipt of a [`ChatMessage`], moving its status to *Delivered*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    /// Id of the [`ChatMessage`] being acknowledged.
    pub id: MessageId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_envelope_round_trips() {
        let original = Envelope::new(Payload::Chat(ChatMessage {
            id: new_message_id(),
            sent_at_ms: 1_730_000_000_000,
            body: "hello 👋\nsecond line".to_owned(),
        }));

        let bytes = original.encode().expect("encode");
        let decoded = Envelope::decode(&bytes).expect("decode");

        assert_eq!(original, decoded);
        assert!(decoded.is_supported());
    }

    #[test]
    fn hello_and_ack_round_trip() {
        let id = new_message_id();
        for payload in [
            Payload::Hello(Hello {
                display_name: "Alice".to_owned(),
            }),
            Payload::Ack(Ack { id }),
        ] {
            let env = Envelope::new(payload);
            let decoded = Envelope::decode(&env.encode().unwrap()).unwrap();
            assert_eq!(env, decoded);
        }
    }

    #[test]
    fn newer_protocol_version_is_rejected() {
        let env = Envelope {
            version: PROTOCOL_VERSION + 1,
            payload: Payload::Ack(Ack {
                id: new_message_id(),
            }),
        };
        // Still decodes structurally, but is flagged as unsupported.
        let decoded = Envelope::decode(&env.encode().unwrap()).unwrap();
        assert!(!decoded.is_supported());
    }
}
