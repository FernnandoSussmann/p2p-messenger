# Step 01-04 — `app` crate: Tauri v2 shell + the four identity commands (backend wired, minimal UI)

**Design doc:** [DD-01 — Identity, Onboarding & My Info](../01-identity-onboarding-and-my-info.md)
**Crate(s):** `app/src-tauri` (new — joins the workspace in this step)

## Dependency & ordering

- **Type: SEQUENTIAL.** Builds the Tauri surface on top of the finished `core::identity` service and wires
  the OS-backed boundaries; one crate, ordered setup — not parallelizable.
- **Depends on:** 01-03 (the complete `IdentityService` and its boundaries).
- **Blocks:** 01-05 (the full first-run wizard + My Info UI render against these commands).

## Goal of this step

Create the Tauri v2 `app` crate, construct a real `IdentityService` backed by the OS keychain
(`OsKeychain`), the minimal settings store, and the NodeId ticket provider, resolve the real OS app-data dir
for the marker, and expose the four Tauri commands. The webview receives a **minimal but functional** shell
(the app launches, calls `identity_status`, can provision, and shows the NodeId / a status line) so the step
is independently usable and compiling. The full wizard + My Info polish is 01-05. Critically, this step is
where the **secret-never-crosses-the-Tauri-boundary** rule is proven by the command return types and the
automated boundary leak guard.

## Files (layout)

