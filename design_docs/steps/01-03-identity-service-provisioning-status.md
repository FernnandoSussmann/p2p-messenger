# Step 01-03 — Identity service: keygen, atomic provisioning, status, load, display name, my-info

**Design doc:** [DD-01 — Identity, Onboarding & My Info](../01-identity-onboarding-and-my-info.md)
**Crate(s):** `core`

## Dependency & ordering

- **Type: SEQUENTIAL.** This step is the orchestrator that composes every boundary, so its sub-parts have
  real ordering dependencies and must **not** be parallelized.
- **Depends on:** 01-01 (secret types, marker, validation) **and** 01-02 (keychain/settings/ticket
  boundaries).
- **Blocks:** 01-04 (Tauri commands call this service).

## Goal of this step

Deliver the complete, UI-agnostic `identity` service: generate the iroh keypair + random 256-bit DB key,
provision them **atomically** (keychain first, both confirmed, `account_id` marker LAST, best-effort
rollback on failure), detect status (present / absent / error-state) without ever silently re-provisioning,
load an existing identity, set/update the display name, and assemble "My Info" (display name, NodeId, live
ticket, QR). After this step `core::identity` is a complete, fully-tested slice. The crate compiles and the
whole DD "Testing" matrix (minus the Tauri-boundary leak guard, which lands in 01-04) passes.

## Files

- `core/src/identity/service.rs` (new) — the `IdentityService` orchestrator and its error types.
- `core/src/identity/keygen.rs` (new) — iroh `SecretKey` generation + DB-key generation (needs `rand`).
- `core/src/identity/mod.rs` (edit) — declare `mod service; mod keygen;` and re-export the public surface.
- `core/tests/identity_integration.rs` (new) — two-in-process-node integration + round-trip tests.
- Root `Cargo.toml` (edit) — add **only** `rand = "0.9"` (or the version matching the pinned iroh's RNG
  trait) to `[workspace.dependencies]`. The iroh key-type crate (`iroh` or `iroh-base`) is **already added in
  01-02** (Unit C uses its public-key type as `NodeId`); use the SAME crate 01-02 chose here — do not add it a
  second time. **Confirm the matching `rand`/`CryptoRng` trait against Context7** — `SecretKey::generate`
  takes a `CryptoRng`, and the `rand` major must match what iroh re-exports.

## Responsibilities

### `keygen.rs`
- `generate_identity_key() -> (SecretKeyBytes, NodeId)`: create `iroh::SecretKey::generate(&mut rand::rng())`
  (OS CSPRNG), derive the public key via `.public()` as the `NodeId`, and return the secret as the
  `SecretKeyBytes` zeroizing newtype (`secret.to_bytes()`), wiping the live `SecretKey` promptly. Also provide
  `node_id_from_secret_bytes(&SecretKeyBytes) -> NodeId` used by `load` (reconstruct `SecretKey::from_bytes`,
  take `.public()`), so loading never needs to cache the NodeId insecurely.
- `generate_db_key() -> DbKey`: 32 bytes from a CSPRNG (`rand::rng().fill_bytes`) into the `DbKey` zeroizing
  newtype. **Independent** of the identity key (DD trade-off: independent keys for blast-radius containment).

Illustrative only:

```rust
// illustrative — confirm iroh/rand versions
let sk = iroh::SecretKey::generate(&mut rand::rng());
let node_id: NodeId = sk.public();
let bytes = SecretKeyBytes::from_bytes(sk.to_bytes()); // zeroizing newtype
```

### `service.rs` — `IdentityService<K, S, T>`
Generic over the three boundaries (`KeychainStore`, `SettingsStore`, `TicketProvider`) so tests inject fakes
and 01-04 injects the OS-backed ones. Holds the resolved app-data dir (`&Path`) for the marker.

- `status(&self) -> IdentityStatus` returning an enum **exactly matching the DD status matrix**:
  - marker **absent** → `Absent`.
  - marker **present** + both secrets loadable → `Present { account_id, node_id }`.
  - marker present + a secret `NotFound`/`AccessDenied`/`Backend`, or marker corrupt → `Error(recoverable
    reason)`. **Never `Absent`, never a fresh provision.**
