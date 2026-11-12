# Step 05-01 — Storage substrate (keyed open, typed errors, migrations) and the logging pipeline

**Estimate:** XL overall, split into two independent L sub-tasks.
- **1A Storage substrate:** ~720 LOC Rust prod, ~600 LOC Rust tests, ~180 LOC frontend (incl. ~110 LOC Vitest), ~40 LOC docs.
- **1B Logging pipeline:** ~480 LOC Rust prod, ~520 LOC Rust tests, 0 LOC frontend, ~50 LOC docs.

## TL;DR

DD-05 needs two cross-cutting substrates. This step builds both, in parallel:

- **1A** adds a `core::storage` slice that owns the single SQLCipher file `messenger.db`.
  It opens the file with the keychain DB key and runs an integrity check. Every failure
  becomes a **typed error state** (key missing, keychain denied, DB file missing, wrong
  key or corrupt, corrupt, newer schema, migration failed, I/O). None of them ever resets
  or recreates the file.
  It also adds a **versioned, forward-only, transactional migration runner**. The
  existing `messages` schema SQL becomes migration 1, byte-for-byte unchanged. When the
  file opens, one of three cases applies:
  - **Fresh install** (no file, provisioning in progress): the file is created and the
    runner applies migration 1.
  - **Healthy DD-04 file** (opens with the key, passes the integrity check, has
    `user_version = 1`): migration 1 is already applied, so nothing runs and no data
    changes. This no-op holds only for a well-formed DD-04 file.
  - **Inconsistent or unopenable file** (missing with an identity present, wrong key,
    corrupt, failed integrity check, newer schema, or a migration that fails and rolls
    back): this is a typed error. The file is left untouched, never migrated, reset or
    recreated. The error reaches the UI through `storage_status` / `storage_retry` and a
    new **"Encrypted storage unavailable"** screen with Retry. Nothing on that screen is
    secret.
- **1B** adds a `core::logging` slice. It is a `tracing-subscriber` pipeline that writes to
  a **size-bounded rotating file** in the OS app log directory, created `0700`/`0600` on
  Unix, with no network sink. The level is configurable, and control characters
  are neutralised. It also adds a reviewed set of field helpers and instruments the
  "what is logged" events: connection lifecycle, hole-punch/path events, relay status,
  discovery and errors. Finally, it turns the existing `core/tests/contacts_log_guard.rs`
  into a **workspace-wide redaction guard** and adds a runtime canary test. Both run in
  `cargo test --workspace`, which means they run in the pre-commit gate.

After this step, settings and contacts still use their interim plaintext stores. They
move in 05-02. `history_clear` also lands in 05-02.

## Settled decisions (confirmed by the user; do not reopen)

1. **When the DB cannot open, the app blocks until Retry.** It shows the full-screen
   "Encrypted storage unavailable" view with a Retry button. Networking, contacts and
   settings stay down until the DB opens: `start_if_present` does not start the node, and
   the UI shows nothing else.
2. **Logs identify peers by the short NodeId** (`NodeId::fmt_short()`, through
   `logging::fields::peer`), never the full key.

Two further decisions (narrow cleanup after a failed provisioning, and blocking on a
corrupt legacy file) belong to 05-02 and are recorded there.

## Dependencies / ordering

- **Before:** DD-04 is merged on `main` (it is: `3176cac`). Nothing else.
- **This step is a parallel group.** 1A and 1B touch disjoint core slices and can be built
  at the same time in separate worktrees by separate sub-agents.
- **After:** 05-02 depends on 1A (the storage handle, the runner and the error state) and
  on 1B (the guard must cover the new stores). 05-03 runs last.

### Worktree / sub-agent split

| Sub-task | Branch | Owns (creates or edits) |
|---|---|---|
| 1A | `step-05-01-storage-substrate-and-logging-1a` | **new** `core/src/storage/{mod.rs,error.rs,database.rs,migrate.rs,schema.rs,handle.rs,tests.rs}`, **new** `core/tests/storage_open.rs`, **new** `core/tests/sql_namespacing_guard.rs`; **edit** `core/src/messaging/{sqlcipher.rs,sqlcipher/tests.rs,mod.rs,model.rs,testing/peer.rs}`, `core/tests/messaging_store.rs`, `core/src/identity/{keychain.rs,service.rs}` (incl. the `FakeKeychain` test helper); **new** `app/src-tauri/src/storage_cmd.rs` (+ `storage_cmd/tests.rs`); **edit** `app/src-tauri/src/{lib.rs,chat_cmd.rs,chat_cmd/tests.rs,contacts_cmd/tests.rs,net_cmd.rs,identity_cmd.rs,test_util.rs}`; **new** `app/src/storage.ts`, `app/src/views/StorageUnavailable.svelte`, `app/src/views/StorageUnavailable.test.ts`; **edit** `app/src/App.svelte`, `app/src/App.test.ts`, `app/src/lib/router.ts`; README "Message storage" section; AGENTS.md "Layout"/"Notes" storage lines |
| 1B | `step-05-01-storage-substrate-and-logging-1b` | **new** `core/src/logging/{mod.rs,config.rs,writer.rs,fields.rs,tests.rs}`, **new** `core/tests/logging_canary.rs`; **rename + rewrite** `core/tests/contacts_log_guard.rs` → `core/tests/log_redaction_guard.rs`; **edit** `core/src/discovery/node.rs` (and `core/src/discovery/lan.rs` only if LAN events are logged there); **edit** root `Cargo.toml`, `core/Cargo.toml`, `app/src-tauri/Cargo.toml`, `app/src-tauri/src/profile.rs`; **edit** `app/src-tauri/src/lib.rs` (logging init only) and the `eprintln!` call sites in `app/src-tauri/src/{lib.rs,net_cmd.rs}`; README new "Diagnostic logs" section + checklist items; AGENTS.md logging bullet |

**Shared files (conflict hotspots), and who wins:**
- `core/src/lib.rs`: 1A adds `pub mod storage;` and 1B adds `pub mod logging;`. The
  conflict is trivial; keep both, in alphabetical order.
- `app/src-tauri/src/lib.rs`: 1A changes the store and keychain wiring inside `setup`. 1B
  adds the logging init at the top of `setup` and replaces the `eprintln!` lines.
- `app/src-tauri/src/net_cmd.rs`: 1A rewrites the `start_if_present` store-open block,
  which has an `eprintln!`. 1B converts every `eprintln!` it finds *after* rebasing.
- `README.md` / `AGENTS.md`: each sub-task edits only its own sections (listed above).
- `Cargo.lock`: regenerate with `cargo build` after merging. Never hand-edit it.

**Merge order:** first merge **1A** into the step branch. Then rebase **1B** onto the step
branch and merge it. 1B goes second because its guard and its `eprintln!` conversion must
cover the code 1A added: `tracing` calls in `core/src/storage/**` and the new app call
sites. After each merge, run the full quality gates on the step branch.

## Branch

Follow the DD-04 pattern (integration branch → step branch → sub-branches):

1. Integration branch `05-encrypted-storage-and-logging`, created from `main`. The DD-05
   step files are committed there first, in a commit titled "Add DD-05 step files".
2. Step branch `step-05-01-storage-substrate-and-logging` from the integration branch.
3. Sub-branches `step-05-01-storage-substrate-and-logging-1a` and `...-1b` from the step
   branch, each in its own worktree. Merge them back with `git merge --no-ff` in the order
   above.
4. PR: `step-05-01-storage-substrate-and-logging` → `05-encrypted-storage-and-logging`.

---

## What to do — Sub-task 1A: storage substrate

