# DD-02 — Transport, Discovery & Connection

**Feature slice:** `core::discovery` + transport setup + `app` connection UX
**Status:** Draft (v1)
**Related:** [DD-01 Identity](01-identity-onboarding-and-my-info.md),
[DD-03 Contacts & Trust](03-contacts-and-trust.md),
[DD-04 Messaging](04-messaging-and-delivery.md)

---

## Context and scope

This slice is how two people's apps actually find each other and open an
encrypted channel, with no account server in the middle. It builds directly on
iroh: connections are addressed by public key (the NodeId from DD-01), peers are
located via LAN mDNS and iroh's internet discovery + relay mesh, and the link is
established by hole punching with a relay fallback that only ever carries
ciphertext.

Scope:

- Standing up the iroh endpoint bound to the identity key, speaking the
  messenger's ALPN (`p2p-messenger/1`, already defined in `protocol`).
- **Finding peers:** LAN discovery via mDNS; internet discovery + relay.
- **Connecting remotely:** parsing a pasted **ticket** (NodeId + relay + direct
  addrs) and dialing it.
- **Presence:** a toggle (default ON) controlling LAN self-advertisement.
- **Ticket assembly** for DD-01's "My Info" tab.
- **Incoming-connection policy:** unknown peer → explicit Accept/Decline;
  known contact → connect silently.
- Surfacing a live "peers on this network" list to the UI.
- A **two-device simulation harness** so a developer (in-process) or a tester
  (two full app instances on one machine) can exercise discovery, connection,
  and the trust gate end-to-end without two physical devices.

It does **not** own what happens *after* a channel is accepted (message framing
and delivery — DD-04), nor the persistence/trust state of peers (DD-03). It
produces connection events and consumes trust decisions.

## Goals and non-goals

### Goals
- One long-lived iroh endpoint per running app, bound to the DD-01 identity key,
  reachable over LAN and internet.
- Locate peers on the same LAN automatically (mDNS) and reachable peers on the
  internet via iroh discovery/relay.
- Connect to a remote peer purely from a pasted ticket — no server, no lookup
  service we operate.
- Traverse NAT by hole punching; fall back to relay (ciphertext only) when
  direct connection is impossible.
- Default-ON presence toggle that, when OFF, stops LAN self-advertisement.
- Gate every inbound connection from an unknown NodeId behind an explicit
  Accept/Decline prompt; let known contacts in silently.

### Non-goals
- **Running our own discovery/relay infrastructure** as a requirement — relays
  are fungible and self-hostable; we rely on the iroh default mesh and allow
  override.
- **Store-and-forward / offline delivery** — both peers must be online (DD-04).
- **Hiding presence on the internet** beyond not advertising on LAN — v1 does
  not promise metadata-level anonymity from relays (they see NodeIds + addrs,
  never plaintext).
- **QR scanning** to import a peer — paste the ticket (DD-01 displays QR only).
- **Blocklists / rate-limit tuning UI** — a minimal built-in guard exists
  (see Security), but no user-facing policy editor in v1.

## The actual design

### Overview

```
          ┌─────────────── core::discovery ───────────────┐
          │                                                │
 identity │   iroh endpoint (bound to identity key, ALPN)  │
 key ────▶│      │                │                 │      │
          │  mDNS advertise   mDNS browse      accept loop  │
          │   (presence)     (LAN peers)     (inbound conns)│
          └──────┬────────────────┬────────────────┬───────┘
                 │                │                │
          LAN peer list    live events to app   Accept/Decline
                                                   (DD-03 trust)
 paste ticket ─▶ parse ─▶ dial (hole punch → relay fallback) ─▶ channel ─▶ DD-04
```

The endpoint is created once, after DD-01 confirms an identity, and lives for
the process lifetime. Three concerns run against it: advertising/browsing on the
LAN, accepting inbound connections, and dialing outbound from tickets.

### Transport establishment

- The endpoint is bound to the identity secret key from DD-01's keychain, so the
  NodeId on the wire is the user's identity. It registers the ALPN constant from
  `protocol` so it will only complete handshakes with other instances of this
  messenger.
