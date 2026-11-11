# Step 03-03 — DD-03 audit and close-out

**Design doc:** [DD-03 Contacts & Trust](../../03-contacts-and-trust.md)
**Depends on:** **03-01 and 03-02 complete on their branches** (they will
usually **not** be merged into `main` yet). This is the last step and runs
sequentially. The DD-03 branches are **stacked**: 03-01 → 03-02 → 03-03, so
the newest DD-03 code is on `step-03-02-contact-commands-and-contact-list-ui`,
not on `main`.
**Parallelism:** none. The audit itself is sequential. If it turns up several
independent gaps, you may fix them in parallel worktrees, but only when the
fixes touch disjoint files. List the files each fix owns in the PR.
**Branch:** `step-03-03-dd03-audit-and-close-out`, created **from
`step-03-02-contact-commands-and-contact-list-ui`** (never from `main`).
**PR base:** `step-03-02-contact-commands-and-contact-list-ui` (stacked PR), so
the diff shows only this step's audit fixes and docs.

## Goal

Prove that every DD-03 Goal, Security bullet, Testing bullet and
Definition-of-done checkbox is met by code and tests on this step's branch
(the full 03-01 + 03-02 stack), or is a
recorded user decision or deviation. Implement anything that is missing. Run
all quality gates. Update the docs, including AGENTS.md. Then **delete every
step file in `design_docs/steps/03-contacts-and-trust/` and the folder
itself.**

Do not trust earlier checkboxes. Verify every item against the code: read the
test body, run it, and run the checks.

## User decisions and deviations to record

These are already decided. Do not reopen them; verify and document them.

| # | Decision / deviation | Where it must be recorded |
|---|----------------------|---------------------------|
| A | **NodeId copy by selection only.** There is no copy button, to keep the AGENTS.md clipboard rule. The full NodeId is revealed as selectable text. **User-accepted deviation** from DD-03's "copy" | DD-03 Status line, README Contacts section, PR |
| B | **Contact list shows only the local nickname** (plus NodeId and badge). The self-asserted name is stored (`self_name`), shown in the Accept and naming prompts, and reserved for a future contact-info screen (code comment only). This is a **user-decided refinement** of DD-03's "shown faintly as 'calls themselves X'" | DD-03 Status line, README, PR |
| C | **Default nickname** = the validated self-asserted name, else `Peer <short NodeId>`. It is a snapshot and is never re-derived | README, PR |
| D | **Shared DD-01 validator rejects invisible format (Cf) characters** (refined). The only exceptions are a ZWJ *between two `Extended_Pictographic` emoji* (each optionally followed by U+FE0F or one skin-tone modifier) and a well-formed tag sequence `U+1F3F4 + U+E0020–U+E007E… + U+E007F`. Classification uses a pinned Unicode 16.0 range table, with no new dependency and no `unsafe`. This tightens DD-01 display names. Homoglyphs stay allowed. A stored name that now fails is treated as unset | README, DD-03 Status line, PR (note the DD-01 impact) |
| E | **A successful dial auto-adds an Unverified contact** with the default nickname. The naming prompt is prefilled with it, and "Not now" keeps the contact. **Extended to Accept:** the inbound Accept nickname is prefilled with `Nickname::default_for(id, self_name)` (backend `suggested_nickname`). It is editable, a cleared field blocks Accept, an explicit Accept click is required, and the self-asserted name is still shown for context | README, PR |
| S | **Interim plaintext contact store** (`contacts.json`) until DD-05's encrypted `contacts` table | DD-03 Status line, README, `store.rs` docs, PR |
| H | **History deletion seam with a no-op production eraser** (`NoMessageStore`), because no message store exists before DD-04/DD-05 | DD-03 Status line, README, `history.rs` docs, PR |
| O | **"Open chat" row action deferred to DD-04.** No placeholder button | DD-03 Status line, Audit D, PR follow-ups |
| F | Duplicate nicknames are allowed, because the NodeId disambiguates | README Contacts section, PR |

## Procedure

