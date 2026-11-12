# DD-03 — Contacts & Trust Model

**Feature slice:** `core::contacts` + `app` contact list / add-contact UX
**Status:** Implemented (v1). Documented deviations / user decisions:
interim plaintext contact store until DD-05; history-deletion seam with a
no-op eraser until DD-04/DD-05; NodeId copy by selection only (clipboard
rule, user-accepted); contact list shows only the local nickname (self-asserted
name kept for a future contact-info screen); DD-01 name validation tightened
to reject invisible format characters except inside well-formed emoji ZWJ/tag
sequences; dialed peers auto-added as unverified with a default nickname, and
the Accept and naming prompts prefilled with that default; "Open chat"
deferred to DD-04.
**Related:** [DD-02 Transport & Discovery](02-transport-discovery-and-connection.md),
[DD-04 Messaging](04-messaging-and-delivery.md),
[DD-05 Storage & Logging](05-encrypted-storage-and-logging.md)

---

## Context and scope

Discovery (DD-02) produces a stream of *peers* — NodeIds with self-asserted
names, all untrusted. This slice is where a peer becomes a **contact**: a
persisted relationship the user has deliberately established, with a
**local nickname** the user controls and a **verified** status. It owns the
trust state that DD-02's incoming-connection policy consults ("is this NodeId a
known contact?") and that DD-04 attaches messages to.

Scope:

- The contact record: NodeId (the real identity), user-assigned local nickname,
  verified/unverified status, the peer's last-seen self-asserted display name,
  and bookkeeping timestamps.
- Adding a contact from either path: a pasted-ticket connection (DD-02) or a
  LAN-discovered peer, and via accepting an inbound unknown peer.
- The contact-list UI: rows with **name bold, NodeId small/gray beneath**, and an
  **unverified badge** for peers not yet confirmed.
- Nickname assignment/editing and removal.

It does **not** establish connections (DD-02) or send messages (DD-04). It is
the source of truth for *who is trusted and what we call them.*

## Goals and non-goals

### Goals
- Persist contacts keyed by NodeId, namespaced by `account_id`.
- Let the user attach and edit a **local** nickname independent of whatever the
  peer calls themselves.
- Represent the **unverified → verified** distinction and surface it in the UI.
- Serve DD-02's "known contact?" check cheaply and correctly.
- Make adding a contact a single deliberate action on both discovery paths.

### Non-goals
- **Cryptographic contact verification** (safety numbers / out-of-band key
  comparison) as a *flow* — v1 defines what "verified" means minimally
  (see below) but does not ship a QR-scan or compare-numbers ceremony.
- **Groups / rooms** — contacts are 1:1 only in v1.
- **Contact sync across devices** — local only.
- **Blocking/muting** — no block list UI in v1 (DD-02 has an anti-flood guard;
  user-facing blocking is deferred).
- **Importing contact lists** from external sources.

## The actual design

### Overview

A contact is the binding `{ NodeId ↔ local nickname, + trust state }`. The
NodeId is immutable and is the primary key within an account; everything else is
user- or peer-mutable metadata. DD-02 asks this slice one question on every
inbound connection; the user drives the rest through add/edit/remove.

```
discovered peer (DD-02, unverified, self-named)
        │  user chooses "Add"
        ▼
   assign local nickname ──▶ contact row persisted (NodeId + nickname)
        │
        ▼
   trust state: unverified ──(user confirms identity)──▶ verified
        │
        ▼
   DD-02 "known contact?" = yes  ·  DD-04 messages attach to this NodeId
```

### The contact record (conceptual fields)

| Field | Source | Mutable? | Notes |
|-------|--------|----------|-------|
| `account_id` | DD-01 | no | namespacing |
| NodeId | transport (verified) | no | the real identity; primary key within account |
| local nickname | user | yes | what *you* call them; shown bold |
| self-asserted display name | peer's `Hello` / mDNS | yes (peer) | last seen; shown subtly, never trusted |
| verified status | user action | yes | see trust model |
| added-at / last-seen | system | yes | bookkeeping, scrollback ordering aid |

