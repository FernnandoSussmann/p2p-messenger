# Step 05-02 — Settings and contacts move into SQLCipher; `history_clear` with page reclaim

**Estimate:** L overall, split into two independent sub-tasks.
- **2A Settings + contacts into the encrypted DB:** L. About 850 LOC of Rust production
  code added, and about 450 LOC of interim file-store writer code removed. About 700 LOC
  of Rust tests, including the updated core, headless and app tests. 0 LOC of frontend,
  and about 50 LOC of docs.
- **2B `history_clear`:** M. About 130 LOC of Rust production code, about 250 LOC of Rust
  tests, about 160 LOC of frontend code (including about 120 LOC of Vitest), and about
  30 LOC of docs.

## TL;DR

- **2A** moves the last plaintext user data into `messenger.db`:
  - Add `SqlCipherSettingsStore` (display name, `provisioned_at`, presence, relay
    override) and `SqlCipherContactStore` (DD-03 contacts). Their tables are new
    migrations on the 05-01 runner, and every row and every query carries `account_id`.
  - On the first open, a one-shot **legacy import** copies `settings.json`,
    `contacts.json` and `known_peers.json` into the DB. Each file is imported inside a
    transaction. That transaction also records the import, so a contact the user later
    revoked is never re-imported.
  - The plaintext files are overwritten and removed only **after** the commit. A corrupt
    `settings.json` or `contacts.json` is a typed, user-visible error that imports
    nothing. A corrupt `known_peers.json` is skipped and left on disk, which is
    documented.
  - A failed provisioning cleans up only its own empty DB and never leaves the app
    stuck.
  - The app and the test-only headless node are rewired. `MemorySettingsStore` and
    `MemoryContactStore` stay as test doubles. The README and AGENTS.md lines that say
    "contacts live in plaintext" are updated.
- **2B** adds the `history_clear` Tauri command. The command is explicit and confirmed. It
  goes through the **same** `HistoryEraser::erase_peer_history` path that the DD-03
  "Remove contact → also delete message history" option already uses:
  - The rows and their FTS postings are deleted.
  - The pages are reclaimed: `secure_delete=ON` zero-fills freed content, then `VACUUM`
    rebuilds the file so no free pages remain. VACUUM always runs off the async runtime,
    and its cost is measured.
  - The chat view gets a reachable **Clear history…** action. Its confirmation dialog is
    honest about what is and isn't guaranteed.

## Settled decisions (confirmed by the user; do not reopen)

1. **When the DB cannot open, the app blocks until Retry** (the same decision as in
   05-01). Settings and contacts now live in the DB, so a storage failure leaves them
   unavailable along with networking. The full-screen "Encrypted storage unavailable"
   view with Retry is the only UI shown.
2. **Narrow cleanup after a failed provisioning.** The DB file is deleted only if *this
   same provisioning attempt* created it **and** it holds no user data. Nothing else is
   ever deleted (2A.2).
3. **A corrupt `settings.json` or `contacts.json` blocks.** It produces a typed, visible
   `LegacyImportFailed` error, and **nothing** is imported. A corrupt `known_peers.json`
   keeps today's behaviour: it is skipped, logged by class only, and those peers are
   prompted again. It **stays on disk as plaintext**. That is documented in the README
   and the module docs, and the 05-03 audit checks for it (2A.5).
4. **Logs use the short NodeId** (`fmt_short`, through `logging::fields::peer`). This
   covers the new `history_clear` and legacy-import log lines.

## Dependencies / ordering

- **Before:** 05-01 must be merged into `05-encrypted-storage-and-logging`. That step
  provides the storage handle, the migration runner, the error screen, and the logging
  guard that the new code must pass.
- **Parallel group:** 2A and 2B are independent and can be built in parallel worktrees by
  separate sub-agents.
- **After:** 05-03, the audit, runs last.

### Worktree / sub-agent split

| Sub-task | Branch | Owns |
|---|---|---|
| 2A | `step-05-02-encrypted-settings-contacts-and-history-clear-2a` | **new** `core/src/identity/settings_sql.rs` (`SqlCipherSettingsStore` + `MIGRATION_SETTINGS`), **new** `core/src/contacts/store_sql.rs` (`SqlCipherContactStore` + `MIGRATION_CONTACTS`), **new** `core/src/storage/legacy.rs`; **edit** `core/src/identity/{settings.rs,mod.rs,service.rs}`, `core/src/contacts/{book.rs,store.rs,store/tests.rs,migrate.rs,migrate/tests.rs,model.rs,mod.rs,contract.rs,testing.rs}`, `core/src/storage/{schema.rs,error.rs,handle.rs,database.rs,mod.rs,tests.rs}` (`database.rs`: only the "file created" flag in `ensure_private_regular_file`/`Database::open`), `core/src/discovery/testing/scenarios.rs`; **edit** the core integration tests that use the interim stores: `core/tests/{discovery_lan.rs,discovery_two_nodes.rs,contacts_two_nodes.rs,profile_isolation.rs,identity_integration.rs}`; **new** `core/tests/storage_legacy_import.rs`, `core/tests/storage_namespacing.rs`; **edit** `app/src-tauri/src/{lib.rs,identity_cmd.rs,net_cmd.rs,storage_cmd.rs,storage_cmd/tests.rs,test_util.rs}`, `headless/src/{node.rs,control.rs}`, `headless/tests/control.rs`, `app/src/storage.ts` (new error kind only); README "Contacts"/"Message storage"/layout sections; AGENTS.md "Notes" |
| 2B | `step-05-02-encrypted-settings-contacts-and-history-clear-2b` | **edit** `core/src/messaging/{sqlcipher.rs,sqlcipher/tests.rs,store.rs}`, `core/src/contacts/history.rs` (contract doc only), `core/src/storage/database.rs` (add `reclaim`); **edit** `app/src-tauri/src/{chat_cmd.rs,chat_cmd/tests.rs,contacts_cmd/tests.rs}` and the one `generate_handler!` line in `lib.rs`; **edit** `app/src/{chat.ts,chat.test.ts,lib/chatStore.ts,lib/chatStore.test.ts,views/Chat.svelte,views/Chat.test.ts,views/RemoveContactDialog.svelte,views/RemoveContactDialog.test.ts}`, **new** `app/src/views/ClearHistoryDialog.svelte`, `app/src/views/ClearHistoryDialog.test.ts`, `app/src/lib/historyCopy.ts`; README "Chat" section + checklist |

**Conflict hotspots:**
- `app/src-tauri/src/lib.rs`. 2A rewires the stores in `setup`. 2B adds `history_clear`
  to `generate_handler!` and changes nothing else.
- `core/src/storage/*`. 2A edits `schema.rs`, `error.rs`, `handle.rs`, `mod.rs` and
  `database.rs` (the "created" flag). 2B adds only `Database::reclaim` in `database.rs`
  and a `test-support` accessor in `messaging/sqlcipher.rs`. Both touch `database.rs`, so
  **2B rebases on top of 2A** before merging.
