# Step 06-01: Lean profiles, Linux linker fix and test consolidation

**Estimate:** M, about 250–400 LOC changed (not counting lines that only move with `git mv`)

## TL;DR

This step covers the build-config half of DD-06 (§1, §2, §3, §5 profile and Tauri
config, §6 linker fix). It does not touch the gate's cleanup or the release
command.

- **1A: Build configuration.** Root `Cargo.toml` changes:
  - Dev profile: workspace crates get line tables only. Third-party crates get no debug info and `opt-level = "s"`.
  - Release profile: `strip`, fat LTO, `codegen-units = 1`, `opt-level = "s"`. Panic stays unwind.
  - `default-members` leaves out `headless`.

  `app/src-tauri/build.rs` hides static-archive symbols on Linux, and
  `tauri.conf.json` gets `build.removeUnusedCommands = true`. The hook's clippy
  line gains `--workspace`, so `headless` stays linted after it leaves
  `default-members`.
- **1B: Test consolidation.**
  - `core` ends up with exactly 3 integration binaries: `integration`, `logging_canary` and `profile_isolation`.
  - `headless` ends up with 1 (`integration`), and so does `app`.
  - Test counts stay the same. Guard coverage is proven unchanged. Every reference to the old test paths is updated, including the mDNS opt-in command.

At the end of the step, `npm start` still works, `cargo build` no longer builds
`messenger-headless`, and `nm -D` on the dev app shows 0 `sqlite3_*` exports.

## Dependencies

- **Sequential:** this is the first DD-06 step and has no prerequisite step.
  Step 06-02 must start from this step's merged result. The hook's
  `CARGO_TARGET_DIR` and cleanup depend on `default-members`, and the
  `gate_cleanup` test goes into the `app` integration binary created here.
- **Parallel inside this step:** sub-tasks **1A** and **1B** are independent.
  You can use one git worktree and one sub-agent per sub-task.
- **Shared files and merge order:** merge **1A first, then 1B**.
  - `app/src-tauri/tests/pre_commit_hook.rs`: 1A edits one needle (the clippy line). 1B moves the file with `git mv` and **must not change its content**, so git's rename detection merges cleanly. If a conflict appears anyway, keep 1B's location (`tests/integration/pre_commit_hook.rs`) with 1A's content.
  - `README.md` and `AGENTS.md`: 1A edits "Build & test" in README and "Commands" in AGENTS.md. 1B edits README's mDNS command, the Docker/parity paths and AGENTS.md's Notes/Security/Testing paths. The sections are disjoint. On a conflict, keep both sides' lines.
  - `Cargo.toml`, `build.rs`, `tauri.conf.json` and `.githooks/pre-commit` belong to 1A only. `core/tests/**`, `headless/tests/**` and `app/src-tauri/tests/**` belong to 1B only (apart from the one needle above).

## Branch guidance

