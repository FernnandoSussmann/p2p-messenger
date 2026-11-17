//! Shared engine state and the helpers every task uses. All store and
//! `KnownPeers` calls run on the blocking pool and fail closed.

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use protocol::MessageId;
use tokio::sync::{broadcast, watch};

use super::config::MessagingConfig;
use super::outbox::Outbox;
use super::{ChatEvent, MessageStatus, MessageStore, MessageStoreError};
use crate::discovery::{Channel, ChannelHandler, KnownPeers};
use crate::identity::{AccountId, NodeId};

pub(super) struct State {
    pub outbox: Outbox,
    pub channels: HashMap<NodeId, Channel>,
}

pub(super) struct Inner {
    pub account: AccountId,
    pub own: NodeId,
    pub known: Arc<dyn KnownPeers>,
    pub store: Arc<dyn MessageStore>,
    pub config: MessagingConfig,
    pub events: broadcast::Sender<ChatEvent>,
    pub state: Mutex<State>,
    pub stop: watch::Sender<bool>,
    pub inbound_live: AtomicUsize,
    pub runtime: tokio::runtime::Handle,
}

/// The node's channel handler: registers each channel with the engine.
pub(super) struct Handler(pub Arc<Inner>);

impl ChannelHandler for Handler {
    fn on_channel(&self, channel: Channel) {
        Inner::on_channel(&self.0, channel);
    }
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Inner {
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn is_stopped(&self) -> bool {
        *self.stop.borrow()
    }

    /// Resolves when the engine is shut down.
    pub async fn stopped(&self) {
        let mut rx = self.stop.subscribe();
        let _ = rx.wait_for(|stopped| *stopped).await;
    }

    pub fn emit(&self, event: ChatEvent) {
        let _ = self.events.send(event);
    }

    /// `Ok(true)` only for a known contact; `Err` means the check failed.
    pub async fn is_known(&self, peer: NodeId) -> Result<bool, ()> {
        let known = self.known.clone();
        tokio::task::spawn_blocking(move || known.is_known(&peer))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())
    }

    pub async fn store_call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn MessageStore, &AccountId) -> Result<T, MessageStoreError> + Send + 'static,
    ) -> Result<T, MessageStoreError> {
        let (store, account) = (self.store.clone(), self.account);
        tokio::task::spawn_blocking(move || f(store.as_ref(), &account))
            .await
            .unwrap_or(Err(MessageStoreError::Unavailable))
    }

    /// The peer's channel if it is still the node's authorized one.
    pub fn current_channel(&self, peer: &NodeId) -> Option<Channel> {
        self.state()
            .channels
            .get(peer)
            .filter(|c| c.is_current())
            .cloned()
    }

    /// Persists `status` for a queued entry and emits it, only on change.
    pub async fn transition(&self, peer: NodeId, id: MessageId, status: MessageStatus) {
        let changed = {
            let mut state = self.state();
            match state.outbox.status_of(&peer, &id) {
                Some(current) if current != status => {
                    state.outbox.update(&peer, &id, |e| e.status = status)
                }
                _ => false,
            }
        };
        if changed {
            self.persist_status(peer, id, status).await;
        }
    }

    /// Persists a status and emits it if the row changed.
    pub async fn persist_status(&self, peer: NodeId, id: MessageId, status: MessageStatus) {
        match self
            .store_call(move |s, a| s.set_status(a, &peer, &id, status))
            .await
        {
            Ok(true) => self.emit(ChatEvent::Status {
                peer,
                id,
                status,
                delivered_at_ms: None,
            }),
            Ok(false) => {}
            Err(e) => tracing::debug!(peer = %peer.fmt_short(), error = ?e, "status not persisted"),
        }
    }

    /// Marks an acked message Delivered at `at_ms` (this device's clock) and
    /// emits it, only if the row changed.
    pub async fn persist_delivered(&self, peer: NodeId, id: MessageId, at_ms: i64) {
        match self
            .store_call(move |s, a| s.mark_delivered(a, &peer, &id, at_ms))
            .await
        {
            Ok(true) => self.emit(ChatEvent::Status {
                peer,
                id,
                status: MessageStatus::Delivered,
                delivered_at_ms: Some(at_ms),
            }),
            Ok(false) => {}
            Err(e) => tracing::debug!(peer = %peer.fmt_short(), error = ?e, "status not persisted"),
        }
    }

    /// The contact is gone (or unauthorized): every queued entry becomes
    /// Failed and its slot is freed.
    pub async fn fail_peer(&self, peer: NodeId) {
        let ids = self.state().outbox.take_all(&peer);
        for id in ids {
            self.persist_status(peer, id, MessageStatus::Failed).await;
        }
    }

    /// Wakes the peer's driver, starting one if it has entries.
    pub fn wake(self: &Arc<Self>, peer: NodeId) {
        let spawn = {
            let mut state = self.state();
            state.outbox.notify(&peer);
            state.outbox.claim_driver(&peer)
        };
        if spawn {
            self.runtime.spawn(super::driver::run(self.clone(), peer));
        }
    }

    fn on_channel(self: &Arc<Self>, channel: Channel) {
        let peer = channel.peer();
        if self.is_stopped() || !channel.is_current() {
            return;
        }
        let fresh = {
            let mut state = self.state();
            let seen = state
                .channels
                .get(&peer)
                .is_some_and(|c| c.stable_id() == channel.stable_id());
            if !seen {
                state.channels.insert(peer, channel.clone());
            }
            !seen
        };
        if fresh {
            self.runtime
                .spawn(super::inbound::run(self.clone(), channel));
            self.wake(peer);
        }
    }
}
