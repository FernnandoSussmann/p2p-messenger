# DD-01 — Identity, Onboarding & My Info

**Feature slice:** `core::identity` + `app` first-run flow and "My Info" tab
**Status:** Draft (v1)
**Related:** [DD-02 Transport & Discovery](02-transport-discovery-and-connection.md),
[DD-05 Storage & Logging](05-encrypted-storage-and-logging.md)

---

## Context and scope

A user's keypair *is* their identity in this messenger: the iroh public key
(NodeId) is simultaneously their address and their cryptographic identity. There
is no account server to register with, recover from, or be deanonymized by. The
very first thing that must exist before anything else in the app can function is
a stable keypair and a place to keep its secret half safe.

This doc covers everything from the moment a fresh install is launched to the
point where the user is sitting on the main screen with a shareable identity:

- Generating the long-term identity keypair.
- Deriving and persisting the secret material (private key + the symmetric key
  that DD-05 uses to encrypt the message database) in the **OS keychain**.
- The `account_id` that namespaces all of this user's local data.
- The first-run UX: generate → store → ask display name → land on main screen.
- The always-available **"My Info"** tab that renders the user's display name,
  NodeId, and a copyable **connection ticket + QR code**.

It does **not** cover how connections are made or how peers are discovered
(DD-02), nor how the keychain-held database key is used to encrypt storage
(DD-05) — only how that key is created and handed off.

## Goals and non-goals

### Goals
- Produce exactly one long-term identity per install on first run, with zero
  user ceremony beyond choosing a display name.
- Keep both secrets — the identity private key and the SQLCipher database key —
  in the OS keychain via the `keyring` crate, never on disk in plaintext.
- Give every local record a stable `account_id` so a future multi-identity
  release needs no data migration.
- Make the user's identity trivially shareable: display name, NodeId, and a
  copy-to-clipboard ticket with a scannable QR rendering.
- Make the display name editable after first run without changing identity.

### Non-goals
- **Passphrase-at-rest / app lock** — explicitly deferred; the keychain is the
  trust anchor for v1.
- **Multiple identities per install** — the data model supports it; the UX does
  not expose it.
- **Multi-device sync of one identity** — a second device is a different
  identity in v1.
- **Key rotation / revocation** — a lost key is a lost identity in v1.
- **QR *scanning*** — v1 only *displays* a QR; importing is paste-the-ticket.

## The actual design

### Overview

On launch, the `identity` slice asks one question: *does this install already
have an identity?* It answers by looking for the `account_id` marker in local
app state and the corresponding secrets in the keychain. If present → load and
continue. If absent → run first-run provisioning. The app shell drives the
first-run wizard through Tauri commands and then never blocks on identity again.

```
launch
  │
  ├─ identity present?  ── yes ─▶ load identity ─▶ main screen
  │
  └─ no ─▶ generate keypair ─▶ derive db key ─▶ store both in keychain
            ─▶ write account_id marker ─▶ prompt display name ─▶ main screen
```

### Identity material

Three distinct secrets/identifiers exist; keeping them separate is a deliberate
trade-off for blast-radius containment:

| Item | Nature | Where it lives | Who reads it |
|------|--------|----------------|--------------|
| Identity private key | iroh secret key (ed25519) | OS keychain | DD-02 transport, to bind the endpoint |
| NodeId / public key | derived from private key, public | derivable; cached in app state | everyone; it's the address |
| Database key | random 256-bit symmetric key | OS keychain | DD-05 storage, to open SQLCipher |
| `account_id` | opaque local UUID | local app state (unencrypted marker) | all slices, for namespacing |

**Trade-off — one key or two?** We could encrypt the DB with a key *derived*
from the identity key. We choose an **independent** random database key so that
(a) the DB key can later be re-wrapped behind a passphrase without touching the
identity key, and (b) a future identity-key rotation does not force a full DB
re-encryption. The cost is a second keychain entry.

**Trade-off — what is `account_id`?** It is **not** the NodeId. Using the NodeId
as the namespacing key would leak identity into every table and complicate a
future key rotation. `account_id` is a locally-generated opaque UUID stored in a
small unencrypted marker file in the app data directory; it reveals nothing
about the user and is stable across restarts.

### Keychain access

All keychain reads/writes go through a single `identity` module boundary so that
the service/account naming scheme (`<app-id>` service, per-`account_id` entries)
lives in exactly one place. The keychain is treated as **authoritative**: if the
`account_id` marker exists but a keychain secret is missing or unreadable, the
app does **not** silently generate a new identity (that would orphan the
encrypted DB and look like silent data loss). Instead it surfaces an explicit
recoverable error state to the UI (see Security).

### First-run flow (app shell)

1. **Generate** — core creates the keypair and database key, stores both in the
   keychain, writes the `account_id` marker. This is atomic from the UI's point
   of view: either a fully provisioned identity exists afterward, or nothing
   was persisted (see Security — partial-write safety).
2. **Name** — the UI prompts for a display name. The name is validated
   (see guardrails) and persisted. Empty/whitespace-only names are rejected;
   the user cannot proceed without a valid name.
3. **Land** — the main screen opens. Identity is now a precondition every other
   slice can assume.

### "My Info" tab

Always reachable from the main screen. Renders:

- **Display name** — editable in place; edits re-validate and persist.
- **NodeId** — shown in full, with a copy button; this is the user's permanent
  address.