1. Start from the tip of the 03-02 branch, **not `main`**:
   ```sh
   git fetch origin
   git switch step-03-02-contact-commands-and-contact-list-ui
   git pull --ff-only     # pick up any review fixes pushed to 03-02
   git switch -c step-03-03-dd03-audit-and-close-out
   ```
   Then confirm the stack is really there before auditing anything:
   `git log --oneline main..HEAD` must show the 03-01 and 03-02 commits, and
   `core/src/contacts/` plus `app/src/views/Contacts.svelte` must exist. If
   they are missing, stop: the wrong base was checked out.
   If 03-02 has already been merged by the time this step starts, branch from
   its merge target instead and set the PR base to match.
2. Run the full quality gates once to establish a green baseline.
3. Work through Audits A–E. For every row:
   - paste the evidence (file + test name, or command output) into the PR's
     audit section;
   - if the evidence is missing, or the test does not really assert the claim,
     implement or fix it now, with a test, following the 03-01/03-02
     conventions.
4. Do the manual GUI checklist in `README.md` on two profiles (Level 2). Run
   `docker/smoke.sh` if Docker is available.
5. Update the docs as described under **Docs**.
6. Delete the step folder as described under **Close-out**. Commit, then open
   the PR.

---

## Audit A — DD-03 Goals

| Goal | Required evidence |
|------|-------------------|
| Persist contacts keyed by NodeId, namespaced by `account_id` | Document keyed `accounts → <uuid> → <node_id>` in `core/src/contacts/store.rs`. Tests: `book::books_for_different_accounts_are_isolated`, `contacts_are_namespaced_by_account_end_to_end`, `contacts_are_namespaced_by_bound_account`, `store::survives_reopen` |
| Local nickname independent of the self-asserted name | Separate `nickname` and `self_name` fields. Tests: `book::rename_keeps_node_id_self_name_and_trust`, `book::observe_hello_keeps_nickname_and_trust`, `book::default_nickname_is_a_snapshot`, `peer_rename_updates_self_name_not_nickname`, `lan_name_change_updates_self_name_of_existing_contact_only` |
| Unverified → verified surfaced in UI | `TrustState`, `TrustBadge.svelte`, `ContactRow.test.ts`, `VerifyContactDialog.test.ts` |
| Serve DD-02's "known contact?" check cheaply and correctly | `impl KnownPeers for ContactBook` — the gate calls `is_known` via `known_call`/`spawn_blocking`, and that is the same `ContactBook::is_known` behind `contacts_is_known`. Cost: one bounded file read per inbound connection (≤ 32 MiB cap), off the async runtime. Observations are throttled (`LAST_SEEN_RESOLUTION_MS`, LAN only on name change). Note in the PR that an in-memory index belongs to DD-05. Tests: `accept_with_nickname_lists_unverified_contact_and_reconnects_silently`, `removed_contact_is_prompted_again`, `corrupt_contacts_file_fails_closed_and_prompts`, `store::observe_skips_write_when_nothing_material_changed` |
| Adding a contact is one deliberate action on both discovery paths, plus inbound Accept | LAN: `LanPeers.svelte` Add… (`LanPeers.test.ts`). Ticket: auto-add plus naming prompt (`dialed_peer_becomes_unverified_contact_with_default_nickname`, `NamingPrompt.test.ts`, `contactsStore.test.ts`). Accept: `accept_incoming` (`IncomingRequestPrompt.test.ts`, `ticket_connect_accept_flow_end_to_end`) |

Also confirm that the non-goals were **not** built: no QR or safety-number
ceremony, no groups, no sync, no block list, no import.

## Audit B — DD-03 Security bullets

