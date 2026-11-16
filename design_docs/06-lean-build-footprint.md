# DD-06 — Lean Build Footprint, Self-Cleaning Test Gate & End-User Binary

**Feature slice:** cross-cutting build/test/release infrastructure (no runtime feature)
**Status:** Draft
**Affected:** root `Cargo.toml` (profiles, `default-members`), `app/src-tauri/build.rs`,
`app/src-tauri/tauri.conf.json`, `core/tests/`, `headless/tests/`,
`app/src-tauri/tests/`, `.githooks/`, root `package.json`, `scripts/`, `.gitignore`,
`README.md`, `AGENTS.md`
**Wire format:** unchanged — no `PROTOCOL_VERSION` bump.
**Related:** [DD-05 Encrypted Storage & Logging](05-encrypted-storage-and-logging.md)
(its guard tests must keep running on every commit; its SQLCipher DB is
involved in the linker fix in §6)

---

## TL;DR

`target/` reached **21 GB**. That isn't because of how many dependencies we
have. It's full debug info copied into **33 linked binaries** (about 350–570 MB
each, 11.4 GB total) plus 7.6 GB of `.rlib`/`.rmeta`. Almost all of those
binaries are test binaries built by the pre-commit hook. This doc fixes the
developer side and adds a clean path for producing the binary end users run:

1. **Lean dev profile.** Workspace crates keep line tables only (panic
   backtraces still show `file:line`). Third-party crates get **no debug info
   and `opt-level = "s"`**. Measured result: a clean `npm start` build goes from
   **5.1 GB → 1.9 GB** of `target/`, and the dev binary from **571 MB → 72 MB**.
2. **Fewer test binaries.** Each crate's integration tests are merged into one
   binary, except two that install process-global state. That takes us from 23
   integration binaries to 5.
3. **Test-only code stays out of the app build.** `headless` leaves the
   workspace `default-members`, so plain `cargo build` and `npm start` never
   compile it or any test.
4. **The pre-commit gate cleans up after itself.** The hook still runs **every**
   gate. It builds in its own `target/gate/` dir and deletes all
   workspace-crate outputs on exit, pass or fail, keeping only compiled
   third-party libraries. A toolchain/lockfile stamp and a size cap trigger a
   full wipe.
5. **One command produces the end-user binary.** `npm run release` builds a
   size-optimized release binary (LTO, `opt-level = "s"`, stripped, unused
   Tauri commands removed). It builds in a throwaway dir and leaves **only**
   the executable plus its SHA-256 in `release/`. Measured: **34.7 MB → 19.5 MB**
   binary, with no embedded home-folder paths.
6. **Linux linker fix found while measuring.** The executable currently
   **exports 51 `sqlite3_*` symbols** from our bundled SQLCipher. The system
   WebKitGTK and libsoup load `libsqlite3`, so their SQLite calls can bind to
   our copy. All Linux builds of the app will hide static-library symbols.

No dependency is removed, and no gate is weakened.

## Context and scope

### Measured facts

`target/` on the `step-05-03` branch after normal development:

| Location | Size | What it is |
|---|---|---|
| `target/debug/deps`, 33 executables | 11.4 GB | 23 integration-test binaries + lib/bin unit-test binaries + 3 stale `app-*` bins; each statically links iroh, rusqlite (SQLCipher + vendored OpenSSL), tokio, and Tauri for `app` |
| `target/debug/deps`, `.rlib`/`.rmeta` | 7.6 GB | dependency libraries with full DWARF; 13–14 copies of e.g. `syn`, `toml`, `zvariant` |
| `build/`, `incremental/`, `app` | ~2 GB | build-script output, incremental caches, the dev app binary |

Clean builds of the app package (what `npm start` compiles), 24-core Linux
machine. Each row is a fresh target dir:

| Dev profile variant | `target/` | Dev binary | Cold build |
|---|---|---|---|
| Today (full debug info everywhere) | 5.1 GB | 571 MB | 1 m 22 s |
| Line tables (own) + no debug info (deps) | 2.3 GB | 118 MB | 1 m 05 s |
| No debug info at all | 2.2 GB | 104 MB | 59 s |
| **Line tables (own) + deps: no debug info, `opt-level = "s"` (chosen)** | **1.9 GB** | **72 MB** | 1 m 42 s |
| Same, deps `opt-level = 1` | 2.0 GB | 75 MB | 1 m 42 s |
| No debug info at all + deps `opt-level = "s"` | 1.8 GB | 57 MB | 1 m 49 s |
| Chosen + iroh without its unused `metrics` feature | −0.5 MB | — | — |

