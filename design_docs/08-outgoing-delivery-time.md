# DD-08 — Outgoing Delivery Time

**Feature slice:** `core::messaging` (store, engine, events, DTOs) + `app` (chat commands' DTOs, chat store, outgoing bubble)
**Status:** Draft
**Affected:** `core/src/messaging/` (`sqlcipher.rs` migration SQL and queries, `store.rs` trait, `testing/` memory store,
`contract.rs`, `driver.rs` / `engine.rs` ack path, `events.rs`, `dto.rs`, `model.rs`), `core/src/storage/schema.rs` (one new
migration), `app/src-tauri/src/chat_cmd.rs`, `app/src/lib/chatStore.ts`, `app/src/views/MessageBubble.svelte`, their tests,
`README.md`
**Wire format:** unchanged — no `PROTOCOL_VERSION` bump. The existing `Ack` frame already tells the sender when a
message was delivered; the new time is this device's clock, never a value sent by the peer.
**Depends on:** [DD-07 Visual Identity & UX Redesign](07-visual-identity-and-ux-redesign.md) (outgoing bubble layout,
next migration number)
**Related:** [DD-04 Messaging & Delivery](04-messaging-and-delivery.md), [DD-05 Encrypted Storage](05-encrypted-storage-and-logging.md)

---

## TL;DR

Incoming bubbles show two times (their *sent* clock and our *received* clock). Outgoing bubbles
show only the send time, because the app never records when a message was delivered. This doc
records it:

1. **New nullable `delivered_at_ms` column on `messages`** (forward-only migration). It is set
   **once**, in the same update that makes an outgoing row Delivered, to this device's clock at the
   moment the matching `Ack` is processed. Delivered is already terminal, so the first ack wins.
2. **A dedicated store operation** to mark a message delivered with its time. The generic status
   update can no longer set Delivered, so a Delivered row without a time can't be created by new
   code.
3. **The time travels to the UI** in `MessageDto` (history, search, send results) and in the
   `message_status` event, so the bubble updates live.
4. **Outgoing bubble:** `sent | delivered` times, each a labelled `<time>`, followed by the
   existing status label. Rows delivered before this change have no time and show the sent time
   only.
5. **No wire change, no peer-supplied timestamps.**

## Context and scope

- DD-04 lifecycle: Sending → Delivered (matching `Ack` received; terminal), or Pending / Failed.
  The peer persists, renders and then acks; a duplicate frame is re-acked without being stored
  again.
- The driver handles `Outcome::Acked` by removing the outbox entry and calling
  `persist_status(peer, id, Delivered)`, which runs `MessageStore::set_status` and, if a row
  changed, emits `ChatEvent::Status { peer, id, status }` → `message_status` event
  (`MessageStatusDto { node_id, id, status }`).
- `set_status`'s SQL only updates outgoing rows whose status is not already `delivered`.
- The `messages` table (migration 1) has `sent_at_ms NOT NULL` and `received_at_ms` (incoming only),
  an FTS5 external-content index over `body` with insert/delete triggers only, and is erased by
  history clear and contact removal with erase.
- `MessageDto` exposes `sent_at_ms` and `received_at_ms`; its `Debug` redacts `body` only.
- `chatStore.ts` merges statuses so a page reload or a late event never downgrades Delivered, and
  buffers statuses that arrive while a page is loading.
- DD-07 defines the outgoing bubble meta as "sent time · status label" and adds the theme migration.

In scope: storing, exposing and showing the delivered time of outgoing messages.

## Goals

- Every outgoing message delivered after this change shows when it was delivered, next to when it
  was sent, live (no reload) and after restarts.
- The time is trustworthy in the sense that matters: it is measured by this device and cannot be
  chosen by the peer.
- Recorded exactly once per message; resends and duplicate acks never move it.
- Old rows keep working: no backfill, no error, a clean "time unknown" presentation.

## Non-goals

- **Read receipts or "seen" times.** Delivery means the peer stored and acked the message, not that
  a person read it.
- **The peer's own receipt timestamp.** Putting the peer's clock in the `Ack` would need a wire
  change and would only be an unverifiable claim. Our own clock at ack time is enough.
- **Backfilling old Delivered rows.** Their delivery time was never known; inventing one (e.g. from
  `sent_at_ms`) would be false.
- **Delivery times for incoming messages.** They already have `received_at_ms`.
- **Changing ordering, retry policy, or the status machine.**

## The actual design

### Meaning of the time

`delivered_at_ms` = this device's wall clock (`now_ms()`, the same clock as an outgoing
`sent_at_ms`) when the driver accepts the `Ack` whose id matches the in-flight message. Because the
peer acks only after persisting, it is an upper bound on when the message reached the peer's
storage, plus the return latency of the ack. The UI labels it "Delivered (this device)".