- All transport encryption (QUIC + Noise) is provided by iroh. We add no
  bespoke crypto. Relays and on-path observers see ciphertext only; this is a
  property we rely on and must not undermine (see Security).
- Relay configuration defaults to the iroh mesh and is overridable by
  configuration so a user can point at a self-hosted relay. Relay choice does
  not affect confidentiality, only reachability/metadata.

### Finding peers

**LAN (mDNS).** When presence is ON, the node advertises a service record
containing its **NodeId + self-chosen display name**. Simultaneously it browses
for peers' records and maintains a live list. Each discovered peer is marked
**unverified** (the display name is self-asserted; only the NodeId is
cryptographic) and handed to the app/DD-03 for rendering.

**Internet.** iroh's discovery publishes/looks up reachability info so a peer
holding a ticket (or a cached NodeId) can be dialed even across NATs. We do not
operate a directory; discoverability derives from the ticket the user shared and
iroh's mesh.

### Presence toggle

- Default **ON**. Persisted (see Data storage) so it survives restarts.
- ON → mDNS advertisement active; the node is listed by other LAN peers.
- OFF → advertisement stops; the node still *browses* and can still dial out and
  accept inbound connections. Presence governs *being broadcast*, not
  *being reachable by someone who already has your ticket*. This distinction is
  surfaced in the UI so "invisible" is not misread as "offline/unreachable."

### Connecting remotely (tickets)

- A ticket encodes NodeId + relay URL(s) + known direct addresses. DD-01's
  My Info tab assembles and displays it; this slice both produces it (from live
  endpoint state) and consumes it (when pasted).
- Pasting a ticket → parse/validate → dial. A successful dial yields a channel
  that flows into the same post-connection path as an inbound one, including the
  trust check and DD-04 messaging.
- Parse failures (truncated/garbage ticket) produce a clear, non-fatal UI error;
  they never crash the endpoint.

### Incoming-connection policy

Every inbound connection carries a verified NodeId (transport-authenticated) and
an opening `Hello` (DD-04 / `protocol`) with the self-chosen display name.

```
inbound connection (verified NodeId)
        │
        ├─ NodeId is a known contact? ── yes ─▶ accept silently ─▶ DD-04
        │
        └─ no ─▶ raise Accept/Decline prompt (shows display name + NodeId,
                 marked UNVERIFIED) ─▶ user decides
                     ├─ Accept ─▶ DD-03 adds/links contact ─▶ DD-04
                     └─ Decline ─▶ close connection, no persistence
```

Both the remote (ticket) path and the local (mDNS) path converge here: trust is
decided by NodeId recognition, identically regardless of how the peer was found
("two paths, one trust model").

### API sketch (Tauri commands + events)

| Command | Purpose |
|---------|---------|
| `net_presence_get` / `net_presence_set` | Read / toggle LAN presence |
| `net_lan_peers` | Snapshot of currently-discovered LAN peers (NodeId + unverified name) |
| `net_connect_ticket` | Parse + dial a pasted ticket |
| `net_my_ticket` | Assemble the current ticket for DD-01's My Info tab |
| `net_respond_incoming` | Resolve a pending Accept/Decline for a given NodeId |

| Event (core → app) | Meaning |
|--------------------|---------|
| `lan_peers_changed` | The LAN peer list changed (add/remove/name update) |
| `incoming_request` | An unknown NodeId is asking to connect (display name + NodeId) |
| `connection_state` | A peer connected / disconnected / failed, with reason class |

### Data storage

- **Presence toggle** and any **relay override** persist in the DD-05 settings
  table keyed by `account_id`.
- **Discovered LAN peers are ephemeral** — held in memory only, never written to
  the DB. Only peers the user explicitly adds become persistent contacts (DD-03).
- No addresses or tickets of *other* people are persisted beyond what DD-03
  stores for added contacts.

### Two-device simulation (dev & test harness)

