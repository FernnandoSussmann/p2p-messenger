# Step 01-02 — Boundaries: keychain store, settings store (DD-05), ticket provider (DD-02)

**Design doc:** [DD-01 — Identity, Onboarding & My Info](../01-identity-onboarding-and-my-info.md)
**Crate(s):** `core`

## Dependency & ordering

- **Type: SEQUENTIAL w.r.t. 01-01 (needs its types), but internally contains THREE PARALLEL units.**
- **Depends on:** 01-01 (uses `DbKey`, `SecretKeyBytes`, `AccountId`, `DisplayName`).
- **Blocks:** 01-03 (provisioning composes all three boundaries).

Each unit owns exactly one new file and depends only on 01-01's public types — **they share no files and
have no ordering between them.** Spin up a worktree + sub-agent per unit. The only shared touch is three
`mod`/`pub use` lines appended to `core/src/identity/mod.rs`, merged by the orchestrating agent.

| Unit | Owns (only) file | Purpose |
|------|------------------|---------|
| Parallel Unit A — keychain store | `core/src/identity/keychain.rs` | authoritative OS-keychain boundary (`keyring`) |
| Parallel Unit B — settings store (DD-05 boundary) | `core/src/identity/settings.rs` | trait DD-05 will fulfil + minimal in-slice impl |
| Parallel Unit C — ticket provider (DD-02 boundary) | `core/src/identity/ticket.rs` | trait DD-02 will fulfil + minimal NodeId ticket + QR |

## Goal of this step

Define the three seams the provisioning orchestrator (01-03) sits on top of, with the keychain being a real
`keyring`-backed implementation and the DD-05/DD-02 seams being **traits with a deliberately minimal
in-slice implementation** — just enough for DD-01 to be demonstrable and testable, with no attempt to
re-implement DD-05 storage or DD-02 transport. Crate still compiles and all tests pass; tests use fakes so
**no real secret touches the OS keychain or disk**.

---

## Parallel Unit A — keychain store (`core/src/identity/keychain.rs`)

Add to the workspace deps (in root `Cargo.toml` `[workspace.dependencies]`):
`keyring = { version = "3", features = ["apple-native", "windows-native", "sync-secret-service", "crypto-rust"] }`
(confirm the current major and the correct platform-backend feature names against the pinned `keyring`
version via Context7 before finalizing — feature names have changed across keyring majors). **The
`crypto-rust` feature — or `crypto-openssl` — is REQUIRED for the Linux Secret Service backend in keyring
3.x; `sync-secret-service` alone will not build on Linux**, which is a v1 target. `core/Cargo.toml`
gets `keyring.workspace = true`.

Responsibilities:
- **This is the single module boundary that owns the service/account naming scheme.** Service =
  `<app-id>` (define a `const APP_KEYCHAIN_SERVICE: &str`, e.g. `"com.p2p-messenger.identity"`). Entries are
  **per-`account_id`**: build the keyring entry *account/user* string from the `AccountId` plus a fixed
  suffix per secret kind, e.g. `"<account_id>/identity-key"` and `"<account_id>/db-key"`. Document the
  scheme in one place here and nowhere else.
- Define a `KeychainStore` trait so provisioning and tests can be written against an abstraction:

```rust
// illustrative
pub trait KeychainStore {
    fn store_identity_key(&self, id: &AccountId, key: &SecretKeyBytes) -> Result<(), KeychainError>;
    fn store_db_key(&self, id: &AccountId, key: &DbKey) -> Result<(), KeychainError>;
    fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError>;
    fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError>;
    fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError>;
    fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError>;
}
```

