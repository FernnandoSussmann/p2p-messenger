# Step 04-04 — DD-04 audit and close-out

**Estimate:** S–M — audit plus gap fixes. Typically ~50–400 lines (Rust/TS/tests, depending on the gaps found), plus deletion of the four DD-04 step files.

## TL;DR

Check every item below against the code and fix anything missing:
- every DD-04 goal, security item, test case and Definition-of-done checkbox from
  `design_docs/04-messaging-and-delivery.md`
- the binding user decisions:
  - the encrypted store as the DD-05 seam
  - the targeted chat-text filter
  - `(sent_at_ms, seq)` ordering
  - restart and removed-contact handling of leftovers
  - no auto-dial

Then verify the docs and delete the DD-04 step files (`design_docs/steps/04-messaging-and-delivery/`,
committed on the DD branch in 04-01), so the DD branch is ready to merge into `main`.

## Context and dependencies

- **Requires 04-01, 04-02 and 04-03 merged** into `04-messaging-and-delivery`.
- **Start from the most recent DD-04 branch, not `main`** (`main` may be stale):
  1. `git fetch --all --prune`, then
     `git branch -a --sort=-committerdate | grep -E '04-messaging-and-delivery|step-04-0'`.
  2. If every step PR is merged, branch `step-04-04-dd04-audit-and-close-out` from
     `origin/04-messaging-and-delivery`. Otherwise branch from the newest `step-04-0N-*` branch
     and rebase onto `04-messaging-and-delivery` once the pending PRs merge.
  3. Confirm with `git log --oneline origin/04-messaging-and-delivery | head -20` that the
     "Add DD-04 step files" commit and 04-01..04-03 are in.
  4. Check that `git ls-files design_docs/steps/04-messaging-and-delivery` lists the four files.
     If it does not, the 04-01 branching instruction was skipped: commit them first, so the
     deletion below is a tracked change.
- Open the PR into `04-messaging-and-delivery`. After it merges, the DD branch PR into `main` is
  the human's call.

## Sequential vs parallel

- **A (sequential first):** build the audit table below by reading code and running tests.
- **B (parallelizable fixes):** if there are gaps, group them by disjoint ownership and fix them
  in parallel worktrees:
  - `step-04-04-dd04-audit-and-close-out-4a`: `core/` + `protocol/`
  - `-4b`: `app/src-tauri/`
  - `-4c`: `app/src/`
  - `-4d`: docs

  Skip the sub-branches and fix inline when the fixes are small.
- **C (sequential last):** merge, run the full gates and the manual checklist, then delete the
  step files in the final commit.

## Work items

### A. Audit table

Create the table in the PR description, not in a repo file. Each row is: item → evidence
(file:function or test name) → status (✅ / fixed in this PR). Every row below must end ✅.

**Goals and Definition of done (DD-04):**
1. Two online peers exchange multi-line/emoji UTF-8 text over the encrypted channel, starting
   with `Hello`. Evidence: `happy_path_sending_then_delivered`, `chat_flow_end_to_end`,
   `chat_as_first_frame_is_refused`, and `discovery_channels.rs` (channels exist only after
   Hello).
2. Status goes Sending → Delivered via `Ack`, or Pending/Failed, and is **persisted** (not just
   UI). Evidence: the `MessageStore::set_status` calls in `service.rs`;
   `send_to_offline_peer_stays_pending_and_survives_reload`.
3. Receive is idempotent: no double store or notify, and lost acks are re-sent. Evidence:
   `duplicate_chat_frame_stores_once_and_acks_twice`,
   `lost_ack_is_retried_with_same_id_and_ends_delivered`.
4. The in-memory outbox is backed by rows, retries in-session, and resumes after a channel
   restore; Failed messages can be resent manually with the same id. Evidence:
   `pending_messages_deliver_after_channel_restore`,
   `retries_exhaust_to_failed_then_manual_resend_reuses_id`,
   `restart_reload_resumes_pending_and_marks_overflow_failed`.
