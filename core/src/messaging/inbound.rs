//! Inbound side of one channel: streams are read concurrently but persisted in
//! accept order, so `seq` is deterministic.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use iroh::endpoint::{ReadToEndError, RecvStream, SendStream, VarInt};
use protocol::{ChatMessage, MAX_FRAME_BYTES};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use super::codec::{decode_frame, encode_ack, Frame, FrameError};
use super::engine::{now_ms, Inner};
use super::{ChatEvent, Insert};
use crate::discovery::Channel;

const RESET: u32 = 0;

enum Decoded {
    Chat { msg: ChatMessage, send: SendStream },
}

/// Accept loop plus persister for one channel; ends with the channel or on
/// shutdown.
pub(super) async fn run(inner: Arc<Inner>, channel: Channel) {
    let (queue, slots) = mpsc::channel(inner.config.max_inbound_per_peer.max(1));
    let mut persister = tokio::spawn(persist(inner.clone(), channel.clone(), slots));
    tokio::select! {
        _ = inner.stopped() => persister.abort(),
        _ = async {
            accept(&inner, &channel, queue).await;
            let _ = (&mut persister).await;
        } => {}
    }
}

async fn accept(
    inner: &Arc<Inner>,
    channel: &Channel,
    queue: mpsc::Sender<oneshot::Receiver<Decoded>>,
) {
    while let Ok((send, recv)) = channel.accept_stream().await {
        let (slot, decoded) = oneshot::channel();
        if queue.send(decoded).await.is_err() {
            return;
        }
        tokio::spawn(read(inner.clone(), channel.clone(), send, recv, slot));
    }
}

struct Live<'a>(&'a Inner);

impl<'a> Live<'a> {
    fn enter(inner: &'a Inner) -> Self {
        let live = inner.inbound_live.fetch_add(1, Ordering::SeqCst) + 1;
        inner.config.record_inbound(live);
        Self(inner)
    }
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        self.0.inbound_live.fetch_sub(1, Ordering::SeqCst);
    }
}

fn reset(send: &mut SendStream) {
    let _ = send.reset(VarInt::from_u32(RESET));
}

async fn read(
    inner: Arc<Inner>,
    channel: Channel,
    mut send: SendStream,
    mut recv: RecvStream,
    slot: oneshot::Sender<Decoded>,
) {
    let _live = Live::enter(&inner);
    let frame = match timeout(
        inner.config.frame_read_timeout,
        recv.read_to_end(MAX_FRAME_BYTES),
    )
    .await
    {
        Ok(Ok(frame)) => frame,
        Ok(Err(ReadToEndError::TooLong)) => {
            tracing::debug!(peer = %channel.peer().fmt_short(), "oversized frame");
            channel.close_protocol();
            return;
        }
        Ok(Err(_)) => return,
        Err(_) => {
            let _ = recv.stop(VarInt::from_u32(RESET));
            reset(&mut send);
            return;
        }
    };
    let peer = channel.peer();
    if !channel.is_current() || inner.is_known(peer).await != Ok(true) {
        reset(&mut send);
        return;
    }
    match decode_frame(&frame) {
        Err(FrameError::TooLarge) => channel.close_protocol(),
        Err(_) | Ok(Frame::Ignored) => reset(&mut send),
        Ok(Frame::Ack(_)) => {}
        Ok(Frame::Chat(msg)) => {
            let _ = slot.send(Decoded::Chat { msg, send });
        }
    }
}

async fn persist(
    inner: Arc<Inner>,
    channel: Channel,
    mut slots: mpsc::Receiver<oneshot::Receiver<Decoded>>,
) {
    let peer = channel.peer();
    while let Some(slot) = slots.recv().await {
        let Ok(Decoded::Chat { msg, mut send }) = slot.await else {
            continue;
        };
        let id = msg.id;
        let inserted = inner
            .store_call(move |s, a| s.insert_incoming(a, &peer, &msg, now_ms()))
            .await;
        match inserted {
            Ok(Insert::Inserted(row)) => inner.emit(ChatEvent::Received(row)),
            Ok(Insert::Duplicate) => {}
            Err(e) => {
                tracing::debug!(peer = %peer.fmt_short(), error = ?e, "incoming not stored");
                reset(&mut send);
                continue;
            }
        }
        if inner.config.take_ack_drop() {
            reset(&mut send);
            continue;
        }
        inner.config.count_ack();
        let ack = encode_ack(id);
        let sent = timeout(inner.config.ack_timeout, async {
            send.write_all(&ack).await.ok()?;
            send.finish().ok()
        })
        .await;
        if !matches!(sent, Ok(Some(()))) {
            reset(&mut send);
        }
    }
}
