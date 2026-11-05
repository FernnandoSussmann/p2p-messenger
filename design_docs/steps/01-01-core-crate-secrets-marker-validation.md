# Step 01-01 — `core` crate foundation: secret types, `account_id` marker, display-name validation

**Design doc:** [DD-01 — Identity, Onboarding & My Info](../01-identity-onboarding-and-my-info.md)
**Crate(s):** `core` (new — joins the workspace in this step)

## Dependency & ordering

- **Type: SEQUENTIAL (first step — depends on nothing), but internally contains THREE PARALLEL units.**
- **Depends on:** nothing (the `protocol` crate already exists; this step does not touch it).
- **Blocks:** 01-02, 01-03, 01-04, 01-05.

This step has a tiny **shared scaffold prelude** (the crate `Cargo.toml`, `core/src/lib.rs`, and
`core/src/identity/mod.rs` with three `mod` declarations) that must be laid down first. After the
scaffold exists, the three units below **share no files and have no ordering between them** — spin up a
git worktree + sub-agent per unit and implement them simultaneously, then run the gates once on the merged
tree.

| Unit | Owns (only) file | Depends on other units? |
|------|------------------|-------------------------|
| Parallel Unit A — secret types + zeroize | `core/src/identity/secret.rs` | no |
| Parallel Unit B — `account_id` marker file | `core/src/identity/marker.rs` | no |
| Parallel Unit C — display-name validation | `core/src/identity/display_name.rs` | no |

The scaffold's `mod.rs` only declares `mod secret; mod marker; mod display_name;` and re-exports — those
three lines are non-conflicting and are merged by the orchestrating agent, not owned by any unit.

## Goal of this step

Stand up the `core` crate with the first real, independently-testable pieces of the `identity` slice: the
in-memory secret hygiene types, the unencrypted `account_id` marker file, and display-name validation.
These are the leaf building blocks that 01-02 (boundaries) and 01-03 (provisioning) compose. No keychain,
no iroh orchestration, no provisioning yet — just correct, tested primitives. The crate compiles and all
tests pass at the end of this step with **real content only** (no stubs, no empty modules).

## Scaffold prelude (do first, once)

- Add `core` to the workspace: edit `/home/suss/Documents/p2p-messenger/Cargo.toml`
  `members = ["protocol", "core"]`. Add shared deps to `[workspace.dependencies]` that this step needs:
  `zeroize = { version = "1", features = ["zeroize_derive"] }`, and reuse the existing
  `uuid` (already `features = ["v4", "serde"]`). `rand` is **not** needed yet (identity keygen lands in 01-03).
- Create `core/Cargo.toml` (package `core`, inherits `version`/`edition`/`rust-version`/`license` from the
  workspace), depending on `zeroize.workspace = true`, `uuid.workspace = true`, `serde.workspace = true`,
  and dev-dep `serde_json` only if a unit needs it (Unit C does not).
  - **Crate-name footgun — `core` shadows the Rust std `core` crate.** The package name `core` is the one
    BLUEPRINT.MD chose, so keep it unless the user decides otherwise. But a dependent crate (`app`, in 01-04)
    that lists a dependency literally named `core` shadows the std `core` prelude name, making `core::fmt`
    etc. ambiguous there. Mitigation (does NOT change the blueprint name): inside `core` itself this is
    harmless, but **01-04 must import the dependency under a rename** — e.g.
    `messenger_core = { package = "core", path = "../../core" }` — so `app` can still use std `core::…`. In
    illustrative code *inside this crate*, prefer `std::fmt::Formatter` over `core::fmt::Formatter` to avoid
    confusion. (Flagged for the human; the rename is import-site only.)
- Create `core/src/lib.rs`: crate-level `//!` doc in the house style (see `protocol/src/lib.rs` — precise,
  trade-offs explicit), `#![forbid(unsafe_code)]`, and `pub mod identity;`.
- Create `core/src/identity/mod.rs`: module `//!` doc describing the slice boundary, plus
  `mod secret; mod marker; mod display_name;` and the public re-exports each unit specifies below.

---

## Parallel Unit A — secret types + zeroize (`core/src/identity/secret.rs`)

Responsibilities:
- A `DbKey` newtype wrapping a `[u8; 32]` (the 256-bit SQLCipher key that DD-05 will consume) and a
  `SecretKeyBytes` newtype wrapping `[u8; 32]` (the raw ed25519/iroh secret-key bytes). Both are the
  *serialized* forms that cross into/out of the keychain — the live `iroh::SecretKey` wrapper is added in
  01-03; this unit only owns the byte-level secret hygiene so 01-02's keychain store has a type to persist.
