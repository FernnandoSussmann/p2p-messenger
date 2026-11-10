# Step 02-03: Level-3 harness (headless test node, container keyring, Docker compose)

**Design doc:** [DD-02: Transport, Discovery & Connection](../02-transport-discovery-and-connection.md)
**Order:** 3 of 4. Requires **02-01** and **02-02** merged. It needs `Node`, presence, LAN, DTOs and `testing::spawn_test_node`. Must be finished before 02-04.
**Branch suggestion:** `step-02-03-headless-docker-harness`

## Goal

Provide DD-02's **Level 3** two-device simulation:

- **Headless node binary**, test-only. It runs the same `core::discovery` stack with no webview and is driven by a scriptable control surface.
- **Container keyring backend.** It is file-backed, lives **inside the headless crate**, and is the headless node's only keyring. It can never reach the desktop app.
- **Docker image + compose setup**: two (or N) isolated nodes on one Docker network. They discover each other over mDNS and connect by ticket. In DD-02 terms they complete the Hello round-trip and reach "connected". `docker compose down -v` leaves no residue.
- **Harness-parity tests**: the same trust-gate/presence scenario code runs against Level-1 in-process nodes and against real headless processes.

Scope reminder from DD-02: the headless node and its control surface exist **only to simulate devices for testing**. They are not shipped, not exposed in the GUI, and carry no stability or security guarantees. Promoting them to a daemon mode is out of scope.

## Current code you build on (after 02-02)

- `core::discovery::{Node, NodeConfig, NodeDeps, RelayChoice, LanMode, LiveTicketProvider, FileKnownPeers}`, the string DTOs in `core::discovery::dto` (`IncomingRequestDto`, `ConnectionStateDto`, `LanPeerDto`, `PresenceDto`, `MyTicketDto`), and `testing::{spawn_test_node, MemoryLanBus, ...}`.
- `core::identity::{IdentityService, KeychainStore, KeychainError, AccountId, SecretKeyBytes, DbKey, FileSettingsStore, validate_display_name, ...}`. These are all public. `SecretKeyBytes::from_bytes` / `DbKey::from_bytes` / `as_bytes` are `pub` (`core/src/identity/secret.rs`), and `KeychainStore` has six methods (store/load/delete × identity/db key). Re-check this when you start; if any of them is not public, stop and report rather than widening core's API casually.
- The pre-commit hook `.githooks/pre-commit` and its self-test `app/src-tauri/tests/pre_commit_hook.rs`, which asserts the hook contains every required gate string.

## Why the keyring lives in `headless/` (not behind a core feature)

Cargo resolver 2 unifies features across a workspace build. If `core` had a `container-keyring` feature enabled by `headless`, a workspace `cargo build` would compile that code into the `core` that `app` links. So `FileKeychain` is written in the `headless` crate against core's public `KeychainStore` trait. `core` gets **no** new feature, and `app` does not depend on `headless`. The grep gate and `no_test_backends.rs` (integration) are extra guards.

## Interface contract (agree before splitting into tracks)

**Container keyring (`headless/src/file_keychain.rs`):**

```rust
/// CONTAINER/TEST ONLY. Secrets rest unencrypted on the container filesystem.
/// Never use for a real identity. The desktop app always uses core's OsKeychain.
pub struct FileKeychain { dir: PathBuf }
impl FileKeychain { pub fn open(dir: impl Into<PathBuf>) -> Result<Self, KeychainError>; } // creates dir 0700
impl KeychainStore for FileKeychain { .. } // "<account_id>.identity-key" / "<account_id>.db-key", 0600, exactly 32 bytes
```

**Headless CLI (crate `messenger-headless`, workspace member `headless/`, `publish = false`, lib + bin):**