| Security bullet | Required evidence |
|-----------------|-------------------|
| Trust keys on NodeId, never on a name | Read `ContactBook::is_known`, the `KnownPeers` impl, the gate in `node.rs`, `accept_incoming` and `remember_dialed`. None of them compares names. Checks **E1**, **E2**. Tests: `store::same_nickname_different_nodes_are_distinct`, `keeps same-named peers distinct by NodeId` |
| Names can't spoof the UI (DD-01 validation on both names before storage and render) | The validator includes the Cf set (decision D). Nicknames, Hello names and LAN names are validated, and the store re-validates on read. Names render as text in `<bdi>`, and the list shows only the user's nickname (decision B). Tests: `display_name::rejected_inputs`, `display_name::zwj_outside_emoji_is_rejected`, `display_name::malformed_tag_sequences_are_rejected`, `display_name::well_formed_emoji_sequences_are_accepted`, `emoji::table_is_sorted_non_overlapping_and_spot_checked`, `model::nickname_rejects_…`, `store::corrupt_files_…`, `hello_name_with_invisible_char_is_dropped`, `ContactRow.test.ts › does not render the self-asserted name…`, `html-guard.test.ts` |
| Unverified looks unverified; badge not suppressible by peer data; verified changes only via `contacts_set_verified` | `TrustBadge` fails closed (`=== true`), comes first in the row and never shrinks. Tests: `ContactRow.test.ts › badge cannot be spoofed…`, `book::only_set_verified_changes_trust`. Check **E3** |
| Honest "verified" semantics | The copy in `TrustBadge.svelte` and `VerifyContactDialog.svelte`. `VerifyContactDialog.test.ts` includes a negative regex. README says the same |
| Removal explicit; destruction opt-in; both outcomes unmistakable | `ContactBook::remove` order is erase, then remove. `HistoryChoice::default() == Keep`. `contacts_remove(clear_history: Option<bool>)`. The `RemoveContactDialog` copy. Tests: `book::remove_*`, `contacts_remove_*`, `RemoveContactDialog.test.ts`, `removed_contact_is_prompted_again`, `contacts_remove_drops_live_connection_and_next_inbound_prompts` |
| No contact data in diagnostics | `core/tests/contacts_log_guard.rs` is green. Checks **E4**, **E5**. See also residual risk R2 (headless stdout) |

## Audit C — DD-03 Testing bullets

Each bullet needs at least one **passing** test whose body really asserts the
claim.

| DD-03 test bullet | Tests |
|-------------------|-------|
| Unit — add/known | `book::add_makes_is_known_true_only_for_that_node`, `contacts_is_known_tracks_add_and_remove` |
| Unit — nickname independence | `book::rename_keeps_…`, `book::observe_hello_keeps_…`, `book::default_nickname_is_a_snapshot`, `peer_rename_updates_self_name_not_nickname` |
| Unit — verify toggle | `book::only_set_verified_changes_trust`, `book::observe_unknown_never_creates_a_contact`, `book::remember_dialed_keeps_existing_nickname_and_trust` |
| Unit — name sanitization | `display_name::rejected_inputs`, `model::nickname_rejects_…`, `store::corrupt_files_…`, `hello_name_with_invisible_char_is_dropped`, `contacts_add_invalid_nickname_kinds`, Vitest `names render inert` |
| Unit — removal reverts trust | `book::remove_reverts_trust`, `removed_contact_is_prompted_again` |
| Unit — removal + clear-history flag | `model::history_choice_defaults_to_keep`, `book::remove_keep_never_calls_eraser`, `book::remove_erase_calls_eraser_once_then_removes`, `book::remove_keeps_contact_when_history_erase_fails`, `contacts_remove_defaults_to_keeping_history`, `RemoveContactDialog.test.ts › unchecked by default` |
| Integration (two nodes) | `contacts_two_nodes::accept_with_nickname_lists_unverified_contact_and_reconnects_silently`, `dialed_peer_becomes_unverified_contact_with_default_nickname`, parity `unknown_prompts_once_then_known_reconnects_silently` (in-process and headless) |
| Namespacing | `book::books_for_different_accounts_are_isolated`, `contacts_are_namespaced_by_account_end_to_end`, `contacts_are_namespaced_by_bound_account`, `migrate::migrates_every_account_separately` |

Also confirm that the DD-02 tests still assert DD-02's requirements after the
03-01 rewrite:

- an unknown peer prompts once;
- a decline persists nothing;
- the flood guard holds;
- a failing store fails closed;
- `dialed_peer_is_remembered_only_after_a_valid_reply` and
  `lan_pick_uses_the_same_trust_gate` still pass.

## Audit D — DD-03 Definition of done

| DoD checkbox | What to verify |
|--------------|----------------|
| Contacts persist keyed by `(account_id, NodeId)` **in the encrypted DB** | **Met with documented deviation S.** Behind the `ContactStore` trait, `FileContactStore` is plaintext and interim, and DD-05 replaces it. Confirm the wording in the `store.rs` docs, the `contacts/mod.rs` docs, README and the PR |
| Added from LAN, ticket and inbound Accept, all converging on the same Unverified record | LAN and naming-fallback go through `contacts_add` → `ContactBook::add`. A dial goes through `remember_dialed` and Accept through `accept_incoming` → `ContactBook::add`. Every creation path inserts `Unverified`. Check **E6**. Do the manual checklist for all three paths |
| Local nickname editable and fully independent | Audit C nickname rows, plus a manual rename |
| Unverified/verified states; honest verify; badge unspoofable | Audit B rows 3–4 |
| Rows render name-bold / NodeId-gray-beneath with an unverified badge | `ContactRow.test.ts › renders nickname bold…` and a manual check. Under decision B, rows show only the nickname |
| `contacts_is_known` backs DD-02's trust gate | Both the `KnownPeers` impl and `contacts_cmd::is_known` call `ContactBook::is_known`. Check **E7** |
| Removal reverts trust; opt-in default-off history option delegates to DD-05; both outcomes surfaced | Through the `HistoryEraser` seam. The production `NoMessageStore` is documented (deviation H), and DD-05 swaps in the real eraser. Audit B row 5 |
| Row actions include "open chat (DD-04)" | **Deferred to DD-04 (deviation O).** Check **E8** shows no placeholder button. Listed in the PR follow-ups |
| All tests pass; `core::contacts` joins with real content only | The gates are green. Check **E9** finds no empty files. Clippy `-D warnings` catches dead code |

## Audit E — checks (run all; paste the output in the PR)

These commands run from the repo root. Patterns use `grep -E` with a plain `|`.