End-user binary built with `tauri build --no-bundle`. Each build used a fresh
target dir, about 1.9 GB of intermediates that the release command deletes:

| Release variant | Binary | Build | Home paths embedded |
|---|---|---|---|
| Today (`strip = true` only) | 34.7 MB | 1 m 59 s | 1,215 |
| + fat LTO, `codegen-units = 1`, `opt-level = 3` | 27.6 MB | 4 m 11 s | 1,076 |
| + fat LTO, `codegen-units = 1`, `opt-level = "s"` | 20.6 MB | 2 m 42 s | 1,096 |
| + `removeUnusedCommands` | 19.6 MB | 2 m 33 s | 1,064 |
| **+ static-symbol hiding + path remapping (chosen)** | **19.5 MB** | 2 m 54 s | **0** |

All release variants contained **no** test-double names (`FileKeychain`,
`MemoryContactStore`, `MemoryMessageStore`, `ChatPeer`). The Linux binary
dynamically needs WebKitGTK 4.1, GTK 3, libsoup 3, GLib/GIO, cairo, gdk-pixbuf
and D-Bus. OpenSSL and SQLCipher are statically linked.

### Why `target/` grows

- Cargo builds **one binary per file** in `tests/`. We have 18 in `core`, 2 in
  `headless` and 3 in `app`, and each one re-links the whole dependency graph.
- The default `dev`/`test` profile writes **full debug info** for every crate.
- Every distinct feature/profile combination gets its own copy of a dependency:
  `cargo build`, `cargo test` (adds `iroh/test-utils`, `core/test-support`),
  `clippy --all-features`, `tauri dev`, and toolchain upgrades. Cargo never
  deletes the old copies.
- The pre-commit hook writes `clippy --all-targets --all-features`,
  `cargo test --workspace` and `cargo build` output into the same `target/` as
  `npm start`.
- Plain `cargo build` / `npm start` already skip integration tests today, but
  plain `cargo build` at the root also compiles the test-only `headless` crate.

### Dependency audit

- Every runtime dependency of `core` is used in non-test code: `qrcode`,
  `serde_json`, `tracing-subscriber`, `rand`, `tokio-stream`, `iroh-tickets`,
  `keyring-core`, `uuid`.
- Test-only crates are already optional or in `[dev-dependencies]`: `tempfile`,
  `iroh/test-utils`, `core[test-support]`.
- About 40 crates appear in two versions because iroh uses the newer RustCrypto
  generation (`digest` 0.11, `cipher` 0.5, …) and Tauri the older one. This
  cannot be fixed by us.
- The remaining ~1.6 GB of `deps/` is the code and metadata of 523 crates. The
  biggest are GTK, gio, Tauri, iroh, OpenSSL and netlink. Removing dependencies
  is **not** a lever here.

## Goals and non-goals

### Goals
- Running the app (`npm start`, `cargo build`) compiles only what the app
  needs: no test targets and no `headless`. A clean dev build fits in about
  **2 GB**.
- The pre-commit hook keeps running **every** current gate, including all
  integration tests. After it finishes, success or failure, no test binaries or
  workspace-crate artifacts it created remain on disk.
- Commit-time cost stays reasonable: third-party dependencies are not rebuilt
  from scratch on every commit.
- One documented command produces a **single, small, stripped end-user
  executable** and leaves nothing else behind: no build cache, and no
  bundles unless asked.
- The end-user binary carries no test code, no builder-specific paths, and no
  exported symbols that let system libraries bind to our bundled SQLCipher.

### Non-goals
- **Removing or replacing dependencies** (iroh, Tauri, SQLCipher). They are the
  product.
- **Moving integration tests behind a Cargo feature** or out of the hook. You
  chose to keep the hook running them.
- **Auto-cleaning manual `cargo test` runs** in `target/`. Use the documented
  cleanup commands.
