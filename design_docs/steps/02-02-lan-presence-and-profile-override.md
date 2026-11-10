# Step 02-02: LAN discovery (mDNS), presence toggle, relay override, and Level-2 profile override

**Design doc:** [DD-02: Transport, Discovery & Connection](../02-transport-discovery-and-connection.md)
**Order:** 2 of 4. Requires **02-01** merged (it needs `core::discovery::Node`, events/DTOs, `net_cmd.rs`, `Network.svelte`, `netStore.ts`). Must be finished before 02-03.
**Branch suggestion:** `step-02-02-lan-presence-profile`

## Goal

Finish the GUI side of DD-02:

- **LAN discovery over mDNS**: a live "peers on this network" list. Each entry is marked unverified and shows the NodeId and the self-asserted name.
- **Presence toggle**: default **ON**, persisted per `account_id`. OFF stops LAN self-advertisement. The node keeps browsing and stays dialable by ticket holders.
- **Relay override**: persisted in settings so a user can point at a self-hosted relay.
- Tauri commands `net_presence_get`, `net_presence_set`, `net_lan_peers` and the event `lan_peers_changed`.
- **Level-2 simulation**: a documented profile override (`--profile <name>` / `MESSENGER_PROFILE`) that isolates the data dir and keychain namespace so two full app instances run on one machine.

Both discovery paths (LAN pick and pasted ticket) must converge on the **same** `connect_ticket` → trust gate from 02-01 ("two paths, one trust model").

When this step is done, the whole DD-02 GUI works end to end:

- LAN peers list,
- presence toggle with "invisible ≠ unreachable" copy,
- connect by ticket,
- Accept/Decline prompt,
- connection state,
- My Info ticket from the live endpoint,
- two instances via profiles.

## Current code you build on (after 02-01)

- `core/src/discovery/node.rs`: `Node::start(identity, NodeDeps { settings, known, tickets }, NodeConfig)`, `subscribe()`, `connect_ticket()`, `NetEvent`, DTOs in `dto.rs`, test-support `testing::spawn_test_node`.
- `core/src/identity/settings.rs`: `SettingsStore` (display name, provisioned_at), `FileSettingsStore` with `StoredAccount` (`deny_unknown_fields`) and the test `file_store_file_contains_no_secrets_and_only_known_keys`, which asserts the exact key set.
- `core/src/identity/keychain.rs`: `APP_KEYCHAIN_SERVICE = "com.p2p-messenger.identity"`, `OsKeychain(())`, `OsKeychain::native()`. Entries are `<account_id>/identity-key` and `<account_id>/db-key`. Tests construct `OsKeychain(())` with `keyring_core::mock`.
- `app/src-tauri/src/lib.rs`: `setup` uses `app.path().app_data_dir()` + `0o700` permissions.
- `app/src/views/Network.svelte`, `app/src/lib/netStore.ts`, `app/src/net.ts`.

## mDNS API (verify against docs; do not invent)

iroh 1.x moved mDNS out of the core crate. Context7 (`/llmstxt/iroh_computer_llms-full_txt`) documents:

```rust
// iroh-mdns-address-lookup = "0.5" (verify the version matching your iroh 1.x)
use iroh_mdns_address_lookup::MdnsAddressLookup;
let mdns = MdnsAddressLookup::builder().build(endpoint.id())?;
endpoint.address_lookup()?.add(mdns.clone());
let events = mdns.subscribe().await; // discovered / expired peer events
```

There is also `iroh-mdns-peer-lookup` with `Options::new().announce(bool)`. Before choosing one, confirm on docs.rs:

- (a) how to **turn advertising off** while still browsing (a builder `advertise(bool)`/`announce(bool)` flag, or removing/re-adding the service),
- (b) how to attach the display name **to the mDNS record only**, e.g. a per-service TXT/user-data option on the mDNS builder or service. Check its byte limit.
- (c) the discovered/expired event types,
- (d) **which address-lookup services publish the endpoint-wide user data.** iroh 1.x has `Endpoint::set_user_data_for_address_lookup(Option<UserData>)`, and `publish_my_addr` hands the same `EndpointData` (including user data) to **every** registered lookup. Under the N0/desktop preset that includes the internet publisher (pkarr/DNS).

