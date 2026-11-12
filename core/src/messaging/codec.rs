//! Frame codec for the messaging streams: one [`Envelope`] per stream.

use protocol::{
    peek_version, Ack, ChatMessage, Envelope, MessageId, Payload, MAX_ACK_BYTES, MAX_BODY_BYTES,
    MAX_FRAME_BYTES, PROTOCOL_VERSION,
};

/// A decoded inbound frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Chat(ChatMessage),
    Ack(MessageId),
    /// Well-formed but not for us (future variant, stray Hello, blank body).
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    TooLarge,
    UnsupportedVersion,
    Malformed,
    EmptyBody,
}

fn check_body(body: &str) -> Result<(), FrameError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(FrameError::TooLarge);
    }
    if body.trim().is_empty() {
        return Err(FrameError::EmptyBody);
    }
    Ok(())
}

/// Validates the body (`EmptyBody` / `TooLarge`) and that the frame fits `MAX_FRAME_BYTES`.
pub fn encode_chat(msg: &ChatMessage) -> Result<Vec<u8>, FrameError> {
    check_body(&msg.body)?;
    let bytes = Envelope::new(Payload::Chat(msg.clone()))
        .encode()
        .map_err(|_| FrameError::Malformed)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    Ok(bytes)
}

pub fn encode_ack(id: MessageId) -> Vec<u8> {
    Envelope::new(Payload::Ack(Ack { id }))
        .encode()
        .expect("an ack always encodes")
}

/// Order is binding: size, `peek_version` (unsupported is never decoded), then decode.
/// A supported-version payload that does not decode is `Ok(Frame::Ignored)`.
pub fn decode_frame(bytes: &[u8]) -> Result<Frame, FrameError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    match peek_version(bytes) {
        Some(PROTOCOL_VERSION) => {}
        Some(_) => return Err(FrameError::UnsupportedVersion),
        None => return Err(FrameError::Malformed),
    }
    let Ok(env) = Envelope::decode(bytes) else {
        return Ok(Frame::Ignored);
    };
    match env.payload {
        Payload::Chat(msg) => match check_body(&msg.body) {
            Ok(()) => Ok(Frame::Chat(msg)),
            Err(FrameError::EmptyBody) => Ok(Frame::Ignored),
            Err(e) => Err(e),
        },
        Payload::Ack(ack) => Ok(Frame::Ack(ack.id)),
        Payload::Hello(_) => Ok(Frame::Ignored),
    }
}

/// A reply on our own stream: at most `MAX_ACK_BYTES`, supported version, `Payload::Ack`.
pub fn decode_ack(bytes: &[u8]) -> Result<MessageId, FrameError> {
    if bytes.len() > MAX_ACK_BYTES {
        return Err(FrameError::TooLarge);
    }
    if peek_version(bytes) != Some(PROTOCOL_VERSION) {
        return Err(FrameError::UnsupportedVersion);
    }
    match Envelope::decode(bytes).map(|e| e.payload) {
        Ok(Payload::Ack(ack)) => Ok(ack.id),
        _ => Err(FrameError::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use protocol::{new_message_id, Hello};

    use super::*;

    fn chat(body: &str) -> ChatMessage {
        ChatMessage {
            id: new_message_id(),
            sent_at_ms: 42,
            body: body.to_owned(),
        }
    }

    #[test]
    fn chat_round_trips_multiline_emoji() {
        let msg = chat("hi 👋\nsecond line");
        let bytes = encode_chat(&msg).unwrap();
        assert_eq!(decode_frame(&bytes), Ok(Frame::Chat(msg)));
    }

    #[test]
    fn encode_rejects_empty_and_oversized_body() {
        assert_eq!(encode_chat(&chat("  \n")), Err(FrameError::EmptyBody));
        assert_eq!(
            encode_chat(&chat(&"a".repeat(MAX_BODY_BYTES + 1))),
            Err(FrameError::TooLarge)
        );
        assert!(encode_chat(&chat(&"a".repeat(MAX_BODY_BYTES))).is_ok());
    }

    #[test]
    fn decode_rejects_oversized_frame() {
        assert_eq!(
            decode_frame(&vec![1; MAX_FRAME_BYTES + 1]),
            Err(FrameError::TooLarge)
        );
    }

    #[test]
    fn body_over_limit_inside_fitting_frame_is_rejected() {
        let bytes = Envelope::new(Payload::Chat(chat(&"a".repeat(MAX_BODY_BYTES + 1))))
            .encode()
            .unwrap();
        assert!(bytes.len() <= MAX_FRAME_BYTES);
        assert_eq!(decode_frame(&bytes), Err(FrameError::TooLarge));
    }

    #[test]
    fn unsupported_version_is_not_decoded() {
        let bytes = Envelope {
            version: PROTOCOL_VERSION + 1,
            payload: Payload::Chat(chat("x")),
        }
        .encode()
        .unwrap();
        assert_eq!(decode_frame(&bytes), Err(FrameError::UnsupportedVersion));
        assert_eq!(
            decode_frame(&[PROTOCOL_VERSION as u8 + 1, 0xFF, 0xFF]),
            Err(FrameError::UnsupportedVersion)
        );
        assert_eq!(decode_frame(&[]), Err(FrameError::Malformed));
    }

    #[test]
    fn unknown_variant_is_ignored() {
        assert_eq!(decode_frame(&[0x01, 0x07, 0x00]), Ok(Frame::Ignored));
        assert_eq!(
            decode_frame(&encode_chat(&chat("x")).unwrap()[..3]),
            Ok(Frame::Ignored)
        );
    }

    #[test]
    fn hello_on_messaging_stream_is_ignored() {
        let bytes = Envelope::new(Payload::Hello(Hello {
            display_name: "Eve".to_owned(),
        }))
        .encode()
        .unwrap();
        assert_eq!(decode_frame(&bytes), Ok(Frame::Ignored));
    }

    #[test]
    fn ack_round_trips_through_both_decoders() {
        let id = new_message_id();
        assert_eq!(decode_ack(&encode_ack(id)), Ok(id));
        assert_eq!(decode_frame(&encode_ack(id)), Ok(Frame::Ack(id)));
    }

    #[test]
    fn decode_ack_rejects_non_ack_and_oversized() {
        let chat_bytes = encode_chat(&chat("x")).unwrap();
        assert_eq!(decode_ack(&chat_bytes), Err(FrameError::Malformed));
        assert_eq!(
            decode_ack(&[1; MAX_ACK_BYTES + 1]),
            Err(FrameError::TooLarge)
        );
        let newer = Envelope {
            version: PROTOCOL_VERSION + 1,
            payload: Payload::Ack(Ack {
                id: new_message_id(),
            }),
        }
        .encode()
        .unwrap();
        assert_eq!(decode_ack(&newer), Err(FrameError::UnsupportedVersion));
    }
}
