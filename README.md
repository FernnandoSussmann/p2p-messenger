# p2p-messenger

A fully decentralized, end-to-end-encrypted peer-to-peer messenger written in
Rust. There is no central server: each user's keypair *is* their identity, and
peers connect directly (addressed by public key) over [iroh], with relay
fallback for ciphertext when hole punching fails.

Desktop (Linux / macOS / Windows) is the v1 target. See [BLUEPRINT.MD](BLUEPRINT.MD)
for the full design, scope, and what is deferred to v2+.

## Workspace layout

This is a Cargo workspace built from vertical feature slices. Crates are added
only once they carry real code — no empty stubs or placeholder modules.

| Crate      | Status  | Responsibility                                               |
| ---------- | ------- | ------------------------------------------------------------ |
| `protocol` | present | Shared serde wire types (`Envelope`, `Payload`, …) + version |
| `core`     | present | UI-agnostic logic by slices: `identity`, `discovery` (iroh node, LAN mDNS discovery + presence) and `contacts` (contact book, trust, interim store; more to come) |
| `app`      | present | Thin Tauri v2 + Svelte shell over `core`                     |
| `headless` | present | Test-only headless node + file keyring for Level-3 simulation; never shipped, never used by `app` |

## Run the app

Requires Rust 1.95 or newer (the workspace MSRV), Node.js 20+, and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
(on Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev`).

```sh
npm start
```

This installs the frontend dependencies, starts the Vite dev server, and opens
the desktop window. Running plain `cargo run` instead shows "connection refused"
because nothing serves the dev frontend.

The app stores its keys in your OS keychain, so the first launch offers to
create a real identity.

### Connect two devices by ticket

1. On device A open **My Info** and copy the ticket (the long live form that
   carries the relay URL and current addresses).
2. On device B open **Network**, paste it and press **Connect and remember this
   device**.
3. Device A shows an Accept/Decline prompt marked **Unverified** (the name is
   self-asserted; trust is the NodeId). Accept, and both sides show Connected.

Trust rule: trust is the peer's NodeId, kept as a contact.

- **Accept** asks for your name for them and adds an **Unverified** contact.
  The field is prefilled with what the peer calls itself (or "Peer <id>" when
  it sent no name or an invalid one); edit it before clicking Accept. Nothing
  is saved until you click Accept.
- A peer you dial is remembered once it replies with a valid Hello, as an
  Unverified contact named after what it calls itself, or "Peer <id>". It may
  later connect to you without a prompt.
- Later name changes by the peer never change your nickname.
- Declined or expired requests leave no trace.
- Contacts are kept per account in `contacts.json` **in plaintext (nicknames
  included) until DD-05's encrypted DB replaces it**.
- An existing `known_peers.json` is migrated once at startup and deleted; its
  peers still connect without a prompt. If the migration fails, they are
  prompted again.

Names (yours and peers') may no longer contain invisible characters such as
zero-width spaces. Joiners are allowed only inside emoji sequences (👨‍👩‍👧,
🏳️‍🌈, subdivision flags such as 🏴󠁧󠁢󠁥󠁮󠁧󠁿). A saved name that now fails is treated as
unset, and My Info asks for a new one.

NAT traversal (hole punching, relay fallback) is provided by iroh; this app
adds no traversal logic of its own. Relays only ever carry ciphertext.

### Peers on this network, presence and relay

The **Network** tab lists peers found on the local network via mDNS. Entries are
always marked **Unverified**: the name is self-asserted, only the NodeId counts.
Picking one runs the same connect and Accept/Decline gate as a pasted ticket.
Discovered peers are kept in memory only, never on disk.

**Visible on this network** (default on, saved per account) controls only
whether *you* advertise on the LAN. Off hides you from other devices' LAN lists
and withdraws your display name from the mDNS record; you still browse, and
anyone holding your ticket can still connect. It does not make you offline:
iroh's internet address lookup keeps publishing your addresses (never your
name) for ticket holders.

Relay override: set `MESSENGER_RELAY_URL` at launch to use your own relay
(`https`, or `http` for a loopback host only), or `default` to go back to the
default relay mesh. The value is saved per account and applied when the node
starts; an invalid value stops the app from starting. There is no GUI editor in
v1. Relays see NodeIds, addresses and timing, never plaintext; self-hosting a
relay is the way to keep that metadata off shared infrastructure.

## Build & test

```sh
cargo test --workspace   # includes the Level-1 two-node tests (real iroh endpoints on loopback)
cd app && npm ci && npm test && npm run check && npm run build
```

### Pre-commit hook (the test gate)

There is no CI, so the tracked hook in `.githooks/pre-commit` is the mandatory
local gate. Install it once per clone:

```sh
git config core.hooksPath .githooks
```

It blocks the commit on any failure of: `cargo fmt --all --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --workspace` (includes the secret-leak guards), the secret grep
gates (no key types under `app/`; clipboard only in `MyInfo.svelte`), and the
frontend `npm run check`, `npm test` and `npm run build`.

Verifying the gate: stage a file under `app/` containing a planted
secret-looking identifier (e.g. a Tauri return type referencing the key type
name) and run `git commit`; the hook must reject it. Do this manually after
changing the hook.

### Manual GUI checklist

No Tauri WebDriver driver is available in this environment, so the first-run
flow is automated at command-invoke level (`first_run_flow_end_to_end` in
`app/src-tauri/src/identity_cmd.rs`, `ticket_connect_accept_flow_end_to_end` in
`app/src-tauri/src/net_cmd.rs`) and component level (vitest). Check by
hand with `cd app && npm run tauri dev` on a fresh profile and empty keychain:

1. First launch shows the wizard; empty or whitespace names are rejected.
2. A valid name lands on the main screen; relaunch skips the wizard.
3. My Info shows name, full NodeId, ticket and QR; both copy buttons paste
   exactly the ticket / NodeId.
4. Denying keychain access shows the blocking error screen, never the wizard.
5. My Info shows the long live ticket (not just `p2pm:<nodeid>`); it updates
   shortly after launch once the relay connects.
6. Paste it on a second device (Network tab): the receiver gets an
   Accept/Decline prompt marked Unverified, on whichever tab is open. It shows
   a prefilled, editable "Your name for them" field and what the peer calls
   itself; an empty field disables Accept.
7. Accept (with the name you chose): both devices show Connected. Decline: the
   dialer shows Declined and the receiver keeps no entry.
8. Relaunch and reconnect from the same device: a known peer connects without
   a prompt.
   After upgrading a profile that has a `known_peers.json`, a previously
   accepted peer still connects without a prompt.
9. Two instances (see Two-device simulation): each appears in the other's
   Peers on this network list as Unverified, with its full NodeId. Connect from
   the list: the other side gets the Accept/Decline prompt.
10. Turn Visible on this network off: the peer disappears from the other
    list within seconds, the other still shows in yours, and a ticket connect
    to you still works. Turn it on: you reappear with your current name.
11. Rename yourself while visible: the other instance shows the new name.
12. `MESSENGER_PROFILE=../x` or `MESSENGER_RELAY_URL=bad` refuses to start.

## Two-device simulation

**Level 1** (in-process, runs in the pre-commit gate): `cargo test --workspace`
drives two real iroh nodes over loopback, with an in-process LAN bus standing in
for multicast. The real mDNS test is opt-in because it needs a
multicast-capable interface:

```sh
cargo test -p core --test discovery_lan -- --ignored
```

**Level 2** (two full app instances on one machine): `--profile <name>` or
`MESSENGER_PROFILE=<name>` (`[a-z0-9-]`, 1-32 chars, not `default`; the command
line wins) gives an instance its own data directory (`<app data>/profiles/<name>`)
and keychain entries, and adds "profile <name>" to the window title. An invalid
name stops the app from starting; it never falls back to the default identity.
The default (no profile) data and keychain are unchanged.

`tauri dev` twice would clash on the Vite port 1420, so run one dev instance and
start the second from the already built debug binary, which loads the same dev
server:

```sh
MESSENGER_PROFILE=a npm start                      # terminal 1: dev server + instance A
cargo build -p app                                 # once, if target/debug/app is missing
MESSENGER_PROFILE=b ./target/debug/app             # terminal 2: instance B
```

Then: complete first-run on each, find the other in **Peers on this network**
(or paste its ticket), Accept on the other side, see Connected, then turn
**Visible on this network** off on one: it vanishes from the other's list but
can still be connected to by ticket.

Same-host mDNS discovery needs a multicast-capable, non-loopback interface
(normal Wi-Fi or Ethernet). On a machine with only loopback, or with multicast
blocked by a VPN or firewall, the LAN list may stay empty; use the ticket path.

The profile override only relocates state. It never disables a prompt, trust
check or any other control.

### Level 3: containerized headless nodes (Docker)

`headless/` builds `messenger-headless`, a test-only node that runs the same
`core::discovery` stack with no webview, driven over a private Unix socket
(`messenger-headless ctl`). `docker/` runs two of them on one Docker network.
Prerequisites: Docker with Compose v2.

```sh
docker compose -f docker/compose.yaml build   # build the image
docker/smoke.sh                               # one command: up, LAN, ticket, reconnect, presence, isolation, teardown
```

Drive the nodes by hand:

```sh
docker compose -f docker/compose.yaml up -d
C="docker compose -f docker/compose.yaml exec -T"
$C node-a messenger-headless ctl --data-dir /data node-id
$C node-a messenger-headless ctl --data-dir /data ticket          # prints {"ok":true,"ticket":"..."}
$C node-b messenger-headless ctl --data-dir /data connect <TICKET>
$C node-a messenger-headless ctl --data-dir /data wait incoming_request --timeout-secs 30
$C node-a messenger-headless ctl --data-dir /data respond <NODE_B_ID> accept [NICKNAME]
$C node-a messenger-headless ctl --data-dir /data wait connection_state --state connected
$C node-a messenger-headless ctl --data-dir /data presence off
$C node-b messenger-headless ctl --data-dir /data peers
docker compose -f docker/compose.yaml down -v
```

`wait` consumes the event it returns: each event is delivered to at most one
`wait` call (a ring of the last 256 events is kept), and a timeout exits
non-zero. Teardown: `down -v` removes the per-node volumes; without `-v`,
identities persist until the next `down -v`. For more nodes, copy a service in
`docker/compose.yaml` together with its own volume; do not use `--scale`, since
replicas would share one volume.

Scope:

- The containers exercise mDNS discovery and the ticket path over direct
  addresses. They run `--relay disabled`, so the
  **relay** path is not tested by Docker; it is covered by the Level-1
  `core/tests/discovery_relay.rs`.
- The Docker bridge is not `internal: true`: mDNS needs the default route, so
  egress is open. Headless nodes always run with n0 internet lookup disabled
  (no n0 DNS/pkarr publishing or resolution); `--relay` is honored as given.
  The desktop app keeps internet lookup on.
- Parity: the same trust-gate and presence scenarios run against in-process
  nodes (`core/tests/discovery_parity.rs`) and against real headless processes
  (`headless/tests/parity.rs`). Host processes use `--lan off` (loopback
  multicast is unreliable), so the presence scenario proves presence
  **persistence and dialability** at both levels. Presence **visibility** is
  checked by `docker/smoke.sh` stage 5 (and at Level 1 by `discovery_lan.rs`).
- Measured on one run, with presence OFF on node A, node B's peer list dropped A
  after about 3 s (smoke.sh polls for up to 180 s; the goodbye is processed quickly, a silent peer would take its mDNS TTL).

Warnings: this is **test-only** with **throwaway identities**; never point a
real identity at a container. The file keyring stores secrets **unencrypted** in
the node's volume. The headless node never uses the OS keychain (`--keyring os`
is rejected). The control surface has no stability or security guarantees and is
not a product feature; it is not exposed in the GUI.

## License

[GPL-3.0-only](LICENSE).

[iroh]: https://www.iroh.computer/
