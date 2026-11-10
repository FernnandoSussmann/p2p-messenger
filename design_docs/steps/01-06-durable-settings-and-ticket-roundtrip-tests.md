# Step 01-06 — Durable settings (interim file store), mutual ticket test, restart round-trip

**Design doc:** [DD-01 Identity, Onboarding & My Info](../01-identity-onboarding-and-my-info.md)
**Depends on:** steps 01-01 .. 01-05 (all merged)
**Purpose:** close the DD-01 gaps found in review:

1. The display name and `provisioned_at` are lost on every restart, because
   `app/src-tauri/src/lib.rs` wires `MemorySettingsStore`. The user is left with
   a provisioned identity and no name after relaunch.
2. The "two in-process nodes parse each other's ticket" integration test is only
   half there (`core/tests/identity_integration.rs` checks node B's ticket
   against node A's identity, never both directions).
3. The "stored-then-loaded identity yields the same NodeId" round-trip test
   exists only as an in-memory loop inside `service.rs`
   (`provision_then_load_yields_same_node_id_for_many_keys`). It does not cover
   a fresh `IdentityService` over the same keychain and data dir (a restart).

## Decisions (already made, do not relitigate)

- **Interim durable store.** Add a file-backed `SettingsStore`
  (`FileSettingsStore`) behind the existing trait in
  `core/src/identity/settings.rs` and wire it in `lib.rs`. It is explicitly
  **interim**: DD-05 replaces it with the SQLCipher-backed store opened with the
  `DbKey`. **Do not implement SQLCipher here.**
- **Honest deviation from DD-01.** DD-01 ("Data storage") says display name and
  provisioning timestamp live in the *encrypted* DB. This step stores them in a
  **plaintext JSON file** in the app data dir. This is a deliberate, temporary
  deviation: the two values are low-sensitivity (the display name is shared with
  peers anyway, DD-03 `Hello`), and the file never contains a secret. The
  module docs and the DoD must record this deviation and that DD-05 removes it.
  The trait stays unchanged so DD-05 swaps the implementation without touching
  `IdentityService`, the Tauri commands, or the frontend.
- **Ticket stays NodeId-only.** `NodeIdTicketProvider` remains a stub that
  encodes only the NodeId. The **live ticket (NodeId + relay + direct addrs) is
  deferred to DD-02**, which owns ticket assembly. This step does not add
  addresses, endpoints, or networking.

## Non-goals

- No SQLCipher, no encryption of the settings file, no use of `DbKey` here.
- No new Tauri commands and no change to command return types.
- No change to the keychain, marker, or ticket formats.
- No new heavy dependencies. `serde` and `uuid` are already `core` deps;
  `serde_json` is already a workspace dependency (add it to `core`'s
  `[dependencies]` with `serde_json.workspace = true`; this is justified, it is
  light and already in the workspace). `tempfile` is already a dev-dependency.
  Do **not** add `proptest`; use the repeated-randomized style the repo already
  uses (see sub-task B2).

## Hard rules

- **Never write credentials in code.** No keys, tokens, passwords, or key bytes
  as literals in source, tests, fixtures or docs. Test key material is generated
  at runtime (`generate_identity_key`, `FakeKeychain`, random `AccountId`). All
  credentials are added at runtime through the OS keychain. The only byte
  literals allowed are the already-existing canary arrays in `service.rs`
  tests, which are test-only and are not to be copied into new files.
- The settings file must **never** contain, and `FileSettingsStore` must never
  accept or log, `SecretKeyBytes` / `DbKey` material. The trait signature already
  makes this impossible (it only takes `DisplayName` and `i64`); keep it so.
- Do not create empty files or folders as placeholders. Every file touched or
  added carries real content.
- `app/` must not reference `SecretKeyBytes` or `DbKey` (pre-commit secret gate).
  Note `identity_cmd.rs` is under `app/`; changing its alias to
  `FileSettingsStore` is fine, it names neither type.

## Parallelism plan

| Sub-task | What | Depends on | Parallel? |
|----------|------|------------|-----------|
| **A** | `FileSettingsStore` + unit tests in `core/src/identity/settings.rs`, `mod.rs` re-export, `Cargo.toml` dep | none | Parallel with B |
| **B** | Tests only: B1 mutual ticket integration test, B2 restart round-trip tests (using `MemorySettingsStore` so it does not need A) | none | Parallel with A (use a worktree each; A touches `settings.rs`, `mod.rs`, `core/Cargo.toml`; B touches `core/tests/identity_integration.rs` only, so no conflicts). B1 and B2 edit the same file: do them in one worktree, one after the other |
| **C** | Wire `FileSettingsStore` in `lib.rs` + `identity_cmd.rs` alias; restart integration test over the file store; frontend relaunch test; full gate | **A** (sequential after A; B should be merged too so the gate runs everything) | Sequential |

Suggested order: run A and B in parallel worktrees, merge both, then do C and
run the full pre-commit gate once on the merged tree. B1 and B2 are logically independent but share
one file, so keep them in the same worktree.

---

## Sub-task A — `FileSettingsStore` (parallel with B)

### What to build

In `core/src/identity/settings.rs`, next to `MemorySettingsStore`, add:

```rust
pub struct FileSettingsStore { path: PathBuf, lock: Mutex<()> }

impl FileSettingsStore {
    /// `dir` is the app-data directory. The file is `<dir>/settings.json`.
    pub fn new(dir: impl Into<PathBuf>) -> Self { /* no I/O in the constructor */ }
}

impl SettingsStore for FileSettingsStore { /* the four trait methods */ }
```

Update the module doc comment to say: `MemorySettingsStore` is the test
double; `FileSettingsStore` is the **interim** durable store; DD-05 replaces
both with SQLCipher. Re-export `FileSettingsStore` from
`core/src/identity/mod.rs`.

### On-disk format

JSON, versioned, keyed by the account UUID string (hyphenated, from
`AccountId::as_uuid()`), so a future multi-identity release needs no migration:

```json
{
  "version": 1,
  "accounts": {
    "<account-uuid>": { "display_name": "Ada", "provisioned_at": 1700000000000 }
  }
}
```

Use private `#[derive(Serialize, Deserialize)]` structs with
`#[serde(deny_unknown_fields)]` on the account entry, and `Option` fields for
both values. Store only `display_name` (string) and `provisioned_at` (i64). No
other field exists, so a secret cannot be added by accident.

### Behaviour

- **Read** (`get_*`): file missing -> `Ok(None)` (fresh install, not an error).
  File present but unreadable, not valid JSON, wrong `version`, or invalid
  UTF-8 -> `Err(SettingsError::Unavailable)`. It must **not** be treated as
  empty and **must not** be overwritten by a later write (see Security: no
  silent reset). Do not log file contents.
- **Re-validate on load.** The stored `display_name` string is passed through
  `validate_display_name` when read. If it fails validation (tampered file),
  return `Err(SettingsError::Unavailable)`, not the raw string.
- **Write** (`set_*`): under the internal `Mutex` (serialises read-modify-write
  inside the process), read the current document (missing file -> empty
  document; corrupt file -> error, do not clobber), modify the one account,
  then write atomically exactly like `write_marker` in `marker.rs`: create
  `settings.json.tmp` in the **same directory**, write, `sync_all`, `rename`
  over `settings.json`, best-effort directory fsync on unix, and on any error
  remove the temp file.
- **Permissions.** On unix create the temp file with mode `0o600`
  (`OpenOptionsExt::mode`). The app data directory itself is created by
  `lib.rs`; also set it to `0o700` there on unix (sub-task C). On non-unix,
  rely on the per-user app data dir ACLs; add a comment saying so.
- **Size bound.** Refuse to read a file larger than a small cap (for example
  64 KiB, `fs::metadata().len()`) and return `Unavailable`, so a swapped-in
  huge file cannot exhaust memory.
- **Symlinks.** Before reading or renaming, if `settings.json` is a symlink
  (`symlink_metadata().file_type().is_symlink()`), return `Unavailable`.
- Leave the constructor free of I/O so `lib.rs` setup cannot fail on it.

### Tests for A (in `settings.rs` `mod tests`, using `tempfile`)

- `file_store_round_trips_display_name_and_provisioned_at`
- `file_store_persists_across_instances`: write with one `FileSettingsStore`,
  read with a **new** instance on the same dir (simulated restart).
- `file_store_overwrite_display_name`
- `file_store_missing_file_returns_none` for both getters.
- `file_store_accounts_are_isolated` (two `AccountId`s in one file).
- `file_store_atomic_write_leaves_no_temp_file`: after several writes the dir
  contains only `settings.json`.
- `file_store_file_contains_no_secrets_and_only_known_keys`: after writing, parse
  the file as `serde_json::Value`; assert the top-level keys are exactly
  `version`, `accounts` and each account has only the two known keys.
- `file_store_corrupt_json_is_unavailable_and_not_overwritten`: write garbage,
  assert `get_*` and `set_*` both return `Unavailable` and the garbage bytes are
  unchanged afterwards.
- `file_store_wrong_version_is_unavailable`.
- `file_store_unknown_field_in_account_is_unavailable` (extra key such as `"secret"` in an account entry is rejected by `deny_unknown_fields`).
- `file_store_tampered_display_name_fails_validation`: hand-write a document
  whose name contains a newline, a bidi override (U+202E), or is over length;
  `get_display_name` must be `Err(Unavailable)`.
- `file_store_oversized_file_is_unavailable`.
- `file_store_rejects_symlinked_settings_file` (`#[cfg(unix)]`).
- `file_store_file_mode_is_0600` (`#[cfg(unix)]`, check
  `metadata().permissions().mode() & 0o777 == 0o600`).
- `file_store_unicode_name_round_trips_verbatim` (emoji and non-ASCII).
- `file_store_concurrent_writers_do_not_lose_updates`: several threads set
  different accounts through one shared store (`Arc`); afterwards all values are
  present and the file is valid JSON.

---

## Sub-task B — Missing integration and round-trip tests (parallel with A)

Tests only. If a test reveals a bug in production code, fix it minimally and say
so in the PR description.

### B1 — Mutual ticket exchange (`core/tests/identity_integration.rs`)

The existing test only parses B's ticket. Add a test (keep the existing one)
with two in-process identities, each with its own `FakeKeychain`, its own
`MemorySettingsStore` and its own tempdir:

```rust
#[test]
fn each_node_parses_the_others_ticket_and_gets_its_node_id() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (node(&dir_a), node(&dir_b));
    let (pa, pb) = (a.provision().unwrap(), b.provision().unwrap());

    let ticket_a = a.my_info().unwrap().ticket;
    let ticket_b = b.my_info().unwrap().ticket;

    assert_eq!(Ticket::parse(ticket_b.as_str()).unwrap().node_id, pb.node_id); // A reads B
    assert_eq!(Ticket::parse(ticket_a.as_str()).unwrap().node_id, pa.node_id); // B reads A
    assert_ne!(pa.node_id, pb.node_id);
}
```

Also assert that a node's own ticket parses to its own `load().node_id`, that a
tampered ticket string (truncated, extra whitespace-only prefix, empty) returns
`Err` from `Ticket::parse`, and that the ticket text does not contain any
substring of the identity key bytes (reuse the "hex of key bytes" idea from
`service.rs`; the key is obtained at runtime from the keychain, never a
literal: share the fake as `Arc<FakeKeychain>`, which already implements
`KeychainStore`, keep a handle, and call `load_identity_key`). Add a comment at the top of the test: **live
ticket content (relay and direct addresses) is deferred to DD-02; this test
covers the NodeId-only ticket and must be extended there.**

### B2 — Stored-then-loaded identity round trip, including restart

The existing 64-iteration in-memory test stays. Add, in `service.rs` tests or
`core/tests/identity_integration.rs` (the latter is preferred because it only
needs the public API, and `FakeKeychain` is available through the
`test-support` feature):

- `restart_yields_same_node_id_repeated`: loop at least 32 times. Each
  iteration: new tempdir, shared `Arc<FakeKeychain>` (`FakeKeychain` is not `Clone`, but
  `Arc<FakeKeychain>` implements `KeychainStore`; `Arc::clone` it into each
  service, as the `app` command tests already do), provision with service #1, record
  `node_id`/`account_id`, **drop** service #1, build service #2 over the same
  keychain and data dir, and assert `load()` and `status()` give the identical
  `node_id` and `account_id`, and that `provision()` on service #2 is rejected
  with `AlreadyProvisioned` (no new identity on restart).
- Randomness comes from the real key generator (each iteration is a fresh
  random key), so repetition is the property test. No `proptest`.

No production change should be needed. If one turns out to be, keep it
`testing`-only (`#[cfg(any(test, feature = "test-support"))]`) and never expose
it in release builds.

### Tests for B

B1 and B2 are themselves the tests; they must fail if the behaviour regresses
(check by temporarily breaking `Ticket::parse` or `node_id_from_secret_bytes`
locally and confirming a failure, then reverting).

---

## Sub-task C — Wiring, restart test, frontend (sequential, after A)

### C1 — Wire the durable store

- `app/src-tauri/src/lib.rs`: replace `MemorySettingsStore::new()` with
  `FileSettingsStore::new(data_dir.clone())`; update the import.
  `create_dir_all(&data_dir)` stays; on unix then set the directory mode to
  `0o700` (`std::fs::set_permissions`), failing setup with a clear error rather
  than ignoring it.
- `app/src-tauri/src/identity_cmd.rs`: change
  `type AppIdentity = IdentityService<OsKeychain, MemorySettingsStore, NodeIdTicketProvider>`
  to use `FileSettingsStore` and fix its imports. Command signatures and DTOs
  do not change.
- Map `SettingsError::Unavailable` as the existing commands already do (typed
  error, no file path or file contents in the message returned to the webview).
- Do not log the settings path contents or any value read from it.

### C2 — Rust restart integration test over the file store

In `core/tests/identity_integration.rs`, add
`display_name_and_provisioned_at_survive_restart`:

1. Tempdir, `Arc<FakeKeychain>` (shared), service #1 with
   `FileSettingsStore::new(dir)`; `provision()`, `set_display_name("Ada 🦀")`.
2. Drop service #1; build service #2 over the **same** dir and keychain with a
   **new** `FileSettingsStore`.
3. Assert `status()` is `Present` with the same `node_id`; `my_info()` returns
   `display_name == Some("Ada 🦀")` verbatim; `FileSettingsStore::get_provisioned_at`
   returns the same timestamp as before restart.
4. Corruption case: overwrite `settings.json` with garbage, build service #3;
   `my_info()` must return `Err(MyInfoError::Settings(_))` (fail closed, no panic,
   no silent reset, no new identity: `status()` is still `Present`).

### C3 — Frontend relaunch behaviour

Current flow (`app/src/App.svelte`): `identity_status` -> `Absent` shows the
wizard, `Present` shows the main screen, `Error` blocks. With a durable store, a
relaunch with a stored name must go straight to the main screen and **never
re-prompt for the display name**. Verify that this already holds and lock it in
with tests in `app/src/App.test.ts`. The existing `vi.mock("./identity", ...)`
factory only provides `identityStatus` and `describeError`; extend it with
`identityMyInfo`, `identityProvision` and `identitySetDisplayName` mocks (reset
in `beforeEach`), and mock `@tauri-apps/plugin-clipboard-manager` as
`MyInfo.test.ts` does, because `MainScreen` renders `MyInfo`, which calls
`identityMyInfo`:

- `relaunch with Present status and a stored name goes to main, not the wizard`:
  `identityStatus` -> `Present`, `identityMyInfo` -> `display_name: "Ada"`;
  assert the wizard (name prompt) is not rendered, the tab list renders, and
  My Info shows `Ada`; assert `identityProvision` and
  `identitySetDisplayName` were **not** called.
- `Present but no stored name still reaches My Info and can be edited`: with
  `display_name: null` the app lands on main, My Info is reachable and its
  edit form lets the user set a name (this is the crash-between-provision-and-name
  case; DD-01 says the name must be captured, so if inspecting
  `FirstRunWizard.svelte` and `MyInfo.svelte` shows the user can reach a valid
  state only through My Info, that is acceptable for this step; do not add a
  new blocking screen). Document the decision in a code comment in `App.svelte`.
- Keep the existing three tests passing (wizard when absent, main when present,
  blocked on error).

Reachability must remain: wizard -> (name saved) -> main -> My Info tab.

### C4 — Run the full gate

Run `.githooks/pre-commit` on the merged tree and fix everything it reports.

---

## Security

| Risk | Fix | How to verify |
|------|-----|---------------|
| **Settings file tampering** (attacker or other process edits `settings.json`, e.g. injects a bidi/newline name that later reaches peers via `Hello`) | Re-run `validate_display_name` on every load; any failure is `Unavailable`, not the raw string. `deny_unknown_fields` rejects extra keys. | `file_store_tampered_display_name_fails_validation`, unknown-field test |
| **Corruption / truncation** (disk full, manual edit) | Corrupt or wrong-version file is `Unavailable`; never treated as "empty"; never overwritten by a later `set_*`. | `file_store_corrupt_json_is_unavailable_and_not_overwritten`, C2 step 4 |
| **Partial writes / crash mid-write** | Temp file in the same dir, `sync_all`, `rename`, dir fsync, temp removed on error (same pattern as `write_marker`). A reader only ever sees the old or the new complete file. | `file_store_atomic_write_leaves_no_temp_file`; code review against `marker.rs` |
| **Wrong permissions** (other local users read the file) | Temp file created `0o600`; app data dir `0o700` on unix; non-unix relies on per-user dir ACLs (commented). | `file_store_file_mode_is_0600`; manual `ls -l` of the data dir in the run check |
| **Path / symlink attacks** (`settings.json` replaced by a symlink to another file, path traversal) | Path is `data_dir.join("settings.json")` with a constant name, never built from user input or the account id; symlinked target rejected via `symlink_metadata`; no `..` components can arise. | `file_store_rejects_symlinked_settings_file` |
| **Oversized file / DoS** | Size cap (64 KiB) checked before reading. | `file_store_oversized_file_is_unavailable` |
| **Secrets leaking into the settings file or logs** | The store API only accepts `DisplayName` and `i64`; the on-disk schema has exactly two fields; no value from the file is logged; error messages returned to the webview carry no path or content. | `file_store_file_contains_no_secrets_and_only_known_keys`; pre-commit secret grep gate (`SecretKeyBytes\|DbKey` absent from `app/`); `grep -n "println\|dbg!\|tracing" core/src/identity/settings.rs` finds nothing that prints values |
| **Concurrent writers losing updates** | Internal `Mutex` serialises read-modify-write in-process. Cross-process races are out of scope (single-instance desktop app). | `file_store_concurrent_writers_do_not_lose_updates` |
| **Plaintext-at-rest deviation from DD-01** | Accepted and recorded: only a display name and a timestamp, no secrets. DD-05 moves them into SQLCipher. | DoD item below; module doc comment |
| **Silent re-provision on settings loss** | Settings loss never affects identity status: `inspect()` keys off marker and keychain only. | C2 step 4 asserts `status()` is still `Present` |

## Quality gates

All must pass; run `.githooks/pre-commit` (install with
`git config core.hooksPath .githooks`):

1. `cargo fmt --all --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --workspace`
4. `cargo build`
5. Secret grep gate (no `SecretKeyBytes` / `DbKey` in `app/`).
6. Clipboard gate (only `app/src/views/MyInfo.svelte` touches the clipboard).
7. `cd app && npm run check && npm test && npm run build`

Do not bypass the hook (`--no-verify` is forbidden). No credentials in code.

## Definition of Done

- [ ] `FileSettingsStore` exists behind the unchanged `SettingsStore` trait,
      re-exported from `core::identity`, with the JSON v1 format above.
- [ ] Writes are atomic (temp + rename + fsync), files are `0o600`, data dir is
      `0o700` on unix, no secrets are ever stored.
- [ ] Corrupt, wrong-version, oversized, symlinked or tampered settings fail
      closed (`Unavailable`) and are never silently reset.
- [ ] Display name is re-validated on load.
- [ ] `lib.rs` and `identity_cmd.rs` use `FileSettingsStore`; display name and
      `provisioned_at` survive an app restart.
- [ ] Relaunch with a stored name does not re-prompt (frontend test passes);
      app still flows wizard -> main -> My Info.
- [ ] Mutual ticket test (B1) passes: each node parses the other's ticket and
      gets the other's NodeId. Live ticket (relay/direct addrs) explicitly
      deferred to DD-02 and noted in the test.
- [ ] Restart round-trip test (B2) passes: repeated stored-then-loaded identity
      yields the same NodeId and account id, and re-provision is rejected.
- [ ] Restart test over the file store (C2) passes, including the corruption case.
- [ ] Module docs state that `FileSettingsStore` is **interim**, deviates from
      DD-01 ("encrypted DB"), and is replaced by DD-05 (SQLCipher); no SQLCipher
      code was added.
- [ ] No credentials in code; all keys generated at runtime.
- [ ] No empty placeholder files or folders were created.
- [ ] Full pre-commit gate passes on the merged tree.
