# Step 02-04: DD-02 audit, gap-fix, and close-out

**Design doc:** [DD-02: Transport, Discovery & Connection](../02-transport-discovery-and-connection.md)
**Order:** 4 of 4 (final). Requires **02-01**, **02-02** and **02-03** merged. It runs alone; nothing runs in parallel with it.
**Branch suggestion:** `step-02-04-dd02-audit`

## Goal

Check every DD-02 Goal, Security bullet, Testing bullet and Definition-of-done item against the **actual code** on `main`. For each one, collect concrete evidence: a file path, a test name, a command and its output. Implement anything missing or weaker than specified, with tests. Run every quality gate and the three simulation levels. Then delete the DD-02 step files, and only those.

Treat the design doc as the source of truth. The step files 02-01…02-03 are implementation guides. Where code and step file disagree but the code satisfies DD-02, the code wins. Where the code does not satisfy DD-02, fix the code.

## Procedure

### 1. Build the evidence matrix (work file, not committed)

Keep a scratch checklist **outside the repo**, e.g. in your session scratch directory. Do not commit an audit report file. For every row, record PASS/FAIL plus evidence.

**Goals**

| # | DD-02 item | Where to look / how to prove |
|---|---|---|
| G1 | One long-lived endpoint per app, bound to the identity key, reachable LAN + internet | `core/src/discovery/node.rs` `Node::start`. App `NetState` starts at most once (`ensure_started`). `NodeConfig::desktop()` uses the iroh default preset (relay + address lookup). |
| G2 | LAN peers auto-located (mDNS), internet via iroh discovery/relay | `lan.rs` `MdnsLan`. Ignored real-mDNS test run manually. Docker smoke LAN stage. |
| G3 | Connect from pasted ticket only, no server we operate | `Ticket::parse` → `connect_ticket`. No hard-coded directory URL: `grep -rnE "https?://" core/src app/src-tauri/src` shows only documented relay defaults, if any. |
| G4 | Hole punching with ciphertext-only relay fallback | `core/tests/discovery_relay.rs` canary test, plus the manual **two-network hole-punch check** in section 3. NAT traversal itself (hole punching, relay fallback, path upgrade) is provided by iroh. The audit verifies that the app uses it unchanged (`NodeConfig::desktop()` on the default preset, IP transports not cleared outside tests), not that it re-implements it. |
| G5 | Presence default ON; OFF stops LAN advertising | `discovery_lan.rs` presence tests. `PresenceToggle` UI. |
| G6 | Unknown NodeId → Accept/Decline; known → silent | `discovery_two_nodes.rs` gate tests. `IncomingRequestPrompt`. |

**Design details**

| # | Item | Proof |
|---|---|---|
| D1 | ALPN `p2p-messenger/1` from `protocol` registered; no other ALPN accepted | `grep -rn "alpns(" core/src` uses `protocol::ALPN` only. |
| D2 | Relay overridable by configuration, persisted in settings by `account_id` | `SettingsStore::{get,set}_relay_url`, `MESSENGER_RELAY_URL`, README. |
| D3 | mDNS record = NodeId + self-chosen name; peers marked unverified | `LanPeerDto.verified == false`. UI badge. The name is carried **only** in the mDNS record (see S12). |
| D4 | Ticket = NodeId + relay URL(s) + direct addrs, produced from the live endpoint | `Ticket::from_endpoint_addr`, `LiveTicketProvider`, `my_ticket_parses_and_names_this_node`. |
| D5 | API: `net_presence_get/set`, `net_lan_peers`, `net_connect_ticket`, `net_my_ticket`, `net_respond_incoming`; events `lan_peers_changed`, `incoming_request`, `connection_state` | `grep -n "net_\|emit(" app/src-tauri/src/*.rs` and `generate_handler!` list. The frontend `listen(` calls use exactly these names. The one addition, `net_pending_incoming` (read-only prompt snapshot for UI seeding), is documented in the PR and exposes nothing beyond `incoming_request`. |
| D6 | LAN peers ephemeral; no other people's addresses or tickets persisted beyond known NodeIds | `lan_peers_are_never_persisted`. Inspect the `known_peers.json` schema: NodeId + name + timestamp only. |
| D7 | Both paths converge on one trust check | `lan_pick_uses_the_same_trust_gate`, `lan_and_ticket_paths_share_one_trust_gate_end_to_end`. |
| D8 | Post-accept hand-off stays inside DD-02 scope (Hello exchange + connected state; no Chat UI) | `connected_peers()` only holds authorized connections. No Chat UI exists yet. |

