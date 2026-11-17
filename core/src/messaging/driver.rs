//! The per-peer delivery driver: spawned when a peer has queued messages,
//! gone when its queue is empty. It owns attempts, retries and backoff.

use std::sync::Arc;

use protocol::{ChatMessage, MessageId, MAX_ACK_BYTES};
use tokio::time::{sleep, sleep_until, timeout, Instant};

use super::codec::{decode_ack, encode_chat};
use super::engine::{now_ms, Inner};
use super::MessageStatus;
use crate::discovery::Channel;
use crate::identity::NodeId;

enum Outcome {
    Acked,
    /// The peer is reachable but did not ack: a counted failure.
    NoAck,
    /// The channel went away: not counted.
    ChannelGone,
    NotAuthorized,
    /// The trust check itself failed: no attempt was made.
    Unverified,
}

pub(super) async fn run(inner: Arc<Inner>, peer: NodeId) {
    tokio::select! {
        _ = inner.stopped() => {}
        _ = drive(&inner, peer) => {}
    }
}

async fn drive(inner: &Arc<Inner>, peer: NodeId) {
    let wake = inner.state().outbox.wake_handle(peer);
    loop {
        if inner.state().outbox.release_driver_if_empty(&peer) {
            return;
        }
        match inner.is_known(peer).await {
            Ok(true) => {}
            Ok(false) => {
                inner.fail_peer(peer).await;
                continue;
            }
            Err(()) => {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = sleep(inner.config.backoff_base) => {}
                }
                continue;
            }
        }
        let Some(channel) = inner.current_channel(&peer) else {
            let sending = inner.state().outbox.sending_ids(&peer);
            for id in sending {
                inner.transition(peer, id, MessageStatus::Pending).await;
            }
            wake.notified().await;
            continue;
        };
        let plan =
            inner
                .state()
                .outbox
                .plan(&peer, Instant::now(), inner.config.max_in_flight_per_peer);
        inner.config.record_outbound(plan.in_flight);
        for msg in plan.start {
            inner
                .runtime
                .spawn(attempt(inner.clone(), peer, channel.clone(), msg));
        }
        let due = async {
            match plan.next_try {
                Some(at) => sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = wake.notified() => {}
            _ = due => {}
            _ = channel.closed() => {}
        }
    }
}

async fn attempt(inner: Arc<Inner>, peer: NodeId, channel: Channel, msg: ChatMessage) {
    let id = msg.id;
    let outcome = tokio::select! {
        _ = inner.stopped() => return,
        outcome = deliver(&inner, peer, &channel, &msg) => outcome,
    };
    finish(&inner, peer, id, outcome).await;
    inner.wake(peer);
}

async fn deliver(inner: &Inner, peer: NodeId, channel: &Channel, msg: &ChatMessage) -> Outcome {
    match inner.is_known(peer).await {
        Ok(true) => {}
        Ok(false) => return Outcome::NotAuthorized,
        Err(()) => return Outcome::Unverified,
    }
    if !channel.is_current() {
        return Outcome::ChannelGone;
    }
    let acked = timeout(inner.config.ack_timeout, exchange(channel, msg)).await;
    match acked {
        Ok(Some(id)) if id == msg.id => Outcome::Acked,
        _ if !channel.is_current() => Outcome::ChannelGone,
        _ => Outcome::NoAck,
    }
}

/// One chat frame on its own stream; the ack comes back on the same stream.
async fn exchange(channel: &Channel, msg: &ChatMessage) -> Option<MessageId> {
    let frame = encode_chat(msg).ok()?;
    let (mut send, mut recv) = channel.open_stream().await.ok()?;
    send.write_all(&frame).await.ok()?;
    send.finish().ok()?;
    let reply = recv.read_to_end(MAX_ACK_BYTES).await.ok()?;
    decode_ack(&reply).ok()
}

async fn finish(inner: &Inner, peer: NodeId, id: MessageId, outcome: Outcome) {
    let backoff_base = inner.config.backoff_base;
    match outcome {
        Outcome::Acked => {
            let at = now_ms();
            inner.state().outbox.remove(&peer, &id);
            inner.persist_delivered(peer, id, at).await;
        }
        Outcome::NoAck => {
            let mut attempts = None;
            {
                let mut state = inner.state();
                state.outbox.update(&peer, &id, |e| {
                    e.attempts += 1;
                    e.in_flight = false;
                    e.next_try = Instant::now() + inner.config.backoff(e.attempts);
                    attempts = Some(e.attempts);
                });
            }
            let Some(attempts) = attempts else { return };
            tracing::debug!(peer = %peer.fmt_short(), attempts, "no ack");
            if attempts >= inner.config.max_attempts {
                inner.state().outbox.remove(&peer, &id);
                inner.persist_status(peer, id, MessageStatus::Failed).await;
            } else {
                inner.transition(peer, id, MessageStatus::Pending).await;
            }
        }
        Outcome::ChannelGone => {
            inner.state().outbox.update(&peer, &id, |e| {
                e.in_flight = false;
                e.next_try = Instant::now();
            });
            inner.transition(peer, id, MessageStatus::Pending).await;
        }
        Outcome::Unverified => {
            inner.state().outbox.update(&peer, &id, |e| {
                e.in_flight = false;
                e.next_try = Instant::now() + backoff_base;
            });
        }
        Outcome::NotAuthorized => inner.fail_peer(peer).await,
    }
}
