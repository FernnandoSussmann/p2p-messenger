# P2P Messenger — v1 Blueprint

## Identity & goals

A fully decentralized, end-to-end-encrypted peer-to-peer messenger in Rust. No
central server owns identity or data. **Multiplatform is the destination;
desktop (Linux/macOS/Windows) is the v1 priority and only target.** Mobile and a
native-shell web build come later as new `app`-layer targets over an unchanged
core.

## Networking — iroh

- Connections are **addressed by public key**. Each user's keypair *is* their
  identity; public key = address.
- Finding a peer right now: **LAN** via mDNS local discovery; **internet** via
  iroh's discovery + relay mesh.
- NAT traversal via **hole punching**; **relay fallback** (ciphertext only) when
  symmetric NAT defeats hole punching.
- No account server anywhere; relays are fungible and self-hostable.

## Security — mandatory E2E

- All traffic encrypted (iroh/QUIC + Noise). Relays and eavesdroppers see
  ciphertext only.
- Private key + SQLCipher key live in the **OS keychain** (`keyring` crate).
  Passphrase lock is a deferred opt-in hardening.

## Connecting to people (two paths, one trust model)

- **Remote:** copy/paste an iroh **ticket** (pubkey + relay + addrs), then assign
  a **local nickname**.
- **Local:** a live **"peers on this network"** list via mDNS.
- **Presence toggle** (default ON) controls whether you broadcast yourself to the
  LAN.
- LAN advertises **NodeId + self-chosen display name**, marked **unverified**
  until added. Contact rows: **name bold, NodeId small/gray beneath**, with an
  unverified badge for discovered peers.
- **Incoming connection policy:** unknown peer → explicit **Accept/Decline**
  prompt; existing contacts connect silently. Same flow for both paths.

## Messaging — v1 scope

- **Plain UTF-8 text only** (multi-line + emoji free). Attachments, voice, video,
  link previews = v2.
- **Both-online-only** delivery; architected (message IDs, delivery-status field,
  local outbox) so store-and-forward drops in later.
- Per-message status: **Sending → Delivered** (via a lightweight
  application-level **ack** message) or **Pending/Failed** (held in outbox,
  retried in-session).
- **Single identity per install**; data namespaced with `account_id` for future
  multi-identity.

## Storage & logging

- Message history in **SQLCipher** (encrypted-on-disk DB file, still queryable
  for search/scrollback), key from the OS keychain.
- Separate **diagnostic logging** via `tracing` → rotating local log file
  (connection/hole-punch/relay events; NodeIds & addresses but **never message
  plaintext**).

## UI — Tauri v2 + Svelte

- Rust core behind Tauri commands (typed TS bindings); **Svelte** frontend in the
  webview.
- **First-run:** generate key → store in keychain → ask display name → land on
  main screen.
- **"My Info" tab:** always-available view of your display name, NodeId, and
  copyable **ticket + QR** to share.
- Wire format: serde structs serialized with **`postcard`**, every message
  carrying a protocol **`version`** field.

## Project structure — Cargo workspace, vertical slices, minimal files

- `protocol` — shared serde message types + version.
- `core` — UI-agnostic logic, organized by **feature slice** (`identity`,
  `discovery`, `messaging`, `contacts`, …), independently testable with two
  in-process nodes.
- `app` — thin Tauri + Svelte shell.
- **Only files with real content** — no empty folders, stubs, or placeholder
  modules.

## License

**GPL-3.0** (copyleft).

## Explicitly deferred to v2+

Mobile & web builds · attachments/media · store-and-forward offline delivery · QR
*scanning* (display is in v1) · multiple identities · passphrase-at-rest ·
multi-device same-identity sync.