Because this slice is meaningless with one node, a first-class way to run **two
nodes on one machine** is part of the feature, not an afterthought. Two levels
are provided, sharing the same `core::discovery` code paths so what is tested is
what ships:

**Level 1 — in-process, for developers and automated tests.** A helper spins up
two independent endpoints inside one process (or test binary), each with its own
identity key and its own in-memory/mock keychain + storage, and lets them
discover and connect over loopback. This is what the [Testing](#testing) cases
below drive. It needs no OS keychain, no real DB file, and no UI, so it runs in
the mandatory pre-commit gate (see README).

**Level 2 — two full app instances, for black-box / manual testing.** A tester
must be able to launch the real Tauri app **twice on one machine** as two
independent identities that see and message each other. This requires per-
instance **isolation of all shared, machine-global state**:

- **Data directory** — an instance/profile override (env var or CLI flag) points
  each instance at its own app-data dir, giving separate `account_id` markers,
  DB files, and log files (DD-01/DD-05).
- **Keychain namespacing** — the OS keychain is shared per OS user, so the
  single keychain boundary from DD-01 must incorporate the profile/instance id
  in its service/account naming. Two instances therefore store distinct identity
  and DB keys and never collide or overwrite each other.
- **Discovery over one host** — both instances bind their own endpoint; mDNS on
  loopback/localhost plus the relay/hole-punch path let them find and dial each
  other exactly as two machines on a LAN would. A tester can also copy one
  instance's ticket (My Info, DD-01) and paste it into the other to exercise the
  remote path.

The profile override is a developer/test affordance (documented, not advertised
in normal UI); without it the app uses the single default profile, so ordinary
users are unaffected. The simulation changes *where state lives*, never the
discovery/connection/trust logic — the trust gate, flood guard, and relay
fallback all behave identically to a real two-device setup.

**Level 3 — containerized nodes (Docker), for clean, repeatable isolation.** The
strongest isolation (and the easiest for CI-less, reproducible manual testing)
is to run each node in its own container. Because a desktop Tauri GUI does not
containerize cleanly, the image wraps a **headless node** — the same
`core::discovery`/messaging stack with no webview, driven by a thin scriptable
control surface (stdin/stdout or a local socket) instead of the Svelte UI. Each
container is a fully separate "device": its own filesystem (data dir, DB, logs),
its own keychain backend (a file- or env-backed keyring implementation selected
for containers, since no OS keychain exists in a minimal image — see Security),
and its own endpoint.

- A provided **compose setup** brings up two (or N) such nodes on one Docker
  network so they discover and dial each other; mDNS works within a shared
  bridge network, and the ticket/relay path works regardless, mirroring Level 2.
- This gives testers a one-command, deterministic two-device environment and
  gives developers an image to run the headless node anywhere, without a GUI.
- The headless node and the GUI app share the same `core`, so the containerized
  behavior is the shipping behavior minus the view layer.

> **Scope — test-only (for now).** The headless node and its scriptable control
> surface exist **solely to simulate devices for testing** in v1. They are not a
> shipped product surface, are not exposed in the GUI, and are not a supported
> way for end users to run the messenger. They are built from the same `core` so
> tests exercise real logic, but the control surface carries **no stability or
> security guarantees** and may change freely. Promoting it to a real headless/
> daemon mode is a separate future decision, not part of this slice.

### Running the two-device simulation

Concrete steps a developer or tester follows to get two simulated devices
talking. (Commands are illustrative; exact crate/binary/compose names are fixed
during implementation and documented alongside the harness.)

**Level 1 — in-process (automated).**
1. Run the discovery slice's test suite (e.g. `cargo test -p core` / the
   workspace tests). The two-node tests spin up both endpoints, connect them,
   and assert discovery, the trust gate, and a round-trip automatically.
2. Nothing else to set up — this runs in the mandatory pre-commit gate.

**Level 2 — two full app instances on one machine (black-box).**
1. Launch the app with the profile override pointing at profile **A** (e.g. a
   `--profile A` flag or a `MESSENGER_PROFILE=A` env var). Complete first-run so
   A has its own identity.
