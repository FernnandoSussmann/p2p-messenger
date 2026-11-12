# DD-04 — Messaging & Delivery

**Feature slice:** `core::messaging` + `protocol` usage + `app` chat UI
**Status:** Implemented (v1). Delivered choices: per-stream framing (no wire change,
`PROTOCOL_VERSION` stays 1); history ordered by `(sent_at_ms, seq)` as written,
accepting sender-clock placement; a targeted chat-text filter (`neutralize_chat_text`)
instead of the strict display-name policy; `history_search` is shared with DD-05 and
the encrypted message store is the DD-05 seam; no auto-dial, so the DD-02 follow-up
is dialing contacts by NodeId.
**Related:** [DD-02 Transport & Discovery](02-transport-discovery-and-connection.md),
[DD-03 Contacts & Trust](03-contacts-and-trust.md),
[DD-05 Storage & Logging](05-encrypted-storage-and-logging.md)

---

## Context and scope

This is the actual conversation: composing, sending, receiving, and tracking the
delivery of plain-text messages over a connection that DD-02 established and
DD-03 authorized. The wire types already exist in the `protocol` crate —
`Envelope` (versioned), and `Payload::{Hello, Chat, Ack}` with `ChatMessage`
carrying a `MessageId`, `sent_at_ms`, and a UTF-8 `body`. This slice defines how
those frames are driven: the send/receive loops, the per-message delivery-status
state machine, the lightweight application-level ack, and an in-memory outbox
that retries within a session.

Scope:

- Opening a conversation stream over an authorized DD-02 channel, beginning with
  the `Hello` exchange.
- Sending a `Chat` frame and tracking its status: **Sending → Delivered** (on
  `Ack`) or **Pending/Failed** (held in the outbox, retried in-session).
- Receiving a `Chat` frame, persisting it (DD-05), rendering it, and emitting the
  `Ack`.
- Deduplication by `MessageId`.
- The chat UI: message list with per-message status, multi-line + emoji
  composer.

It does **not** define transport/trust (DD-02/DD-03) or the DB internals (DD-05).
Critically, it is **architected for store-and-forward** (message IDs, a
delivery-status field, a persisted outbox-capable model) even though v1 only
delivers when both peers are online.

## Goals and non-goals

### Goals
- Send/receive plain UTF-8 text (multi-line, emoji) between two online peers over
  the encrypted channel.
- Per-message lifecycle with visible status: Sending, Delivered, Pending, Failed.
- Delivery confirmation via an application-level `Ack` keyed by `MessageId`.
- Idempotent receive: a duplicate `MessageId` is never shown or stored twice.
- An outbox that holds un-acked/failed messages and retries them while the
  session lasts and the peer is reachable.
- Persist every sent/received message to the encrypted history (DD-05) so
  scrollback and search work.

### Non-goals
- **Store-and-forward / offline delivery** — if the peer is offline the message
  stays Pending; it is *not* relayed or held by a third party. (The model is
  built so this can be added without a schema break.)
- **Attachments, voice, video, link previews** — v2.
- **Read receipts / typing indicators / reactions / edits / deletes-for-peer** —
  v1 has delivery status only, not read status.
- **Group messaging** — 1:1 only.
- **Ordering guarantees beyond best-effort** — messages are ordered by sender
  timestamp + receive order; no vector clocks in v1.

## The actual design

### Overview

```
compose ─▶ create ChatMessage(id, sent_at_ms, body) ─▶ persist (status=Sending)
            │                                                   │
            ├─ encode Envelope(version, Chat) ─▶ send on channel │
            │                                                     ▼
            └─ enqueue in outbox until Ack              render in list (Sending)
                                                                  │
 peer receives ─▶ dedupe by id ─▶ persist ─▶ render ─▶ send Ack   │
                                                                  ▼
 Ack(id) arrives ─▶ dequeue from outbox ─▶ status=Delivered ─▶ update UI
```

Every frame is a `protocol::Envelope`; a receiver first checks
`Envelope::is_supported()` and drops/errors on version mismatch before acting on
the payload.

### Conversation setup