**Security bullets** (each must have a passing test or a grep/manual check that is actually executed)

| # | Bullet | Verify |
|---|---|---|
| S1 | No side channel or debug bypass carrying content | `grep -rniE "skip_trust\|auto.?accept\|no_prompt\|bypass\|insecure\|danger" core/src app/src-tauri/src headless/src`: any hit must be test-only and justified. Relay canary test passes. |
| S2 | NodeId is the only identity; names never used for trust/dedup; same-name peers distinct | Review every use of `display_name` in `core/src/discovery`. Same-name tests (Rust + Vitest). |
| S3 | Unknown peers gated; declined/pending carry no persistence or history | Gate tests incl. `decline_closes_and_persists_nothing`, `pending_prompt_expires`. |
| S4 | Flood guard: cap pending, per-NodeId rate limit, silent drops | `flood_guard_bounds_prompts` + unit tests. No log above trace on the drop path. |
| S5 | Ticket validation; malformed → handled error, never panic | Fuzz + corrupted-ticket tests. `grep -n "unwrap()\|expect(" core/src/discovery core/src/identity/ticket.rs` outside tests. |
| S6 | mDNS opt-out discoverable; presence OFF verifiably stops advertising | Presence tests + GUI copy + Docker presence stage. |
| S7 | Relay trust boundary documented (metadata visible, never plaintext; self-host mitigation) | README section exists. |
| S8 | No plaintext in diagnostics | `grep -rnE "(info\|warn\|error\|debug\|trace)!\(" core/src headless/src app/src-tauri/src`: inspect every call; no names, frames, tickets or keys. If a capture-subscriber test exists, it passes. |
| S9 | Simulation affordance is isolation, not bypass; no looser test default | Profile tests. `no_test_backends.rs`. App defaults unchanged when no profile is set. |
| S10 | Container keyring scoped to containers; never a desktop default; documented | `FileKeychain` lives in `headless/` (no core feature, so no feature unification into `app`). `app` does not depend on `headless`. `--keyring file` is the only accepted value and `os` is rejected. Pre-commit gate, `no_test_backends.rs`, README warning. |
| S11 | AGENTS.md rules | The pre-commit secret grep + clipboard grep pass. `grep -rn "forbid(unsafe_code)" core/src/lib.rs headless/src/lib.rs headless/src/main.rs`. |
| S12 | Self-chosen name never published to internet infrastructure; not published while presence is OFF | `internet_published_record_has_no_name`, `presence_off_withdraws_name_from_mdns_record`. `grep -rn "set_user_data_for_address_lookup" core/src` → none, or `None` only. The `lan.rs` module doc records which lookup services publish user data. |

**Testing bullets.** Map each DD-02 Testing bullet to a named, passing test:

| DD-02 Testing bullet | Expected test(s) |
|---|---|
| Two in-process nodes (LAN sim) | `discovery_lan.rs::lan_sim_b_discovers_a_with_node_id_and_unverified_name` |
| Presence off / on | `presence_off_hides_a_and_on_brings_it_back`, `presence_off_still_browses_and_stays_dialable` |
| Ticket round-trip + corrupted ticket | `ticket_round_trip_yields_live_channel`, `corrupted_ticket_is_handled_and_endpoint_stays_up`, `net_cmd` end-to-end flow |
| Trust gate: unknown | `unknown_peer_raises_exactly_one_prompt_and_gets_no_traffic_before_accept`, `decline_closes_and_persists_nothing` |
| Trust gate: known | `known_peer_reconnects_silently_across_restart` |
| Flood guard | `flood_guard_bounds_prompts` + `flood.rs` unit tests |
| Relay fallback + payload capture | `discovery_relay.rs` |
| Resilience (mDNS records + tickets) | `malformed_and_oversized_mdns_records_never_panic`, ticket fuzz, `non_hello_first_frame_and_oversized_hello_are_rejected` |
| Profile isolation | `core/tests/profile_isolation.rs`, `app/src-tauri/src/profile.rs` tests |
| Containers | `docker/smoke.sh` PASS (all stages incl. teardown residue check) |
| Harness parity | `core/tests/discovery_parity.rs` + `headless/tests/parity.rs` (same scenarios) |