5. Everything persists to the encrypted history; scrollback is paged and searchable. Evidence:
   `persistence_round_trip_with_sqlcipher`, `keyset_paging_has_no_gaps_or_duplicates`,
   `history_paging_via_command`, `search_via_command`,
   `sqlcipher_file_never_contains_plaintext_body`.
6. No content is logged or sent outside the channel, and only authorized peers can message.
   Evidence: `messaging_never_logs_bodies_or_secrets`, `removed_contact_cannot_deliver_or_receive`,
   `pending_peer_chat_stream_is_never_processed`, `send_to_non_contact_is_refused`.
7. Frame/body and outbox bounds are enforced, unsupported versions are rejected, and inbound text
   is inert. Evidence:
   - the codec tests, including `body_over_limit_inside_fitting_frame_is_rejected`
   - `oversized_*`, `outbox_full_rejects_without_persisting`, `in_flight_per_peer_is_bounded`,
     `inbound_concurrency_is_bounded`, `unsupported_version_frame_is_rejected_not_decoded`
   - `MessageBubble.test.ts`, `render-guard.test.ts`
8. The design leaves room for store-and-forward without a schema break. Evidence:
   - the `status` column is TEXT with **no SQL CHECK**; statuses are validated by the typed
     `MessageStatus` mapping in Rust (`unknown_status_text_reads_as_unavailable`), so a new
     status (e.g. "queued") needs only a Rust enum variant plus a `user_version` bump, never a
     table rebuild
   - the persisted outbox (the `messages_outbox` index, `undelivered`), a `MessageId` per row,
     and reload-on-start

   Note in the PR that offline relay delivery would only add rows and statuses, with no column
   change.
9. All design-doc tests pass, and `core::messaging` contains real content only (no stubs, no
   empty files): `find core/src/messaging -type f -empty` prints nothing.

**Design-doc Testing section → test name.** All must exist and pass:
- happy path; dedupe; ack loss / retransmit
- peer offline (Pending, survives reload); channel drop then restore
- Failed + manual resend
- version mismatch (at the loop); oversized body (send and receive)
- injection/render safety (the Rust `chat_text_*` tests + `MessageBubble`/`render-guard`)
- persistence round-trip

**User decisions → check:**
- **Store (04-01):**
  - Messages are durable and encrypted at rest with the keychain `DbKey`.
  - The `MessageStore` trait is documented as the DD-05 seam, and the in-memory double is under
    `test-support`.
  - The store implements `HistoryEraser` and replaces `NoMessageStore` in the app:
    `grep -rn NoMessageStore app/` finds nothing.
  - Nothing else from DD-05 was built (no settings migration, no logging pipeline, no
    `history_clear`); `history_search` is the only DD-05-named surface.
- **Chat-text filter (A):**
  - `neutralize_chat_text` neutralizes only bidi embedding/override/isolate controls, ZWSP,
    U+2060–2064, BOM and C0/C1 controls except `\n`/`\t`. It keeps ZWNJ/ZWJ, LRM/RLM/ALM, the
    soft hyphen and the Arabic number signs. It reuses the identity helpers (`is_bidi_control`,
    `is_invisible_break`, `is_disallowed_control`), with no duplicated tables:
    `grep -n "202A\|2066\|200B" core/src/messaging` finds nothing.
  - The display-name validator is unchanged: `display_name_policy_unchanged` and all the
    pre-existing display-name tests pass.
  - The `chat_text_*` tests cover Persian ZWNJ, Hindi ZWJ and emoji ZWJ.
- **Ordering (B):**
  - History sorts by `(sent_at_ms, seq)` through the `messages_by_peer_time` index and a keyset
    cursor (opaque token at the IPC boundary).
  - **No "seq-only ordering" wording remains** in the code docs, README or design doc:
    `grep -rni "seq-only\|ordered by seq\|arrival order only" core app README.md design_docs`.
  - Inbound persists follow accept order (`inbound_persist_follows_accept_order`).
  - `sent_at_ms` is never part of a unique key, dedupe, retry or authorization.
