# Step 02-01: Live endpoint, tickets, trust gate, and connect-by-ticket UI

**Design doc:** [DD-02: Transport, Discovery & Connection](../02-transport-discovery-and-connection.md)
**Order:** 1 of 4. Do this first. Steps 02-02, 02-03 and 02-04 depend on it and run one after another.
**Branch suggestion:** `step-02-01-endpoint-tickets-trust-gate`

## Goal

Add the `core::discovery` slice: one long-lived iroh endpoint bound to the DD-01 identity key and the `protocol::ALPN`. Add these pieces with it:

- tickets produced from the live endpoint and parsed from pasted text,
- dialing a ticket, with an accept loop on the other side,
- the `Hello` exchange,
- the NodeId-based trust gate (Accept/Decline for unknown peers, silent connect for known ones),
- the inbound-flood guard,
- connection events,
- the Level-1 in-process two-node harness.

Wire it through the GUI end to end for the **ticket path**:

- My Info shows a ticket built from the live endpoint (`net_my_ticket`).
- A new **Network** tab has a "connect by ticket" field and a connection-state list.
- A global Accept/Decline prompt (`incoming_request`) appears whichever tab is open.

When this step is done, a tester can already connect two machines by pasting a ticket. LAN/mDNS, presence and the profile override come in 02-02.

Decisions this step must follow (already made with the user):

1. DD-03 Contacts does not exist yet. `core::discovery` defines and uses a `KnownPeers` trait (`is_known(&NodeId)`, `add(NodeId, display_name)`). This step ships a **minimal persisted** implementation: an "accepted NodeIds" store namespaced by `account_id`, so a known peer reconnects silently across restarts. DD-03 later replaces or extends it. A self-asserted name is never used as a key.
2. DD-04 Messaging does not exist yet. In DD-02 the "round-trip" means both sides exchange `protocol::Hello` frames (postcard `Envelope`) over the accepted channel, then surface a "connected" state. There is no Chat UI. An unknown peer gets no traffic past `Hello` until the user Accepts.

## Current code you build on (read first)

- `protocol/src/lib.rs`: `ALPN = b"p2p-messenger/1"`, `PROTOCOL_VERSION`, `Envelope::{new, encode, decode, is_supported}`, `Payload::Hello(Hello { display_name })`.
- `core/src/identity/ticket.rs`: `Ticket` (`p2pm:<nodeid>`), `ParsedTicket { node_id }`, the `TicketProvider` trait, `NodeIdTicketProvider` (the placeholder that DD-02 replaces), `ticket_to_qr_svg`. `NodeId` is `iroh_base::PublicKey`.
- `core/src/identity/service.rs`: `IdentityService<K, S, T>` with private `inspect()`, which loads the identity key from the keychain and drops it right away. `my_info()` calls `tickets.current_ticket(&node_id)`.
- `core/src/identity/settings.rs`: `SettingsStore` trait, `MemorySettingsStore`, and `FileSettingsStore`, a hardened JSON file with `deny_unknown_fields`, a size cap, symlink refusal, atomic 0600 writes. The `known_peers` store copies these hardening patterns.
- `core/src/identity/keychain.rs`: the `KeychainStore` boundary, plus `testing::FakeKeychain` behind `cfg(any(test, feature = "test-support"))`.
- `app/src-tauri/src/lib.rs` + `identity_cmd.rs`: plain functions generic over the boundaries, with thin `#[tauri::command]` wrappers. `AppIdentity = IdentityService<OsKeychain, FileSettingsStore, NodeIdTicketProvider>`.
- `app/src/lib/router.ts` (`tabs`, `TabId`), `app/src/views/MainScreen.svelte` (tab bar), `app/src/views/MyInfo.svelte` (+ `MyInfo.test.ts`), `app/src/clipboard-guard.test.ts`. The guard requires `copy(info.ticket,` and `copy(info.node_id,` in MyInfo.

## Dependencies and versions (verify; do not invent APIs)

The workspace already pins `iroh-base = "1"` (1.4.x in `Cargo.lock`, `key` feature). Use the **iroh 1.x** line that resolves to the same `iroh-base` 1.x. If `cargo add` would pull a second, incompatible `iroh-base`, align the workspace pin instead of duplicating it. In iroh 1.x the old `NodeId`/`NodeAddr` names became `EndpointId`/`EndpointAddr`; `EndpointId` is a type alias of `PublicKey`, so the existing `identity::NodeId` re-export stays valid.

Add to root `[workspace.dependencies]` and use with `.workspace = true`:

- `iroh = "1"`: `Endpoint::builder(presets::…)`, `.secret_key(..)`, `.alpns(vec![ALPN.to_vec()])`, `.relay_mode(RelayMode::{Default, Disabled, Custom(..)})`, `.bind().await`, `endpoint.accept()`, `incoming.accept()?.await`, `conn.remote_id()`, `conn.open_bi()` / `accept_bi()`, `recv.read_to_end(limit)`, `send.finish()`, `conn.close(code, reason)`, `endpoint.addr()` / `watch_addr()`, `endpoint.close().await`. Confirm each name against docs.rs for the exact version you lock. Context7 `/websites/rs_iroh` shows these for iroh 1.x.
- `iroh-tickets` (the version compatible with iroh 1.x / iroh-base 1.x; verify): `iroh_tickets::endpoint::EndpointTicket::new(addr)`, its `Display`/`FromStr` string form, `.endpoint_addr()`.
- `tokio` with `rt-multi-thread`, `macros`, `sync`, `time`. iroh needs a tokio runtime. Tauri's `tauri::async_runtime` is tokio, so core tasks spawned from it work.
- Optional: `tracing`, for debug-level lifecycle events carrying **NodeIds only**. DD-05 owns the subscriber and log files; this step installs none.
- Dev-only, for the relay-fallback test: iroh's test relay helper (`iroh` `test-utils` feature / `iroh_relay::server` behind its `server` feature; verify which exists for your version). It must stay in `[dev-dependencies]` or behind `test-support`. Never put it in normal deps.

MSRV: `iroh-base` 1.4.1 already needs Rust 1.91 and `keyring` 4.2 needs 1.88, yet the workspace still declares `rust-version = "1.80"`. Raise it **unconditionally** in the foundation sub-task:

1. After the new deps resolve, find the highest `rust-version` among locked dependencies, e.g. `cargo metadata --format-version 1 | jq -r '.packages[].rust_version // empty' | sort -V | tail -1`.
2. Set the workspace `rust-version` to that value (at least 1.91).
3. Check it with `cargo +<that version> build` if that toolchain is installed.
4. Update AGENTS.md ("MSRV …") and README in the same change.

The 02-03 Dockerfile's rust image tag follows this value.

## Interface contract (agree before splitting into tracks)

All tracks code against these signatures. **The integration sub-task owns this contract.** A track that needs a change records it and leaves it to integration; it must not change a shared signature in its own worktree.