- `#[derive(Zeroize, ZeroizeOnDrop)]` on both so memory is wiped on drop. Confirm against the pinned
  `zeroize` version (derive is under the `zeroize_derive` feature).
- **No `Debug`/`Display`/`Serialize` that prints the bytes.** Implement `Debug` manually to print a
  redacted placeholder (e.g. `DbKey(<redacted>)`). Do **not** derive `Serialize`/`Deserialize` on these.
- A constructor for `DbKey` that fills from a caller-supplied RNG is deferred to 01-03 (needs `rand`); for
  this step provide `DbKey::from_bytes([u8;32])` / `as_bytes(&self) -> &[u8;32]` and the same for
  `SecretKeyBytes`, plus `Zeroizing`-based helpers for any transient buffers.

Illustrative only (confirm API against pinned versions):

```rust
// illustrative
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DbKey([u8; 32]);

impl core::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DbKey(<redacted>)")
    }
}
```

Re-export from `mod.rs`: `pub use secret::{DbKey, SecretKeyBytes};`

Tests in this unit (`#[cfg(test)]` in `secret.rs`):
- `debug_output_is_redacted`: `format!("{:?}", key)` contains neither the bytes nor any hex of them (assert
  the redaction marker is present and a known byte pattern is absent). **This is the unit-level slice of the
  secret-leak guard** mandated by the DD Testing section.
- `round_trip_bytes`: `from_bytes(x).as_bytes() == &x`.
- Compile-fail intent documented (not a test): a note that `DbKey`/`SecretKeyBytes` must never gain a
  derived `Serialize`.

---

## Parallel Unit B — `account_id` marker file (`core/src/identity/marker.rs`)

Responsibilities:
- `AccountId(Uuid)` newtype (v4), `Serialize`/`Deserialize` (it is **not** a secret — it is an opaque local
  UUID that reveals nothing about the user and is explicitly NOT the NodeId).
- Marker file read/write against a caller-supplied directory path (the actual OS app-data dir is resolved in
  01-04 via `directories`/Tauri path APIs; this unit takes a `&Path` so it is testable with a temp dir):
  - `read_marker(dir) -> Result<Option<AccountId>, MarkerError>`: `Ok(None)` when the file is absent;
    `Ok(Some(id))` when present and parseable; `Err` when present but corrupt/unparseable (tamper → explicit
    error, never silently treated as absent).
  - `write_marker(dir, AccountId) -> Result<(), MarkerError>`: **atomic** write — write to a temp file in
    the same dir then `rename` into place, so a crash never leaves a half-written marker. This is the write
    that `identity_status` keys off, and 01-03 calls it **LAST** in provisioning.
- Marker holds the UUID **only**. No secrets, no display name, no NodeId.

Re-export: `pub use marker::{AccountId, MarkerError, read_marker, write_marker};`