**Definition of done (DD-02).** Copy all 12 checkboxes from the design doc into the scratch list and tick each one with evidence.

**User decisions** (must hold):
- `KnownPeers` trait with `is_known`/`add`, plus a persisted, `account_id`-namespaced accepted-NodeId store. A known peer reconnects silently across restarts.
- The round-trip is the Hello exchange + connected state. No Chat UI. Nothing past Hello for unknown peers before Accept.
- Interim dial rule: an outbound dial adds the peer to the known set **only after** its Hello reply (`dial_adds_known_only_after_hello_reply`). The UI says "Connect and remember this device", and README documents the rule.

### 2. Fix gaps

For every FAIL:
- implement the missing behaviour in the owning slice/file,
- add or extend the test that proves it,
- update README/AGENTS.md if behaviour or structure changes.

Keep fixes minimal and in scope; do not start DD-03/DD-04 work. If a gap needs a design decision the doc does not settle, stop and report it instead of guessing.

### 3. Run every gate and every simulation level

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build && cd ..
cargo test -p core --test discovery_lan -- --ignored          # real mDNS (needs a multicast-capable LAN)
docker/smoke.sh                                                 # Level 3 (needs Docker)
```

Also complete these manual checks and record the results in the PR:
- **Level 2 manual run**: follow README "Two-device simulation → Level 2" exactly as written, on a clean machine state: two profiles, first-run each, LAN list, ticket paste, Accept prompt, Connected, presence OFF hides from the LAN list but the ticket still connects, relaunch → known peer connects silently.
  - If a README instruction is wrong, fix the README.
  - Then check the keychain: there are separate entries for both profile services, and the default `com.p2p-messenger.identity` is untouched.
- **README manual GUI checklist**: walk every item.
- **Pre-commit hook verification** (AGENTS.md): stage a throwaway file under `app/` containing a secret-type identifier, and separately one naming the test-only file keychain type. Confirm `git commit` rejects each, then unstage and delete them. Never commit them.
- **Security greps**: run the commands in the tables above and resolve every unexpected hit.
- **Image hygiene**:
  - `docker history --no-trunc p2pm-headless:test` shows no secret-looking values.
  - `docker compose -f docker/compose.yaml config` shows no `ports:`, no bind mounts, and only the per-node named volumes.
  - After `docker/smoke.sh`, no project volumes or containers remain.
- **Two-network hole-punch check** (manual; NAT traversal is iroh's job, and this confirms the app does not defeat it):
  1. Run two app instances (or two headless nodes with `--relay default --lan off`) on **different networks**, e.g. home Wi-Fi and a phone hotspot, each behind its own NAT.
  2. Connect by ticket and Accept.
  3. Using iroh's path/connection-type information (log NodeIds and path type only; verify the exact API, e.g. the connection's path list or `is_ip()` / `is_relay()` on path info), confirm the connection starts on the relay and, when the NATs permit, **upgrades to a direct (IP) path**.
  4. Record the observed path types in the PR.

  If both NATs are symmetric and no direct path appears, record that and confirm the relay path still carries the session (ciphertext-only, per the canary test).

If the real-mDNS test or Docker cannot run on this machine, the audit is **not complete**. Report exactly which check could not run and why, and do **not** delete the step files.

### 4. Close out

Only when every row is PASS and every gate is green:
1. Delete the DD-02 step files: `git rm design_docs/steps/02-*.md`. Delete nothing else under `design_docs/steps/`; other designs' step files are not yours. If `design_docs/steps/` becomes empty, git simply stops tracking it. Do not add a placeholder file.
2. Optionally update the DD-02 header `Status:` line to reflect "Implemented (v1)". Do not rewrite the design.
3. Commit through the hook (never `--no-verify`). The PR follows `.github/PULL_REQUEST_TEMPLATE.md`:
   - affected crates/slices (`protocol` constant, `core::identity` ticket/settings/keychain/profile, `core::discovery`, `app`, `headless`, `docker`),
   - **wire format unchanged, `PROTOCOL_VERSION` not bumped** (or, if any audit fix changed the wire, bump it and say so),
   - verification commands + manual results,
   - README updated.

## Quality gates (all must pass)

`cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo build`, `cd app && npm ci && npm test && npm run check && npm run build`, the pre-commit hook (never `--no-verify`), and additionally the ignored real-mDNS test and `docker/smoke.sh`.

## Guardrails (AGENTS.md, enforced; do not weaken)

- `SecretKeyBytes` / `DbKey` never cross into `app/`, not even in types or comments. Re-run the hook's grep.
- Clipboard (`writeText` / `clipboard`) only in `app/src/views/MyInfo.svelte` (and tests).
- Logs never contain message plaintext, display names, private keys or secrets. NodeIds and addresses are fine.
- `core` forbids unsafe (`#![forbid(unsafe_code)]`). The headless crate too.
- Only real-content files: the audit adds no report files, stubs or placeholders.
- Wire format: postcard `Envelope` with `version`. Any breaking change bumps `PROTOCOL_VERSION`.
- All persisted data is namespaced by `account_id`.
- Never write credentials, keys, tokens or secrets in code, tests, fixtures, Dockerfiles, compose files or docs. All credentials are provided/generated at runtime (keychain, env at container start, or generated test keys).

