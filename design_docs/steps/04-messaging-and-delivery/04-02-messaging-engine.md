# Step 04-02 — Messaging engine: authorized channels, framing, send/receive, ack, dedupe, outbox and retry

**Estimate:** XL — ~2,400 lines changed: Rust prod ~1,200 (discovery hook ~180, codec/DTO/events/chat-text filter/re-exports ~340, engine ~680), Rust tests ~1,200. No TypeScript.

## TL;DR

Build the whole DD-04 delivery engine in `core`, testable without the UI:
- `discovery` hands every **authorized** connection to a `ChannelHandler`. A channel exists only
  after the existing `Hello` exchange and trust gate.
- `messaging` sends one `Envelope` per QUIC bi-stream (bounded by `MAX_FRAME_BYTES`). The `Ack`
  comes back on the same stream.
- Inbound messages are persisted **in accept order per peer**. They are deduped by the storage
  `UNIQUE` key, and duplicates are re-acked.
- Outgoing messages go **Sending → Delivered | Pending → Failed** with persisted status.
  - The in-memory outbox is bounded, with backoff.
  - Delivery resumes after a channel restore.
  - Rows are reloaded after a restart, with a fresh retry budget; anything over the cap or for a
    non-contact becomes Failed.
  - Removing a contact fails its queued messages.
  - In-flight messages are bounded per peer.
  - Manual resend reuses the same `MessageId`.
- Also: the targeted chat-text filter, the events, the string DTOs, and the re-exports the app
  needs.

The app does not use the engine yet and stays exactly as usable as after 04-01. Step 04-03 wires
the commands and UI.

## Context and dependencies

- **Requires 04-01 merged** into `04-messaging-and-delivery`. 04-01 provides `MessageStore`,
  `StoredMessage`, `MessageStatus`, `HistoryCursor`, `history_page`, `validate_body`,
  `MAX_BODY_BYTES`/`MAX_FRAME_BYTES`/`MAX_ACK_BYTES` and `ChatState`.
- **Branching:**
  - `git fetch`, then branch `step-04-02-messaging-engine` from the latest
    `origin/04-messaging-and-delivery`. If 04-01's PR is still open, branch from
    `step-04-01-encrypted-message-store` instead and rebase once it merges.
  - Sub-task branches: `step-04-02-messaging-engine-2a` and `-2b`, both from
    `step-04-02-messaging-engine`. Merge both back, then do 2c on `step-04-02-messaging-engine`.
  - Open the PR into `04-messaging-and-delivery`.
- Reuse, do not duplicate. The `Hello` frame, its validation and the trust gate stay in
  `core/src/discovery/hello.rs` / `node.rs`; messaging never sends or parses `Hello` itself. The
  Unicode classification stays in `core/src/identity/display_name.rs` / `emoji.rs`; the chat
  filter calls shared helpers and keeps no copy of the tables.
- **No auto-dial (user decision).** Messaging never dials. Re-establishing a channel is DD-02's
  job (ticket connect, LAN Connect, or an inbound reconnect from a known peer). Dialing a known
  contact by NodeId is a DD-02 follow-up; record it in the PR.

## Sequential vs parallel

```
 2a (discovery channel hook)  ─┐
                               ├─▶ 2m merge + green gates ─▶ 2c engine + integration tests (sequential)
 2b (codec, DTO, events, chat  ─┘
     text filter, re-exports)
```

- **2a and 2b run in parallel** in separate worktrees (`git worktree add ../w-2a step-04-02-messaging-engine-2a`, and the same for 2b), one sub-agent each. File ownership is disjoint:

| Sub-task | Owns (only these files) |
|---------|--------------------------|
| 2a | `core/src/discovery/channel.rs` (new), `core/src/discovery/node.rs`, `core/src/discovery/mod.rs`, `core/src/discovery/testing/mod.rs`, `core/tests/discovery_channels.rs` (new) |
| 2b | `protocol/src/lib.rs`, `core/src/identity/display_name.rs`, `core/src/identity/mod.rs`, `core/src/messaging/codec.rs` (new), `core/src/messaging/display.rs` (new), `core/src/messaging/events.rs` (new), `core/src/messaging/dto.rs` (new), `core/src/messaging/mod.rs` |

- Each sub-branch must pass the full hook on its own. Items unused until 2c are **`pub`** and
  re-exported, so `clippy -D warnings` does not flag dead code. Do not use
  `#[allow(dead_code)]`.
- **2m (merge):** merge 2a, then 2b, into `step-04-02-messaging-engine` and run all gates.
- **2c is sequential** after 2m. It owns:
  - `core/src/messaging/{config,outbox,service}.rs` (new)
  - `core/src/messaging/mod.rs`
  - `core/src/messaging/testing.rs`
  - `core/tests/messaging_two_nodes.rs` (new)
  - `core/tests/messaging_frames.rs` (new)
  - `core/tests/contacts_log_guard.rs`
  - the README/AGENTS docs

## Work items

### 2a. `discovery` — authorized channels for the messaging slice

New `core/src/discovery/channel.rs`, re-exported from `discovery/mod.rs`:

```rust
/// An authorized connection to one peer: it exists only after the Hello
/// exchange and the trust gate admitted the peer. Cheap to clone. Holds a
/// `Weak` to the node so it never keeps the node alive (NetState shutdown
/// relies on `Arc::try_unwrap(node)`).
#[derive(Clone)]
pub struct Channel { peer: NodeId, conn: Connection, stable_id: usize, node: Weak<Inner> }

impl Channel {
    pub fn peer(&self) -> NodeId;
    pub fn stable_id(&self) -> usize;
    /// True while this is still the node's authorized channel for `peer`:
    /// not replaced, not closed, node not closing.
    pub fn is_current(&self) -> bool;
    pub async fn open_stream(&self) -> Result<(SendStream, RecvStream), ChannelClosed>;
    pub async fn accept_stream(&self) -> Result<(SendStream, RecvStream), ChannelClosed>;
    pub async fn closed(&self);
    /// The peer broke the messaging protocol (e.g. oversized frame): close with CLOSE_PROTOCOL.
    pub fn close_protocol(&self);
}
impl fmt::Debug for Channel { /* peer.fmt_short() + stable_id only */ }

/// Receives every newly authorized channel. Must be idempotent per
/// (peer, stable_id): a channel may be delivered twice when registration races
/// with authorization. Always called with no node lock held; must not block.
pub trait ChannelHandler: Send + Sync + 'static {
    fn on_channel(&self, channel: Channel);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct ChannelClosed;
```