```rust
// core/src/discovery/mod.rs: curated re-exports only
pub use dto::{event_json, ConnectionStateDto, IncomingRequestDto, MyTicketDto};
pub use events::{ConnectionState, Direction, FailureReason, IncomingRequest, NetEvent};
pub use flood::{Admission, FloodGuard, GateLimits};
pub use known_peers::{FileKnownPeers, KnownPeers, KnownPeersError};
pub use node::{ConnectError, Node, NodeConfig, NodeDeps, RelayChoice, RespondError, StartError};
pub use ticket_provider::LiveTicketProvider;
#[cfg(any(test, feature = "test-support"))]
pub mod testing; // MemoryKnownPeers, spawn_test_node, raw attacker dialer

// known_peers.rs (trait: foundation; FileKnownPeers: Track A)
pub trait KnownPeers: Send + Sync + 'static {
    /// Trust check. Keyed on NodeId only. An Err must be treated as "unknown" (fail closed).
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError>;
    /// Records an accepted/dialed peer. `display_name` is the last-seen self-asserted
    /// name: advisory only, never a key, never used for lookup or dedup.
    fn add(&self, id: NodeId, display_name: Option<DisplayName>) -> Result<(), KnownPeersError>;
}
pub struct FileKnownPeers { /* <data_dir>/known_peers.json, scoped to one AccountId */ }
impl FileKnownPeers { pub fn new(data_dir: impl Into<PathBuf>, account: AccountId) -> Self; }

// flood.rs (foundation): pure, clock injected, no I/O
pub struct GateLimits {
    pub max_pending: usize,            // default 5   (concurrent Accept/Decline prompts, global)
    pub max_attempts_per_peer: u32,    // default 3   (inbound attempts per NodeId per window)
    pub attempt_window: Duration,      // default 60s
    pub prompt_ttl: Duration,          // default 120s (pending prompt auto-declines)
    pub hello_timeout: Duration,       // default 10s
    pub max_handshakes: usize,         // default 32  (concurrent pre-Hello connections)
    pub max_tracked_peers: usize,      // default 1024 (bounded attempt table; evict oldest)
}
pub enum Admission { Prompt, Drop }
impl FloodGuard {
    pub fn new(limits: GateLimits) -> Self;
    pub fn admit(&mut self, peer: NodeId, now: Instant) -> Admission; // records the attempt
    pub fn resolve(&mut self, peer: &NodeId);                         // prompt answered/closed
    pub fn expire(&mut self, now: Instant) -> Vec<NodeId>;            // prompts past ttl
    pub fn pending_len(&self) -> usize;
}

// events.rs + dto.rs (foundation)
pub enum NetEvent {
    IncomingRequest(IncomingRequest),              // -> Tauri event "incoming_request"
    ConnectionState { node_id: NodeId, direction: Direction, state: ConnectionState }, // -> "connection_state"
    MyTicketChanged,                               // -> "my_ticket_changed" (no payload; UI re-fetches net_my_ticket)
    // 02-02 adds: LanPeersChanged(Vec<LanPeer>)  -> "lan_peers_changed"
}
pub struct IncomingRequest { pub node_id: NodeId, pub display_name: Option<DisplayName> } // name UNVERIFIED
/// Who opened the connection. Inbound = the peer dialed us (trust gate side); Outbound = we dialed.
pub enum Direction { Inbound, Outbound }
pub enum ConnectionState { Connecting, Connected, Disconnected, Failed(FailureReason) }
pub enum FailureReason { Declined, Timeout, Unreachable, Protocol, Expired, Closed }

// Serializable, string-only DTOs shared by the Tauri shell and (02-03) the headless node,
// so both emit byte-identical JSON. Plain serde: core still knows nothing about Tauri.
#[derive(Serialize)] pub struct MyTicketDto { pub ticket: String, pub qr_svg: String }
#[derive(Serialize)] pub struct IncomingRequestDto { pub node_id: String, pub display_name: Option<String> }
#[derive(Serialize)] pub struct ConnectionStateDto {
    pub node_id: String,
    pub direction: &'static str,      // "inbound" | "outbound"
    pub state: &'static str,          // "connecting" | "connected" | "disconnected" | "failed"
    pub reason: Option<&'static str>, // "declined" | "timeout" | "unreachable" | "protocol" | "expired" | "closed"
}
impl From<&IncomingRequest> for IncomingRequestDto { .. }
impl ConnectionStateDto { pub fn new(node_id: &NodeId, direction: &Direction, state: &ConnectionState) -> Self { .. } }

/// THE single mapping from a core event to its external event name + JSON payload.
/// Used by the Tauri shell (`app.emit(name, payload)`) and by the 02-03 headless node's
/// stdout lines, so names and shapes can never drift from the DD-02 API table.
pub fn event_json(ev: &NetEvent) -> (&'static str, serde_json::Value);
// "incoming_request" -> IncomingRequestDto, "connection_state" -> ConnectionStateDto,
// "my_ticket_changed" -> null. (02-02 adds "lan_peers_changed" -> Vec<LanPeerDto>.)

// node.rs (Track B)
pub enum RelayChoice { Default /* iroh n0 mesh */, Disabled, Custom(RelayUrl) }
pub struct NodeConfig { pub relay: RelayChoice, pub bind_addr: Option<SocketAddr>, pub limits: GateLimits }
impl NodeConfig {
    pub fn desktop() -> Self;                         // RelayChoice::Default, iroh N0 preset (hole punching + relay)
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback() -> Self;                        // relay disabled, bind 127.0.0.1:0, no internet lookup
    #[cfg(any(test, feature = "test-support"))]
    pub fn relay_only(relay: RelayUrl) -> Self;       // custom relay + IP transports cleared (relay test)
}
pub struct NodeDeps {
    pub settings: Arc<dyn SettingsStore + Send + Sync>, // reads own display name for Hello at send time
    pub known: Arc<dyn KnownPeers>,
    pub tickets: LiveTicketProvider,                   // node attaches itself on start
}
impl Node {
    pub async fn start<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
        identity: &IdentityService<K, S, T>, deps: NodeDeps, config: NodeConfig,
    ) -> Result<Node, StartError>;
    pub fn node_id(&self) -> NodeId;
    pub fn ticket(&self) -> Result<Ticket, TicketError>;                    // from endpoint.addr(), right now
    /// Waits (bounded by `timeout`, default 5 s) until the endpoint is online — home relay
    /// connected and/or direct addrs known (verify the iroh 1.x API, e.g. `endpoint.online()`
    /// or the first non-empty `watch_addr()` value) — then returns `ticket()`. On timeout it
    /// returns the best ticket available now; it never fails just because the network is slow.
    pub async fn ticket_when_ready(&self, timeout: Duration) -> Result<Ticket, TicketError>;
    pub fn connect_ticket(&self, input: &str) -> Result<NodeId, ConnectError>; // validates, spawns dial, returns at once
    pub fn respond_incoming(&self, peer: &NodeId, accept: bool) -> Result<(), RespondError>;
    pub fn pending_incoming(&self) -> Vec<IncomingRequest>; // snapshot of open prompts (UI seeding after mount/reload)
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<NetEvent>;
    pub fn connected_peers(&self) -> Vec<NodeId>;   // authorized channels only
    pub async fn shutdown(self);
}
pub enum ConnectError { InvalidTicket, SelfDial, ShuttingDown }
pub enum RespondError { NoSuchRequest }
```