```text
messenger-headless run  --data-dir <DIR> --keyring file --name <DISPLAY NAME>
                        [--relay default|disabled|<URL>] [--lan mdns|off] [--bind <IP:PORT>]
messenger-headless ctl  --data-dir <DIR> <command>

ctl commands (one JSON request → one JSON response line on stdout, exit code 0/1):
  node-id                      -> {"ok":true,"node_id":"…"}
  ticket                       -> {"ok":true,"ticket":"…"}                 (MyTicketDto.ticket)
  presence [on|off]            -> {"ok":true,"enabled":true}               (PresenceDto)
  peers                        -> {"ok":true,"peers":[LanPeerDto…]}
  connect <TICKET>             -> {"ok":true,"node_id":"…"} | {"ok":false,"error":"InvalidTicket"}
  respond <NODE_ID> accept|decline
  connected                    -> {"ok":true,"peers":["<node_id>",…]}
  wait <incoming_request|connection_state|lan_peers_changed> [--node <ID>] [--state <S>] [--timeout-secs N]
                               -> first matching event (from a ring buffer of the last 256 events, then live)
  shutdown
```

- **The keyring is file-only.** `--keyring file` is required as an explicit acknowledgement. Any other value (including `os`) and a missing flag are rejected with exit code 2. The headless node **never** touches the OS keychain, so it cannot read or overwrite the desktop app's real identity on a developer machine.
- On first `run`, the binary provisions an identity if the status is `Absent` (`IdentityService::provision` + `set_display_name`). The data dir holds the marker, `settings.json`, `known_peers.json`, `keyring/` and `control.sock`.
- **Control surface:**
  - It is a Unix domain socket at `<data-dir>/control.sock`, created with mode 0600 inside the 0700 data dir.
  - A stale socket is removed only if it is a socket. The binary refuses to run if the path is a symlink or a regular file.
  - **It is never a TCP listener.**
  - Each request is one JSON line, capped at 8 KiB, with a 5 s read timeout per connection.
  - Unknown commands → `{"ok":false,"error":"UnknownCommand"}`.
- **Event output:** `run` also prints each event to stdout as one JSON line, `{"event":"incoming_request","payload":IncomingRequestDto}`, using the same names and payloads as the Tauri events. Build each line from `core::discovery::dto::event_json` (02-01), the same mapping the Tauri shell uses, so no event name is hard-coded in `headless/`. These lines are harness output. Diagnostics go to stderr with NodeIds only; display names never appear in stderr logs.
- **Startup banner** on stderr: "TEST-ONLY headless node: throwaway identity, not for real use; secrets stored unencrypted in <dir>/keyring".
- **Unix only.** On non-unix targets, `main` prints "messenger-headless is supported on unix only" and exits 2, so `cargo build` still succeeds workspace-wide on Windows. Every integration test file under `headless/tests/` (`control.rs`, `parity.rs`) starts with `#![cfg(unix)]`, and unix-only modules (`control.rs`, `file_keychain.rs` permission code) are `#[cfg(unix)]`, so `cargo test --workspace` and `cargo clippy --all-targets` also pass on Windows.

**Parity scenarios (core test-support, `core/src/discovery/testing/scenarios.rs`):**

```rust
#[allow(async_fn_in_trait)]
pub trait NodeDriver {
    async fn node_id(&self) -> String;
    async fn ticket(&self) -> String;
    async fn connect(&self, ticket: &str) -> Result<String, String>;
    async fn respond(&self, node_id: &str, accept: bool) -> Result<(), String>;
    async fn set_presence(&self, on: bool) -> Result<bool, String>;
    async fn presence(&self) -> bool;
    async fn connected(&self) -> Vec<String>;
    /// Wait for an event by Tauri event name; returns its JSON payload.
    async fn wait_event(&self, name: &str, node: Option<&str>, state: Option<&str>, timeout: Duration)
        -> Option<serde_json::Value>;
    async fn restart(&mut self);   // same data dir + keyring
}
pub async fn unknown_prompts_once_then_known_reconnects_silently(a: &mut impl NodeDriver, b: &mut impl NodeDriver);
pub async fn decline_leaves_no_trust(a: &mut impl NodeDriver, b: &mut impl NodeDriver);
pub async fn presence_off_persists_and_stays_dialable(a: &mut impl NodeDriver, b: &mut impl NodeDriver);
pub struct InProcessDriver { .. }  // wraps testing::TestNode
```

The scenarios assert on JSON payloads (the shared DTOs), so the Level-1 and Level-3 runs compare the same observable shape.

## Work breakdown

### Foundation sub-task (sequential, before tracks A/B/C)