- `README.md`. Each sub-task edits only its own sections.
- `Cargo.lock`. Regenerate it; never hand-edit it.

**Merge order:** merge **2A** first; it is the larger diff and it changes
`database.rs`. Then rebase **2B** onto the step branch (now containing 2A) and merge it.
After each merge, run the full gates.

## Branch

- Step branch `step-05-02-encrypted-settings-contacts-and-history-clear`, created from
  `05-encrypted-storage-and-logging` after 05-01 has merged.
- Sub-branches `...-2a` and `...-2b`. Merge them back with `--no-ff`.
- PR: step branch → `05-encrypted-storage-and-logging`.

---

## What to do — Sub-task 2A: settings and contacts in SQLCipher

### 2A.1 New migrations (slice-owned SQL, ordered in `storage/schema.rs`)

```rust
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, label: "messages",       sql: crate::messaging::MIGRATION_MESSAGES_V1 },
    Migration { version: 2, label: "settings",       sql: crate::identity::MIGRATION_SETTINGS },
    Migration { version: 3, label: "contacts",       sql: crate::contacts::MIGRATION_CONTACTS },
    Migration { version: 4, label: "legacy_imports", sql: super::legacy::MIGRATION_LEGACY_IMPORTS },
];
pub const SCHEMA_VERSION: u32 = 4;
```

```sql
-- v2, owned by identity (core/src/identity/settings_sql.rs)
CREATE TABLE account_settings (
  account_id       BLOB    PRIMARY KEY CHECK (length(account_id) = 16),
  display_name     TEXT,
  provisioned_at   INTEGER,
  presence_enabled INTEGER CHECK (presence_enabled IN (0, 1)),
  relay_url        TEXT    CHECK (relay_url IS NULL OR length(relay_url) <= 2048)
);

-- v3, owned by contacts (core/src/contacts/store_sql.rs)
CREATE TABLE contacts (
  account_id   BLOB    NOT NULL CHECK (length(account_id) = 16),
  node_id      TEXT    NOT NULL,
  nickname     TEXT    NOT NULL,
  self_name    TEXT,
  trust        TEXT    NOT NULL CHECK (trust IN ('unverified', 'verified')),
  added_at_ms  INTEGER NOT NULL CHECK (added_at_ms >= 0),
  last_seen_ms INTEGER CHECK (last_seen_ms IS NULL OR last_seen_ms >= 0),
  PRIMARY KEY (account_id, node_id)
) WITHOUT ROWID;

-- v4, owned by storage (core/src/storage/legacy.rs): install-level bookkeeping,
-- the one documented exception to "every row carries account_id" (like user_version).
CREATE TABLE legacy_imports (
  file           TEXT    PRIMARY KEY CHECK (file IN ('settings.json', 'contacts.json', 'known_peers.json')),
  imported_at_ms INTEGER NOT NULL
);
```

Account ids are stored as the 16 raw UUID bytes, the same encoding the `messages` table
uses (`account.as_uuid().as_bytes()`). Add `SQL_LEGACY_*` and the `legacy_imports` table
to the allowlists in `core/tests/sql_namespacing_guard.rs` (statements and `CREATE TABLE`),
with a comment naming this exception next to the existing `messages_fts` one.

### 2A.2 `SqlCipherSettingsStore` (`core/src/identity/settings_sql.rs`)

```rust
pub struct SqlCipherSettingsStore { storage: Arc<Storage> }
impl SqlCipherSettingsStore { pub fn new(storage: Arc<Storage>) -> Self }
impl SettingsStore for SqlCipherSettingsStore { /* every method: storage.database(id)? then constant SQL */ }
```

- All statements are `const SQL_SETTINGS_*: &str`, and every one has
  `WHERE account_id = ?1` or inserts `account_id`. Each setter is an UPSERT on one column:

  ```sql
  INSERT INTO account_settings (account_id, display_name) VALUES (?1, ?2)
  ON CONFLICT (account_id) DO UPDATE SET display_name = excluded.display_name
  ```

- Read-back keeps today's `FileSettingsStore` rules:
  - `get_display_name` validates with `validate_display_name`.
    `DisplayNameError::InvisibleCharacter` maps to `Ok(None)`; any other validation error
    maps to `Err(SettingsError::Unavailable)`.
  - `get_relay_url` re-validates with `validate_relay_url`. An invalid stored value is
    `Err`, so the node refuses to start rather than falling back to the default mesh.
  - `set_relay_url` stores the canonical form, and `None` clears it.
- A `StorageError` maps to `SettingsError::Unavailable`.

**Provisioning and the orphan-DB hazard (settled decision 2).** `IdentityService::persist`
calls `set_provisioned_at` *before* it writes the marker. With this store, that call
creates `messenger.db` through the 05-01 creation policy (no marker yet → creation
allowed), keyed with the freshly stored DB key. If a later step fails, the rollback
deletes both keychain keys. That would leave a DB no key can open, and every later
`provision` would fail. Provisioning must **never leave the app stuck**:

- **`database.rs` (2A owns this change).** `ensure_private_regular_file` returns
  `FileState { had_data: bool, created: bool }`, where `created` is true only if this call
  created the file. `Storage` records `last_created: Option<AccountId>`, the account
  whose open created the file, even if the open later failed.
- Add a defaulted method to `SettingsStore` (`core/src/identity/settings.rs`):
  `fn discard_new_account(&self, _id: &AccountId) -> Result<(), SettingsError> { Ok(()) }`.
  The `Arc<T>` forwarding impl forwards it. The wrappers in
  `core/src/discovery/testing` and the `net_cmd.rs` tests keep the default.
- `IdentityService::provision` calls it best-effort in the rollback branch, next to the
  keychain deletes. It is called whichever step failed: the keychain store,
  `set_provisioned_at` itself, or the marker write.
- `SqlCipherSettingsStore` forwards it to `Storage::discard_provisioning(&self, account)`.
  The method runs entirely under the slot mutex, in this order:
  1. **Always reset the slot to `Closed`.** This clears a latched `Failed(..)` and drops an
     `Open` for the failed account. The next provisioning attempt, which uses a new
     `AccountId`, therefore never sees a stale failure or `OtherAccount`.
  2. Delete the file **only if all** of these hold:
     - `last_created == Some(account)`: this attempt created it;
     - `read_marker(data_dir)` is still `None`;
     - it holds no user data. If the slot was `Open`, check
       `SELECT count(*) FROM messages` = 0, `contacts` = 0, and `account_settings` rows
       other than `account` = 0, using `pub(crate)` constant SQL in each slice. If the
       slot was `Failed`, require file length 0;
     - Storage holds the only handle: `Arc::try_unwrap(db)` succeeds.

     Then drop the connection, remove `messenger.db` and any `-journal`/`-wal`/`-shm`
     sibling, and set `last_created = None`.
  3. **If only the sole-handle check fails**, someone else holds the `Arc<Database>`.
     That cannot happen in the app or headless: stores take the `Arc` per call and drop
     it, and `ChatState` opens only after a successful provision. If it happens anyway,
     keep the file and store `pending_discard: Option<(AccountId, Weak<Database>)>`. The
     next `Storage::database` call while the marker is still absent first retries the
     deletion once the `Weak` is dead. If the `Weak` is still alive, that call returns
     `StorageError::Io` **without latching it**, so it can be retried. It never returns
     `OtherAccount`.
  4. In every other case, keep the file. It was not created by this attempt, or it holds
     data. The slot is still `Closed`, per step 1. A file this attempt did not create, or
     a file holding data, is never deleted.