- **Debugger variable inspection.** With line tables only, gdb/lldb show where
  you are but not local variables. You accepted this trade-off.
- **Installers and packages** (`.deb`, AppImage, `.msi`, `.dmg`). `bundle.active`
  stays `false`, and the release command produces a bare executable.
  Packaging is a later doc.
- **Code signing, notarization, auto-update, and bit-for-bit reproducible
  builds.** The SHA-256 is printed so users can compare it with a published
  value, but builds are not yet guaranteed reproducible.
- **Statically linking the system webview.** WebKitGTK (Linux) and WebView2
  (Windows) stay OS-provided runtime requirements.
- **Adding tools** such as `cargo-nextest` or `sccache`. `cargo-sweep` is
  recommended in the docs only and is never required.

## The actual design

### Overview

```
npm start / cargo build ──▶ target/debug/        app + core + protocol only
                                                  own: line tables · deps: no debuginfo, opt "s"

git commit ──▶ .githooks/pre-commit
                 │  lock target/gate/ · stamp check (toolchain + Cargo.lock)
                 │  CARGO_TARGET_DIR=target/gate/  CARGO_INCREMENTAL=0
                 ├─ fmt · clippy --all-targets --all-features · test --workspace · build
                 ├─ secret / backend / clipboard greps · exported-symbol check (Linux)
                 ├─ npm check/test/build
                 └─ on EXIT (pass, fail, Ctrl-C): clean workspace-crate outputs,
                    keep third-party libs; wipe all if over size cap

npm run release ──▶ scripts/build-release.mjs
                 │  npm ci · fresh temp CARGO_TARGET_DIR · --locked · path remapping
                 ├─ tauri build --no-bundle   (release: LTO, cu=1, opt "s", strip)
                 ├─ verify binary (no test doubles, no exported sqlite3_*, no home paths)
                 ├─ copy to release/p2p-messenger-<ver>-<os>-<arch>[.exe] + .sha256
                 └─ finally: delete temp target dir (success or failure)
```

### 1. Dev profile (root `Cargo.toml`)

- `[profile.dev]` sets debug info to **line tables only** for our own crates.
  The `test` profile inherits from `dev`.
- A package override for **all non-workspace packages**
  (`[profile.dev.package."*"]`) turns debug info **off** and sets
  **`opt-level = "s"`**. Build scripts and proc-macros already default to no
  debug info.
- Trade-offs, measured:
  - The first cold build is about 40 s slower, because dependencies are now
    optimized. This cost is paid only when dependencies rebuild: a toolchain
    or `Cargo.lock` change, or a gate stamp wipe.
  - Edit-and-rebuild of our own crates is unchanged, because they stay at
    `opt-level = 0`.
  - Dependency code (iroh, ring, SQLCipher, tokio) runs noticeably faster in
    dev and in the Level-1 tests.
  - Dependency backtrace frames show function names but no line numbers.
- `"s"` was chosen over `1`: it's slightly smaller (1.9 vs 2.0 GB, 72 vs 75 MB)
  at the same build time.
- A developer who temporarily needs full DWARF overrides it locally through
  Cargo's environment config (`CARGO_PROFILE_DEV_DEBUG=true`). The README
  documents this. It must never be committed.

### 2. Keep test-only crates out of the app build

- The workspace gets `default-members` = `protocol`, `core`, `app/src-tauri`.
  `headless` stays a **member**, so `--workspace` and `-p messenger-headless`
  still see it, but plain `cargo build` / `cargo check` at the root no longer
  compile it.
- The hook keeps `cargo test --workspace` and `cargo clippy --all-targets`, so
  `headless` is still built, linted and tested on every commit. The hook's
  `cargo build` step stays as the "app builds without test features" check.
- The Docker image already builds with `-p messenger-headless`, so nothing
  changes there.

### 3. One integration-test binary per crate

Cargo treats `tests/<name>/main.rs` as one test target, and `.rs` files beside
it are modules, not separate targets.