This step creates the `headless` crate with real content, so the parallel tracks can build independently. No stubs.

Owns:
- root `Cargo.toml`: add `"headless"` to `[workspace] members`.
- `core/Cargo.toml`: anything Track A's scenarios need beyond what 02-01 added. `serde_json` is already a normal dependency (02-01, for `event_json`); verify it, and add any other crate the `NodeDriver` scenarios use, so Track A builds without touching a manifest.
- `headless/Cargo.toml`:
  - `messenger_core = { package = "core", path = "../core" }` with **no extra features**, plus `protocol`, `tokio`, `serde`, `serde_json`, `zeroize`, all `.workspace = true` where shared.
  - `[dev-dependencies]`: `messenger_core` with `test-support`, and `tempfile`.
- `headless/src/lib.rs`:
  - `#![forbid(unsafe_code)]` and a crate doc that says "test-only harness".
  - `pub mod cli; pub mod file_keychain;`.
  - Two marked regions, `// ── track B modules ──` and `// ── integration ──`, where later code adds its `pub mod` lines. Integration removes the markers.
- `headless/src/cli.rs`: full argument parsing for `run`/`ctl` (`std::env::args` by hand, or a small workspace-pinned parser if justified). It returns typed `RunArgs`/`CtlCommand` values.
  - `--keyring file` is required, and any other value is rejected.
  - `--name` is validated with `validate_display_name`.
  - `--relay` is parsed (`default` / `disabled` / a URL), and `--bind` as `SocketAddr`.
  - Unit tests: a missing `--keyring` and `--keyring os` both give `Err`; also cover bad name, bad relay URL, every `ctl` command, and unknown command.
- `headless/src/file_keychain.rs`, with in-module tests:
  - Create the dir with 0700 (unix). Every write is an atomic tmp+rename with 0600.
  - Read with `symlink_metadata`; refuse non-files and symlinks. Reject length ≠ 32 (`KeychainError::Backend("stored key has wrong length")`).
  - Read into `zeroize::Zeroizing<Vec<u8>>`, then convert with `SecretKeyBytes::from_bytes` / `DbKey::from_bytes`. Zeroize the intermediate array.
  - `delete` on a missing file → `NotFound`.
  - File names use the `AccountId` uuid only; no user input ever reaches a path.
  - Tests: round-trip and delete for both keys; missing → `NotFound`; wrong length → `Backend`; a symlink at an entry path → error and the target is untouched; file mode 0600 and dir mode 0700; two accounts isolated; a full `IdentityService::provision` + `load` over `FileKeychain`.

Gates: `cargo fmt`, `clippy -D warnings`, `cargo test --workspace` and `cargo build` pass on this commit alone.

### Parallel track A: Level-1 parity scenarios (core test-support)

Owns **only** these files:
- `core/src/discovery/testing/{mod.rs, scenarios.rs}`: move and split the existing `testing.rs` (move the code, do not duplicate it), then add `NodeDriver`, the three scenarios, and `InProcessDriver`.
- `core/src/discovery/mod.rs`: only if the module path changes.
- `core/tests/discovery_parity.rs`: runs the three scenarios with `InProcessDriver`.

Every scenario is `#[tokio::test(flavor = "multi_thread")]` with explicit timeouts.

### Parallel track B: headless binary

Owns **only** these files:
- `headless/src/main.rs`
- `headless/src/control.rs` (socket server, client, request/response types)
- `headless/src/node.rs` (wires core)
- the "track B modules" region of `headless/src/lib.rs`
- `headless/tests/control.rs`

Instructions:
- `node.rs` builds the same objects the Tauri shell builds:
  - `IdentityService<FileKeychain, Arc<FileSettingsStore>, LiveTicketProvider>`, `FileKnownPeers`, and `Node::start` with `NodeConfig::desktop()`, adjusted only by `--relay` / `--lan` / `--bind`.
  - There is **no** option that skips the trust prompt, auto-accepts, or disables encryption. `respond` is the only way to accept.
- `control.rs`:
  - Server: `tokio::net::UnixListener`. Each connection handles one request line (capped, with timeout) and sends one response line.
  - Client (`ctl`): connect, write the request, read the response, print it, exit 0 if `ok` and 1 otherwise.
  - An unparseable request → `{"ok":false,"error":"BadRequest"}`, never a panic.