## Security: what the audit must re-verify

For the **prevention** each check relies on, see the Security tables of 02-01 (endpoint, gate, tickets, flood guard, relay), 02-02 (mDNS, presence, name placement, profile, relay override) and 02-03 (headless, keyring, Docker). Read them before running the checks below. The step files are deleted only at close-out, after this table is all PASS.

| Vulnerability | Check |
|---|---|
| Unknown peer reaching messaging before Accept | Gate tests pass. Single `accept_bi` call site. `connected_peers()` excludes pending. |
| Prompt-flood DoS | Flood tests. Review the bounded tables (`max_tracked_peers`, `max_handshakes`). |
| Ticket / mDNS / Hello / control-request parsing panics or oversized input | Fuzz + resilience tests. `unwrap`/`expect` grep on peer-input paths. |
| Self-asserted name used for trust/dedup | Code review of every `display_name` use. Same-name tests. |
| Presence OFF still advertising (incl. startup) | Presence tests, Docker presence stage, manual Level 2. |
| Plaintext in logs / side channel / debug bypass | Relay canary test. Log-call review. Bypass grep. |
| Profile override path traversal / keychain namespace collision / silent fallback | Profile tests. Manual `MESSENGER_PROFILE=../x` refuses to start. |
| Container keyring becoming the desktop default | `no_test_backends.rs`, hook gate. `cargo tree -p app -e normal \| grep -i headless` → empty. `core/Cargo.toml` has no keyring feature. `run_refuses_os_keyring`. |
| Self-chosen name leaked to public infra (pkarr/DNS) or kept published while presence is OFF | `internet_published_record_has_no_name`, `presence_off_withdraws_name_from_mdns_record`, `set_user_data_for_address_lookup` grep (02-02 security table). |
| Secrets baked into Docker images/layers; exposed control socket/ports | `docker history`, `compose config`, socket-mode test, no `TcpListener`. |
| Test-only TLS/relay shortcuts in shipping code | `grep` for skip-verify/insecure in non-test code → none. `test-support` only in dev-deps. |

## Definition of Done

- [ ] Every DD-02 Goal, design detail, Security bullet, Testing bullet and Definition-of-done checkbox has recorded evidence of PASS. Both user decisions hold.
- [ ] All gaps found were fixed with tests, and README/AGENTS.md reflect the final behaviour and structure.
- [ ] The self-chosen name is verified absent from internet-published data and withdrawn while presence is OFF (S12).
- [ ] The two-network hole-punch check was performed and its observed path types recorded. NAT traversal is confirmed to be iroh's, used unchanged.
- [ ] All quality gates, the pre-commit hook (no `--no-verify`), the real-mDNS ignored test, the Level-2 manual run and `docker/smoke.sh` pass.
- [ ] `design_docs/steps/02-*.md` deleted, and only those files. The commit/PR follows the template and states the protocol impact explicitly.