Ticket API (Track A, in the existing `core/src/identity/ticket.rs`):

```rust
pub const MAX_TICKET_LEN: usize = 4096;      // checked BEFORE any decoding
pub struct ParsedTicket { pub node_id: NodeId, pub addr: EndpointAddr }
impl Ticket {
    pub fn parse(input: &str) -> Result<ParsedTicket, TicketError>; // trims ASCII whitespace
    pub fn from_endpoint_addr(addr: &EndpointAddr) -> Result<Ticket, TicketError>;
}
pub enum TicketError { Malformed, TooLarge, Qr(String) }
```

Format: keep the `p2pm:` prefix. The body is the iroh-tickets `EndpointTicket` string. The legacy body `p2pm:<nodeid>` (written by DD-01) must still parse, as a NodeId-only `EndpointAddr`. Reject when the input is longer than `MAX_TICKET_LEN`, has more than 16 transport addrs, or has more than 4 relay URLs. Rejection returns `Malformed`/`TooLarge` and never panics. Producing a ticket trims direct addrs to at most 8, so the ticket stays under `MAX_TICKET_LEN` and still renders as a QR code. A ticket is not an `Envelope` frame, so `PROTOCOL_VERSION` is unaffected.

### Connection protocol (Hello exchange), shared by both sides

1. The dialer connects with `ALPN`, opens **one** bi stream, and writes a single `Envelope::new(Payload::Hello(Hello{ display_name }))` encoded with postcard. Then it calls `finish()`. The display name comes from `settings.get_display_name(account)`; if none is set, it sends an empty string.
2. The acceptor checks the handshake budget (`max_handshakes`). It runs `accept_bi()` and `read_to_end(MAX_HELLO_BYTES)` inside `hello_timeout`. `MAX_HELLO_BYTES = 1024` goes in `protocol` as an additive `pub const`, so there is no version bump. It decodes the frame and requires `is_supported()` and `Payload::Hello`. The name goes through `identity::validate_display_name`. An invalid or empty name becomes `None`, never an error that leaks out. Any other first frame, oversize, timeout or decode error means: close with `CLOSE_PROTOCOL`, no event, no prompt.
3. Trust decision, keyed on `conn.remote_id()`, the transport-verified NodeId (**never** the Hello name):
   - `known.is_known(&id) == Ok(true)` → write our Hello reply, `finish()`, record the authorized connection, emit `ConnectionState::Connected`.
   - unknown, or `is_known` errored (fail closed) → `flood.admit(id, now)`:
     - `Drop` → close with `CLOSE_BUSY`. Silent: no event, no prompt, and no log above `trace`.
     - `Prompt` → hold the connection as pending and emit exactly one `NetEvent::IncomingRequest`. Then wait for `respond_incoming` or `prompt_ttl`.
       - Accept → `known.add(id, name)`, reply with Hello, mark authorized, emit `Connected`.
       - Decline → `conn.close(CLOSE_DECLINED)`. Nothing is persisted. Emit `Failed(Declined)` locally so the UI clears the prompt.
       - TTL → treat as Decline, emit `Failed(Expired)`.
4. The dialer reads the peer's Hello reply (`read_to_end(MAX_HELLO_BYTES)`, timeout `prompt_ttl + 10s`), runs the same validation, then calls `known.add(peer, name)`. The user deliberately dialed this NodeId; this is the interim DD-02 rule, and DD-03 will add naming at that point. The add happens **only after** a valid Hello reply has been received. It never happens on dial start, on a decline/timeout/unreachable result, or on an invalid reply, and a test asserts this. The UI labels the action "Connect and remember this device", and the README states the rule. It then emits `Connected`. If the connection closes with `CLOSE_DECLINED` → `Failed(Declined)`. Unreachable → `Failed(Unreachable)`. Timeout → `Failed(Timeout)`.
5. While pending, the acceptor never calls `accept_bi`/`accept_uni` again and never sends anything. A pending or declined peer has no path to any later layer. Only authorized connections enter `connected_peers()`, and DD-04 will consume only that set.
6. For every authorized connection, watch `conn.closed()` and emit `Disconnected`. If a known peer reconnects, the new connection replaces the old one, which is closed.
7. `connect_ticket` refuses its own NodeId (`SelfDial`).
8. Every `ConnectionState` event carries its `Direction`. On the **acceptor** side, a Decline or TTL expiry emits `Failed(Declined|Expired)` with `Direction::Inbound` **only** so the UI can clear the prompt. DD-02 says declined/pending peers carry "no persistence and no history", so the UI must not keep these in any list (see Track C), and core keeps no record of them after the event.

Live ticket freshness: a background task watches the endpoint's address (`watch_addr()` or equivalent; verify) and emits `NetEvent::MyTicketChanged` when the relay URL or direct-address set changes, debounced to at most one event per 2 s. This keeps My Info correct after start-up and after a network change.

Close codes are `pub const` in `node.rs`: `CLOSE_DECLINED = 1`, `CLOSE_PROTOCOL = 2`, `CLOSE_BUSY = 3`.

The identity key never leaves `core`. Add `pub(crate) fn load_identity_secret(&self) -> Result<(AccountId, SecretKeyBytes), LoadError>` to `IdentityService`, reusing `inspect()`'s path. `Node::start` converts the key to `iroh_base::SecretKey`, passes it to the builder, and drops the `SecretKeyBytes` right away. Neither the app nor the DTOs ever see a key type.

`LiveTicketProvider` (Track B): a `Clone` handle (`Arc<RwLock<Option<…endpoint handle…>>>`) that implements `identity::TicketProvider`. Once a node has attached, it returns `Ticket::from_endpoint_addr(&endpoint.addr())`. Before that, it returns the legacy NodeId-only ticket. The app's `IdentityService` uses it as `T`, so `identity_my_info` and `net_my_ticket` always agree.

## Work breakdown

### Foundation sub-task (sequential, before tracks A/B/C)

This small commit lets the three tracks build independently. Every file it adds carries real content: no stubs, no empty modules.

