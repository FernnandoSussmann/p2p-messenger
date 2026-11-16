# Design Docs — P2P Messenger v1

These documents plan the v1 implementation of the decentralized, end-to-end
encrypted peer-to-peer messenger described in [`../BLUEPRINT.MD`](../BLUEPRINT.MD).
They exist to solicit feedback and derive alignment **before** code is written.
They contain technical and product specification, trade-offs, security
requirements, and a definition of done — not source code.

## How the work is sliced

The blueprint maps onto the `core` feature slices (`identity`, `discovery`,
`messaging`, `contacts`) plus the shared `protocol` crate and the `app` (Tauri +
Svelte) shell. Each design doc below is a **vertical slice**: it covers the wire
types, the `core` logic, the persisted state, the Tauri command surface, and the
Svelte UI for one coherent feature.

| # | Design doc | Feature slice | Primary crates |
|---|------------|---------------|----------------|
| 1 | [Identity, Onboarding & My Info](01-identity-onboarding-and-my-info.md) | `identity` | `core::identity`, `app` |
| 2 | [Transport, Discovery & Connection](02-transport-discovery-and-connection.md) | `discovery` | `core::discovery`, `protocol`, `app` |
| 3 | [Contacts & Trust Model](03-contacts-and-trust.md) | `contacts` | `core::contacts`, `app` |
| 4 | [Messaging & Delivery](04-messaging-and-delivery.md) | `messaging` | `core::messaging`, `protocol`, `app` |
| 5 | [Encrypted Storage & Diagnostic Logging](05-encrypted-storage-and-logging.md) | cross-cutting | `core` (storage, logging) |
| 6 | [Lean Build Footprint, Self-Cleaning Test Gate & End-User Binary](06-lean-build-footprint.md) | cross-cutting | build/test/release tooling, `app` |

## Conventions shared by all docs

- **Language / license:** Rust, GPL-3.0-only (workspace-inherited).
- **Wire format:** `serde` structs serialized with `postcard`; every frame is a
  `protocol::Envelope` carrying a `version` field (already implemented).
- **Identity = public key:** a peer's verified identity is always its iroh
  NodeId, established by the transport. Self-reported names are untrusted until
  a user acts on them.
- **Single identity per install**, but all persisted rows are namespaced by an
  `account_id` so multi-identity can be added later without migration.
- **"Only files with real content"**: no empty folders, stubs, or placeholder
  modules. `core` and `app` crates join the workspace when they carry their
  first real code.
- **Testing is enforced locally, not by CI.** There is no CI pipeline. Instead,
  the test suite (including the automated secret-/plaintext-leak guards) is
  **mandatory on every local commit attempt**: a `pre-commit` git hook runs the
  workspace tests and blocks the commit if any fail. Each doc's "all tests pass"
  definition-of-done item is satisfied through this local gate.

## Status

All six docs are v1 drafts targeting the desktop build (Linux/macOS/Windows);
DD-03 is implemented, with the deviations listed in its Status line. DD-06 is implemented (see its Implementation notes).
Mobile/web, attachments, offline store-and-forward, QR *scanning*, multiple
identities, passphrase-at-rest, and multi-device sync are **out of scope** for
v1 (see each doc's non-goals).