### 1A.1 The `core::storage` slice

Create `core/src/storage/` and add `pub mod storage;` to `core/src/lib.rs`. The slice owns
the DB file, keying, integrity checks, the error state and the migration runner. Feature
slices own **their own table SQL**, and storage only orders it. All SQL is constant text,
and every value goes through a bound parameter. This is the existing rule in
`messaging/sqlcipher.rs` and it now applies everywhere.

**`mod.rs`**: module docs stating the invariants:
- one encrypted DB per install;
- the key comes from the keychain only and is held only while the connection lives;
- failures are typed and never reset or recreate the file;
- migrations are forward-only and transactional;
- every row and every query is scoped by `account_id`. The documented exceptions are:
  - schema bookkeeping (`PRAGMA user_version`);
  - the FTS5 index `messages_fts`. It is an external-content table whose rows hold only
    `rowid` + `body`, and it is reached **only** through a join on
    `messages.seq = messages_fts.rowid` filtered by `messages.account_id` (see
    `SQL_SEARCH_FTS`). Its maintenance statement `SQL_FTS_OPTIMIZE` is allowlisted;
  - `legacy_imports`, which 05-02 adds and documents.

Re-export only the curated API:

```rust
mod database;
mod error;
mod handle;
mod migrate;
mod schema;
#[cfg(test)]
mod tests;

pub use database::Database;
pub use error::StorageError;
pub use handle::{KeySource, Storage, StorageStatus};
pub use schema::SCHEMA_VERSION;

/// The single encrypted database file in the (profile) data dir.
pub const DB_FILE: &str = "messenger.db";
```

Move `DB_FILE` here. Delete `messaging::DB_FILE` and its re-export. Update **every**
user to `storage::DB_FILE`. `rg -n 'DB_FILE' --type rust` today finds:
- `core/src/messaging/sqlcipher/tests.rs`;
- `core/tests/messaging_store.rs` (imports it);
- `app/src-tauri/src/chat_cmd/tests.rs` (imports it);
- `app/src-tauri/src/net_cmd.rs` (test `start_opens_message_store_when_identity_present`).

**`error.rs`**: one `Copy` enum with no payload. A payload could carry a backend string, a
path or key material, so there is none:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// No identity exists yet, so there is nothing to open.
    NotProvisioned,
    /// The DB key is missing from the keychain.
    KeyMissing,
    /// An identity exists but `messenger.db` does not, and creating one is not allowed
    /// (see "Creation policy"). Never silently recreated.
    Missing,
    /// The keychain is locked or the user refused access.
    KeychainDenied,
    /// Any other keychain failure (backend detail deliberately dropped).
    KeychainUnavailable,
    /// The SQLite library has no SQLCipher codec (`PRAGMA cipher_version` empty).
    NotEncrypted,
    /// The key does not decrypt the file, or its first page is damaged.
    WrongKeyOrCorrupt,
    /// The key decrypts, but the integrity check failed.
    Corrupt,
    /// The file was written by a newer app version.
    NewerSchema,
    /// Migration `version` failed; it was rolled back and the prior version kept.
    MigrationFailed { version: u32 },
    /// The handle is already open for a different account (v1 has one account).
    OtherAccount,
    /// File-system / SQLite I/O failure (permissions, disk full, symlink, lock).
    Io,
}
```

`Display` returns one fixed sentence per variant. It must not contain a path, a backend
message, a key, a byte count or the SQLite error text. Map `KeychainError` with
`NotFound → KeyMissing`, `AccessDenied → KeychainDenied`, `Backend(_) → KeychainUnavailable`.

**`database.rs`**: move the open sequence out of `SqlCipherMessageStore::open_with_key`,
then harden it:

```rust
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Opens the keyed DB at `path` and migrates it with `migrations`. Creates the file
    /// only when `create` is `Create::Allowed`; an absent file otherwise is `Missing`.
    /// Never truncates, deletes or recreates an existing file.
    pub(crate) fn open(path: &Path, key: &DbKey, migrations: &[Migration], create: Create) -> Result<Self, StorageError> {
        let had_data = ensure_private_regular_file(path, create)?;  // see below
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        // No SQLITE_OPEN_CREATE (the file exists already) and no SQLITE_OPEN_URI.
        let mut conn = Connection::open_with_flags(path, flags).map_err(|_| StorageError::Io)?;
        apply_key(&conn, key)?;                                       // moved verbatim
        let version: Option<String> =
            conn.query_row("PRAGMA cipher_version", [], |r| r.get(0)).ok();
        if version.is_none_or(|v| v.is_empty()) {
            return Err(StorageError::NotEncrypted);
        }
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .map_err(|_| StorageError::WrongKeyOrCorrupt)?;
        if had_data {
            integrity_check(&conn)?;
        }
        conn.execute_batch("PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON; PRAGMA foreign_keys = ON;")
            .map_err(|_| StorageError::Io)?;
        verify_session_pragmas(&conn)?; // secure_delete reads back 1, temp_store reads back 2, else Io
        migrate::run(&mut conn, migrations)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Crate-private: slices run their constant SQL through the guard.
    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.conn.lock().map_err(|_| StorageError::Io)
    }
}
```

- `apply_key`: keep today's construction exactly. It is a `Zeroizing<String>` built from
  the fixed `0123456789abcdef` alphabet into `PRAGMA key = "x'<64 hex>'";` and dropped
  right after `execute_batch`. Never use `format!`, `pragma_update` or any other
  string-building path for the key. Per the SQLCipher docs, a 64-hex blob literal is used
  as the raw 256-bit key with no KDF.
- `integrity_check`: run `PRAGMA cipher_integrity_check`. It returns **one row per
  problem**, so any row means `Corrupt`. Then run `PRAGMA quick_check`, where a first row
  other than `"ok"` also means `Corrupt`. Run both only when the file had data before this
  open: a just-created empty file has no pages to check. **Cost criterion:** measure the
  open time on a DB with 100 000 messages (add an `#[ignore]` benchmark-style test). If
  `cipher_integrity_check` adds more than ~1 s on a dev machine, keep only `quick_check`
  at startup and run the full `cipher_integrity_check` on `Storage::retry`. Write the
  decision and the measurement in the module docs.
- `pub(crate) enum Create { Allowed, Forbidden }`.
- `ensure_private_regular_file(path, create) -> Result<bool /*had_data*/, StorageError>`.
  It replaces `create_private_file`:
  - use `symlink_metadata`. A symlink, or anything that is not a regular file, gives `Io`;
    never follow it;
  - if the file is absent and `create == Forbidden`, return `StorageError::Missing`;
  - if the file is absent and `create == Allowed`, create it with `create_new(true)` and
    `mode(0o600)` on Unix;
  - if it exists on Unix with any group/other bits set, tighten it to `0o600`;
  - return whether `len() > 0`.
  - On non-Unix, keep relying on the per-user app-data ACLs, as today.
  - 05-02 (2A) extends this to also report whether this call created the file, for its
    provisioning cleanup. Do not add that in 1A.

**`migrate.rs`**: the runner. It is generic over a migration list so tests can feed it
synthetic migrations:

```rust
pub(crate) struct Migration {
    pub version: u32,          // 1, 2, 3, ... contiguous
    pub label: &'static str,   // e.g. "messages"; logged, never user data (not `name`: the guard forbids that identifier)
    pub sql: &'static str,     // constant SQL owned by the slice
}

/// Brings the DB to the last migration's version. Each migration runs in its own
/// IMMEDIATE transaction together with its `user_version` bump, so a failure rolls
/// back both and leaves the previous version intact. Re-running is a no-op.
pub(crate) fn run(conn: &mut Connection, migrations: &[Migration]) -> Result<u32, StorageError> {
    let latest = migrations.last().map_or(0, |m| m.version);
    let current: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| StorageError::Io)?;
    if current > latest {
        return Err(StorageError::NewerSchema);
    }
    for m in migrations.iter().filter(|m| m.version > current) {
        let failed = StorageError::MigrationFailed { version: m.version };
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failed)?;
        tx.execute_batch(m.sql).map_err(|_| failed)?;          // drop = rollback
        tx.pragma_update(None, "user_version", m.version).map_err(|_| failed)?;
        tx.commit().map_err(|_| failed)?;
        tracing::info!(version = m.version, migration = m.label, "storage migrated");
    }
    Ok(latest)
}
```

Add a `#[cfg(test)]` check (or a `debug_assert!` in `run`) that the versions are
contiguous from 1. A gap or a reordering is a programming error.

**`schema.rs`**: the ordered, global registry. Slices own the SQL constants.

```rust
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, label: "messages", sql: crate::messaging::MIGRATION_MESSAGES_V1 },
];
pub const SCHEMA_VERSION: u32 = 1; // must equal MIGRATIONS.last().version (unit-tested)
```

In `core/src/messaging/sqlcipher.rs`, rename `SCHEMA_V1` to
`pub(crate) const MIGRATION_MESSAGES_V1` and **keep its text byte-for-byte identical**.
DD-04 databases already carry `user_version = 1` with exactly this schema, so they need no
change and keep all their rows. Delete the old `migrate()` function, `SCHEMA_VERSION` and
the `user_version` handling from `sqlcipher.rs`.

**`handle.rs`**: a lazily opened, process-wide handle. 05-02 depends on its laziness: the
settings store must be able to open the DB during provisioning.

```rust
/// Loads the DB key for an account. Blanket-implemented for every keychain, so the
/// app passes its keychain `Arc` without ever naming the key type. The method is
/// deliberately not called `load_db_key`: with the blanket impl, two methods of that
/// name in scope would make `keychain.load_db_key(..)` ambiguous (E0034).
pub trait KeySource: Send + Sync {
    fn db_key(&self, account: &AccountId) -> Result<DbKey, KeychainError>;
}
impl<K: KeychainStore + Send + Sync + ?Sized> KeySource for K {
    fn db_key(&self, account: &AccountId) -> Result<DbKey, KeychainError> {
        KeychainStore::load_db_key(self, account)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageStatus { NotOpened, Ready, Failed(StorageError) }

pub struct Storage {
    data_dir: PathBuf,
    keys: Arc<dyn KeySource>,
    slot: Mutex<Slot>,
}
enum Slot { Closed, Open { account: AccountId, db: Arc<Database> }, Failed(StorageError) }

impl Storage {
    pub fn new(data_dir: impl Into<PathBuf>, keys: Arc<dyn KeySource>) -> Self;
    /// Opens on first call (keychain → key → `Database::open` with `schema::MIGRATIONS`).
    /// A failure is latched: later calls return it without touching the keychain
    /// again, until `retry`. A different account while open → `OtherAccount`.
    pub fn database(&self, account: &AccountId) -> Result<Arc<Database>, StorageError>;
    pub fn status(&self) -> StorageStatus;
    /// Clears a latched failure so the next `database` call opens again.
    pub fn retry(&self);
    /// Test fixtures only: like `database`, but always `Create::Allowed`. Lets
    /// fixtures that provision with `MemorySettingsStore` (no legacy file, marker
    /// already written) create the DB. Gated like `FakeKeychain`; the app never
    /// enables `test-support` (`app/src-tauri/tests/no_test_backends.rs`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn database_creating(&self, account: &AccountId) -> Result<Arc<Database>, StorageError>;
}
```

**Creation policy** (`Storage::database` decides `Create` before calling
`Database::open`). The DB file may be created **only** when:
- **provisioning is in progress:** `identity::read_marker(&data_dir)` returns `None`.
  `IdentityService::provision` writes the marker last, so any settings write during
  provisioning (05-02) happens before it; or
- **legacy plaintext data is being imported on an existing install:** `settings.json`,
  `contacts.json` or `known_peers.json` exists in `data_dir`. An install that predates
  DD-04 has an identity and these files but no `messenger.db` yet. In 1A, the app's
  `FileSettingsStore` always writes `settings.json`, so a freshly provisioned 1A identity
  takes this path. After 05-02 imports and removes the legacy files, only the
  provisioning path remains.

In every other case (an identity marker exists, there are no legacy files, and the file
is absent), return `StorageError::Missing`. It is latched and shown on the storage screen
like any other error. Never create an empty replacement: that would look exactly like
silent data loss.

On every open attempt, log one line: `tracing::info!(schema = SCHEMA_VERSION, "storage
opened")` or `tracing::warn!(error = %e, "storage unavailable")`. `e` is a `StorageError`,
so the line carries the class only. The latch exists because every attempt can raise an
OS keychain prompt; without it, each settings or contacts call would prompt again.

**Keychain sharing.** In `core/src/identity/keychain.rs`, add
`impl<T: KeychainStore + ?Sized> KeychainStore for Arc<T>`, which forwards all six
methods. **Delete** the now-conflicting `impl KeychainStore for Arc<FakeKeychain>` in
`testing`. Then `IdentityService<Arc<OsKeychain>, ..>` and `Storage::new(dir, keychain.clone())`
share one keychain handle. In `core/src/identity/service.rs`, delete the crate-private
`load_db_key` and `data_dir`, which become unused. Clippy `-D warnings` will flag them
otherwise. Keep `load_identity_secret`. **Delete** the two service tests that call the
removed method, `load_db_key_requires_provisioning` and
`load_db_key_returns_the_stored_key_for_the_marker_account`. Their coverage moves to
`core/tests/storage_open.rs`: `key_missing_is_reported` and
`opens_with_the_marker_accounts_key`.

**Test helper (`identity::testing::FakeKeychain`, `test-support`).** Add
`pub fn replace_db_key_with_random(&self, id: &AccountId)`, which overwrites the stored
DB-key entry with `rand::random::<[u8; 32]>()`. App tests use it to simulate a wrong
key. The grep gate forbids naming the key type under `app/`, so app tests must never
construct a key themselves.

### 1A.2 Rehome the message store

In `core/src/messaging/sqlcipher.rs`:
- `SqlCipherMessageStore { db: Arc<Database> }` with `pub fn new(db: Arc<Database>) -> Self`.
  Replace `self.conn()` with `self.db.conn()`, mapping `StorageError` to
  `MessageStoreError::Unavailable`.
- Delete `open_message_store`, `open_with_key`, `create_private_file` and `migrate`.
  Remove `open_message_store` from the `messaging/mod.rs` re-exports, and update the
  module docs: the "DD-05 seam" sentence becomes "the table lives in `storage`'s single
  DB as migration 1".
- In `messaging/model.rs`, remove the open-only variants `WrongKeyOrCorrupt`,
  `NewerSchema` and `NotEncrypted` from `MessageStoreError` if nothing else uses them
  (grep first). Keep `Unavailable`, `AlreadyExists`, `InvalidBody` and `InvalidQuery`.
