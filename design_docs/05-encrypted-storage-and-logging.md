# DD-05 — Encrypted Storage & Diagnostic Logging

**Feature slice:** cross-cutting `core` storage + logging infrastructure
**Status:** Draft (v1)
**Related:** [DD-01 Identity](01-identity-onboarding-and-my-info.md),
[DD-03 Contacts & Trust](03-contacts-and-trust.md),
[DD-04 Messaging](04-messaging-and-delivery.md)

---

## Context and scope

Every other slice persists something — settings (DD-01/DD-02), contacts (DD-03),
messages (DD-04) — and every slice emits diagnostics. This doc defines the two
cross-cutting substrates they share: an **encrypted-on-disk database**
(SQLCipher, keyed from the OS keychain) and a **diagnostic logging** pipeline
(`tracing` → rotating local file) with a hard rule that message plaintext never
reaches it.

Scope:

- Opening/initializing the SQLCipher database with the key DD-01 stored in the
  keychain.
- The schema's shape and the `account_id` namespacing convention all slices use.
- Schema evolution (migrations) and integrity handling.
- History queryability: scrollback + text search over messages.
- The `tracing` logging pipeline: what is logged, rotation, location, and the
  plaintext-exclusion guardrail.
- History/data deletion semantics.

It does **not** define the specific columns each feature needs (those live in the
owning docs) — it defines *how* persistence and logging work and the invariants
they must uphold.

## Goals and non-goals

### Goals
- A single encrypted DB file per install, unlocked by a key held only in the OS
  keychain, that is still fully queryable (search/scrollback) while encrypted at
  rest.
- One namespacing convention (`account_id` on every row) so a future
  multi-identity release needs no migration.
- A forward-compatible migration mechanism so the schema can evolve across
  releases without data loss.
- Structured diagnostic logging of connection/hole-punch/relay events (NodeIds +
  addresses) to a rotating local file.
- An enforced guarantee that **no message plaintext** (and no secret) is ever
  logged.

### Non-goals
- **Passphrase-protecting the DB key** — deferred; the keychain is the anchor
  (DD-01). The key-wrapping seam is left open but unused in v1.
- **Remote/centralized log aggregation or telemetry** — logs are local only;
  nothing is sent off-device.
- **Cross-device DB sync or cloud backup** — local file only.
- **Full-text search ranking / fuzzy search** — v1 provides substring/keyword
  search sufficient for scrollback, not a search engine.
- **Automatic PII scrubbing of free text** beyond simply never logging message
  bodies.

## The actual design

### Overview

```
OS keychain ──(DB key, from DD-01)──▶ open SQLCipher ──▶ encrypted DB file
                                              │
          ┌───────────────────────────────────┼───────────────────────────┐
     settings (DD-01/02)             contacts (DD-03)            messages (DD-04)
          └───────────── all rows namespaced by account_id ─────────────────┘

tracing spans/events ──▶ redaction-by-construction ──▶ rotating log file (local)
         (NodeIds, addrs, event kinds)        (never: bodies, keys, tickets' secrets)
```

### Database: opening & keying

- On startup, after DD-01 confirms identity, the DB key is fetched from the
  keychain and used to open the SQLCipher database file in the OS app-data
  directory. The key is held in memory only for as long as the connection lives
  and is never persisted outside the keychain or logged.
- If the key is missing/wrong (DD-01 error-state), the DB is not opened and the
  app surfaces the recoverable error rather than creating a fresh unencrypted or
  differently-keyed DB.
- Encryption is SQLCipher's whole-file encryption; the app issues normal queries
  against a decrypted-in-memory view, so search and scrollback work exactly as on
  a plaintext DB while the file at rest is ciphertext.

**Trade-off — SQLCipher vs. app-level field encryption.** We encrypt the whole
DB file rather than individual fields so that *all* data (including indexes used
for search) is protected at rest with no per-query crypto and no loss of
queryability. The cost is that an attacker who obtains the running process's
memory or the live key sees everything; that is accepted for v1 (the keychain +
OS user account is the boundary), and is exactly why no passphrase layer is
promised yet.

### Schema shape & namespacing

- Tables (owned by their slices, defined here only in shape): an
  `account`/settings table (DD-01/DD-02), `contacts` (DD-03), `messages` (DD-04).
- **Every row carries `account_id`.** Every query filters on it. v1 has one
  account, but this makes the single-identity assumption a *data* convention, not
  a structural one — a second identity is new rows, not a new schema.
- Message storage is indexed for (a) per-peer chronological scrollback and (b)
  text search over bodies; `MessageId` is unique per peer to make DD-04's dedupe
  a storage-enforced invariant.

### Migrations & integrity

- A versioned migration mechanism runs on startup: the DB records its schema
  version; the app applies ordered, forward-only migrations to reach the current
  version. Migrations are idempotent and transactional — a failure rolls back and
  leaves the prior version intact.
- Integrity: an unreadable/corrupt DB (or a SQLCipher key that fails to decrypt)
  is reported as a distinct, recoverable error state; the app never silently
  discards or recreates the user's history.

### History queryability

- **Scrollback:** paged queries per peer, newest-first windows, backing DD-04's
  `chat_history`.
- **Search:** keyword/substring search over message bodies within an account,
  returning matches with enough context to locate them in a conversation. Runs
  against the decrypted in-memory view; results never leave the device.

### Data deletion