```sh
# E1 no name comparison used as identity — every hit must be test code or display-only; review each by hand
grep -rnE 'nickname *==|self_name *==|display_name *==|as_str\(\) *==' core/src app/src-tauri/src headless/src

# E2 Svelte lists are never keyed by a name
grep -rnE '\((c|p|r|contact|peer)\.(nickname|display_name|self_name)\)' app/src --include=*.svelte   # expect no output

# E3 trust changes only via set_trust / set_verified, never from discovery
grep -rnE 'set_trust|set_verified' core/src/discovery        # expect no output
grep -rnE 'set_trust\(|set_verified\(' core/src app/src-tauri/src headless/src
#   expect only: ContactStore trait + impls, ContactBook::set_verified, contacts_cmd::set_verified, tests

# E4 no names in log/print macros
grep -rnE '(trace|debug|info|warn|error|eprintln|println)!\(.*(nickname|self_name|display_name)' core/src app/src-tauri/src headless/src   # expect no output

# E5 no Debug formatting of contacts/names in logs
grep -rnE '(trace|debug|info|warn|error|eprintln|println)!\(.*\{:\?\}' core/src/contacts core/src/discovery app/src-tauri/src
#   review each hit: the argument must not be a Contact, Nickname, DisplayName or DTO

# E6 contact creation paths
grep -rnE '\.(insert|insert_missing)\(' core/src/contacts --include=*.rs | grep -v testing.rs
#   expect only: ContactBook::add, ContactBook::remember_dialed, migrate.rs, store impls/tests

# E7 one is_known implementation behind both the gate and the command
grep -rn 'fn is_known' core/src app/src-tauri/src

# E8 no placeholder "Open chat"
grep -rniE 'open chat|openChat' app/src --include=*.svelte   # expect no output

# E9 no empty files in the slice or the app
find core/src/contacts app/src app/src-tauri/src -type f -empty   # expect no output

# E10 hook gates
grep -RnE 'SecretKeyBytes|DbKey' app --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=target --exclude-dir=gen    # expect no output
grep -RnE 'FileKeychain|messenger-headless|messenger_headless' app --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=target --exclude-dir=gen   # expect no output
grep -RlE 'writeText|clipboard' app/src --exclude='*.test.ts'   # expect only app/src/views/MyInfo.svelte
grep -rn '{@html' app/src --include=*.svelte                    # expect only app/src/views/MyInfo.svelte

# E11 test doubles never in shipped app code.
#   Authoritative proof: the doubles live behind core's `test-support` feature, which the app enables
#   only in [dev-dependencies] (asserted by app/src-tauri/tests/no_test_backends.rs). A non-test
#   use therefore fails this build:
cargo build -p app
#   Plus a manual review: every hit below must be inside test_util.rs or a `mod tests` block.
grep -rnE 'MemoryContactStore|FailingContactStore|RecordingHistoryEraser|MemoryKnownPeers|BookCell' app/src-tauri/src

# E12 unsafe
grep -rn 'unsafe' core/src | grep -v 'forbid(unsafe_code)'   # expect no output

# E13 interim known-peers store removed
grep -rnE 'FileKnownPeers|known_peers\.json' core/src app headless docker README.md
#   expect only core/src/contacts/migrate.rs and the README migration note

# E14 the node's trust-store calls are exactly is_known / observe / remember_dialed, all via known_call
grep -nE 'known\.(add|insert)\(' core/src/discovery/node.rs                 # expect no output
grep -nE '\.(is_known|observe|remember_dialed)\(' core/src/discovery/node.rs
#   expect: the inbound gate, authorize (Hello), lan_loop (Lan), run_dial
grep -nE 'inner\.known\.' core/src/discovery/node.rs                        # expect hits only inside fn known_call

# E15 single production store instance per binary
grep -rn 'FileContactStore::new' app/src-tauri/src headless/src   # expect one in app lib.rs, one in headless node.rs (+ tests)

# E16 no wire change
git diff main...HEAD -- protocol/   # whole DD-03 stack vs main; expect empty
```

Also confirm the following:

- **Migration.** It runs at app and headless startup in one atomic write. A
  corrupt legacy file stays untouched and those peers are prompted. A legacy
  name that now fails decision D counts as no name. The legacy file is deleted
  only after success.
- **Validator tightening.** A stored display name that now fails reads as unset
  (`settings::stored_name_now_invisible_reads_as_unset`), and startup is not
  blocked. Do this manually as well: write such a name into a test profile's
  `settings.json` and launch. The app opens, and My Info asks for a name.
- **Contacts binding.** Contacts are bound before and independently of node
  start (`contacts_bound_even_if_node_start_fails`).

## Residual risks (state them in the PR; no fix expected in v1)

| # | Risk | Mitigation in place |
|---|------|---------------------|
| R1 | **Removal racing an in-flight known-peer handshake.** If `contacts_remove` lands between the gate's `is_known == true` and `authorize`, `disconnect` finds nothing and the peer gets one session | The window is milliseconds. The next inbound connection prompts. Messaging (DD-04) can re-check `is_known` before accepting content |
| R2 | **Headless stdout event stream** prints self-asserted names (`display_name` in `incoming_request`, `connection_state` and `lan_peers_changed`) | The headless node is test-only, uses throwaway identities and is never shipped. The README already marks it as having no security guarantees |
| R3 | **Homoglyphs** and visually blank non-Cf letters (for example U+3164 HANGUL FILLER) are still accepted | The list shows only the user's own nickname. The NodeId is always visible. The trust badge is structural |
| R4 | **Plaintext `contacts.json`** (nicknames, self-asserted names) | File mode 0600 in a 0700 directory. DD-05 replaces it |
| R5 | **The emoji exception widens the allowed set.** The validator checks the *structure* of ZWJ and tag sequences, not membership in the RGI emoji list or the ISO 3166-2 subdivisions. A peer can therefore use an unlisted, well-formed sequence that renders as several glyphs, or invisible tag letters inside a 🏴 sequence | ZWJ is allowed only between two `Extended_Pictographic` characters, and tags only inside a terminated 🏴 run. Every other Cf character is rejected. Names never carry trust: the badge is structural, the list shows only the user's own nickname, and the NodeId is always visible. Test evidence: `display_name::zwj_outside_emoji_is_rejected`, `display_name::malformed_tag_sequences_are_rejected` |

