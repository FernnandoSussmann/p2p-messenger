# AGENTS.md

Guidance for AI coding agents working in this repo.

## Project

A decentralized, end-to-end-encrypted peer-to-peer messenger in Rust. No central
server: each user's keypair is their identity, and peers connect directly over
[iroh](https://www.iroh.computer/) (addressed by public key, relay fallback
carries ciphertext only). Desktop (Linux/macOS/Windows) is the first target for this app, it will expand to mobile and web interface in the future.
UI is Tauri v2 + Svelte 5. License: GPL-3.0-only.

## Layout

Cargo workspace built from vertical feature slices.

| Path             | Role                                                                          |
| ---------------- | ----------------------------------------------------------------------------- |
| `protocol/`      | Shared serde wire types (`Envelope`, `Payload`, …), `PROTOCOL_VERSION`, `ALPN`. Transport/UI-agnostic. |
| `core/`          | UI-agnostic logic, one module per feature slice (currently `identity/`, `discovery/`, `contacts/`, `messaging/` and `storage/`). Knows nothing about Tauri. |
| `app/src-tauri/` | Thin Tauri shell: command handlers (`identity_cmd.rs`, `net_cmd.rs`, `contacts_cmd.rs`, `chat_cmd.rs`, `storage_cmd.rs`); shared test helpers in `test_util.rs` over `core`. |
| `headless/`      | **Test-only** headless node (`messenger-headless`) with a file-backed keyring (`FileKeychain`, unencrypted, throwaway identities) and a Unix-socket control surface; simulates devices at Level 3. Unix only. Never shipped; `app/` must never depend on or name it (pre-commit grep gate). |
| `docker/`        | **Test-only** `headless.Dockerfile`, `compose.yaml` and `smoke.sh` (two isolated nodes; `docker/smoke.sh` is not part of the hook). No stability or security guarantees. |
| `app/src/`       | Svelte frontend (`views/`, `lib/`), Vitest tests alongside (`*.test.ts`).     |
| `.githooks/`     | Tracked pre-commit hook: the mandatory local test gate (there is no CI).      |

Notes:
- The package name `core` shadows std `core`; dependents import it with `package = "core"`.
- Shared dependency versions live in the root `Cargo.toml` `[workspace.dependencies]`; member crates use `dep.workspace = true`.
- `core` has a `test-support` feature exposing test doubles (e.g. `identity::testing`, `discovery::testing`, `contacts::testing`: `MemoryContactStore`, `FailingContactStore`, `RecordingHistoryEraser`, `BookCell`; `identity::MemorySettingsStore` (always available); `messaging::testing`: `MemoryMessageStore`, `FailingMessageStore`, `ChatPeer`, `Faults`, `send_raw_frame`). Discovery exposes authorized channels to other slices only through `ChannelHandler`; messaging never dials.
- All persisted user data (settings, contacts, messages) lives in the SQLCipher `messenger.db` through `core::storage` (keyed open, typed `StorageError`, forward-only migrations; failures are latched, never reset or recreate the file). Never write it anywhere else. New tables are new migrations in `storage/schema.rs`, with SQL owned by the slice and `account_id` on every row. The old plaintext `settings.json`, `contacts.json` and `known_peers.json` are only read once by `storage::legacy` and then removed.
- Contacts are the single trust store (`ContactBook` implements `discovery::KnownPeers`).

## Commands

```sh
npm start                        # install frontend deps + run the desktop app (tauri dev)
cargo test --workspace           # all Rust tests
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cd app && npm ci && npm test && npm run check && npm run build   # frontend
docker/smoke.sh                  # Level-3 two-container smoke (needs Docker; not in the hook)
```

Do not use plain `cargo run` for the app: it shows "connection refused" because
nothing serves the dev frontend. Use `npm start`.

Install the hook once per clone: `git config core.hooksPath .githooks`. It runs
fmt, clippy (warnings are errors), workspace tests, `cargo build`, the secret
gates below, and the frontend check/test/build. Never bypass it (`--no-verify`);
fix the failure instead.

## Conventions

- **Only files with real content.** No empty folders, stub modules, or placeholder files. Add a crate or module only when it carries working code.
- **Vertical slices.** New functionality goes in a `core` feature slice with its own types and invariants; slices expose a curated API via re-exports in `mod.rs`. Keep slices testable without the UI (tests with in-process nodes/fakes).
- **`core` forbids unsafe** (`#![forbid(unsafe_code)]`).
- **Wire format:** serde structs serialized with `postcard`; every frame carries a protocol `version`. Any breaking wire change must bump `PROTOCOL_VERSION`.
- **Data is namespaced by `account_id`** (random v4 UUID, unrelated to any key or NodeId) to leave room for multiple identities.
- Workspace Rust edition 2021, MSRV 1.95. Follow `rustfmt` defaults.
- Frontend: Svelte 5 + TypeScript, built with Vite; tests with Vitest + Testing Library.

- **Profile override:** `--profile <name>` / `MESSENGER_PROFILE` isolates the data dir and keychain namespace so two instances run on one machine. It is a dev/test affordance, fails closed on an invalid name, and must never weaken or skip any control (trust prompt, presence, keychain). `MESSENGER_RELAY_URL` sets the stored relay at launch.

## Security rules (enforced; do not weaken)

- Private keys and the SQLCipher key live in the OS keychain (`keyring`). Test-only backends (`FileKeychain`, the headless crate) never reach `app/` either (same grep gate). Secret types (`SecretKeyBytes`, `DbKey`) are zeroized and must **never** cross into `app/` — the pre-commit hook greps for these names under `app/`, so do not reference them there, even in types or comments.
- Clipboard access (`writeText` / `clipboard`) is permitted **only** in `app/src/views/MyInfo.svelte` (and tests).
- Logs must never contain message plaintext, private keys, or secrets. NodeIds and addresses are fine.
- Everything on the wire is encrypted; relays only ever see ciphertext.
- Logging goes through `core::logging` helpers. The redaction guard (`core/tests/log_redaction_guard.rs`) is enforced; extend its allowlist only with reviewed, non-sensitive field names.

## Testing

- Rust unit and integration tests: `core/tests/`, plus `#[cfg(test)]` modules; app-shell flow tests live in `app/src-tauri/` (e.g. `first_run_flow_end_to_end`, `ticket_connect_accept_flow_end_to_end`, `chat_flow_end_to_end`, `tests/pre_commit_hook.rs`). Level-1 two-node tests (real iroh endpoints on loopback, plus a relay-only canary capture) live in `core/tests/discovery_*.rs`; the Level-1 messaging engine tests live in `core/tests/messaging_*.rs`.
- Frontend component tests: `app/src/**/*.test.ts`.
- No Tauri WebDriver is available here, so GUI flows are covered at command-invoke and component level, plus the manual checklist in `README.md`. If you change the pre-commit hook, verify it by staging a file under `app/` containing a secret-looking identifier and confirming the commit is rejected.
- Keychain-dependent tests should use the in-memory test doubles, not the real OS keychain.

## Pull requests

Follow `.github/PULL_REQUEST_TEMPLATE.md`: name affected crates/slices, state explicitly whether the wire format changed (and `PROTOCOL_VERSION` bumped), list how it was verified, and update `README.md` if behavior or structure changed.