- When a DD-02 channel to a peer opens (inbound-accepted or outbound-dialed), the
  first frame each side sends is `Hello` with its self-chosen display name
  (DD-03 treats this as untrusted context). The verified identity remains the
  NodeId from the transport.
- After `Hello`, both directions can carry `Chat`/`Ack` frames. A single logical
  conversation maps to a single peer NodeId.

### Delivery-status state machine

Per outgoing message:

```
          send attempt succeeds           Ack(id) received
 Sending ───────────────────────▶ (on wire) ───────────────▶ Delivered
    │                                                             ▲
    │ send attempt fails / channel down                          │
    ▼                                                             │
 Pending ──(retry while session alive & peer reachable)──────────┘
    │
    │ retries exhausted this session / unrecoverable
    ▼
 Failed  ──(user may manually resend → new attempt)
```

- **Sending:** created and handed to the channel; not yet acked.
- **Delivered:** matching `Ack` received. Terminal (happy path).
- **Pending:** could not be sent or no ack yet and the channel is not currently
  usable; lives in the outbox, retried.
- **Failed:** surfaced to the user as not delivered; eligible for manual resend.
  (A resend reuses the same `MessageId` so a late-arriving peer still dedupes.)

The status is a persisted field on the message row (DD-05), not just UI state, so
it survives a scrollback reload and so the store-and-forward upgrade has a place
to hang offline queueing.

### Outbox & retry

- The outbox is the set of messages not yet Delivered. v1 keeps it in memory for
  active retry, but each entry is backed by its persisted row (status
  Sending/Pending/Failed), so a restart shows the correct last-known status even
  though in-session retry timers reset.
- Retry policy: bounded, backing-off attempts while the peer's channel is up;
  when the channel drops, entries sit Pending until DD-02 re-establishes a
  channel, then retry resumes. Retries must be **idempotent** — same
  `MessageId` — so they can never create duplicates on the peer.

### Receiving & dedupe

- On a `Chat` frame: validate version, then check whether `MessageId` already
  exists for this peer. If yes → it's a retransmit; **re-send the `Ack`** (the
  previous ack may have been lost) but do **not** store or display again. If no →
  persist, render, then send `Ack(id)`.
- `Ack` handling: look up the outbox entry by `MessageId`; mark Delivered;
  dequeue. An `Ack` for an unknown id is ignored (harmless).

### Message validation

- `body` must be valid UTF-8 (guaranteed by the type) and is bounded in size
  (reject/avoid unbounded frames — see Security). Empty/whitespace-only bodies
  are not sent.
- Incoming bodies are treated as **text to display, never as markup or
  commands** (see Security).

### Chat UI

- A message list per contact: own vs. peer alignment, sender timestamp from
  `sent_at_ms` (rendered in local time), and a per-message status indicator for
  own messages (Sending / Delivered / Pending / Failed).
- Composer: multi-line input, emoji allowed; send on an explicit action.
- Failed messages offer a resend affordance.
- Scrollback loads from DD-05 history (paged), newest at bottom.

### API sketch (Tauri commands + events)

| Command | Purpose |
|---------|---------|
| `chat_send` | Compose+send a `ChatMessage` to a peer NodeId; returns the new `MessageId` + initial status |
| `chat_resend` | Retry a Failed message (same `MessageId`) |
| `chat_history` | Paged scrollback for a peer from the encrypted DB |

| Event (core → app) | Meaning |
|--------------------|---------|
| `message_received` | A new inbound message was persisted (peer, id, body, time) |
| `message_status` | An outgoing message's status changed (id → Sending/Delivered/Pending/Failed) |

### Data storage

- A `messages` table in the DD-05 encrypted DB, keyed within
  `(account_id, peer NodeId)`, storing: `MessageId`, direction, `sent_at_ms`,
  received-at, `body`, and `status`. Indexed to support per-peer scrollback and
  text search.
- `MessageId` uniqueness per peer is enforced to make dedupe a storage-level
  guarantee, not just an in-memory check.

## Security

