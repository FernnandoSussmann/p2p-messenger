# Step 04-01 — Encrypted message store (`core::messaging` seam) and protocol size bounds

**Estimate:** L — ~1,250 lines changed: Rust prod ~620 (protocol ~40, core store ~480, identity ~30, app wiring ~70), Rust tests ~520, docs/config ~80. No TypeScript.

## TL;DR

Add the `core::messaging` slice with the **`MessageStore` trait** (the DD-05 seam), a durable
**SQLCipher-encrypted** implementation keyed with the keychain `DbKey`, and an in-memory test double
behind `test-support`. Add the frame/body size constants to `protocol`. History is ordered by
**sender timestamp, then local arrival** (`(sent_at_ms, seq)`, as DD-04 says) and paged with a
keyset cursor. The store implements `contacts::HistoryEraser`, and the app now uses it instead of
`NoMessageStore`, so **Remove… → "Also delete our message history"** really deletes rows. No chat
traffic yet. The app stays fully usable.

## Context and dependencies

- DD-04 (`design_docs/04-messaging-and-delivery.md`) is the source of truth. This step covers its
  **Data storage** section, the **Resource bounds** constants and the persistence half of the
  **Persistence round-trip** test.
- User decision: DD-05 (full SQLCipher storage + logging) is **not** built. DD-04 ships a minimal,
  clearly-labelled DD-05 seam: one encrypted `messages` table and nothing else from DD-05 (no
  settings migration, no logging pipeline, no `history_clear` UI).
- **Branching (do this first, exactly in this order).** The four DD-04 step files are currently
  untracked in the working tree. Commit them to the DD branch before anything else, so every
  worktree has them and 04-04's `git rm -r` works:
  ```sh
  git fetch origin
  git switch -c 04-messaging-and-delivery origin/main      # or `git switch 04-messaging-and-delivery` if it exists; then `git merge --ff-only origin/main`
  git add design_docs/steps/04-messaging-and-delivery/     # stage ONLY this folder
  git commit -m "Add DD-04 step files"                     # the hook runs; it must pass
  git push -u origin 04-messaging-and-delivery
  git switch -c step-04-01-encrypted-message-store
  ```
  The working tree may show unrelated pending changes (e.g. deleted DD-02 step files). Do not
  stage or commit them here. Open this step's PR into `04-messaging-and-delivery`.
- No earlier DD-04 step is needed.

## Sequential vs parallel

**Fully sequential, one implementer, one branch.** The store, the identity accessor and the app
wiring share types, and splitting them would leave sub-branches that fail clippy (`-D warnings`
flags unused `pub(crate)` items). Order: 1 → 2 → 3 → 4 → 5 → 6.

## Work items

### 1. `protocol` — size bounds (no wire-format change)

Add to `protocol/src/lib.rs`, next to `MAX_HELLO_BYTES`. These are the **single source** for the
limits; everything else (core, app, the frontend cross-check in 04-03) refers to them:

```rust
/// Upper bound, in UTF-8 bytes, of a [`ChatMessage::body`]. Senders refuse to
/// encode more; receivers reject more. Raising it is a breaking change for
/// older receivers and requires a `PROTOCOL_VERSION` bump.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// Upper bound, in bytes, of one encoded `Chat` envelope: `MAX_BODY_BYTES` plus
/// the envelope overhead (version varint, variant tag, 16-byte id with length
/// prefix, zig-zag `sent_at_ms`, body length varint) with headroom.
pub const MAX_FRAME_BYTES: usize = MAX_BODY_BYTES + 64;

/// Upper bound, in bytes, of an encoded `Ack` envelope.
pub const MAX_ACK_BYTES: usize = 64;
```

Unit tests in the existing `#[cfg(test)] mod tests`:
- `max_body_chat_fits_within_max_frame_bytes`: a body of exactly `MAX_BODY_BYTES` bytes (a 4-byte
  emoji repeated `MAX_BODY_BYTES / 4` times) with `sent_at_ms = i64::MIN` (worst-case varint)
  encodes to `<= MAX_FRAME_BYTES`.
- `ack_fits_within_max_ack_bytes`.