- `KeychainError` **must distinguish** at least: `NotFound` (entry absent — maps keyring's `NoEntry`),
  `AccessDenied` (user denied the OS keychain prompt / locked keychain — **fail closed**), and
  `Backend(..)` (other). This distinction is what lets 01-03 decide "absent" vs "error-state" and never
  silently re-provision.
- `OsKeychain` struct implementing the trait over `keyring::Entry`. Use **binary** secret APIs
  (`entry.set_secret(&bytes)` / `entry.get_secret() -> Vec<u8>`), not the password/string APIs, because the
  key material is raw bytes. Wrap any transient `Vec<u8>` from `get_secret` in `Zeroizing` and convert into
  the 01-01 zeroizing newtypes immediately. Illustrative:

```rust
// illustrative — confirm keyring 3.x API against the pinned version
let entry = keyring::Entry::new(APP_KEYCHAIN_SERVICE, &db_key_account(id))?;
entry.set_secret(key.as_bytes())?;          // store
let raw = zeroize::Zeroizing::new(entry.get_secret()?); // load, wiped on drop
```

- Map `keyring::Error::NoEntry` → `KeychainError::NotFound`; map lock/denied errors → `AccessDenied`.

Re-export: `pub use keychain::{KeychainStore, KeychainError, OsKeychain};`

Tests — **must not touch the real OS keychain**:
- Use `keyring`'s built-in **mock store** (set the mock credential builder as the default store in the test)
  so set/get/delete run entirely in memory; confirm the mock-store entry point name against the pinned
  keyring version. If the installed keyring version exposes no in-process mock, provide a `FakeKeychain`
  (an in-memory `HashMap`-backed `KeychainStore`) and test `OsKeychain` only behind an `#[ignore]` manual
  test. State which path was taken.
- **Test-fake visibility (REQUIRED if `FakeKeychain` is also needed by 01-03's integration test):** a
  `#[cfg(test)]` item in `keychain.rs` is compiled only for `core`'s own unit tests and is **not reachable**
  from integration tests under `core/tests/` (a separate crate). `MemorySettingsStore` (Unit B) is public and
  fine; the keyring mock store is a global default and fine. But if the fail-injecting `FakeKeychain` must be
  shared with the two-node integration test, expose it via a cargo-feature-gated public module rather than
  `#[cfg(test)]` — e.g. `#[cfg(any(test, feature = "test-support"))] pub mod testing;` in `core` (declare a
  `test-support` feature in `core/Cargo.toml`), and have `core/tests/` enable it as a dev-dependency feature.
  State explicitly in the step which fakes cross the unit/integration boundary.
- `store_then_load_round_trips` identity key and db key (bytes equal).
- `load_missing_is_not_found` returns `KeychainError::NotFound`, not a panic or empty value.
- `delete_removes_entry` then load → `NotFound`.
- `naming_scheme_is_per_account`: two different `AccountId`s produce different entry names and do not collide.

---

## Parallel Unit B — settings store, the DD-05 seam (`core/src/identity/settings.rs`)

The display name + provisioning timestamp live in the **encrypted DB (DD-05)** keyed by `account_id`. DD-05
is **not built yet**. This unit defines the seam and ships a minimal in-slice implementation so DD-01 is
demonstrable — it does **not** re-implement SQLCipher or the full DD-05 schema.

Responsibilities:
- `SettingsStore` trait (the seam DD-05 will later fulfil with a SQLCipher-backed impl):

```rust
// illustrative
pub trait SettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError>;
    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError>;
    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError>;
    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError>;
}
```

- A minimal implementation sufficient for this slice. Note the chicken-and-egg the DD calls out: the DB key
  exists (created in 01-03 step 1) *before* the display name is captured (step 2), so the encrypted store is
  already openable. Model this honestly: the minimal impl is a small **file-backed JSON store encrypted at
  rest is NOT required here** — but it MUST be opened with/guarded by the `DbKey` conceptually so the seam is
  faithful. Ship either (a) an in-memory `MemorySettingsStore` used for tests + first demo, or (b) a tiny
  JSON-file store keyed by `account_id` that takes the `DbKey` in its constructor (even if the minimal impl
  only *holds* the key for DD-05 to later use). **Document explicitly** that this is a narrow placeholder the
  DD-05 slice will replace, store display name and timestamp only, and store no secret in it.
- `SettingsError` typed enum.

Re-export: `pub use settings::{SettingsStore, SettingsError, MemorySettingsStore};` (plus the file-backed
impl if chosen).

Tests:
- `set_get_display_name_round_trips` (stores the validated `DisplayName` verbatim).
- `overwrite_display_name` (edit-later path works; last write wins).
- `provisioned_at_round_trips`.
- `missing_returns_none` (not an error).
- If a file-backed impl is shipped: assert the written file contains the display name/timestamp and
  **contains no key bytes** (a leak-guard over the settings file).

---

## Parallel Unit C — ticket provider, the DD-02 seam (`core/src/identity/ticket.rs`)

The "My Info" connection ticket (NodeId + relay + direct addrs) is **owned by DD-02**, which is not built
yet. This unit defines the seam and ships a minimal implementation returning at least a **NodeId-based
ticket** so the My Info tab is functional and the screen is reachable — it does **not** stand up an iroh
endpoint or implement transport/discovery (that is DD-02).

Add deps to the workspace `[workspace.dependencies]` + `core/Cargo.toml`:
- `qrcode = { version = "0.14", features = ["svg"] }` (confirm current version/feature name).
- The iroh key-type crate, because this unit re-exports `iroh`'s public-key type as `NodeId` and its test
  constructs a known `NodeId`: `iroh-base = { version = "..." }` (just the `PublicKey`/`SecretKey` key types,
  no endpoint) **or** the full `iroh` crate — **pick ONE and use the same one in 01-03** when the live
  `SecretKey`→`NodeId` derivation is added. Confirm the current major and the exact crate/module path for the
  public-key type against Context7. This step is the FIRST to introduce the iroh dependency; do not defer it
  to 01-03, or `core` will reference a type from an undeclared crate and 01-02's `cargo test --workspace`
  gate will fail to compile.

Responsibilities:
- `TicketProvider` trait (DD-02 will later own the real impl that includes live relay/direct addrs):

```rust
// illustrative
pub trait TicketProvider {
    /// Assemble the current shareable ticket. DD-02's impl regenerates this
    /// LIVE from endpoint state each call; this slice returns a NodeId-only form.
    fn current_ticket(&self, node_id: &NodeId) -> Result<Ticket, TicketError>;
}
```

- `Ticket` is a string-serializable blob (`as_str()` / `Display`) — the public shareable value. A
  `NodeIdTicketProvider` minimal impl that encodes **just the NodeId** into the ticket string form (document
  that relay/direct addrs are intentionally absent until DD-02). It must be parseable back to the NodeId so
  the DD "two in-process nodes can parse each other's ticket" integration test (in 01-03) has something real
  to exercise; provide `Ticket::parse(&str) -> Result<ParsedTicket, TicketError>` returning at least the
  NodeId.
- `ticket_to_qr_svg(&Ticket) -> Result<String, TicketError>`: render the ticket string to an **SVG string**
  via `qrcode` (`QrCode::new(ticket.as_str().as_bytes())?.render::<svg::Color>().build()`). **Display only** —
  no scanning. Keeping QR generation in Rust makes it unit-testable; the frontend just renders the returned
  SVG.
- `NodeId` type: re-export iroh's public-key type (`iroh::PublicKey` / `iroh_base::PublicKey`) as the
  `NodeId` so the format is correct. **Do not** create an endpoint; only use the key type. (The live
  `iroh::SecretKey` → `NodeId` derivation happens in 01-03.)

Re-export: `pub use ticket::{TicketProvider, Ticket, NodeIdTicketProvider, ticket_to_qr_svg, NodeId};`

Tests:
- `ticket_round_trips_node_id`: build ticket from a known NodeId, `parse` it back, assert same NodeId.
- `qr_svg_is_wellformed`: output starts with `<svg` / contains `</svg>` and is non-trivially sized.
- `ticket_contains_no_secret`: the ticket string and QR SVG contain only public data (assert a canary
  secret-looking byte pattern is absent — this feeds the slice secret-leak guard).

---

## Guardrails

- **Secret confidentiality:** the keychain is the ONLY place the two secrets are persisted. `OsKeychain`
  wraps every `get_secret` result in `Zeroizing` and converts straight into the 01-01 zeroizing newtypes; no
  secret is logged, returned as a plain `Vec`, or stored in the settings store or the ticket. Ticket/QR carry
  public data only.
- **Partial-write safety / rollback ordering:** the `KeychainStore` exposes `delete_*` so 01-03 can roll back
  a partially-written identity. (Ordering — marker LAST — is enforced in 01-03; this step just supplies the
  verbs.)
- **No silent re-provision:** `KeychainError` separates `NotFound` from `AccessDenied`/`Backend`, so 01-03
  can map "marker present + secret `NotFound`/`AccessDenied`" to an error-state, never to "fresh install."
- **Display-name validation:** the settings store stores only an already-validated `DisplayName` (validation
  from 01-01); it does no re-parsing that could reintroduce an invalid value.
- **Fail-closed on keychain access denial:** a denied/locked keychain surfaces as `KeychainError::AccessDenied`
  (never a silent success or empty key).
- **Single `<app-id>` service + per-`account_id` entries** naming scheme lives only in `keychain.rs`.

## Security (prevent + verify)

- **Secret leakage via keychain-adjacent code.** *Prevent:* binary secret APIs + `Zeroizing`, no secret in
  `SettingsStore`/`TicketProvider`. *Verify:* `ticket_contains_no_secret` test; `grep -RnE 'get_secret|set_secret' core/src/identity` returns matches **only** in `keychain.rs`.
- **Keychain prompt bypass / access denied treated as success.** *Prevent:* `AccessDenied` variant, fail
  closed. *Verify:* `load_missing_is_not_found` + a test (via fake) that an injected access-denied error
  propagates as `AccessDenied`, not as a default/empty key.
- **Orphaning the encrypted DB (identity leak into tables).** *Prevent:* settings keyed by `AccountId`, never
  by NodeId; DB key held only to open DD-05's store, never serialized into it. *Verify:* settings-file leak
  test (no key bytes) if a file impl is used; code review of the trait signatures.
- **Ticket spoofing / leaking private data.** *Prevent:* ticket carries only public NodeId (+ later DD-02
  public addrs). *Verify:* `ticket_contains_no_secret` + `ticket_round_trips_node_id`.

## Credentials / secrets rule

NEVER hardcode credentials, keys, passphrases, or secrets in source. The keychain service name
(`com.p2p-messenger.identity`) is an identifier, not a secret. All key bytes are generated/retrieved at
runtime (generation is 01-03). **Tests use the keyring mock store or an in-memory `FakeKeychain` and never
write a real secret to the OS keychain or to disk**, and no test commits key material.

## Quality gates (warnings are errors)

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `grep -RnE 'get_secret|set_secret' core/src/identity` shows matches only in `keychain.rs` (secret I/O is
  localized to the boundary).

## Definition of Done (verifiable checklist)

- [ ] `KeychainStore` + `OsKeychain` persist/load/delete both secrets via `keyring` binary APIs, with the
      `<app-id>`/per-`account_id` naming scheme living only in `keychain.rs`. *(DD DoD: keychain stores both secrets.)*
- [ ] `KeychainError` distinguishes `NotFound` vs `AccessDenied` vs `Backend`. *(DD DoD: "Missing/unreadable secrets surface a recoverable error, never a silent re-provision" — foundation.)*
- [ ] `SettingsStore` seam + minimal impl store display name + provisioning timestamp keyed by `account_id`, with the DD-05 chicken-and-egg noted and no secret stored. *(DD DoD: "persisted in the encrypted DB keyed by `account_id`" — seam + placeholder.)*
- [ ] `TicketProvider` seam + minimal NodeId ticket + `ticket_to_qr_svg`, parseable round-trip. *(DD DoD: "live ticket … and a QR rendering" — seam + placeholder.)*
- [ ] All tests use fakes/mock store; no real secret written to keychain or disk; crate compiles, tests pass.
- [ ] `fmt`/`clippy -D warnings`/`test --workspace` green; secret-I/O-localization grep passes.