### 2A.3 `SqlCipherContactStore` (`core/src/contacts/store_sql.rs`)

`pub struct SqlCipherContactStore { storage: Arc<Storage> }` implements the existing
`ContactStore` trait unchanged. It must pass `contacts::contract::contract_suite`, the
same suite `MemoryContactStore` passes. The cap rule is `MAX_CONTACTS_PER_ACCOUNT`.

**Transactions and locking.** Every method takes `storage.database(account)` once and
then locks `db.conn()` **once**. Read-modify-write methods (`insert`, `insert_missing`,
`observe`) open their own IMMEDIATE transaction on that guard and do the read through a
private `fn get_tx(tx: &Transaction, account, id)`. They must **never** call `self.get()`
or any other trait method while holding the guard: that would relock the non-reentrant
mutex and deadlock. The transaction-level SQL lives in `pub(crate)` free functions that
take `&Transaction`. These are `insert_if_absent_tx`, `count_tx` and `get_tx`. The legacy
import (2A.5) reuses them on its own transaction.

| Method | SQL (all constant, all scoped) | Notes |
|---|---|---|
| `list` | `SELECT node_id, nickname, self_name, trust, added_at_ms, last_seen_ms FROM contacts WHERE account_id = ?1` | `ContactBook` sorts |
| `get` | same `... AND node_id = ?2` | |
| `insert` | own IMMEDIATE tx: `count_tx` (≥ cap → `Full`), then `INSERT INTO contacts (...) VALUES (?1, ..., ?7)`; a unique violation → `AlreadyExists` | Never overwrites |
| `insert_missing` | one IMMEDIATE tx: `insert_if_absent_tx` (`INSERT ... ON CONFLICT (account_id, node_id) DO NOTHING`) per entry; then `count_tx` per touched account > cap → rollback, `Full` | Report `inserted` / `already_present` from the changed-row counts |
| `set_nickname` / `set_trust` | `UPDATE contacts SET nickname = ?3 WHERE account_id = ?1 AND node_id = ?2 RETURNING <cols>`; no row → `NotFound` | `set_trust` stays the ONLY trust write path |
| `observe` | own IMMEDIATE tx: `get_tx`, apply the shared pure rule, `UPDATE ... SET self_name = ?3, last_seen_ms = ?4 WHERE account_id = ?1 AND node_id = ?2` only if it changed | Unknown id → `Ok(())` no-op |
| `remove` | `DELETE FROM contacts WHERE account_id = ?1 AND node_id = ?2`; returns `changes > 0` | `secure_delete=ON` zero-fills the freed nickname bytes |

- Extract the body of `Ledger::observe` (`contacts/store.rs`) into a pure function in
  `contacts/model.rs`:
  `pub(crate) fn apply_observation(c: &mut Contact, name: NameUpdate<'_>, seen_at_ms: Option<i64>) -> bool`.
  `Ledger::observe` and `SqlCipherContactStore::observe` both use it, so the
  `LAST_SEEN_RESOLUTION_MS` rule exists exactly once.
- **Row decoding** validates exactly as `Document::into_ledger` does today:
  - the NodeId parses and round-trips;
  - `Nickname::parse(s)` equals `s`;
  - `validate_display_name(self_name)` equals `self_name`;
  - the trust text is known;
  - times are non-negative.

  Any failure is `ContactStoreError::Corrupt`.
- Replace `ContactStoreError::Io(io::Error)` with an opaque `ContactStoreError::Unavailable`,
  for any `StorageError` or rusqlite failure, and update `From<ContactStoreError> for ContactError`
  in `core/src/contacts/book.rs`. A new error type must not carry an `io::Error` or
  `rusqlite::Error`, whose text can include paths or SQL.
- The settings store follows the same rule. `SqlCipherSettingsStore` setters are single
  statements, and the import uses
  `pub(crate) fn insert_settings_if_absent_tx(tx, account, &LegacySettings)` in
  `settings_sql.rs`.

### 2A.4 Retire the interim writers; keep read-only legacy parsers

Readers **parse only**. They never open the DB and never insert. One shared error type
lives in `core/src/storage/legacy.rs` and is re-exported `pub(crate)`:

```rust
/// Why a legacy plaintext file could not be read. No paths, no contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyError {
    /// Present but not a regular file (symlink, directory, device, ...). Never followed.
    NotRegularFile,
    /// Unparseable, wrong version, over the size cap, or a field fails validation.
    Corrupt,
    /// Present but unreadable (permissions, I/O).
    Io,
}
```

**Symlink rule, used everywhere:** a legacy path that is a symlink or not a regular file
is `LegacyError::NotRegularFile`. It is never read, never written through and never
unlinked by the import. For `settings.json`/`contacts.json`, that blocks with
`LegacyImportFailed`. For `known_peers.json`, it is skipped like a corrupt file.

- **`identity/settings.rs`.** Delete `FileSettingsStore`'s write path and the type
  itself. Keep only a crate-private reader:
  `pub(crate) fn read_legacy_settings(data_dir: &Path) -> Result<Option<Vec<(AccountId, LegacySettings)>>, LegacyError>`,
  where `pub(crate) struct LegacySettings { display_name: Option<String>, provisioned_at: Option<i64>, presence_enabled: Option<bool>, relay_url: Option<String> }`.
  It reuses the existing `SettingsDocument`/`StoredAccount` serde types, the size cap, the
  `symlink_metadata` regular-file check and `deny_unknown_fields`. Keep
  `MemorySettingsStore`, `validate_relay_url` and the `SettingsStore` trait. Rewrite the
  module docs: the interim deviation is gone.
- **`contacts/store.rs`.** Delete `FileContactStore`. Keep `Ledger`, which
  `MemoryContactStore` and the legacy reader use, and the `ContactStore` trait. Keep a
  crate-private
  `pub(crate) fn read_legacy_contacts(data_dir: &Path) -> Result<Option<Vec<(AccountId, Contact)>>, LegacyError>`
  built from `Document::into_ledger`. Rewrite the module docs.
- **`contacts/migrate.rs`.** Turn `migrate_legacy_known_peers`'s file handling into
  `pub(crate) fn read_legacy_known_peers(data_dir: &Path) -> Result<Option<Vec<(AccountId, Contact)>>, LegacyError>`.
  Delete the public function, `MigrationError`, `MigrationReport` and their re-exports;
  the import now lives in storage.
