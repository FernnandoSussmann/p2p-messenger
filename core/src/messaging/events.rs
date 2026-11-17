//! Events the messaging engine broadcasts to the app.

use protocol::MessageId;

use super::{MessageStatus, StoredMessage};
use crate::identity::NodeId;

#[derive(Clone)]
pub enum ChatEvent {
    /// A new inbound message was persisted (never emitted for duplicates).
    Received(StoredMessage),
    /// An outgoing message's status changed.
    Status {
        peer: NodeId,
        id: MessageId,
        status: MessageStatus,
        /// `Some` only when this transition made the row Delivered.
        delivered_at_ms: Option<i64>,
    },
}