- `provision(&self) -> Result<Provisioned, ProvisionError>` — the atomic first-run path:
  1. **Reject if already provisioned** (idempotence): if `status()` is `Present`, return
     `ProvisionError::AlreadyProvisioned`; if `Error`, return the error (do not overwrite). This is the DD
     "provisioning idempotence" rule — running provision when an identity exists is rejected, not duplicated.
  2. Generate identity key (+ NodeId) and DB key.
  3. `store_identity_key`, then `store_db_key` — **both confirmed stored** before touching the marker.
  4. Open/initialize the settings store with the DB key and write the provisioning timestamp
     (`set_provisioned_at`). (Display name is captured later, step 2 of first-run.)
  5. **Write the `account_id` marker LAST** (atomic temp+rename from 01-01). Only now is the identity
     "visible" to `status()`.
  6. On ANY failure in steps 3–5: **best-effort roll back** — delete whichever keychain entries were written,
     and if the marker write itself failed, ensure no marker remains. Return a typed `ProvisionError`.
  - Returns `Provisioned { account_id, node_id }` — **never any secret**.
- `load(&self) -> Result<LoadedIdentity, LoadError>`: given a present marker, read `account_id`, load the
  identity key from the keychain, derive the NodeId, confirm the DB key is present (open settings). Returns
  `{ account_id, node_id }` (+ a handle to the settings/ticket seams). No secret leaves this type.
- `set_display_name(&self, name: &str) -> Result<DisplayName, SetNameError>`: validate via 01-01
  `validate_display_name`, persist through `SettingsStore`. Used for both first-run capture and later edits
  (overwrite). Invalid input returns a typed validation error (the UI cannot proceed on first run until this
  succeeds).
- `my_info(&self) -> Result<MyInfo, MyInfoError>`: assemble `{ display_name: Option<DisplayName>, node_id,
  ticket, qr_svg }` by reading the display name from settings, taking the NodeId from the loaded identity,
  calling `TicketProvider::current_ticket` **live**, and `ticket_to_qr_svg`. Returns only public data.
- All of `Provisioned`, `LoadedIdentity`, `MyInfo` and every error type: **no field is a secret** and none
  derive a `Debug`/`Serialize` that could carry one (they only carry NodeId/name/ticket/account_id).

Re-export from `mod.rs`: `IdentityService`, `IdentityStatus`, `Provisioned`, `LoadedIdentity`, `MyInfo`, and
the error enums.

## Tests

Map 1:1 to the DD "Testing" section. Unit tests live in `service.rs`; cross-node tests in
`core/tests/identity_integration.rs`. **All use the in-memory `FakeKeychain`/mock store + a temp-dir marker +
`MemorySettingsStore` — no real secret or real OS keychain.**

> **Test-fake reachability:** `core/tests/identity_integration.rs` is a separate crate, so it cannot see a
> `#[cfg(test)]` `FakeKeychain` defined inside `core/src`. Because the integration test needs `FakeKeychain`
> (and `MemorySettingsStore`), 01-02 exposes these via the feature-gated `pub mod testing`
> (`#[cfg(any(test, feature = "test-support"))]`); enable that `test-support` feature for the integration
> target (e.g. a `core` dev-dependency on itself with `features = ["test-support"]`, or run the suite with
> `--features test-support`). `MemorySettingsStore` is already public. State the chosen mechanism in the step.

- **Unit — provisioning idempotence:** provision once; a second `provision()` on the same service returns
  `AlreadyProvisioned` and does not create a second identity (marker + keychain entries unchanged, NodeId
  identical).
- **Unit — status detection matrix:** three cases — (marker absent → `Absent`), (marker + both secrets →
  `Present`), (marker present + identity-key `NotFound` → `Error`, asserting **not** `Absent`). Add a fourth:
  marker present + `AccessDenied` → `Error`.
- **Unit — partial-write rollback:** inject a `FakeKeychain` that **fails on the second secret**
  (`store_db_key`). Assert: `provision` returns `Err`, **no `account_id` marker was written**, and the first
  secret (identity key) was **rolled back** (keychain empty afterward).
- **Unit — display-name validation:** delegate-through test — empty/whitespace/over-length/control/newline/
  bidi rejected by `set_display_name`; valid names incl. emoji accepted and read back verbatim from settings.
- **Unit — no-silent-re-provision:** marker present but keychain cleared → `status()` is `Error`, and
  `provision()` refuses (returns the error), proving a cleared keychain never yields a new identity.
- **Property / round-trip:** for many random seeds, `provision` then `load` yields the **same NodeId**; and
  `SecretKeyBytes` → `SecretKey::from_bytes` → `.public()` equals the NodeId returned by provision.
- **Integration (two in-process nodes):** build two `IdentityService` instances with **isolated** fake
  keychains (distinct namespaces) and distinct temp marker dirs; provision both; assert each exposes a
  distinct NodeId and a ticket, and that node A can `Ticket::parse` node B's ticket back to B's NodeId
  (hands off to DD-02 tests; here it proves the seam is real and parseable).