- Move the open/keying/corruption tests from `messaging/sqlcipher/tests.rs` to
  `storage/tests.rs`: `sqlcipher_reports_cipher_version`, `fts5_trigram_is_available`,
  `wrong_key_is_reported_and_file_untouched`, `garbage_file_is_reported_and_left_in_place`,
  `newer_schema_is_refused`, `db_file_is_private`, `store_errors_never_embed_details`.
  Keep the contract, plaintext-scan and erase tests in messaging, now built via
  `Database::open(.., Create::Allowed)` + `SqlCipherMessageStore::new(..)`.
- Update `core/tests/messaging_store.rs` to open through `Storage` (`database_creating`
  for the first open, then `database` for the reopen).
- `core/src/messaging/testing/peer.rs` `sqlcipher_store` (it calls `open_message_store`
  today): build a `Storage` over the node's dir and `node.keychain.clone()`, then
  `SqlCipherMessageStore::new(storage.database_creating(&account)?)`.
- App test fixtures that called `chat.open_store(&identity)`:
  `app/src-tauri/src/test_util.rs` `started_with_chat`,
  `app/src-tauri/src/contacts_cmd/tests.rs` (the test around line 354), and
  `app/src-tauri/src/chat_cmd/tests.rs` (`provisioned`, `open_store_is_idempotent`,
  `open_store_fails_without_identity`). Each builds a `Storage` and calls
  `storage.database_creating(&account)` once before `chat.open_store(&storage, &account)`.

### 1A.3 Surface the storage state in the app

**`app/src-tauri/src/lib.rs` (setup)**:

```rust
let keychain = Arc::new(OsKeychain::native_for(layout.profile.as_ref())?);
let storage = Arc::new(Storage::new(data_dir.clone(), keychain.clone()));
let service: AppIdentity = IdentityService::new(keychain, Arc::clone(&settings), tickets.clone(), data_dir.clone());
// Open eagerly when an identity exists, so the first `storage_status` is already
// decided; a failure is latched and shown by the UI, never auto-retried.
if let IdentityStatus::Present { account_id, .. } = service.status() {
    let _ = storage_cmd::open_for(&storage, &chat, &account_id);
}
app.manage(storage);
```

`AppIdentity` in `identity_cmd.rs` becomes
`IdentityService<Arc<OsKeychain>, Arc<FileSettingsStore>, LiveTicketProvider>`. The
settings store changes again in 05-02.

**`chat_cmd.rs`**: `ChatState::open_store(&self, storage: &Storage, account: &AccountId) -> Result<(), StorageError>`
stays idempotent: it calls `storage.database(account)` and then
`SqlCipherMessageStore::new`. Erase still fails closed while no store is open.