- Move or adapt the existing reader tests in `store/tests.rs`, `migrate/tests.rs` and
  `settings.rs`: corrupt, oversized, symlink → `NotRegularFile`, unknown version, and older
  looser names (treated as unset). Delete the tests that only exercised the deleted
  writers. Re-exports in `identity/mod.rs` and `contacts/mod.rs`: drop the `File*` types
  and add the `SqlCipher*` types and the `MIGRATION_*` constants (`pub(crate)`).
- Every test that built an interim store switches to `MemoryContactStore` /
  `MemorySettingsStore`, or to the SQL stores over `FakeKeychain` + a temp dir where
  persistence across a restart is the point:
  - `core/src/discovery/testing/scenarios.rs`;
  - `core/tests/discovery_lan.rs`;
  - `core/tests/discovery_two_nodes.rs`. Its `contacts.json` assertion becomes "the
    contact is listed after restart" through `SqlCipherContactStore`;
  - `core/tests/contacts_two_nodes.rs`;
  - `core/tests/profile_isolation.rs`. Two profiles get two `Storage`s in two dirs, and a
    test checks that neither DB opens with the other profile's key;
  - `core/tests/identity_integration.rs`. Its `FileSettingsStore` persistence test
    becomes a `SqlCipherSettingsStore` persistence test.

### 2A.5 One-shot legacy import (`core/src/storage/legacy.rs`)

`Storage::database` runs the import once per process, right after the first successful
`Database::open` and before it returns the handle. So nothing (settings read,
`my_info`, node start) can observe a half-imported state. **The import works only on
the `&Database` it is given.** It must never call `Storage::database`, a
`SqlCipher*Store` trait method, or anything else that re-enters the slot mutex or
`db.conn()` while it holds the guard. That would recurse into the open or deadlock.

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ImportReport {
    pub settings_accounts: usize,      // account_settings rows inserted
    pub contacts: usize,               // contacts rows inserted from contacts.json
    pub known_peers: usize,            // contacts rows inserted from known_peers.json
    pub known_peers_skipped: bool,     // known_peers.json present but unreadable: left on disk
    pub removals_pending: usize,       // committed imports whose plaintext file could not be removed yet
}

