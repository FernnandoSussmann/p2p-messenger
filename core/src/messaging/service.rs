//! The messaging engine's public API: send, resend, forget and the events.

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use protocol::{new_message_id, ChatMessage, MessageId, MAX_BODY_BYTES};
use tokio::sync::{broadcast, watch};

use super::config::{MessagingConfig, EVENT_CAPACITY};
use super::engine::{now_ms, Handler, Inner, State};
use super::outbox::{Entry, Outbox};
use super::StoredMessage;
use super::{ChatEvent, MessageDirection, MessageStatus, MessageStore, MessageStoreError};
use crate::discovery::{KnownPeers, Node};
use crate::identity::NodeId;

pub struct MessagingDeps {
    pub known: Arc<dyn KnownPeers>,
    pub store: Arc<dyn MessageStore>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    EmptyBody,
    BodyTooLarge,
    SelfMessage,
    NotAContact,
    ContactsUnavailable,
    OutboxFull,
    NotFound,
    NotResendable,
    StoreUnavailable,
    ShuttingDown,
}

/// The delivery engine. Never holds the `Node`; call [`Messaging::shutdown`]
/// before `Node::shutdown`.
pub struct Messaging {
    inner: Arc<Inner>,
}

impl Messaging {
    /// Binds to the node's account and NodeId, reloads the outbox from the
    /// store, then registers itself as the node's channel handler.
    pub async fn start(
        node: &Node,
        deps: MessagingDeps,
        config: MessagingConfig,
    ) -> Result<Messaging, MessageStoreError> {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let inner = Arc::new(Inner {
            account: node.account(),
            own: node.node_id(),
            known: deps.known,
            store: deps.store,
            events,
            state: Mutex::new(State {
                outbox: Outbox::new(config.max_outbox),
                channels: HashMap::new(),
            }),
            config,
            stop: watch::channel(false).0,
            inbound_live: AtomicUsize::new(0),
            runtime: tokio::runtime::Handle::current(),
        });
        reload(&inner).await?;
        node.set_channel_handler(Arc::new(Handler(inner.clone())));
        let peers = inner.state().outbox.peers_with_entries();
        for peer in peers {
            inner.wake(peer);
        }
        Ok(Messaging { inner })
    }

    pub async fn send(&self, peer: NodeId, body: String) -> Result<StoredMessage, SendError> {
        let inner = &self.inner;
        if inner.is_stopped() {
            return Err(SendError::ShuttingDown);
        }
        let body = normalize_newlines(&body);
        if body.trim().is_empty() {
            return Err(SendError::EmptyBody);
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(SendError::BodyTooLarge);
        }
        if peer == inner.own {
            return Err(SendError::SelfMessage);
        }
        self.require_contact(peer).await?;
        if !inner.state().outbox.reserve() {
            return Err(SendError::OutboxFull);
        }
        let msg = ChatMessage {
            id: new_message_id(),
            sent_at_ms: now_ms(),
            body,
        };
        let status = self.initial_status(&peer);
        let (m, p) = (msg.clone(), peer);
        let row = match inner
            .store_call(move |s, a| s.insert_outgoing(a, &p, &m, status))
            .await
        {
            Ok(row) => row,
            Err(_) => {
                inner.state().outbox.release();
                return Err(SendError::StoreUnavailable);
            }
        };
        inner.state().outbox.push(peer, Entry::new(msg, status));
        inner.wake(peer);
        Ok(row)
    }

    /// Re-queues a `Failed` outgoing row with its original id, timestamp and
    /// body and a fresh attempt budget.
    pub async fn resend(&self, peer: NodeId, id: MessageId) -> Result<MessageStatus, SendError> {
        let inner = &self.inner;
        if inner.is_stopped() {
            return Err(SendError::ShuttingDown);
        }
        let row = inner
            .store_call(move |s, a| s.get(a, &peer, &id))
            .await
            .map_err(|_| SendError::StoreUnavailable)?
            .ok_or(SendError::NotFound)?;
        if row.direction != MessageDirection::Outgoing || row.status != MessageStatus::Failed {
            return Err(SendError::NotResendable);
        }
        self.require_contact(peer).await?;
        {
            let mut state = inner.state();
            if state.outbox.contains(&peer, &id) {
                return Err(SendError::NotResendable);
            }
            if !state.outbox.reserve() {
                return Err(SendError::OutboxFull);
            }
        }
        let status = self.initial_status(&peer);
        match inner
            .store_call(move |s, a| s.set_status(a, &peer, &id, status))
            .await
        {
            Ok(true) => {}
            other => {
                inner.state().outbox.release();
                return Err(match other {
                    Ok(_) => SendError::NotFound,
                    Err(_) => SendError::StoreUnavailable,
                });
            }
        }
        inner.emit(ChatEvent::Status {
            peer,
            id,
            status,
            delivered_at_ms: None,
        });
        let msg = ChatMessage {
            id: row.id,
            sent_at_ms: row.sent_at_ms,
            body: row.body,
        };
        inner.state().outbox.push(peer, Entry::new(msg, status));
        inner.wake(peer);
        Ok(status)
    }

    /// The contact was removed: every queued entry for `peer` becomes Failed
    /// and its outbox slot is freed.
    pub async fn forget_peer(&self, peer: NodeId) {
        self.inner.fail_peer(peer).await;
        self.inner.wake(peer);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChatEvent> {
        self.inner.events.subscribe()
    }

    /// Stops drivers, accept loops and persisters.
    pub fn shutdown(&self) {
        self.inner.stop.send_replace(true);
        self.inner.state().channels.clear();
    }

    /// Whether the engine has registered a current channel to `peer`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_channel(&self, peer: &NodeId) -> bool {
        self.inner.current_channel(peer).is_some()
    }

    async fn require_contact(&self, peer: NodeId) -> Result<(), SendError> {
        match self.inner.is_known(peer).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(SendError::NotAContact),
            Err(()) => Err(SendError::ContactsUnavailable),
        }
    }

    fn initial_status(&self, peer: &NodeId) -> MessageStatus {
        if self.inner.current_channel(peer).is_some() {
            MessageStatus::Sending
        } else {
            MessageStatus::Pending
        }
    }
}

impl Drop for Messaging {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn normalize_newlines(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\r', "\n")
}

/// Rebuilds the outbox from the store after a restart: rows for peers that are
/// no longer contacts and rows beyond the cap become Failed; the rest are
/// queued Pending with a fresh attempt budget.
async fn reload(inner: &Arc<Inner>) -> Result<(), MessageStoreError> {
    let rows = inner
        .store_call(|s, a| s.undelivered(a, usize::MAX))
        .await?;
    let mut verdicts: HashMap<NodeId, bool> = HashMap::new();
    for row in &rows {
        if let std::collections::hash_map::Entry::Vacant(slot) = verdicts.entry(row.peer) {
            slot.insert(inner.is_known(row.peer).await.unwrap_or(true));
        }
    }
    for row in rows {
        let (peer, id) = (row.peer, row.id);
        let known = verdicts.get(&peer).copied().unwrap_or(false);
        let status = if row.status == MessageStatus::Sending {
            MessageStatus::Pending
        } else {
            row.status
        };
        let msg = ChatMessage {
            id,
            sent_at_ms: row.sent_at_ms,
            body: row.body,
        };
        let queued = known
            && inner
                .state()
                .outbox
                .push_reserving(peer, Entry::new(msg, status));
        if !queued {
            inner.persist_status(peer, id, MessageStatus::Failed).await;
        } else if status != row.status {
            inner.persist_status(peer, id, status).await;
        }
    }
    Ok(())
}