Pick the crate that supports (a) and (b). Record the choice, version and the answers to (a)–(d) in the `lan.rs` module doc. If neither crate supports turning advertising off while browsing, **stop and report**. Do not fake presence OFF.

**Name placement (privacy rule).** The self-chosen name must never be published to public internet infrastructure, and it must not stay published while presence is OFF.

- **Do not** put the name in the endpoint-wide `set_user_data_for_address_lookup`. The endpoint-wide `EndpointData` must carry **no name**, ever.
- Put the name only in the mDNS service's own record data (b). It must fit that limit: truncate on a `char` boundary.
- If the only way to reach mDNS is the endpoint-wide user data, an acceptable fallback is a documented, test-verified option that makes the internet publisher exclude user data. If neither exists, **stop and report**; for example, the user may choose to drop the name from mDNS and show it only after Hello.
- When presence goes OFF, the mDNS record (and its name) is withdrawn. When it comes back ON, the current name is re-attached.
- Receivers re-validate the name with `validate_display_name`; invalid input becomes `None`.

## Interface contract (agree before splitting into tracks)

```rust
// core/src/discovery/lan.rs (Track A)
pub const MAX_LAN_PEERS: usize = 256;
/// Pure codec for the name carried in the mDNS record, unit-tested in the pre-commit gate.
pub fn encode_lan_name(name: &DisplayName, max_bytes: usize) -> Vec<u8>;  // truncates on a char boundary
pub fn decode_lan_name(raw: &[u8], max_bytes: usize) -> Option<DisplayName>; // oversize / non-UTF-8 / invalid -> None
/// Pure registry (no I/O): NodeId-keyed, self-filtered, capped, the only place LanEvents become peers.
pub struct LanPeers { .. }
impl LanPeers {
    pub fn new(own_id: NodeId) -> Self;
    pub fn apply(&mut self, ev: LanEvent) -> bool; // true if the visible list changed; calls decode_lan_name
    pub fn snapshot(&self) -> Vec<LanPeer>;         // sorted by NodeId
}
pub struct LanPeer { pub node_id: NodeId, pub display_name: Option<DisplayName> /* UNVERIFIED */, pub addr: EndpointAddr }
pub enum LanEvent { Discovered { addr: EndpointAddr, user_data: Option<Vec<u8>> }, Expired(NodeId) }
/// Transport seam. MdnsLan is the shipping impl; testing::MemoryLan is a deterministic in-process bus.
pub trait LanDiscovery: Send + Sync + 'static {
    fn set_advertise(&self, on: bool, local_name: Option<&DisplayName>) -> Result<(), LanError>;
    fn events(&self) -> tokio::sync::mpsc::Receiver<LanEvent>; // called once by Node
}
pub enum LanMode { Mdns, Off, #[cfg(any(test, feature = "test-support"))] Memory(testing::MemoryLanBus) }

// NodeConfig gains: pub lan: LanMode   (desktop() = Mdns, loopback() = Off unless a test sets Memory)
// NetEvent gains:   LanPeersChanged(Vec<LanPeer>)        -> Tauri "lan_peers_changed"
// dto.rs gains:
#[derive(Serialize)] pub struct LanPeerDto { pub node_id: String, pub display_name: Option<String>, pub verified: bool /* always false in DD-02 */, pub ticket: String }
#[derive(Serialize)] pub struct PresenceDto { pub enabled: bool }

impl Node {
    pub fn presence(&self) -> bool;
    pub fn set_presence(&self, on: bool) -> Result<(), PresenceError>; // persist FIRST, then apply
    pub fn lan_peers(&self) -> Vec<LanPeer>;                          // snapshot, sorted by NodeId
    pub fn refresh_local_name(&self);                                  // re-publish name after rename
}

// core/src/identity/settings.rs (Track A): SettingsStore gains
fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError>;   // None => ON
fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError>;
fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError>;
fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError>;

// core/src/identity/profile.rs (Track B)
pub struct Profile(String);                  // validated, lowercase
pub enum ProfileError { Empty, TooLong, InvalidChar, Reserved }
impl Profile {
    pub fn parse(raw: &str) -> Result<Profile, ProfileError>; // [a-z0-9-]{1,32} after ASCII-lowercasing, no leading '-', not "default"
    pub fn as_str(&self) -> &str;
    pub fn keychain_service(&self) -> String;                  // "com.p2p-messenger.identity.profile.<name>"
    pub fn data_dir(&self, base: &Path) -> PathBuf;            // base/profiles/<name>
}
// keychain.rs: OsKeychain stores its service name
impl OsKeychain {
    pub fn native() -> Result<Self, KeychainError>;                         // unchanged: APP_KEYCHAIN_SERVICE
    pub fn native_for(profile: Option<&Profile>) -> Result<Self, KeychainError>; // None == native()
}
```

