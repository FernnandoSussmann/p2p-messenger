# Step 01-05 — First-run wizard + "My Info" tab: complete end-to-end feature + pre-commit gate

**Design doc:** [DD-01 — Identity, Onboarding & My Info](../01-identity-onboarding-and-my-info.md)
**Crate(s):** `app` (Svelte frontend + a little `app/src-tauri` glue)

## Dependency & ordering

- **Type: SEQUENTIAL overall (final integration), with TWO PARALLEL UI units in the middle.**
- **Depends on:** 01-04 (the registered commands + typed bindings + minimal shell).
- **Blocks:** nothing (this is the last step for DD-01). This step must leave a **complete, working,
  end-to-end feature**: fresh install → first-run wizard → main screen; "My Info" tab reachable from the
  main screen; every view connected; the secret-leak guard wired into the mandatory local pre-commit hook.

### Internal structure

1. **Sequential prelude** — app routing/state skeleton: a top-level view machine driven by `identity_status`
   (`Loading → ErrorBlocking | FirstRunWizard | MainScreen`), a main-screen layout with a tab bar exposing a
   **"My Info"** tab, and shared types/stores. This creates the files the two parallel units plug into.
2. **TWO PARALLEL units** (share no files; no ordering between them — worktree + sub-agent each):

   | Unit | Owns (only) | Builds |
   |------|-------------|--------|
   | Parallel Unit A — First-run wizard | `app/src/views/FirstRunWizard.svelte` (+ its own component files) | generate → name → land flow |
   | Parallel Unit B — My Info tab | `app/src/views/MyInfo.svelte` (+ its own component files) | name edit, NodeId copy, ticket copy, QR |

   Each consumes the typed command bindings from 01-04 and the routing contract from the prelude; neither
   imports the other. The prelude's router file is the only shared file and is finished before the units
   start.
3. **Sequential finish** — wire both views into the router, add the pre-commit hook, write the end-to-end
   test, run all gates.

## Goal of this step

Deliver the full DD-01 user experience and close every remaining DD "Definition of done" bullet: a fresh
install lands in the first-run wizard, provisions, forces a valid display name, and arrives on the main
screen; the always-reachable "My Info" tab shows the editable display name, full copyable NodeId, copyable
live ticket, and a QR rendering of that ticket; no secret ever reaches the clipboard, the webview, or a log;
and the automated secret-/plaintext-leak guard is enforced on **every local commit** via a `pre-commit` git
hook (there is no CI — the local hook is the gate).

## Responsibilities

### Sequential prelude — routing & state (`app/src/App.svelte`, `app/src/lib/router.ts`, stores)
- Replace 01-04's bare `App.svelte` with a view machine: on mount call `identity_status` →
  - `Error`/`KeychainAccessDenied` → **ErrorBlocking** view (recoverable message + retry; **never**
    auto-provision). Fail-closed.
  - `Absent` → **FirstRunWizard**.
  - `Present` → **MainScreen** (which hosts the tab bar incl. **My Info**).
- **Subsequent launches land directly on the main screen with no prompt** (status `Present` skips the wizard).

### Parallel Unit A — First-run wizard (`app/src/views/FirstRunWizard.svelte`)
- Step 1 "Generate": a single action calls `identity_provision`; on success advance; on `AlreadyProvisioned`
  route to main screen; on error show the recoverable/fail-closed state. No key material is shown.
- Step 2 "Name": prompt for display name; on submit call `identity_set_display_name`. On
  `InvalidDisplayName { kind }` show a specific inline message and **block progress** — the user cannot
  proceed with empty/whitespace/over-length/control/newline/bidi input (validation is authoritative in
  `core`; the UI may mirror it for instant feedback but must not rely on client-side checks alone).
- Step 3 "Land": navigate to the main screen. After this, identity is a precondition every other slice
  assumes.

### Parallel Unit B — "My Info" tab (`app/src/views/MyInfo.svelte`)
- On show, call `identity_my_info` (regenerate the ticket + QR **live** each time the tab is opened — per the
  DD, relay/direct addrs can change; DD-02 will make the ticket richer, but the "regenerate live" contract is
  honored now).
