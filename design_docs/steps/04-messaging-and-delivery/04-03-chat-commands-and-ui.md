# Step 04-03 — Chat commands, events and the Svelte chat UI

**Estimate:** L — ~1,850 lines changed: Rust prod ~380 (`chat_cmd.rs`, wiring, `test_util` fixtures), Rust tests ~380, TS/Svelte prod ~600, TS tests ~500.

## TL;DR

Expose the 04-02 engine to the webview and ship the chat screen, so the feature works end to end:
- Tauri commands `chat_send`, `chat_resend`, `chat_history` (keyset-paged) and `history_search`
  (DD-05's name, used now for searchable scrollback). History and search are served straight from
  the encrypted store, so they work even when the node failed to start.
- Events `message_received`, `message_status` and `chat_resync` (sent when the event stream lags).
- A chat view reached from **Contacts → a contact row → "Message"**, with:
  - own/peer alignment and local-time timestamps from `sent_at_ms`
  - a per-message status, with races handled
  - a multi-line/emoji composer
  - Resend on Failed
  - scrollback ordered by sender time, then arrival, with the newest at the bottom
  - in-conversation search
  - inert text rendering

## Context and dependencies

- **Requires 04-01 and 04-02 merged** into `04-messaging-and-delivery`. This step uses:
  - from 04-01: `ChatState`, `history_page`, `HistoryCursor`, and the `DEFAULT_*`/`MAX_*` page and
    search constants
  - from 04-02: `Messaging`, `MessagingConfig`, the DTOs (`MessageDto` with `cursor`,
    `MessageStatusDto`, `HistoryPageDto`), `chat_event_json`, `CHAT_RESYNC`, and the re-exports
    `MessageId`, `MAX_BODY_BYTES` and `parse_message_id`
- **No `core` edits are expected in this step.** If one seems necessary, the 04-02 contract was
  missed: fix it in 04-02's area and say so in the PR.
- **Branching:**
  - `git fetch`, then branch `step-04-03-chat-commands-and-ui` from the latest
    `origin/04-messaging-and-delivery`. If 04-02's PR is still open, branch from
    `step-04-02-messaging-engine` and rebase once it merges.
  - Sub-task branches: `step-04-03-chat-commands-and-ui-3a`, `-3b` and `-3c`.
  - Open the PR into `04-messaging-and-delivery`.
- **How the chat screen is reached:**
  1. Main screen → **Contacts** tab (the default) → a contact row → **Message** button. The
     button shows the unread count, e.g. "Message (2)".
  2. The button calls `openChat(nodeId)` from `app/src/lib/chatStore.ts`, which sets the
     `activeChat` store there. `router.ts` is left untouched, to avoid an import cycle
     (`router.ts` imports the views).
  3. `MainScreen.svelte` renders `Chat.svelte` in the tab panel instead of the tab component
     while `$activeChat` is set.
  4. **Back**, or clicking any tab, calls `closeChat()`.
- **No auto-dial (user decision).** When the peer is not connected, messages stay Pending until
  DD-02 brings a channel back: the peer reconnects, or the user uses Contacts → Connect or a
  ticket. The chat screen says this. Dialing contacts by NodeId is a DD-02 follow-up.

## Sequential vs parallel

```
 3a Rust commands + wiring ─┐
 3b Frontend chat UI        ├─▶ 3m merge, cross-check test, full gates, manual checklist
 3c Docs                    ─┘
```

**3a, 3b and 3c run in parallel** (separate worktrees, one sub-agent each) against the
**contract below**, which is binding for all three. File ownership:

| Sub-task | Owns (only these files) |
|----------|-------------------------|
| 3a | `app/src-tauri/src/chat_cmd.rs`, `app/src-tauri/src/lib.rs`, `app/src-tauri/src/net_cmd.rs`, `app/src-tauri/src/contacts_cmd.rs` (make `parse_node_id` `pub(crate)`; call `forget_peer` after a removal), `app/src-tauri/src/contacts_cmd/tests.rs`, `app/src-tauri/src/test_util.rs` |
| 3b | `app/src/chat.ts` (new), `app/src/lib/chatStore.ts` (new), `app/src/lib/time.ts` (new), `app/src/views/Chat.svelte` (new), `app/src/views/MessageBubble.svelte` (new), `app/src/views/Composer.svelte` (new), `app/src/views/MainScreen.svelte`, `app/src/views/ContactRow.svelte`, their `*.test.ts` files, and `app/src/render-guard.test.ts` (new) |
| 3c | `README.md`, `AGENTS.md` |

Each sub-branch passes the hook on its own; 3b mocks `invoke`/`listen` in tests, exactly like the
existing tests. 3m runs sequentially after all three are merged.

### Binding contract (3a ⇄ 3b)

Commands. JS passes camelCase keys and Tauri maps them to snake_case Rust parameters, as in
`net_respond_incoming`.

| Command | JS args | Returns | Errors (`{ type }`) |
|---------|---------|---------|---------------------|
| `chat_send` | `{ nodeId: string, body: string }` | `MessageDto` (the stored row; `status` `"sending"` or `"pending"`) | `NotReady`, `StoreUnavailable`, `InvalidNodeId`, `SelfMessage`, `NotAContact`, `EmptyBody`, `BodyTooLarge`, `OutboxFull`, `ContactsUnavailable`, `Internal` |
| `chat_resend` | `{ nodeId: string, messageId: string }` | `MessageStatusDto` | `NotReady`, `StoreUnavailable`, `InvalidNodeId`, `InvalidMessageId`, `NotFound`, `NotResendable`, `NotAContact`, `OutboxFull`, `ContactsUnavailable`, `Internal` |
| `chat_history` | `{ nodeId: string, before: string \| null, limit: number \| null }` | `HistoryPageDto` (`messages` ascending by `(sent_at_ms, seq)`; `next_before` = opaque cursor for older, or `null`) | `StoreUnavailable`, `InvalidNodeId`, `InvalidCursor`, `Internal` |
| `history_search` | `{ query: string, nodeId: string \| null, limit: number \| null }` | `MessageDto[]` (newest first) | `StoreUnavailable`, `InvalidNodeId`, `InvalidQuery`, `Internal` |

Readiness codes:
- **`StoreUnavailable`**: the encrypted store is not open (no identity yet, or the open failed).
- **`NotReady`**: the store is open but messaging has not started (the node is not started or
  failed to start). Only `chat_send` and `chat_resend` can return it, because `chat_history` and
  `history_search` need only the store.

DTO JSON (from `core::messaging::dto`, already tested in 04-02):

```ts
interface MessageDto { node_id: string; id: string; seq: number; cursor: string; direction: "in" | "out"; body: string;
  sent_at_ms: number; received_at_ms: number | null;
  status: "sending" | "pending" | "delivered" | "failed" | "received"; }
interface MessageStatusDto { node_id: string; id: string; status: MessageDto["status"]; }
interface HistoryPageDto { messages: MessageDto[]; next_before: string | null; }
```

Events:
- `message_received` → `MessageDto`
- `message_status` → `MessageStatusDto`
- `chat_resync` → `null` (the UI reloads the open conversation's newest page)

Defaults come from core (`DEFAULT_PAGE` 50, clamped to `1..=MAX_PAGE`; `DEFAULT_SEARCH_RESULTS`
50, clamped to `1..=MAX_SEARCH_RESULTS`). `MAX_BODY_BYTES = 16384` (UTF-8 bytes) is mirrored in
`chat.ts` and cross-checked in 3m.

## Work items

### 3a. Rust: commands, wiring, forwarder (`app/src-tauri`)

`chat_cmd.rs` extends the 04-01 `ChatState`:

```rust
pub struct ChatState { store: OnceLock<(AccountId, Arc<SqlCipherMessageStore>)>, messaging: OnceLock<Arc<Messaging>> }
impl ChatState {
    /// Starts the engine once, over the opened store. No store → Err(StoreUnavailable).
    pub async fn start_messaging(&self, node: &Node, known: Arc<dyn KnownPeers>, config: MessagingConfig) -> Result<Arc<Messaging>, ChatCmdError>;
    /// StoreUnavailable if the store is absent; NotReady if the store is open but messaging is not started.
    pub fn messaging(&self) -> Result<Arc<Messaging>, ChatCmdError>;
    /// StoreUnavailable if absent. History/search use this directly (account-scoped).
    pub fn store_ref(&self) -> Result<(AccountId, Arc<SqlCipherMessageStore>), ChatCmdError>;
    pub fn shutdown(&self);                                             // Messaging::shutdown if started
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ChatCmdError { NotReady, StoreUnavailable, InvalidNodeId, InvalidMessageId, InvalidCursor, SelfMessage,
    NotAContact, EmptyBody, BodyTooLarge, OutboxFull, NotFound, NotResendable, InvalidQuery, ContactsUnavailable, Internal }
impl From<SendError> for ChatCmdError { /* 1:1 incl. SelfMessage; ShuttingDown → NotReady; StoreUnavailable → StoreUnavailable */ }
impl From<MessageStoreError> for ChatCmdError { /* InvalidQuery → InvalidQuery; everything else → StoreUnavailable */ }
```

- The logic lives in plain functions, testable without a Tauri runtime:
  - `send(chat, node_id, body)`
  - `resend(chat, node_id, message_id)`
  - `history(chat, node_id, before, limit)`, via `spawn_blocking` + `messaging::history_page`
    over `store_ref()`
  - `search(chat, query, node_id, limit)`, via `spawn_blocking` + `store.search` over
    `store_ref()`

  The `#[tauri::command]` wrappers (`chat_send`, `chat_resend`, `chat_history`,
  `history_search`) only forward the managed state.
- Validate input before any core call:
  - `node_id` via `contacts_cmd::parse_node_id` (at most 128 bytes, `NodeId::from_str`)
  - `message_id` via `messenger_core::messaging::parse_message_id`, else `InvalidMessageId`
  - `before` via `HistoryCursor::parse_token`, else `InvalidCursor`
  - `limit` is `Option<u32>`, clamped by core

  Bodies are not inspected here; `Messaging::send` validates them.
- Map results with `MessageDto::from(&row)` (neutralized body, opaque cursor), never with a
  hand-built string.
- `contacts_cmd::remove`: after a successful removal (and the existing `disconnect`), call
  `chat.messaging()` and, if `Ok`, `forget_peer(peer).await`. Its queued messages become
  `Failed` and free their outbox slots. Add a `chat: &ChatState` parameter and update the
  `contacts_remove` wrapper and its call sites.
- `net_cmd.rs::ensure_started`:
  1. After `start_if_present` returns `true`, get `let node = state.node()?` (an `Arc<Node>`) and
     `contacts.book()`, then call
     `chat.start_messaging(&node, book, MessagingConfig::desktop()).await`.
  2. Subscribe both event streams: `node.subscribe()` and `messaging.subscribe()`.
  3. **`drop(node)`** before spawning the forwarders. `shutdown_with_timeout` relies on
     `Arc::try_unwrap`, so no forwarder may hold the node.
  4. Spawn the **chat forwarder**:
     ```rust
     pub fn forward_chat(ev: Result<ChatEvent, RecvError>, emit: impl Fn(&str, Value)) -> bool /* false = stop */ {
         match ev {
             Ok(ev) => { let (name, payload) = chat_event_json(&ev); emit(name, payload); true }
             Err(RecvError::Lagged(_)) => { emit(CHAT_RESYNC, Value::Null); true }
             Err(RecvError::Closed) => false,
         }
     }
     ```
  If the store is unavailable, messaging is not started. The app stays usable: chat commands
  return `StoreUnavailable`, and history works whenever the store is open.
- `lib.rs`: register the four commands in `generate_handler!`. On `RunEvent::ExitRequested | Exit`,
  call `app.state::<Arc<ChatState>>().shutdown()` **before**
  `NetState::shutdown_with_timeout`.
- `test_util.rs`:
  - Keep `started` / `started_with` exactly as they are, with `RecordingHistoryEraser`, for the
    existing tests.
  - Add **`started_with_chat(name) -> StartedChat`**:
    1. Create `let chat = Arc::new(ChatState::new())`.
    2. Build `ContactsState::new(Arc::new(MemoryContactStore::new()), chat.clone())`, so that
       `contacts_remove` with `clear_history` erases **real** history.
    3. Spawn the node with the known factory bound to those contacts.
    4. `chat.open_store(&identity)`, then `chat.start_messaging(&node, book, MessagingConfig::fast())`.
       `fast()` is available because the dev-dependency enables `test-support`.
  - Move the private `accept`/`decline` helpers from `net_cmd.rs`'s test module into
    `test_util.rs`, made `pub`, and update `net_cmd.rs` to import them. Add `connect_pair(a, b)`
    (ticket from `b`, `connect_ticket` on `a`, wait for the incoming request on `b`, `accept`,
    wait for both `Connected`).
- Never log or `eprintln!` a body, a query or a DTO.

### 3b. Frontend (`app/src`)

- **`chat.ts`:**
  - the DTO interfaces above and the `ChatCmdError` union, including `SelfMessage` and
    `InvalidCursor`
  - `invoke` wrappers `chatSend`, `chatResend`, `chatHistory(nodeId, before, limit)` and
    `historySearch`, as in `net.ts`
  - `export const MAX_BODY_BYTES = 16384` and
    `utf8Length(s) = new TextEncoder().encode(s).length`
  - `describeChatError(err)`, with a sentence for **every** error type. Examples:
    - `NotAContact` → "You can only message your contacts."
    - `SelfMessage` → "You can't message yourself."
    - `OutboxFull` → "Too many messages are waiting to be sent. Try again once some are
      delivered."
    - `BodyTooLarge` → "That message is too long."
    - `StoreUnavailable` → "Message history is unavailable right now."

    Messages never echo the body.
- **`lib/time.ts`:** `formatLocalTime(ms: number): string` using
  `Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "short" })`. Non-finite or
  out-of-range values (`|ms| > 8.64e15`) → `"unknown time"`. This is the sender's advisory
  `sent_at_ms`.
- **`lib/chatStore.ts`:**
  - Stores:
    - `activeChat: Writable<string | null>`
    - `conversation: { nodeId, messages, nextBefore, loading, error } | null`
    - `unread: Record<string, number>`
  - Functions:
    - `openChat(nodeId)` sets `activeChat` and calls `openConversation(nodeId)`, which loads the
      newest page and resets unread
    - `closeChat()`
    - `loadOlder()` (prepends using `nextBefore`, keeps the scroll position)
    - `sendMessage(body)`
    - `resendMessage(id)`
    - `startChatEvents(): Promise<() => void>`
  - **Ordering:**
    - Messages are kept sorted ascending by `(sent_at_ms, seq)`.
    - An incoming `message_received` for the open conversation is inserted at its sorted
      position, deduped by `id`.
    - If it sorts **before** the oldest loaded message while `nextBefore !== null`, it is not
      inserted; it appears when paging back.
    - The UI never builds cursors; it uses `dto.cursor` / `next_before` only.
  - **Status merge (race-safe):**
    - A `message_status` event for an `id` not (yet) in the list is stored in a bounded
      `pendingStatus: Map<id, status>` (at most 256 entries, oldest evicted first).
    - Whenever a DTO is appended or merged (the `chat_send` reply, a page load, a received event),
      apply and delete a matching `pendingStatus` entry.
    - **Never downgrade from `delivered`.** If either the current or the incoming status is
      `delivered`, the result is `delivered`. Otherwise the most recent source wins (an event
      over a snapshot loaded before it).
  - Other events:
    - `message_received` for another peer increments `unread[node_id]`.
    - `chat_resync` reloads the newest page of the open conversation, merging statuses with the
      same rules.
  - Cap the loaded list at 1,000 messages: drop the oldest and set `nextBefore` to the new oldest
    message's `cursor`.
- **`views/MainScreen.svelte`:**
  - Add `startChatEvents()` to the `onMount` list, and unlisten on destroy.
  - The tab buttons also call `closeChat()`.
  - The tab panel renders
    `{#if $activeChat}<Chat nodeId={$activeChat} onBack={closeChat} />{:else}<Active />{/if}`.
- **`views/ContactRow.svelte`:** add a **Message** button as the first action, labelled `Message`
  or `Message (n)` from `unread`; it calls `openChat(contact.node_id)`.
- **`views/Chat.svelte`** (props `nodeId`, `onBack`):
  - Header: a Back button, `<bdi>{nickname}</bdi>` from the contacts store, `TrustBadge`, and the
    connection text from `netStore.connections` ("Connected" / "Not connected").
  - When not connected, a `role="status"` note: "Not connected. Messages wait as Pending and are
    sent automatically once you are connected again: when they connect to you, or when you use
    Contacts → Connect or paste their ticket in Network."
  - If the contact is no longer in the contacts store: "This contact was removed.", and the
    composer is disabled.
  - A search field: `historySearch(query, nodeId)`, at least 1 character. Results are listed
    with direction and local time, rendered as inert text the same way as `MessageBubble`. Esc
    clears it.
  - An `<ol role="log" aria-live="polite">` of `MessageBubble`, ordered ascending (newest at the
    bottom). Show "Load older messages" while `nextBefore !== null`. Scroll to the bottom on
    open, and on a new message when already near the bottom.
  - `Composer` at the bottom.
- **`views/MessageBubble.svelte`** (prop `message: MessageDto`):
  - An `li` with class `own`/`peer` by `direction`: own messages are right-aligned, peer messages
    left-aligned.
  - The body is rendered **only** as text interpolation inside
    `<bdi dir="auto" class="body">{message.body}</bdi>`, styled
    `white-space: pre-wrap; overflow-wrap: anywhere; unicode-bidi: isolate;`.
  - `<time datetime={guardedIso(ms)}>{formatLocalTime(message.sent_at_ms)}</time>`, placed
    outside the `bdi`.
  - For `own` messages, a status label: "Sending…", "Delivered", "Pending: will be sent when
    connected" or "Not delivered", with `data-status` set to the raw status. For `failed`, add a
    **Resend** button that calls `resendMessage(id)` and shows any error inline.
  - No `{@html}`, no `<a>`, no `href`, no markdown and no auto-linking: URLs stay plain text and
    are never clickable.
- **`views/Composer.svelte`:**
  - `<textarea aria-label="Message" rows="3">`: multi-line, emoji allowed.
  - **Send** is enabled only when `body.trim() !== ""`, `utf8Length(body) <= MAX_BODY_BYTES`
    and not busy. Show a byte counter above 90 % of the limit.
  - **Ctrl/Cmd+Enter** sends; plain Enter inserts a newline.
  - On success, clear the field. On error, keep the text and show `describeChatError` in
    `role="alert"`.
  - Send the text exactly as typed; core normalizes line endings.

### 3c. Docs

- `README.md`:
  - New **Chat** section:
    - How to open it: Contacts → Message.
    - Status meanings: Sending, Delivered, Pending (waits for a connection and retries this
      session; after a restart it is retried again), Not delivered + Resend.
    - Removing a contact marks its waiting messages Not delivered.
    - Delivery needs both peers online: no store-and-forward and no automatic dialing in v1.
    - The 16 KiB limit.
    - Ordering is by the sender's clock and then arrival, so a wrong clock on the other side can
      misplace their messages.
    - Text is shown literally: no links or formatting, and hidden direction/zero-width/control
      characters show as �.
    - Search, and storage (`messenger.db`, encrypted).
  - Extend the **Manual GUI checklist** with items 20–29 (see Tests → Manual).
  - Mention `chat_flow_end_to_end` beside the other command-level flow tests.
- `AGENTS.md`: in the layout row for `app/src-tauri/`, list `chat_cmd.rs`. In the Testing
  section, name `chat_flow_end_to_end`.

### 3m. Merge and cross-checks

Merge 3a, 3b and 3c into `step-04-03-chat-commands-and-ui`. Then add
`app/src-tauri/tests/frontend_limits.rs`: `frontend_body_limit_matches_protocol` reads
`../src/chat.ts` and asserts that it contains
`MAX_BODY_BYTES = {messenger_core::messaging::MAX_BODY_BYTES}` (re-exported by 04-02; no new
dependency). Run all gates and walk the manual checklist.

## Guardrails

- AGENTS.md security rules are binding. **Never log message plaintext, keys or secrets**: no
  `eprintln!` or `console.log` of bodies, queries or DTOs.
- `DbKey` / `SecretKeyBytes` never appear under `app/`, not even in comments or test names (grep
  gate). The store is opened inside `core`.
- Clipboard access stays **only** in `app/src/views/MyInfo.svelte`. The chat UI has no copy
  button; users select text and use the system shortcut. **Warning:** the pre-commit hook greps
  for the bare words `writeText` and `clipboard` in every non-test file under `app/src`, so
  these words must not appear anywhere in the chat components, stores or helpers, **not even in
  comments or strings**. `clipboard-guard.test.ts` stays unchanged and green.
- `{@html` stays only in `MyInfo.svelte`; `html-guard.test.ts` stays unchanged and green.
- `core` keeps `#![forbid(unsafe_code)]`; this step should not touch `core`.
- Never bypass the hook (`--no-verify`). Fix the failure instead.
- No empty files, stubs or placeholders. No half-wired UI: the Message button, the chat view and
  the commands land in the same step.
- Wire format: unchanged in this step, and `PROTOCOL_VERSION` stays 1. Say so in the PR.
- All data stays namespaced by the store's `account_id`, which comes from the identity. The
  webview never passes an account id.
- The profile override must not weaken any control. Chat uses the same trust gate, store
  encryption and checks under every profile.
- `headless` / `FileKeychain` are never referenced from `app/`.
- **Never write credentials, keys, passwords or secrets in code, test fixtures, config or docs.
  All credentials/keys are provided at runtime (OS keychain / env); test keys are generated at
  runtime.**

## Security review

| Risk | Prevention | How to check |
|------|------------|--------------|
| Untrusted body executed as HTML or markup | Text interpolation only: no `{@html}`, no markdown. The CSP is already `default-src 'self'`. | `MessageBubble.test.ts::renders_html_and_markdown_as_inert_text` (no `img`/`script`/`strong`/`a` elements; `textContent` equals the input); `html-guard.test.ts` |
| Auto-linking or navigation | No `<a>`/`href` in the chat components | `render-guard.test.ts::chat_components_have_no_links` (the sources of `Chat`, `MessageBubble` and `Composer` contain no `<a`, `href`, `window.open` or `location`); `MessageBubble.test.ts::urls_are_not_clickable` |
| Bidi or zero-width spoofing (fake sender, hidden text) | Core neutralizes the targeted set (`MessageDto.body`). The UI isolates each body in `<bdi dir="auto">`. Time, status and alignment live outside the isolate. | `chat_cmd.rs::received_dto_body_is_neutralized` (Rust, end to end; also checks that a ZWNJ is kept); `MessageBubble.test.ts::body_is_isolated_from_metadata` |
| Sender-controlled ordering | **Accepted (DD-04).** A peer can backdate or postdate where its own messages appear. Mitigations: the store merges by `id`, `sent_at_ms` is never a key, and the README explains the clock caveat. | `chatStore.test.ts::received_message_is_inserted_in_sorted_position`; README Chat section |
| Unauthorized peer reaches send or receive | `Messaging` re-checks contacts (04-02). The commands validate the NodeId. The UI offers Message only on contact rows. Removal calls `forget_peer`. | `chat_send_rejects_non_contact_and_bad_input`, `remove_contact_fails_queued_messages`; `ContactRow.test.ts::message_button_opens_chat` |
| Oversized input over IPC | `Messaging::send` bounds the body; `parse_node_id` (≤ 128 B), `parse_message_id` (≤ 64 B) and `parse_token` (≤ 48 B) bound the ids; limits are clamped | `chat_send_rejects_non_contact_and_bad_input` (`BodyTooLarge` for `MAX_BODY_BYTES + 1`; `InvalidNodeId` for a 1 KiB string), `history_rejects_bad_cursor`, `history_limits_are_clamped` |
| Body or secret leaks via errors | `ChatCmdError` carries only a type tag | `chat_errors_serialize_with_type_tag`; `dto_json_never_contains_db_key` (`test_util::assert_forms_absent` over every DTO JSON, using the runtime key bytes from the test keychain) |
| Status shown wrong because of event races or lag | The bounded `pendingStatus` map, the never-downgrade-from-`delivered` merge, and `chat_resync` → reload | `status_event_before_send_reply_is_applied`, `page_load_never_downgrades_delivered`, `forward_chat_emits_resync_on_lag`, `resync_reloads_open_conversation` |
| Shutdown hang or node leak | `drop(node)` before the forwarders are spawned; `ChatState::shutdown` runs before the node shutdown; messaging holds no `Arc<Node>` | `shutdown_with_messaging_completes` (Rust: start, connect, `chat.shutdown()`, then `shutdown_with_timeout` completes and `Arc::try_unwrap` succeeded) |
| Plaintext on disk | The same store as 04-01 | Manual checklist item 29 |

## Tests

**Rust (`app/src-tauri/src/chat_cmd.rs` tests module, over `test_util`):**
- `chat_errors_serialize_with_type_tag` (every variant → `{"type": …}`) and
  `send_error_mapping_is_total` (every `SendError`, including `SelfMessage`, maps to the expected
  `ChatCmdError`)
- `readiness_codes`:
  - a fresh `ChatState` → `StoreUnavailable` from every command
  - store opened but messaging not started → `chat_send`/`chat_resend` return `NotReady`, while
    `chat_history`/`history_search` succeed
- `chat_flow_end_to_end` (two `started_with_chat` nodes, `connect_pair`):
  - `send(A → B, "hi 👋\nline 2")` returns `status == "sending"`.
  - B's forwarder (`forward_chat` into a `Mutex<Vec<(String, Value)>>`) records exactly one
    `message_received` with the same `id` and body.
  - A records `message_status` `delivered`.
  - `history(A)` and `history(B)` each return the message, with direction `out`/`in`.
- `pending_then_delivered_after_reconnect` (command level): disconnect, then `send` →
  `"pending"`. Reconnect via `connect_pair`, then `message_status` `delivered`.
- `resend_flow_via_command`:
  - Set B's faults to drop all acks → the message becomes `failed`. Clear the faults, then
    `resend` → `delivered`.
  - `resend` on a delivered id → `NotResendable`; a garbage id → `InvalidMessageId`.
- `chat_send_rejects_non_contact_and_bad_input` (including `SelfMessage` for its own NodeId)
- `history_paging_via_command`:
  - 60 messages: the first page holds the 50 newest, ascending; `next_before` walks to the
    remaining 10, then becomes `null`.
  - Add equal-`sent_at_ms` messages: no gap and no duplicate across pages.
- `history_limits_are_clamped`, `history_rejects_bad_cursor`
- `search_via_command`: a match, the peer filter, and `InvalidQuery` for an empty or over-long
  query.
- `received_dto_body_is_neutralized`: `"a\u{202E}b"` → payload body `"a\u{FFFD}b"`;
  `"می\u{200C}خواهم"` is unchanged.
- `forward_chat_emits_resync_on_lag`, `forward_chat_stops_on_closed`
- `dto_json_never_contains_db_key`
- `shutdown_with_messaging_completes`
- In `contacts_cmd/tests.rs`:
  - `remove_with_erase_clears_chat_history_end_to_end`: exchange messages, then
    `remove(.., Some(true))` → `history` is empty. With `Some(false)` the history is kept.
  - `remove_contact_fails_queued_messages`: B is offline and A has 2 `pending`. Remove B → both
    become `failed`, `message_status` events are recorded, and the outbox has room again.

**Vitest (`app/src/**/*.test.ts`):**
- `chat.test.ts`: `describeChatError` has a message for every `ChatCmdError` type (an exhaustive
  list, including `SelfMessage` and `InvalidCursor`); `utf8Length` counts an emoji as 4 bytes.
- `lib/time.test.ts`: a valid ms value formats via `Intl` (compare with the
  `new Date(ms).toLocaleString` parts); `NaN`, `Infinity` and `9e15` → `"unknown time"`.
- `lib/chatStore.test.ts`:
  - `open_loads_newest_page_and_resets_unread`
  - `load_older_prepends_using_next_before_token`
  - `received_message_is_inserted_in_sorted_position` (an older `sent_at_ms` lands in the
    middle; one older than the window while `nextBefore` is set is not inserted)
  - `received_event_for_other_peer_increments_unread`
  - `received_event_for_known_id_is_not_duplicated`
  - `status_event_updates_message`
  - `status_event_before_send_reply_is_applied`: the event `delivered` arrives, then the
    `chatSend` promise resolves with `sending` → shows `delivered`
  - `page_load_never_downgrades_delivered`: the event `delivered` is applied, then a page load
    returns `pending` for the same id → it stays `delivered`
  - `status_event_for_unloaded_id_is_applied_on_page_load`
  - `pending_status_map_is_bounded`
  - `resync_reloads_open_conversation`
  - `loaded_list_is_capped`
  - `open_and_close_chat_set_activeChat`
- `views/MessageBubble.test.ts`:
  - `own_and_peer_alignment`
  - `shows_local_time_from_sent_at_ms`
  - `status_labels_for_each_status`
  - `failed_shows_resend_and_calls_chatResend`
  - `renders_html_and_markdown_as_inert_text`
  - `urls_are_not_clickable`
  - `body_is_isolated_from_metadata`: the body is inside a `bdi`, and time/status are not its
    descendants
  - `multiline_emoji_and_zwnj_preserved`
- `views/Composer.test.ts`:
  - `send_disabled_for_empty_and_whitespace`
  - `send_disabled_over_byte_limit_with_counter`
  - `ctrl_enter_sends_enter_inserts_newline`
  - `error_keeps_text_and_shows_alert`
  - `success_clears`
- `views/Chat.test.ts`:
  - `renders_messages_newest_at_bottom`
  - `load_older_button_visible_while_cursor`
  - `not_connected_note_explains_reconnect`
  - `removed_contact_disables_composer`
  - `search_shows_results_as_text`
  - `back_calls_onBack`
- `views/ContactRow.test.ts` (extend): `message_button_opens_chat`, `message_button_shows_unread`.
- `views/MainScreen.test.ts` (extend): `active_chat_replaces_tab_panel`,
  `tab_click_closes_chat`, `chat_events_started_and_unlistened`. Extend the existing
  `contactsStore` mock and add a `chatStore` mock in the same style.
- `render-guard.test.ts`: `chat_components_have_no_links`.
- The existing `html-guard.test.ts` and `clipboard-guard.test.ts` stay unchanged and green.

**Manual** (README checklist items 20–29; walk them in 3m with two profiles, see README
"Two-device simulation"):
20. Contacts → **Message** opens the chat. **Back**, or any tab, returns to the tabs.
21. With both instances connected, multi-line + emoji text arrives once. The sender shows Sending → Delivered, and times are local.
22. Quit B and send from A: the message is **Pending**, and the not-connected note explains the reconnect. Relaunch B and Connect: it becomes Delivered, and B shows it once.
23. An unread count appears on B's Message button while B's chat is closed, and clears when the chat opens.
24. Send `<b>x</b>`, `**x**`, `https://example.com`, a text containing U+202E, and a Persian word containing ZWNJ. All appear literally and nothing is clickable. The U+202E shows as �, and the Persian word renders correctly.
25. Restart A: the history reloads with its persisted statuses, the newest is at the bottom, and **Load older** pages back.
26. Search in the chat finds a word, and only from this conversation.
27. Remove the contact with "Also delete our message history", then re-add it: the chat is empty. Without the option, the history remains. Queued Pending messages for a removed contact show Not delivered.
28. Empty or whitespace input keeps Send disabled. A paste over 16 KiB disables Send and shows the counter.
29. `grep -c "<a word you sent>" <profile data dir>/messenger.db` returns 0 (the file is encrypted).

The Failed → Resend path is covered by `resend_flow_via_command`, because it is hard to provoke
by hand.

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
git config core.hooksPath .githooks   # commit normally on each sub-branch and after 3m
```

- No new native or npm dependency. Do not add a markdown, linkify or sanitizer library; inert
  text needs none.
- `npm run check` must report 0 errors and 0 warnings for the new components.
- Before committing, run the hook's clipboard grep by hand:
  `grep -RlE 'writeText|clipboard' app/src --exclude='*.test.ts'` must print only
  `app/src/views/MyInfo.svelte`.

PR (per `.github/PULL_REQUEST_TEMPLATE.md`):
- Affected: `app` (Tauri commands, Svelte UI) and docs. `core` is not expected to change.
- **Protocol impact: no wire-format change; `PROTOCOL_VERSION` unchanged.**
- Note the DD-02 follow-up: dialing contacts by NodeId.
- Verification: the commands above plus manual checklist items 20–29 on two profiles.

## Definition of done

- [ ] `chat_send`, `chat_resend`, `chat_history` (keyset cursor token) and `history_search` are registered, validated and tested. Errors are type-tagged only, including `SelfMessage` and `InvalidCursor`. Readiness codes: `StoreUnavailable` when there is no store, `NotReady` when the store is open but messaging is not started.
- [ ] History and search are served from `ChatState`'s store (account-scoped) and work without a running node.
- [ ] `message_received`, `message_status` and `chat_resync` are forwarded. Lag triggers a resync. `Arc<Node>` is dropped before the forwarders spawn, and shutdown stops messaging before the node.
- [ ] Removing a contact calls `forget_peer`, so queued messages become Not delivered.
- [ ] The chat screen is reachable via Contacts → Message (`activeChat` lives in `chatStore.ts`; `router.ts` is unchanged), and Back or a tab returns.
- [ ] The chat screen has: own/peer alignment; local-time timestamps from `sent_at_ms`; status for own messages; a multi-line/emoji composer with the byte limit and Ctrl/Cmd+Enter; Resend on Failed; scrollback ordered by `(sent_at_ms, seq)` with the newest at the bottom and "Load older"; in-chat search; an unread count; and the not-connected/reconnect note.
- [ ] Status races are handled (bounded `pendingStatus` map; never downgrade from `delivered`) and tested.
- [ ] Bodies render as inert text: no `{@html}`, no links, no markdown. The targeted characters are neutralized in core, and each body is isolated in a `bdi`. The guards are green, and the words `writeText`/`clipboard` appear nowhere in the chat files.
- [ ] `test_util` has `started_with_chat` (real eraser) and the shared `accept`/`decline`/`connect_pair` helpers; the existing fixtures are unchanged.
- [ ] The frontend body limit equals `protocol::MAX_BODY_BYTES` (cross-check test via the `core` re-export).
- [ ] README (Chat section, checklist items 20–29, flow-test names) and AGENTS.md are updated.
- [ ] All gates are green through the hook, and the manual checklist was walked on two profiles.