| Crate | Integration binaries | Contents |
|---|---|---|
| `core` | `integration` | all current `core/tests/*.rs` as modules with their original file names, **except** the two below |
| `core` | `logging_canary` | stays its own binary: installs a **process-global** `tracing` subscriber and asserts on what its log files contain |
| `core` | `profile_isolation` | stays its own binary: installs a **process-global** `keyring_core` default store (mock) |
| `headless` | `integration` | `control` + `parity` (no global state; both spawn the bin) |
| `app` | `integration` | `frontend_limits`, `no_test_backends`, `pre_commit_hook`, plus the new `gate_cleanup` test (§4) |

Rules:

- **Process-global state decides separation.** A test that sets a global
  subscriber, a default keyring store, env vars, the working directory, or any
  other per-process singleton keeps its own binary. That binary carries a
  header comment saying why.
- Module names keep the old file names. For example, the opt-in real-mDNS test
  becomes `cargo test -p core --test integration discovery_lan -- --ignored`.
  README and AGENTS.md are updated to match.
- The source-scanning guards (`log_redaction_guard`, `sql_namespacing_guard`)
  skip paths containing `/tests/`, and their roots come from
  `CARGO_MANIFEST_DIR`. Moving them one level deeper must not change the
  scanned set. This is verified, not assumed (see Testing).
- Tests within one binary run in parallel threads. Any test that implicitly
  relied on running alone is fixed or separated under the rule above, never
  `#[ignore]`d.

### 4. Self-cleaning pre-commit gate

The hook's checks and their order stay as they are, plus one new check (§6).
Only the environment around them changes:

- **Dedicated target dir.** The hook exports `CARGO_TARGET_DIR` as
  `<repo>/target/gate`. It never touches `target/debug`. A plain `cargo clean`
  removes the gate dir too.
- **No incremental compilation in the gate** (`CARGO_INCREMENTAL=0`). Gate
  builds are one-shot and our own crates are deleted afterwards.
- **Exclusive lock** (`flock`) on the gate dir for the whole run, so two
  concurrent commits in one clone can't delete each other's in-flight build.
- **Stamp-based full wipe (before building).** A stamp file records
  `rustc -vV` and a hash of `Cargo.lock`. If either changed, the gate dir is
  wiped first, which drops stale copies from upgrades.
- **Cleanup on exit (always).** An `EXIT`/`INT`/`TERM` trap runs
  `.githooks/gate-cleanup.sh`:
  - It runs Cargo's own per-package clean, scoped to the gate dir, for
    `protocol`, `core`, `messenger-headless` and `app`. Third-party compiled
    libraries stay. **No hand-written glob deletion inside `deps/`.**
  - **Size-cap backstop:** if the gate dir is still over a cap (default
    8 GiB, overridable by a documented environment variable), the whole gate
    dir is wiped.
  - The **hook's exit status is the gate result**. A failing gate stays
    failing. A cleanup error after a passing gate prints a loud warning and the
    commit proceeds; the next run retries the cleanup.
- **Commit-time trade-off (accepted):** a cold build happens only on the first
  run or after a toolchain or lockfile change. Otherwise the hook rebuilds our
  four crates and links.

### 5. End-user binary: `npm run release`

**Release profile (root `Cargo.toml` `[profile.release]`).** These settings
apply to every release build, including the Docker headless image:

- `strip = true` (kept), fat `lto = true`, `codegen-units = 1`,
  `opt-level = "s"`.
- `panic` stays **unwind**. Tauri's size guide suggests `panic = "abort"`, but
  it's rejected. Today a panic inside a spawned tokio task is contained by the
  runtime, and abort would kill the whole app instead. Abort also skips
  `Drop`, so zeroizing key material on the panic path would no longer happen.
  The size win is not worth either. A future doc can revisit this with explicit
  panic containment.
- `opt-level = "s"` instead of `3` trades CPU speed for 7 MB (27.6 → 20.6 MB).
  Messaging workloads are I/O-bound. The Testing section includes a smoke check
  that chat and storage still feel responsive.

**Tauri config.** `build.removeUnusedCommands = true` in `tauri.conf.json`
drops Tauri core/plugin commands not granted by
`capabilities/default.json` (`core:default`,
`clipboard-manager:allow-write-text`). That saves another 1 MB, and fewer
reachable IPC commands also means less attack surface. Requires
tauri ≥ 2.4 / tauri-build ≥ 2.1 / tauri-cli ≥ 2.4; we have 2.12.