In `node.rs`:
- Make `Inner` `pub(super)` so `channel.rs` can hold `Weak<Inner>`. Its fields stay private; add
  `pub(super)` helpers for what `Channel::is_current` needs: compare
  `state.authorized[peer].stable_id`, and check `closing`.
- `Inner` gains `channel_handler: Mutex<Option<Arc<dyn ChannelHandler>>>`.
- `authorize(..)`: after the `authorized` map holds the new entry and `emit_connected` ran, clone
  the handler out of its mutex, build the `Channel`, release every lock, then call `on_channel`.
- `Node::set_channel_handler(&self, handler: Arc<dyn ChannelHandler>)`:
  1. Store the handler.
  2. **Collect** the currently authorized entries into a `Vec<Channel>` while holding the `state`
     lock.
  3. Release the lock, then call `on_channel` for each entry. **Never call the handler while
     holding `state`**: the handler may call `Node::channel`/`is_current`, which take the same
     lock, and calling it under the lock would deadlock.
- `Node::channel(&self, peer: &NodeId) -> Option<Channel>` (the current authorized channel) and
  `Node::account(&self) -> AccountId`.
- `Inner::close()` sets the handler to `None`, which breaks the `Node → handler → Channel`
  reference path (`Channel` holds only `Weak`).
- Unknown and pending peers are unchanged: until authorization, nothing past the first frame is
  read. The new hook fires **only** from `authorize` (and the replay of authorized entries), so a
  declined, expired or pending peer can never reach messaging.

`discovery/testing/mod.rs`: add `RecordingChannelHandler`. It records `(peer, stable_id)`, keeps
the channels, and has a `wait_for(peer)` helper. A variant calls `Node::channel` from inside
`on_channel`, which proves there is no deadlock.

### 2b. Codec, chat-text filter, events, DTOs, re-exports

**`protocol/src/lib.rs`**: an additive helper; the wire format does not change.

```rust
/// Reads only an encoded [`Envelope`]'s `version`, without touching the
/// payload, so a receiver can reject an unsupported version before decoding.
pub fn peek_version(bytes: &[u8]) -> Option<u16> {
    postcard::take_from_bytes::<u16>(bytes).ok().map(|(v, _)| v)
}
```

Tests:
- `peek_version_reads_only_the_version`: an envelope encoded with `version: 7` → `Some(7)`;
  empty input → `None`.
- `peek_version_of_unknown_payload_variant`: bytes `[0x01, 0x07, 0x00]` → `Some(1)`, while
  `Envelope::decode` fails.

**`core/src/messaging/codec.rs`** (`pub`, re-exported):

```rust
pub enum Frame { Chat(ChatMessage), Ack(MessageId), Ignored }
pub enum FrameError { TooLarge, UnsupportedVersion, Malformed, EmptyBody }
/// Validates the body (EmptyBody / TooLarge) and that the frame fits MAX_FRAME_BYTES.
pub fn encode_chat(msg: &ChatMessage) -> Result<Vec<u8>, FrameError>;
pub fn encode_ack(id: MessageId) -> Vec<u8>;
/// Order is binding: size → peek_version → (unsupported ⇒ Err, never decoded) → Envelope::decode.
/// A supported-version payload that does not decode (future additive variant or junk) is
/// Ok(Frame::Ignored): not fatal. Hello on a messaging stream → Ignored. Chat with body >
/// MAX_BODY_BYTES (even if the frame fits MAX_FRAME_BYTES) → Err(TooLarge). Chat with an
/// empty/whitespace body → Ignored.
pub fn decode_frame(bytes: &[u8]) -> Result<Frame, FrameError>;
/// A reply on our own stream: ≤ MAX_ACK_BYTES, supported version, Payload::Ack.
pub fn decode_ack(bytes: &[u8]) -> Result<MessageId, FrameError>;
```

**Chat-text filter (user decision A).** This is a targeted policy. It reuses the identity
classification through shared helpers and is **not** the display-name policy.
- In `core/src/identity/display_name.rs`, split the classification into `pub(crate)` named
  helpers, and rebuild `is_invisible_format` from them so the name validator's behaviour is
  byte-for-byte unchanged:
  ```rust
  /// U+202A–202E, U+2066–2069 (embedding/override/isolate controls). Already exists; make pub(crate).
  pub(crate) fn is_bidi_control(c: char) -> bool;
  /// Invisible breaks/joiners that carry no linguistic meaning: U+200B, U+2060–2064, U+FEFF.
  pub(crate) fn is_invisible_break(c: char) -> bool;
  /// C0/C1 controls (`char::is_control`) except '\n' and '\t'.
  pub(crate) fn is_disallowed_control(c: char) -> bool;
  ```
  `is_invisible_format` must still cover exactly the same set as today; it now calls
  `is_invisible_break` for its subset.
- New `core/src/messaging/display.rs`:
  ```rust
  /// Display-safe copy of an untrusted chat body. Replaces with U+FFFD ONLY: bidi controls
  /// (`is_bidi_control`), `is_invisible_break`, and `is_disallowed_control` (so '\r', C1 like
  /// U+0085 too). KEEPS everything else, notably ZWNJ U+200C, ZWJ U+200D, LRM U+200E, RLM U+200F,
  /// ALM U+061C, soft hyphen U+00AD, Arabic number signs U+0600–0605, emoji ZWJ/tag sequences,
  /// '\n' and '\t'. Never trims.
  pub fn neutralize_chat_text(text: &str) -> String;
  ```
  The UI isolates each message in `<bdi>` (04-03); this filter only removes characters that can
  spoof layout or hide content.

