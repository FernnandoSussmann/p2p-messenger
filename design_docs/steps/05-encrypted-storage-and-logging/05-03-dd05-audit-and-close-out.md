# Step 05-03 — DD-05 audit and close-out

**Estimate:** S–M. Expect about 0–250 LOC of Rust production fixes, 50–250 LOC of Rust
tests, 0–80 LOC of frontend, and about 80 LOC of docs. Docs include the README checklist,
AGENTS.md, and the DD status and DoD ticks. The step files are deleted at the end.

## TL;DR

This is the last, sequential step. It checks every Goal, Security bullet, Testing bullet
and Definition-of-done item in `design_docs/05-encrypted-storage-and-logging.md` against
the merged code, using concrete commands and tests. Anything missing or weak gets fixed,
with a test. Then it finishes the docs: the README manual checklist and structure,
AGENTS.md, and the DD-05 status set to *Implemented* with its DoD ticked. Last, it
deletes the DD-05 step files. At the end, DD-05 is a complete working feature end to end:
- an encrypted, migrated, `account_id`-namespaced DB holding settings, contacts and
  messages;
- typed, visible storage errors;
- size-bounded local logs with a redaction guard in the pre-commit gate;
- an honest, confirmed `history_clear` reachable from the chat UI.

## Settled decisions (confirmed by the user; audit against them, do not reopen)

1. When the DB cannot open, the app blocks until Retry. The full-screen storage view is
   shown, and networking, contacts and settings stay down until the DB opens.
2. After a failed provisioning, the DB is deleted only if that same attempt created it
   and it holds no user data.
3. A corrupt `settings.json` or `contacts.json` blocks with a typed, visible error, and
   nothing is imported. A corrupt `known_peers.json` is skipped and **stays on disk as
   plaintext**. This is documented, and this audit surfaces it.
4. Logs identify peers by the short NodeId (`fmt_short`).

## Dependencies / ordering

- **Before:** 05-01 and 05-02 are both merged into `05-encrypted-storage-and-logging`.
- **Sequential.** Nothing runs in parallel with this step, and it does not fan out to
  sub-agents. The fixes it finds are usually small and spread across files, so one agent
  keeps them consistent.
- **Start from the newest DD-05 branch, not from `main`.** `main` does not contain DD-05
  until the final PR merges. Run:

  ```sh
  git fetch --all --prune
  git branch -a --sort=-committerdate | grep -E '05-|step-05' | head -20
  git log --oneline -15 origin/05-encrypted-storage-and-logging
  # confirm both step branches are merged into the integration branch:
  git branch -a --merged origin/05-encrypted-storage-and-logging | grep step-05
  ```

  If a `step-05-0x` branch is newer than the integration branch, or is not merged into
  it, stop. Get it merged through its PR first, or report the gap. Never cherry-pick
  around a missing PR.

## Branch

`step-05-03-dd05-audit-and-close-out`, created from `05-encrypted-storage-and-logging`
(the latest commit). Open a PR into `05-encrypted-storage-and-logging`. After that PR
merges, open the final PR `05-encrypted-storage-and-logging` → `main`. DD-04 did the same:
PR #22 into the integration branch, then PR #23 into main.

## What to do

### 1. Audit matrix

Copy this table into the PR description and fill in the **Evidence** column: a test
name, command output or file:line. Every row needs evidence. If a check fails, fix it in
this step and add or strengthen a test.