Owns:
- root `Cargo.toml`: `[workspace.dependencies]` for `iroh`, `iroh-tickets`, `tokio`, optional `tracing`; `iroh-base` features aligned (`key`, plus whatever `iroh-tickets` needs); `rust-version` raised as described under MSRV.
- `core/Cargo.toml`: those deps, `serde_json.workspace = true` (normal dep: `event_json` builds JSON payloads), `tempfile` as optional, `test-support = ["dep:tempfile"]`, and the dev-only relay test helper.
- `protocol/src/lib.rs`: `pub const MAX_HELLO_BYTES: usize = 1024;` with its doc, plus a unit test that a max-length valid `Hello` encodes within it.
- `core/src/lib.rs`: `pub mod discovery;`.
- `core/src/discovery/mod.rs`: the slice doc comment and `mod events; mod dto;` with their re-exports. It also has two marked regions, `// ── track A modules ──` and `// ── track B modules ──`. Each track adds its own `mod …;` / `pub use …;` lines **only inside its own region**, so the merges are purely additive and never conflict. Integration removes the markers.
- `core/src/discovery/events.rs` and `core/src/discovery/dto.rs`: the full event and DTO types from the contract, plus `event_json`, with unit tests for the exact JSON shape (`direction`/`state`/`reason` strings, `display_name: null`) and `event_names_match_dd02_api_table`: every `NetEvent` variant maps to exactly one of `"incoming_request"`, `"connection_state"`, `"my_ticket_changed"` (02-02 extends the test with `"lan_peers_changed"`). Use an exhaustive `match` so adding a variant without a name fails to compile.
- `core/src/discovery/flood.rs`: the full `FloodGuard` and its unit tests, as specified under Track A below. It is pure and small, and Track B's gate needs it.
- `core/src/discovery/known_peers.rs`, **trait part only**: the `KnownPeers` trait and `KnownPeersError`. That is real content: the trust seam with its fail-closed doc. Track A adds `FileKnownPeers` to this file.
- `AGENTS.md` and `README.md`: MSRV line only.

Gates: `cargo fmt`, `clippy -D warnings`, `cargo test --workspace` and `cargo build` pass on this commit alone. Tracks branch from it.

### Parallel track A: pure core building blocks (no networking)