- **Leftovers (C):**
  - Restart reload gives a fresh budget, and rows over the cap become Failed
    (`restart_reload_resumes_pending_and_marks_overflow_failed`).
  - Rows for unknown contacts become Failed on reload (`reload_fails_rows_for_unknown_contacts`).
  - Removal fails the queued messages and frees their slots
    (`removing_contact_fails_pending_and_frees_slots`, `remove_contact_fails_queued_messages`).
- **No auto-dial (D):**
  - `grep -rn "connect(\|connect_ticket\|dial" core/src/messaging` finds nothing.
  - The chat's not-connected note explains reconnecting (`not_connected_note_explains_reconnect`).
  - The DD-02 follow-up (dialing contacts by NodeId) is recorded in the PR and in the design-doc
    status note.

**Conventions (AGENTS.md):**
- `#![forbid(unsafe_code)]` is still present in `core/src/lib.rs`.
- Every SQL statement uses bound parameters, and optional filters use `(?n IS NULL OR …)`.
  `grep -n 'format!' core/src/messaging/sqlcipher.rs` finds no SQL built from data; the key
  pragma is built by pushing nibbles into a `Zeroizing` buffer.
- Every query filters on `account_id`.
- Every engine limit is a named constant in `core/src/messaging/config.rs`, and `MAX_BODY_BYTES`
  is defined only in `protocol` (mirrored in `chat.ts`, enforced by
  `frontend_body_limit_matches_protocol`).
- `PROTOCOL_VERSION` is still `1`, and every DD-04 PR states "no wire-format change". If any step
  changed how `Envelope`/`Payload` are encoded, bump the version now and document it.
- `headless` is untouched by messaging and not referenced from `app/`. `docker compose -f docker/compose.yaml build`
  still succeeds.
- README and AGENTS.md:
  - README: the layout, Chat section, prerequisites and checklist items 20–29 are present and
    accurate. The Chat section explains the sender-clock ordering caveat and the absence of
    auto-dial.
  - AGENTS.md: the layout and testing notes mention `messaging` and `chat_cmd.rs`.

### B. Fix gaps

For every ❌ row, implement the missing code or test in the owning file, following the patterns
and file ownership of the step that should have delivered it. Do not weaken a test to make it
pass, and do not mark tests `#[ignore]`.

### C. Close-out

1. Merge any fix sub-branches and run every gate. Run
   `cargo test -p core --test messaging_two_nodes` 5 times in a row; every run must pass. Walk
   README manual checklist items 1–29 on two profiles: at least items 20–29, plus the items from
   1–19 that DD-04 touches (16 Remove, 18 reconnect).
2. Update `design_docs/04-messaging-and-delivery.md`:
   - set **Status** to `Implemented (v1)` and tick its Definition-of-done checkboxes
   - add one note line covering the delivered choices:
     - per-stream framing (no wire change)
     - ordering by `(sent_at_ms, seq)` as written, accepting sender-clock placement
     - the targeted chat-text filter
     - `history_search` is shared with DD-05, and the store is the DD-05 seam
     - no auto-dial (DD-02 follow-up)
3. Delete the DD-04 step files in the final commit:
   `git rm -r design_docs/steps/04-messaging-and-delivery/`
   - Delete only this folder; leave other DDs' step files, and their pending deletions, as they
     are on the branch.
   - Confirm that nothing still links to the deleted files:
     `grep -rn "steps/04-messaging-and-delivery" --include=*.md .` prints nothing.

## Guardrails

- AGENTS.md security rules are binding. **Never log message plaintext, keys or secrets.**
- `DbKey` / `SecretKeyBytes` never cross into `app/`, not even in comments (grep gate).
- Clipboard access stays only in `app/src/views/MyInfo.svelte`. The words `writeText` and
  `clipboard` must not appear in any other non-test file under `app/src`, including comments.