- `headless/tests/control.rs` spawns `env!("CARGO_BIN_EXE_messenger-headless")` with a temp data dir and covers:
  - `control_socket_is_unix_only_and_private`: the socket mode is 0600; on Linux, `/proc/<pid>/net/tcp*` shows no listener owned by the process. Plus a review that `headless/src` has no `TcpListener`.
  - `oversized_or_garbage_control_request_is_rejected`
  - `run_refuses_without_keyring_flag`
  - `run_refuses_os_keyring`
  - `run_refuses_symlinked_socket_path`
  - `banner_says_test_only` (stderr contains "TEST-ONLY")
  - `stdout_events_contain_no_key_material`: a canary scan over captured stdout/stderr and the files in the data dir except `keyring/`.
  - `stdout_event_names_are_the_dd02_names`: every `"event"` value on stdout during a connect/accept run is one of the names `event_json` produces.
  - Kill child processes on drop.

### Parallel track C: Docker image, compose, smoke script

Owns **only** these files:
- `docker/headless.Dockerfile`, `docker/compose.yaml`, `docker/smoke.sh`
- repo-root `.dockerignore`

Code against the CLI contract above.

- `docker/headless.Dockerfile` (multi-stage):
  - **Build stage:** the official `rust` slim image, pinned to the workspace `rust-version` that 02-01 raised (at least 1.91; read it from root `Cargo.toml`), e.g. `rust:<ver>-slim-bookworm`.
    - `COPY . .`, then `cargo build --release --locked -p messenger-headless`.
    - Only the headless crate (and its deps) is compiled. The Tauri crate's manifest is read but not built, so no webkit packages are needed.
  - **Runtime stage:** `debian:bookworm-slim` with only `ca-certificates`.
    - A non-root user: `useradd --system --uid 10001 node`.
    - `RUN mkdir -m 0700 /data && chown node:node /data`. Docker copies this ownership and mode into a fresh, empty named volume on first mount.
    - Copy the single binary, then `USER node` and `ENTRYPOINT ["messenger-headless"]`.
  - **No `ENV`/`ARG` carrying keys or tokens, and no `COPY` of any keyring or data dir.** Identities are generated at container start, inside the per-service volume.
- `.dockerignore`: `target/`, `**/node_modules/`, `app/dist/`, `.git/`, `**/*.key`, `**/*.pem`, `.env*`, `**/keyring/`, `**/control.sock`. This keeps local artifacts and stray key material out of the build context and image layers.
- `docker/compose.yaml`. It uses a named volume per service, so `/data` (identity, known peers) survives `docker compose restart`, and `down -v` removes it:

  ```yaml
  name: p2pm-harness
  x-node: &node
    build: { context: .., dockerfile: docker/headless.Dockerfile }
    image: p2pm-headless:test
    read_only: true
    cap_drop: [ALL]
    security_opt: ["no-new-privileges:true"]
    networks: [lan]
    # NO "ports:" — the control socket lives inside the container only.
  services:
    node-a:
      <<: *node
      volumes: ["node-a-data:/data"]
      command: [run, --data-dir, /data, --keyring, file, --name, "Node A", --relay, disabled, --lan, mdns]
    node-b:
      <<: *node
      volumes: ["node-b-data:/data"]
      command: [run, --data-dir, /data, --keyring, file, --name, "Node B", --relay, disabled, --lan, mdns]
  volumes:
    node-a-data: {}
    node-b-data: {}
  networks:
    lan: { driver: bridge, internal: true }   # multicast works on the bridge; no internet needed with relay disabled
  ```

  For N nodes, document copying a service plus its own volume. Do not use `--scale`, because scaled replicas would share one volume.

  **Relay path in Level 3 (explicit scope decision).** DD-02 says that in containers "the ticket/relay path works regardless". This compose setup runs `--relay disabled` on an `internal: true` network, so the containers exercise the **ticket** path over direct addresses but **not** a relay. Relay coverage comes from the Level-1 test `core/tests/discovery_relay.rs` (relay-only connection + ciphertext canary capture), which runs the same `core` code in the mandatory gate. State this in a comment at the top of `compose.yaml` and in the README Level 3 section, so nobody assumes the containers test the relay.