1. `git fetch --all --prune && git branch -a && git log --all --oneline --decorate | head -30`.
   The design-doc branch `06-lean-build-footprint` already exists (created from
   `origin/main` at #27, which contains the DD-05 close-out). It holds
   `design_docs/06-lean-build-footprint.md` and this steps folder. Branch from it,
   and target step PRs at it.
2. Check that the base contains `design_docs/06-lean-build-footprint.md` and
   this steps folder (`git ls-tree -r --name-only <base> design_docs/`). If not,
   stop and ask the user.
3. Create `step-06-01-lean-profiles-and-test-consolidation`. Sub-task branches:
   `step-06-01-lean-profiles-and-test-consolidation-1a` and `-1b`, each in its own
   worktree (`git worktree add ../p2pm-06-01-1a -b ...-1a`). Merge them back into the
   step branch in the order above with `--no-ff`.

## Before you change anything: record the baseline

On the base branch, record (and paste into the PR):

```sh
for p in protocol core messenger-headless app; do
  printf '%s ' "$p"; cargo test -p "$p" --all-targets -- --list 2>/dev/null | grep -cE ': (test|bench)$'
done
cargo test --workspace --doc -- --list 2>/dev/null | grep -cE ': test$'   # doc tests, if any
ls core/tests headless/tests app/src-tauri/tests
nm -D --defined-only target/debug/app | grep -c ' sqlite3_'   # expected today: 51 (Linux)
```

The static count of `#[test]` in `core/tests` is about 133 in the files that
will merge, plus 2 in `logging_canary.rs` and 4 in `profile_isolation.rs`.
`headless/tests` has 14 and `app/src-tauri/tests` has 5. Use the `--list`
numbers, not these.

---

## Sub-task 1A: Build configuration

### Files

- `Cargo.toml` (root)
- `app/src-tauri/build.rs`
- `app/src-tauri/tauri.conf.json`
- `.githooks/pre-commit` (clippy line only)
- `app/src-tauri/tests/pre_commit_hook.rs` (clippy needle only)
- `README.md` ("Build & test"), `AGENTS.md` ("Commands")

### Root `Cargo.toml`

```toml
[workspace]
resolver = "2"
members = ["protocol", "core", "headless", "app/src-tauri"]
# `headless` is test-only: `--workspace` and `-p messenger-headless` still see it,
# but plain `cargo build` / `cargo check` and `npm start` never compile it (DD-06 §2).
default-members = ["protocol", "core", "app/src-tauri"]

# Dev (and `test`, which inherits it): line tables for our crates, so panic
# backtraces keep file:line. Third-party crates: no debug info, size-optimized.
# Full DWARF for a local debugging session: CARGO_PROFILE_DEV_DEBUG=true (never commit).
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = false
opt-level = "s"

# End-user binary (also the Docker headless image). panic stays "unwind": abort
# would kill the app on a contained tokio-task panic and skip zeroizing Drops.
[profile.release]
strip = true
lto = "fat"
codegen-units = 1
opt-level = "s"
```

- `[profile.dev.package."*"]` only matches non-workspace packages (Cargo
  reference, "Profile overrides"). Build scripts and proc-macros already default
  to no debug info. Do not add `build-override`.
- Do **not** set `panic` anywhere. Unwind is the default and must stay.

### `app/src-tauri/build.rs`

```rust
fn main() {
    // Linux only: never export symbols from static archives. Without this the
    // executable exports the bundled SQLCipher's sqlite3_* functions and the
    // system WebKitGTK/libsoup (which load libsqlite3) bind to our copy
    // (DD-06 §6). `cargo::rustc-link-arg` covers every linked target of this
    // package: bins and tests. CARGO_CFG_TARGET_OS is the *target* OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg=-Wl,--exclude-libs,ALL");
    }
    tauri_build::build();
}
```

Use `rustc-link-arg`, not `-bins`, so the `app` lib's unit-test binary and the
integration binary get it too. macOS and Windows get no change.

### `app/src-tauri/tauri.conf.json`

Add `"removeUnusedCommands": true` to `build`. This needs tauri ≥ 2.4,
tauri-build ≥ 2.1 and tauri-cli ≥ 2.4. We have tauri 2.12.5, tauri-build 2.7.4
and `@tauri-apps/cli` 2.12.4. The option only takes effect through the Tauri CLI
(`tauri dev` / `tauri build`). Plain `cargo build` ignores it.

**Risk to check:** the option removes commands that the ACL
(`capabilities/default.json`: `core:default`,
`clipboard-manager:allow-write-text`) never allows. Our own
`invoke_handler` commands have no app manifest and are allowed by default, so
they must keep working. Prove it under `npm start`: walk through steps 1–3 of
the README manual GUI checklist, plus one chat send and the My Info copy
buttons. (The release-binary GUI run happens in 06-03.)

### Hook: keep `headless` linted

With `default-members`, `cargo clippy` without a package flag lints only the
default members, so `headless` would silently stop being linted. That would
weaken the gate, even though DD-06 §2 says clippy keeps covering it. Make this
correction, and call it out in the PR:

- `.githooks/pre-commit`: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `app/src-tauri/tests/pre_commit_hook.rs`: change the clippy needle to the same
  string. The old needle is not a literal substring of the new line, so replace
  it rather than keeping both. The new needle still contains the old flags
  `--all-targets --all-features -- -D warnings` verbatim, and the command lints
  a strict superset of what it linted before (it now names `--workspace`
  explicitly), so the gate gets stronger.
- Why this is needed (verified Cargo behavior): the root `Cargo.toml` is a
  virtual manifest. In a virtual workspace, a command with no `-p` /
  `--workspace` selects `default-members` when they are set, and all members
  only when they are not. Today `cargo clippy` therefore lints all four crates.
  After this step it would lint only three.
- `AGENTS.md` Commands, and the README hook paragraph, use the `--workspace`
  form.
- `cargo fmt --all` and `cargo test --workspace` already cover all members.
  `cargo build` intentionally stays plain: it is the "app builds without test
  features" check, and it now skips `headless`.

### Docs (1A)

- README "Build & test":
  - `cargo build` / `npm start` build only `protocol`, `core` and `app`. Build the headless node with `cargo build -p messenger-headless` (Docker already does).
  - The debug-info trade-off: our crates keep `file:line` in backtraces. Debuggers show locations but no local variables. Dependency frames have names but no line numbers. Dependencies are optimized (`"s"`), so the first cold build is about 40 s slower and later edit-rebuild is unchanged.
  - The local full-DWARF override: `CARGO_PROFILE_DEV_DEBUG=true cargo build` (or `npm start`). Never commit it.
- AGENTS.md "Commands": the clippy line with `--workspace`, and a one-line note that `cargo build` skips the test-only `headless`.
- Note in the PR that `docker/smoke.sh` / the Docker image now builds with fat LTO
  (slower image build, same behavior). Running it is optional, since it is not in
  the hook.

### 1A verification

```sh
cargo clean && cargo build
ls target/debug/messenger-headless 2>/dev/null && echo "FAIL: headless built"
ls target/debug/deps | grep -E '^(messenger_headless|integration)' && echo "FAIL"
nm -D --defined-only target/debug/app | grep -c ' sqlite3_'        # must print 0
readelf -S -W target/debug/app | grep -c '\.debug_line'           # >= 1 (line tables)
readelf --debug-dump=info target/debug/deps/libcore-*.rlib 2>/dev/null | grep -c DW_TAG_variable  # 0 (workspace crate: line tables, no full DWARF)
readelf --debug-dump=info target/debug/app 2>/dev/null | grep -c DW_TAG_variable   # far below the baseline (see note)
readelf -S -W target/debug/deps/libserde-*.rlib | grep -c '\.debug_'  # 0 (third-party: no debug)
du -sh target                                                     # about 1.9-2.3 GB after npm start
```

Note on the `DW_TAG_variable` count for the linked `app`: the precompiled std from
rustup carries its own debug info, which our profile does not control. Record the
baseline count (before this step) and the new count. Workspace and third-party
objects must contribute none. If the binary count is not 0, show with the rlib
checks above that the remainder comes from std, not from our crates or deps.

Prove backtraces with a scratch test that you do not commit: a `#[test]` that
panics inside a `core` function, run with `RUST_BACKTRACE=1`. Workspace frames
must show `core/src/...:<line>`.

---

## Sub-task 1B: One integration-test binary per crate

### Target layout (Cargo auto-discovers `tests/<dir>/main.rs` as one target)

```
core/tests/integration/main.rs          # mod list + header comment
core/tests/integration/{contacts_two_nodes,discovery_channels,discovery_lan,
  discovery_names,discovery_parity,discovery_relay,discovery_two_nodes,
  identity_integration,log_redaction_guard,messaging_frames,messaging_store,
  messaging_two_nodes,sql_namespacing_guard,storage_legacy_import,
  storage_namespacing,storage_open}.rs   # git mv, names unchanged
core/tests/logging_canary.rs            # stays: global tracing subscriber
core/tests/profile_isolation.rs         # stays: global keyring_core default store
headless/tests/integration/{main,control,parity}.rs
app/src-tauri/tests/integration/{main,frontend_limits,no_test_backends,pre_commit_hook}.rs
```

Use `git mv` for every move so history and rename detection survive. Do not add
`[[test]]` entries to any manifest. Auto-discovery is enough.

`core/tests/integration/main.rs` (shape):

```rust
//! All `core` integration tests in one binary (DD-06 §3): one link of the
//! dependency graph instead of sixteen. Modules keep their original file
//! names, so filters stay stable (e.g. `--test integration discovery_lan`).
//! Tests that install process-global state keep their own binary:
//! `logging_canary.rs` (global tracing subscriber) and `profile_isolation.rs`
//! (global keyring_core default store). Put any new test that sets a global
//! subscriber, default store, env var or working directory in its own binary.

mod contacts_two_nodes;
mod discovery_channels;
// ... one `mod` per moved file, alphabetical
```

Notes:

- The package `core` shadows std `core` through the extern prelude, which is
  crate-wide. `use core::...` inside the modules keeps working as it does today.
- `headless/tests/control.rs` and `parity.rs` start with `#![cfg(unix)]`. That is
  still valid as an inner attribute of a module file, so leave it. Both use
  `env!("CARGO_BIN_EXE_messenger-headless")`, which works in any integration
  target of the package. Both define `const BIN`, which is fine because each is
  in its own module.
- In `logging_canary.rs` and `profile_isolation.rs`, add a first doc line that
  says explicitly: "Separate test binary on purpose (DD-06 §3): installs
  process-global <subscriber | keyring_core default store>; do not merge into
  `integration`."
- Moving the files must not change what the source-scanning guards scan. Both
  derive their roots from `CARGO_MANIFEST_DIR` and scan only `src` dirs:
  `log_redaction_guard` scans `core/src`, `app/src-tauri/src` and `headless/src`,
  and `sql_namespacing_guard` scans `core/src`. Their `/tests/` exemptions apply
  to paths inside those roots. Verify this rather than assume it (see the checks
  below).
- All tests in one binary now run in parallel threads with each other. If a test
  turns flaky (timeouts in two-node tests, shared temp names, ports), fix it or
  move it into its own binary with a global-state comment. **Never** `#[ignore]`
  it, and never add `--test-threads=1` to the hook.

### References to update (all found by `grep -rn "tests/[a-z_]*\.rs\|--test "`)

- `core/tests/integration/discovery_lan.rs`: the `#[ignore = "..."]` text becomes
  `run: cargo test -p core --test integration discovery_lan -- --ignored`.
- `README.md`: the mDNS opt-in command (`cargo test -p core --test integration discovery_lan -- --ignored`), `core/tests/discovery_relay.rs`, `core/tests/discovery_parity.rs`, `headless/tests/parity.rs` and `discovery_lan.rs` (Level-3 "Scope" bullets).
- `AGENTS.md`: the `sql_namespacing_guard.rs` path (Notes), the `log_redaction_guard.rs` path (Security rules), and the Testing paragraph. Use the new paths: `core/tests/integration/discovery_*.rs`, `messaging_*.rs`, `storage_*.rs`, the guards, `core/tests/logging_canary.rs`, `core/tests/profile_isolation.rs`, `headless/tests/integration/`, and `app/src-tauri/tests/integration/pre_commit_hook.rs`.
- `docker/compose.yaml` (comment), `core/src/logging/mod.rs:11`,
  `core/src/identity/settings_sql/tests.rs:32`, `core/src/storage/legacy/tests.rs:2`,
  the doc comments of `discovery_parity.rs` and `headless/.../parity.rs` (each
  names the other's path).

### 1B verification

```sh
ls core/tests core/tests/integration headless/tests app/src-tauri/tests
# exactly: core -> integration/, logging_canary.rs, profile_isolation.rs; headless -> integration/; app -> integration/
# same per-crate counts as the baseline:
for p in protocol core messenger-headless app; do printf '%s ' "$p"; cargo test -p "$p" --all-targets -- --list 2>/dev/null | grep -cE ': (test|bench)$'; done
for i in $(seq 10); do cargo test -p core --test integration || { echo "FAILED run $i"; break; }; done
cargo test -p core --test integration discovery_lan -- --ignored   # needs a multicast-capable interface
cargo test -p core --test logging_canary && cargo test -p core --test profile_isolation
```

**Guard coverage (deliberate violations; revert each one, never commit):**

1. Add a log call with a plaintext-looking field (e.g. `tracing::info!(body = %x, "...")`)
   in a production file under `core/src`. `cargo test -p core --test integration log_redaction_guard` must fail.
2. Add an un-namespaced `CREATE TABLE` SQL const (no `account_id`) in `core/src`.
   `cargo test -p core --test integration sql_namespacing_guard` must fail.
3. Put a `FileKeychain` reference (a comment is enough) in
   `app/src-tauri/src/lib.rs`. That file matters: `no_test_backends` only reads
   `app/src-tauri/src/lib.rs` and the manifests, so a plant elsewhere under `app/`
   would only trip the grep gate.
   `cargo test -p app --test integration no_test_backends` and the hook's grep gate
   (`bash .githooks/pre-commit`, or stage it and `git commit`) must both fail.
4. Optional extra proof: temporarily print the guards' scanned file lists before
   and after the move and `diff` them. They must be identical.

Revert everything and confirm `git status` / `git diff` are clean apart from the
intended changes.

---

## Tests created or updated in this step

- Moved (content unchanged except reference strings): every `core/tests/*.rs`
  except the two global-state binaries, both `headless/tests/*.rs` and all three
  `app/src-tauri/tests/*.rs`.
- New: `core/tests/integration/main.rs`, `headless/tests/integration/main.rs` and
  `app/src-tauri/tests/integration/main.rs` (mod lists with a header comment).
- Updated: the `pre_commit_hook` clippy needle (`--workspace`), the
  `logging_canary` / `profile_isolation` header comments, and the mDNS `#[ignore]`
  text.

## Quality gates (all must pass)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

Commit through the pre-commit hook (`git config core.hooksPath .githooks`).
**Never use `--no-verify`.** If the hook fails, fix the cause. Then run
`npm start`, confirm the app launches, and go through README GUI checklist
steps 1–3.

## Guardrails (AGENTS.md; do not weaken)

- Never reference `SecretKeyBytes` / `DbKey` under `app/`, not even in comments, test strings or types.
- Never reference `FileKeychain`, `messenger-headless` or `messenger_headless` under `app/`, including the moved `app/src-tauri/tests/integration/*`. Keep the existing `concat()` trick in `no_test_backends.rs` and `pre_commit_hook.rs`.
- Clipboard access (`writeText` / `clipboard`) stays only in `app/src/views/MyInfo.svelte` (and tests).
- Logs never contain message plaintext, private keys or secrets. Logging still goes through `core::logging`. Do not extend the redaction allowlist.
- `core` keeps `#![forbid(unsafe_code)]`.
- **No wire-format change, and no `PROTOCOL_VERSION` bump.**
- Only files with real content: no empty files, folders, stubs or placeholders. Every new `main.rs` carries a real mod list and comment.
- Don't weaken any gate. Every existing hook check and needle stays. `headless` stays linted (clippy `--workspace`), built and tested by the hook.
- Never bypass the hook (`--no-verify`). Never `#[ignore]` a test to make it pass.
- **Credentials and secrets:** never write credentials or secrets in code, config, docs or test fixtures. All credentials are supplied at runtime (OS keychain, or env at runtime only). No build file in this step may bake `MESSENGER_PROFILE`, `MESSENGER_RELAY_URL` or any other environment value into a binary. Do not add `option_env!`/`env!` for them, and do not set them in `build.rs`.
- `CARGO_PROFILE_DEV_DEBUG=true` is a local override only. Never commit it into `Cargo.toml`, `.cargo/config.toml` or scripts.

## Security

| Risk | Prevention | How to check |
|---|---|---|
| Bundled SQLCipher symbols interposed by system WebKitGTK/libsoup (two SQLite implementations mixing handles) | `-Wl,--exclude-libs,ALL` for every linked target of `app` on Linux | `nm -D --defined-only target/debug/app \| grep -c ' sqlite3_'` is `0` (was 51). The app still opens a **fresh** and an **existing** encrypted DB (GUI checklist steps 1–3 on both). |
| `headless` silently dropping out of lint coverage after `default-members` | Clippy runs with `--workspace`. The needle is updated. | Plant a clippy warning in `headless/src` temporarily. The hook / `cargo clippy --workspace ...` must fail. Revert it. |
| Guards losing coverage after the move | Roots come from `CARGO_MANIFEST_DIR`, and only `src` is scanned | Deliberate violations 1–3 above fail as expected |
| Test-only code reaching the app build | `headless` is not a default member. `test-support` is only in dev-deps. | `cargo build` builds no `messenger_headless` and no test binaries. `no_test_backends` still passes. |
| `removeUnusedCommands` removing a command the app needs (availability) or leaving extra IPC (attack surface) | ACL-driven removal. App commands stay allowed. | GUI walk-through under `npm start`. Release check in 06-03. |
| Reduced debug info hiding secrets? (no new risk) | Line tables contain only paths and symbol names | n/a: no secrets in debug info. Logging rules are unchanged. |
| Release `panic = "abort"` skipping zeroizing `Drop`s | Keep the default unwind | `grep -n panic Cargo.toml` shows nothing |

## Open questions (ask the user before implementing)

- None blocking. The clippy `--workspace` correction above follows from DD-06's
  own requirement that headless stays linted. Flag it in the PR for review.

## Definition of done (this step)

- [ ] Root `Cargo.toml` dev profile: `debug = "line-tables-only"`, and `[profile.dev.package."*"]` with `debug = false` and `opt-level = "s"`. *(DD DoD 1)*
- [ ] Release profile: `strip = true`, `lto = "fat"`, `codegen-units = 1`, `opt-level = "s"`, panic unwind (unset). *(DD DoD 2)*
- [ ] `default-members = ["protocol", "core", "app/src-tauri"]`. `headless` is still in `members`. `cargo build` builds no `messenger-headless`. *(DD DoD 3)*
- [ ] `core` has exactly the `integration`, `logging_canary` and `profile_isolation` test binaries. `headless` and `app` have one `integration` binary each. The separate binaries carry a global-state header comment. *(DD DoD 4)*
- [ ] Per-crate test counts are identical before and after (recorded in the PR). The merged `core` suite passes 10× in a row. The mDNS opt-in command is updated in the code, README and AGENTS.md, and the new command runs the ignored test. *(DD DoD 5)*
- [ ] `build.rs` emits `cargo::rustc-link-arg=-Wl,--exclude-libs,ALL` on Linux only. `nm -D` on `target/debug/app` shows 0 `sqlite3_*`. *(DD DoD 9, debug half; the release half is in 06-02/06-03)*
- [ ] `tauri.conf.json` has `build.removeUnusedCommands = true`. App commands work under `npm start`. *(DD DoD 10, partial; the release-binary checklist is in 06-03)*
- [ ] Hook clippy uses `--workspace`, and the `pre_commit_hook` needle matches.
- [ ] Guard deliberate-violation checks (redaction, SQL namespacing, test backend under `app/`) fail as expected, and are reverted. *(DD DoD 13, guard half)*
- [ ] README "Build & test" (headless not built by default, the debug-info trade-off, the `CARGO_PROFILE_DEV_DEBUG=true` override) and AGENTS.md (Commands, Notes/Security/Testing paths) are updated. *(DD DoD 15/16, partial)*
- [ ] All quality gates pass. The commit went through the hook. `npm start` launches the app.
- [ ] PR follows `.github/PULL_REQUEST_TEMPLATE.md`. It names the affected crates (`core` tests, `headless` tests, `app` build/tests, tooling) and states **"wire format unchanged, no `PROTOCOL_VERSION` bump"**. It includes the baseline and after test counts and the `nm` before/after numbers.
