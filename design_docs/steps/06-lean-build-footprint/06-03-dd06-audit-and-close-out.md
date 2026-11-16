# Step 06-03: DD-06 audit and close-out

**Estimate:** M, about 150–400 LOC changed (docs, the design doc's implementation notes, and any gaps found; measurements and manual checks take most of the time)

## TL;DR

Audit everything DD-06 asked for against the code. That covers every
**Definition of done**, **Security** and **Testing** item in
`design_docs/06-lean-build-footprint.md`. Implement whatever is missing.

- Run and record measurements (a)–(e).
- Run the manual hook end-to-end checks and prove that each release check fails on a bad input.
- Exercise the interrupt and stale-lock cases.
- Run the full GUI checklist on the release binary.

Then finish the docs (README, AGENTS.md, `design_docs/README.md`), mark DD-06
**Implemented**, tick its DoD, and delete this steps folder. The result is a
complete feature from end to end: a lean dev build, a self-cleaning gate, and one
command that produces the end-user binary.

## Dependencies

- **Sequential:** this is the last step. It needs **06-01 and 06-02 merged**
  into its base.
- **Parallel:** none by default. The checks mutate shared state: `target/`,
  `release/`, temporary edits to `build.rs`/`Cargo.toml`/the script, and real
  commits. Run them in one worktree, one at a time. You may hand pure doc edits
  (README/AGENTS.md wording) to a sub-agent while measurements run, but merge
  them before the final hook run.

## Branch guidance

1. `git fetch --all --prune && git branch -a && git log --all --oneline --decorate | head -40`.
   `main` may not be up to date. Find the latest DD-06 work, normally
   `step-06-02-self-cleaning-gate-and-release-command` (or its merge into `main`).
   Confirm that it contains both 06-01 and 06-02 (`git log --oneline <branch> | grep -i "06-0"`).
2. Branch `step-06-03-dd06-audit-and-close-out` from it.
3. Make sure no temporary "bad input" edit is ever committed. Before every real
   commit, `git diff --cached` must show only intended changes.

## Open questions

- The 06-02 open questions were answered (see "Resolved decisions" in 06-02).
  If any was implemented differently from those answers, stop and ask before closing.
- Ask before closing if any measurement is "far off" the expectation: (a) about
  1.9 GB, (e) about 19.5 MB on Linux x86-64. DD-06 says to investigate first.
  Investigate, record the finding, and report it.

---

## 1. Audit checklist (walk the design doc, fix what's missing)

Mark each item with the file or line that proves it.

**Definition of done (DD-06):**

- [ ] Dev profile: line tables for the workspace, and `[profile.dev.package."*"]` with `debug = false` and `opt-level = "s"` (`Cargo.toml`).
- [ ] Release profile: `strip = true`, fat LTO, `codegen-units = 1`, `opt-level = "s"`. No `panic` key.
- [ ] `default-members` leaves out `headless`, which is still in `members`.
- [ ] `ls core/tests headless/tests app/src-tauri/tests` shows exactly: core `integration/`, `logging_canary.rs`, `profile_isolation.rs`; headless `integration/`; app `integration/`. The separate binaries carry a global-state header comment.
- [ ] Test counts are identical to the 06-01 baseline (re-run the per-crate `--list` count, and add the `gate_cleanup` tests explicitly as new). The merged suite passes 10× in a row. The mDNS command is updated everywhere (`grep -rn "test discovery_lan" .` finds nothing outside git history).
- [ ] Hook: `target/gate`, `CARGO_INCREMENTAL=0`, lock, stamp wipe, unchanged sequence (`fmt`, `clippy --workspace`, `test --workspace`, `build`, then the symbol check, then the greps, then npm), the Linux symbol check, and the cleanup trap with the status preserved.
- [ ] `gate-cleanup.sh` refusals and size cap are covered by the `gate_cleanup` test.
- [ ] `pre_commit_hook` asserts every old needle plus the new ones.
- [ ] `build.rs` linker arg on Linux. `nm -D` shows 0 `sqlite3_*` for debug and release.
- [ ] `removeUnusedCommands: true`. The GUI checklist passes on the release binary (§4).
- [ ] `npm run release` meets §5: every check, cleanup, atomic copy, `.sha256`. `release/` is git-ignored.
- [ ] Each release check is proven to fail on a bad input. The interrupt and stale-lock cases behave as specified (§3).
- [ ] Guard deliberate-violation checks pass again on the final tree. The planted-secret commit is rejected **and** cleaned (§2).
- [ ] Measurements (a)–(e) are recorded (§5).
- [ ] README and AGENTS.md are complete, and `design_docs/README.md` lists DD-06 (§6).
- [ ] The full hook passes. The PR follows the template and states "wire format unchanged".

**Security (DD-06):**

- [ ] No `--no-verify` guidance anywhere: `grep -rn "no-verify" README.md AGENTS.md design_docs .githooks scripts` only finds "never use" wording.
- [ ] The cap override accepts only positive integers.
- [ ] Deletion-safety refusals in both `gate-cleanup.sh` and `build-release.mjs` (temp dir only; never `release/`, `target/`, the source tree, `node_modules`, app data or the keychain).
- [ ] Every end-user binary check from the Security list is implemented: test doubles plus headless names, builder paths, `sqlite3_*`, stripped, devtools, release mode/embedded frontend, no baked env, pinned inputs.
- [ ] Hook and release output print only sizes, paths, hashes and check names.
- [ ] `grep -rn "option_env!\|env!(\"MESSENGER" core/src app/src-tauri/src` finds nothing. `MESSENGER_*` stay runtime-only.

**Testing (DD-06):** the items in §§2–5 below.

---

## 2. Hook end-to-end (manual; record the outcome of each in the PR)

Setup: `git config core.hooksPath .githooks`. Record `du -sh target/debug` and
`stat -c '%Y' target/debug` first.

1. **Passing commit cleans up.** Commit a harmless real change (e.g. this step's
   doc edits). Afterwards:
   ```sh
   find target/gate/debug -maxdepth 2 -type f \( -name 'app' -o -name 'app-*' -o -name 'integration-*' \
     -o -name 'core-*' -o -name 'protocol-*' -o -name 'libcore-*' -o -name 'libprotocol-*' \
     -o -name 'libapp_lib-*' -o -name 'logging_canary-*' -o -name 'profile_isolation-*' \) | wc -l   # 0
   ```
   Also check `ls target/gate/debug/deps | grep -c '^libserde-'` ≥ 1 (third-party cache kept), and that `target/debug` size and mtime are unchanged.
2. **Planted secret: rejected and cleaned.** Stage a file under `app/` that
   contains the secret key type name (the AGENTS.md check). `git commit` must be
   rejected by the grep gate, and step 1's `find` must still print 0. Unstage and
   delete the file.
3. **Ctrl-C mid-build.** `touch core/src/lib.rs`, start `git commit` on a staged
   change, and press Ctrl-C during `cargo test`. Cleanup must print and run. Then
   `flock -n target/gate.lock true && echo released` prints `released`, and the
   step 1 `find` prints 0.
4. **Stamp change.** `echo stale > target/gate/.gate-stamp` (or change
   `Cargo.lock`, e.g. `cargo update -p <leaf crate> --precise <same version>` and
   revert). The next run must log the wipe and do a cold build. Record its wall time.
5. **Linker arg removal rejected.** Temporarily delete the `cargo::rustc-link-arg`
   line in `app/src-tauri/build.rs`, stage it and commit. The exported-symbol check
   must reject it with `sqlite3_*` listed, and cleanup must still run. Restore with
   `git checkout -- app/src-tauri/build.rs`.

Extra checks:

- Two concurrent `bash .githooks/pre-commit` runs: the second waits on the lock, and both pass.
- A symlinked `target/gate` makes the hook refuse. Remove the link afterwards.
- `P2PM_GATE_MAX_MIB=1` (or the agreed name) makes the run end with a full wipe.

## 3. Release command checks (record the exit code, `ls release/`, and temp dir state for each)

Before every run: `ls release/ > "$SCRATCH/before.txt"` (a session scratch dir,
not the repo) and record `du -sh target`.

- **Happy path, clean clone:**
  - `git clone <repo> <scratch>/clone && cd <scratch>/clone && npm run release`.
  - It must produce exactly `release/p2p-messenger-<ver>-<os>-<arch>` plus `.sha256`, and print the path, size and hash.
  - `sha256sum -c` passes.
  - `ls "${TMPDIR:-/tmp}" | grep -c p2pm-release-` is `0`.
  - The clone's `target/` does not exist, or is unchanged.
- **Proving each check fires.** Use temporary local edits only, and revert each with `git checkout -- <file>`. Every run must exit non-zero, `release/` must equal `before.txt`, and the temp dir must be gone.
  1. **Test-support build:** add `features = ["test-support"]` to `messenger_core` in `app/src-tauri/Cargo.toml` `[dependencies]`. The "No test code" check must fail.
     - Caveat: with fat LTO, test-double code that the app never reaches can be removed as dead code, so the names may not survive in the binary even with the feature on. If the check passes on this input, the check is not broken; the input is too weak. Make the code reachable in the same temporary edit (e.g. in `app/src-tauri/src/lib.rs`, print `std::any::type_name::<messenger_core::contacts::testing::MemoryContactStore>()` once at startup) and re-run. Record both outcomes. Never loosen the check.
  2. **No path remapping:** comment out the `CARGO_ENCODED_RUSTFLAGS` remap in `scripts/build-release.mjs`. The "No builder paths" check must fail.
  3. **No linker arg:** remove the `build.rs` link arg. The `sqlite3_*` check must fail.
  4. **Devtools:** `tauri = { version = "2", features = ["devtools"] }` in the root `[workspace.dependencies]`. The devtools check must fail, and the evidence must show it was the devtools check, not Cargo's `--locked` error (06-02 runs the manifest checks in preflight, before the build, for this reason). (Revert `Cargo.lock` too if it changed.)
  5. **Stripping:** run with `CARGO_PROFILE_RELEASE_STRIP=false`. The env must be scrubbed, so the binary is still stripped. If not, the strip check must fail.
- **Interrupt:** press Ctrl-C during the Cargo build. The temp dir must be deleted, and `release/` must have no `.partial-*` and no new file. Repeat with Ctrl-C during the copy if you can time it (or simulate it by throwing after `copyFile` in a temporary edit).
- **Stale lockfile:** add an unused dependency line to `app/src-tauri/Cargo.toml` without updating `Cargo.lock`. The build must fail with Cargo's `--locked` error. Nothing is published and the temp dir is gone. Revert.
- **Portability review:** every `nm`, `readelf`, `pkg-config`, `otool` or `xcode-select` call is inside a `process.platform === '...'` branch. Windows and macOS runs are recorded only if a machine is available. Otherwise say "not run" in the PR.

## 4. GUI and runtime checks

- **Dev:** `npm start` launches the app. Run README manual GUI checklist steps 1–2.
- **Linker fix:** run the release binary and `target/debug/app` on a **fresh profile** and an **existing profile** (an existing `messenger.db`). The app starts, opens the encrypted DB and shows the main window with WebKit working (checklist steps 1–3). `nm -D --defined-only` on both shows 0 `sqlite3_*`.
- **Release binary:** run the **full** README manual GUI checklist against `release/p2p-messenger-...`. Use `--profile`/`MESSENGER_PROFILE` for a second instance, exchange messages in both directions, and confirm that chat and storage feel responsive (the smoke check for `opt-level = "s"`). Confirm that every app command works with `removeUnusedCommands`: wizard, My Info copy buttons, connect by ticket, contacts, chat, history clear, presence and relay settings.
- **Backtrace:** a scratch, uncommitted panic test with `RUST_BACKTRACE=1` shows `file:line` for workspace frames.
- **Dev profile sections:** after `cargo clean && cargo build`:
  - `readelf -S -W target/debug/app` has `.debug_line`;
  - `readelf --debug-dump=info target/debug/app | grep -c DW_TAG_variable` is `0`;
  - a workspace `.rlib` has line tables only;
  - `readelf -S -W target/debug/deps/libserde-*.rlib | grep -c '\.debug_'` is `0`;
  - no `messenger-headless` and no test binaries were built.

## 5. Measurements (record in the PR and in the design doc)

Run on the same machine. Note the CPU core count and the OS.

| Id | Procedure | Expected |
|---|---|---|
| (a) | `cargo clean && npm start` (close the window once it shows), then `du -sh target target/*` | about 1.9 GB |
| (b) | `bash .githooks/pre-commit` once, then `du -sh target target/*` | `target/gate`: third-party cache only |
| (c) | Five consecutive hook runs, then `du -sh target target/*` | No growth after run 1 |
| (d) | `time bash .githooks/pre-commit` (warm) | Record it |
| (e) | `time npm run release`, plus the binary size | about 19.5 MB, about 3 min |

Add an **"Implementation notes"** section at the end of
`design_docs/06-lean-build-footprint.md` (before "Decisions recorded" or after it;
keep the doc's existing order). Record in it:

- the measured values;
- the test counts before and after;
- the deviations agreed during implementation: the clippy `--workspace` correction, the `/target` remap, the cap variable name and unit, lock portability, file-name tokens, the same-name file policy, the `.dockerignore` entry, the release script deleting only its own `.partial-*` files in `release/`, the hook also refusing a symlinked `target/` (stricter than DD-06), and the position of the symbol check (right after `cargo build`);
- anything not run (e.g. Windows and macOS).

## 6. Docs

- **README.md**: check that all of these are present and accurate.
  - "Build & test": `headless` is not built by default, the debug-info trade-off, and the `CARGO_PROFILE_DEV_DEBUG=true` override (never commit it).
  - "Pre-commit hook": gate dir, cleanup, lock, stamp, cap variable, symbol check, and the manual verification steps (§2 checks 1–5).
  - "Reclaiming disk space": `cargo clean`, `cargo sweep`, and the folder meanings.
  - "Building the end-user binary" and "Running the release binary".
  - The mDNS command and the test paths.
- **AGENTS.md**:
  - Commands: clippy `--workspace`, `npm run release`, and the cleanup commands.
  - Testing paths: `core/tests/integration/`, `core/tests/logging_canary.rs`, `core/tests/profile_isolation.rs`, `headless/tests/integration/`, `app/src-tauri/tests/integration/` (`pre_commit_hook`, `gate_cleanup`). State the rule that tests installing process-global state get their own binary.
  - Layout notes: `release/` (git-ignored end-user binaries, never deleted by tooling), `target/gate/` (hook-owned; workspace outputs deleted after every run), and `scripts/build-release.mjs`.
  - Update the guard paths in Notes and Security rules.
- **`design_docs/README.md`**: DD-06 is listed in the table (it already is in the working tree; check that the committed version has it).
- **The design doc**: set `**Status:** Implemented` and tick every DoD checkbox once it is actually verified. Do not tick anything that was not run. Instead, leave it unticked and explain why in the implementation notes and the PR.

## 7. Close-out

1. Run all quality gates and one final real commit through the hook. Then re-check
   §2 check 1 (no workspace artifacts left).
2. After everything above passes, delete the step files:
   `git rm design_docs/steps/06-lean-build-footprint/*.md` if they are tracked on
   this branch, or plain `rm` if they were never committed (check with
   `git ls-files design_docs/steps/06-lean-build-footprint/`). Leave no empty
   folder; git does not track empty dirs, so also `rmdir` the folder in your
   worktree. Do not touch other step folders.
3. Commit, e.g. "Close out DD-06: mark Implemented, tick DoD, remove step files".
4. Open the PR following `.github/PULL_REQUEST_TEMPLATE.md`:
   - **Summary**
   - **Changes**, by area:
     - root `Cargo.toml` profiles and `default-members`;
     - `app` (`build.rs`, `tauri.conf.json`, tests);
     - `core`/`headless` tests;
     - `.githooks`;
     - `scripts` and `package.json`;
     - docs.
   - **Affected crates:** `core` (tests only), `app`, tooling/docs.
   - **Protocol impact:** tick "No change to the wire format". Write explicitly: **"wire format unchanged, no `PROTOCOL_VERSION` bump."**
   - **Testing:** the gate commands, test counts before and after, measurements (a)–(e), the outcomes of hook checks 1–5, the release bad-input outcomes, interrupt and stale-lock results, and the GUI checklist on the release binary (platforms).
   - **Checklist:** no empty files or placeholders; no plaintext, keys or secrets in logs; README updated.

## Tests to create or update

- No new test files are planned. The DD-06 tests (`gate_cleanup`, the extended
  `pre_commit_hook`, the merged `integration` binaries) come from 06-01/06-02.
- Any gap found in the audit gets a test in the same commit as its fix: an
  automated one where possible (`gate_cleanup` / `pre_commit_hook` cases, a new
  needle), otherwise a documented manual check in README and the PR.
- Bad-input runs (§2 check 5, §3) are temporary local edits and are never
  committed as tests.

## Quality gates (all must pass)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
npm run release
```

Commit only through the hook. **Never use `--no-verify`.** `npm start` must
launch the app at the end.

## Guardrails (AGENTS.md; do not weaken)

- Never reference `SecretKeyBytes` / `DbKey` under `app/`, not even in comments. The planted file in §2 check 2 is temporary and never committed.
- Never reference `FileKeychain`, `messenger-headless` or `messenger_headless` under `app/` (the same gate). Temporary plants are reverted.
- Clipboard only in `app/src/views/MyInfo.svelte` (and tests).
- Logs never contain plaintext, private keys or secrets. Hook and release output show only paths, sizes, hashes and check names.
- `core` keeps `#![forbid(unsafe_code)]`.
- **No wire-format change, and no `PROTOCOL_VERSION` bump.**
- Only files with real content: no empty files, folders or placeholders. The steps folder is removed completely at close-out.
- Don't weaken any gate to make an audit item pass. Fix the cause instead. Every bad-input edit is reverted, and `git diff` is checked before each commit.
- Never bypass the hook (`--no-verify`).
- **Credentials and secrets:** never write credentials or secrets in code, config, docs or the PR. All credentials are supplied at runtime (OS keychain, or env at runtime). The release script must never set or bake `MESSENGER_PROFILE`, `MESSENGER_RELAY_URL` or any other environment value into the binary. When recording measurements, do not paste env dumps, home paths of other users, or keychain contents into the PR.

## Security (audit focus)

| Risk | Check in this step |
|---|---|
| Deletion safety (gate cleanup, release temp dir) | Re-run the `gate_cleanup` test. Read `cleanup()` in the script against the refusal list. Check the temp dir after success, failure and interrupt. |
| Symlink / path traversal / TOCTOU | Symlinked `target/gate` refused. The concurrent-run lock works. The trap is installed after `flock`. |
| Command injection | `grep -n "exec(\|execSync\|shell: true" scripts/` is empty. Every spawn uses an arg array. |
| Builder paths in the binary | The no-remap bad input fails. `strings release/p2p-messenger-* \| grep -c "$HOME"` is `0`. |
| Exported `sqlite3_*` interposition | The linker-removal bad input fails in both the hook and the release. Both binaries show `0`. |
| Test doubles or headless in the release binary | The test-support bad input fails. `strings` finds none of the names. |
| Devtools | The devtools bad input fails. |
| Stale lockfile / unpinned inputs | The stale-lock case fails because of `--locked`. `npm ci` is used. |
| Gate weakened by the changes | The `pre_commit_hook` needles are a superset of the pre-DD-06 list. `headless` is linted (clippy `--workspace`). The guards still fail on planted violations. |
| Secrets in output or logs | Read the full hook and release output. It contains no env values, no file contents, and no keys. |
| `panic = "abort"` creeping in | `grep -n panic Cargo.toml` is empty |

## Definition of done (this step)

- [ ] Every DD-06 DoD, Security and Testing item was audited, with evidence noted. Missing pieces were implemented.
- [ ] Hook manual checks 1–5 and the extra checks pass, with results recorded.
- [ ] Release happy path on a clean clone. Each check proven to fail on a bad input (test-support, no remap, no linker arg; plus devtools and strip). Interrupt and stale-lock cases behave as specified.
- [ ] GUI checklist passes on the release binary (two instances, message exchange). The linker fix is verified on fresh and existing profiles.
- [ ] Measurements (a)–(e) are recorded in the PR and in the design doc's implementation notes.
- [ ] README, AGENTS.md (Commands, Testing paths, `release/` and `target/gate/` in Layout notes) and `design_docs/README.md` are complete.
- [ ] Design doc `Status: Implemented`, with DoD ticked only for verified items.
- [ ] All DD-06 step files are deleted (`design_docs/steps/06-lean-build-footprint/`), with no empty folder left.
- [ ] All quality gates pass. The final commit went through the hook. `npm start` and the release binary both launch.
- [ ] The PR follows the template and states **"wire format unchanged"**.