Owns **only** these files:
- `core/src/identity/ticket.rs` (new format, bounds, `from_endpoint_addr`, tests)
- `core/src/discovery/known_peers.rs` (adds `FileKnownPeers` below the foundation's trait)
- the "track A modules" region of `core/src/discovery/mod.rs` (`FileKnownPeers` re-export)

(`flood.rs` is written in the foundation sub-task using the flood spec and tests below. Track A does not touch it.)

Instructions:
- **known_peers.rs:** `FileKnownPeers` stores `<data_dir>/known_peers.json`:

  ```json
  {"version":1,"accounts":{"<account uuid>":{"<node id string>":{"name":"Ada","added_at_ms":1730000000000}}}}
  ```

  - Copy `FileSettingsStore`'s hardening: `deny_unknown_fields`, a 256 KiB read cap, `symlink_metadata` refusing non-files, an atomic tmp+rename write with mode 0600, and corrupt/unknown version = error, never "empty".
  - Map keys are parsed back as `NodeId`. Invalid keys make the file corrupt. Names are re-validated with `validate_display_name` on read.
  - Cap entries per account at 10 000 (`KnownPeersError::Full`).
  - Module doc: this is the **interim** plaintext store (same documented deviation as `FileSettingsStore`). It holds public NodeIds and advisory names only. DD-03 replaces it with the encrypted `contacts` table.
  - Put `MemoryKnownPeers` in `core/src/discovery/testing.rs`. Track B owns that file, so hand the struct over in integration, or have Track B add it.
- **flood.rs:** pure state machine with the semantics in the contract. A peer that already has a pending prompt → `Drop`. Attempt timestamps are pruned per window. When the table exceeds `max_tracked_peers`, the oldest entries are evicted, so NodeId rotation cannot grow memory without bound. All decisions take `now: Instant`. Never sleep or read the clock inside.
- **ticket.rs:** implement the contract above. Keep `NodeIdTicketProvider`: it is the fallback inside `LiveTicketProvider` and is used by existing tests. Update the module doc, which still says "placeholder... until DD-02".

Tests (in-module `#[cfg(test)]`):
- ticket: round-trip with relay URL + 2 IP addrs. Legacy `p2pm:<nodeid>` parses. Garbage, truncated, wrong prefix, empty, whitespace-only → `Malformed`. `MAX_TICKET_LEN + 1` → `TooLarge`. 17 addrs → `Malformed`. A fuzz loop of 10 000 random byte strings, lossy-UTF-8 and truncated real tickets, never panics. A max-size produced ticket still renders a QR (`ticket_to_qr_svg`). No secret bytes appear in a ticket (keep the existing test).
- known_peers: add/is_known. A different NodeId is unknown. Survives reopen. Account isolation: same NodeId under account A ≠ B. Two NodeIds with the same name are both stored and distinct. A name change does not create a second entry. Corrupt JSON / wrong version / unknown field → error, and the file is not overwritten. Symlinked file → error. File mode is 0600 (unix). No temp file is left behind.
- flood: one pending per peer. The `max_pending` global cap holds. `max_attempts_per_peer` per window, and the window slides (old attempts expire). `expire()` returns stale prompts. `resolve()` frees a slot. Tracked-peer eviction keeps the table bounded.

### Parallel track B: transport, gate and Level-1 harness

Owns **only** these files:
- `core/src/discovery/{node.rs, hello.rs, ticket_provider.rs, testing.rs}`
- the "track B modules" region of `core/src/discovery/mod.rs`
- `core/src/identity/service.rs` (`load_identity_secret` only)
- `core/src/identity/settings.rs` (`impl<T: SettingsStore + ?Sized> SettingsStore for Arc<T>` only)
- `core/tests/discovery_two_nodes.rs`, `core/tests/discovery_relay.rs`
- `core/tests/identity_integration.rs` (extend the ticket test it marks "must be extended in DD-02")

The `KnownPeers` trait and `FloodGuard` already exist (from the foundation), so Track B compiles on its own. It tests against `testing::MemoryKnownPeers`. The `known_peer_reconnects_silently_across_restart` test needs Track A's `FileKnownPeers`; write it in Track B and enable it during integration (with no `#[ignore]` left behind), or rebase B onto A once A merges.

Instructions:
- Build the endpoint once in `Node::start`:
  - `desktop()` → iroh's N0 preset (default relay mesh + internet address lookup, giving hole punching with relay fallback).
  - `loopback()` → minimal preset, `RelayMode::Disabled`, bound to `127.0.0.1:0`.
  - Always `.alpns(vec![protocol::ALPN.to_vec()])`.
- `test-support` feature: add `tempfile` as an optional normal dependency (`test-support = ["dep:tempfile"]`) so `testing::spawn_test_node()` can create isolated dirs.
- `testing.rs` (test-support):
  - `MemoryKnownPeers`.
  - `TestNode { node, identity, known: Arc<MemoryKnownPeers or FileKnownPeers>, dir: TempDir }`.
  - `spawn_test_node(name) -> TestNode`: provisions with `FakeKeychain` + `MemorySettingsStore`, sets the display name, `NodeConfig::loopback()`.
  - `restart(TestNode) -> TestNode`, which keeps the same keychain/dir/known store (for "across restarts").
  - `raw_dial(addr, first_frame_bytes)`: a bare iroh endpoint that sends arbitrary bytes, used by resilience tests.
  - `next_event(rx, timeout)`.
  - `MemoryAddressLookup`: an in-process implementation of iroh's address-lookup trait (verify the 1.x trait name and methods, e.g. `AddressLookup` with `publish`/`resolve`). Nodes sharing one instance publish their `EndpointAddr` to it and resolve each other by NodeId. It stands in for iroh's internet lookup in tests, so the "dial by cached NodeId" path is tested without the network. `NodeConfig` gets a test-support-only field (or `loopback_with_lookup(lookup)` constructor) to register it.
  - Everything here is a double over the **same** `Node` code paths. There is no alternate gate.
- `hello.rs`: `encode_hello(name) -> Vec<u8>`, `decode_hello(bytes) -> Result<Option<DisplayName>, HelloError>`, both with size checks. Unit tests: oversize, wrong version, Chat payload, invalid name → `Ok(None)`, valid round-trip.
- Event fan-out uses `tokio::sync::broadcast` (capacity 256). A lagging receiver must not block the accept loop.
- Logging (if you add `tracing`): NodeIds, close codes and reason classes only. Never Hello names, ticket strings, or frame bytes.

Integration tests: `core/tests/discovery_two_nodes.rs`, `#[tokio::test(flavor = "multi_thread")]`, every wait bounded with a timeout. These map to the DD-02 Testing bullets:
- `ticket_round_trip_yields_live_channel`: A.ticket() → B.connect_ticket. A gets exactly one `IncomingRequest` with B's NodeId and name. A accepts → both emit `Connected`. Each observes the other's Hello name. A's known store now contains B, and B's contains A.
- `corrupted_ticket_is_handled_and_endpoint_stays_up`: truncated, garbage and oversized tickets → `Err(InvalidTicket)`. A valid connect then still succeeds.
- `unknown_peer_raises_exactly_one_prompt_and_gets_no_traffic_before_accept`: before respond, `a.connected_peers()` is empty, B has no `Connected`, and B's Hello-reply read is still pending. After Accept, both are connected.
- `decline_closes_and_persists_nothing`: B sees `Failed(Declined)`. A's known store is unchanged. A's data-dir file listing and bytes are unchanged before vs after.
- `pending_prompt_expires`: with `prompt_ttl` set short via `GateLimits` → `Failed(Expired)`, nothing persisted.
- `known_peer_reconnects_silently_across_restart`: after Accept, restart A with the same `FileKnownPeers` dir and keychain. B dials → no `IncomingRequest`, both `Connected`.
- `flood_guard_bounds_prompts`: B dials A 10× rapidly → at most 1 pending prompt and at most `max_attempts_per_peer` prompts in the window. Then 8 distinct nodes dial → `pending_len() <= max_pending`, and the rest are dropped with no events.
- `non_hello_first_frame_and_oversized_hello_are_rejected`: via `raw_dial`, send a `Chat` envelope first, then 64 KiB of junk, then random bytes. Each time: no prompt, no event, and A still serves a normal connect afterwards.
- `self_dial_is_rejected`.
- `connection_drop_emits_disconnected`.
- `declined_inbound_event_is_inbound_and_leaves_no_record`: A declines B → A's event is `Failed(Declined)` with `Direction::Inbound`. Afterwards A's `connected_peers()` and `pending_incoming()` are empty and no file in A's data dir changed. B's event carries `Direction::Outbound`.
- `cached_node_id_dials_via_address_lookup`: A and B share a `testing::MemoryAddressLookup`. B dials A from a **NodeId-only** ticket (the legacy `p2pm:<nodeid>` form, i.e. what a cached NodeId gives you) → iroh resolves A's addresses through the lookup → prompt → Accept → `Connected`. This covers DD-02 "a peer holding a ticket (or a cached NodeId) can be dialed".
- `ticket_when_ready_waits_then_returns_and_never_hangs`: on a started node it returns a ticket that parses and contains at least one direct address (loopback). With an artificially short timeout it still returns a parseable ticket, within the timeout.
- `my_ticket_changed_fires_when_addresses_change`: verify iroh exposes a way to trigger an address change in tests (e.g. rebinding or a test hook); if none exists, unit-test the debounce/compare logic as a pure function (`addr_set_changed(old, new) -> bool`) and say so in the test doc.
- `identity_key_never_leaves_core`: DTO JSON (`MyTicketDto`, events) contains none of hex/decimal/base64 forms of the identity key. Reuse the canary approach from `service.rs`.

Relay-fallback test, `core/tests/discovery_relay.rs` (maps to "Relay fallback (simulated no-direct-path)"):
- Start a local test relay on loopback. Prefer plain HTTP if the helper allows it, so the capture checks iroh's QUIC encryption and not only relay TLS. Put a small recording TCP proxy between the nodes and the relay, and point `NodeConfig::relay_only(proxy_url)` at it.
- Run a full connect + Accept with a canary display name (e.g. `"CanaryZ9Q"`) and assert `Connected`. Assert the path is relay-only: iroh's path info `is_relay()`. Verify the exact API name.
- Assert the canary bytes (UTF-8, and the postcard Hello encoding) appear nowhere in the captured proxy bytes.
- If the test relay can only run with TLS, keep the assertion and record in the test's doc comment that it verifies TLS + QUIC.
- Test-only TLS shortcuts (e.g. skip-verify for the self-signed test relay) are allowed **only** in this test / `test-support` code.

### Parallel track C: frontend (Svelte)

Owns **only** these files:
- `app/src/net.ts` (new)
- `app/src/lib/netStore.ts` (new)
- `app/src/views/Network.svelte` (new)
- `app/src/views/IncomingRequestPrompt.svelte` (new)
- their `*.test.ts`
- `app/src/lib/router.ts`, `app/src/views/MainScreen.svelte`, `app/src/views/MainScreen.test.ts` (new), `app/src/views/MyInfo.svelte`, `app/src/views/MyInfo.test.ts`

Code against this JSON contract (it matches the DTOs above and the commands in the integration task):

```ts
// net.ts
export interface MyTicketDto { ticket: string; qr_svg: string }
export interface IncomingRequestDto { node_id: string; display_name: string | null }
export interface ConnectionStateDto {
  node_id: string;
  direction: "inbound" | "outbound";
  state: "connecting" | "connected" | "disconnected" | "failed";
  reason: "declined" | "timeout" | "unreachable" | "protocol" | "expired" | "closed" | null;
}
export type NetCmdError =
  | { type: "NotReady" } | { type: "InvalidTicket" } | { type: "SelfDial" }
  | { type: "NoSuchRequest" } | { type: "Internal" };
export const netMyTicket = () => invoke<MyTicketDto>("net_my_ticket");
export const netConnectTicket = (ticket: string) => invoke<{ node_id: string }>("net_connect_ticket", { ticket });
export const netRespondIncoming = (nodeId: string, accept: boolean) =>
  invoke<void>("net_respond_incoming", { nodeId, accept });
// Read-only snapshot of open prompts. It complements the `incoming_request` event the way
// `net_lan_peers` complements `lan_peers_changed` (02-02). This is an addition to the DD-02 API sketch.
export const netPendingIncoming = () => invoke<IncomingRequestDto[]>("net_pending_incoming");
export function describeNetError(err: unknown): string { /* user-facing copy per type */ }
// event "my_ticket_changed": no payload. Listeners re-fetch netMyTicket(). Addition to the DD-02 API sketch.
```

> **Clipboard gate warning:** the pre-commit hook and `clipboard-guard.test.ts` grep for the words `writeText` and `clipboard` in **every** non-test file under `app/src`, comments included. Do not write either word anywhere except `MyInfo.svelte`. Say "copy" or "paste" in comments and copy text instead.

- `lib/netStore.ts`:
  - Svelte stores `incomingRequests` (queue keyed by node_id, no duplicates) and `connections` (map node_id → ConnectionStateDto).
  - `startNetEvents()` first registers `listen("incoming_request")` and `listen("connection_state")` from `@tauri-apps/api/event`. **Then** it seeds `incomingRequests` from `netPendingIncoming()`, merging by node_id so an event that raced the snapshot is not duplicated. It returns an unlisten function. This way prompts raised before MainScreen mounted, or during a webview reload, are not lost.
  - A `NotReady` from the snapshot is ignored; the queue simply starts empty.
  - Any `connection_state` for a node_id removes that node's pending prompt.
  - **No history for declined/expired inbound peers** (DD-02 Security): a `connection_state` with `direction: "inbound"` and `state: "failed"` only removes the prompt. It is **not** added to `connections`, and any existing entry for that node_id from an earlier inbound attempt is removed. Outbound failures (e.g. "declined" on the dialer side) are shown, because the user initiated that dial and needs the result.
- `MainScreen.svelte`: in `onMount`, call `startNetEvents()` and unlisten on destroy. Render `<IncomingRequestPrompt />` above the tab panel, so a prompt shows whichever tab is open.
- `router.ts`: `TabId = "my-info" | "network"`. Append `{ id: "network", label: "Network", component: Network }`.
- `Network.svelte`:
  - "Connect by ticket" form: textarea + a button labelled **"Connect and remember this device"**, with helper text "Once they reply, this device is remembered and can reconnect without asking you again." The interim DD-02 rule adds a dialed peer to the known set only after its Hello reply. Trim the input; submit is disabled when empty. Show inline `role="alert"` errors via `describeNetError` (e.g. "That ticket is not valid.", "That's your own ticket.").
  - A connection list rendered from `connections`: NodeId in full, state, reason.
  - Peer names are rendered **only** as text interpolation. Never `{@html}`.
- `IncomingRequestPrompt.svelte`:
  - Shows the first queued request and "+N more" when more are queued.
  - Copy: "**Unverified** — someone is asking to connect". The self-asserted name is shown as "calls themselves “X”", or "(no name)" when there is none, with the full NodeId underneath.
  - Accept / Decline buttons → `netRespondIncoming(node_id, true|false)`. On `NoSuchRequest`, drop the prompt silently (it expired).
  - The UNVERIFIED badge is static markup and cannot be suppressed by the name.
- `MyInfo.svelte`: keep `identityMyInfo()` for name and NodeId, then merge the live ticket: `info = { ...(await identityMyInfo()), ...(await netMyTicket()) }`. This keeps `copy(info.ticket,` and `copy(info.node_id,` intact for `clipboard-guard.test.ts`. If `netMyTicket` rejects with `NotReady`, keep the identity ticket and show a small "network starting…" note. Network.svelte must **not** use the clipboard.
  - **Freshness:** while mounted, MyInfo listens to `my_ticket_changed` and re-fetches `netMyTicket()`, replacing ticket + QR in place; it unlistens on destroy. It also re-fetches once when a `NotReady` fallback was shown and the node later becomes ready (the first `my_ticket_changed` covers this). This way the ticket a user copies always reflects the live endpoint (relay URL + current direct addrs), not a start-up snapshot.

Vitest:
- `Network.test.ts`: submits the trimmed ticket. InvalidTicket shows an alert and the field keeps its value. The connection list updates when the mocked store changes. A name with HTML/bidi-looking text renders inert.
- `IncomingRequestPrompt.test.ts`: renders UNVERIFIED + full NodeId. Accept/Decline call `netRespondIncoming` with exact args. Two requests with the **same name, different NodeIds** render as distinct entries (the second shows via "+1 more", then appears after the first is resolved). The prompt disappears when a `connection_state` arrives for its NodeId. `NoSuchRequest` removes it.
- `netStore.test.ts`: mock `@tauri-apps/api/event`'s `listen` and `netPendingIncoming`, then check:
  - queue dedup;
  - removal on `connection_state`;
  - unlisten is called;
  - **seeding**: requests returned by `netPendingIncoming()` before any event appear in `incomingRequests`;
  - a request present both in the snapshot and in an event appears once;
  - a `NotReady` snapshot leaves an empty queue without throwing;
  - an **inbound** `failed` (declined/expired) event removes the prompt and leaves `connections` without that node_id; an **outbound** `failed` event is kept in `connections`.
- `MyInfo.test.ts`: mock `../net` `netMyTicket`. The ticket/QR shown come from it, copy still copies exactly `info.ticket` / `info.node_id`, and the NotReady fallback works. Emitting a mocked `my_ticket_changed` re-fetches and the displayed ticket and the copied value update to the new ticket. The listener is removed on unmount.
- `MainScreen.test.ts` (reachability; none exists today): mock `../net` / `../identity` and `@tauri-apps/api/event`. Assert:
  - the tab bar shows both "My Info" and "Network";
  - clicking "Network" renders the Network view (the connect-by-ticket field is present), and clicking "My Info" goes back;
  - with a pending request seeded from `netPendingIncoming`, the Accept/Decline prompt is visible on **both** tabs;
  - `startNetEvents`' unlisten runs on unmount.
- `clipboard-guard.test.ts` must still pass unchanged.

### Integration sub-task (sequential, after A, B, C merge)

Integration also owns every change to the interface contract and removes the `mod.rs` region markers.

Owns:
- `app/src-tauri/src/net_cmd.rs` (new), `app/src-tauri/src/test_util.rs` (new, `#[cfg(test)]`), `app/src-tauri/src/lib.rs`, `app/src-tauri/src/identity_cmd.rs`
- `app/src-tauri/Cargo.toml` (dev-dep `tokio`)
- `README.md`, `AGENTS.md` (layout table: `core/` now has `identity/` and `discovery/`)

1. Settings sharing: build `Arc<FileSettingsStore>` once in `setup` and pass clones to the `IdentityService` and to `NodeDeps`. Change `AppIdentity` to `IdentityService<OsKeychain, Arc<FileSettingsStore>, LiveTicketProvider>`.
2. `net_cmd.rs` follows the `identity_cmd.rs` pattern: plain functions over `&Node` that return DTOs and `NetCmdError` (`#[serde(tag = "type")]`: `NotReady`, `InvalidTicket`, `SelfDial`, `NoSuchRequest`, `Internal`). Thin async `#[tauri::command]` wrappers:
   - `net_my_ticket() -> MyTicketDto` (ticket + `ticket_to_qr_svg`), built from `node.ticket_when_ready(Duration::from_secs(5))` so the first ticket the user sees already carries the relay URL / direct addrs when the network allows it
   - `net_connect_ticket(ticket: String) -> { node_id }`
   - `net_respond_incoming(node_id: String, accept: bool)`. Parse `node_id` as `NodeId`; a malformed id → `NoSuchRequest`, never a panic.
   - `net_pending_incoming() -> Vec<IncomingRequestDto>` (from `Node::pending_incoming()`). This is an addition to the DD-02 API sketch: a read-only snapshot for UI seeding. It exposes nothing beyond what `incoming_request` already carries. Note it in the PR.
   - The `my_ticket_changed` event (no payload) is the second addition to the DD-02 API sketch; note it in the PR too.
3. `NetState` (managed): `tokio::sync::OnceCell<Node>` (or `RwLock<Option<Node>>`) plus a "starting" guard, so the node is started at most once. Keep the start logic in a plain, testable function, `NetState::ensure_started_with(&self, start: impl Future<Output = Result<Node, StartError>>)`, so unit tests can drive it without Tauri. `fn ensure_started(app: &AppHandle)` spawns with `tauri::async_runtime::spawn`:
   - It loads `FileKnownPeers::new(data_dir, account_id)` and calls `Node::start(&identity, deps, NodeConfig::desktop())`.
   - It forwards `node.subscribe()` to the webview with **one** generic loop: `let (name, payload) = dto::event_json(&ev); app.emit(name, payload)` (`tauri::Emitter`). No event names are hard-coded in `app/`; they come from core's single mapping.
   - Once the node is stored, it emits one `my_ticket_changed` so a My Info view that showed the `NotReady` fallback refreshes.
   - Commands called before the node is ready return `NotReady`.
4. Start points:
   - In `setup`, if `identity.status()` is `Present`, call `ensure_started`.
   - In the `identity_provision` command wrapper, call it after a successful provision, which needs an `AppHandle` parameter. The first run then gets a live node without a new command.
   - A start failure leaves the app usable (My Info falls back to the NodeId-only ticket) and reports `NotReady`.
   - **Graceful shutdown:** switch from `.run(ctx)` to `.build(ctx)?.run(|app, event| …)` and, on `RunEvent::ExitRequested`/`Exit`, take the node out of `NetState` and `block_on(node.shutdown())` with a bounded timeout (e.g. 2 s), so connected peers get a prompt close (`Disconnected`) instead of waiting for an idle timeout. Put the take-and-shutdown logic in a plain `NetState::shutdown_with_timeout(&self, d)` function so it can be unit-tested.
5. Register `net_my_ticket`, `net_connect_ticket`, `net_respond_incoming`, `net_pending_incoming` in `generate_handler!`. Check `capabilities/default.json`: `core:default` must cover event listen. Do not add clipboard or other permissions.
6. Command tests in `net_cmd.rs` (`#[tokio::test]`, using `messenger_core::discovery::testing`):
   - `errors_serialize_with_type_tag`
   - `my_ticket_parses_and_names_this_node`
   - `connect_invalid_ticket_is_invalid_ticket`
   - `respond_unknown_is_no_such_request`
   - `malformed_node_id_is_no_such_request`
   - `ticket_connect_accept_flow_end_to_end`: two in-process nodes driven only through the plain `net_cmd` functions; prompt → accept → both connected. This mirrors `first_run_flow_end_to_end`.
   - `dtos_never_contain_key_material`: reuse `assert_forms_absent`. It is currently private to `identity_cmd.rs`'s test module, so move it (and `base64_standard`) into a shared `#[cfg(test)] mod test_util;` (`app/src-tauri/src/test_util.rs`, declared in `lib.rs`). Both `identity_cmd` and `net_cmd` tests import it. This is real shared test code, not a stub. Its comments must not spell the secret type names; the hook greps for them.
   - `pending_incoming_snapshot_lists_open_prompts_and_drops_resolved`.
   - `net_state_starts_at_most_once`: call `ensure_started_with` concurrently and repeatedly; the start future runs exactly once (count it with an `AtomicUsize`).
   - `commands_return_not_ready_before_start`: every `net_*` plain function on an unstarted `NetState` returns `NotReady`.
   - `start_failure_reports_not_ready_and_my_info_falls_back`: a failing start future leaves `NotReady`, and `identity_my_info` still returns the NodeId-only ticket.
   - `dial_adds_known_only_after_hello_reply`: dialing a peer that declines, times out or is unreachable leaves the dialer's known store unchanged. A dial that gets a valid reply adds it.
   - `shutdown_closes_connections_and_peer_sees_disconnected`: two in-process nodes connected; `NetState::shutdown_with_timeout` on A → B emits `Disconnected` within a few seconds, and a second shutdown call is a no-op (no panic).
   - `forwarded_event_names_are_the_dd02_names`: drive a prompt + accept through two nodes, collect `event_json` output via the same forwarding helper the app uses (factor it as `fn forward(ev, emit: impl Fn(&str, Value))`), and assert the names seen are exactly from `{"incoming_request", "connection_state", "my_ticket_changed"}`.
7. README:
   - "Run the app" gains "Connect two devices by ticket".
   - Manual GUI checklist gains steps: My Info ticket is the long live form; paste it on another device → Accept/Decline prompt on the receiver, marked Unverified; Accept → both show Connected; Decline → dialer shows Declined; relaunch → a known peer connects without a prompt.
   - Document the interim trust rule: **"Connect and remember this device"** means a peer you dialed is remembered, and may later connect to you without a prompt, once it has replied with its Hello. Accepting an inbound request remembers the peer too. DD-03 replaces this with named contacts.
   - State that NAT traversal (hole punching, relay fallback) is provided by iroh; this app adds no traversal logic of its own.
   - "Build & test" mentions the Level-1 two-node tests.
   - Update the workspace table (`core` now has `discovery`).

## Quality gates (all must pass before commit)

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
git commit   # runs .githooks/pre-commit; NEVER --no-verify. Fix failures instead.
```

The two-node and relay tests run inside `cargo test --workspace`, so they are part of the mandatory gate. Each must finish within a few seconds. Use explicit timeouts, never unbounded waits.

## Guardrails (AGENTS.md, enforced; do not weaken)

- `SecretKeyBytes` / `DbKey` (and iroh `SecretKey`) never cross into `app/`, not even in types or comments. The pre-commit hook greps for the names. The identity key is loaded and converted inside `core` only.
- Clipboard access (`writeText` / `clipboard`) only in `app/src/views/MyInfo.svelte` (and tests). Network and the prompt never copy anything.
- Logs never contain message plaintext, Hello/display names, private keys or secrets. NodeIds and addresses are fine.
- `core` keeps `#![forbid(unsafe_code)]`.
- Only real-content files: no empty modules, stubs, or placeholder folders. `discovery/testing.rs` is real test-support code, gated like `identity::testing`.
- Wire format: postcard `Envelope` with `version`. This step adds only an additive constant (`MAX_HELLO_BYTES`). No breaking wire change, so `PROTOCOL_VERSION` stays 1. State this explicitly in the PR.
- Data is namespaced by `account_id` (`known_peers.json` keys by account uuid).
- Never write credentials, keys, tokens or secrets in code, tests, fixtures, Dockerfiles, compose files or docs. All credentials are provided/generated at runtime (keychain, env at container start, or generated test keys).

## Security (threats specific to this step: prevent and verify)

| Threat | Prevention | How to check |
|---|---|---|
| Unknown peer reaches the messaging layer before Accept | Acceptor never reads a second stream or sends anything while pending. Only authorized connections enter `connected_peers()`. | `unknown_peer_raises_exactly_one_prompt_and_gets_no_traffic_before_accept`, `non_hello_first_frame...`. Review: `grep -n "accept_bi\|accept_uni" core/src/discovery` shows exactly one call site (the Hello read). |
| Self-asserted name used for trust/dedup | `is_known` and the pending map are keyed by `conn.remote_id()`. Names are stored as advisory values only. | known_peers test "two NodeIds same name". Review: `grep -rn "display_name" core/src/discovery/node.rs` shows no comparison or lookup on a name. Vitest same-name prompt test. |
| Prompt-flood DoS (one peer or many rotated NodeIds) | `FloodGuard` (per-peer pending=1, per-peer rate limit, global `max_pending`, `max_handshakes`, bounded tables). Drops are silent. | `flood_guard_bounds_prompts` + flood unit tests. Make sure there is no `warn!`/`info!` on the drop path: `grep -n "Drop" -A3 core/src/discovery/node.rs`. |
| Malformed/oversized ticket or Hello crashes the endpoint | Length check before decode. Bounded `read_to_end`. Every error is mapped. No `unwrap`/`expect` on peer input. | Fuzz loop in ticket tests, the raw-dial resilience test, `corrupted_ticket_is_handled_and_endpoint_stays_up`. `grep -n "unwrap()\|expect(" core/src/discovery/*.rs` outside `#[cfg(test)]`/testing → none. |
| `is_known` storage error silently trusts | Fail closed: an error is treated as unknown, so the user is prompted. | Unit test with a `KnownPeers` double that returns `Err`: a prompt is raised, not a silent connect. |
| Identity key leaks to app/DTO/logs | `load_identity_secret` is `pub(crate)`. Conversion happens inside `Node::start`. DTOs hold strings only. | `identity_key_never_leaves_core`, `dtos_never_contain_key_material`, the pre-commit secret grep, `grep -rnE "SecretKey(::\|Bytes)\|iroh_base::SecretKey\|DbKey" app/src app/src-tauri/src` → nothing. |
| Plaintext/names in logs or a side channel | No payload/name logging. All traffic is iroh QUIC. No debug flag bypasses the gate or encryption. | Relay canary capture test. `grep -rniE "debug_bypass\|skip_trust\|insecure" core/src app/src-tauri/src` → none outside test-support/tests. |
| Test-only TLS/relay shortcuts reach the desktop build | They are confined to `[dev-dependencies]` / `cfg(any(test, feature = "test-support"))`. The app's normal deps never enable `test-support`. | `grep -n "test-support" app/src-tauri/Cargo.toml` → only under `[dev-dependencies]`. `cargo tree -p app -e normal -f '{p} {f}' \| grep test-support` → empty (verify flags). |
| Declined/expired unknown peers leave a trace ("no history" violation) | Acceptor-side failures carry `direction: "inbound"`; netStore only clears the prompt and never lists them. Core keeps no record after the event. | `declined_inbound_event_is_inbound_and_leaves_no_record`, netStore inbound/outbound failure test. |
| Stale/incomplete ticket shared (missing relay → peer behind NAT can't connect) | `ticket_when_ready` + `my_ticket_changed` refresh in My Info. | `ticket_when_ready_waits_then_returns_and_never_hangs`, MyInfo refresh Vitest. Manual: open My Info right after launch; the ticket updates once the relay connects. |
| Event names drift from the DD-02 API (UI silently stops receiving prompts) | One `event_json` mapping in core, used by app and headless; exhaustive match. | `event_names_match_dd02_api_table`, `forwarded_event_names_are_the_dd02_names`. `grep -n '"incoming_request"\|"connection_state"' app/src-tauri/src` → none (names only come from core). |
| HTML/bidi injection through peer names in UI | Core validates names (`validate_display_name` rejects control and bidi). Svelte uses text interpolation only. | Vitest inert-render tests. `grep -n "@html" app/src/views/Network.svelte app/src/views/IncomingRequestPrompt.svelte` → none. |

## Definition of Done

- [ ] `core::discovery` exists with real content and curated re-exports. `core` still forbids unsafe.
- [ ] A single endpoint per process is bound to the DD-01 identity key and `protocol::ALPN`. The key is never exposed outside `core`.
- [ ] `net_my_ticket` returns a live ticket (NodeId + relay + direct addrs) that another node parses and dials. My Info displays it, and the legacy NodeId-only ticket still parses.
- [ ] Pasted tickets are validated (bounds, structure). Bad input gives a clear UI error; the endpoint stays up.
- [ ] Hello exchange both ways. Unknown NodeIds are gated behind exactly one Accept/Decline prompt. Known peers (persisted, per `account_id`) connect silently, including after a restart. Decline/expiry persists nothing.
- [ ] Flood guard caps pending prompts and rate-limits per-NodeId retries, silently.
- [ ] Relay-only connection works. Canary capture shows no plaintext on the relay path.
- [ ] Events `incoming_request` / `connection_state` reach the GUI. The Network tab (reachable from MainScreen) offers connect-by-ticket ("Connect and remember this device") and connection state. The prompt is global and is seeded from `net_pending_incoming`, so no prompt is lost before mount or on reload.
- [ ] `NetState` starts the node at most once. Commands return `NotReady` before it is ready. A dialed peer is remembered only after its Hello reply. The node shuts down gracefully on app exit.
- [ ] My Info's ticket reflects the live endpoint: `net_my_ticket` waits (bounded) for the endpoint to be online, and `my_ticket_changed` refreshes it after address changes.
- [ ] A peer known only by NodeId (legacy/cached) can be dialed via iroh address lookup (tested with `MemoryAddressLookup`).
- [ ] Declined/expired inbound peers leave no history in core or the UI; every `connection_state` carries `direction`.
- [ ] Event names come only from core's `event_json` and match the DD-02 API table (tested). `MainScreen.test.ts` proves the Network tab is reachable and the prompt is global.
- [ ] All tests listed above exist and pass. All quality gates and the pre-commit hook pass with no `--no-verify`.
- [ ] README (run/test/manual checklist) and AGENTS.md layout are updated. The PR states "wire format unchanged; PROTOCOL_VERSION not bumped".