pub(crate) fn import_all(db: &Database, data_dir: &Path) -> Result<ImportReport, StorageError> {
    let mut report = ImportReport::default();
    import_one(db, data_dir, "settings.json",    Fatal::Yes, &mut report, |tx| {
        let parsed = identity::read_legacy_settings(data_dir)?;            // parse only
        insert_all(tx, parsed, identity::insert_settings_if_absent_tx)      // inserts here
    })?;
    import_one(db, data_dir, "contacts.json",    Fatal::Yes, &mut report, |tx| { /* read_legacy_contacts + contacts::insert_if_absent_tx + cap check */ })?;
    import_one(db, data_dir, "known_peers.json", Fatal::No,  &mut report, |tx| { /* read_legacy_known_peers (Unverified) + insert_if_absent_tx */ })?;
    Ok(report)
}
```

`import_one` does the following, for each file:
1. If `legacy_imports` already has the file, skip the import and go to step 4 (crash
   recovery).
2. Parse the file with the slice's reader. "Absent" (`Ok(None)`) means there is nothing to
   do. Any `LegacyError` is handled by `Fatal`:
   - `Fatal::Yes` (`settings.json`, `contacts.json`): return
     `StorageError::LegacyImportFailed`. It is latched and shown on the storage screen.
     Nothing is imported, and the file is left untouched (settled decision 3).
   - `Fatal::No` (`known_peers.json`): log `warn!(error = ?e, "legacy known peers
     skipped")` and set `report.known_peers_skipped`. The file stays on disk **as
     plaintext**, and those peers are prompted again. This is the existing behaviour,
     kept on purpose. The README and the module docs state that the file remains, and
     05-03 audits it.
3. Open one IMMEDIATE transaction on `db.conn()`. Insert the rows with **insert-if-absent**
   semantics, through the slices' `*_tx` helpers:
   - settings: `INSERT ... ON CONFLICT (account_id) DO NOTHING`;
   - contacts: `ON CONFLICT (account_id, node_id) DO NOTHING`, plus the cap check;
   - known peers: always `Unverified`, as today.

   In the same transaction, run `INSERT INTO legacy_imports (file, imported_at_ms) VALUES (?1, ?2)`.
   Then commit. Any failure rolls back and returns `LegacyImportFailed` (`Fatal::No`: warn
   and skip), and the file stays.
4. **After** the commit, and after releasing the guard, call `remove_plaintext(path)`:
   - `symlink_metadata` must say regular file. Otherwise leave it alone, and add one to
     `removals_pending` with a `warn!` (it was regular when read, so this means tampering;
     never follow it);
   - **hard-link check (Unix):** only when `metadata.nlink() == 1`, overwrite the file in
     place with zeros for its full length and `sync_all`. If `nlink > 1`, do **not**
     zero-fill, because that would destroy the other link's content. Just unlink this
     name and log `warn!` by class;
   - `remove_file`, then a best-effort parent-dir fsync (Unix);
   - do the same for a leftover `<file>.tmp` from the old atomic writer.

   A removal failure is logged (`warn!`, class only), counted in `removals_pending`, and
   retried at the next start. The `legacy_imports` row ensures the stale file is **never
   re-imported**.

Add `StorageError::LegacyImportFailed` and its UI kind and honest reason in
`app/src-tauri/src/storage_cmd.rs` and `app/src/storage.ts`. Example reason: "Older
unencrypted app data could not be read, so it was left in place and nothing was imported.
Your encrypted data is unchanged." Log the outcome counts only:
`info!(count = n, kind = "contacts", "legacy data imported")`.

**Honesty note** (module docs + README):
- Zero-overwriting a file does not guarantee erasure on SSDs, copy-on-write file systems
  (APFS, btrfs), or in backups and snapshots.
- A corrupt `known_peers.json` is left on disk unencrypted. The user can delete it.

The README says both plainly.

### 2A.6 Wiring

- **`app/src-tauri/src/lib.rs` `setup`.** Create `storage` (from 05-01), then:
  - `let settings = Arc::new(SqlCipherSettingsStore::new(storage.clone()));`
  - `let contacts_store = Arc::new(SqlCipherContactStore::new(storage.clone()));`

  Delete the `FileContactStore` / `FileSettingsStore` construction and the
  `migrate_legacy_known_peers` block. `AppIdentity` becomes
  `IdentityService<Arc<OsKeychain>, Arc<SqlCipherSettingsStore>, LiveTicketProvider>`.
  `NetBoot.settings` becomes `Arc<SqlCipherSettingsStore>`.
- **Order.** With an identity Present, `storage_cmd::open_for` still runs in `setup`, as
  in 05-01, so the import happens before the first command. On a fresh install, the DB
  is created during `provision` by `set_provisioned_at`; the marker is absent, so the
  creation policy allows it. `identity_provision` then calls `open_for` as before.
  `open_for` is idempotent. Here `Storage::database` returns the already-open `Database`,
  and `open_for` opens the chat store (`SqlCipherMessageStore`) over it and reports
  `Ready`.
- **`headless/src/node.rs`:**
  - `let keychain = Arc::new(FileKeychain::open(..)?);`
  - `let storage = Arc::new(Storage::new(args.data_dir.clone(), keychain.clone()));`
  - use `SqlCipherSettingsStore` and `SqlCipherContactStore`;
  - delete the `migrate_legacy_known_peers` block;
  - `HeadlessIdentity = IdentityService<Arc<FileKeychain>, Arc<SqlCipherSettingsStore>, LiveTicketProvider>`.

  Keep `NoMessageStore` as the eraser.
- **Headless tests change.** `headless/tests/control.rs` `nicknames()` reads
  `contacts.json` directly today, and `respond_accept_with_and_without_nickname`
  depends on it. That file no longer exists. Add a test-only control query
  `Request::Contacts` in `headless/src/control.rs`, answered in `headless/src/node.rs`
  from `book.list()`. It returns `[{ "node_id": .., "nickname": .. }]`. Rewrite
  `nicknames()` to call `ctl(&["contacts"])`. Prefer this over opening the DB from the
  test process. The node holds the DB open, and a second `Storage` in another process
  would duplicate the open/import path. The headless node is test-only and never
  shipped, so exposing nicknames on its local socket is acceptable. Note it in the
  `control.rs` docs. Then `headless/tests/*` must pass, and `docker/smoke.sh` is run
  manually, with the result noted in the PR.
- **Tests and fixtures** (`core/src/discovery/testing`, `app/src-tauri/src/test_util.rs`).
  Keep `MemorySettingsStore` / `MemoryContactStore` as the defaults. Add one Level-1
  scenario that runs the real `SqlCipher*` stores over `FakeKeychain`
  (`app/src-tauri` `first_run_flow_end_to_end` variant, see Tests).

### 2A.7 Docs

- **README:**
  - "Contacts": replace "Contacts are stored in plaintext until DD-05" and the
    `contacts.json` bullet. They now live encrypted in `messenger.db`. Existing
    `settings.json` / `contacts.json` / `known_peers.json` are imported once and then
    overwritten and removed, with the SSD/backup caveat.
  - "Message storage" becomes "Encrypted storage": settings, contacts and messages.
  - Layout row for `core`: add `storage` and `logging`.
- **AGENTS.md:** replace "Contacts are the single trust store …; until DD-05 they live in
  plaintext `contacts.json`" and "Messages live in the SQLCipher `messenger.db` (DD-05
  seam)" with: "All persisted user data (settings, contacts, messages) lives in the
  SQLCipher `messenger.db` through `core::storage`. Never write it anywhere else. New
  tables are new migrations in `storage/schema.rs`, with SQL owned by the slice and
  `account_id` on every row." Update the `test-support` doubles list.

---

## What to do — Sub-task 2B: `history_clear`

### 2B.1 Reclaim on the shared erase path (core)

`SqlCipherMessageStore::erase_peer_history` already deletes inside a transaction. The
`messages_ad` trigger removes the FTS rows, and `optimize` merges away the trigram
postings. Extend it with page reclaim:

```rust
impl Database {
    /// Rebuilds the file so no free page can hold deleted ciphertext.
    /// Must run outside any transaction. With temp_store=MEMORY the rebuild
    /// image never touches a temp file.
    pub(crate) fn reclaim(&self) -> Result<(), StorageError> {
        let conn = self.conn()?;
        conn.execute_batch("VACUUM;").map_err(|_| StorageError::Io)?;
        let free: i64 = conn.pragma_query_value(None, "freelist_count", |r| r.get(0))
            .map_err(|_| StorageError::Io)?;
        if free == 0 { Ok(()) } else { Err(StorageError::Io) }
    }
}
```

In `erase_peer_history`, commit the delete transaction (unchanged), **drop the
connection guard**, then call `self.db.reclaim().map_err(|_| HistoryEraseError)`.
`reclaim` takes the guard itself, so holding it here would deadlock. If the reclaim
fails, the rows are already gone. Report the error anyway, so the caller can retry,
which is idempotent. `ContactBook::remove` then keeps the contact, which is the existing
contract.

**VACUUM cost: never on the async runtime.** `VACUUM` rewrites the whole file while
holding the connection mutex. With `temp_store=MEMORY`, the rebuild image is held in RAM.
So both the lock-hold time and the peak memory grow with the DB size. Rules:
- Every caller reaches `erase_peer_history` through `spawn_blocking`:
  - `ChatState::clear_history` (2B.2);
  - `contacts_cmd::remove` through its existing `blocking` helper.

  Never call it from an async task directly. Add a doc comment on `reclaim` and on
  `HistoryEraser::erase_peer_history` saying so.
- **Measure** with an `#[ignore]`d test `vacuum_cost_on_100k_messages`. It inserts 100 000
  messages of about 200 bytes across 50 peers, clears one peer, and prints the wall time
  and the RSS delta (`/proc/self/status` `VmHWM` on Linux). Record the numbers in the
  `messaging/sqlcipher.rs` module docs and the PR.
- **Criteria.**
  - If the wall time is ≤ 2 s and the memory delta ≤ 2× the DB size, keep VACUUM as is.
  - If the time is above 2 s, keep VACUUM, because correctness wins. The dialog
    already shows a busy state and disables its buttons; add "This may take a moment."
    to the busy text.
  - Do **not** switch `temp_store` to `FILE` to save memory. That would move the rebuild
    image to a temp file, whose protection we have not verified. If the memory is
    unacceptable, raise it in the PR instead of changing the pragma.

Update the `HistoryEraser` docs in `contacts/history.rs` and `MessageStore` in
`messaging/store.rs` to state the contract: "removes the peer's rows and FTS postings,
then reclaims pages. Idempotent. All-or-nothing for rows."

**What this does and does not guarantee.** Write this in the module docs, and use the same
facts in the UI copy:
- `secure_delete=ON`, set and verified at open in 05-01, makes SQLite overwrite deleted
  content with zeros before the page is encrypted and written.
- `VACUUM` rewrites the database so the file has no free pages. Verify with a test that
  the vacuumed file reopens with the key and is still rejected with a wrong key.
- The rollback journal (`journal_mode` is the default `DELETE`) holds only SQLCipher-
  encrypted pages, and it is unlinked after commit.
- This does **not** cover:
  - the old file blocks on SSDs or copy-on-write file systems;
  - OS backups and snapshots;
  - the peer's copy of the conversation.

Do not claim "secure erase".

### 2B.2 The command (`app/src-tauri/src/chat_cmd.rs`)

```rust
impl ChatState {
    /// Fails queued outgoing messages for `peer` (so nothing is sent after the
    /// user cleared the conversation), then erases through the shared eraser.
    pub async fn clear_history(&self, peer: NodeId) -> Result<(), ChatCmdError> {
        let (account, store) = self.store_ref()?;            // StoreUnavailable if closed
        if let Ok(messaging) = self.messaging() {
            messaging.forget_peer(peer).await;
        }
        tauri::async_runtime::spawn_blocking(move || store.erase_peer_history(&account, &peer))
            .await
            .map_err(|_| ChatCmdError::Internal)?
            .map_err(|_| ChatCmdError::HistoryClearFailed)
    }
}

pub async fn clear(chat: &ChatState, node_id: &str, confirm: bool) -> Result<(), ChatCmdError> {
    if !confirm { return Err(ChatCmdError::NotConfirmed); }
    let peer = node_id_of(node_id)?;
    chat.clear_history(peer).await?;
    tracing::info!(peer = %logging::fields::peer(&peer), "history cleared");
    Ok(())
}

#[tauri::command]
pub async fn history_clear(chat: State<'_, Arc<ChatState>>, node_id: String, confirm: bool) -> Result<(), ChatCmdError> {
    clear(&chat, &node_id, confirm).await
}
```

- Add `NotConfirmed` and `HistoryClearFailed` to `ChatCmdError`, and extend
  `chat_errors_serialize_with_type_tag`.
- `account_id` comes from `ChatState`, never from the webview.
- Clearing is allowed for a removed contact: history can outlive the contact when the
  user chose "keep".
- Register `history_clear` in `lib.rs`.
- The remove-contact path keeps going through `ContactBook::remove` →
  `ChatState::erase_peer_history` → the same `SqlCipherMessageStore::erase_peer_history`.
  It therefore gets the reclaim with no extra code. Assert that in a test.
- `confirm` is a guard against accidental or buggy callers. The real confirmation is the
  dialog. Say so in a doc comment rather than overselling it.

### 2B.3 UI

- **`app/src/lib/historyCopy.ts`** holds the shared, honest copy used by both dialogs:

```ts
export const HISTORY_DELETE_GUARANTEE =
  "The messages are removed from this device's encrypted database and the space they used is overwritten. " +
  "This cannot remove the other person's copy, backups or snapshots of this computer, or traces some storage hardware (such as SSDs) may keep.";
```

- **`app/src/chat.ts`**:
  - `export const historyClear = (nodeId: string) => invoke<void>("history_clear", { nodeId, confirm: true });`
  - add `NotConfirmed` and `HistoryClearFailed` to `ChatCmdError` and to `messages`
    ("History could not be cleared. Nothing was sent. Try again.").
- **`app/src/lib/chatStore.ts`**: `export function resetConversation(nodeId: string)`.
  If the open conversation is `nodeId`, set `messages: []`, `nextBefore: null` and
  `error: null`. Clear `unread[nodeId]`.
- **New `app/src/views/ClearHistoryDialog.svelte`.** It follows the
  `RemoveContactDialog.svelte` pattern: `role="dialog"` with
  `aria-label="Clear history"`, the bold title "Clear message history with “<nickname>”?",
  "This permanently deletes …", `{HISTORY_DELETE_GUARANTEE}`, and
  "This cannot be undone." **Cancel** is the first button and gets autofocus. **Clear
  history** is disabled while busy. On success it calls `onDone()`. On error it shows
  `describeChatError(e)` in a `role="alert"` paragraph and stays open.
- **`app/src/views/Chat.svelte`.** Add a header button **Clear history…**. It is shown
  for removed contacts too, and disabled while the dialog is open. It opens the dialog,
  with the nickname as `contact?.nickname ?? nodeId.slice(0, 8)`. In `onDone`, call
  `resetConversation(nodeId)`, clear `query`/`results`, close the dialog and show a
  `role="status"` "History cleared."
- **`RemoveContactDialog.svelte`.** When "Also delete our message history" is ticked,
  append `HISTORY_DELETE_GUARANTEE` to the existing "permanently deleted" line. Then
  both entry points make the same claim about the same path.
- **README "Chat"**:
  - add `history_clear` to the command list;
  - add a **Clear history…** bullet with the guarantee text;
  - add manual checklist items: clear history while connected, and the peer's history is
    unchanged; clear history of a removed contact; search finds nothing afterwards;
    `grep -c <cleared word> messenger.db` returns 0.

---

## Guardrails

- AGENTS.md conventions apply:
  - vertical slices, with SQL owned by its slice and ordered only in `storage/schema.rs`;
  - no stubs, placeholder modules, empty files or empty folders. Delete dead writer code
    rather than leaving it unused;
  - `#![forbid(unsafe_code)]` in `core`;
  - `account_id` on every row and in every query. There are two documented, allowlisted
    exceptions: `messages_fts` (05-01, reached only through a join on
    `messages.account_id`) and `legacy_imports` (install-level bookkeeping).
- Never reference `SecretKeyBytes` or `DbKey` under `app/`, not even in comments.
  `Storage` takes the keychain through `KeySource`. Run the pre-commit grep.
- `FileKeychain` and `messenger-headless` stay out of `app/`. The headless node may use
  `FileKeychain` itself.
- Clipboard: only `MyInfo.svelte`. Neither new dialog copies anything.
- Never write credentials, keys, passphrases or tokens in code, in test fixtures committed
  as real secrets, in config, or in docs. All credentials and keys are obtained at
  runtime: from the OS keychain, or generated in tests with `FakeKeychain` /
  `rand::random`. Legacy-import fixtures contain only throwaway names and random NodeIds.
- Never bypass the hook with `--no-verify`.
- Wire format unchanged: `PROTOCOL_VERSION` stays as is.

## Security

| # | Risk | Prevention | How to check |
|---|---|---|---|
| 1 | Plaintext residue: legacy JSON and `.tmp` files left on disk | Overwrite + fsync + unlink after commit; retry each start; honest SSD/CoW caveat | `legacy_files_are_removed_after_import`; `data_dir_has_no_plaintext_after_import` byte-scans every file in the data dir for the canary nickname/display name |
| 2 | Revoked contact resurrected by a re-import, if the file survived a crash | `legacy_imports` row committed in the same transaction as the rows | `stale_legacy_file_is_never_reimported`: import, simulate a failed unlink (restore the file), remove the contact, reopen; the contact stays removed |
| 3 | Partial import | One IMMEDIATE transaction per file; file kept on failure | `failed_import_rolls_back_and_keeps_file` (force `Full` with a cap-exceeding fixture) |
| 4 | Corrupt legacy file treated as empty (silent data loss) | `LegacyImportFailed`, latched and shown; file untouched | `corrupt_settings_file_blocks_with_typed_error`, `corrupt_contacts_file_blocks_with_typed_error` (file hash unchanged) |
| 5 | Symlinked legacy file read or zero-filled through the link | One rule everywhere: a symlink or non-regular legacy path is `LegacyError::NotRegularFile`. It is never read, written through or unlinked by the import; that blocks for settings/contacts and skips for known_peers | `symlinked_legacy_file_is_typed_error_and_target_untouched` (Unix) |
| 5b | Zero-filling a hard-linked legacy file destroys another link's data | `nlink == 1` check before zero-fill; otherwise unlink only | `hardlinked_legacy_file_is_unlinked_not_zeroed` (Unix) |
| 5c | A corrupt `known_peers.json` stays as plaintext | Settled decision 3: kept by design; documented in the README and the module docs; audited in 05-03 | `corrupt_known_peers_is_skipped_and_left_on_disk`; README text |
| 6 | Cross-account leak | `account_id` in every statement; SQL guard; `OtherAccount` | `storage_namespacing.rs` (settings, contacts, messages under A vs B); `sql_namespacing_guard.rs` |
| 7 | SQL injection via nickname, self_name, relay URL, search | Bound parameters only; constant SQL | `hostile_strings_round_trip` with `'); DROP TABLE contacts;--`, `"`, NUL-free Unicode; the guard flags inline SQL literals |
| 8 | Orphaned, unopenable DB, or app stuck (`OtherAccount` / latched failure) after a failed provision | `discard_new_account` → `Storage::discard_provisioning`: always resets the slot to `Closed`; deletes only a file this attempt created that holds no user data; defers (unlatched `Io`) if another handle exists | `failed_set_provisioned_at_then_reprovision_succeeds`, `failed_marker_write_then_reprovision_succeeds`, `existing_db_is_never_discarded`, `db_with_user_data_is_never_discarded`, `discard_with_extra_handle_defers_and_never_returns_other_account` |
| 9 | Trust escalation through import or store | Only `set_trust` writes `trust`; known-peers import is always Unverified; `contacts.json` trust is carried over (it was the user's own attestation) | Contract suite; `known_peers_import_is_unverified` |
| 10 | Lost updates, races or self-deadlock in read-modify-write | One `Storage` per process; `observe`/`insert`/`insert_missing` use their own IMMEDIATE transaction with `get_tx`, never `self.get()` under the guard; the import never re-enters `Storage` | Contract suite under `std::thread::scope` with 8 concurrent observers (a deadlock fails by timeout); `import_does_not_reenter_storage` |
| 10b | Long VACUUM blocking the async runtime or exhausting memory | `erase_peer_history` only via `spawn_blocking`; measured on 100k messages; `temp_store` stays `MEMORY` | Review of call sites (`rg -n 'erase_peer_history' app/src-tauri/src`); `vacuum_cost_on_100k_messages` numbers in the PR |
| 11 | Relay override silently dropped (privacy) | Stored value re-validated; an invalid value refuses start; import preserves it | `relay_override_survives_import`; existing `relay_env_*` tests on the SQL store |
| 12 | Unconfirmed destructive command from the webview | `confirm: true` required; dialog with Cancel default | `history_clear_requires_confirmation`; `ClearHistoryDialog.test.ts` |
| 13 | Incomplete deletion: FTS postings, free pages, temp files | Trigger + `optimize`; `secure_delete=ON`; `VACUUM` with `temp_store=MEMORY`; `freelist_count == 0` asserted | `erase_reclaims_free_pages`, `erase_leaves_no_fts_terms` (existing), `erase_leaves_no_plaintext_in_export` (existing), raw-file canary scan |
| 14 | Messages delivered after the user cleared the conversation | `forget_peer` before erase | `history_clear_fails_queued_messages` |
| 15 | Names, bodies or counts of cleared data leaking to logs | Log only `peer` + fixed text; the guard covers all new code | `log_redaction_guard` passes |
| 16 | Overclaiming deletion (dishonest UI) | Shared copy constant names the limits | `ClearHistoryDialog.test.ts` and `RemoveContactDialog.test.ts` assert the guarantee text |

## Tests

**2A:**
- `core/src/identity/settings_sql.rs` (`#[cfg(test)]`): one shared settings contract.
  Move the existing `MemorySettingsStore` tests (round-trip, overwrite, missing → None,
  accounts isolated, relay validation, presence default) into a
  `pub(crate) fn settings_contract(store: &dyn SettingsStore)`, and run it against
  `MemorySettingsStore` **and** `SqlCipherSettingsStore`. Also cover: an invisible-char
  stored name reads as `None`; a stored invalid relay reads as `Err`.
- `core/src/contacts/store_sql.rs` tests: run `contract_suite` on `SqlCipherContactStore`.
  Add `corrupt_row_is_reported`, which writes a bad nickname via raw SQL in the test and
  expects `Corrupt`, plus `hostile_strings_round_trip`.
- `core/tests/storage_legacy_import.rs`, covering DD-05 *Migration forward* for 2A's
  tables:
  - build a DD-04-era profile dir: a v1 `messenger.db` with messages, plus `settings.json`,
    `contacts.json` and `known_peers.json` written with the old document formats as JSON
    literals;
  - open via `Storage`, and assert:
    - the schema is v4;
    - every message row is intact;
    - settings and contacts are equal to the JSON;
    - the files are gone and the data-dir byte-scan is clean;
    - a second open does no work.
  - Plus Security rows 2–5 and 11, and:
    - `corrupt_known_peers_is_skipped_and_left_on_disk`. Settled decision 3: the file
      stays byte-identical, `report.known_peers_skipped` is true, and settings and
      contacts are still imported;
    - `hardlinked_legacy_file_is_unlinked_not_zeroed` (Unix): a second hard link keeps
      its original content;
    - `import_does_not_reenter_storage`: the import runs under a `Storage` whose
      `KeySource` counts calls, and exactly one key load happens.
- `core/tests/storage_namespacing.rs` (*Namespacing*): write settings, contacts and
  messages under account A, then show that every read or search under B returns nothing,
  and the reverse.
- `core/src/identity/service.rs` and `core/src/storage/tests.rs` (provisioning cleanup,
  settled decision 2):
  - `failed_set_provisioned_at_then_reprovision_succeeds`: create a **directory** named
    `messenger.db` so the first open fails with `Io` and latches it. `provision` then
    fails and rolls back. The directory is left untouched, because this attempt did not
    create it. Remove the directory in the test, call `provision` again, and expect
    success with `Storage::status() == Ready`; never `OtherAccount`, never the stale `Io`;
  - `failed_marker_write_then_reprovision_succeeds`: make the marker path (`account_id`)
    a directory so `write_marker` fails after the DB was created. Expect the DB file
    deleted and the slot `Closed`. Remove the obstacle, reprovision, and expect success;
  - `existing_db_is_never_discarded`: a DB not created by this attempt survives a failed
    provision;
  - `db_with_user_data_is_never_discarded`;
  - `discard_with_extra_handle_defers_and_never_returns_other_account`: hold an extra
    `Arc<Database>` and expect the file kept, then `Io` (not latched) while the handle
    lives. Drop it, and expect the next call to delete the file and succeed.
- `app/src-tauri`:
  - `first_run_flow_end_to_end_with_encrypted_settings`: provision, set the name, read
    `my_info`, restart the service, then check the name persists and that `settings.json`
    is never created;
  - `storage_status_reports_legacy_import_failure`;
  - the existing `relay_env_*` and presence tests pass on the SQL-backed store;
  - `contacts_survive_restart_encrypted`: add a contact, rebuild the state from the same
    dir and keychain, and expect it listed; no `contacts.json` exists.
- `headless/tests/control.rs` **changes**. `nicknames()` is rewritten to use the new
  `contacts` control query (2A.6), and `respond_accept_with_and_without_nickname` must
  pass with it. Add `contacts_query_lists_nicknames` and
  `headless_persists_contacts_encrypted`, which runs two `run` cycles over one data dir,
  checks the contact is listed by the second, and checks no `contacts.json` exists. The
  other `headless/tests/*` must pass as they are.

**2B:**
- `core/src/messaging/sqlcipher/tests.rs`:
  - `erase_reclaims_free_pages`: insert 500 rows for a peer, erase, then expect
    `freelist_count == 0` and a smaller `page_count`;
  - `vacuumed_db_reopens_with_key_and_rejects_wrong_key`;
  - `erase_raw_file_has_no_canary`;
  - keep `erase_leaves_no_fts_terms` and `erase_leaves_no_plaintext_in_export`.
  This covers *History deletion*, best-effort check.
- `app/src-tauri/src/chat_cmd/tests.rs`:
  - `history_clear_requires_confirmation` (`NotConfirmed`, rows intact);
  - `history_clear_end_to_end`: two connected nodes exchange messages; A clears B; A's
    history and search for B are empty, and B's history on B is untouched;
  - `history_clear_without_store_fails_closed`;
  - `history_clear_rejects_bad_node_id`;
  - `history_clear_fails_queued_messages`;
  - `history_clear_on_removed_contact_works`;
  - `vacuum_cost_on_100k_messages` (`#[ignore]`, run manually; see 2B.1).
- `app/src-tauri/src/contacts_cmd/tests.rs`: the existing
  `remove_with_erase_clears_chat_history_end_to_end` lives **here**, not in
  `chat_cmd/tests.rs`. It must still pass. Extend it to assert
  `store.freelist_count() == 0` after the removal with erase. The accessor is
  `#[cfg(feature = "test-support")] pub fn freelist_count(&self) -> Result<i64, MessageStoreError>`
  on `SqlCipherMessageStore`. It is a `test-support` accessor, not `#[cfg(test)]`,
  because `#[cfg(test)]` items in `core` are invisible to the `app` crate's tests. The
  test proves that the remove path gets the same reclaim.
- Vitest:
  - `ClearHistoryDialog.test.ts`: copy includes the guarantee text; Cancel does not
    invoke; Confirm invokes `historyClear` with the node id; an error keeps the dialog
    open with an alert; buttons are disabled while busy.
  - `Chat.test.ts`: **Clear history…** is reachable for contacts and for removed contacts;
    after confirming, the message log and search results are empty and the "History
    cleared." status shows.
  - `chat.test.ts`: `historyClear` sends `confirm: true`; the new error types have
    messages.
  - `chatStore.test.ts`: `resetConversation` empties only that peer and clears its unread
    count.
  - `RemoveContactDialog.test.ts`: the guarantee text shows when erase is ticked.

## Quality gates

Run in each worktree, and again on the step branch after both merges:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

Commit only through the installed hook (`git config core.hooksPath .githooks`). Never use
`--no-verify`. Run `docker/smoke.sh` manually after 2A, since headless storage changed,
and record the result in the PR.

## Usable-state guarantee

- `npm start` works on a fresh profile. The wizard provisions and writes the display name
  into the encrypted DB, and no `settings.json` or `contacts.json` is ever created.
- An upgraded DD-04 profile imports its settings and contacts on the first launch:
  - the name, presence, relay override, contacts with nicknames and trust, and known
    peers are all kept;
  - the plaintext files disappear;
  - everything else works as before;
  - a corrupt `settings.json`/`contacts.json` shows the reachable storage screen with
    Retry, and nothing is lost or imported;
  - a corrupt `known_peers.json` is skipped and left on disk, as documented.
- **Clear history…** is reachable from every open chat. It confirms with honest copy,
  empties the conversation and search, and leaves the peer's side untouched. Remove
  contact with "also delete history" uses the same path.
- The headless node works with encrypted stores. Its control tests use the new
  `contacts` query, and `docker/smoke.sh` keeps working.
- A failed provisioning never leaves the app stuck. Re-running the wizard succeeds.

## Definition of done

- [ ] Migrations 2–4 (`account_settings`, `contacts`, `legacy_imports`) are in
      `storage/schema.rs`, and `SCHEMA_VERSION = 4`. A DD-04 DB migrates with every row
      intact.
- [ ] `SqlCipherSettingsStore` and `SqlCipherContactStore` pass the shared contracts. Every
      statement is `account_id`-scoped (the guard passes).
- [ ] `FileSettingsStore` and `FileContactStore` are deleted. Only read-only legacy
      parsers remain.
- [ ] The legacy import is transactional, idempotent and crash-safe, with no resurrection
      of revoked contacts. Readers only parse, and `legacy.rs` inserts on its own
      transaction without re-entering `Storage`. Files are zero-filled (`nlink == 1`
      only) and removed only after commit. A corrupt or non-regular
      `settings.json`/`contacts.json` gives a typed, visible error and imports nothing. A
      corrupt `known_peers.json` is skipped and left on disk, as documented.
- [ ] A failed provision always resets the storage slot, deletes the DB only if this
      attempt created it and it holds no user data, and never leaves `OtherAccount` or a
      stale latched error. Reprovisioning succeeds (tested).
- [ ] Contact-store read-modify-write methods use their own transaction and never call
      `self.get()` under the guard.
- [ ] App and headless are rewired, and `headless/tests/control.rs` uses the `contacts`
      control query. The Memory doubles are kept for tests.
- [ ] `history_clear` exists, requires confirmation, fails queued messages, deletes rows
      and FTS postings, and reclaims pages (`freelist_count == 0`). It runs only via
      `spawn_blocking`, and the VACUUM cost on 100k messages is measured and recorded.
      Remove-contact erase uses the same path (asserted in `contacts_cmd/tests.rs`
      through the `test-support` `freelist_count` accessor).
- [ ] The **Clear history…** UI is reachable, confirmed and honest, and tested in Vitest.
- [ ] README and AGENTS.md no longer say contacts or settings are plaintext. Chat,
      storage and checklist sections are updated.
- [ ] All quality gates pass. `docker/smoke.sh` was run, and the PR follows the template:
      `core` identity/contacts/messaging/storage, `app`, `headless`; "No change to the
      wire format".