- **Display name** — rendered editable in place; an edit re-calls `identity_set_display_name` (re-validates,
  persists) with the same inline error handling as the wizard.
- **NodeId** — shown in full with a **copy button** (copies the NodeId string only).
- **Connection ticket** — shown with **copy-to-clipboard as the primary action** (copies the ticket string
  only), via `tauri-plugin-clipboard-manager`.
- **QR code** — render the `qr_svg` string returned by `identity_my_info` (generated in `core`, display
  only; no scanning).
- The clipboard write is wired **only** to the ticket/NodeId strings from the DTO — never to any other value.

### Sequential finish — glue, hook, e2e
- Register both views in the router; confirm the My Info tab is reachable from the main screen and the wizard
  is reachable on a fresh install.
- **Pre-commit hook:** add a tracked hook script (e.g. `scripts/pre-commit` or `.githooks/pre-commit`) plus a
  one-time install note / `git config core.hooksPath .githooks` instruction in the README, that runs the
  mandatory local gate and **blocks the commit on any failure**:
  `cargo fmt --all --check` → `cargo clippy --all-targets --all-features -- -D warnings` →
  `cargo test --workspace` (which includes the secret-leak guards from 01-01/01-02/01-03/01-04) →
  the secret grep gates → the frontend typecheck/build. Document in the README (under "Build & test") that
  this local hook is the test gate since there is no CI.
- Update `README.md` workspace table: mark `core` and `app` as **present**.

## Tests

- **Frontend component/logic tests** (Svelte testing-library or equivalent): wizard blocks on invalid names
  and advances on valid ones; My Info renders name/NodeId/ticket/QR and the copy actions invoke the clipboard
  with the ticket/NodeId string (assert the argument passed to the mocked clipboard is exactly the ticket,
  never any other field).
- **End-to-end smoke** (the DD's user-visible flow, automated as far as the harness allows — Tauri
  WebDriver/`tauri-driver` if available, otherwise a scripted invoke-level test): fresh state (temp app-data
  dir + mock/empty keychain) → `identity_status` = `Absent` → `identity_provision` → `identity_set_display_name`
  → `identity_my_info` returns name + NodeId + ticket + QR; a second `identity_status` = `Present` (no second
  prompt). If a full GUI driver is unavailable in this environment, state that and implement the e2e at the
  command-invoke level with the real `core` + fakes, and leave a documented manual GUI checklist.
- **Clipboard leak guard (automated):** a test asserting the clipboard write path is reachable **only** from
  the ticket/NodeId copy handlers (grep/AST check that the clipboard API is called with a `MyInfoDto.ticket`
  / `.node_id` binding and nowhere else): `grep -RnE 'writeText|clipboard' app/src` shows calls only in the
  My Info copy handlers.
- **Secret-leak guard wired into the pre-commit hook:** a test that asserts the hook script exists, is
  executable, and its command list includes `cargo test --workspace` and the secret grep gates; and a manual
  verification note that a staged commit containing a planted secret-looking string in a Tauri return type is
  rejected by the hook.
- Re-run all prior gates (the whole `cargo test --workspace` leak-guard suite from 01-01→01-04 must still be
  green).

## Guardrails

- **Secret confidentiality:** the webview still only ever receives public DTOs (01-04); no change widens the
  boundary. No secret is logged by the frontend or the small glue added here.
- **No secret on clipboard:** the clipboard is wired **exclusively** to `MyInfoDto.ticket` / `.node_id`;
  grep-enforced. The identity private key and DB key can never reach the clipboard because they never reach
  the webview.
- **No secret on disk outside keychain:** unchanged; the frontend writes nothing sensitive; only the
  `account_id` marker (non-secret) exists outside the keychain.
- **Partial-write safety + rollback:** owned by `core` (01-03); the wizard treats provisioning as atomic —
  either `Present` afterward or it shows an error, never a half-state it tries to patch up.
- **No silent re-provision:** the ErrorBlocking view never offers "just make a new identity"; it offers
  recovery/retry only. `Absent` is the only state that reaches the wizard.
- **Display-name validation:** authoritative in `core`; the wizard and My Info both block on
  `InvalidDisplayName` and cannot submit empty/whitespace/over-length/control/newline/bidi values.
