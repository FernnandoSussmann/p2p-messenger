//! The first frame of every connection: a postcard `Envelope` carrying
//! `Payload::Hello`. The announced name is self-asserted and advisory only.

use std::fmt;

use protocol::{Envelope, Hello, Payload, MAX_HELLO_BYTES};

use crate::identity::{validate_display_name, DisplayName};

/// Why a received first frame was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloError {
    TooLarge,
    Malformed,
    UnsupportedVersion,
    NotHello,
}

impl fmt::Display for HelloError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "hello frame too large",
            Self::Malformed => "hello frame malformed",
            Self::UnsupportedVersion => "unsupported protocol version",
            Self::NotHello => "first frame is not a hello",
        })
    }
}

impl std::error::Error for HelloError {}

/// Encodes our Hello. A name that would not fit the frame budget is sent as
/// empty rather than failing the connection.
pub(crate) fn encode_hello(name: &str) -> Vec<u8> {
    let encode = |name: &str| {
        Envelope::new(Payload::Hello(Hello {
            display_name: name.to_owned(),
        }))
        .encode()
        .ok()
        .filter(|bytes| bytes.len() <= MAX_HELLO_BYTES)
    };
    encode(name).or_else(|| encode("")).unwrap_or_default()
}

/// Decodes a peer's Hello. An invalid or empty name is `Ok(None)`, never an
/// error: the connection is judged on the transport-verified NodeId, not on
/// what the peer calls itself.
pub(crate) fn decode_hello(bytes: &[u8]) -> Result<Option<DisplayName>, HelloError> {
    if bytes.len() > MAX_HELLO_BYTES {
        return Err(HelloError::TooLarge);
    }
    let envelope = Envelope::decode(bytes).map_err(|_| HelloError::Malformed)?;
    if !envelope.is_supported() {
        return Err(HelloError::UnsupportedVersion);
    }
    match envelope.payload {
        Payload::Hello(hello) => Ok(validate_display_name(&hello.display_name).ok()),
        _ => Err(HelloError::NotHello),
    }
}

#[cfg(test)]
mod tests {
    use protocol::{new_message_id, Ack, ChatMessage, PROTOCOL_VERSION};

    use super::*;

    #[test]
    fn valid_hello_round_trips() {
        let name = decode_hello(&encode_hello("Ada")).unwrap().unwrap();
        assert_eq!(name.as_str(), "Ada");
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let bytes = vec![0u8; MAX_HELLO_BYTES + 1];
        assert_eq!(decode_hello(&bytes), Err(HelloError::TooLarge));
    }

    #[test]
    fn oversized_name_is_sent_as_empty() {
        let bytes = encode_hello(&"x".repeat(MAX_HELLO_BYTES));
        assert!(bytes.len() <= MAX_HELLO_BYTES);
        assert_eq!(decode_hello(&bytes), Ok(None));
    }

    #[test]
    fn wrong_version_is_rejected() {
        let bytes = Envelope {
            version: PROTOCOL_VERSION + 1,
            payload: Payload::Hello(Hello {
                display_name: "Ada".into(),
            }),
        }
        .encode()
        .unwrap();
        assert_eq!(decode_hello(&bytes), Err(HelloError::UnsupportedVersion));
    }

    #[test]
    fn non_hello_payloads_are_rejected() {
        let chat = Envelope::new(Payload::Chat(ChatMessage {
            id: new_message_id(),
            sent_at_ms: 1,
            body: "hi".into(),
        }))
        .encode()
        .unwrap();
        assert_eq!(decode_hello(&chat), Err(HelloError::NotHello));
        let ack = Envelope::new(Payload::Ack(Ack {
            id: new_message_id(),
        }))
        .encode()
        .unwrap();
        assert_eq!(decode_hello(&ack), Err(HelloError::NotHello));
    }

    #[test]
    fn garbage_is_malformed() {
        assert_eq!(decode_hello(&[0xff; 12]), Err(HelloError::Malformed));
        assert_eq!(decode_hello(&[]), Err(HelloError::Malformed));
    }

    #[test]
    fn invalid_or_empty_name_is_none() {
        assert_eq!(decode_hello(&encode_hello("")), Ok(None));
        assert_eq!(decode_hello(&encode_hello("   ")), Ok(None));
        assert_eq!(decode_hello(&encode_hello("a\u{202e}b")), Ok(None));
        assert_eq!(decode_hello(&encode_hello("a\nb")), Ok(None));
    }
}