Rules:

- `LanPeer.addr` comes **only** from the mDNS record. Each `LanPeerDto.ticket` is built by core (`Ticket::from_endpoint_addr`). The UI's "Connect" on a LAN peer calls the existing `net_connect_ticket(peer.ticket)`. There is no separate LAN dial path and no separate trust path.
- Entries are keyed by NodeId only. Our own NodeId is filtered out. The list is capped at `MAX_LAN_PEERS` (further discoveries are ignored until entries expire). A name update replaces the name and never creates a second entry.
- Discovered LAN peers are **memory only**. Never persist them. That includes `known_peers.json`: only Accept/dial adds there.

## Work breakdown

### Parallel track A: core LAN, presence, settings

Owns **only** these files:
- `core/src/discovery/lan.rs` (new)
- `core/src/discovery/{node.rs, events.rs, dto.rs, mod.rs, testing.rs}`
- `core/src/identity/settings.rs`
- `core/Cargo.toml`, root `Cargo.toml`
- `core/tests/discovery_lan.rs` (new)

Instructions:
- **Settings:**
  - Add `presence_enabled: Option<bool>` and `relay_url: Option<String>` to `StoredAccount` and `AccountSettings`, both with `#[serde(default)]`. Old v1 files still parse, so `SETTINGS_VERSION` stays 1.
  - Update `file_store_file_contains_no_secrets_and_only_known_keys` to the new exact key set.
  - `set_relay_url` validates with `RelayUrl::from_str`, scheme `https` (or `http` only for loopback hosts), max 2048 bytes, and stores the canonical string. Reading back an invalid stored URL → `SettingsError::Unavailable`.
  - Implement for `MemorySettingsStore` and `FileSettingsStore`.
- **Presence in `Node`:**
  - On start, read `get_presence` (`None` → `true`) and apply it **before** the first advertisement. With presence persisted as OFF, the node must never advertise even briefly at startup.
  - `set_presence` persists first. If persisting fails, return `PresenceError` and leave advertising unchanged. Then it calls `lan.set_advertise(on, name)`.
  - OFF never stops browsing, the accept loop, or dialing.
  - `refresh_local_name()` re-publishes the user data when presence is ON.
- **Relay override:** `NodeConfig::desktop()` stays the default. When the app starts the node, `settings.get_relay_url(account)`, if `Some`, maps to `RelayChoice::Custom(url)`. Relay choice affects reachability and metadata only. The encrypted channel and trust gate are unchanged.
- **Pure decoding and registry:** `encode_lan_name` / `decode_lan_name` and the `LanPeers` registry (see contract) hold **all** record interpretation: name decoding, NodeId keying, self-filter, cap, name update. They are pure functions/structs with in-module unit tests that run in the mandatory gate. `MdnsLan` only moves bytes from the multicast socket into `LanEvent { user_data: raw bytes }` and never interprets the name. That leaves the multicast socket as the only part outside the gate.
- **MdnsLan:** wraps the chosen crate. The browse task maps discovered/expired events into raw `LanEvent`s. Malformed events are skipped, never `unwrap`ped. `Node` feeds them to `LanPeers::apply` and turns changes into `NetEvent::LanPeersChanged` snapshots, debounced, at most one event per 250 ms. `set_advertise(true, name)` attaches `encode_lan_name(name)` to the mDNS record only (see "Name placement"). `set_advertise(false, _)` withdraws the record.
- **`testing::MemoryLanBus` / `MemoryLan` (test-support):**
  - It is an in-process bus. Advertising nodes are visible to every other node on the same bus. `set_advertise(false)` sends `Expired` to the others.
  - `inject(LanEvent)` lets resilience tests feed malformed records through the **same** `LanPeers` registry and node code. Only the multicast socket is replaced.