- Deleting a conversation / clearing history is an explicit, irreversible user
  action that removes the message rows for a peer (and should be followed by a
  SQLCipher-level reclaim so deleted ciphertext isn't trivially recoverable from
  free pages where feasible).
- History deletion is this slice's clearly-labeled action, reachable two ways:
  directly via `history_clear`, or from DD-03's remove-contact confirmation,
  which offers an opt-in (default-off) "also delete message history" choice that
  invokes the same deletion path for that peer. Removing a contact without that
  option leaves history intact.

### Logging pipeline

- `tracing` is the single logging facade across `core`. Events are structured
  (event kind + typed fields), written to a **rotating** file in the OS
  app-data/log directory with a bounded total size / file count so logs can't
  grow without limit.
- **What is logged:** connection lifecycle, hole-punch attempts/outcomes, relay
  usage, discovery events, errors — identified by NodeId and network address,
  which are necessary to diagnose connectivity.
- **What is never logged:** message bodies, `Hello`/nickname display names
  (personal, unneeded), the identity private key, the DB key, and the secret
  interior of any ticket. See Security for enforcement.
- Logs are local only; no network sink.

### API sketch

Storage and logging are mostly internal `core` facilities, not Tauri commands.
The thin surfaces exposed:

| Surface | Purpose |
|---------|---------|
| storage init (internal) | Open/migrate the keyed DB; return a handle or a typed error state |
| `history_search` (Tauri) | Keyword search over the account's messages (backs UI search) |
| `history_clear` (Tauri) | Delete a peer's conversation history (explicit, confirmed) |
| log config (internal/config) | Log level, rotation bounds, directory |

## Security

- **DB key hygiene.** The SQLCipher key comes only from the keychain, lives only
  in memory while open, and is never logged, returned to the webview, or written
  to disk. A review must confirm the key never appears in a log line, error
  message, or Tauri return.
- **Plaintext never hits disk unencrypted.** The only on-disk home for message
  content is inside the SQLCipher-encrypted file. No temp files, no debug dumps,
  no log lines with bodies. This is the central invariant of this slice.
- **Logging redaction by construction.** Rather than scrubbing strings after the
  fact, the logging layer is built so that message bodies and secrets are simply
  never passed into `tracing` fields. Provide a reviewed set of logging helpers;
  forbid logging raw payloads. Add an automated guard (test or lint) that fails
  if a `Chat` body, key, or display name is passed to a log macro in `core`.
- **Log file is sensitive-ish.** Even without plaintext, logs reveal who you
  talked to (NodeIds) and when (metadata). Store them with restrictive file
  permissions in the user's app dir, bound their retention via rotation, and
  document that clearing them is a privacy action. Never transmit them.
- **Error messages don't leak.** Storage/keychain errors surfaced to the UI or
  logs must describe the failure class, not embed secrets, key bytes, or message
  content.
- **Corruption ≠ reset.** A failed decrypt or integrity check must never trigger
  silent recreation of the DB; that would be indistinguishable from catastrophic
  data loss and could mask tampering. Fail into a recoverable, user-visible
  state.
- **Deletion actually deletes.** `history_clear` must remove rows and, as far as
  SQLCipher/SQLite allow, reclaim the pages so the ciphertext of deleted messages
  isn't left lingering; the UI must be honest about what is and isn't guaranteed.
- **Namespacing is a safety boundary too.** Every query filters by `account_id`;
  a missing filter is a latent cross-identity leak for the multi-identity future
  and should be caught in review/tests.

## Testing

- **Keyed open round-trip:** a DB created with the keychain key reopens with that
  key and rejects a wrong key (reported as recoverable error, not reset).
- **Encrypted at rest:** the raw DB file on disk does not contain a known
  plaintext message body (byte-scan asserts ciphertext).
- **Migration forward:** an older-schema DB migrates to current with all rows
  intact; a failing migration rolls back and preserves the prior version.
- **Namespacing:** queries under `account_id` A never return rows written under
  B.
- **Scrollback paging:** per-peer history pages correctly, newest-first, stable
  across reloads.
- **Search:** keyword search returns the right messages and nothing from other
  accounts; results stay on-device.
- **History deletion:** `history_clear` removes the peer's rows; a best-effort
  check that reclaimed pages don't surface the deleted plaintext.
- **Log redaction guard (automated):** attempting to log a message body, the DB
  key, the identity key, or a display name fails the guard/lint; a normal
  connection event logs NodeId + address as expected.
- **Log rotation:** logs rotate and stay within the configured size/count bound;
  no unbounded growth.
- **No network sink:** assert the logging layer has no remote/transport target.

## Definition of done

- [ ] One SQLCipher DB per install opens with the keychain-held key; wrong/missing
      key yields a recoverable error, never a silent reset.
- [ ] Data is encrypted at rest yet fully queryable; scrollback and keyword
      search work on-device.
- [ ] Every row is `account_id`-namespaced and every query filters on it.
- [ ] Forward-only, transactional migrations evolve the schema without data loss.
- [ ] `tracing` logs connection/hole-punch/relay/discovery events (NodeIds +
      addresses) to a size-bounded rotating local file.
- [ ] Message bodies, display names, and all secrets are provably never logged,
      enforced by an automated guard in the mandatory local pre-commit test gate
      (see README — no CI is used).
- [ ] History deletion removes rows and reclaims pages as far as feasible, with
      honest UI about guarantees.
- [ ] Logs are local-only, permission-restricted, and never transmitted.
- [ ] All tests above pass; storage/logging live in `core` with real content
      only (no stubs).