- `app/src-tauri/Cargo.toml` (new) — the workspace member; package `app` (bin), deps:
  `tauri = { version = "2", features = [...] }`, the `core` crate imported **under a rename to avoid shadowing
  the Rust std `core` crate** — `messenger_core = { package = "core", path = "../../core" }` (referred to as
  `messenger_core::…` in this crate; see 01-01's crate-name note) — `serde`/`serde_json`,
  `directories` (or use Tauri's own `app.path().app_data_dir()`), and the clipboard plugin crate
  `tauri-plugin-clipboard-manager = "2"` (used in 01-05; can be added here). `app/src-tauri/build.rs`
  (`tauri_build::build()`), `app/src-tauri/tauri.conf.json`, `app/src-tauri/capabilities/*.json`.
- `app/src-tauri/src/main.rs` + `app/src-tauri/src/lib.rs` — builder, `manage` state, `invoke_handler`.
- `app/src-tauri/src/identity_cmd.rs` (new) — the four `#[tauri::command]`s + their serde DTOs + typed error.
- `app/` frontend skeleton (Svelte + Vite + TS): `package.json`, `vite.config.ts`, `svelte.config.js`,
  `tsconfig.json`, `index.html`, `src/main.ts`, `src/App.svelte` (minimal), and the generated/bindings glue.
- Root `Cargo.toml` (edit) — `members = ["protocol", "core", "app/src-tauri"]`.

**Confirm against Context7 before finalizing:** the Tauri v2 `Builder`/`generate_handler!`/`manage` shapes,
the correct feature flags, `app.path().app_data_dir()` for resolving the marker directory, and the
`tauri-plugin-clipboard-manager` v2 registration — these are the APIs most likely to have drifted.

## Responsibilities

### State & wiring (`lib.rs`)
- On setup, resolve the app-data dir (`app.path().app_data_dir()?`, or `directories::ProjectDirs::from`),
  construct `OsKeychain` with the `<app-id>` service, the minimal settings store (opened with the DB key when
  present), the `NodeIdTicketProvider`, and build the `IdentityService`. Wrap it in whatever interior
  mutability it needs and `manage` it as Tauri state. Commands retrieve it via the `State` guard.
- Register `tauri::generate_handler![identity_status, identity_provision, identity_set_display_name,
  identity_my_info]`.

### Commands (`identity_cmd.rs`) — shapes, not literal code

| Command | Input | Returns (DTO) |
|---------|-------|---------------|
| `identity_status` | — | `StatusDto`: `Present { account_id, node_id }` \| `Absent` \| `Error { reason }` |
| `identity_provision` | — | `ProvisionDto { node_id, account_id }` \| typed error |
| `identity_set_display_name` | `{ name: String }` | `()` \| `ValidationErrorDto` |
| `identity_my_info` | — | `MyInfoDto { display_name: Option<String>, node_id, ticket, qr_svg }` |

- Every DTO contains **only** public data: `node_id` (string), `account_id` (UUID string), display name,
  ticket string, QR SVG string. **No DTO has a field that could hold the identity private key or DB key** —
  this is enforced structurally (the DTOs simply have no such field) and verified by the leak guard below.
- A single `IdentityCmdError` enum implementing `serde::Serialize` maps `core` errors to webview-friendly,
  non-secret messages, preserving the important distinctions the UI needs: `AlreadyProvisioned`,
  `RecoverableErrorState { reason }` (marker present + secret missing/denied → the blocking error screen),
  `KeychainAccessDenied` (fail-closed screen), `InvalidDisplayName { kind }`, `Internal`. Error messages must
  not embed secret material.
- Commands are thin: validate/convert input, call the `IdentityService`, map the result/error to a DTO. No
  business logic here (it lives in `core`).

### Minimal frontend (`App.svelte`)
- On mount, call `identity_status`. If `Error`/access-denied → show a blocking message (no provisioning). If
  `Absent` → show a single "Create identity" button that calls `identity_provision`. If `Present` → show the
  NodeId and the display name (from `identity_my_info`). This is intentionally bare — enough to demonstrate
  the backend end-to-end and keep the app **usable and compiling**; the real wizard and My Info tab are 01-05.
- Generate/commit the typed TS bindings for the commands (or hand-write a thin typed `invoke` wrapper) so
  01-05 builds against stable signatures.

## Tests

- **Rust unit — command/error mapping** (`#[cfg(test)]` in `identity_cmd.rs`, against an `IdentityService`
  built with the in-memory fakes from 01-02/01-03): `status`/`provision`/`set_display_name`/`my_info` map
  core results and each error class to the correct DTO/`IdentityCmdError` variant; `AlreadyProvisioned`,
  `RecoverableErrorState`, `KeychainAccessDenied`, and `InvalidDisplayName` all round-trip.
- **Secret-leak guard (Tauri-boundary slice — automated, the DD's mandated guard):**
  - A test that `serde_json::to_value` each DTO produced from a provisioned identity and asserts the JSON
    contains the NodeId/account_id but **not** the key bytes (seed via the fake keygen canary from 01-03 and
    assert its absence).
  - A **static grep gate** asserting no command return type names a secret type: `grep -RnE
    'SecretKeyBytes|DbKey' app/src-tauri/src` returns **nothing** (the secret newtypes must never appear in
    the app crate). This is the concrete "no secret in any Tauri command return type" check.
  - A grep gate over logging: `grep -RnE 'println!|dbg!|tracing::(info|debug|trace|warn|error)!' app/src-tauri/src`
    is reviewed so no secret is formatted (and the DTOs contain none anyway).
- **Frontend typecheck/build** runs clean (see Quality gates) — proves the minimal UI compiles against the
  bindings.

## Guardrails

- **Secret confidentiality / Tauri boundary:** DTOs expose only public data; the `SecretKeyBytes`/`DbKey`
  types from `core` **never** appear in the app crate (grep-enforced). Nothing is logged with a secret.
- **No secret on clipboard:** no clipboard wiring yet in the minimal UI; when added in 01-05 it is bound only
  to ticket/NodeId strings. (Stated here so the rule is present from the moment the webview exists.)
- **No secret on disk outside the keychain:** the marker file (unencrypted) holds only the `account_id`; the
  app writes no secret anywhere but the OS keychain (via `core`).
- **Partial-write safety + rollback:** provisioning atomicity and marker-last ordering are owned by `core`
  (01-03); the command is a thin pass-through and adds no new write path.
- **No silent re-provision:** `identity_status` surfaces `Error`/access-denied as a blocking state; the
  minimal UI never auto-provisions out of an error state.
- **Display-name validation:** `identity_set_display_name` returns `InvalidDisplayName { kind }` from the
  `core` validator; the UI cannot proceed on invalid input (enforced fully in 01-05's wizard).
- **Fail-closed on keychain access denial:** `KeychainAccessDenied` → blocking screen, never proceed without
  secrets.

## Security (prevent + verify)

- **Secret leakage via Tauri return types.** *Prevent:* DTOs carry no secret field; secret newtypes barred
  from the app crate. *Verify:* `grep -RnE 'SecretKeyBytes|DbKey' app/src-tauri/src` empty + the DTO-JSON
  canary test.
- **Secret leakage via logs.** *Prevent:* no secret reaches the app layer. *Verify:* logging grep review.
- **Keychain prompt bypass.** *Prevent:* `KeychainAccessDenied` blocking state. *Verify:* command-mapping
  test that an injected access-denied error maps to `KeychainAccessDenied`, not a success/empty DTO.
- **Silent re-provision via the command surface.** *Prevent:* `identity_provision` delegates to `core`, which
  refuses unless `Absent`. *Verify:* command-mapping test that provision on an already-present service maps
  to `AlreadyProvisioned`.
- **Marker tampering surfaced to the user.** *Prevent:* corrupt marker → `core` `Error` → `RecoverableErrorState`.
  *Verify:* mapping test for the error-state case.

## Credentials / secrets rule

NEVER hardcode credentials, keys, passphrases, or secrets in source (Rust, TS, or `tauri.conf.json`). The
`<app-id>` keychain service string is an identifier, not a secret. All key material is generated/retrieved at
runtime by `core` and lives only in the OS keychain. Command-mapping tests use the in-memory fakes; **no test
writes a real secret or commits one**, and no secret is ever embedded in a DTO, config, or binding.

## Quality gates (warnings are errors)

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo build` (the whole workspace, incl. `app/src-tauri`, builds)
- Frontend: `npm ci && npm run build` (or the project's package manager) and the Svelte/TS typecheck
  (`svelte-check` / `tsc --noEmit`) pass with no errors.
- `grep -RnE 'SecretKeyBytes|DbKey' app/src-tauri/src` returns nothing.

## Definition of Done (verifiable checklist)

- [x] `app/src-tauri` joins the workspace with real content; the app builds and launches. *(DD DoD bullet 8 — app crate real content.)*
- [x] The four commands (`identity_status`, `identity_provision`, `identity_set_display_name`,
      `identity_my_info`) are registered and return the documented DTOs. *(DD API sketch.)*
- [x] Minimal UI: fresh install can provision and then shows its NodeId; error/access-denied states block
      instead of auto-provisioning. *(DD DoD bullets 1–3 surfaced end-to-end at a basic level.)*
- [x] No DTO/return type can carry a secret; `SecretKeyBytes`/`DbKey` do not appear in `app/src-tauri`;
      boundary leak-guard test + grep gate pass. *(DD DoD bullet 6; DD Testing "secret-leak guard" — Tauri slice.)*
- [x] `identity_set_display_name` surfaces validation errors; access-denied maps to a fail-closed error. *(DD DoD bullets 3, 4.)*
- [x] `fmt`/`clippy -D warnings`/`test --workspace`/`cargo build` + frontend build & typecheck all green; no
      hardcoded secrets.