**`core/src/messaging/events.rs`:**

```rust
#[derive(Clone)]
pub enum ChatEvent {
    /// A new inbound message was persisted (never emitted for duplicates).
    Received(StoredMessage),
    /// An outgoing message's status changed.
    Status { peer: NodeId, id: MessageId, status: MessageStatus },
}
```

**`core/src/messaging/dto.rs`**: string-only DTOs, and the single mapping to external names
(mirrors `discovery::event_json`).

```rust
#[derive(Serialize)] pub struct MessageDto {
    pub node_id: String, pub id: String, pub seq: i64,
    pub cursor: String,                 // HistoryCursor::of(row).to_token(): the UI never builds cursors
    pub direction: &'static str,        // "in" | "out"
    pub body: String,                   // neutralize_chat_text(&row.body)
    pub sent_at_ms: i64, pub received_at_ms: Option<i64>,
    pub status: &'static str,           // "sending" | "pending" | "delivered" | "failed" | "received"
}
#[derive(Debug, Serialize)] pub struct MessageStatusDto { pub node_id: String, pub id: String, pub status: &'static str }
#[derive(Serialize)] pub struct HistoryPageDto { pub messages: Vec<MessageDto>, pub next_before: Option<String> }
impl From<&HistoryPage> for HistoryPageDto { /* next_before = cursor token */ }
pub const MESSAGE_RECEIVED: &str = "message_received";
pub const MESSAGE_STATUS: &str = "message_status";
/// Emitted by the app when it lagged behind the event stream; the UI reloads.
pub const CHAT_RESYNC: &str = "chat_resync";
pub fn chat_event_json(ev: &ChatEvent) -> (&'static str, serde_json::Value);
```

`MessageDto` and `HistoryPageDto` get a hand-written `Debug` that redacts `body`, because DTOs end
up in test-failure output.

**Re-exports (`messaging/mod.rs`)**, so 04-03 never has to edit `core`:
- `pub use protocol::{MessageId, MAX_BODY_BYTES};`
- the codec, display, events and DTO items
- `pub fn parse_message_id(s: &str) -> Option<MessageId>`: input of at most 64 bytes, then
  `Uuid::parse_str`; anything else → `None`.

### 2m. Merge

Merge 2a, then 2b, into `step-04-02-messaging-engine` (ownership is disjoint, so there is nothing
to resolve by hand) and run every quality gate.

### 2c. The engine (`config.rs`, `outbox.rs`, `service.rs`)

**Transport mapping (no wire change).** The conversation for a peer is its authorized `Channel`.
The `Hello` exchange is the connection's first stream, done by discovery.
- Each later frame is **one `Envelope` on its own bi-stream**: `write_all` + `finish`, read with
  `read_to_end(limit)`, the same framing `Hello` uses.
- A `Chat` goes on a stream opened by the sender. The receiver replies `Ack(id)` on the **same**
  stream's send half and finishes.
- A retransmit is a new stream with the same `MessageId`.
- There is no length prefix and there are no new payload types, so **`PROTOCOL_VERSION` stays 1.**

**Config (`config.rs`).** Every limit is a named constant here and only here; `desktop()` reads
them:

```rust
pub const ACK_TIMEOUT: Duration = Duration::from_secs(10);
pub const FRAME_READ_TIMEOUT: Duration = Duration::from_secs(15);  // slow-sender bound per inbound stream
pub const BACKOFF_BASE: Duration = Duration::from_secs(1);         // doubled per counted failure
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
pub const MAX_ATTEMPTS: u32 = 5;                                   // counted attempts per session → Failed
pub const MAX_OUTBOX: usize = 500;                                 // undelivered messages, all peers
pub const MAX_IN_FLIGHT_PER_PEER: usize = 8;                       // concurrent unacked outbound streams
pub const MAX_INBOUND_PER_PEER: usize = 16;                        // concurrent inbound streams awaiting persist
pub const EVENT_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct MessagingConfig { pub ack_timeout: Duration, pub frame_read_timeout: Duration, pub backoff_base: Duration,
    pub backoff_max: Duration, pub max_attempts: u32, pub max_outbox: usize, pub max_in_flight_per_peer: usize,
    pub max_inbound_per_peer: usize,
    #[cfg(any(test, feature = "test-support"))] pub faults: Arc<testing::Faults> }
impl MessagingConfig { pub fn desktop() -> Self; #[cfg(any(test, feature = "test-support"))] pub fn fast() -> Self /* ack 300 ms, read 2 s, backoff 50→200 ms, attempts 3 */; }
```

**Public API (`service.rs`, re-exported).** Reads go through the store (`history_page`, `search`)
and are served by the app straight from `ChatState`, so `Messaging` has no history or search
methods.

```rust
pub struct Messaging { inner: Arc<Inner> }   // never holds Arc<Node>
pub struct MessagingDeps { pub known: Arc<dyn KnownPeers>, pub store: Arc<dyn MessageStore> }
#[derive(Debug, PartialEq, Eq)]
pub enum SendError { EmptyBody, BodyTooLarge, SelfMessage, NotAContact, ContactsUnavailable,
                     OutboxFull, NotFound, NotResendable, StoreUnavailable, ShuttingDown }

impl Messaging {
    /// Binds to node.account()/node.node_id(), reloads the outbox from the store,
    /// then registers itself as the node's ChannelHandler.
    pub async fn start(node: &Node, deps: MessagingDeps, config: MessagingConfig) -> Result<Messaging, MessageStoreError>;
    pub async fn send(&self, peer: NodeId, body: String) -> Result<StoredMessage, SendError>;
    pub async fn resend(&self, peer: NodeId, id: MessageId) -> Result<MessageStatus, SendError>;
    /// The contact was removed: every queued (Sending/Pending) entry for `peer` becomes Failed
    /// (persisted if the row still exists, Status event emitted) and its outbox slot is freed.
    pub async fn forget_peer(&self, peer: NodeId);
    pub fn subscribe(&self) -> broadcast::Receiver<ChatEvent>;  // capacity EVENT_CAPACITY
    /// Stops drivers, accept loops and persisters. Call before Node::shutdown.
    pub fn shutdown(&self);
}
```