**`net_cmd.rs` `start_if_present`**: add a `storage: &Storage` parameter. Bind contacts as
today, then `chat.open_store(storage, &account)`. **If that fails, return `false`, so the
node does not start** (settled decision 1: block until Retry). Replace the old comment ("a failed open leaves contacts and
networking usable") with: "A storage failure is a blocking, user-visible state (DD-05:
corruption is never a silent reset); the node starts after `storage_retry` succeeds."
Thread `storage` through `ensure_started` (`app.state::<Arc<Storage>>()`) and through the
tests (`test_util.rs`, which builds `Storage::new(dir, keychain.clone())`).

**`identity_cmd.rs` `identity_provision`**: after a successful `provision`, call
`storage_cmd::open_for(..)` for the new account, *before* `net_cmd::ensure_started`. The
storage state is then decided by the time the wizard finishes.

**New `app/src-tauri/src/storage_cmd.rs`**: plain functions plus thin
`#[tauri::command]` wrappers, following the same pattern as the other `*_cmd.rs` files.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StorageErrorKind { KeyMissing, Missing, KeychainDenied, KeychainUnavailable, NotEncrypted,
    WrongKeyOrCorrupt, Corrupt, NewerSchema, MigrationFailed, OtherAccount, Io }

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum StorageStatusDto { Ready, NotProvisioned, Error { kind: StorageErrorKind, reason: String } }

/// Opens the store for `account` (idempotent) and reports the outcome.
pub fn open_for(storage: &Storage, chat: &ChatState, account: &AccountId) -> StorageStatusDto;

/// `storage_status`: Present identity → `open_for`; Absent → NotProvisioned;
/// identity Error → NotProvisioned (the identity screen owns that state).
#[tauri::command] pub fn storage_status(..) -> StorageStatusDto;

/// `storage_retry`: `storage.retry()`, then `open_for`; on Ready calls
/// `net_cmd::ensure_started(&app)` (idempotent). Never deletes, resets or rekeys.
#[tauri::command] pub fn storage_retry(app: AppHandle, ..) -> StorageStatusDto;
```

`reason` is a fixed, honest string per kind, and none of them contains a path or detail.
For example:
- `WrongKeyOrCorrupt`: "The encrypted database could not be opened with the key in your
  system keychain, or the file is damaged. Nothing was deleted or reset."
- `MigrationFailed`: "Upgrading the database failed and was rolled back. Your data is
  unchanged."
- `NewerSchema`: "This database was written by a newer version of the app."
- `Missing`: "Your encrypted database file is missing from the app data folder. A new
  empty one was not created, so nothing is overwritten if you restore it."

There is **no** "reset database" or "start fresh" command, and none may be added in DD-05.
Register `storage_status` and `storage_retry` in `generate_handler!`.

**Frontend.**
- `app/src/storage.ts`: the `StorageStatusDto` type, `storageStatus()` and
  `storageRetry()` (`invoke("storage_status")` / `invoke("storage_retry")`).
- `app/src/lib/router.ts`: add `{ kind: "storage"; message: string }` to `AppView`.
- `app/src/App.svelte` `refresh()`: on `Present`, `await storageStatus()`. If it returns
  `Error`, set `appView` to `{ kind: "storage", message: s.reason }`; otherwise go to main.
  Change the wizard `onDone` to call `refresh()` instead of setting `main` directly, so a
  storage failure right after provisioning is shown.
- New `app/src/views/StorageUnavailable.svelte`:

```svelte
<script lang="ts">
  import { storageRetry } from "../storage";
  let { message, onResolved }: { message: string; onResolved: () => void } = $props();
  let busy = $state(false);
  let current = $state(message);
  async function retry() {
    busy = true;
    try {
      const s = await storageRetry();
      if (s.type === "Error") current = s.reason; else onResolved();
    } finally { busy = false; }
  }
</script>

<section role="alert" aria-labelledby="storage-title">
  <h2 id="storage-title">Encrypted storage unavailable</h2>
  <p>{current}</p>
  <p>Your messages, contacts and settings were not deleted, reset or replaced.</p>
  <button type="button" disabled={busy} onclick={retry}>Retry</button>
</section>
```

---

## What to do — Sub-task 1B: logging pipeline

### 1B.1 Dependencies

Root `Cargo.toml` `[workspace.dependencies]`:

```toml
tracing-subscriber = { version = "0.3", default-features = false, features = ["fmt", "std", "registry"] }
```

`core/Cargo.toml`: add `tracing-subscriber.workspace = true`. `app/src-tauri/Cargo.toml`:
add `tracing.workspace = true` (the app now logs its own events). Do **not** add
`env-filter`, `ansi`, `json`, `tracing-appender`, `tracing-journald`, `opentelemetry`,
`sentry` or any other sink. `tracing-subscriber 0.3.23` is already in `Cargo.lock`
through `iroh-relay`.

**Why not `tracing-appender`:** its `RollingFileAppender` rotates by **time** only
(minutely, hourly, daily or never), and `max_log_files` bounds the *count* of files, not
their *size*. A chatty hour or day can therefore grow one file without limit, which
breaks DD-05's "bounded total size / file count". A ~150-line size-based roller in
`core` guarantees the bound, adds no dependency, and keeps permissions and symlink
handling under our control. Synchronous writes under a mutex are fine at our volume.

### 1B.2 The `core::logging` slice

Add `pub mod logging;` to `core/src/lib.rs`.

**`config.rs`**:

```rust
#[derive(Debug, Clone)]
pub struct LogConfig {
    pub dir: PathBuf,           // OS app log dir (profile-namespaced by the app)
    pub level: LevelFilter,     // our crates; third-party crates are capped at WARN
    pub max_file_bytes: u64,    // default 2 MiB
    pub max_files: usize,       // default 5 (active + 4 rotated) → ≤ 10 MiB + one line
}
pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;
pub const MAX_LINE_BYTES: usize = 8 * 1024;

/// `MESSENGER_LOG`: off|error|warn|info|debug|trace (case-insensitive).
/// Unset → INFO. Anything else is an error: the app refuses to start, like
/// `MESSENGER_PROFILE` / `MESSENGER_RELAY_URL`.
pub fn level_from_env(raw: Option<&str>) -> Result<LevelFilter, LogConfigError>;
```

**`writer.rs`**: `RollingFile`, which implements `std::io::Write`. Wrap it in
`std::sync::Mutex`, which `tracing-subscriber` already implements `MakeWriter` for:

- **Open:**
  - Create `dir` with `DirBuilder::new().recursive(true)`, then on Unix
    `set_permissions(0o700)`.
  - Refuse a symlinked `dir` or a symlinked/non-regular `messenger.log`, checked with
    `symlink_metadata`.
  - Open with `create(true).append(true)` and `mode(0o600)` on Unix, then `set_permissions(0o600)`.
  - Compare `(dev, ino)` from the opened file's `metadata()` with a fresh
    `symlink_metadata(path)` (Unix), so a swap between check and open is detected.
  - Start `written` at the existing length.
- **Write** (one call = one formatted event, because the fmt layer buffers each event and
  calls `write_all` once):
  1. **sanitise**: decode lossily; replace every `char::is_control()` except the final
     `'\n'` (and `'\t'`), plus the bidi controls U+202A–U+202E and U+2066–U+2069, with
     `U+FFFD`; an interior `'\n'` becomes the two characters `\n`;
  2. truncate to `MAX_LINE_BYTES` at a char boundary and end with `'\n'`;
  3. if `written + len > max_file_bytes` and `written > 0`, **rotate**: delete
     `messenger.log.{max_files-1}`, rename `.{i}` → `.{i+1}` downwards, rename
     `messenger.log` → `messenger.log.1`, then open a fresh file as above;
  4. write, then add to `written`.
  Return `Ok(buf.len())`, so the formatter never retries a partial write. I/O errors are
  returned. The fmt layer ignores them, so logging can never crash or block the app.
- File names are constants. No path component is ever built from data.

**`fields.rs`**: the reviewed logging vocabulary. Log calls in `core` and `app` use these
helpers, or plain enums and counters, and nothing else:

```rust
/// Short NodeId (iroh `fmt_short`): enough to correlate, less metadata than the full key.
pub fn peer(id: &NodeId) -> impl fmt::Display + Copy { id.fmt_short() }
/// "relay:<url>" | "ip:<socketaddr>" | "custom" for an iroh `TransportAddr`.
pub fn transport(addr: &iroh::TransportAddr) -> TransportField;
/// "direct" | "relay" | "custom".
pub fn path_kind(addr: &iroh::TransportAddr) -> &'static str;
```

**`mod.rs`**:
- Module docs with the DD-05 lists: **logged** = connection lifecycle, hole-punch/path
  events, relay status, discovery events and errors, identified by short NodeId and
  address. **Never logged** = message bodies, search queries, display names and
  nicknames, the identity key, the DB key, and ticket strings.
- The public API:

```rust
/// The production filter: `core::` and `app_lib::` targets at `level`, everything
/// else capped at WARN (iroh at TRACE could otherwise flood or reveal internals).
pub fn filter(level: LevelFilter) -> Targets;
/// Builds the subscriber (file only, ANSI off) without installing it; used by tests.
pub fn subscriber(config: &LogConfig) -> Result<impl Subscriber + Send + Sync, LogInitError>;
/// Installs it as the global default. `level == OFF` installs nothing and creates no file.
/// Errors are I/O-class only (`LogInitError::Io`: dir/file cannot be created, symlink
/// refused, inode mismatch) or `AlreadyInstalled`. Invalid *configuration* never
/// reaches `init`: it is rejected earlier by `level_from_env` (`LogConfigError`).
pub fn init(config: &LogConfig) -> Result<(), LogInitError>;
```

Build it with
`tracing_subscriber::registry().with(fmt::layer().with_writer(Mutex::new(file)).with_ansi(false).with_target(true).with_filter(filter(level)))`.
Check `Targets` prefix semantics in the `tracing-subscriber 0.3.23` source. If
`with_target("core", ..)` would also match third-party targets such as `core_foundation`,
use `"core::"` instead. Either way, cover the case with a unit test.

### 1B.3 Wire it into the app

- `app/src-tauri/src/profile.rs`: add `log_dir` to `ProfileLayout`.
  `layout(base_data, base_log, profile)` returns `base_log/profiles/<name>` for a profile,
  so two instances never share a log file. Extend the existing `profile.rs` tests.
- `app/src-tauri/src/lib.rs` `setup`: right after the profile and relay are parsed,
  compute `app.path().app_log_dir()?` (Tauri v2 `PathResolver`; Linux:
  `$XDG_DATA_HOME/<identifier>/logs`, macOS: `~/Library/Logs/<identifier>`, Windows:
  `%LOCALAPPDATA%\<identifier>\logs`). Then call
  `logging::level_from_env(std::env::var("MESSENGER_LOG").ok().as_deref())?`. A
  `LogConfigError` from it refuses to start (fail closed, like the other env vars). Then
  call `logging::init(&config)`. `init` can only fail with an I/O-class `LogInitError`.
  In that case, continue without a file sink, because diagnostics must never brick the
  app, and report it with **the single allowed stderr write in the app**, a fixed byte
  string with no formatting:

  ```rust
  let _ = std::io::Write::write_all(
      &mut std::io::stderr(),
      b"p2p-messenger: diagnostic log file unavailable; continuing without file logging\n",
  );
  ```

  This is the sole exception to the "no direct stdout/stderr output" rule. The redaction
  guard allowlists exactly this line in `app/src-tauri/src/lib.rs` (see 1B.5).
- Replace every `eprintln!` in `app/src-tauri/src/{lib.rs,net_cmd.rs}` (including the
  ones 1A touched) with `tracing` events that carry counts or error classes only. For
  example, `tracing::info!(count = r.migrated, "legacy known peers migrated")`.

### 1B.4 Instrument the "what is logged" list (`core/src/discovery/node.rs`)

All of these are at `info` unless noted. They use only `logging::fields` helpers,
`Direction`/`ConnectionState`/`FailureReason` (via `?`) and counts.
- **Node lifecycle:** `node started` (`node = peer(own)`, `relay = <Default|Disabled|Custom url>`,
  `lan = <mode>`, `bound = ?endpoint.bound_sockets()`). Add `node stopping` in `Inner::close`.
- **Connection lifecycle:** one event inside `Inner::emit_state`, and the same in
  `emit_connected`, **without** the display name: `connection state` with `peer`,
  `direction = ?d`, `state = ?s`.
- **Dial / hole-punch attempt:** in `dial` before connecting, `dialing` with `peer`,
  `direct_addrs = addr.ip_addrs().count()` and `relay = ?addr.relay_urls().next()`.
- **Path / hole-punch outcome:** in `authorize`, spawn a task on the cloned connection's
  `conn.path_events()` (iroh 1.4: `Opened`, `Closed`, `Selected` and `Lagged`; the stream
  ends when the connection closes):
  - `Opened` → `path opened` (`peer`, `kind = path_kind(..)`, `addr = transport(..)`);
  - `Selected` → `path selected`. A selected `direct` path is logged as "direct path
    selected (hole punch succeeded)";
  - `Closed` → `debug`;
  - when the stream ends, log `hole punch outcome` with `outcome = "direct" | "relay_only"`,
    depending on whether any IP path was ever opened.
- **Relay usage:** spawn a watcher on `endpoint.home_relay_status()`
  (`RelayStatus::url()`, `is_connected()`, `last_error()`). Log `home relay` with
  `relay = %url` and `connected`. On a failure, log at `warn` with `error = %e`; the
  `AnyError` text names the relay, which is fine. Stop the task on `endpoint.closed()`.
- **Discovery:** presence toggled (`presence = on|off`), ticket changed (`debug`), LAN peer
  appeared or vanished (`debug`, `peer` only, **never the name**).
- **Errors:** keep the existing `warn!` lines.

### 1B.5 The redaction guard (generalise, don't duplicate)

Run `git mv core/tests/contacts_log_guard.rs core/tests/log_redaction_guard.rs` and
rewrite the file. Keep `rust_files` and `argument_text`, and add the following:

- **Scope:** every `.rs` file under `core/src`, `app/src-tauri/src` and `headless/src`
  (from `env!("CARGO_MANIFEST_DIR")/..`). Assert each root is non-empty.
- **Macros:** `trace! debug! info! warn! error! event! span! trace_span! debug_span!
  info_span! warn_span! error_span!`, with or without the `tracing::` prefix.
- **`#[instrument`** / `#[tracing::instrument`: forbidden unless the attribute contains
  `skip_all`, because instrument records every argument with `Debug`.
- **Direct output:** `println! eprintln! print! eprint! dbg!` and any use of
  `std::io::stdout` / `std::io::stderr` are forbidden in `core/src` and
  `app/src-tauri/src`, except in `tests.rs`, `tests/` and `testing/` paths. The **only**
  allowlisted line is the fixed-string `write_all` fallback in `app/src-tauri/src/lib.rs`
  from 1B.3. Match it by its exact byte-string literal, so any change to that line, or a
  second such line, fails the guard. `headless/src` is exempt from this rule; it is
  test-only and writes to stderr by design.
- **Field allowlist.** Every field name (`name =`, `name = %`, `name = ?`, and the
  `%ident` / `?ident` / bare `ident,` shorthands) must be one of: `peer`, `node`,
  `direction`, `state`, `reason`, `error`, `attempts`, `count`, `kind`, `addr`, `relay`,
  `connected`, `outcome`, `direct_addrs`, `bound`, `lan`, `presence`, `version`,
  `migration`, `schema`.
- **Forbidden identifiers.** These apply **only inside the argument text** of a log macro
  (the balanced delimiters after the macro name) and inside a `#[instrument(..)]`
  attribute. Ordinary code may of course use `body`, `key` and so on. Within that
  argument text, an identifier token outside string literals, and any `{ident}` inline
  capture inside a literal, must not be one of: `body`, `text`,
  `plaintext`, `query`, `nickname`, `self_name`, `display_name`, `name`, `ticket`,
  `secret`, `key`, `db_key`, `DbKey`, `SecretKeyBytes`, `as_bytes`, `hex`, `passphrase`,
  `payload`, `hello`, `msg`, `message`, `contact`.
- **Word-boundary matching.** Match on identifier boundaries instead of substrings, so
  `nobody` and `keyed` do not trigger. The forbidden list blocks the dangerous names, and
  the allowlist blocks everything not yet reviewed.
- **Self-tests**, which cover the "fails the guard" Testing bullet. Each planted snippet
  must be rejected:
  - `debug!(body = %m.body)`
  - `info!("{}", chat.body)`
  - `warn!(key = ?db_key)`
  - `trace!(secret = ?k.as_bytes())`
  - `info!(name = %display_name)`
  - `info!("hi {nickname}")`
  - `#[instrument] fn f(x: Y)`
  - `eprintln!("{body}")` in a non-test path

  and accepted:
  - `info!(peer = %peer(&id), addr = %transport(&a), "path opened")`
  - `warn!(error = %e, "storage unavailable")`
  - the exact allowlisted stderr fallback line.

  Plain string literals are fine for these fixtures. The guard scans only `core/src`,
  `app/src-tauri/src` and `headless/src`, so its own file in `core/tests/` is never
  scanned. The pre-commit `DbKey`/`SecretKeyBytes` grep scans only `app/`.

### 1B.6 Runtime canary (`core/tests/logging_canary.rs`)

Put exactly one `#[tokio::test(flavor = "multi_thread")]` in its own test binary, so it
can call `tracing::subscriber::set_global_default`. The subscriber is
`logging::subscriber(&LogConfig { level: TRACE, dir: tempdir, small bounds })`, which uses
**the production filter**. Then:
- spawn two loopback nodes with the messaging harness from `core/tests/messaging_two_nodes.rs`;
- use display names and nicknames `canary-name-<uuid>`, a chat body `canary-body-<uuid>`
  and a search query `canary-query-<uuid>`;
- open a `Storage` over a `FakeKeychain`, then read the DB key bytes and the identity key
  bytes back from the fake to hex-encode them as needles. These are generated at runtime,
  never committed;
- add each node's **ticket string** (`node.ticket()?.to_string()`) as a needle. Tickets
  are on the never-logged list;
- read the whole log directory.

Assert that no needle appears, in plain or hex form. Assert positively that a
`connection state` line contains `peer(&other)` for both directions, and a `path opened`
line has an `addr`. This also covers DD-05's "a normal connection event logs NodeId +
address".

### 1B.7 Docs

- README new section **"Diagnostic logs"**: the location per OS and per profile; the
  default level and `MESSENGER_LOG`; the size bound (5 × 2 MiB); the "never logged" list;
  "logs are local only and never sent anywhere"; and "logs reveal *who* you connected to
  and *when* (short NodeIds, IP addresses). Deleting the log folder is a privacy action and
  is safe at any time."
- AGENTS.md: a bullet "Logging goes through `core::logging` helpers. The redaction guard
  (`core/tests/log_redaction_guard.rs`) is enforced; extend its allowlist only with
  reviewed, non-sensitive field names."

---

## Guardrails

- Follow AGENTS.md conventions:
  - vertical slices with curated `mod.rs` re-exports;
  - only files with real content: no stubs, placeholder modules, empty files or empty
    folders. Every new module above carries working code and tests;
  - `#![forbid(unsafe_code)]` stays in `core`, so no `unsafe` anywhere. `O_NOFOLLOW` via
    libc is not needed: use the `symlink_metadata` + inode comparison;
  - `account_id` on every data row and in every data query;
  - `core` knows nothing about Tauri.
- `SecretKeyBytes` and `DbKey` must **never** appear under `app/`, not even in comments,
  type names, test names or strings. The `KeySource` blanket impl exists so the app never
  names the key type. Run the pre-commit grep yourself before committing.
- `FileKeychain` and `messenger-headless` must never be referenced from `app/`.
- Clipboard access stays only in `app/src/views/MyInfo.svelte`.
- Never write credentials, keys, passphrases or tokens in code, test fixtures committed as
  real secrets, config or docs. All credentials and keys are obtained at runtime: from the
  OS keychain, or generated in tests (`rand::random`, `FakeKeychain`).
- Never run `git commit --no-verify`. Fix the failure instead.
- The wire format does not change: `PROTOCOL_VERSION` stays as is and `protocol/` is not
  touched.
- Do not add a "reset/recreate database" path, even behind a flag. `database_creating`
  exists only under `test-support`.
- **Logs never contain message plaintext, private keys or secrets.** The same goes for
  display names, nicknames, search queries and ticket strings. This is the AGENTS.md
  security rule, enforced here by the redaction guard and the canary. Do not relax either
  to make a test pass.
- **The profile override must not weaken any control.** `--profile` /
  `MESSENGER_PROFILE` now also selects the log directory. It changes *where* files go,
  never *whether* a control applies:
  - the log dir under a profile gets the same `0700`/`0600` modes, symlink refusal,
    size bound and redaction;
  - the storage creation policy, integrity check and typed errors are identical;
  - an invalid profile still refuses to start.

## Security

| # | Risk in this step | Prevention | How to check |
|---|---|---|---|
| 1 | DB key leaks via `Debug`/`Display`/error strings/logs/Tauri returns | `StorageError` is a payload-free `Copy` enum; the key exists only as `DbKey` + `Zeroizing<String>` inside `database.rs`; `StorageStatusDto` carries static reasons | `storage/tests.rs::storage_errors_never_embed_details` (Display/Debug of every variant contains no hex of a random key, no path, no "sqlite"); `storage_cmd/tests.rs::status_json_never_contains_db_key` (reuse `test_util::assert_forms_absent` with the `FakeKeychain` key); the pre-commit `DbKey` grep |
| 2 | PRAGMA key formatting/injection | Key text built only from the fixed hex alphabet; no user string ever reaches a PRAGMA; no `SQLITE_OPEN_URI` | Unit test: keys containing bytes `0x22`, `0x27`, `0x00` round-trip; `rg -n 'format!\(.*PRAGMA' core/src` returns nothing |
| 3 | Silent DB recreation on wrong key / corruption / newer schema | `Database::open` never truncates/deletes; open without `CREATE`; typed error is latched | Tests `wrong_key_is_reported_and_file_untouched`, `garbage_file_is_reported_and_left_in_place`, `newer_schema_is_refused` assert the SHA-256/bytes of the file are unchanged after the failed open |
| 3b | Missing DB silently recreated empty (indistinguishable from data loss) | Creation policy: create only while provisioning (no marker) or while importing legacy JSON; otherwise `Missing`, latched and shown | `missing_db_with_identity_is_reported_not_recreated` (no file exists afterwards); `create_allowed_without_marker`; `create_allowed_with_legacy_files` |
| 4 | Migration partial failure | One IMMEDIATE transaction per migration including the `user_version` bump | `failing_migration_rolls_back_and_keeps_version` (below) |
| 5 | Plaintext residue in temp/journal/free pages | `temp_store=MEMORY`, `secure_delete=ON` verified by read-back; SQLCipher encrypts journal/WAL pages | `session_pragmas_are_enforced`; the existing raw-file canary scan over every `messenger.db*` file |
| 6 | Symlink / TOCTOU on DB path | `symlink_metadata` refuses links and non-regular files; data dir is `0700` (existing) | `symlinked_db_is_refused_and_target_untouched` (Unix) |
| 7 | World-readable DB | `0600` on create; existing loose modes tightened | `db_file_is_private`, `loose_db_permissions_are_tightened` (Unix) |
| 8 | Cross-account reads | `Storage` refuses a second account (`OtherAccount`); static namespacing guard | `other_account_is_refused`; `core/tests/sql_namespacing_guard.rs` (below) |
| 9 | Keychain prompt storm / DoS on failure | Failure latched until explicit `storage_retry` | `failure_is_latched_until_retry` counts `FakeKeychain` loads |
| 10 | Destructive action from the webview | No reset/delete command exists; `storage_retry` only re-opens | Review `generate_handler!`; `retry_never_modifies_the_file` |
| 11 | Secrets/bodies/names/tickets in logs or on stderr | Redaction guard (allowlist + forbidden identifiers + `#[instrument]` rule + direct-output ban with one exact allowlisted line) and runtime canary (bodies, names, query, both keys, tickets) | `cargo test -p core --test log_redaction_guard --test logging_canary` |
| 12 | Log injection (newlines, ANSI, bidi from peer-influenced strings such as relay URLs or error text) | `with_ansi(false)` + writer sanitisation + line cap | `writer_neutralises_control_and_bidi_chars`, `writer_caps_line_length` |
| 13 | World-readable logs | Dir `0700`, files `0600`, re-applied on rotation | `log_files_and_dir_are_private` (Unix) |
| 14 | Symlink attacks on log path | Refuse symlinked dir/file; inode check after open; constant file names | `symlinked_log_file_is_refused` (Unix) |
| 15 | Unbounded log growth | Size-based roller, `max_files`, `MAX_LINE_BYTES` | `rotation_keeps_total_size_bounded` |
| 16 | Network sink | No remote sink crates; logging module uses only `std::fs` | `logging_has_no_network_sink` (below) |
| 17 | Third-party crates logging sensitive internals | Production filter caps non-`core`/`app` targets at WARN | `filter_caps_third_party_targets`; the canary runs with the production filter |

## Tests

**1A (map to DD-05 Testing):**
- `core/src/storage/tests.rs`:
  - *Keyed open round-trip:* `create_then_reopen_with_same_key`,
    `wrong_key_is_reported_and_file_untouched` (returns `WrongKeyOrCorrupt`, never `Ok`,
    and the file bytes are unchanged).
  - *Encrypted at rest:* keep `sqlcipher_file_never_contains_plaintext_body` (moved or
    kept in messaging).
  - *Migration forward:* `forward_migration_keeps_rows`. It builds a DB with only
    `MIGRATIONS[..1]` plus two synthetic later migrations (`ALTER TABLE ... ADD COLUMN`,
    `CREATE TABLE`), inserts rows at v1, reopens with the full list, and asserts the rows
    are intact and `user_version` is the latest.
  - `dd04_database_opens_unchanged`: create the DB exactly as DD-04 did (raw `rusqlite`:
    key, the old SQL text, `PRAGMA user_version = 1`), insert messages, open through
    `Storage`, then page through them.
  - `failing_migration_rolls_back_and_keeps_version`: synthetic v2 =
    `CREATE TABLE t(x); INSERT INTO missing VALUES (1);`. Expect `MigrationFailed { version: 2 }`,
    `user_version` still 1, and table `t` absent.
  - `rerun_is_noop`.
  - `migrations_are_contiguous_and_match_schema_version`.
  - `failure_is_latched_until_retry`, `other_account_is_refused`.
  - `missing_db_with_identity_is_reported_not_recreated`: write a marker, no legacy
    files, no `messenger.db`. Expect `Missing`, and the file still absent afterwards.
    Also `create_allowed_without_marker` and `create_allowed_with_legacy_files` (a
    `settings.json` is present).
  - The Unix permission and symlink tests from the Security table.
  - `session_pragmas_are_enforced`, `storage_errors_never_embed_details`.
- `core/tests/storage_open.rs`: an end-to-end `Storage` over `FakeKeychain` (missing key →
  `KeyMissing`; `fail_with(AccessDenied)` → `KeychainDenied`), with the file left in place.
  It also covers the two deleted service tests: `key_missing_is_reported` and
  `opens_with_the_marker_accounts_key`.
- `core/tests/sql_namespacing_guard.rs` (*Namespacing*, static half):
  - scan `core/src/**/*.rs` (excluding test files) for `const SQL_[A-Z0-9_]*: &str = ... ;`
    items, joining `concat!` parts, and assert that each contains `account_id`.
  - explicit allowlist: `SQL_FTS_OPTIMIZE` (FTS maintenance, see the `messages_fts`
    exception). Also flag any `.execute("`, `.prepare("`, `.prepare_cached("` or
    `.query_row("` whose inline literal starts with `SELECT|INSERT|UPDATE|DELETE`, except
    `PRAGMA` and the `sqlite_master` probe.
  - Also check every `CREATE TABLE` and `CREATE VIRTUAL TABLE` in the `MIGRATION_*`
    constants. Each must declare an `account_id` column, except an explicit allowlist:
    `messages_fts` here, plus `legacy_imports` when 05-02 adds it.
  - include a self-test with a planted violating const and a planted table without
    `account_id`.
- `app/src-tauri/src/storage_cmd/tests.rs`:
  - `status_is_not_provisioned_without_identity`;
  - `status_ready_after_provision`;
  - `wrong_key_surfaces_error_and_keeps_file`: provision, write a message, call
    `keychain.replace_db_key_with_random(&account)` (the core `test-support` helper; the
    app test never constructs a key), use a fresh `Storage`, and expect `Error` with kind
    `WrongKeyOrCorrupt` and an unchanged file;
  - `missing_db_surfaces_missing`;
  - `retry_recovers_after_key_restored`;
  - `status_json_never_contains_db_key`;
  - `start_if_present_does_not_start_node_when_storage_fails`.
  Update the existing `net_cmd` / `chat_cmd` tests for the new signatures.
- Vitest:
  - `StorageUnavailable.test.ts`: renders reason + reassurance; Retry calls `storageRetry`;
    an `Error` result updates the text; `Ready` calls `onResolved`.
  - `App.test.ts`: Present + storage `Error` shows the screen, not main; Retry leading to
    `Ready` shows main; the wizard `onDone` re-checks storage. Mock `./storage` the way
    `./identity` is mocked.

**1B:**
- `core/src/logging/tests.rs`:
  - `level_from_env_accepts_known_and_rejects_others`;
  - *Log rotation:* `rotation_keeps_total_size_bounded`. Use `max_file_bytes = 4 KiB`,
    `max_files = 3`, and write 200 KiB of events. Assert at most 3 files, each
    ≤ 4 KiB + `MAX_LINE_BYTES`, and that the newest content is in `messenger.log`;
  - `writer_neutralises_control_and_bidi_chars` (`"\x1b[31m"`, `"a\nb"`, U+202E);
  - `writer_caps_line_length`;
  - `log_files_and_dir_are_private`, `symlinked_log_file_is_refused` (Unix);
  - `filter_caps_third_party_targets`: `iroh::x` at INFO is disabled at TRACE level,
    `core::x` at TRACE is enabled, and a `core_foundation` target is capped;
  - `off_level_creates_no_file`.
- *No network sink:* `logging_has_no_network_sink`.
  - Scan only the sink code, `core/src/logging/{writer.rs,mod.rs,config.rs}`. Skip
    `fields.rs`: it legitimately formats `RelayUrl` and addresses.
  - Strip `//` comments first, then match on identifier/path boundaries for `std::net`,
    `tokio::net`, `TcpStream`, `UdpSocket`, `reqwest`, `hyper`, `ureq`. A bare substring
    `Url` or `http` would hit `RelayUrl` and doc text.
  - Scan the root, core and app `Cargo.toml` dependency names for `opentelemetry`,
    `sentry`, `loki`, `gelf`, `syslog`, `journald`, `datadog` and `tracing-appender`.
  - Assert none are present.
- *Log redaction guard (automated):* `core/tests/log_redaction_guard.rs` (self-tests + the
  workspace scan).
- *Normal connection event logs NodeId + address:* `core/tests/logging_canary.rs`.
- `app/src-tauri/src/profile.rs` tests: `log_dir_is_profile_namespaced`.

## Quality gates

Run in each worktree before merging, then again on the step branch after both merges:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

The hook must be installed (`git config core.hooksPath .githooks`), and every commit must
pass through it normally. **Never** use `--no-verify`. The hook is not changed in this
step. If you change it anyway, verify it as AGENTS.md describes: stage a file under `app/`
containing a secret-looking identifier and confirm the commit is rejected.

## Usable-state guarantee

- `npm start` launches the app. A fresh profile goes through the wizard, which now ends
  with a storage check. An existing DD-04 profile opens its unchanged `messenger.db`
  (migration 1 is a no-op), and chat, history and search work as before.
- Contacts and settings still use their interim stores, unchanged, until 05-02.
- **The new screen is reachable:** with a test profile, overwrite `messenger.db` with
  random bytes (`head -c 8192 /dev/urandom > <profile dir>/messenger.db`) and launch. The
  "Encrypted storage unavailable" screen shows, the file is untouched, and Retry after
  restoring a backup copy returns to the main screen. Moving `messenger.db` away from a
  profile that has an identity shows the same screen with the "missing" reason, and no
  new file is created. Add both to the README manual checklist.
- A log file appears in the OS log dir, `0600` inside a `0700` dir, with `connection
  state` / `path opened` lines when two instances connect. Add this to the checklist.
- `MESSENGER_LOG=bogus npm start` refuses to start with a clear message, like an invalid
  profile.

## Definition of done

- [ ] `core::storage` exists with `Database`, `Storage`, `KeySource`, `StorageError`,
      `StorageStatus` and the migration runner. The messages schema is migration 1,
      byte-identical to DD-04's.
- [ ] Wrong key, garbage, newer schema and a failed migration each produce their typed
      error, and the file is left byte-identical (tested). A missing DB with an
      identity is `Missing` and is never recreated (tested).
- [ ] `KeySource::db_key` (not `load_db_key`). `FakeKeychain::replace_db_key_with_random`
      exists, and no app test constructs a key.
- [ ] The integrity check runs on open. The cost decision is written in the module docs.
- [ ] `storage_status` / `storage_retry` exist. The storage-error screen is reachable and
      tested (Rust + Vitest). No reset command exists.
- [ ] `core::logging` writes to a size-bounded rotating file (`0600`/`0700`), has no
      network sink, neutralises control and bidi characters, and is configurable via
      `LogConfig` + `MESSENGER_LOG`.
- [ ] Connection lifecycle, dial/path/hole-punch outcome, home relay status, discovery
      and error events are logged with short NodeIds and addresses.
- [ ] `log_redaction_guard.rs` replaces `contacts_log_guard.rs`, scans core, app and
      headless, has self-tests, and runs in `cargo test --workspace`.
      `logging_canary.rs` passes.
- [ ] `sql_namespacing_guard.rs` passes.
- [ ] All `eprintln!` in `app/src-tauri/src` are replaced with guarded `tracing` events.
      The only direct stderr output is the single fixed-string log-init fallback in
      `lib.rs`, allowlisted by exact match in the guard.
- [ ] README ("Message storage", "Diagnostic logs", manual checklist) and AGENTS.md are
      updated.
- [ ] All quality gates pass on the step branch, and commits went through the hook.
- [ ] PR to `05-encrypted-storage-and-logging` follows `.github/PULL_REQUEST_TEMPLATE.md`:
      the affected slices are `core` storage/logging/messaging/identity/discovery and
      `app`, "No change to the wire format" is ticked, and the verification commands and
      manual checks are listed.