**The command.** Root `package.json` gains `release`, which runs
`scripts/build-release.mjs`. It's a Node script so it works on Linux, macOS
and Windows, and Node is already required for the frontend. The script runs
these steps in order and stops at the first failure:

1. **Preflight:** check the Rust toolchain against the MSRV, and Node/npm.
   Print the platform's build prerequisites if missing; on Linux, the
   WebKitGTK 4.1/GTK 3 dev packages.
2. **Pinned inputs:** run `npm ci` in `app/` (exact lockfile). The Cargo build
   uses `--locked`, so a stale `Cargo.lock` fails instead of silently
   resolving new versions.
3. **Throwaway target dir:** create a fresh temporary dir in the OS temp
   location and point `CARGO_TARGET_DIR` at it. It never reuses `target/` or
   `target/gate/`, so dev and gate caches can't leak into the release and the
   release can't pollute them.
4. **Path remapping:** release `RUSTFLAGS` remap the Cargo home, the rustup
   home and the repo root to neutral prefixes (`/cargo`, `/rustup`, `/src`).
   Panic-location strings then no longer reveal the builder's user name or
   folder layout. Measured: 1,064 → 0 occurrences of the home path.
5. **Build:** `tauri build --no-bundle --ci`. This runs the frontend
   `beforeBuildCommand` (`npm run build`), embeds the frontend (no `devUrl`),
   and builds only the `app` package with default features, so no
   `test-support` and no `headless`.
6. **Verify the binary** (§Security has the full list). If any check fails,
   nothing is copied to `release/`.
7. **Publish locally:** copy the executable to
   `<repo>/release/p2p-messenger-<version>-<os>-<arch>` (`.exe` on Windows). The
   copy goes to a temp name first and is then renamed, so a half-written file
   never appears. Write a `<name>.sha256` next to it, and print the path, size
   and hash.
8. **Always clean up:** in a `finally` block that runs on success, failure or
   Ctrl-C, delete the throwaway target dir. Afterwards the only new artifacts
   are the binary and its `.sha256` in `release/`. The small `app/dist`
   (~124 KB) is left in place, as `npm run build` already does today.

`release/` is added to `.gitignore`. The script never deletes anything in
`release/`: older versions stay until the user removes them.

**End-user runtime requirements** (README "Running the release binary"):

- **Linux:** WebKitGTK 4.1, GTK 3, libsoup 3, D-Bus (usually present on
  desktop distros), and an OS keychain (Secret Service).
- **Windows:** WebView2 runtime (preinstalled on Windows 10/11).
- **macOS:** nothing extra. A bare executable runs, but macOS users normally
  expect a `.app`; that's deferred to the packaging doc.

### 6. Hide bundled SQLCipher symbols on Linux

**Finding.** On Linux, both `target/debug/app` and the release binary export
**51 `sqlite3_*` symbols** in their dynamic symbol table: `sqlite3_open_v2`,
`sqlite3_prepare_v2`, `sqlite3_step`, `sqlite3_close`, and others. These are
the functions our statically bundled SQLCipher defines that WebKitGTK and
libsoup import. Both of those libraries depend on the system `libsqlite3`.

The ELF dynamic linker resolves symbols from the executable first. So WebKit's
and libsoup's calls to those 51 functions bind to **our SQLCipher build**,
while any other SQLite function they call binds to the system library. That
mixes two SQLite implementations on the same connection handles and can
corrupt memory or crash. It also runs WebKit's own databases (cookies, HSTS,
storage) through our encryption-capable library unintentionally. A crash
hasn't been reproduced. The risk follows from the symbol table and is treated
as real.

**Fix.** `app/src-tauri/build.rs` adds a Linux-only linker argument to every
linked target of the `app` package (bins and tests): `--exclude-libs,ALL`.
Symbols coming from static archives are then never exported. Measured with the
release build: exported symbols 51 → 0, binary otherwise unchanged. macOS
(two-level namespace) and Windows (explicit imports) don't have this
interposition and get no change.

**Regression gate.** On Linux, the pre-commit hook checks the `app` binary it
just built in `target/gate` with `nm -D --defined-only`. It fails the commit if
any `sqlite3_*` symbol is exported. The release script runs the same check.

### 7. Reclaiming space from manual builds