All store and `KnownPeers` calls run on `tokio::task::spawn_blocking`. A panicked or cancelled
task maps to `StoreUnavailable` / `ContactsUnavailable` (fail closed), like `known_call` in
`node.rs`.

**`send(peer, body)`**. The order is binding:
1. Normalize `\r\n` and lone `\r` to `\n`. A whitespace-only body → `EmptyBody`. More than
   `MAX_BODY_BYTES` bytes → `BodyTooLarge`. Nothing is stored or sent in either case.
2. `peer == own` → `SelfMessage`.
3. **Authorization at the boundary:** call `known.is_known(&peer)`. `Ok(false)` → `NotAContact`;
   `Err` → `ContactsUnavailable`. Never trust the caller.
4. Reserve an outbox slot under the outbox lock. If it is full → `OutboxFull`, with nothing
   persisted. Release the reservation if step 5 fails.
5. Build `ChatMessage { id: new_message_id(), sent_at_ms: now_ms(), body }`. The status is
   `Sending` if `channel(peer)` is current, else `Pending`. Call `store.insert_outgoing`.
6. Push the entry `{ msg, attempts: 0, next_try: now, in_flight: false }` onto the peer queue,
   wake the peer driver, and return the stored row. The initial status is carried by the return
   value; no `Status` event is emitted for it.

**Outbox (`outbox.rs`).** A `HashMap<NodeId, VecDeque<Entry>>` plus a total count. **Invariant:**
the total is at most `max_outbox`, so persisted Sending/Pending rows never exceed it. Delivered
and Failed entries are removed.

**Per-peer driver.** It is spawned lazily when a peer has entries and exits when its queue is
empty or on shutdown.
- On every wake, re-check `is_known(peer)`. On `Ok(false)`, run the `forget_peer` path (all
  entries → Failed, slots freed). On `Err`, do nothing and stay fail-closed: no attempts are made
  this round.
- When there is no current channel:
  - Every entry whose persisted status is `Sending` becomes `Pending` (persisted, event emitted).
  - The driver then waits on the peer's `Notify` (new channel, new entry, forget, shutdown).
  - **Attempts are not consumed while the channel is down.**
- When the channel is current:
  - Start attempts for due entries (`next_try <= now`, not in flight), in queue order, with at
    most `max_in_flight_per_peer` in flight.
  - Wait on `Notify`, the earliest `next_try`, an attempt completing, or `channel.closed()`.
- **Attempt:**
  1. Re-check authorization (`is_known` must be `Ok(true)`, else the outcome is `NotAuthorized`)
     and `channel.is_current()`.
  2. `open_stream` → `write_all(encode_chat)` → `finish` →
     `timeout(ack_timeout, recv.read_to_end(MAX_ACK_BYTES))` → `decode_ack`.
  3. The acked id must equal the sent id. An ack for a different id counts as no ack.
  4. Under `test-support`, `faults.outbound_in_flight_max` records the peak number of
     concurrently in-flight attempts for the peer.
- **Outcome handling:**
  - **Ack:** `set_status(Delivered)`, emit `Status`, remove the entry. If the store returns
    `Ok(false)` (the row was erased with the contact), still remove the entry and emit nothing.
  - **Failure while `channel.is_current()` still holds** (peer reachable, no ack): this is a
    counted failure, `attempts += 1`.
    - If `attempts >= max_attempts`: the status becomes `Failed` (persisted, event emitted) and
      the entry is removed.
    - Otherwise the status becomes `Pending` (persisted and emitted only on change), with
      `next_try = now + min(backoff_base * 2^(attempts-1), backoff_max)`.
  - **Failure because the channel went away:** not counted. The status becomes `Pending` (on
    change) and the entry waits for the next channel.
  - **`NotAuthorized`:** run the `forget_peer` path.
- **Retries keep the status `Pending`.** Only the first attempt runs as `Sending`, so the state
  machine is Sending → Delivered | Pending → Delivered | Failed.

**Resume after channel restore.** `ChannelHandler::on_channel` records the channel in
`channels: HashMap<NodeId, Channel>`, ignoring a `(peer, stable_id)` it already has. It then
starts one accept loop and one persister for that channel, and wakes (or spawns) the peer driver.
Pending entries retry immediately; there is no dial logic.

**Restart reload** (in `start`):
1. `store.undelivered(account, usize::MAX)`, then `is_known` once per distinct peer.
2. Rows whose peer is `Ok(false)` (removed or unknown contact) are set to **`Failed`** and not
   queued. Rows whose peer check returns `Err` stay queued; attempts re-check authorization anyway.
3. Of the remaining rows, the first `max_outbox` (oldest first) enter the outbox with
   `attempts = 0` (a fresh retry budget). Any of them persisted as `Sending` is set to `Pending`,
   since nothing is in flight after a restart.
4. Rows beyond the cap are set to **`Failed`**, so the user can resend them.
5. `Failed` rows are never retried automatically.

**`resend(peer, id)`:**
1. `store.get` → `None` → `NotFound`. The row must be outgoing and `Failed`, else
   `NotResendable`.
2. Run the same contact check and outbox reservation as `send`.
3. The status becomes `Sending` if the channel is current, else `Pending` (persisted, event
   emitted).
4. Enqueue `ChatMessage { id, sent_at_ms, body }` **from the stored row** (same id, timestamp and
   body) with a fresh attempt budget.

**Inbound: read concurrently, persist in accept order (deterministic `seq`).** Each channel gets
an accept loop and a single persister task, connected by a bounded
`mpsc::channel::<oneshot::Receiver<Decoded>>(max_inbound_per_peer)`.
- **Accept loop:** for each `channel.accept_stream()`, create a `oneshot`, `send(rx).await` the
  receiver into the queue (this waits when the queue is full, so QUIC flow control pushes back on
  the peer), then spawn a reader task holding `tx` and the stream.