## Security — what this audit must catch

| Vulnerability | Prevention / fix (if the audit finds it missing) | Check |
|---------------|--------------------------------------------------|-------|
| Name-based authorization | Route every trust decision through `ContactBook::is_known(NodeId)`. Delete any name comparison used as identity | E1, E2; read `accept_incoming` and the gate |
| Bidi / control / zero-width spoofing of nickname vs badge | Keep the Cf set in `validate_display_name`, with the emoji exception limited to structurally valid sequences, `<bdi>` around names, and the fixed-text `TrustBadge` first in the row. Show only the nickname in rows | Audit B rows 2–3. Manual: set a nickname to `Verified by you`; the row must still start with **Unverified** |
| Homoglyph spoofing | Not blocked (R3). Keep the NodeId visible on every row and prompt | Manual: confirm `NodeIdLine` is on every row, prompt and LAN entry |
| Accept prefill adopts the peer's name unnoticed | Backend-computed `suggested_nickname` from the validated name, visibly editable, separate "chosen by them, not verified" context, explicit Accept click, a cleared field blocks Accept, always Unverified | 03-01 Vitest `the nickname field is prefilled…`, `clearing the field disables Accept`, `nothing is sent until Accept is clicked`; `dto::incoming_request_json_shape` |
| Peer data flips trust or creates contacts from inbound traffic | `observe` cannot create entries or change trust. Inbound creation only via `accept_incoming`. Dial creation only after a valid reply, always Unverified | `book::only_set_verified_changes_trust`, `book::observe_unknown_never_creates_a_contact`, `dialed_peer_is_remembered_only_after_a_valid_reply`; E3, E14 |
| XSS via `{@html}` | Use text interpolation only. Remove any new `{@html}` | `html-guard.test.ts`; E10 |
| Oversized input / DoS of the file store | Byte and count caps, `Full`, a length-capped `parse_node_id`, throttled observe, `spawn_blocking` | `store::oversized_file_is_refused`, `store::full_store_…`, `store::observe_skips_write_…`, `contacts_add_rejects_malformed_node_id`; E14 |
| Corrupt file treated as empty (fail-open) | Corrupt → `Err` → prompt. Never write over a corrupt file | `store::corrupt_files_…`, `corrupt_contacts_file_fails_closed_and_prompts`, `migrate::corrupt_legacy_file_…` |
| Validator change bricks startup | Map `InvisibleCharacter` to unset or no name, never to `Unavailable` / `Corrupt` | `settings::stored_name_now_invisible_reads_as_unset`, `migrate::invisible_legacy_name_counts_as_no_name`; manual launch |
| Path traversal via account_id / profile | Use a constant file name under the validated `data_dir`. Account keys must be canonical UUIDs | `store::corrupt_files_…` (UUID cases); `core/tests/profile_isolation.rs` green; `grep -n 'join(' core/src/contacts/store.rs` |
| TOCTOU on the Accept / trust check | Order: validate → prompt → add → answer, with rollback. Disconnect on removal | `accept_rolls_back_new_contact_when_prompt_vanished`, `contacts_remove_drops_live_connection_and_next_inbound_prompts`; R1 documented |
| Removal not reverting trust | Remove the NodeId row, then `node.disconnect` | `removed_contact_is_prompted_again` |
| History deletion defaulting on | `HistoryChoice::default() == Keep`, `Option<bool>` arg, unchecked checkbox reset on open | Audit C removal rows; `RemoveContactDialog.test.ts` |
| Nicknames in logs | Remove the name field from the log call. Extend `contacts_log_guard.rs` if a new macro form slipped through | `contacts_log_guard.rs`; E4, E5 |
| Forged IPC args (malformed NodeId, self id, missing flag, missing nickname) | Typed parsing and validation at the command boundary, with exhaustive error mapping | `contacts_add_rejects_malformed_node_id`, `contacts_add_rejects_self`, `contacts_remove_defaults_to_keeping_history`, `accept_requires_a_valid_nickname`, `accept_maps_every_contact_error` |
| Lost updates (two store instances) | One `FileContactStore` per process, shared via `Arc` | E15 |
| Naming prompt lost to an event race | Recent-outbound buffer in `contactsStore` | `contactsStore.test.ts › connected event that arrives before expectNaming…` |