- `core` keeps `#![forbid(unsafe_code)]`.
- Never bypass the hook (`--no-verify`). Fix the failure instead.
- No empty files, folders, stubs or placeholders. Deleting the step folder must leave no empty
  directory; `git rm -r` handles that, and `design_docs/steps/` remains only if other files are in
  it.
- Wire-format change ⇒ bump `PROTOCOL_VERSION` and say so in the PR (expected: no change).
- Every row and every query is namespaced by `account_id`.
- The profile override must not weaken any control.
- `headless` and `FileKeychain` are never referenced from `app/`.
- **Never write credentials, keys, passwords or secrets in code, test fixtures, config or docs.
  All credentials/keys are provided at runtime (OS keychain / env); test keys are generated at
  runtime.** As part of the audit,
  `grep -rnE '\[0x[0-9a-fA-F]{2}; *32\]' core/src/messaging core/tests/messaging_* app/src-tauri/src/chat_cmd.rs`
  must find no hard-coded key arrays in DD-04 code.

## Security review

Re-verify every risk from 04-01 through 04-03 with its named check, and additionally:

| Risk | Prevention / fix if missing | How to check |
|------|-----------------------------|--------------|
| Plaintext in temp, journal or WAL files | `PRAGMA temp_store = MEMORY` on open; bodies only inside SQLCipher | After a two-node exchange, run the canary scan over every `messenger.db*` sibling. If `sqlcipher_file_never_contains_plaintext_body` covers only the main file, extend it. |
| Erased history lingering in FTS segments or free pages | Erase = DELETE + `INSERT INTO messages_fts(messages_fts) VALUES('optimize')` in one transaction, plus `secure_delete = ON`. If `optimize` is missing, add it. | Keyed checks: `erase_leaves_no_fts_terms` (fts5vocab before == after; MATCH returns 0) and `erase_leaves_no_plaintext_in_export` (`sqlcipher_export` to a plaintext temp DB inside the test, scanned for the canary). A raw scan of the encrypted file is **not** accepted as evidence. |
| Key material lingering in memory | The hex key is built by pushing nibbles into a pre-sized `Zeroizing<String>` (no `format!`/`collect`); `DbKey` is dropped right after the open. Fix any `format!` of key bytes. | Read `open_with_key`; `grep -n 'format!\|collect' core/src/messaging/sqlcipher.rs` |
| Silent DB reset | The open path never deletes, renames or recreates the file; errors are distinct. Remove any such call. | `wrong_key_is_reported_and_file_untouched`; `grep -rn 'remove_file\|rename' core/src/messaging` prints nothing |
| Error strings leaking details | Class-only `Display`; type-tagged `ChatCmdError` | `store_errors_never_embed_details`, `chat_errors_serialize_with_type_tag` |
| Growth without bound | Outbox, in-flight, inbound and page/search limits, each a named constant. **Accepted residual risk** (record it in the PR): an authorized contact can still fill the DB at `MAX_BODY_BYTES` per message, because v1 has no inbound rate limit and DD-04 does not require one. | `outbox_full_rejects_without_persisting`, `in_flight_per_peer_is_bounded`, `inbound_concurrency_is_bounded`, `limits_are_clamped` |
| Queued messages for removed or unknown contacts holding slots | `forget_peer` on removal; the driver's wake check; reload marks rows Failed | `removing_contact_fails_pending_and_frees_slots`, `reload_fails_rows_for_unknown_contacts`, `remove_contact_fails_queued_messages` |
| Unauthorized messaging after contact removal | The `is_known` re-checks; `contacts_remove` still disconnects | `removed_contact_cannot_deliver_or_receive` |
| UI spoofing (bidi, zero-width, controls) | The targeted `neutralize_chat_text` + `<bdi>` isolation, with metadata outside the isolate | `chat_text_*`, `body_is_isolated_from_metadata`, manual item 24 |
| Over-filtering that breaks legitimate scripts | ZWNJ, ZWJ, LRM/RLM/ALM, the soft hyphen and the Arabic number signs are kept | `chat_text_keeps_persian_zwnj`, `chat_text_keeps_hindi_zwj`, `chat_text_keeps_marks_soft_hyphen_and_arabic_signs` |
| Sender-controlled ordering | **Accepted (DD-04 "sender timestamp + receive order")**. `sent_at_ms` is never a key or a security input. Inbound persists follow accept order, so `seq` is deterministic. | `history_orders_by_sender_time_then_arrival`, `inbound_persist_follows_accept_order`, `grep -n sent_at_ms core/src/messaging/{service,outbox}.rs` |
| Status shown wrong because of event races | The bounded `pendingStatus` map; never downgrade from `delivered`; `chat_resync` | `status_event_before_send_reply_is_applied`, `page_load_never_downgrades_delivered` |
| Shutdown blocked by a held `Arc<Node>` | `drop(node)` before the forwarders spawn; `Channel` holds only `Weak` | `shutdown_with_messaging_completes`, `messaging_does_not_keep_node_alive` |