- **Real-mDNS test:** one two-node test using `LanMode::Mdns`, marked `#[ignore = "needs a multicast-capable interface; run: cargo test -p core --test discovery_lan -- --ignored"]`. Loopback multicast is unreliable on Linux (`lo` lacks MULTICAST), so this test stays out of the mandatory gate. Docker (02-03) and the audit (02-04) run it.

Tests (`core/tests/discovery_lan.rs`, `#[tokio::test(flavor = "multi_thread")]`, bounded waits), mapped to the DD-02 Testing bullets:
- `lan_sim_b_discovers_a_with_node_id_and_unverified_name`: A ON → B's `lan_peers()` contains A's NodeId with name "Alice", and `LanPeersChanged` fires.
- `presence_off_hides_a_and_on_brings_it_back`: OFF → A disappears from B. ON → A reappears.
- `presence_off_still_browses_and_stays_dialable`: A OFF still lists B. B dials A by ticket → A gets `IncomingRequest` (same gate) → Accept → `Connected`.
- `presence_defaults_on_and_persists_across_restart`: a fresh account → ON. Set OFF, restart with the same `FileSettingsStore` → still OFF, and the memory bus never saw A advertise during startup.
- `presence_set_persist_failure_leaves_advertising_unchanged` (settings double that returns an error).
- `lan_pick_uses_the_same_trust_gate`: B connects using A's `LanPeerDto.ticket` → exactly one prompt on A. Once A has accepted B, B reconnecting via LAN is silent.
- `same_name_two_node_ids_stay_distinct`.
- `malformed_and_oversized_mdns_records_never_panic`: inject a 64 KiB user data blob, invalid UTF-8, a bidi/control-char name, our own NodeId, an empty addr, and 1 000 distinct peers. Result: no panic, `len() <= MAX_LAN_PEERS`, self not listed, bad names → `None`.
- `lan_peers_are_never_persisted`: after discovery, `known_peers.json` and `settings.json` contain no peer NodeId.
- `real_mdns_two_nodes_discover_each_other` (`#[ignore]`, see above).
- `internet_published_record_has_no_name`:
  - Register a recording `AddressLookup` (test-support; it implements iroh's `AddressLookup::publish` and stores every `EndpointData`) on a node built through the **same** builder path `NodeConfig::desktop()` uses, minus the real network publishers.
  - With presence ON and a canary name, assert no recorded `EndpointData` has user data containing the canary, or any user data at all if the endpoint-wide field is unused.
  - Toggle OFF then ON and rename: assert the same.
  - If the fallback "internet publisher excludes user data" option was used instead, assert directly on that publisher's configuration and payload.
- `presence_off_withdraws_name_from_mdns_record`: with `MemoryLan` recording what `set_advertise` attaches, OFF results in no record and no name, and ON re-attaches the current name.
- `refresh_local_name_republishes_after_rename`: rename while ON → the other node sees the new name via `LanPeersChanged`. Rename while OFF → nothing is published.
- In-module unit tests for `encode_lan_name`/`decode_lan_name`/`LanPeers` (pure, in the gate): truncation on a multi-byte char boundary, oversize/non-UTF-8/bidi/control → `None`, self-filter, cap, name update without a duplicate entry, `Expired` removal.
- Settings unit tests: presence/relay round-trip in Memory and File stores. Old v1 file without the new fields parses. Invalid relay URL rejected. Account isolation.

### Parallel track B: profile override (Level 2)

Owns **only** these files:
- `core/src/identity/profile.rs` (new), `core/src/identity/keychain.rs`, `core/src/identity/mod.rs` (`mod profile;` + re-export `Profile`, `ProfileError`)
- `core/tests/profile_isolation.rs` (new)
- `app/src-tauri/src/profile.rs` (new)
- the single line `mod profile;` in `app/src-tauri/src/lib.rs`, so the module and its tests compile in this track. No other track touches `lib.rs` in parallel; integration edits it afterwards.

No foundation sub-task is needed in this step. Track A is the only track touching `Cargo.toml`s (mDNS dependency) and `core/src/discovery/mod.rs`. Track B declares its own modules as listed. Track C touches only frontend files. Each track builds on its own.

Instructions:
- `Profile::parse`:
  - Trim, then ASCII-lowercase, so case-insensitive filesystems (macOS/Windows) cannot collide "A" and "a".
  - Accept only `[a-z0-9-]`, length 1..=32, no leading `-`. Reject `default` (reserved for "no override").
  - This rejects `..`, `/`, `\`, `.`, NUL, spaces and Unicode, which blocks path traversal and keychain-name injection.
- `OsKeychain` becomes `OsKeychain { service: String }`. `entry()` uses `self.service`. `native()` keeps **exactly** `APP_KEYCHAIN_SERVICE`, so existing users' keys are found, and needs no migration. Update the in-module tests that construct `OsKeychain(())`.
- `app/src-tauri/src/profile.rs`:
  - `pub fn profile_from(args: impl IntoIterator<Item = String>, env: Option<String>) -> Result<Option<Profile>, ProfileError>`. It accepts `--profile <name>` and `--profile=<name>` from args and `MESSENGER_PROFILE` from env, and the CLI wins.
  - `pub struct ProfileLayout { pub data_dir: PathBuf, pub profile: Option<Profile> }` and `pub fn layout(base: &Path, profile: Option<Profile>) -> ProfileLayout`.
  - An invalid value is a **startup error**, shown as a clear message (`setup` returns `Err`). It must **never** silently fall back to the default profile, because that would mix two testers' identities.
  - `pub fn window_title(base: &str, profile: Option<&Profile>) -> String`: returns `base` unchanged when there is no profile, and `"<base> — profile <name>"` otherwise. It is pure, so it can be tested; integration calls it.
- This is a developer/test affordance: document it in README, not in the normal UI. The window title gets a suffix ("p2p-messenger — profile b") only when a profile is active, so testers can tell the windows apart.

Tests:
- `profile.rs` (core) unit tests: valid names, lowercasing, and rejections for `""`, `"   "`, `".."`, `"a/b"`, `"a\\b"`, `"-a"`, 33 chars, `"défault"`, `"default"`, `"a b"`, NUL.
- `core/tests/profile_isolation.rs`, maps to "Simulation harness: profile isolation":
  - Use `keyring_core::mock` as the default store (as `keychain.rs` tests do).
  - Two `IdentityService`s with `OsKeychain::native_for(Some(a))` / `Some(b)` and data dirs `base/profiles/a` / `base/profiles/b` provision distinct `account_id`/NodeIds.
  - Storing a key for the **same** `AccountId` under both services yields distinct, independently loadable values.
  - Deleting profile a's keys leaves b's intact.
  - b's data dir never contains a's marker/settings files, and the reverse.
  - `native_for(None)` uses `APP_KEYCHAIN_SERVICE` (regression guard for existing users).
  - Verify that the mock store keys entries by (service, user). If it does not, assert at the service-name level instead and say so in the test doc.
- `app/src-tauri/src/profile.rs` unit tests: CLI-over-env precedence, `--profile=` form, invalid → `Err` (not `Ok(None)`), `layout(base, None).data_dir == base`, distinct profiles → distinct data dirs and keychain services, `window_title("p2p-messenger", None) == "p2p-messenger"`, and `window_title(.., Some(b))` ends with `"profile b"`.

### Parallel track C: frontend

Owns **only** these files:
- `app/src/net.ts`, `app/src/lib/netStore.ts`
- `app/src/views/Network.svelte`
- `app/src/views/PresenceToggle.svelte` (new), `app/src/views/LanPeers.svelte` (new)
- their `*.test.ts`

JSON contract:

```ts
export interface LanPeerDto { node_id: string; display_name: string | null; verified: boolean; ticket: string }
export interface PresenceDto { enabled: boolean }
export const netPresenceGet = () => invoke<PresenceDto>("net_presence_get");
export const netPresenceSet = (enabled: boolean) => invoke<PresenceDto>("net_presence_set", { enabled });
export const netLanPeers = () => invoke<LanPeerDto[]>("net_lan_peers");
// event "lan_peers_changed" payload: LanPeerDto[] (full snapshot)
// NetCmdError gains { type: "SettingsUnavailable" }
```

> **Clipboard gate warning:** the pre-commit hook and `clipboard-guard.test.ts` grep for the words `writeText` and `clipboard` in **every** non-test file under `app/src`, comments included. Do not write either word in any file except `MyInfo.svelte`.

- `netStore.ts`: a `lanPeers` store. `startNetEvents()` also listens to `lan_peers_changed` and seeds itself with `netLanPeers()` once.
- `PresenceToggle.svelte`:
  - A clearly labelled switch "Visible on this network". It is first on the Network tab, so a user on café Wi-Fi finds it immediately.
  - Copy under it: "Off = other devices on this network won't see you in their list. You are **not** offline: anyone who already has your ticket can still connect, and you can still connect to others."
  - It is optimistic only after `netPresenceSet` resolves. On error, revert and show `role="alert"`.
- `LanPeers.svelte`: heading "Peers on this network". Each row shows:
  - the name as text, or "(no name)",
  - an **Unverified** badge (static, always present while `verified === false`),
  - the full NodeId in small gray text,
  - a "Connect" button → `netConnectTicket(peer.ticket)`, which reuses the error display from 02-01.

  Empty state: "No peers found on this network." **No clipboard use.**
- `Network.svelte` order: PresenceToggle, LanPeers, connect-by-ticket (02-01), connection list (02-01).

Vitest:
- `PresenceToggle.test.ts`: loads state, toggles via `netPresenceSet`, reverts on error, and the "not offline / ticket holders can still connect" copy is present.
- `LanPeers.test.ts`: renders a name + Unverified + full NodeId. Two peers with the same name show two rows with distinct NodeIds. An HTML/bidi-looking name renders inert. Connect calls `netConnectTicket` with that peer's ticket. Empty state.
- `netStore.test.ts`: `lan_peers_changed` replaces the snapshot.
- `clipboard-guard.test.ts` must still pass unchanged.

### Integration sub-task (sequential, after A, B, C)

Owns:
- `app/src-tauri/src/{lib.rs, net_cmd.rs, identity_cmd.rs}`
- `README.md`, `AGENTS.md`
- root `package.json` (optional convenience script only)

1. `lib.rs` `setup`:
   - `let profile = profile::profile_from(std::env::args().skip(1), std::env::var("MESSENGER_PROFILE").ok())?;`
   - `let layout = profile::layout(&app.path().app_data_dir()?, profile);`
   - Create `layout.data_dir` with `0o700` (unix), the same as today.
   - `OsKeychain::native_for(layout.profile.as_ref())?`.
   - Use `layout.data_dir` everywhere the old `data_dir` was used: settings, marker, `known_peers.json`.
   - Set the window title with `profile::window_title(..)` (main window `set_title`).
   - Remove nothing else that Track B added to `lib.rs` (`mod profile;`).
2. `net_cmd.rs` adds plain functions + wrappers:
   - `net_presence_get() -> PresenceDto`
   - `net_presence_set(enabled: bool) -> PresenceDto`
   - `net_lan_peers() -> Vec<LanPeerDto>`

   Forward `NetEvent::LanPeersChanged` as `app.emit("lan_peers_changed", Vec<LanPeerDto>)`. Settings failure → `SettingsUnavailable`.
3. Relay override:
   - Node start reads `get_relay_url(account)` for `RelayChoice`.
   - Writer: `MESSENGER_RELAY_URL` at launch, for testers and self-hosters, documented in README. There is no GUI editor in v1.
   - Parsing is a pure function in `net_cmd.rs`: `pub fn relay_override_from_env(raw: Option<String>) -> Result<RelayOverride, RelayEnvError>` with `RelayOverride { Keep, Clear, Set(String) }`. Unset → `Keep`. The literal value `default` → `Clear`. A valid URL → `Set` (same validation as `set_relay_url`). Anything else → `Err`.
   - `setup` calls it eagerly, so an invalid value is a **startup error**, never ignored. The parsed result is stored in `NetState`.
   - It is **applied inside `ensure_started`**, before `Node::start`: `set_relay_url(account_id, …)` for `Set`/`Clear`. On a first run there is no `account_id` until provisioning, and `ensure_started` is the first point where both the account and the settings exist (it runs from `setup` when Present and from `identity_provision`).
4. The `identity_set_display_name` wrapper calls `node.refresh_local_name()` (if the node is running) after a successful save. Factor it as a plain function `set_display_name_and_refresh(&service, &NetState, name)` so it can be tested.
5. Command tests (`net_cmd.rs` / `identity_cmd.rs`):
   - `relay_env_unset_keeps`, `relay_env_valid_is_set_and_stored_on_start`, `relay_env_default_clears_stored_value`, `relay_env_invalid_is_startup_error` (pure fn returns `Err`; `setup`-level helper propagates it)
   - `relay_override_applied_only_once_account_exists`: with no identity, `ensure_started_with` is not invoked and nothing is written. After provisioning, the value is stored under the new `account_id`.
   - `rename_refreshes_lan_name`: via `set_display_name_and_refresh` on a `MemoryLan` test node, the peer observes the new name. With presence OFF, nothing is published.
   - `presence_get_defaults_on_and_set_persists`
   - `lan_peers_dto_is_unverified_and_has_parseable_ticket`, using a `MemoryLan` test node
   - `presence_set_failure_maps_to_settings_unavailable`
   - `lan_and_ticket_paths_share_one_trust_gate_end_to_end`: driving two in-process nodes only via `net_cmd` functions, a LAN pick → prompt → accept → connected; then presence OFF → the peer leaves B's `net_lan_peers()` → B still connects by ticket.
6. README:
   - Workspace/feature text: LAN discovery + presence.
   - New section **"Two-device simulation"**:
     - **Level 1**: `cargo test --workspace` (runs in pre-commit), plus `cargo test -p core --test discovery_lan -- --ignored` for real mDNS.
     - **Level 2**: exact commands to launch two instances. Verify them and write down the ones that work. Running `tauri dev` twice clashes on Vite port 1420, so for example: instance A `MESSENGER_PROFILE=a npm start`; instance B runs the built debug binary with `MESSENGER_PROFILE=b` against the same dev server, or use `npm run tauri build -- --debug` and launch the bundle twice.
     - Note: same-host mDNS discovery needs a **multicast-capable, non-loopback** interface (normal Wi-Fi/Ethernet). On a machine with only loopback, or with multicast blocked by a VPN or firewall, the LAN list may stay empty; use the ticket path instead.
     - Then: first-run each, LAN list or ticket paste, Accept prompt, Connected state, presence OFF → disappears from the other's list but still connectable by ticket.
     - State that the profile override only relocates state and never disables any control.
   - Extend the manual GUI checklist accordingly.
   - Document the relay override and the relay metadata limitation: relays see NodeIds, addresses and timing, never plaintext; self-hosting is the mitigation.
   - Document that presence OFF hides you from the **LAN** only; internet address lookup (iroh's publishing) still works for ticket holders.
   - AGENTS.md: mention the `--profile`/`MESSENGER_PROFILE` affordance and that it must never weaken controls.

## Quality gates (all must pass before commit)

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
git commit   # runs .githooks/pre-commit; NEVER --no-verify
```

Also run the ignored real-mDNS test once on a machine with a normal LAN interface and note the result in the PR.

## Guardrails (AGENTS.md, enforced; do not weaken)

- `SecretKeyBytes` / `DbKey` never cross into `app/`, not even in types or comments. The profile code in `app/` deals with names and paths only.
- Clipboard (`writeText` / `clipboard`) only in `app/src/views/MyInfo.svelte` (and tests). LanPeers/PresenceToggle never copy.
- Logs never contain message plaintext, display names, private keys or secrets. NodeIds and addresses are fine.
- `core` keeps `#![forbid(unsafe_code)]`.
- Only real-content files. `MemoryLan` is gated test-support code, like `FakeKeychain`.
- Wire format: no `Envelope` change in this step. mDNS user data is not an `Envelope` frame. `PROTOCOL_VERSION` stays 1. State this in the PR.
- Data namespaced by `account_id`: presence/relay live in the per-account settings entry. LAN peers are never persisted.
- Never write credentials, keys, tokens or secrets in code, tests, fixtures, Dockerfiles, compose files or docs. All credentials are provided/generated at runtime (keychain, env at container start, or generated test keys).

## Security (threats specific to this step: prevent and verify)

| Threat | Prevention | How to check |
|---|---|---|
| Presence OFF still advertising (including at startup) | Persisted state is applied before the first advertise. `set_presence` persists then applies. There is no other call to `set_advertise(true)`. | `presence_off_hides_a...`, `presence_defaults_on_and_persists_across_restart` (bus never saw A). `grep -n "set_advertise" core/src` → only in presence code. Manual Level-2 check + Docker smoke (02-03). |
| "Invisible" misread as "unreachable" | UI copy states that ticket holders can still connect. README documents that internet lookup still publishes. | `PresenceToggle.test.ts` copy assertion. Manual review. |
| mDNS record parsing panics / memory blow-up | Bounded user data, re-validated names, skip-on-error, `MAX_LAN_PEERS`, debounced events. | `malformed_and_oversized_mdns_records_never_panic`. `grep -n "unwrap()\|expect(" core/src/discovery/lan.rs` outside tests → none. |
| Self-asserted mDNS name used for trust/dedup | Registry keyed by NodeId. The LAN Connect goes through `connect_ticket` → the 02-01 gate. | `same_name_two_node_ids_stay_distinct`, `lan_pick_uses_the_same_trust_gate`. Vitest same-name test. |
| LAN peers persisted (privacy) | Memory only. Only Accept/dial writes `known_peers.json`. | `lan_peers_are_never_persisted`. |
| Profile path traversal / keychain namespace collision | Strict `[a-z0-9-]{1,32}` + lowercase + reserved `default`. Service = `…identity.profile.<name>`. Default profile unchanged. | Profile unit tests (`..`, `/`, `\`, NUL, case). `profile_isolation.rs`. |
| Invalid profile silently falling back to the default identity | `profile_from` returns `Err`, and setup aborts with a message. | App unit test "invalid → Err". Manual: `MESSENGER_PROFILE=../x` → app refuses to start. |
| Profile override weakening controls | The override only changes `data_dir` + keychain service. The same `Node`, gate and OS keychain are used. No flag skips the prompt. | `lan_and_ticket_paths_share_one_trust_gate_end_to_end` run with a profile layout. `grep -rniE "skip_trust\|no_prompt\|insecure" app/src-tauri/src core/src` → none outside tests. |
| Self-chosen name leaked to public internet infrastructure (pkarr/DNS) or kept published while presence is OFF | The name is never in the endpoint-wide `EndpointData` user data. It lives only in the mDNS record and is withdrawn when OFF. | `internet_published_record_has_no_name`, `presence_off_withdraws_name_from_mdns_record`. `grep -n "set_user_data_for_address_lookup" core/src` → none, or only with `None`. |
| Relay override pointing at a plaintext/malicious relay | The relay sees only QUIC ciphertext (02-01 canary test). The URL is validated (https, or http only for loopback). Invalid → startup error. | Settings relay URL tests. 02-01 `discovery_relay.rs` still passes. |

## Definition of Done

- [ ] LAN peers are discovered via mDNS and surfaced live (`lan_peers_changed`, `net_lan_peers`). Each is marked unverified, with NodeId + self-asserted name. Same-name peers stay distinct.
- [ ] Presence defaults ON, persists per `account_id`, and when OFF verifiably stops advertisement (tested) while browsing continues and ticket holders can still connect. The GUI copy explains "invisible ≠ unreachable".
- [ ] The LAN pick and the pasted ticket both go through the single 02-01 trust gate.
- [ ] The self-chosen name is carried only in the mDNS record: never in internet-published data, and withdrawn while presence is OFF (tested).
- [ ] Relay override is persisted in settings, validated, applied in `ensure_started` once an account exists, and documented (tested: unset/valid/`default`/invalid). The default remains the iroh mesh.
- [ ] Renaming re-publishes the LAN name (tested). The window title shows the profile suffix only when a profile is active (tested).
- [ ] `--profile` / `MESSENGER_PROFILE` isolates data dir + keychain namespace. It is validated and fails closed, the default profile is unchanged, and it is documented as a dev/test affordance.
- [ ] Network tab (reachable from MainScreen) contains presence toggle, LAN peers, connect by ticket and connection state. The prompt stays global.
- [ ] All listed tests exist and pass. The ignored real-mDNS test was run manually once. All quality gates + the pre-commit hook pass (no `--no-verify`).
- [ ] README (Level 1 + Level 2 run steps, checklist, relay/presence notes) and AGENTS.md are updated. The PR states "wire format unchanged".