- **Reader task:**
  1. `timeout(frame_read_timeout, recv.read_to_end(MAX_FRAME_BYTES))`.
     - Timeout → `stop` the stream.
     - `Err(ReadToEndError::TooLong)` (`iroh::endpoint::ReadToEndError`) →
       `channel.close_protocol()`, which drops the connection.
     - Any other error → return. Dropping `tx` releases the slot.
  2. **Re-verify at the boundary:** `channel.is_current()` and `is_known(peer) == Ok(true)`. If
     either fails, reset the stream and return; nothing is stored or acked.
  3. Run `decode_frame`:
     - `Err(TooLarge)` → `close_protocol()`.
     - `Err(UnsupportedVersion | Malformed)` → reset the stream: no decode, no store, no ack. The
       connection stays.
     - `Ok(Ignored)` → reset (unknown future variant, stray `Hello`, empty body).
     - `Ok(Ack(_))` → ignore it. Acks are valid only as replies on our own streams, so an `Ack`
       for an unknown id is harmless.
     - `Ok(Chat(msg))` → `tx.send(Decoded::Chat { msg, send })`.
- **Persister:** `while let Some(rx) = queue.recv().await { if let Ok(Decoded::Chat { msg, mut send }) = rx.await { … } }`.
  It awaits slots strictly in accept order, so inserts, and therefore `seq`, follow accept order
  even when readers finish out of order. A slow reader holds the line for at most
  `frame_read_timeout`. For each `Chat`, call
  `store.insert_incoming(account, peer, &msg, now_ms())`:
  - `Inserted(row)` → emit `Received(row)`, then ack.
  - `Duplicate` → **re-ack only**: no event, no second row.
  - Store error → no ack, so the sender retries.
  - The ack is `timeout(ack_timeout, write_all(encode_ack(id)) + finish)`.
  - Under `test-support`: if `faults.drop_acks > 0`, decrement it and reset the stream instead of
    acking. `faults.acks_sent` counts real acks, and `faults.inbound_in_flight_max` records the
    peak number of outstanding reader tasks.

**`sent_at_ms` (user decision B).** It is stored, displayed and used as the **primary history sort
key**, with ties broken by `seq` (store contract from 04-01). It is never used for dedupe, a
unique key, retry scheduling, backoff, outbox order or any authorization or security decision.
The outbox is ordered by queue position and seq. A peer can therefore backdate or postdate where
its *own* messages appear in the conversation. DD-04 accepts this; record it in the PR.

**Logging.** `tracing::debug!` lines may carry `peer = %peer.fmt_short()`, attempt counts, status
names and error classes. They must **never** carry a body, a query or frame bytes.

**Test support (`messaging/testing.rs`, extend):**
- `Faults { drop_acks: AtomicUsize, acks_sent: AtomicUsize, inbound_in_flight_max: AtomicUsize, outbound_in_flight_max: AtomicUsize }`
- `ChatPeer { node: TestNode, book: BookCell, store: Arc<dyn MessageStore>, messaging: Messaging, config: MessagingConfig }`
- `chat_peer(name, kind: StoreKind, config)` with `enum StoreKind { Memory, SqlCipher }`:
  - `Memory`: build a `MemoryMessageStore` first and pass it both as the messaging store and as
    the book's eraser (`BookCell::known_factory(contacts, store.clone())`). Trait upcasting
    `Arc<dyn MessageStore>` → `Arc<dyn HistoryEraser>` is stable on MSRV 1.95.
  - `SqlCipher`: the store can only open after provisioning. Spawn the node first (book eraser
    `NoMessageStore`, since these tests do not exercise erase), then call
    `open_message_store(&node.identity)`, then `Messaging::start`.
- `restart_chat_peer(old) -> ChatPeer` (same store, same dir)
- `connect_and_accept(a, b)`: ticket dial + `accept_incoming`; waits for both sides to be
  `Connected`.
- `send_raw_frame(node: &Node, peer: &NodeId, bytes: &[u8]) -> RawReply { ack: Option<MessageId>, reset: bool, connection_closed: bool }`:
  opens a stream on `node.channel(peer)` and writes the raw bytes. This is the only way tests
  inject adversarial or hand-timestamped frames from an authorized peer.

**Log guard.** In `core/tests/contacts_log_guard.rs`, add `messaging_never_logs_bodies_or_secrets`.
It scans `src/messaging` (which must be non-empty) with the existing `violations` function plus
the forbidden words `body`, `query`, `bytes`, `key` and `hex`. Add one `guard_detects_violations`
case for `debug!(body = %m.body)`.

**Docs:**
- `README.md` layout row: messaging = "encrypted message store and delivery engine".
- `AGENTS.md`:
  - Testing: mention the Level-1 messaging tests in `core/tests/messaging_*.rs`.
  - Notes: mention `messaging::testing` (`ChatPeer`, `Faults`, `send_raw_frame`), and that
    discovery exposes authorized channels via `ChannelHandler` only.

## Guardrails

- AGENTS.md security rules are binding. **Never log message plaintext, keys or secrets.** NodeIds,
  stable ids, attempt counts and status names are fine. The log guard enforces this.
- `DbKey` / `SecretKeyBytes` never cross into `app/`, not even in comments (grep gate). This step
  does not touch `app/`.
- Clipboard access stays only in `app/src/views/MyInfo.svelte`.
- `core` keeps `#![forbid(unsafe_code)]`.
- Never bypass the hook (`--no-verify`). Fix the failure instead.
- No empty files, stubs or placeholders. Sub-task items unused until 2c are real `pub` code with
  tests, not stubs.
- Wire format: **unchanged**. `Envelope`/`Payload` are the same, the per-stream framing is the
  same as `Hello`, and `peek_version` is an additive helper. `PROTOCOL_VERSION` stays 1; state
  this in the PR. If you deviate (length-prefixed multiplexing, new payload variants, changed
  field types), you must bump `PROTOCOL_VERSION` and say so.