- **Connection ticket** — the shareable blob DD-02 defines (NodeId + relay +
  direct addrs). Copy-to-clipboard is the primary action.
- **QR code** — a rendering of the same ticket for device-to-device sharing by
  camera. Display only; scanning is out of scope.

Because relay/direct addresses in a ticket can change as the network
environment changes, the ticket and its QR are **regenerated live** from the
current endpoint state each time the tab is shown (DD-02 owns ticket assembly;
this tab only renders what DD-02 produces).

### API sketch (Tauri commands)

Names are indicative; shapes are described, not written as code.

| Command | Purpose | Returns |
|---------|---------|---------|
| `identity_status` | Is there a provisioned identity on this install? | present / absent / error-state |
| `identity_provision` | First-run: generate keypair + db key, store in keychain, write `account_id` | NodeId + `account_id`, or a typed error |
| `identity_set_display_name` | Set/update the display name (validated) | ok / validation error |
| `identity_my_info` | Fetch display name, NodeId, and the live ticket for the My Info tab | the three fields |

Note the identity private key and database key are **never** returned across the
Tauri boundary into the webview. The frontend only ever receives the public
NodeId, the display name, and tickets.

### Data storage

- **`account_id` marker:** a tiny unencrypted file in the OS app-data dir. Holds
  the opaque UUID only. No secrets.
- **Display name + provisioning timestamp:** stored in the encrypted DB (DD-05),
  in an `account`/settings row keyed by `account_id`. (Chicken-and-egg note: the
  display name is captured in first-run step 2, which runs *after* the DB key
  exists in step 1, so the encrypted DB is already openable.)
- **Secrets:** OS keychain only. Never written to the DB, logs, or marker file.

## Security

This slice is the security foundation; its concerns dominate the design.

- **Secret confidentiality.** Identity private key and DB key live only in the OS
  keychain (`keyring`). They must never be: written to disk outside the
  keychain, logged by `tracing` (DD-05 enforces redaction), returned to the
  webview, or placed on the clipboard. Reviews must check every new log line and
  every Tauri return type in this slice against this rule.
- **Partial-write safety.** Provisioning touches three stores (keychain ×2,
  marker file). A crash mid-provision must not leave a half-identity that the
  app would mistake for a corrupt one. Order the writes so the `account_id`
  marker — the thing `identity_status` keys off — is written **last**, only
  after both keychain entries are confirmed stored. If provisioning fails,
  best-effort roll back any keychain entries written so far.
- **No silent re-provision.** If the marker exists but a secret is missing (user
  cleared keychain, moved profiles, OS keychain reset), treat it as a recoverable
  error surfaced to the user, never as "fresh install." Silently generating a
  new identity would orphan the encrypted history and change the user's address
  without consent.
- **Display-name injection.** The display name is attacker-influenceable only for
  *peers* (DD-03), but the local name is user-supplied and later shown to peers
  via `Hello`. Validate: non-empty after trimming, max length bound (e.g. a few
  hundred UTF-8 code points), reject control characters / newlines / bidi
  override characters that could spoof rendering. Store exactly what passed
  validation.
- **Keychain prompt UX.** Some OS keychains prompt on access. The app must fail
  closed (stay on a blocking error screen) rather than proceeding without
  secrets if the user denies access.
- **Clipboard exposure.** Copying the ticket puts NodeId + network addresses on
  the clipboard. That is public-by-design information; the private key must never
  reach the clipboard. The copy action must be wired only to ticket/NodeId data.

## Testing

- **Unit — provisioning idempotence:** running provision when an identity already
  exists is rejected, not duplicated.
- **Unit — status detection:** marker-present + secrets-present → present;
  marker-present + secret-missing → error-state (not absent); marker-absent →
  absent.
- **Unit — partial-write rollback:** inject a keychain failure on the second
  secret; assert no `account_id` marker was written and the first secret was
  rolled back.
- **Unit — display-name validation:** empty, whitespace-only, over-length,
  control/newline/bidi characters all rejected; valid names (including emoji)
  accepted and stored verbatim.
- **Integration (two in-process nodes):** a freshly provisioned node exposes a
  NodeId and a ticket that the other node can parse (hands off to DD-02 tests).
- **Property/round-trip:** a stored-then-loaded identity yields the same NodeId.
- **Secret-leak guard (automated):** a test (or lint) that fails if identity key
  / DB key material appears in any Tauri command return type or log output in
  this slice.

## Definition of done

- [ ] First launch generates a keypair + DB key, stores both in the OS keychain,
      and writes an `account_id` marker — atomically w.r.t. the status check.
- [ ] Subsequent launches detect and load the existing identity with no prompt.
- [ ] Missing/unreadable secrets surface a recoverable error, never a silent
      re-provision.
- [ ] Display name is captured on first run, validated, editable later, and
      persisted in the encrypted DB keyed by `account_id`.
- [ ] "My Info" tab shows display name, full NodeId (copyable), live ticket
      (copyable), and a QR rendering of the ticket.
- [ ] No secret ever crosses the Tauri boundary, hits the clipboard, or is
      logged.
- [ ] All tests above pass; the secret-leak guard is part of the mandatory
      local pre-commit test gate (see README — no CI is used).
- [ ] `core::identity` joins the workspace with real content only (no stubs).