2. Launch the app a second time with the override set to profile **B**. B gets a
   separate identity, data dir, keychain entries, DB, and logs.
3. Connect them one of two ways:
   - **LAN:** leave presence ON; A appears in B's "peers on this network" list
     (and vice versa). Pick the peer and Add it.
   - **Ticket:** copy A's ticket from its My Info tab (DD-01), paste it into B's
     "connect by ticket" field.
4. On the first connection from an unknown peer, the Accept/Decline prompt
   appears on the receiving instance — Accept to establish trust.
5. Send messages both directions (DD-04) to verify the full round-trip; toggle
   presence off on one side to confirm it disappears from the other's LAN list.

**Level 3 — containerized nodes (Docker, reproducible).**
1. Build the headless-node image (documented build command).
2. Bring up the provided compose setup (e.g. `docker compose up`), which starts
   two headless nodes on one Docker network, each an isolated "device."
3. Drive each node through its control surface (the compose/harness docs give the
   exact invocation): have one print its ticket, hand it to the other to dial —
   or rely on mDNS within the shared network — then Accept the inbound request
   and exchange a message.
4. Observe both nodes' output/logs to confirm discovery, the trust gate, and the
   message round-trip; `docker compose down` leaves no residue.

**What the harness lets you verify:** both discovery paths (LAN list + pasted
ticket), the unknown→Accept/Decline prompt vs. known-contact silent connect,
presence on/off visibility, and (handing off to DD-04) a real round-trip
message between the two instances.

| Level | Form | Primary user | Isolation of shared state |
|-------|------|--------------|---------------------------|
| 1 | In-process two endpoints | Developer / pre-commit tests | In-memory mock keychain + storage |
| 2 | Two full app instances, one host | Black-box / manual tester | Per-profile data dir + keychain namespace |
| 3 | Containerized headless nodes (Docker) | Tester / developer, reproducible | Separate container FS + file/env keyring |

## Security

- **Confidentiality is the transport's job — don't undermine it.** All app
  traffic rides iroh QUIC+Noise. This slice must never add a side channel that
  carries message content in the clear (e.g. logging payloads, or a debug
  bypass). Relay fallback carries ciphertext only; verify no code path sends
  content outside the encrypted channel.
- **NodeId is the only identity that counts.** The `Hello` display name and the
  mDNS-advertised name are attacker-controlled and must be rendered as
  **unverified** everywhere until the user adds the peer. Never use the
  self-asserted name for trust decisions, de-duplication of contacts, or
  authorization. Two different NodeIds claiming the same name must remain
  visibly distinct.
- **Unknown peers are gated.** No inbound connection from an unrecognized NodeId
  may reach the messaging layer (DD-04) before the user Accepts. Declined and
  pending connections carry no persistence and no history.
- **Inbound-flood protection.** Because anyone with a ticket can attempt to
  connect, a minimal built-in guard is required: cap concurrent pending
  Accept/Decline prompts and rate-limit repeated inbound attempts from the same
  NodeId so a peer cannot spam the prompt UI into a denial of service. Excess
  attempts are dropped silently (no prompt, no log spam).
- **Ticket validation.** Treat pasted tickets as untrusted input: validate
  structure and bounds before dialing; a malformed ticket is a handled error,
  never a panic. Dialing a ticket reveals your addresses to that peer — this is
  inherent to connecting and is acceptable because the user initiated it.
- **mDNS exposure is local and opt-out.** Advertising broadcasts NodeId + name
  to the LAN. The default-ON toggle must be clearly discoverable so a user on an
  untrusted network (café, conference) can go dark. Presence OFF must verifiably
  stop advertisement (tested).
- **Relay trust boundary.** Relays are untrusted intermediaries: they can see
  NodeIds, addresses, and connection timing (metadata), but never plaintext.
  Documented as a known v1 metadata-privacy limitation; self-hosting a relay is
  the mitigation.
- **No plaintext in diagnostics.** Connection/hole-punch/relay events may log
  NodeIds and addresses (DD-05) but never message content.