- Every store call is scoped by the node's `account_id`.
- The profile override must not weaken any control; messaging does not read it at all.
- `headless` is never referenced from `app/`. Do not add messaging to `headless` in this step.
- **Never write credentials, keys, passwords or secrets in code, test fixtures, config or docs.
  All credentials/keys are provided at runtime (OS keychain / env); test keys are generated at
  runtime** (`spawn_test_node` provisions fresh random keys into `FakeKeychain`).

## Security review

| Risk | Prevention | How to check |
|------|------------|--------------|
| Unauthorized NodeId reaches messaging | Channels come only from `authorize()`. `is_known` + `is_current` are re-checked on every send, every attempt, every driver wake and every inbound stream, failing closed on `Err`. Pending peers never get streams accepted. | `pending_and_declined_peers_never_reach_the_handler`, `pending_peer_chat_stream_is_never_processed`, `removed_contact_cannot_deliver_or_receive`, `send_to_non_contact_is_refused` |
| Deadlock in the handler replay | Authorized entries are collected under the lock; `on_channel` runs after the lock is released | `handler_can_query_node_inside_on_channel` |
| Memory DoS via a huge frame | `read_to_end(MAX_FRAME_BYTES)`, so we never buffer more; `TooLong` closes the connection. A body over `MAX_BODY_BYTES` is rejected on encode and decode, even inside a frame that fits. | `oversized_inbound_frame_closes_connection_without_storing`, `oversized_body_rejected_on_send`, codec `body_over_limit_inside_fitting_frame_is_rejected` |
| Slow-loris or stream flood | `frame_read_timeout` per stream; the bounded slot queue per peer (`max_inbound_per_peer`); QUIC stream limits | `inbound_concurrency_is_bounded` (`inbound_in_flight_max <= max_inbound_per_peer + 1`, the +1 being the slot the persister awaits) |
| Outbox or in-flight growth for a stuck or removed peer | `max_outbox` reservation (`OutboxFull`); `max_in_flight_per_peer`; `max_attempts` → Failed; removed contact → Failed, slot freed | `outbox_full_rejects_without_persisting`, `in_flight_per_peer_is_bounded` (uses `outbound_in_flight_max`), `removing_contact_fails_pending_and_frees_slots`, `reload_fails_rows_for_unknown_contacts` |
| Replay or duplicate display | `UNIQUE(account_id, peer, message_id)` + `Insert::Duplicate` → re-ack only, no event | `duplicate_chat_frame_stores_once_and_acks_twice` |
| Version mismatch mis-decode | `peek_version` runs before the payload decode; an unsupported version is reset, never decoded | `unsupported_version_frame_is_rejected_not_decoded`, codec `unsupported_version_is_not_decoded` |
| Unknown future variant kills the connection | A supported-version decode failure → `Ignored`; the connection is kept | `unknown_payload_variant_is_ignored_and_connection_survives` |
| Forged or misdirected ack marks the wrong message Delivered | Acks are accepted only on the reply half of our own stream, and the id must match; unsolicited acks are ignored | `ack_for_unknown_id_is_ignored`, codec `decode_ack_rejects_non_ack_and_oversized` |
| Incoming id equal to our outgoing id hides or alters our row | The duplicate path never touches status, so the outgoing row is untouched | `incoming_id_collision_does_not_change_outgoing_status` |
| `sent_at_ms` abused | Display + primary sort key only, never dedupe, unique key, retry or authorization. **Accepted risk (DD-04):** a peer can backdate or postdate where its own messages appear. Non-deterministic `seq` is prevented by persisting in accept order. | `history_orders_by_sender_time_then_arrival`, `inbound_persist_follows_accept_order`; `grep -n sent_at_ms core/src/messaging/{service,outbox}.rs` shows only message construction |
| Body in logs | No body in any `tracing` call; `StoredMessage`/DTO `Debug` redact it | `messaging_never_logs_bodies_or_secrets`, `message_dto_debug_redacts_body` |
| Display spoofing via bidi, zero-width or control chars | `neutralize_chat_text` (targeted set → U+FFFD) in `MessageDto`; legitimate ZWNJ/ZWJ/LRM/RLM/ALM/soft hyphen/Arabic signs are kept; isolation in `<bdi>` (04-03) | `chat_text_*` unit tests, `message_dto_body_is_neutralized`, `display_name_policy_unchanged` |
| Node kept alive or shutdown blocked | `Channel` holds `Weak`; `Messaging` never holds `Arc<Node>`; the handler is cleared on close | `messaging_does_not_keep_node_alive`: hold the node in an `Arc`, start messaging, connect, `messaging.shutdown()`; `Arc::try_unwrap` succeeds, `node.shutdown().await` completes within 2 s, and every recorded channel reports `is_current() == false` |

## Tests

Two-in-process-node tests use real iroh loopback endpoints (`NodeConfig::loopback()` via
`spawn_test_node_with`), like `core/tests/discovery_*.rs`. Use the same `run(..)` multi-thread
runtime wrapper, because the package name `core` breaks `#[tokio::test]`. Wait with deadlines,
never with fixed sleeps, except for bounded "nothing happens" windows of at most 600 ms (like
`QUIET` in `contacts_two_nodes.rs`). Never assert an order that the test does not control.

**2a — `core/tests/discovery_channels.rs`:**
- `handler_sees_channel_after_hello_on_both_sides`: exactly one channel per side, and
  `is_current()` holds.
- `pending_and_declined_peers_never_reach_the_handler`: while the prompt is open → none; after a
  decline → none.
- `handler_registered_late_replays_current_channels`
- `handler_can_query_node_inside_on_channel`: the recording variant calls `Node::channel` inside
  `on_channel`, both on replay and on a live authorize, and completes within the deadline.
- `replaced_or_closed_channel_is_not_current`: after `disconnect` → false. After a reconnect →
  a new `stable_id`, and the old one stays false.
- `authorized_peers_exchange_post_hello_streams`: A calls `open_stream`, B calls
  `accept_stream`, and the bytes round-trip.
- `close_protocol_closes_with_protocol_code`