README (Build & test) and AGENTS.md (Commands) document:

- `cargo clean` removes everything, including `target/gate`.
- `cargo sweep --time 7` (optional tool) removes artifacts unused for a week.
  `cargo sweep --installed` removes artifacts from toolchains no longer
  installed.
- Running tests by hand still writes to `target/debug` and isn't auto-cleaned.
- What each folder is: `target/debug/` is the dev app and manual runs,
  `target/gate/` is the hook's dependency cache, and `release/` holds
  end-user binaries.

## Alternatives considered

| Option | Why not |
|---|---|
| Cut or replace dependencies | Every dependency is used at runtime; the cost is debug info × binary count, not dependency count. |
| Feature-gate integration tests and skip them in the hook | You chose to keep all integration tests on every commit. |
| Wipe the whole gate dir after every commit | Every commit would cold-build iroh, Tauri and SQLCipher/OpenSSL. You chose to keep the third-party cache. |
| Clean the shared `target/debug` after the hook | Hook output can't be told apart from the developer's own build. A separate dir makes "hook-owned" unambiguous. |
| No debug info at all in dev | Saves only about 0.1 GB more and loses `file:line` in our panic backtraces. |
| Turn off incremental compilation in dev | Saves about 150 MB but slows every edit-and-rebuild. |
| Trim iroh features (`metrics` is unused) | Measured saving was 0.5 MB, not worth a non-default feature set. |
| System OpenSSL instead of vendored (Linux) | Saves about 45 MB in `build/`. The app would depend on the distro's OpenSSL version, and Windows/macOS still need vendored. |
| `opt-level = 3` for release | 27.6 MB vs 20.6 MB and a slower build, with no user-visible speed benefit for an I/O-bound messenger. |
| `panic = "abort"` in release | Kills the app on any contained task panic and skips zeroizing `Drop`s (§5). |
| Ship installers (`tauri build` with bundles) | Out of scope. You asked for the bare binary; packaging is a later doc. |
| `cargo-nextest` (process per test) | Adds a required tool to a gate that must run on every clone. |

## Security

This doc must not weaken any enforced control from AGENTS.md and adds the
following:

**Gate integrity**
- Every existing gate still runs on every commit. The `pre_commit_hook`
  self-test keeps asserting all existing needles (`set -euo pipefail`, fmt,
  clippy `--all-targets --all-features -D warnings`, `cargo test --workspace`,
  `cargo build`, the secret-type grep, the test-backend grep, the clipboard
  grep, `npm run check|test|build`). It is extended to assert the gate target
  dir, the `EXIT` trap calling `gate-cleanup.sh`, the lock, and the
  exported-symbol check.
- The guards keep their coverage. `log_redaction_guard` and
  `sql_namespacing_guard` must scan exactly the same production files before
  and after the test move (proven with a deliberate violation; see Testing).
- `default-members` only stops `headless` from building on `cargo build`.
  The grep gate forbidding `FileKeychain` / `messenger-headless` /
  `messenger_headless` under `app/` stays unchanged.
- No `--no-verify` guidance anywhere. Documented overrides (the gate cap) only
  accept a positive integer and otherwise fall back to the default.

**Deletion safety (gate cleanup and release script)**
- `gate-cleanup.sh` resolves the gate dir to an absolute path and refuses
  (non-zero exit, nothing deleted) if it is empty, `/`, `$HOME`, the repo root,
  outside `<repo>/target/`, or a symlink.
- The release script deletes **only** the temp dir it created in this run (path
  from the OS temp-dir API, recorded at creation), with the same refusals. It
  never deletes anything in `release/`, `target/`, the source tree,
  `node_modules`, the OS app-data dir (`messenger.db`, logs) or the keychain.

**End-user binary checks** (run by the release script; a failure means no
binary is published):
- **No test code.** The binary contains none of the test-double names
  (`FileKeychain`, `MemoryContactStore`, `FailingContactStore`,
  `MemoryMessageStore`, `FailingMessageStore`, `ChatPeer`,
  `RecordingHistoryEraser`) and none of the `messenger_headless` /
  `messenger-headless` names.
- **No builder paths.** No occurrence of the builder's home directory or the
  absolute repo path.