**Wire format:** unchanged. `Envelope`, `Payload` and their encoding are untouched; the constants
only bound values that no shipped build has ever sent (no build before DD-04 exchanges `Chat`
frames). **`PROTOCOL_VERSION` stays `1`.** Say so explicitly in the PR ("Protocol impact: no change
to the wire format, limits only").

### 2. `identity` — crate-private DB-key access (key never leaves `core`)

In `core/src/identity/service.rs` add, next to `load_identity_secret`:

```rust
/// Loads the SQLCipher key for the message store. Crate-private: the key is
/// used and dropped inside `core` and never reaches the app.
pub(crate) fn load_db_key(&self) -> Result<(AccountId, DbKey), LoadError> { /* read_marker → keychain.load_db_key */ }

/// The app-data directory holding the marker (and the encrypted DB).
pub(crate) fn data_dir(&self) -> &Path { &self.data_dir }
```

Errors map exactly like `load_identity_secret` (`NotProvisioned`, `Unavailable(..)`). Do not add
any `Clone`/`Debug`-revealing path for `DbKey`; its existing redaction tests stay green.

### 3. `core::messaging` — model, trait, stores

Create the module with real content only, and add `pub mod messaging;` to `core/src/lib.rs`.

```
core/src/messaging/mod.rs        // slice docs + curated re-exports
core/src/messaging/model.rs      // StoredMessage, MessageDirection, MessageStatus, HistoryCursor, Insert, MessageStoreError, validate_body
core/src/messaging/store.rs      // trait MessageStore, HistoryPage, history_page(), page/search limit constants
core/src/messaging/sqlcipher.rs  // SqlCipherMessageStore, open_message_store, schema/migration
core/src/messaging/contract.rs   // #[cfg(test)] contract suite run against every impl
core/src/messaging/testing.rs    // #[cfg(any(test, feature = "test-support"))] MemoryMessageStore, FailingMessageStore
```

`mod.rs` docs must state these points:
- The store is the **DD-05 seam**; DD-05 later moves the table into its single DB and migration
  framework.
- Bodies live only inside the SQLCipher file, and every row and every query is scoped by
  `account_id`.
- Dedupe is a storage invariant (`UNIQUE(account_id, peer, message_id)`).
- History order is `(sent_at_ms, seq)`. `sent_at_ms` is sender-controlled and is used for display
  and ordering only, never for a unique key or a security decision.

**Model (`model.rs`).**
- Write `Debug` for `StoredMessage` by hand, printing `body: <redacted, N bytes>`.
- `StoredMessage` derives `Clone, PartialEq, Eq` (not `Debug`).
- The enums derive `Clone, Copy, Debug, PartialEq, Eq`.
- Step 04-02 relies on `StoredMessage: Clone` for broadcast events.

```rust
pub enum MessageDirection { Incoming, Outgoing }             // stored as 'in' / 'out'
/// Mapped to/from TEXT in Rust only (`as_str` / `parse`); the table has no CHECK on it, so a
/// future status (e.g. store-and-forward "queued") needs no table rebuild. An unknown value read
/// back is treated as corruption (`Unavailable`).
pub enum MessageStatus { Sending, Pending, Delivered, Failed, Received }
pub struct StoredMessage {
    pub seq: i64,                    // local arrival/insertion order: the tie-breaker
    pub peer: NodeId,
    pub id: MessageId,
    pub direction: MessageDirection,
    pub sent_at_ms: i64,             // sender-controlled: display + primary sort key only
    pub received_at_ms: Option<i64>, // Some for Incoming only
    pub body: String,
    pub status: MessageStatus,       // Received iff Incoming (enforced in Rust)
}
/// Keyset cursor over (sent_at_ms, seq). Crosses the IPC boundary only as an opaque token
/// "<sent_at_ms>:<seq>" (decimal), because a JS number cannot carry every i64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryCursor { pub sent_at_ms: i64, pub seq: i64 }
impl HistoryCursor {
    pub fn of(row: &StoredMessage) -> Self;
    pub fn to_token(&self) -> String;
    /// At most 48 bytes, exactly one ':', both parts parse as i64; else None.
    pub fn parse_token(s: &str) -> Option<Self>;
}
pub enum Insert { Inserted(StoredMessage), Duplicate }
pub enum MessageStoreError { Unavailable, WrongKeyOrCorrupt, NewerSchema, NotEncrypted, AlreadyExists, InvalidBody, InvalidQuery }
/// Non-empty after trim and at most protocol::MAX_BODY_BYTES bytes.
pub fn validate_body(body: &str) -> Result<(), MessageStoreError>;
```

`Display` for `MessageStoreError` names the failure class only, never a path, key, id or body.

**Trait (`store.rs`).** Blocking API; async callers use `spawn_blocking`, like contacts.

```rust
pub const DEFAULT_PAGE: usize = 50;
pub const MAX_PAGE: usize = 200;
pub const DEFAULT_SEARCH_RESULTS: usize = 50;
pub const MAX_SEARCH_RESULTS: usize = 100;
pub const MAX_SEARCH_QUERY_CHARS: usize = 128;

pub trait MessageStore: HistoryEraser {
    /// New outgoing row (status Sending or Pending). Duplicate id for the peer → AlreadyExists.
    fn insert_outgoing(&self, account: &AccountId, peer: &NodeId, msg: &ChatMessage, status: MessageStatus) -> Result<StoredMessage, MessageStoreError>;
    /// New incoming row (status Received). Existing (account, peer, id) → Ok(Insert::Duplicate), row untouched.
    fn insert_incoming(&self, account: &AccountId, peer: &NodeId, msg: &ChatMessage, received_at_ms: i64) -> Result<Insert, MessageStoreError>;
    /// Outgoing rows only. Delivered is terminal (never overwritten). `Received` is refused.
    /// Ok(false) if no such row / not outgoing / already Delivered.
    fn set_status(&self, account: &AccountId, peer: &NodeId, id: &MessageId, status: MessageStatus) -> Result<bool, MessageStoreError>;
    fn get(&self, account: &AccountId, peer: &NodeId, id: &MessageId) -> Result<Option<StoredMessage>, MessageStoreError>;
    /// Rows strictly before `before` in (sent_at_ms, seq) order, newest-first window of `limit`
    /// rows, returned oldest→newest.
    fn page(&self, account: &AccountId, peer: &NodeId, before: Option<HistoryCursor>, limit: usize) -> Result<Vec<StoredMessage>, MessageStoreError>;
    /// Substring search over bodies, newest first by (sent_at_ms, seq), scoped to the account
    /// (and the peer if given).
    fn search(&self, account: &AccountId, peer: Option<&NodeId>, query: &str, limit: usize) -> Result<Vec<StoredMessage>, MessageStoreError>;
    /// Outgoing rows with status Sending or Pending, oldest first by seq, at most `limit`.
    fn undelivered(&self, account: &AccountId, limit: usize) -> Result<Vec<StoredMessage>, MessageStoreError>;
}

pub struct HistoryPage { pub messages: Vec<StoredMessage>, pub next_before: Option<HistoryCursor> }
/// Clamps `limit` to 1..=MAX_PAGE (default DEFAULT_PAGE). `next_before` is the cursor of the
/// oldest returned row when the page is full, else None. Used by the app's chat_history.
pub fn history_page(store: &dyn MessageStore, account: &AccountId, peer: &NodeId, before: Option<HistoryCursor>, limit: Option<usize>) -> Result<HistoryPage, MessageStoreError>;
```

- `insert_*` call `validate_body` first, as defence in depth: the store never accepts an empty or
  oversized body.
- `search` trims the query and rejects an empty one, or one longer than `MAX_SEARCH_QUERY_CHARS`
  code points, with `InvalidQuery`. Every `limit` is clamped in the store (`page` to
  `1..=MAX_PAGE`, `search` to `1..=MAX_SEARCH_RESULTS`). `undelivered` accepts `usize::MAX`.
  Convert with `i64::try_from(limit).unwrap_or(i64::MAX)` before binding, never with `as`.
- **Search case folding contract:** ASCII case-insensitive is guaranteed by every implementation.
  Non-ASCII case folding is **not** guaranteed: the trigram path may fold some non-ASCII letters,
  while the `LIKE` path and `MemoryMessageStore` do not. Non-ASCII text is always found when the
  query has the same case. Document this on `search`.

**SQLCipher implementation (`sqlcipher.rs`).**

- Dependency: add to the root `Cargo.toml` `[workspace.dependencies]`
  `rusqlite = { version = "0.42", features = ["bundled-sqlcipher-vendored-openssl"] }` (rusqlite
  0.42 declares `rust-version = "1.89"`, below our MSRV 1.95; the feature exists in 0.42).
  - In `core/Cargo.toml` add `rusqlite.workspace = true`.
  - `cargo tree -i libsqlite3-sys` must show exactly one version.
  - **Do not add a cargo feature to `core`.**
    `app/src-tauri/tests/no_test_backends.rs::core_defines_no_keyring_backend_feature` asserts the
    only feature is `test-support`.
- Public entry point. It is generic over the identity boundaries, so the key never leaves `core`:

```rust
pub const DB_FILE: &str = "messenger.db";

/// Opens (creating if absent) the encrypted store in the identity's data dir, keyed with the
/// keychain DB key. Never recreates or deletes an existing file.
pub fn open_message_store<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    identity: &IdentityService<K, S, T>,
) -> Result<(AccountId, SqlCipherMessageStore), MessageStoreError>;

pub struct SqlCipherMessageStore { conn: Mutex<rusqlite::Connection> }
impl SqlCipherMessageStore {
    pub(crate) fn open_with_key(path: &Path, key: &DbKey) -> Result<Self, MessageStoreError>;
}
```

- Open sequence. Each step's failure maps to the named error, and on failure the file is **never**
  deleted, renamed or re-created:
  1. On Unix, if the file does not exist, create it with `OpenOptions` + `mode(0o600)` before
     `Connection::open`. If it exists, leave its permissions alone and do not truncate it.
  2. Key with the raw 32 bytes (no KDF). Build the hex digits into a pre-sized zeroizing buffer by
     pushing nibbles, with no `format!` and no `collect` (both would leave unzeroed copies). The
     buffer holds only `[0-9a-f]`, so no injection is possible:
     ```rust
     const HEX: &[u8; 16] = b"0123456789abcdef";
     let mut sql = Zeroizing::new(String::with_capacity(16 + 64 + 3)); // never reallocates
     sql.push_str("PRAGMA key = \"x'");
     for b in key.as_bytes() {
         sql.push(HEX[usize::from(b >> 4)] as char);
         sql.push(HEX[usize::from(b & 0x0f)] as char);
     }
     sql.push_str("'\";");
     conn.execute_batch(&sql).map_err(|_| MessageStoreError::Unavailable)?;
     ```
  3. `PRAGMA cipher_version` must return a non-empty string, else `NotEncrypted`. This guards
     against accidentally linking plain SQLite.
  4. `SELECT count(*) FROM sqlite_master`. A failure (`SQLITE_NOTADB`) → `WrongKeyOrCorrupt`.
  5. `PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON; PRAGMA foreign_keys = ON;`.
     `temp_store` keeps sort and temp b-trees out of temp files. `secure_delete` zeroes freed
     content before the page is re-encrypted.
  6. Run the migration on `PRAGMA user_version`. `0` → create schema v1 inside one transaction
     and set `user_version = 1`. `1` → do nothing. `> 1` → `NewerSchema` (refuse, do not touch).
- Schema v1. All statements are constants, and **all values go through bound parameters**; never
  `format!` user data into SQL. There is **no CHECK on `status`**: statuses are validated by the
  typed `MessageStatus` mapping in Rust, so adding one later needs no table rebuild.

```sql
CREATE TABLE messages (
  seq            INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id     BLOB    NOT NULL CHECK (length(account_id) = 16),
  peer           TEXT    NOT NULL,                 -- NodeId::to_string()
  message_id     BLOB    NOT NULL CHECK (length(message_id) = 16),
  direction      TEXT    NOT NULL CHECK (direction IN ('in','out')),
  sent_at_ms     INTEGER NOT NULL,
  received_at_ms INTEGER,
  body           TEXT    NOT NULL,
  status         TEXT    NOT NULL,
  UNIQUE (account_id, peer, message_id)
);
CREATE INDEX messages_by_peer_time ON messages (account_id, peer, sent_at_ms, seq);
CREATE INDEX messages_outbox       ON messages (account_id, status, seq) WHERE direction = 'out';
CREATE VIRTUAL TABLE messages_fts USING fts5 (body, content = 'messages', content_rowid = 'seq', tokenize = 'trigram');
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts (rowid, body) VALUES (new.seq, new.body); END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.seq, old.body); END;
```

  `account_id` is `AccountId::as_uuid().as_bytes()` and `message_id` is `MessageId::as_bytes()`.
  Bodies are never updated, so there is no update trigger.
- `insert_incoming`: `INSERT ... ON CONFLICT (account_id, peer, message_id) DO NOTHING`. Zero
  changed rows → `Insert::Duplicate`. The UNIQUE key does not include the direction, so an
  incoming `Chat` whose id collides with one of our own outgoing ids is a `Duplicate`: it is not
  stored, and the outgoing row's status is untouched.
- `set_status` (refuse `Received` in Rust first):
  `UPDATE messages SET status = ?4 WHERE account_id = ?1 AND peer = ?2 AND message_id = ?3 AND direction = 'out' AND status <> 'delivered'`.
- `page`:
  `SELECT … FROM messages WHERE account_id = ?1 AND peer = ?2 AND (?3 IS NULL OR (sent_at_ms, seq) < (?3, ?4)) ORDER BY sent_at_ms DESC, seq DESC LIMIT ?5`
  (row values need SQLite ≥ 3.15, which the bundled SQLCipher satisfies). Then reverse the rows.
  `?3`/`?4` are the cursor's `sent_at_ms`/`seq`, or `NULL`.
- `search`: queries of 3 or more code points use the trigram index:
  `SELECT m.… FROM messages_fts f JOIN messages m ON m.seq = f.rowid WHERE messages_fts MATCH ?2 AND m.account_id = ?1 AND (?3 IS NULL OR m.peer = ?3) ORDER BY m.sent_at_ms DESC, m.seq DESC LIMIT ?4`.
  - The MATCH value is the query as an FTS5 string, `"\"" + q.replace('"', "\"\"") + "\""`. It
    is a **bound parameter value**, not SQL text.
  - Queries of 1–2 code points fall back to `m.body LIKE ?2 ESCAPE '\'`, with `%`, `_` and `\`
    escaped, and the same `account_id`/peer/order/limit clauses.
  - **FTS5 fallback rule:** first add the test `fts5_trigram_is_available`. If the bundled
    SQLCipher lacks FTS5/trigram, drop the virtual table, triggers and `optimize` step, use the
    `LIKE` path for every query length, and record that in the PR. Do not add a dependency.
- `HistoryEraser` impl, in one transaction:
  `DELETE FROM messages WHERE account_id = ?1 AND peer = ?2` (the delete trigger removes the FTS
  entries), **then** `INSERT INTO messages_fts (messages_fts) VALUES ('optimize')`. The external
  content `'delete'` command only adds tombstones, and `optimize` merges the segments so the
  erased trigram postings are gone from `messages_fts_data`. Combined with `secure_delete = ON`,
  the freed pages are zeroed. The erase is idempotent, and any SQL error maps to
  `HistoryEraseError`.
- Concurrency: one `Mutex<Connection>`; a poisoned lock → `Unavailable`. Every method blocks.

**Test doubles (`testing.rs`, `test-support`).** `MemoryMessageStore` has the exact same
contract:
- a monotonically increasing `seq`
- dedupe on `(account, peer, id)`
- Delivered is terminal
- `(sent_at_ms, seq)` keyset paging
- ASCII-only case-insensitive substring search
- the clamps
- it also implements `HistoryEraser`.

`FailingMessageStore` returns `Unavailable` / `HistoryEraseError` from every call.

**Re-exports (`mod.rs`).** `MessageStore`, `SqlCipherMessageStore`, `open_message_store`,
`StoredMessage`, `MessageDirection`, `MessageStatus`, `HistoryCursor`, `HistoryPage`,
`history_page`, `Insert`, `MessageStoreError`, `validate_body`, `DB_FILE`, `DEFAULT_PAGE`,
`MAX_PAGE`, `DEFAULT_SEARCH_RESULTS`, `MAX_SEARCH_RESULTS`, `MAX_SEARCH_QUERY_CHARS`, plus
`pub mod testing` under `#[cfg(any(test, feature = "test-support"))]`.

### 4. `contacts::NoMessageStore` docs

Keep the type, because headless and existing tests still use it. Reword its doc comment: "Eraser
for builds that persist no messages (the headless node and tests). The desktop app uses
`messaging::SqlCipherMessageStore`."

### 5. App wiring — the real store becomes the history eraser

`ContactsState` is built in `setup` before an identity may exist, but the store needs the
account's key. Add `app/src-tauri/src/chat_cmd.rs` with real content:

```rust
/// The encrypted message store, opened once the identity exists. Until then,
/// or if opening failed, history erase fails closed.
pub struct ChatState { store: OnceLock<(AccountId, Arc<SqlCipherMessageStore>)> }
impl ChatState {
    pub fn new() -> Self;
    /// Idempotent. Opens via `messaging::open_message_store(identity)`.
    pub fn open_store<K: KeychainStore, S: SettingsStore, T: TicketProvider>(&self, identity: &IdentityService<K, S, T>) -> Result<(), MessageStoreError>;
    pub fn store(&self) -> Option<(AccountId, Arc<SqlCipherMessageStore>)>;
}
impl HistoryEraser for ChatState { /* delegate to the store; no store → Err(HistoryEraseError) */ }
```

- `lib.rs`: create `let chat = Arc::new(ChatState::new());`. Replace `Arc::new(NoMessageStore)`
  with `app.manage(Arc::new(ContactsState::new(contacts_store, chat.clone())))`, and add
  `app.manage(chat)`. Remove the `NoMessageStore` import.
- `net_cmd.rs::start_if_present`: add a `chat: &ChatState` parameter, and call
  `chat.open_store(identity)` right after the `IdentityStatus::Present` match, before the node
  starts.
  - On error, run `eprintln!("message store unavailable: {e}")`, where `{e}` is the class-only
    `Display`, and continue the start (contacts and networking must stay usable).
  - Update `ensure_started` and every test call site.
  - The file is never deleted or recreated on failure.
- If the store fails to open, remove-with-erase returns `HistoryEraseFailed` and the contact is
  kept (existing `ContactBook::remove` ordering). That is the fail-closed behaviour.

### 6. Docs and tooling

- `docker/headless.Dockerfile`, build stage: the vendored OpenSSL build needs `perl` and `make`.
  Add `RUN apt-get update && apt-get install -y --no-install-recommends perl make && rm -rf /var/lib/apt/lists/*`
  before `cargo build`, and run `docker compose -f docker/compose.yaml build` once to confirm. The
  image is not part of the hook, but it must still build.
- `README.md`:
  - Layout table: `core` slices are now `identity`, `discovery`, `contacts`, `messaging`
    (encrypted message store).
  - Run-the-app prerequisites: a C compiler, `perl` and `make` (vendored OpenSSL for SQLCipher;
    on Windows, Strawberry Perl).
  - Contacts → Remove: replace "No message store exists yet…" with "Ticking it deletes the stored
    messages with that contact from the encrypted message database."
  - Add a short "Message storage" note: messages live in `messenger.db` in the (profile) data
    dir, encrypted with SQLCipher under the keychain DB key. A wrong key or a corrupt file is
    reported, and the file is never reset.
- `AGENTS.md`:
  - Layout row for `core/`: "currently `identity/`, `discovery/`, `contacts/` and `messaging/`".
  - Notes: `messaging::testing` provides `MemoryMessageStore` and `FailingMessageStore`.
  - Notes: "messages live in the SQLCipher `messenger.db` (DD-05 seam); never write bodies
    anywhere else".

## Guardrails

- AGENTS.md security rules are binding. **Never log message plaintext, keys or secrets.** NodeIds
  and addresses are fine. No `tracing` macro in `core/src/messaging/` may take a body, a query or
  a key/hex variable.
- `DbKey` / `SecretKeyBytes` must **never** cross into `app/`. Do not name them in `app/` code,
  types, tests **or comments** (the pre-commit grep gate rejects it). The store is opened inside
  `core` via `open_message_store`; `app` only sees `SqlCipherMessageStore` and
  `MessageStoreError`.
- Clipboard access stays only in `app/src/views/MyInfo.svelte`.
- `core` keeps `#![forbid(unsafe_code)]`. No `unsafe` and no raw FFI calls; use `rusqlite`'s safe
  API only.
- Never bypass the hook (`--no-verify`). Fix the failure instead.
- No empty files, folders, stubs or placeholders. Every new file above carries working code and
  tests.
- Wire format: unchanged in this step (limits only). Any change to how `Envelope`/`Payload` are
  encoded requires bumping `PROTOCOL_VERSION` and saying so in the PR.
- Every row and every query is namespaced by `account_id`. No query may lack
  `WHERE account_id = ?`.
- The `--profile` / `MESSENGER_PROFILE` override only relocates the data dir and the keychain
  namespace. It must not skip encryption, the key check or any other control.
- The `headless` crate and `FileKeychain` are never referenced from `app/`.
- **Never write credentials, keys, passwords or secrets in code, test fixtures, config or docs.
  All credentials/keys are provided at runtime (OS keychain / env); test keys are generated at
  runtime** (`rand::random::<[u8; 32]>()` or a freshly provisioned `FakeKeychain` identity). New
  tests must contain no hard-coded key bytes, not even "throwaway" ones.

## Security review

| Risk | Prevention | How to check |
|------|------------|--------------|
| Plaintext body on disk | The only home for bodies is the SQLCipher file. `temp_store = MEMORY`. Rollback-journal pages are encrypted by SQLCipher. No debug dumps. | `sqlcipher_file_never_contains_plaintext_body` scans `messenger.db` and any `messenger.db-journal`/`-wal` for a runtime-generated canary. `grep -rn "fs::write\|File::create" core/src/messaging` finds only the 0600 pre-create. |
| Plain SQLite linked by mistake | `PRAGMA cipher_version` must be non-empty, else `NotEncrypted`. | `sqlcipher_reports_cipher_version` |
| Wrong key or corrupt file silently reset | Open never deletes or recreates; `WrongKeyOrCorrupt` is a distinct error. | `wrong_key_is_reported_and_file_untouched` (the file's hash is unchanged after a failed open) |
| `DbKey` leak | `pub(crate)` accessor. Hex built by pushing nibbles into a pre-sized `Zeroizing<String>` (no `format!`/`collect` copies). Error `Display` is class-only. The key's `Debug` is redacted. | `grep -RnE 'SecretKeyBytes\|DbKey' app` is empty. `store_errors_never_embed_details`. Review: `grep -n 'format!\|collect' core/src/messaging/sqlcipher.rs` shows no key use. |
| SQL injection via body, query or NodeId | Constant SQL with bound parameters only. FTS phrase escaping and `LIKE` escaping. Optional filters use `(?n IS NULL OR …)`. | `grep -n "format!" core/src/messaging/sqlcipher.rs` finds no SQL built from data. `search_treats_sql_and_fts_syntax_as_text`. |
| Erased history recoverable (FTS segments or free pages) | Erase = DELETE (trigger tombstones) + FTS `'optimize'` in the same transaction, plus `secure_delete = ON`. | Checks run **with the key**: `erase_leaves_no_fts_terms` (fts5vocab has none of the canary's trigrams after erase; MATCH returns 0) and `erase_leaves_no_plaintext_in_export` (`sqlcipher_export` into a plaintext temp DB inside the test, then a raw scan for the canary). A raw scan of the encrypted file proves nothing and is not used for this. |
| Unbounded body or DB growth per row | `validate_body` (`MAX_BODY_BYTES`) in every insert. Limits clamped in the store. | `insert_rejects_empty_and_oversized_bodies`, `limits_are_clamped` |
| Cross-account leak | Every query filters `account_id`. | Contract suite `accounts_are_isolated` |
| Sender reorders history via `sent_at_ms` | **Accepted (DD-04: "sender timestamp + receive order")**: a peer can backdate or postdate where *its own* messages appear. `sent_at_ms` is never part of a unique key, dedupe or any security decision; `seq` breaks ties. | Contract `page_orders_by_sent_at_then_seq`. `grep -n sent_at_ms core/src/messaging/sqlcipher.rs` shows no UNIQUE or index other than the ordering index. |
| Bodies in logs | Row contents are never logged. The hand-written `Debug` redacts the body. | `stored_message_debug_redacts_body` |
| DB file readable by other users | 0600 on create; the data dir is already 0700. | `#[cfg(unix)] db_file_is_private` |

## Tests

All new tests use runtime-generated keys and temp dirs (`tempfile`).

`core/src/messaging/contract.rs` (`#[cfg(test)]`): `fn contract_suite(store: &dyn MessageStore)`,
run by `memory_store_meets_contract` (in `testing.rs` tests) and `sqlcipher_store_meets_contract`
(in `sqlcipher.rs` tests). It asserts:
- an insert round-trips every field (multi-line + emoji body, negative and huge `sent_at_ms`)
- `insert_incoming` of the same `(peer, id)` twice → `Duplicate`, one row, row unchanged;
  `insert_outgoing` duplicate → `AlreadyExists`
- `set_status` lifecycle: Sending→Pending→Delivered. Delivered→Failed is refused (`Ok(false)`).
  `Received` is refused. Incoming rows never change.
- `page_orders_by_sent_at_then_seq`: insert rows with `sent_at_ms` `[300, 100, 200, 100, 100]`.
  The full history is `100(seq2), 100(seq4), 100(seq5), 200, 300`.
- `keyset_paging_has_no_gaps_or_duplicates`: same data, `limit 2`. Walking `next_before` yields
  every row exactly once in order, including ties that straddle a page boundary. The last page is
  short and `next_before` is `None`.
- `history_cursor_token_round_trips`: `i64::MIN`/`i64::MAX` values; `parse_token` rejects junk,
  extra colons and >48 bytes.
- `undelivered` returns only outgoing Sending/Pending rows, oldest first by `seq`, bounded by
  `limit`, and accepts `usize::MAX` (`limits_are_clamped` also covers `page(limit = usize::MAX)`
  → `MAX_PAGE`).
- search:
  - a substring hit; ASCII case-insensitive; 1–2 char queries work
  - `%`, `_`, `"`, `'` and `--` are literal text
  - the peer filter works for `Some` and `None`; `limit` is respected
  - an empty or oversized query → `InvalidQuery`
  - **non-ASCII:** `"Grüße aus 日本 👋"` is found by `"üße"`, `"日本"` and `"👋"` (same case) in
    every implementation
- `accounts_are_isolated`: rows of account B are never returned for A, including by search and
  erase.
- erase removes only that peer's rows, is idempotent, and search no longer finds them.

`core/src/messaging/sqlcipher.rs` `#[cfg(test)]`:
- `sqlcipher_reports_cipher_version`
- `fts5_trigram_is_available` (or remove it together with FTS, per the fallback rule)
- `reopen_with_same_key_reads_rows`
- `wrong_key_is_reported_and_file_untouched`
- `newer_schema_is_refused` (set `user_version = 2` with the right key, reopen → `NewerSchema`)
- `unknown_status_text_reads_as_unavailable` (write `'queued'` with raw SQL in the test; reading
  the row → `Unavailable`, proving there is no SQL CHECK and Rust validates)
- `sqlcipher_file_never_contains_plaintext_body`: canary `format!("canary-{}", uuid::Uuid::new_v4())`,
  insert, drop the store, then raw-scan every `messenger.db*` file.
- `erase_leaves_no_fts_terms`:
  - Before the insert, collect the term set via `CREATE VIRTUAL TABLE temp.v USING fts5vocab(main, messages_fts, 'row')`.
  - Insert the canary row, erase, and collect the term set again. It equals the "before" set.
  - `MATCH` on the canary returns 0 rows.
- `erase_leaves_no_plaintext_in_export`: after the erase, inside the test only (temp dir, test
  data only):
  - `ATTACH DATABASE ?1 AS plaintext KEY ''`, then `SELECT sqlcipher_export('plaintext')`, then
    `DETACH`.
  - Raw-scan the exported plaintext file: it contains a still-present control body and does
    **not** contain the canary.
  - If `sqlcipher_export` refuses the FTS virtual table, export only `messages` via
    `CREATE TABLE plaintext.m AS SELECT * FROM main.messages`. Also hex-scan the `block` values of
    `messages_fts_data` for the canary's distinctive 3-byte trigrams, and note it in the PR.
- `#[cfg(unix)] db_file_is_private` (mode `0o600`)
- `insert_rejects_empty_and_oversized_bodies` (`MAX_BODY_BYTES + 1`, whitespace-only)
- `store_errors_never_embed_details`, `stored_message_debug_redacts_body`

`core/src/identity/service.rs` tests: `load_db_key_requires_provisioning`,
`load_db_key_returns_the_stored_key_for_the_marker_account`.

`core/tests/messaging_store.rs` (integration, through the public API):
- `open_message_store` on a provisioned identity (an `IdentityService` over `FakeKeychain` + a
  temp dir) creates `<data_dir>/messenger.db`. A second open succeeds and sees the rows: this is
  the store half of the **persistence round-trip**.
- With the keychain DB key deleted, the open fails with `Unavailable` and the file stays in
  place.

`app/src-tauri`:
- `chat_cmd.rs` tests: `erase_without_store_fails_closed` (`ChatState::new()` →
  `erase_peer_history` returns `Err`), `open_store_is_idempotent`.
- `contacts_cmd/tests.rs`: `remove_with_erase_deletes_stored_messages`.
  - Build `ContactsState::new(Arc::new(MemoryContactStore::new()), chat.clone())` directly, where
    `chat` is a `ChatState` opened on a provisioned test identity.
  - Add the contact. Insert one outgoing and one incoming row for the peer through `store()`.
  - `remove(.., Some(true))` → the store's `page` is empty. With `Some(false)`, the rows are kept.
  - The existing `test_util::started*` fixtures keep the `RecordingHistoryEraser` for the existing
    tests.
- `net_cmd.rs` tests: update all `start_if_present` call sites, and add
  `start_opens_message_store_when_identity_present`.

## Quality gates

Run from the repo root; all must pass:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
git config core.hooksPath .githooks   # then commit normally; the hook re-runs everything
```

New native dependency: `rusqlite` 0.42 with `bundled-sqlcipher-vendored-openssl` compiles
SQLCipher and OpenSSL from source.
- It needs a C compiler, `perl` and `make`. The first build is slow (minutes); incremental builds
  are not affected.
- If `cargo build` fails on `openssl-sys`, install `perl`/`make` (Fedora: `perl-core`). Do not
  switch to system SQLCipher and do not drop encryption.
- `cargo tree -i libsqlite3-sys` must list one version.
- Confirm `docker compose -f docker/compose.yaml build` still succeeds after the Dockerfile
  change.

PR (per `.github/PULL_REQUEST_TEMPLATE.md`):
- Affected: `protocol` (limits), `core` (identity, messaging, contacts docs), `app` (wiring),
  docs/tooling.
- **Protocol impact: no wire-format change; `PROTOCOL_VERSION` unchanged (limits only).**
- Verification: the commands above plus the `docker compose build`.

## Definition of done

- [ ] The DD-04 step files are committed on `04-messaging-and-delivery` (first commit of the DD branch) before this step's branch was cut.
- [ ] `protocol` has `MAX_BODY_BYTES`, `MAX_FRAME_BYTES` and `MAX_ACK_BYTES` with tests. `PROTOCOL_VERSION` is still 1, and the PR says the wire format is unchanged.
- [ ] `core::messaging` exists with real content only: model, `MessageStore`, `HistoryCursor`, `history_page`, `SqlCipherMessageStore`, `open_message_store`, the contract suite, and `MemoryMessageStore`/`FailingMessageStore` under `test-support`.
- [ ] The DB opens only with the keychain `DbKey` (zeroizing nibble-built hex), verifies `cipher_version`, and reports a wrong key, a corrupt file or a newer schema. It never recreates or deletes the file.
- [ ] Schema v1 matches this step: `UNIQUE(account_id, peer, message_id)`, the `(account_id, peer, sent_at_ms, seq)` index, the outbox index, FTS5 trigram (or the documented `LIKE` fallback), and no CHECK on `status`.
- [ ] History is ordered by `(sent_at_ms, seq)` and paged with a keyset cursor (opaque token at the IPC boundary), with no gaps or duplicates.
- [ ] All SQL uses bound parameters, every query filters `account_id`, and limits are clamped (`usize::MAX`-safe).
- [ ] Erase deletes rows and optimizes FTS; keyed checks (fts5vocab, `sqlcipher_export` scan) prove nothing remains. A raw-file scan proves bodies are never on disk in plaintext. The file is `0600` on Unix.
- [ ] The app uses the real store as its `HistoryEraser`, and erase fails closed while the store is unavailable. `NoMessageStore` is no longer used by `app/`.
- [ ] `DbKey` never appears under `app/` (grep gate green), and `core` has no new cargo feature.
- [ ] README (layout, prerequisites, removal wording, message storage note) and AGENTS.md (layout, testing doubles) are updated, and the Dockerfile builds.
- [ ] All quality gates pass via a normal commit with the hook installed.