**2b — unit tests:**
- `codec.rs`:
  - `chat_round_trips_multiline_emoji`
  - `encode_rejects_empty_and_oversized_body`
  - `decode_rejects_oversized_frame`
  - `body_over_limit_inside_fitting_frame_is_rejected`: build `Envelope::new(Payload::Chat { body: "a".repeat(MAX_BODY_BYTES + 1), .. }).encode()` directly. Its length is `<= MAX_FRAME_BYTES`, and `decode_frame` → `Err(TooLarge)`.
  - `unsupported_version_is_not_decoded`: an encoded
    `Envelope { version: PROTOCOL_VERSION + 1, payload: Chat }` → `UnsupportedVersion`.
  - `unknown_variant_is_ignored`
  - `hello_on_messaging_stream_is_ignored`
  - `decode_ack_rejects_non_ack_and_oversized`
- `display.rs`:
  - `chat_text_keeps_persian_zwnj`: `"می\u{200C}خواهم"` is unchanged.
  - `chat_text_keeps_hindi_zwj`: `"क्\u{200D}ष"` is unchanged.
  - `chat_text_keeps_emoji_sequences`: 👨‍👩‍👧, 🏳️‍🌈 and 🏴󠁧󠁢󠁥󠁮󠁧󠁿 are unchanged.
  - `chat_text_keeps_marks_soft_hyphen_and_arabic_signs`: U+200E, U+200F, U+061C, U+00AD and
    U+0600 are unchanged.
  - `chat_text_neutralizes_bidi_controls`: RLO U+202E, LRI U+2066 and PDI U+2069 → U+FFFD.
  - `chat_text_neutralizes_invisible_breaks`: ZWSP U+200B, U+2060 and BOM U+FEFF → U+FFFD.
  - `chat_text_neutralizes_controls_but_keeps_newline_and_tab`: `"a\u{0007}b\r\u{0085}c\n\td"` →
    `"a\u{FFFD}b\u{FFFD}\u{FFFD}c\n\td"`.
  - `chat_text_is_identity_on_plain_text`
- `display_name.rs`:
  - `display_name_policy_unchanged`: a regression test. ZWNJ, LRM, RLM, ALM, the soft hyphen and
    U+0600 are still rejected as `InvisibleCharacter`; bidi is still `BidiOverride`; emoji ZWJ
    sequences are still accepted.
  - All existing display-name tests stay green and unedited.
- `dto.rs`:
  - `message_dto_json_shape` (includes `cursor`)
  - `history_page_dto_carries_cursor_token`
  - `message_status_dto_json_shape`
  - `chat_event_json_names`
  - `message_dto_body_is_neutralized`
  - `message_dto_debug_redacts_body`
- `mod.rs`: `parse_message_id_bounds_input` (valid UUID → `Some`; garbage or more than 64 bytes →
  `None`).

**2c — `core/tests/messaging_two_nodes.rs`** (the design-doc Testing section). Each test uses
`StoreKind::Memory` unless stated otherwise.
- `happy_path_sending_then_delivered` (**two nodes — happy path**):
  - The body is `"hi 👋\nsecond line"`. `send` returns `Sending`.
  - B emits exactly one `Received` with the same id, body and `sent_at_ms`, and B's store has one
    incoming row.
  - A emits `Status Delivered`, and A's row is `Delivered`.
- `duplicate_chat_frame_stores_once_and_acks_twice` (**dedupe**): `send_raw_frame` the same
  encoded `Chat` twice from A's node. Both replies carry `Ack(id)`, `acks_sent == 2`, and B has
  one row and one `Received` event.
- `lost_ack_is_retried_with_same_id_and_ends_delivered` (**ack loss**): B has `drop_acks = 1`. A
  ends `Delivered`; B has one row and no second `Received`.
- `send_to_offline_peer_stays_pending_and_survives_reload` (**peer offline**):
  - Use `StoreKind::SqlCipher` for A. Connect, shut B down, A sends → `Pending`.
  - No `Failed` within 5 × `backoff_max`: attempts are not consumed while the channel is down.
  - `restart_chat_peer(A)`: `history_page` shows the row as `Pending` with the same body, and it
    is queued again with a fresh budget.
- `pending_messages_deliver_after_channel_restore` (**channel drop then restore**):
  - Connect, `disconnect`, then send 3 messages (all `Pending`). Reconnect via ticket.
  - All 3 become `Delivered`.
  - B's id **set** equals A's id set, with each message stored exactly once.
  - B's `history_page` is sorted by `(sent_at_ms, seq)`: assert the order is non-decreasing on
    that key. Do not assert send order, since concurrent in-flight streams may arrive in any
    order.
- `retries_exhaust_to_failed_then_manual_resend_reuses_id` (**failed + manual resend**):
  - B has `drop_acks = usize::MAX`. A goes Sending → Pending → `Failed` after `max_attempts`.
  - Set B's `drop_acks = 0` and `resend`: it ends `Delivered`, and B has exactly one row with
    that id.
  - `resend` of a `Delivered` row → `NotResendable`; an unknown id → `NotFound`.
- `restart_reload_resumes_pending_and_marks_overflow_failed`: `max_outbox = 2`. Persist 3
  undelivered rows for a contact directly in A's store, then restart. Two rows are queued as
  `Pending` (one of them persisted as `Sending` becomes `Pending`), and the third is `Failed`.
  After connecting, the two are delivered.
- `reload_fails_rows_for_unknown_contacts`: persist Pending rows for a NodeId that is not a
  contact, then restart. Those rows are `Failed`, they are not queued, and the outbox capacity
  is unaffected.
- `removing_contact_fails_pending_and_frees_slots`:
  - `max_outbox = 2`, B offline, A sends 2 → `Pending`. A third send to contact C →
    `OutboxFull`.
  - `book.remove(B)` + `forget_peer(B)`: both become `Failed` (events emitted). The send to C now
    succeeds.
  - A second variant without calling `forget_peer` relies on the driver's wake check: reconnect
    B, and its entries become `Failed` without being sent.