The local nickname and the peer's self-asserted name are deliberately separate
columns. The nickname is authoritative for display; the self-asserted name is
advisory context (e.g. shown faintly as "calls themselves X").

### Trust model (minimal, honest)

v1 defines two states with clear, non-overstated meaning:

- **Unverified** — this NodeId appeared via discovery or an inbound request and
  the user has interacted with it, but the user has not asserted "this NodeId
  really belongs to the human I think it does." All freshly-discovered peers and
  freshly-accepted inbound unknowns start here.
- **Verified** — the user has explicitly marked the contact verified, meaning
  *they* vouch that this NodeId is the right person (e.g. they received the
  ticket over a trusted channel, or confirmed the NodeId out of band).

v1's "verify" action is an explicit user attestation, **not** an automated
cryptographic ceremony — and the UI must say exactly that so the badge is not
misread as a cryptographic guarantee the app did not perform. A full
safety-number comparison flow is a candidate for a later version and should slot
in by *upgrading* what the verify action does, without changing the stored
states.

Because identity = NodeId, there is no risk of a silent key change
impersonation within one contact: a different NodeId is simply a different
contact. The verification gap v1 leaves open is *initial* misattribution (being
handed the wrong ticket), which the user, not the app, closes in v1.

### Adding contacts (both paths converge)

- **From LAN peer:** user picks a discovered peer → "Add" → assign nickname →
  contact persisted as unverified.
- **From ticket connect:** after DD-02 dials a pasted ticket, the user is
  prompted to name the new contact → persisted as unverified.
- **From inbound Accept:** when the user Accepts an unknown peer (DD-02), that is
  simultaneously the "add contact" action; the user assigns a nickname at Accept
  time → persisted as unverified.

In all three, the nickname step is where the untrusted self-asserted name is
shown for context, and the user decides what to actually call them.

### Contact-list UI

- Each row: **nickname in bold**, **NodeId in small/gray beneath** (truncated
  with full value on hover/expand + copy), and an **unverified badge** when
  applicable.
- Discovered-but-not-added LAN peers may appear in a separate "on this network"
  affordance (owned visually here, data from DD-02), always badged unverified.
- Row actions: open chat (DD-04; deferred to DD-04, deviation O: no placeholder button), edit nickname, mark verified, remove contact.
- Removing a contact opens a confirmation that includes an explicit, opt-in
  **"also delete our message history"** choice (default **off**, so the
  destructive part is never the accidental default). The two effects are
  decoupled: confirming always removes the trust relationship; the checkbox
  additionally triggers DD-05's history deletion for that peer. See Security for
  exactly what each does.

### API sketch (Tauri commands)

| Command | Purpose |
|---------|---------|
| `contacts_list` | All contacts for the current `account_id` with trust state |
| `contacts_add` | Persist a NodeId + nickname as a new unverified contact |
| `contacts_rename` | Change the local nickname (validated) |
| `contacts_set_verified` | Flip verified status (explicit user attestation) |
| `contacts_remove` | Delete the contact record; a `clear_history` flag optionally also deletes that peer's messages (delegates to DD-05) |
| `contacts_is_known` | DD-02 trust check: is this NodeId a contact? |

### Data storage

- A `contacts` table in the DD-05 encrypted DB, keyed by `(account_id, NodeId)`.
- Nickname, self-asserted name, verified flag, timestamps stored per row.
- No addresses/tickets stored here beyond the NodeId; reconnection goes through
  DD-02 discovery/dial using the NodeId.

## Security

- **Trust decisions key on NodeId, never on a name.** Both the local nickname and
  the peer's self-asserted name are display concerns. Authorization ("is this a
  known contact?") must read the NodeId column only. A review must confirm no
  code path treats a name match as identity.