- **Content stays inside the encrypted channel.** `Chat`/`Ack` frames travel only
  over the DD-02 QUIC+Noise channel. No message body may be written to the
  diagnostic log (DD-05), echoed to a relay out-of-band, or placed anywhere an
  eavesdropper sees it. This is the headline invariant.
- **Only authorized peers exchange messages.** The messaging loop must refuse to
  start for a NodeId that DD-02/DD-03 have not authorized. A declined/pending
  connection never reaches `chat_send`/receive. Re-verify at the boundary rather
  than trusting the caller.
- **Untrusted inbound text.** A received `body` is attacker-controlled. It must be
  rendered as inert text: no HTML/markdown execution, no auto-linking that
  triggers navigation without intent, and neutralize bidi-override / zero-width /
  control characters that could spoof the display (e.g. make a message look like
  it came from someone else or hide content). The webview must not interpret
  message content as UI.
- **Resource bounds (DoS).** Enforce a maximum frame/body size on both encode and
  decode so a peer cannot send a multi-gigabyte `body` to exhaust memory or the
  DB. Oversized frames are rejected and the connection may be dropped. Bound the
  outbox and per-peer in-flight message counts so a stuck peer can't grow memory
  without limit.
- **Protocol-version safety.** Reject frames failing `Envelope::is_supported()`
  before acting; never best-effort-decode an unknown version into the wrong
  variant. Unknown payload variants (future additive types) are ignored
  gracefully, not fatal.
- **Dedupe prevents replay-display.** Because receive is idempotent by
  `MessageId`, a replayed `Chat` frame cannot duplicate history or re-notify;
  it only re-acks. (Transport already provides anti-tamper; this guards the
  application layer.)
- **Timestamps are advisory.** `sent_at_ms` is sender-controlled; display it but
  never use it for security decisions or as a unique key.

## Testing

- **Two in-process nodes — happy path:** A sends; B receives exactly once,
  persists, renders, and acks; A transitions Sending → Delivered.
- **Dedupe:** delivering the same `Chat` frame twice yields one stored/displayed
  message and two acks; no duplicate in B's history.
- **Ack loss / retransmit:** drop the first `Ack`; A retries the same
  `MessageId`; B re-acks without re-storing; A ends Delivered.
- **Peer offline:** sending to an unreachable peer leaves the message Pending in
  the outbox; it is not lost and shows the right status after a scrollback
  reload.
- **Channel drop then restore:** Pending messages retry and deliver once DD-02
  re-establishes the channel.
- **Failed + manual resend:** after retries exhaust, status is Failed; resend
  reuses the id and (when the peer returns) dedupes correctly.
- **Version mismatch:** a frame with an unsupported version is rejected, not
  mis-decoded (covered partly by existing `protocol` tests; extend at the loop).
- **Oversized body:** a body over the size bound is rejected on send and on
  receive without crashing.
- **Injection/render safety:** bodies containing HTML, markdown, bidi/zero-width
  characters render as inert, non-spoofing text.
- **Persistence round-trip:** sent and received messages reload from DD-05 with
  correct bodies, directions, timestamps, and statuses.

## Definition of done

- [x] Two online peers exchange multi-line/emoji UTF-8 text over the encrypted
      channel, starting with the `Hello` exchange.
- [x] Each outgoing message shows Sending → Delivered (via `Ack`) or
      Pending/Failed, with status persisted (not just UI).
- [x] Receive is idempotent by `MessageId`; duplicates never double-store or
      double-notify; lost acks are safely re-sent.
- [x] An in-memory outbox (backed by persisted rows) retries un-delivered
      messages in-session and resumes after a channel restore; Failed messages
      can be manually resent with the same id.
- [x] All messages persist to the DD-05 encrypted history; scrollback is paged
      and searchable.
- [x] No message content is ever logged or sent outside the encrypted channel;
      only authorized peers can message.
- [x] Frame/body size and outbox bounds enforced; unsupported versions rejected;
      inbound text rendered inert.
- [x] Model demonstrably leaves room for store-and-forward (status field +
      persisted outbox) without a schema break.
- [x] All tests above pass; `core::messaging` joins the workspace with real
      content only.