- **No exported SQLite symbols (Linux).** `nm -D --defined-only` lists no
  `sqlite3_*`.
- **Stripped.** No symbol table or debug sections, checked by section listing.
- **No devtools.** `app`'s `tauri` dependency doesn't enable the `devtools`
  feature (checked in the manifest).
- **Release mode.** The frontend is embedded, not served from `devUrl`.
- **No baked-in environment.** The script never sets `MESSENGER_PROFILE` or
  `MESSENGER_RELAY_URL`. These stay runtime-only and the binary has no
  defaults derived from the builder's environment.
- **Pinned inputs.** `npm ci` and `cargo --locked` only. The printed SHA-256
  lets a user compare against a published hash, but the build isn't claimed to
  be reproducible.

**Unchanged guarantees.** Private keys and the DB key stay in the OS keychain.
Nothing about encryption, the wire format or logging changes. Line-table debug
info and remapped panic paths contain only source paths and symbol names, never
secrets or message content. Hook and release output print sizes, paths and
hashes, never environment values or file contents.

## Testing

**Dev profile**
- After `cargo clean && cargo build`, a section listing shows the app binary
  and a workspace `.rlib` with line tables but no full DWARF, and a
  third-party `.rlib` with no debug sections.
- A deliberate panic in a scratch test prints `file:line` for workspace frames
  under `RUST_BACKTRACE=1`.
- `cargo build` at the root doesn't compile `messenger-headless` and produces
  no test binaries.
- `npm start` launches the app. Run steps 1–2 of the README manual GUI
  checklist.

**Test consolidation**
- `cargo test --workspace` runs the **same test count** as before (per-crate
  counts recorded in the PR).
- The merged `core` suite passes 10 runs in a row with default parallelism.
- The new mDNS opt-in command runs the ignored test.
- `logging_canary` and `profile_isolation` still run as their own binaries.
- **Guard coverage:** temporarily add a plaintext-looking log field and an
  un-namespaced SQL table in production files. The merged binary must fail
  both guards. Plant a `FileKeychain` reference under `app/`; `no_test_backends`
  and the grep gate must fail. Revert all of them.