- **Fail-closed on keychain access denial:** `KeychainAccessDenied` → ErrorBlocking view, user stuck on a
  blocking screen rather than proceeding without secrets.

## Security (prevent + verify)

- **Secret leakage via clipboard.** *Prevent:* clipboard bound only to ticket/NodeId. *Verify:*
  `grep -RnE 'writeText|clipboard' app/src` shows only the two copy handlers + the clipboard-argument test.
- **Secret leakage via the webview/logs.** *Prevent:* public DTOs only; no secret crosses the boundary.
  *Verify:* the 01-04 boundary leak-guard + `grep -RnE 'SecretKeyBytes|DbKey' app` empty (Rust side) and no
  secret field in the TS bindings.
- **Display-name injection / homoglyph / bidi spoofing in the UI.** *Prevent:* `core` validator is
  authoritative; render names as text (never as HTML) so a crafted name cannot inject markup. *Verify:* a
  component test that a name containing `<script>`/HTML is rendered as inert text, plus the `core` validation
  tests (01-01/01-03).
- **Silent re-provision orphaning the DB / changing address without consent.** *Prevent:* wizard only runs on
  `Absent`; errors route to recovery. *Verify:* e2e asserting an `Error` status never reaches the wizard.
- **Marker tampering surfaced, not silently fixed.** *Prevent:* corrupt marker → `core` `Error` →
  ErrorBlocking. *Verify:* e2e/mapping path for the error state.
- **Un-gated commits shipping a leak.** *Prevent:* pre-commit hook runs the full leak-guard suite + grep
  gates and blocks on failure. *Verify:* the hook-existence test + the planted-secret manual rejection check.

## Credentials / secrets rule

NEVER hardcode credentials, keys, passphrases, or secrets in source (Svelte/TS/Rust/config/hook scripts). All
key material is generated/retrieved at runtime by `core` and lives only in the OS keychain; the webview only
ever handles public NodeId/display-name/ticket values. No test (frontend, e2e, or hook) writes a real secret
to disk or commits one — e2e uses a temp app-data dir and the in-memory/mock keychain.

## Quality gates (warnings are errors) — these ARE the pre-commit gate

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo build`
- Frontend: `npm run build` + `svelte-check`/`tsc --noEmit` clean.
- `grep -RnE 'SecretKeyBytes|DbKey' app` returns nothing; `grep -RnE 'writeText|clipboard' app/src` shows only
  the My Info copy handlers.
- The `pre-commit` hook runs the above and blocks the commit on any failure.

## Definition of Done (verifiable checklist)

- [ ] Fresh install shows the first-run wizard: generate → store (via `core`) → **prompt + validate display
      name (cannot proceed if invalid)** → land on the main screen. *(DD DoD bullets 1, 4; DD first-run flow.)*
- [ ] Subsequent launches detect `Present` and land on the main screen with **no prompt**. *(DD DoD bullet 2.)*
- [ ] Missing/unreadable secrets (incl. keychain access denied, corrupt marker) show a recoverable
      ErrorBlocking screen — **never a silent re-provision**. *(DD DoD bullet 3.)*
- [ ] Display name is editable later in "My Info" and re-validated + persisted. *(DD DoD bullet 4.)*
- [ ] "My Info" tab is reachable from the main screen and shows: editable display name, full **copyable**
      NodeId, **copyable** live ticket, and a **QR rendering** of the ticket (display only, regenerated live).
      *(DD DoD bullet 5.)*
- [ ] No secret ever crosses the Tauri boundary, hits the clipboard, or is logged (clipboard bound only to
      ticket/NodeId; boundary + clipboard leak guards pass). *(DD DoD bullet 6.)*
- [ ] All DD tests pass and the **secret-leak guard is part of the mandatory local pre-commit test gate**
      (hook installed, blocks on failure; README documents it as the gate since there is no CI). *(DD DoD bullet 7.)*
- [ ] Feature is complete and end-to-end: frontend ↔ backend ↔ `core` wired; every view reachable; `core` and
      `app` marked present in the README; no unconnected pieces. *(DD DoD bullet 8; task constraint 4.)*
- [ ] `fmt`/`clippy -D warnings`/`test --workspace`/`cargo build` + frontend build/typecheck green; no
      hardcoded secrets.