Both outgoing times come from the same local clock, so they are comparable. If the user changes the
system clock between send and ack, the delivered time can appear earlier than the sent time; both
are shown as recorded, never reordered or clamped, and neither affects ordering (still
`(sent_at_ms, seq)`).

### Data storage

- **Migration (next after DD-07's theme migration), owned by the messaging slice:** add a nullable
  `delivered_at_ms INTEGER` column to `messages`. Existing rows read `NULL`. If the bundled SQLite
  accepts it on `ADD COLUMN`, the column carries a `CHECK` that it is `NULL` unless
  `direction = 'out'`; either way, the store's update is restricted to outgoing rows and a contract
  test proves incoming rows can't get one.
- No index: the column is only read with the row.
- The FTS index is unaffected (no body change, and the table has no update trigger). History clear
  and erase-on-remove delete whole rows, so the new column needs no extra erasure step.
- `account_id` scoping is unchanged (same table, same `WHERE account_id = ?`), so the
  SQL-namespacing guard needs no allowlist change.

### Store API

- **New `mark_delivered(account, peer, id, delivered_at_ms) -> bool`:** one `UPDATE` that sets
  `status = 'delivered'` **and** `delivered_at_ms` together, only for an outgoing row that is not
  already delivered. Returns `false` (no change) for a missing row, an incoming row, or an
  already-delivered row, so the first ack's time is never overwritten.
- **`set_status` refuses `Delivered`** with `InvalidQuery`, as it already refuses `Received`. All
  other transitions (Sending, Pending, Failed) are unchanged.
- `StoredMessage` gains `delivered_at_ms: Option<i64>`; every `SELECT` column list includes it.
- Implemented for the SQLCipher store and `MemoryMessageStore` (and `FailingMessageStore` follows its
  existing failure mode). The shared contract suite in `contract.rs` runs against both.

**Trade-off:** a separate method instead of an optional timestamp on `set_status`. It is a slightly
larger trait, but "Delivered" and "has a delivery time" can't diverge, and the compiler points at
every call site that used to set Delivered.

### Engine and events

- On `Outcome::Acked`, the driver takes `now_ms()` and calls `mark_delivered` through the existing
  `store_call` path instead of `persist_status(…, Delivered)`.
- If the row changed, it emits `ChatEvent::Status` with the status and the new
  `delivered_at_ms: Option<i64>` (set only for Delivered; `None` for every other transition).
- Store failure keeps today's behaviour: a `debug` log with the short peer id and the error, no
  event. No new log fields.
- Nothing else in the engine changes: Failed / Pending transitions, unverified handling and the
  contact-gone path still use `set_status`.

### DTOs and commands

- `MessageDto` gains `delivered_at_ms: number | null` (outgoing and delivered after this change;
  `null` otherwise). It flows through every command that already returns messages (`chat_send`,
  `chat_resend`, `chat_history`, `history_search`) with no new command.
- `MessageStatusDto` (`message_status` event) gains `delivered_at_ms: number | null`.
- Both `Debug` impls print the new field like the other timestamps; `body` stays redacted.

### Frontend

- **`chatStore.ts`:** the status merge carries `delivered_at_ms` with the same rules as status:
  a known value is never replaced by `null` or by a later value, whether it arrives first from the
  event, the buffered pending status, or a page load.
- **Outgoing bubble** (DD-07 layout):

  ```
                 ┌────────────────────────────────────┐
                 │                          You       │
                 │ Hi                                 │
                 │  09:40 · Sent │ 09:41 · Delivered  │   ← sent │ delivered · existing status label
                 └────────────────────────────────────┘
  ```

  - Sent time: `<time>` with the title "Sent (this device)".
  - Delivered time: `<time>` with the title "Delivered (this device)", shown only when
    `delivered_at_ms` is set and valid (through the existing `formatLocalTime` / `guardedIso`
    guards).
  - The existing status label follows unchanged ("Sending…", "Delivered", "Pending: will be sent
    when connected", "Not delivered"), plus "Resend" when failed. A Delivered row from before this
    change shows "09:40 · Delivered".
- Styling uses DD-07's shared classes only (no component CSS).

### Text inventory

| Kind | Where | Text |
|---|---|---|
| New | Outgoing bubble, sent time tooltip | "Sent (this device)" |
| New | Outgoing bubble, delivered time tooltip | "Delivered (this device)" |

No existing text changes.

## Security and privacy

- **No peer-controlled timestamp:** the value comes only from this device's clock when a matching,
  authenticated ack arrives on the verified channel (DD-04). A peer can delay its ack but cannot
  choose the recorded value or set it for a message it never received; an ack for an unknown or
  non-matching id changes nothing.
- **Write-once:** the SQL update only touches outgoing, not-yet-delivered rows of the caller's
  `account_id`; resends, duplicate acks and races can't move the time. Contract-tested.
- **Stays encrypted and local:** stored only in `messenger.db` (SQLCipher), never on the wire,
  never in web storage. Erased with the row by history clear and erase-on-remove.
- **No new metadata leak in logs:** no new log lines or fields; if one is added later it goes
  through `core::logging` and the redaction-guard allowlist review. Message bodies stay redacted in
  every `Debug`.
- **Rendering:** the time is a number formatted by the existing guarded formatters and rendered as
  text; invalid values fall back to the existing "unknown time" path or are hidden. No `{@html}`;
  `render-guard` stays green.
- **Wire and secrets:** no protocol change, no secret types touched in `app/` (grep gate green).

## Tests

Rust (`core`):

- Migration: a DB at the previous version (with DD-07's migration applied) upgrades; existing
  delivered rows read `delivered_at_ms = NULL` and keep their status.
- Contract suite (SQLCipher and memory store):
  - `mark_delivered` on Sending / Pending / Failed sets both status and time.
  - A second `mark_delivered` returns `false` and keeps the first time.
  - `mark_delivered` on an incoming row or a missing id returns `false` and changes nothing.
  - `set_status(Delivered)` is refused with `InvalidQuery`.
  - `page`, `get`, `search` and `undelivered` return `delivered_at_ms`.
  - Another account's row with the same peer and id is untouched.
- Level-1 messaging engine (`core/tests/integration/messaging_*.rs`):
  - A sent and acked message ends Delivered with `sent_at_ms ≤ delivered_at_ms ≤ now`, and the
    status event carries the same value.
  - A lost first ack followed by a resend and a re-ack keeps the first recorded time and emits one
    Delivered event.
  - A message that ends Failed has no delivered time.
- DTO tests: `MessageDto` / `MessageStatusDto` serialise `delivered_at_ms` (number or `null`);
  `Debug` still redacts the body.
- The log-redaction guard and SQL-namespacing guard stay green.

Rust (`app`):

- `chat_flow_end_to_end`: after delivery, `chat_history` returns the outgoing message with a
  `delivered_at_ms`, and the emitted `message_status` payload carries it.

Frontend (Vitest):

- `chatStore`: a `message_status` event with a time sets it; a later page load with `null` does not
  clear it; a buffered status applied after a page load keeps its time; a second event never
  replaces the first time.
- `MessageBubble`: outgoing delivered with a time shows two `<time>` elements with the titles "Sent
  (this device)" / "Delivered (this device)" and the "Delivered" label; without a time only the sent
  `<time>` shows; Pending / Failed show no delivered time; an invalid value is hidden.

Manual (README checklist):

- Two instances: send while connected; the bubble shows *sent | delivered* within a second, and
  after a relaunch the same times appear.
- Send while the peer is offline: Pending, no delivered time; reconnect: the delivered time appears
  and is later than the sent time.

## Definition of done

- [ ] Migration adds `messages.delivered_at_ms` (nullable, outgoing-only); schema version and
      migration registry updated.
- [ ] `mark_delivered` added; `set_status` refuses Delivered; `StoredMessage` and all queries carry
      the new column; SQLCipher and memory stores pass the extended contract suite.
- [ ] Driver records the time on the matching ack; `ChatEvent::Status`, `MessageStatusDto` and
      `MessageDto` carry `delivered_at_ms`.
- [ ] `chatStore` merges the time without ever downgrading it; the outgoing bubble shows
      *sent | delivered* with the two new tooltips, using only shared classes.
- [ ] No wire change (`PROTOCOL_VERSION` untouched); no new log fields; all guards green.
- [ ] All tests listed above added and green.
- [ ] README: delivered time described in the messaging section and manual checklist.
- [ ] Pre-commit hook passes.