**`gate-cleanup.sh`** (new `gate_cleanup` test in `app`'s integration binary)
- Run against a temp gate dir with fake workspace artifacts (`core-<hash>`,
  `libcore-<hash>.rlib`, `app-<hash>`) and a fake third-party
  `libserde-<hash>.rlib`. Workspace artifacts are gone; the third-party one
  remains.
- It refuses, exits non-zero and deletes nothing for: empty path, `/`, `$HOME`,
  the repo root, a path outside `target/`, and a symlinked gate dir.
- The size-cap wipe triggers above a tiny test cap. An invalid cap value falls
  back to the default.

**Hook end-to-end (manual, documented)**
1. Make a passing commit. Afterwards `target/gate` holds no workspace
   executables, and `target/debug` is unchanged.
2. Plant a secret type name under `app/` (the existing AGENTS.md check). The
   commit is rejected **and** cleanup still ran.
3. Press Ctrl-C mid-build. Cleanup runs and the lock is released.
4. Change `Cargo.lock` or the stamp. The next run wipes and rebuilds.
5. Temporarily remove the linker argument from `build.rs`. The
   exported-symbol check rejects the commit. Then restore it.

**Linker fix**
- `nm -D --defined-only` on `target/debug/app` and the release binary lists no
  `sqlite3_*` (today: 51).
- The app starts, creates/opens its SQLCipher DB, and shows the main window
  with WebKit working (manual GUI checklist steps 1–3). Run on a fresh profile
  and on an existing one, to prove existing encrypted DBs still open.

**Release command**
- `npm run release` on a clean clone produces exactly
  `release/p2p-messenger-<ver>-<os>-<arch>` plus `.sha256`, prints path/size/hash,
  and leaves no temp target dir behind. Afterwards, check the OS temp dir and
  that `target/` is unchanged.
- The produced binary passes every check in Security. To prove each check
  fires, run the script once per check against a deliberately bad input:
  - a build with the `test-support` feature (test-double names present);
  - a build without path remapping (home paths present);
  - a build without the linker argument (exported `sqlite3_*`).

  Each must fail with nothing published.
- Interrupt the script during the Cargo build. The temp dir is deleted and
  `release/` has no partial file.
- A stale `Cargo.lock` (manifest changed, lock not updated) makes the release
  fail because of `--locked`.
- The release binary runs the full README manual GUI checklist. Exchange
  messages with a second instance under another profile, as a
  responsiveness smoke check for `opt-level = "s"`. The size is about 19.5 MB
  on Linux x86-64; record the actual value.
- Run on Linux. Windows and macOS runs are recorded when a machine is
  available; the script itself must not contain Linux-only commands outside
  `process.platform === 'linux'` branches.

**Measure and record** in the PR and in this doc's implementation notes:
`du -sh target target/*` after (a) `cargo clean && npm start`, (b) one hook
run, (c) five consecutive hook runs; (d) wall-clock time of a warm hook run;
(e) release binary size and build time. Expected, from the measurements above:
(a) about 1.9 GB, (e) about 19.5 MB. Investigate before closing if any of them
is far off.

## Definition of done

- [ ] Root `Cargo.toml` dev profile: line tables only for workspace crates;
      no debug info and `opt-level = "s"` for all non-workspace packages.
- [ ] Root `Cargo.toml` release profile: `strip = true`, fat LTO,
      `codegen-units = 1`, `opt-level = "s"`, panic unwind (unchanged).
- [ ] `default-members` excludes `headless`, which stays a workspace member.
- [ ] `core` has exactly three integration binaries (`integration`,
      `logging_canary`, `profile_isolation`), and `headless` and `app` one
      each. Separate binaries carry a global-state comment.
- [ ] Test counts are identical before and after (recorded). The merged suite
      passes 10× in a row. The mDNS opt-in command is updated.
- [ ] Hook: `target/gate`, incremental off, lock, stamp wipe, unchanged gate
      sequence, the Linux exported-symbol check, and `gate-cleanup.sh` always
      run with the exit status preserved.
- [ ] `gate-cleanup.sh` path-safety refusals and size cap are implemented and
      covered by the `gate_cleanup` test.
- [ ] `pre_commit_hook` self-test asserts every old needle plus the
      gate-dir/trap/lock/symbol-check needles.
- [ ] `app/src-tauri/build.rs` hides static-archive symbols on Linux, and
      `nm -D` shows 0 `sqlite3_*` exports for debug and release.
- [ ] `tauri.conf.json` has `build.removeUnusedCommands = true`. The GUI
      checklist passes on the release binary.
- [ ] `npm run release` + `scripts/build-release.mjs` work as in §5, including
      every binary check, the guaranteed temp-dir cleanup, the atomic copy,
      and the `.sha256`. `release/` is in `.gitignore`.
- [ ] Each release check is proven to fail on a bad input. The interrupt and
      stale-lock cases behave as specified.
- [ ] Guard deliberate-violation checks pass. The planted-secret commit is
      rejected **and** cleaned up.
- [ ] Measurements (a)–(e) are recorded.
- [ ] README updated:
  - "Build & test", "Pre-commit hook", and cleanup commands;
  - the debug-info trade-off and the full-DWARF override;
  - a new "Building the end-user binary" section (`npm run release`, prereqs
    per OS, output location, how to verify the SHA-256);
  - "Running the release binary" runtime requirements.
- [ ] AGENTS.md updated (Commands, Testing paths, `release/` and
      `target/gate/` in Layout notes). `design_docs/README.md` lists DD-06.
- [ ] The full hook passes. The PR follows the template and states "wire format
      unchanged".

## Decisions recorded

- **Hook keeps running all integration tests** and cleans up after itself.
  *(Your decision.)*
- **Gate cleanup keeps third-party compiled libs** and deletes our own crates'
  outputs, with a size-cap full wipe. *(Your decision.)*
- **Dev: line tables for our crates. Third-party crates get no debug info and
  `opt-level = "s"`.** *(Your decision.)*
- **End users get one command producing only the binary.** *(Your request.)*
  The bare executable instead of installers, `opt-level = "s"` for release,
  and keeping panic unwind are this doc's recommendations, open to review.
- **The Linux SQLCipher symbol-export fix** is included here because it affects
  the binary this doc ships. It can be split into its own change if you prefer.