## Tests

- Run the full suite. Every test named in 04-01 through 04-03 must exist:
  `cargo test --workspace -- --list 2>/dev/null | grep -cE 'happy_path_sending_then_delivered|duplicate_chat_frame|lost_ack|offline_peer|channel_restore|manual_resend|unsupported_version_frame|oversized_|persistence_round_trip|chat_text_|keyset_paging|erase_leaves_|reload_fails_rows|removing_contact_fails|inbound_persist_follows_accept_order'`
  should report every family. Read the file list where in doubt.
- For Vitest, check `MessageBubble`, `Composer`, `Chat`, `chatStore` (including both status-race
  tests) and `render-guard` with `cd app && npx vitest list`, or by reading the files.
- Add any missing test found in A. For every bug fixed in B, add a regression test first.

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
git config core.hooksPath .githooks   # commit normally, including the final deletion commit
docker compose -f docker/compose.yaml build   # image must still build (SQLCipher toolchain)
```

The native dependency (rusqlite 0.42 with `bundled-sqlcipher-vendored-openssl`) needs a C
compiler, `perl` and `make`. If the hook fails on a clean machine for that reason, fix the
prerequisite documentation, not the dependency.

PR (per `.github/PULL_REQUEST_TEMPLATE.md`):
- Affected crates/slices for every fix.
- **Protocol impact: no wire-format change, `PROTOCOL_VERSION` 1** (unless the audit forced a
  bump, which must then be explained).
- Verification: the gates plus the manual checklist.
- Include the audit table, the accepted risks (DB fill by a contact, sender-clock placement) and
  the DD-02 follow-up.

## Definition of done

- [ ] The audit table covers every DD-04 goal, Security bullet, Testing case and Definition-of-done item, the user decisions (store, filter, ordering, leftovers, no auto-dial) and the AGENTS.md conventions. Every row is ✅ with evidence.
- [ ] Every gap found is fixed with a test, and no test was weakened or ignored.
- [ ] The grep checks are clean: no `DbKey`/`SecretKeyBytes`/`NoMessageStore` in `app/`; `{@html` only in `MyInfo.svelte`; no links in the chat components; no clipboard words outside `MyInfo.svelte`; no bodies in log macros; no hard-coded key arrays in DD-04 code; no seq-only ordering wording; no dialing in `core/src/messaging`.
- [ ] Store-and-forward readiness is consistent: there is no SQL CHECK on `status`, and statuses are validated in Rust.
- [ ] The `PROTOCOL_VERSION` decision is documented (expected: unchanged).
- [ ] README and AGENTS.md are accurate, and `design_docs/04-messaging-and-delivery.md` is marked Implemented with its DoD ticked and the delivered choices noted.
- [ ] The manual checklist was walked on two profiles, the two-node suite passed 5 runs in a row, and the Docker image builds.
- [ ] `design_docs/steps/04-messaging-and-delivery/` is deleted in the final commit, and all gates are green through the hook.