- **Don't let names spoof the UI.** The peer's self-asserted name is untrusted
  input rendered in the user's app. Apply the same validation/sanitization as
  DD-01's display name (length bound; strip/reject control, newline, and bidi
  override characters) before rendering, so a malicious peer can't forge a
  convincing "verified"-looking row or impersonate another contact's nickname.
  The local nickname is user-supplied but gets the same validation for
  consistency.
- **Unverified must look unverified.** The badge must be visually unmistakable and
  must never be suppressible by anything the peer controls (name, length,
  unicode tricks). Verified status changes only via `contacts_set_verified`,
  which only the local user can invoke.
- **Honest "verified" semantics.** The UI copy around the verify action must not
  claim cryptographic verification v1 doesn't perform; overclaiming is itself a
  security problem because it induces misplaced trust.
- **Removal semantics are explicit, and destruction is opt-in.**
  `contacts_remove` always removes the trust relationship (the NodeId reverts to
  "unknown" and DD-02 re-prompts on next inbound). The confirmation also offers
  an explicit **"also delete our message history"** option that is **off by
  default**, so losing history is never the silent side effect of removing a
  contact. When that option is on, removal additionally invokes DD-05's history
  deletion for that peer (same irreversible semantics and page-reclaim as a
  direct history clear). The confirmation copy must make both outcomes — trust
  removed always, history removed only if opted in — unmistakable, since one is
  irreversible.
- **No contact data in diagnostics.** NodeIds may appear in logs (DD-05 policy);
  nicknames and self-asserted names should not be logged (they are personal and
  unnecessary for diagnostics).

## Testing

- **Unit — add/known:** adding a NodeId makes `contacts_is_known` true for it and
  false for a different NodeId.
- **Unit — nickname independence:** renaming the local nickname does not change
  the stored NodeId or the peer's self-asserted name; a peer changing its
  self-asserted name does not change the local nickname.
- **Unit — verify toggle:** only `contacts_set_verified` changes verified state;
  nothing a peer sends can flip it.
- **Unit — name sanitization:** control/newline/bidi/over-length names are
  rejected or neutralized for both nickname and self-asserted name before
  storage/render.
- **Unit — removal reverts trust:** after `contacts_remove`, `contacts_is_known`
  is false and DD-02 would re-prompt.
- **Unit — removal + clear-history flag:** `contacts_remove` without the flag
  leaves the peer's messages intact (default); with the flag set, the contact is
  gone *and* DD-05 history deletion ran for that peer. The flag defaults off.
- **Integration (two nodes):** accept an unknown peer with a nickname → it is
  listed as an unverified contact; reconnect connects silently (DD-02 check
  passes).
- **Namespacing:** contacts created under one `account_id` are invisible to
  queries under another.

## Definition of done

- [x] Contacts persist keyed by `(account_id, NodeId)` in the encrypted DB.
      (Deviation S: interim plaintext `contacts.json` behind the `ContactStore`
      trait; DD-05 replaces it with the encrypted table.)
- [x] A peer can be added from a LAN discovery, a ticket connection, and an
      inbound Accept — all converging on the same unverified contact record.
- [x] Local nickname is editable and fully independent of the peer's
      self-asserted name.
- [x] Unverified/verified states exist; verify is an explicit, honestly-labeled
      user attestation; the badge can't be spoofed by peer-controlled data.
- [x] Contact rows render name-bold / NodeId-gray-beneath with an unverified
      badge where applicable. (Rows show only the local nickname, deviation B;
      the "open chat" row action is deferred to DD-04, deviation O.)
- [x] `contacts_is_known` backs DD-02's trust gate correctly.
- [x] Removal reverts trust and its confirmation offers an opt-in
      (default-off) "also delete message history" option that delegates to DD-05;
      both outcomes are clearly surfaced before confirming. (Deviation H: the
      `HistoryEraser` seam has a no-op production eraser until DD-04/DD-05.)
- [x] All tests above pass; `core::contacts` joins the workspace with real
      content only.