| DD-05 item | Verify with |
|---|---|
| **Goal / DoD: one SQLCipher DB per install, keychain key; wrong/missing key → recoverable error, never reset** | `cargo test -p core storage::` (`create_then_reopen_with_same_key`, `wrong_key_is_reported_and_file_untouched`, `garbage_file_is_reported_and_left_in_place`); `cargo test -p core --test storage_open`; `cargo test -p app storage_cmd` (`wrong_key_surfaces_error_and_keeps_file`, `retry_recovers_after_key_restored`). `rg -n 'remove_file|truncate\(true\)|DROP TABLE' core/src/storage` shows only `discard_provisioning` (this attempt's empty DB) and `remove_plaintext` (legacy JSON). |
| **Encrypted at rest yet queryable; scrollback + keyword search** | `sqlcipher_file_never_contains_plaintext_body`; the messaging contract suite (paging newest-first, stable across reopen: `rows_survive_closing_and_reopening_the_store`); `search_via_command`; `history_paging_via_command`. Manual checklist item 29 (`grep -c`). |
| **Every row `account_id`-namespaced; every query filters on it** | `cargo test -p core --test sql_namespacing_guard --test storage_namespacing`. Review: there are exactly **two** documented exceptions. One is `messages_fts`, the FTS5 external-content index, reached only through a join on `messages.account_id`. The other is `legacy_imports`, install bookkeeping. Both are allowlisted in the guard (statements and `CREATE TABLE`), documented in `storage/mod.rs` / `storage/legacy.rs`, and noted in the DD-05 doc (add a note under "Schema shape & namespacing"). Any other table without `account_id` is a defect. |
| **Forward-only, transactional migrations without data loss** | `forward_migration_keeps_rows`, `dd04_database_opens_unchanged`, `failing_migration_rolls_back_and_keeps_version`, `rerun_is_noop`, `migrations_are_contiguous_and_match_schema_version`, `storage_legacy_import` (v1 → v4 with every row intact). |
| **Integrity: corrupt DB is a distinct recoverable state** | `Corrupt` is reachable: corrupt a page *after* page 1 in a test copy (flip bytes in the middle of the file) and expect `Corrupt` or `WrongKeyOrCorrupt`, never `Ok` and never a reset. Add `mid_file_corruption_is_detected` if it is missing. |
| **`tracing` logs connection / hole-punch / relay / discovery events (NodeIds + addresses) to a size-bounded rotating file** | `cargo test -p core --test logging_canary` (positive assertions); `rotation_keeps_total_size_bounded`. Manual: run two instances, then `ls -l <log dir>` (modes `-rw-------` and `drwx------`) and `grep -E 'connection state|path opened|home relay' <log dir>/messenger.log`. |
| **Bodies, display names and all secrets provably never logged; automated guard in the pre-commit test gate** | `cargo test -p core --test log_redaction_guard` (self-tests + workspace scan); `logging_canary`. Confirm the hook runs it: `.githooks/pre-commit` runs `cargo test --workspace`. Plant `tracing::info!(body = %x, "y");` in `core/src/messaging/engine.rs`, stage it, run `git commit` → **rejected**, then revert. Do the same for `info!(key = ?k)` and `#[tracing::instrument] fn f(..)`. |
| **History deletion removes rows and reclaims pages as far as feasible; honest UI** | `erase_reclaims_free_pages`, `vacuumed_db_reopens_with_key_and_rejects_wrong_key`, `erase_leaves_no_fts_terms`, `erase_leaves_no_plaintext_in_export`, `history_clear_end_to_end`, `remove_with_erase_clears_chat_history_end_to_end`; Vitest `ClearHistoryDialog.test.ts` (guarantee text). Read the dialog copy against the module docs' "does not guarantee" list. Both must name the same limits. |
| **`history_clear` reachable two ways, same path** | `rg -n 'erase_peer_history' core/src app/src-tauri/src` shows one implementation (`SqlCipherMessageStore`), reached by both `ChatState::clear_history` and `ContactBook::remove`. |
| **Remove-contact "also delete message history" is opt-in, default-off (DD-05 Data deletion)** | `RemoveContactDialog.svelte` initialises `eraseHistory` to `false`; `RemoveContactDialog.test.ts` asserts the checkbox renders unchecked and that confirming without ticking calls `contactsRemove(id, false)`; the backend `contacts_cmd::remove` maps `clear_history: None`/`Some(false)` to `HistoryChoice::Keep`. Add an assertion if one is missing. |
| **Missing DB is never silently recreated** | `missing_db_with_identity_is_reported_not_recreated`, `missing_db_surfaces_missing`; the creation policy in `storage/handle.rs` allows creation only with no marker or with legacy JSON present. |
| **Settled decision: a corrupt `known_peers.json` stays on disk as plaintext** | `corrupt_known_peers_is_skipped_and_left_on_disk`. The README and the `storage/legacy.rs` docs say so. On the audit machine, run `ls <data dir>/known_peers.json` for each test profile. If one remains, report it in the PR, with whether it parses, and tell the user it can be deleted. This is the plaintext residue the design accepts on purpose, and the audit must surface it, not hide it. |
| **Settled decision: a failed provisioning never leaves the app stuck** | `failed_set_provisioned_at_then_reprovision_succeeds`, `failed_marker_write_then_reprovision_succeeds`, `existing_db_is_never_discarded`, `db_with_user_data_is_never_discarded`, `discard_with_extra_handle_defers_and_never_returns_other_account`. |
| **Logs local-only, permission-restricted, never transmitted** | `logging_has_no_network_sink`; `log_files_and_dir_are_private`; `cargo tree -p app -e normal \| rg -i 'opentelemetry\|sentry\|loki\|gelf\|syslog\|journald\|datadog\|tracing-appender'` → empty. |
| **Security: DB key hygiene (never logged, returned or written)** | `storage_errors_never_embed_details`; `status_json_never_contains_db_key`; `dto_json_never_contains_db_key`; the pre-commit `DbKey` / `SecretKeyBytes` grep. `rg -n 'DbKey' core/src` must show only `identity/{mod,secret,keychain,keygen,service}.rs` (`mod.rs` is the re-export), `storage/{database,handle}.rs`, and test code (`tests.rs`, `tests/`, `testing`). |
| **Security: plaintext never on disk unencrypted** | Raw-file scans; `data_dir_has_no_plaintext_after_import`; `session_pragmas_are_enforced` (`temp_store=MEMORY`, `secure_delete=ON`). `rg -n 'File::create\|fs::write\|OpenOptions' core/src app/src-tauri/src` lists only the marker, the logging writer, `ensure_private_regular_file` and `remove_plaintext`. Investigate anything else. |
| **Security: error messages don't leak** | Every `Display` of `StorageError`, `SettingsError`, `ContactStoreError`, `MessageStoreError`, `HistoryEraseError` and `LogInitError` is a fixed string (unit tests). Every webview DTO error is a type tag or a static reason. |
| **Security: corruption ≠ reset** | As in the first row. Also confirm no code path calls `Storage` with a "create fresh" option. None may exist. |
| **Security: namespacing is a safety boundary** | As in the namespacing row; `other_account_is_refused`. |
| **Testing: no network sink** | `logging_has_no_network_sink`. |
| **Non-goals respected** | No passphrase layer, telemetry, sync or ranking was added. The key-wrapping seam (`KeySource`) remains open but unused. |

### 2. Gaps to look for specifically

These are the places a parallel build most often leaves inconsistent:
- **Leftover plaintext writers or readers.**
  `rg -n 'contacts.json|settings.json|known_peers.json' core/src app headless` should
  match only the legacy readers, `storage/legacy.rs`, tests and docs.
- **Leftover `eprintln!` / `println!`** in `app/src-tauri/src` (the guard should already
  fail on these).
- **Stale docs.** `rg -n 'until DD-05|interim|DD-05 seam|plaintext' README.md AGENTS.md core/src`
  shows what to update. Update every claim that is no longer true. Keep the historical
  notes in the DD-01 to DD-04 docs as they are.
- **The storage screen is reachable after every entry point:** launch, the end of the
  wizard, and Retry.
- **Profile isolation of logs:** two profiles never write to the same `messenger.log`
  (`log_dir_is_profile_namespaced`).
- **Headless:** `cargo test -p messenger-headless`, plus `docker/smoke.sh` if Docker is
  available. Record the result in the PR or state that it was skipped.

### 3. Docs

- **README:**
  - Workspace layout: the `core` row lists `storage` and `logging`.
  - "Encrypted storage" section: what is stored; the keychain key; error states and
    Retry; the one-time legacy import and its limits.
  - "Diagnostic logs" section: location per OS and profile, `MESSENGER_LOG`, the size
    bound, the never-logged list, and that clearing logs is a privacy action.
  - "Chat": `history_clear` with the guarantee text.
  - "Pre-commit hook": mention the redaction and namespacing guards among the tests it
    runs.
  - Manual GUI checklist: **verify** the items that 05-01 and 05-02 already added, and add
    only the ones that are missing. Do not renumber or duplicate existing items. The
    DD-05 set must cover:
    1. a corrupted `messenger.db` shows "Encrypted storage unavailable", leaves the file
       untouched, and Retry after restoring a copy recovers;
    2. after upgrading a DD-04 profile, settings and contacts are kept and
       `settings.json`/`contacts.json` are gone;
    3. the log files are `0600` in a `0700` dir and contain connection and path lines,
       and `grep` for a sent word or a nickname in the log dir returns nothing;
    4. **Clear history…** confirms with honest copy, empties the chat and search, and the
       peer's side is unchanged;
    5. `MESSENGER_LOG=bogus` refuses to start;
    6. moving `messenger.db` away from a profile with an identity shows the storage
       screen with the "missing" reason, and no new file is created;
    7. the Remove contact dialog's "also delete message history" box is unchecked by
       default.
  - Update the "Manual GUI checklist" intro so it names the new command-level flow tests.
- **AGENTS.md:**
  - Layout row for `core`: add `storage/` and `logging/`.
  - Notes: all persisted data lives in `messenger.db` through `core::storage`; new
    tables are migrations; `messages_fts` and `legacy_imports` are the only tables
    without `account_id` (both documented and allowlisted).
  - Security rules: logs go only through `core::logging` helpers, and the guard is
    enforced.
  - Testing: list `log_redaction_guard.rs`, `logging_canary.rs`,
    `sql_namespacing_guard.rs`, `storage_*.rs`.
- **`design_docs/05-encrypted-storage-and-logging.md`:**
  - set `**Status:** Implemented`;
  - tick every DoD box. Tick only items with evidence in the audit table;
  - add a short "Implementation notes" paragraph covering:
    - the size-based roller chosen instead of `tracing-appender`, and why;
    - the `messages_fts` and `legacy_imports` namespacing exceptions;
    - the settled decisions: block until Retry; narrow provisioning cleanup; a corrupt
      `settings.json`/`contacts.json` blocks, while a corrupt `known_peers.json` is
      skipped and left on disk; short NodeIds in logs;
    - `VACUUM` + `secure_delete` as the reclaim, and its stated limits;
    - the short-NodeId logging choice.
- **`design_docs/README.md`:** mark DD-05 as implemented if that file tracks status.

### 4. Delete the step files (last commit of this step)

After every gate passes and the docs are final:

```sh
git rm design_docs/steps/05-encrypted-storage-and-logging/05-0*.md
# the folder is then empty; git does not track empty folders, so nothing else is needed
test -z "$(ls -A design_docs/steps/05-encrypted-storage-and-logging 2>/dev/null)" && rmdir design_docs/steps/05-encrypted-storage-and-logging 2>/dev/null || true
git commit -m "Close out DD-05: mark Implemented, tick DoD, remove step files"
```

The commit goes through the hook normally.

## Guardrails

- AGENTS.md conventions apply:
  - vertical slices;
  - no stubs, placeholder modules, empty files or empty folders;
  - `#![forbid(unsafe_code)]` in `core`;
  - `account_id` on every row and query, except the documented `messages_fts` and
    `legacy_imports`.
- `SecretKeyBytes` / `DbKey` never appear under `app/`, not even in comments.
  `FileKeychain` and headless never appear in `app/`. Clipboard access stays only in
  `MyInfo.svelte`.
- Never write credentials, keys, passphrases or tokens in code, test fixtures committed as
  real secrets, config or docs. All credentials and keys are obtained at runtime, from the
  OS keychain or generated in tests. The planted guard checks in the audit are **never
  committed**. Revert them after you see the hook reject them.
- Never bypass the hook (`--no-verify`).
- Do not weaken a guard, test or check to make the audit pass. Fix the code. If an
  allowlist entry really is needed, add it with a one-line justification comment and
  mention it in the PR.
- Wire format unchanged: `PROTOCOL_VERSION` is untouched. Confirm with
  `git diff origin/main -- protocol/` → empty.

## Security

This step audits the whole DD-05 attack surface. For each item, check the code and run
the listed check:

| Risk | Prevention | How to check |
|---|---|---|
| Key leak (Debug, Display, errors, logs, Tauri) | Payload-free error enums; `DbKey`/`SecretKeyBytes` redacted `Debug`, no `Display`/`Clone`/serde; static DTO reasons; `KeySource` keeps the key type out of `app/` | Error-`Display` unit tests; the DTO key-absence tests; the canary; the pre-commit grep. Read `storage/database.rs` `apply_key`: `Zeroizing`, fixed hex alphabet, dropped immediately. |
| PRAGMA key formatting or injection | Key text built only from a fixed hex alphabet; never `format!`; no `SQLITE_OPEN_URI` | `rg -n 'PRAGMA key' core/src --glob '!**/tests.rs' --glob '!**/tests/**' --glob '!**/testing/**'` shows exactly one construction site (`storage/database.rs`). Test files may build DD-04-era fixtures with their own `PRAGMA key`. |
| SQL injection (FTS `MATCH`, `LIKE`, names) | Constant SQL + bound parameters only; `fts_phrase` quoting; `like_pattern` escaping | `fts_phrase` / `like_pattern` tests; `hostile_strings_round_trip`; the namespacing guard flags inline SQL. |
| Plaintext residue (WAL/journal/temp/free pages/legacy JSON) | SQLCipher-encrypted journal; `temp_store=MEMORY`; `secure_delete=ON`; VACUUM on clear; legacy files zero-filled (`nlink == 1`) and removed after commit; corrupt `known_peers.json` kept by design and documented | `session_pragmas_are_enforced`, `erase_reclaims_free_pages`, `data_dir_has_no_plaintext_after_import`, `corrupt_known_peers_is_skipped_and_left_on_disk`, and the raw-file scans over `messenger.db*`. |
| TOCTOU / symlinks on the DB, log and legacy paths | `symlink_metadata` checks; refuse non-regular files; post-open inode comparison for logs; legacy symlink → typed `NotRegularFile`, never followed | Unix tests `symlinked_db_is_refused_and_target_untouched`, `symlinked_log_file_is_refused`, `symlinked_legacy_file_is_typed_error_and_target_untouched`, `hardlinked_legacy_file_is_unlinked_not_zeroed`. |
| World-readable files | `0600` files, `0700` dirs, tightened on open and on rotation | `db_file_is_private`, `loose_db_permissions_are_tightened`, `log_files_and_dir_are_private`; the manual `ls -l`. |
| Log injection (newlines/ANSI/bidi) | `with_ansi(false)`; the writer neutralises control and bidi characters | `writer_neutralises_control_and_bidi_chars`. |
| Unbounded log growth / DoS | Size-based roller, file-count cap, line cap | `rotation_keeps_total_size_bounded`, `writer_caps_line_length`. |
| Secrets, bodies, names or tickets in logs or stderr | Redaction guard (field allowlist, forbidden identifiers inside log-macro args, `#[instrument]` rule, direct-output ban with one exact allowlisted line); production filter caps third parties at WARN | `log_redaction_guard`, `logging_canary`, `filter_caps_third_party_targets`; the planted-violation commits. |
| Cross-account leaks | `account_id` on every row and query except the two documented tables; `OtherAccount` | `storage_namespacing.rs`, `sql_namespacing_guard.rs`, `other_account_is_refused`. |
| Silent DB recreation (wrong key, corrupt, missing) | No reset command; typed latched errors; creation only during provisioning or legacy import | The first-row tests; `missing_db_with_identity_is_reported_not_recreated`; no reset command (`rg -n 'generate_handler' -A40 app/src-tauri/src/lib.rs`). |
| Provisioning leaves an orphan DB or a stuck app | `discard_provisioning`: always resets the slot; deletes only this attempt's empty file; defers with an unlatched error | The provisioning-cleanup tests listed in the audit matrix. |
| Partial migration or import | One IMMEDIATE transaction per migration and per legacy file; the `legacy_imports` row is in the same transaction | `failing_migration_rolls_back_and_keeps_version`, `failed_import_rolls_back_and_keeps_file`, `stale_legacy_file_is_never_reimported`. |
| Self-deadlock or a blocked runtime in storage code | Read-modify-write uses one guard and its own transaction; the import never re-enters `Storage`; VACUUM only via `spawn_blocking` | The contract suite under threads; `import_does_not_reenter_storage`; `rg -n 'erase_peer_history' app/src-tauri/src` shows only `spawn_blocking`/`blocking` call sites. |
| Unconfirmed destructive webview command | `confirm: true` required; the dialog has Cancel as the default; remove-contact erase is default-off | `history_clear_requires_confirmation`; the dialog tests; the default-off audit row. |
| Overclaimed deletion | One shared copy constant (`HISTORY_DELETE_GUARANTEE`) naming the limits | The copy review (audit row "History deletion"). |
| Profile override weakening a control | The profile only changes paths (data dir, keychain service, log dir); every check is identical | `log_dir_is_profile_namespaced`, `profile_isolation.rs`; read `profile.rs` for any conditional on the profile beyond path selection. |

## Tests

- Add the tests for any gap found, named after the DD-05 Testing bullet they cover.
  Likely candidates: `mid_file_corruption_is_detected`, and the docs-claim checks above.
- The full DD-05 Testing section must map to named, passing tests. Include the mapping
  table (bullet → test names) in the PR:
  - keyed open round-trip;
  - encrypted at rest;
  - migration forward and rollback;
  - namespacing;
  - scrollback paging;
  - search;
  - history deletion;
  - log redaction guard;
  - log rotation;
  - no network sink.

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

Keep the hook installed (`git config core.hooksPath .githooks`) and commit normally;
never use `--no-verify`. Also run the planted-violation commits described in the audit
matrix. They must be rejected, and they must never be committed.

## Usable-state guarantee

At the end of this step DD-05 is delivered end to end:
- `npm start` launches. A fresh profile provisions into the encrypted DB. An upgraded
  DD-04 profile imports and removes its plaintext files.
- Chat, history, paging and search work on-device.
- A broken DB or key shows the reachable "Encrypted storage unavailable" screen, with
  Retry and no data loss.
- **Clear history…** works from the chat view, and remove-contact erase uses the same
  path.
- Diagnostic logs appear in the OS log dir, bounded and private, and without bodies,
  names or secrets.
- The pre-commit hook enforces the redaction and namespacing guards.

## Definition of done

- [ ] Started from the newest DD-05 branch, with both step branches confirmed merged.
- [ ] The audit matrix is fully evidenced in the PR, and every failing row is fixed with a
      test.
- [ ] The planted log-violation commits were rejected by the hook (and reverted).
- [ ] README (layout, Encrypted storage, Diagnostic logs, Chat, Pre-commit, manual
      checklist) and AGENTS.md are updated; no stale "plaintext until DD-05" claims
      remain.
- [ ] DD-05 status is *Implemented*, the DoD is ticked, and the implementation notes are
      added.
- [ ] `cargo test -p messenger-headless` passes. `docker/smoke.sh` was run, or its skip is
      stated.
- [ ] All quality gates pass, and every commit went through the hook.
- [ ] All `design_docs/steps/05-encrypted-storage-and-logging/*.md` files are deleted (and
      the folder with them).
- [ ] The PR into `05-encrypted-storage-and-logging` follows the template ("No change to
      the wire format"). The final PR `05-encrypted-storage-and-logging` → `main` is
      opened.