- **The simulation affordance is isolation, not a bypass.** The profile/instance
  override only relocates per-instance state (data dir + keychain namespace); it
  must never weaken a security control — each simulated instance still keeps its
  own secrets in the keychain, still enforces the Accept/Decline trust gate, and
  still rides the encrypted channel. There is no mode that skips the trust prompt
  or disables encryption for testing. A test build must not ship a looser default.
- **Container keyring backend is scoped to containers.** The file-/env-backed
  keyring the Docker image uses (because a minimal container has no OS keychain)
  keeps secrets at rest inside that ephemeral container only. It must be selected
  explicitly for the headless/container target and must **never** become the
  default for the shipped desktop app, which always uses the real OS keychain
  (DD-01). The images are for testing non-sensitive, throwaway identities; this
  must be documented so no one points a real identity at a container.

## Testing

- **Two in-process nodes (LAN sim):** node A advertises, node B discovers A in
  its peer list with A's NodeId and unverified name.
- **Presence off:** with presence OFF, A does not appear in B's browse results;
  toggling back ON makes A reappear.
- **Ticket round-trip:** `net_my_ticket` on A, pasted into `net_connect_ticket`
  on B, yields a live channel; corrupted ticket yields a handled error, endpoint
  stays up.
- **Trust gate — unknown:** B (unknown to A) connecting raises exactly one
  `incoming_request` on A; no DD-04 traffic flows until Accept; Decline closes
  with no persisted state.
- **Trust gate — known:** once A has added B as a contact (DD-03), B reconnecting
  produces no prompt and connects silently.
- **Flood guard:** repeated rapid inbound attempts from one NodeId are
  rate-limited to a bounded number of prompts.
- **Relay fallback (simulated no-direct-path):** connection still establishes via
  relay; a payload-capture assertion confirms only ciphertext traverses the
  relay.
- **Resilience:** malformed/oversized mDNS records and tickets never panic the
  accept/browse loops.
- **Simulation harness — profile isolation:** two Level-2 instances pointed at
  distinct profiles get distinct keychain entries and data dirs; neither reads or
  overwrites the other's identity, DB, or logs.
- **Simulation harness — containers:** the compose setup brings up two headless
  nodes that discover and connect on a shared Docker network and complete a
  message round-trip; tearing down a container leaves no residue in another.
- **Harness parity:** a trust-gate / presence test passes identically at Level 1
  (in-process) and against the Level 3 headless node, proving the harness
  exercises shipping `core` logic, not a stub.

## Definition of done

- [ ] A single iroh endpoint binds to the DD-01 identity key and speaks the
      `protocol` ALPN.
- [ ] LAN peers are discovered via mDNS and surfaced live, each marked
      unverified with NodeId + self-asserted name.
- [ ] Presence toggle defaults ON, persists, and when OFF verifiably stops LAN
      advertisement while remaining dialable by ticket holders.
- [ ] A pasted ticket dials and connects, with NAT hole punching and
      ciphertext-only relay fallback.
- [ ] `net_my_ticket` produces a ticket that another node can parse and dial.
- [ ] Unknown inbound NodeIds are gated behind Accept/Decline; known contacts
      connect silently; both discovery paths share this one trust check.
- [ ] Inbound-flood guard caps pending prompts and rate-limits per-NodeId retries.
- [ ] No message plaintext is ever logged or sent outside the encrypted channel.
- [ ] A two-device simulation exists at three levels: in-process (pre-commit
      tests), two full app instances via a profile override (black-box), and
      containerized headless nodes via a Docker compose setup — all driving the
      same `core` logic.
- [ ] The profile override and container keyring backend isolate per-instance
      state without weakening any security control, and never become desktop
      defaults.
- [ ] The headless node / control surface is test-only in v1 (not exposed in the
      GUI, no stability/security guarantees), and the harness ships with
      documented step-by-step instructions for running the simulation at each
      level.
- [ ] All tests above pass; `core::discovery` joins the workspace with real
      content only.