## Fix-up rules

- Put missing behaviour in the slice that owns it:
  - core rules go in `core::contacts`;
  - gate behaviour in `core::discovery`;
  - validation in `core::identity::display_name`;
  - commands in `contacts_cmd.rs`;
  - UI in the matching view.

  Every fix comes with a test.
- Never weaken a test to make it pass. Never mark an item done on intent alone.
- If a requirement needs a decision the user has not made, stop. List it under
  "Open questions" in the PR instead of guessing.

---

## Docs

### `README.md`

- The workspace table names the `contacts` slice.
- The trust rule matches the code. That covers Accept with a nickname, dial
  auto-add with the default nickname, and the snapshot rule.
- The Contacts section and manual checklist from 03-02 are present, including
  the concrete reconnect-silently item.
- The name rule change (decision D refined: Cf characters rejected except inside well-formed emoji ZWJ/tag sequences) and the Accept prefill (decision E extended) are documented.
- The plaintext interim storage note and the "nothing to delete until
  DD-04/DD-05" note are present.
- The level-3 syntax `respond … accept [NICKNAME]` is documented.
- No references to step files remain.

### `AGENTS.md` (decision F; exact edits)

- **Layout table, `core/` row.** Replace "(currently `identity/` and
  `discovery/`)" with "(currently `identity/`, `discovery/` and `contacts/`)".
- **Layout table, `app/src-tauri/` row.** Replace "command handlers
  (`identity_cmd.rs`, `net_cmd.rs`)" with "command handlers (`identity_cmd.rs`,
  `net_cmd.rs`, `contacts_cmd.rs`); shared test helpers in `test_util.rs`".
- **Notes, test-support bullet.** Replace "(e.g. `identity::testing`,
  `discovery::testing`)" with "(e.g. `identity::testing`, `discovery::testing`,
  `contacts::testing`: `MemoryContactStore`, `FailingContactStore`,
  `RecordingHistoryEraser`, `BookCell`)".
- **Notes, new bullet.** Add: "Contacts are the single trust store
  (`ContactBook` implements `discovery::KnownPeers`); until DD-05 they live in
  plaintext `contacts.json`."
- Change nothing else in AGENTS.md. In particular, leave the security rules as
  they are; the clipboard rule stays per decision A.

### `design_docs/README.md` (exact change)

In `## Status`, replace the first sentence "All five docs are v1 drafts
targeting the desktop build (Linux/macOS/Windows)." with:

> All five docs are v1 drafts targeting the desktop build (Linux/macOS/Windows);
> DD-03 is implemented, with the deviations listed in its Status line.

### `design_docs/03-contacts-and-trust.md`

Set the Status line exactly to:

> **Status:** Implemented (v1). Documented deviations / user decisions:
> interim plaintext contact store until DD-05; history-deletion seam with a
> no-op eraser until DD-04/DD-05; NodeId copy by selection only (clipboard
> rule, user-accepted); contact list shows only the local nickname (self-asserted
> name kept for a future contact-info screen); DD-01 name validation tightened
> to reject invisible format characters except inside well-formed emoji ZWJ/tag
> sequences; dialed peers auto-added as unverified with a default nickname, and
> the Accept and naming prompts prefilled with that default; "Open chat"
> deferred to DD-04.