- `outbox_full_rejects_without_persisting`
- `in_flight_per_peer_is_bounded`: B has `drop_acks = MAX`, A sends 3 × `max_in_flight_per_peer`.
  A's `faults.outbound_in_flight_max <= max_in_flight_per_peer`.
- `send_to_non_contact_is_refused`, `empty_or_whitespace_body_is_not_sent`,
  `oversized_body_rejected_on_send` (**oversized body, send side**: `BodyTooLarge`, store empty),
  `self_message_is_refused`
- `removed_contact_cannot_deliver_or_receive`:
  - After connecting, `book.remove(peer, Keep)` on B **without** disconnecting. A's send is never
    stored or acked by B, and A ends `Failed` or `Pending`.
  - On A, remove B, then send → `NotAContact`.
- `persistence_round_trip_with_sqlcipher` (**persistence round-trip**): both peers use
  `StoreKind::SqlCipher`. Exchange messages both ways, shut down, reopen the stores. Bodies,
  directions, `sent_at_ms`, `received_at_ms` (incoming only) and statuses (`Delivered` /
  `Received`) all match.
- `history_orders_by_sender_time_then_arrival`: A sends, via sequential `send_raw_frame` calls
  awaiting each ack, three `Chat` frames with `sent_at_ms` `[2_000, 1_000, 1_000]` (ids x, y, z).
  B's history is `y, z, x`, which is deterministic because the sends are sequential.
- `inbound_persist_follows_accept_order`: from A's node, open 3 raw streams in order and write
  their frames in **reverse** order with equal `sent_at_ms`. B's `seq` order equals the stream
  open order. Assert on accept order, which the test controls via sequential `open_stream` before
  any write.
- `incoming_id_collision_does_not_change_outgoing_status`
- `messaging_does_not_keep_node_alive`

**`core/tests/messaging_frames.rs`** (adversarial, via `send_raw_frame` / `raw_dial`):
- `unsupported_version_frame_is_rejected_not_decoded` (**version mismatch** at the loop): no ack,
  no row, no event; a following valid frame is acked.
- `unknown_payload_variant_is_ignored_and_connection_survives`
- `oversized_inbound_frame_closes_connection_without_storing` (**oversized body, receive side**):
  `MAX_FRAME_BYTES + 1` bytes → `connection_closed`, B's store is empty, and B still accepts a new
  connection and a valid message.
- `ack_for_unknown_id_is_ignored`: no state change; the connection is kept.
- `pending_peer_chat_stream_is_never_processed`: `raw_dial` with a valid `Hello`, then open a
  second stream with a `Chat` while the prompt is open. No ack within `QUIET`, and B's store is
  empty. After a decline it is still empty.
- `chat_as_first_frame_is_refused` (Hello-first): `raw_dial` whose first frame is a `Chat` →
  closed with `CLOSE_PROTOCOL`, nothing stored.
- `inbound_concurrency_is_bounded`

**Log guard:** `messaging_never_logs_bodies_or_secrets` (see above).

## Quality gates

Run these on each sub-branch (2a, 2b), and again after 2m and 2c:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
git config core.hooksPath .githooks   # commit normally; the hook runs all of the above plus grep gates
```

No new native dependency: SQLCipher arrived in 04-01. If a two-node test is flaky, fix the wait
condition or the asserted property (set vs order). Do not add retries around assertions, and do
not mark tests `#[ignore]`. Run `cargo test -p core --test messaging_two_nodes` 5 times in a row
before opening the PR; it must pass every time.

PR (per `.github/PULL_REQUEST_TEMPLATE.md`):
- Affected: `protocol` (the `peek_version` helper), `core` (the discovery channel hook, the
  identity classification refactor, the messaging engine), docs.
- **Protocol impact: no wire-format change; `PROTOCOL_VERSION` unchanged.**
- Explain the per-stream framing and the accepted `sent_at_ms` ordering risk. Note the DD-02
  follow-up (dialing contacts by NodeId).
- Verification: the commands above. List the new test files.

## Definition of done

- [ ] `discovery` exposes `Channel`/`ChannelHandler`. Channels come only from `authorize()` (after `Hello` + trust gate), hold only `Weak` to the node, and the handler is cleared on close. The replay calls the handler outside the `state` lock.
- [ ] `protocol::peek_version` exists. The codec rejects unsupported versions before decoding and ignores unknown variants. It bounds frames, bodies (even inside a fitting frame) and acks on both encode and decode.
- [ ] `neutralize_chat_text` neutralizes only the targeted set via shared identity helpers. The display-name validator's behaviour is unchanged (regression test). `MessageDto.body` is neutralized, and DTOs carry an opaque `cursor`.
- [ ] `core::messaging` re-exports `MessageId`, `MAX_BODY_BYTES` and `parse_message_id`, so 04-03 needs no `core` edits.
- [ ] All engine limits are named constants in `messaging/config.rs` (10 s ack, 5 attempts, 500 outbox, …), and `MAX_BODY_BYTES` lives only in `protocol`.
- [ ] Status goes Sending → Delivered or Pending → Failed and is persisted, with `message_status` events. `send` returns the initial status.
- [ ] Receive is persisted in accept order per peer (deterministic `seq`), dedupes via storage (re-ack, no event), ignores acks for unknown ids, and does not ack on a store error.
- [ ] Outbox, in-flight and inbound limits are enforced. Retries back off and are counted only while the channel is up. Restart reload gives a fresh budget, and overflow or unknown contacts → Failed. `forget_peer` and the driver wake check fail a removed contact's queue and free its slots.
- [ ] Resend works only for `Failed` rows, with the same id, timestamp and body. Pending messages resume on channel restore, with no auto-dial.
- [ ] Authorization is re-checked at every send, attempt, driver wake and inbound stream.
- [ ] Every 2a/2b/2c test above exists and passes (including all design-doc Testing cases at the core level), the two-node suite passes 5 runs in a row, and the log guard covers `src/messaging`.
- [ ] README and AGENTS.md are updated. The PR states the wire format did not change, records the accepted ordering risk, and notes the DD-02 follow-up. All gates are green through the hook.