- `docker/smoke.sh`: `#!/usr/bin/env bash`, `set -euo pipefail`, executable, with a `trap` that always runs `docker compose -f docker/compose.yaml down -v --remove-orphans`. It automates DD-02 "Level 3" steps 1–4:
  1. `up -d --build`.
  2. **LAN:** poll `exec -T node-b messenger-headless ctl --data-dir /data peers` until it lists node-a's NodeId (timeout 60 s).
  3. **Ticket:** A `ticket` → B `connect <ticket>` → A `wait incoming_request` → A `respond <B> accept` → both `wait connection_state --state connected`.
  4. **Known reconnect:** `docker compose restart node-b`. The named volume keeps B's identity and known peers. Assert B's `node-id` is unchanged. B reconnects with A's ticket → A sees `connected` with **no** `incoming_request`. Check this with `wait` and a short timeout that is expected to time out.
  5. **Presence:** A `presence off` → B's `peers` no longer lists A within the mDNS expiry window (poll with a timeout; record the observed time in README) → A `presence on` → A reappears.
  6. **Per-container isolation** (DD-02 "tearing down a container leaves no residue in another"):
     - A's and B's `node-id` differ, and the `account_id` in each node's marker file differs (read with `exec -T node-x cat /data/<marker file>`; use the real marker file name from `core::identity`).
     - B's `/data` contains only B's account: `exec -T node-b ls -R /data` lists no file or keyring entry named after A's `account_id`, and the reverse for A.
     - `docker compose rm -s -f -v node-a` (stop and remove A plus its anonymous volumes; A's named volume is removed only by the final `down -v`). Then B is still running and answers `node-id` with its unchanged NodeId, and B's `/data` listing is byte-identical to before A was removed.
  7. `down -v --remove-orphans`. Then assert both of these are empty:
     - `docker volume ls -q --filter label=com.docker.compose.project=p2pm-harness`
     - `docker ps -a -q --filter label=com.docker.compose.project=p2pm-harness`

  Print PASS/FAIL per stage. Never write tickets to files on the host.

### Integration sub-task (sequential, after A, B, C)

Owns:
- `headless/tests/parity.rs`
- the "integration" region of `headless/src/lib.rs`
- `.githooks/pre-commit`
- `app/src-tauri/tests/pre_commit_hook.rs`
- `app/src-tauri/tests/no_test_backends.rs` (new)
- `README.md`, `AGENTS.md`

1. **`headless/tests/parity.rs`.** `HeadlessDriver` implements `NodeDriver` (Track A) by spawning `messenger-headless run --data-dir <tmp> --keyring file --name <n> --relay disabled --lan off --bind 127.0.0.1:0` (Track B). It drives the node through the `control.rs` client exported from the lib. It runs the **same** three scenarios as `core/tests/discovery_parity.rs`; together they are DD-02's "Harness parity" test.
   - **Scope note (write it in the test's doc comment and the README):** host processes run with `--lan off` because loopback multicast is unreliable, so `presence_off_persists_and_stays_dialable` proves presence **persistence and dialability** at both levels, not LAN **visibility**. DD-02 asks for "a trust-gate / presence test" to pass identically; the trust-gate scenarios satisfy that fully. Presence **visibility** at Level 3 is covered by `docker/smoke.sh` stage 5, and at Level 1 by `discovery_lan.rs` (`MemoryLan`).
2. **Pre-commit gate** blocking test-only backends from the desktop app:

   ```bash
   # Test-only backends must never reach the desktop shell.
   if grep -RnE 'FileKeychain|messenger-headless|messenger_headless' app \
     --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=target --exclude-dir=gen; then
     echo "pre-commit: test-only keyring/headless referenced in app/" >&2
     exit 1
   fi
   ```

   - Add the gate string to the `required` list in `pre_commit_hook.rs`.
   - Both that test and `no_test_backends.rs` live under `app/`, so they would trip the grep themselves. Build the forbidden names with `concat()`, the way `pre_commit_hook.rs` already builds `secret_gate` (e.g. `["File", "Keychain"].concat()`, `["messenger", "-headless"].concat()`).
   - Verify the hook as AGENTS.md requires: stage a file under `app/` that names the file keychain type and confirm the commit is rejected. Then unstage and delete it. Never commit the planted file.
3. **`app/src-tauri/tests/no_test_backends.rs`** (forbidden names built with `concat()`) asserts that:
   - `app/src-tauri/Cargo.toml` `[dependencies]` contains no dependency on the headless crate, and the `core` entry there does not enable `test-support` (that may appear only under `[dev-dependencies]`);
   - `core/Cargo.toml` defines no keyring-backend feature;
   - `app/src-tauri/src/lib.rs` constructs only `OsKeychain` (`native` / `native_for`).
4. **README**, new subsection **"Level 3: containerized headless nodes (Docker)"**:
   - prerequisites (Docker + compose v2);
   - build: `docker compose -f docker/compose.yaml build`;
   - one-command smoke: `docker/smoke.sh`;
   - manual driving: `docker compose -f docker/compose.yaml exec node-a messenger-headless ctl --data-dir /data ticket`, plus `connect` / `wait` / `respond` / `presence` examples;
   - teardown: `docker compose -f docker/compose.yaml down -v` (the `-v` removes the per-node volumes; without it, identities persist until the next `down -v`);
   - scope: containers exercise mDNS + the ticket path over direct addresses; the **relay** path is covered by the Level-1 `discovery_relay.rs` test, not by Docker; parity runs prove presence persistence/dialability, and presence visibility is checked by `smoke.sh` stage 5;
   - clear warnings: **test-only, throwaway identities; never point a real identity at a container**; the file keyring stores secrets unencrypted in the node's volume; the headless node never uses the OS keychain; the control surface has no stability or security guarantees and is not a product feature.

   Add `headless` (test-only) to the workspace table. In AGENTS.md, add `headless/` and `docker/` to the layout table with the same caveats.
5. Run `docker/smoke.sh` locally if Docker is available and paste the PASS output into the PR. If Docker is unavailable, say so explicitly; the audit step must then run it.

## Quality gates (all must pass before commit)

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace                                       # includes headless control + parity tests
cargo build
cd app && npm ci && npm test && npm run check && npm run build
docker/smoke.sh                                              # when Docker is available (not part of the hook)
git commit   # runs .githooks/pre-commit; NEVER --no-verify
```

## Guardrails (AGENTS.md, enforced; do not weaken)

- `SecretKeyBytes` / `DbKey` never cross into `app/`, not even in types or comments. `FileKeychain` lives in `headless/` only, and `app` never depends on `headless`.
- Clipboard (`writeText` / `clipboard`) only in `app/src/views/MyInfo.svelte` (and tests). This step changes no frontend code.
- Logs never contain message plaintext, display names, private keys or secrets; NodeIds and addresses are fine. Headless stderr is a log. Stdout event lines are the harness contract and carry DTOs only.
- `core` keeps `#![forbid(unsafe_code)]`. `headless/src/lib.rs` and `main.rs` set it too.
- Only real-content files. `docker/` and `headless/` carry working code; no placeholder scripts.
- Wire format unchanged. The control protocol is local JSON, not the peer wire. `PROTOCOL_VERSION` stays 1; state this in the PR.
- Data namespaced by `account_id`. The headless node uses the same stores and namespacing as the app.
- Never write credentials, keys, tokens or secrets in code, tests, fixtures, Dockerfiles, compose files or docs. All credentials are provided/generated at runtime (keychain, env at container start, or generated test keys).

## Security (threats specific to this step: prevent and verify)

| Threat | Prevention | How to check |
|---|---|---|
| Container keyring becomes the desktop default / leaks into `app` via feature unification | `FileKeychain` lives in `headless/`, not behind a core feature. `app` does not depend on `headless`. The app constructs only `OsKeychain`. | `no_test_backends.rs`. New pre-commit grep gate (verified with a planted staged file). `cargo tree -p app -e normal \| grep -i headless` → empty. `grep -n "\[features\]" -A5 core/Cargo.toml` shows only `test-support`. |
| Headless node touching the developer's real OS keychain identity | The keyring is file-only; `--keyring os` is rejected and the headless crate never constructs `OsKeychain`. | `run_refuses_os_keyring`, cli unit tests. `grep -rn "OsKeychain" headless/src` → none. |
| Secrets baked into image layers / build context | Identities are generated at runtime in the per-service volume. `.dockerignore` excludes key-like files and data dirs. No `ENV`/`ARG` secrets. | `docker history --no-trunc p2pm-headless:test` shows no secret-looking values. `docker run --rm --entrypoint sh p2pm-headless:test -c 'ls -la /data'` shows an empty dir. `grep -nE 'ENV\|ARG' docker/headless.Dockerfile` → reviewed. |
| Exposed control-surface socket/ports | Unix socket only, 0600 in a 0700 dir, no TCP. Compose has no `ports:` and an `internal: true` network. | `control_socket_is_unix_only_and_private`. `grep -rn "TcpListener" headless/src` → none. `grep -n "ports:" docker/compose.yaml` → none. `docker compose ps` shows no published ports. |
| Headless harness bypasses the trust gate or encryption | Same `Node` code. Only `respond` accepts; no auto-accept flag. Parity scenarios prove identical gate behaviour at Level 1 and Level 3. | `discovery_parity.rs` + `parity.rs` pass identically. `grep -rniE "auto.?accept\|skip_trust\|insecure" headless/src` → none. |
| Malformed control input crashes the node | 8 KiB cap, timeout, typed errors. | `oversized_or_garbage_control_request_is_rejected`. |
| Symlink attacks on keyring/socket paths | `symlink_metadata` checks; non-file/non-socket paths are refused; file names are fixed and built from the uuid. | FileKeychain symlink test, `run_refuses_symlinked_socket_path`. |
| Real identity pointed at a container | Banner and README warnings. A fresh identity per volume; no import path. | README review. `banner_says_test_only`. |
| Residue across containers or after teardown | Each service has its own named volume. `down -v --remove-orphans` removes volumes and containers. | `smoke.sh` stage 6 (distinct NodeIds/account_ids, no cross-node files, removing A leaves B unchanged) and stage 7 (no volumes, no containers left for the project label). |
| Testers assume Level 3 exercises the relay | Explicit scope note in `compose.yaml` and README; relay is covered by the Level-1 `discovery_relay.rs` canary test. | README review; `grep -n "relay" docker/compose.yaml` shows the scope comment. |
| Plaintext/secrets in harness output | DTO-only stdout; NodeId-only stderr. | `stdout_events_contain_no_key_material`. |

## Definition of Done

- [ ] `headless/` (test-only, unix-only) runs the shipping `core` stack, provisions a throwaway identity with the file keyring, and exposes the documented `run`/`ctl` control surface over a private Unix socket. It is not exposed in the GUI.
- [ ] `FileKeychain` lives in `headless/` (no core feature). `--keyring file` is the only accepted backend, `os` is rejected, and the hook + `no_test_backends.rs` keep it out of `app`.
- [ ] `docker/headless.Dockerfile`, `docker/compose.yaml` (per-service named volumes) and `docker/smoke.sh` bring up two isolated nodes that:
  - discover each other over mDNS,
  - connect by ticket: prompt → Accept → connected (the Hello round-trip),
  - keep the same identity across `restart` and reconnect silently when known,
  - honour presence OFF,
  - stay isolated from each other (removing one leaves the other's data and identity untouched),
  - tear down with `down -v`, leaving no volumes or containers.
- [ ] Level 3's relay scope is documented (ticket path in containers; relay covered by the Level-1 relay test).
- [ ] Harness parity: the same scenarios pass at Level 1 (`core/tests/discovery_parity.rs`) and against real headless processes (`headless/tests/parity.rs`), with the presence-visibility scope note written down.
- [ ] Headless stdout event names come from `event_json` (tested). `headless/tests/*` are `#![cfg(unix)]`, so workspace tests pass on non-unix targets.
- [ ] All quality gates + the pre-commit hook pass (no `--no-verify`). The smoke script was run, or its absence is explicitly reported for 02-04.
- [ ] README has step-by-step Level 3 instructions and the warnings. The workspace table and AGENTS.md layout are updated. The PR states "wire format unchanged".