Tick the DoD boxes that are met. Annotate the encrypted-DB box with deviation S
and the open-chat item with deviation O. Do not rewrite the design.

### DD-05 follow-ups (PR description only; do not edit DD-05)

- Replace `FileContactStore` with the `contacts` table, migrating
  `contacts.json`, and add an in-memory index for `is_known`.
- Replace `NoMessageStore` with the real `HistoryEraser`.
- Extend the log redaction guard to the contacts fields.

### DD-04 follow-ups (PR description only)

- Add the "Open chat" row action.
- Re-check `is_known` before accepting content (R1).

---

## Close-out: delete the step files

The folder `design_docs/steps/03-contacts-and-trust/` was committed on the
03-01 branch, which is part of this stack, so the files are tracked here. After every audit row is green and the
docs are updated:

```sh
cd "$(git rev-parse --show-toplevel)"
for f in design_docs/steps/03-contacts-and-trust/*.md; do
  git rm -q "$f" 2>/dev/null || rm -f "$f"     # fallback if a file was never committed
done
rmdir design_docs/steps/03-contacts-and-trust 2>/dev/null || true
test ! -e design_docs/steps/03-contacts-and-trust && echo "step folder removed"
```

If the folder still exists, list what remains (`ls -A`) and remove it. Nothing
may be left behind, and no empty folder may be committed. Do not touch other
folders under `design_docs/steps/`.

---

## Guardrails (mandatory)

- **Never write credentials, keys, tokens or secrets in code, tests, fixtures or
  config. All credentials must be provided at runtime (OS keychain /
  environment).**
- No `SecretKeyBytes` or `DbKey` under `app/`.
- No test backends in shipped `app/` code: no `FileKeychain`, no headless crate,
  and no contact or known-peer doubles.
- `core` keeps `#![forbid(unsafe_code)]`.
- Never log nicknames, self-asserted names, message plaintext, keys or secrets.
  NodeIds are fine.
- Never bypass `.githooks/pre-commit` with `--no-verify`.
- Only files with real content: no empty folders and no stub modules. The
  close-out removes the step folder entirely.
- Any wire-format change bumps `PROTOCOL_VERSION`. DD-03 is expected to have
  none (E16).
- Clipboard access stays confined to `app/src/views/MyInfo.svelte`.

## Tests

Do not add new tests unless the audit finds a gap. Each gap fix comes with a
test named after the DD-03 claim it proves. The full suite (Rust and Vitest)
must pass. Keychain-dependent tests use the in-memory doubles.

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

Commit through `.githooks/pre-commit`; never use `--no-verify`. Run
`docker/smoke.sh` if Docker is available and record the result.

## Definition of done

- [ ] Audits A–E are complete, with evidence pasted in the PR. Every gap found
      is fixed with a test.
- [ ] Every DD-03 Goal, Security bullet, Testing bullet and DoD checkbox is
      verified, or recorded as user decision / deviation A, B, C, D, E, S, H, O
      or F.
- [ ] Residual risks R1–R5 are listed in the PR.
- [ ] The manual GUI checklist ran on two profiles, with the results in the PR.
      `docker/smoke.sh` ran if Docker is available.
- [ ] README, AGENTS.md (the exact edits above), `design_docs/README.md` (the
      exact sentence) and the DD-03 Status line and DoD boxes are updated.
- [ ] Every file in `design_docs/steps/03-contacts-and-trust/` is deleted, and
      the folder no longer exists. Other step folders are untouched.
- [ ] All quality gates pass through the pre-commit hook, without
      `--no-verify`.
- [ ] PR follows `.github/PULL_REQUEST_TEMPLATE.md`:
  - lists the crates touched by fixes and the docs;
  - **Wire format: no change; `PROTOCOL_VERSION` not bumped**;
  - the verification section holds the audit evidence;
  - lists the DD-04 and DD-05 follow-ups and any open questions.