- **Secret-leak guard (core level, automated):** a test that exercises `provision`, `load`, `my_info` and
  asserts none of the returned structs' `Debug` output nor any value they expose contains the raw key bytes
  (seed the fake keygen with a known byte canary and assert its absence in all public outputs). The
  Tauri-return-type/log slice of this guard is completed in 01-04.

## Guardrails

- **Secret confidentiality:** secrets exist only as 01-01 zeroizing newtypes inside `keygen`/`service`, are
  handed to the keychain, and are dropped (zeroized) immediately. No `Provisioned`/`LoadedIdentity`/`MyInfo`
  field is a secret; nothing is logged with a secret; nothing is serialized.
- **Partial-write safety + rollback ordering:** keychain ×2 confirmed FIRST, `account_id` marker LAST;
  best-effort rollback of written keychain entries on failure (tested by the injected-failure case).
- **No silent re-provision:** `status()` maps missing/unreadable secrets (or corrupt marker) to `Error`, and
  `provision()` refuses when not `Absent`.
- **Display-name validation:** enforced in `set_display_name` via the single 01-01 validator; store verbatim.
- **Fail-closed on keychain access denial:** `AccessDenied` from the keychain propagates to `status()`/`load`
  as an `Error`/`LoadError`, never a silent success.

## Security (prevent + verify)

- **Secret leakage via return types/logs.** *Prevent:* public structs carry only NodeId/name/ticket/
  account_id; zeroizing newtypes never escape `service`. *Verify:* core secret-leak-guard test with a byte
  canary; `grep -RnE 'println!|dbg!|tracing::' core/src/identity` reviewed to confirm no secret value is
  formatted.
- **Half-identity mistaken for corruption / silent data loss.** *Prevent:* marker written last + rollback.
  *Verify:* partial-write-rollback test (no marker, first secret rolled back).
- **Silent re-provision orphaning the encrypted DB / changing the address without consent.** *Prevent:*
  `Error` state for marker-present-secret-missing; provision refuses unless `Absent`. *Verify:*
  no-silent-re-provision + status-matrix tests.
- **Non-independent keys widening blast radius.** *Prevent:* DB key generated independently of the identity
  key. *Verify:* code review + a test asserting the two generated byte arrays differ and the DB key is not
  derived from the identity key.
- **Weak randomness.** *Prevent:* OS CSPRNG (`rand::rng()`/`OsRng`) for both keys. *Verify:* review that no
  seeded/deterministic RNG is used in non-test code (tests may inject a fake keygen, clearly `#[cfg(test)]`).

## Credentials / secrets rule

NEVER hardcode credentials, keys, passphrases, or secrets. Both keys are generated at **runtime** from the
OS CSPRNG and stored only in the OS keychain (via 01-02). Tests inject an in-memory fake keychain + temp-dir
marker + in-memory settings; **no test writes a real secret to disk or the OS keychain, and none commits key
bytes** (the byte canary used by the leak guard is a non-secret test fixture, not a real key).

## Quality gates (warnings are errors)

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `grep -RnE 'println!|dbg!' core/src/identity` returns nothing (no debug prints in the slice).

## Definition of Done (verifiable checklist)

- [ ] First-run `provision()` generates keypair + 256-bit DB key, stores both in the keychain, writes the
      `account_id` marker **last** — atomic w.r.t. `status()`. *(DD DoD bullet 1.)*
- [ ] `status()`/`load()` detect and load an existing identity (the backend half of "subsequent launches …
      no prompt"). *(DD DoD bullet 2.)*
- [ ] Missing/unreadable secrets → `Error` state; `provision()` refuses; never a silent re-provision. *(DD DoD bullet 3.)*
- [ ] `set_display_name` validates and persists (first-run + edit) keyed by `account_id` via the settings
      seam. *(DD DoD bullet 4.)*
- [ ] `my_info()` returns display name + NodeId + live ticket + QR SVG, all public. *(DD DoD bullet 5 — backend.)*
- [ ] No secret appears in any return struct, `Debug`, or log (core slice of the leak guard passes). *(DD DoD bullet 6 — core.)*
- [ ] Every DD "Testing" item is implemented and green: idempotence, status matrix, partial-write rollback,
      display-name validation, two-node integration, property round-trip, secret-leak guard. *(DD DoD bullet 7 — core tests.)*
- [ ] `core::identity` is now a complete slice with real content only. *(DD DoD bullet 8.)*
- [ ] `fmt`/`clippy -D warnings`/`test --workspace` green; no hardcoded secrets.