Tests (`#[cfg(test)]`, use `tempfile` dev-dep or `std::env::temp_dir` + unique subdir — never the real
app-data dir):
- `absent_marker_reads_none`.
- `write_then_read_round_trips` the same UUID.
- `atomic_write_leaves_no_temp_file` on success.
- `corrupt_marker_errors_not_none`: write garbage bytes, assert `read_marker` returns `Err`, not `Ok(None)`
  (prevents a tampered/corrupt marker being mistaken for a fresh install — ties to the Security "marker-file
  tampering" concern).

---

## Parallel Unit C — display-name validation (`core/src/identity/display_name.rs`)

Responsibilities:
- `validate_display_name(&str) -> Result<DisplayName, DisplayNameError>` enforcing exactly the DD rules:
  - Trim leading/trailing whitespace; **reject empty / whitespace-only** after trim.
  - **Max length bound** in UTF-8 code points (pick and document a bound, e.g. 128 code points — state the
    number explicitly).
  - **Reject control characters and newlines** (Unicode `Cc`, plus `\n`/`\r`/`\t`).
  - **Reject bidi-override / spoofing characters** (at minimum U+202A–U+202E and U+2066–U+2069; document the
    exact set rejected).
  - **Accept emoji and normal multi-byte UTF-8** verbatim.
  - Store exactly what passed validation (store the trimmed string).
- `DisplayName` newtype wrapping the validated `String` with `as_str()`. `DisplayNameError` is an enum with
  one variant per failure class (empty, too long, control char, bidi) so the UI can show a specific message.

Re-export: `pub use display_name::{DisplayName, DisplayNameError, validate_display_name};`

Tests (`#[cfg(test)]`): table-driven, mapping directly to the DD "display-name validation" test:
- rejected: `""`, `"   "`, `"\t\n"`, an over-length string (bound + 1 code points), `"a\u{202E}b"` (bidi),
  `"a\nb"` (newline), `"a\u{0007}b"` (control).
- accepted (and returned trimmed, verbatim): `"Alice"`, `"  Bob  "` → `"Bob"`, `"héllo 👋 world"`,
  a name exactly at the bound.

---

## Guardrails

- **Secret confidentiality (Unit A):** `DbKey`/`SecretKeyBytes` derive `ZeroizeOnDrop`, have redacted
  `Debug`, no `Serialize`, no `Display`. No `println!`/`tracing` call in `core` ever takes a secret value.
  They are never written to disk by this crate (persistence is the keychain's job in 01-02).
- **Partial-write safety (Unit B):** marker writes are atomic (temp + rename). This step does not yet do the
  provisioning ordering (that is 01-03), but it provides the atomic primitive 01-03 relies on and the
  rollback-friendly `read`/`write` shape.
- **No silent re-provision (Unit B):** a corrupt/unparseable marker returns `Err`, never `Ok(None)` — the
  downstream status logic (01-03) must be able to tell "tampered" from "absent."
- **Display-name validation (Unit C):** full rule set above; reject empty/whitespace/over-length/
  control/newline/bidi; store verbatim-after-trim.
- **Fail-closed:** marker read errors and validation errors are typed and propagated, never swallowed into a
  default value.

## Security (vulnerabilities addressed in this step, prevent + verify)

- **Secret leakage via `Debug`/logs/serialization.** *Prevent:* no derive of `Debug`/`Serialize`/`Display`
  that exposes bytes; `ZeroizeOnDrop`. *Verify:* `debug_output_is_redacted` test; and a repo grep gate run
  in Quality gates below (`grep -RnE 'derive\([^)]*Serialize' core/src/identity/secret.rs` must find
  nothing).
- **Marker-file tampering / corruption mistaken for fresh install.** *Prevent:* strict parse, `Err` on
  corrupt. *Verify:* `corrupt_marker_errors_not_none` test.
- **Display-name injection / homoglyph / bidi spoofing.** *Prevent:* reject control/newline/bidi, bound
  length. *Verify:* table-driven rejection tests enumerating each class.
- **`account_id` leaking identity.** *Prevent:* `AccountId` is a random v4 UUID, never derived from the
  NodeId. *Verify:* code review — assert in `mod.rs` doc that `AccountId` is unrelated to any key.

## Credentials / secrets rule (applies to every step)

NEVER hardcode credentials, keys, passphrases, or secrets in source code. All secret material is generated
or retrieved at **runtime** and stored only in the OS keychain (`keyring`, wired in 01-02). No test in this
step writes a real secret to disk or commits one: Unit A tests use throwaway in-memory byte arrays, Unit B
tests use a temp directory and only the non-secret UUID, Unit C tests use plain strings.

## Quality gates (all must pass before this step is "done"; warnings are errors)

- `cargo fmt --all --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- Secret-hygiene grep gate: `! grep -RnE 'derive\([^)]*Serialize' core/src/identity/secret.rs` returns true
  (no match).

## Definition of Done (verifiable checklist)

- [ ] `core` is a workspace member with real, compiling content (no stubs/empty modules). *(DD DoD: "`core::identity` joins the workspace with real content only".)*
- [ ] `DbKey` and `SecretKeyBytes` are `ZeroizeOnDrop`, have redacted `Debug`, and derive no `Serialize`/`Display`. *(DD DoD: "No secret ever … is logged" — leaf-level guard.)*
- [ ] `AccountId` marker reads/writes atomically; absent → `None`, corrupt → `Err`. *(DD DoD: partial-write atomicity primitive; "never a silent re-provision" — corrupt ≠ absent.)*
- [ ] `validate_display_name` rejects empty/whitespace/over-length/control/newline/bidi and accepts emoji, storing trimmed verbatim. *(DD DoD: "Display name is … validated".)*
- [ ] All unit tests above pass; `cargo fmt`/`clippy -D warnings`/`cargo test --workspace` green.
- [ ] No secret, key, or passphrase is hardcoded anywhere in the step's code or tests.
