# Step 06-02: Self-cleaning pre-commit gate and `npm run release`

**Estimate:** L, about 900–1,200 LOC changed (hook ~80, `gate-cleanup.sh` ~120, `gate_cleanup` test ~220, hook self-test ~40, `build-release.mjs` ~450, docs ~150)

## TL;DR

This step adds the two pieces of DD-06 that run on the developer's machine.

- **2A: Self-cleaning gate (§4, §6 regression gate, §7 docs).**
  - `.githooks/pre-commit` builds in `target/gate` with `CARGO_INCREMENTAL=0`.
  - It holds an exclusive `flock` and wipes the gate dir when the `rustc -vV` / `Cargo.lock` stamp changes.
  - It always runs `.githooks/gate-cleanup.sh` from an `EXIT`/`INT`/`TERM` trap and keeps the gate's exit status.
  - On Linux it adds an `nm -D --defined-only` check that rejects exported `sqlite3_*`.
  - `gate-cleanup.sh` cleans only our 4 packages, with `cargo clean -p` scoped to the gate dir. It keeps third-party libraries, applies a size cap (default 8 GiB) and refuses unsafe paths.
  - Tests: a new `gate_cleanup` test, and an extended `pre_commit_hook` self-test.
- **2B: `npm run release` (§5, Security binary checks).**
  - `scripts/build-release.mjs` does a preflight, then runs `npm ci` and `tauri build --no-bundle --ci -- --locked` in a throwaway temp `CARGO_TARGET_DIR`, with path-remapping rustflags.
  - It verifies the binary: no test doubles, no builder paths, no exported `sqlite3_*`, stripped, no devtools, frontend embedded.
  - It publishes the binary atomically to `release/p2p-messenger-<ver>-<os>-<arch>[.exe]` with a `.sha256` next to it.
  - It always deletes the temp dir.
  - `release/` goes into `.gitignore`.

At the end of the step, every commit leaves no workspace-crate artifacts in
`target/gate`, `target/debug` is never touched by the hook, and `npm run release`
produces the end-user binary.

## Dependencies

- **Sequential:** this step needs **06-01 merged** into its base. It relies on
  `default-members`, the release profile, the `build.rs` linker arg,
  `removeUnusedCommands`, and the `app/src-tauri/tests/integration/` binary.
  06-03 (audit) needs this step.
- **Parallel inside this step:** **2A** and **2B** are independent. Use one
  worktree and one sub-agent each.
- **Shared files and merge order:** merge **2A first, then 2B**.
  - `README.md`: 2A owns "Pre-commit hook (the test gate)" and a new "Reclaiming disk space" subsection under "Build & test". 2B owns the new sections "Building the end-user binary" and "Running the release binary". On a conflict, keep both. Only one place lists the three folders (`target/debug/`, `target/gate/`, `release/`): 2A's "Reclaiming disk space".
  - `AGENTS.md`: 2A edits the hook sentence in Commands and the Layout notes for `target/gate/`. 2B adds `npm run release` to Commands and a Layout note for `release/` and `scripts/`. Keep both.
  - `.gitignore` and `package.json` are touched only by 2B. `.githooks/*` and `app/src-tauri/tests/integration/*` are touched only by 2A.

## Branch guidance

1. `git fetch --all --prune && git branch -a && git log --all --oneline --decorate | head -30`.
   Find the most recent DD-06 branch (normally `step-06-01-lean-profiles-and-test-consolidation`,
   or `main` if its PR is merged). `main` may be behind, so branch from whichever
   contains 06-01's final merge commit.
2. Create `step-06-02-self-cleaning-gate-and-release-command`. Sub-task branches:
   `...-2a` and `...-2b` in separate worktrees. Merge them in the order above with `--no-ff`.
3. If the DD-06 doc or this steps folder is missing from the base, see the note in
   06-01. Do not commit it silently.

## Resolved decisions (answered by the user, 2026-11-16)

These answers are final. Implement exactly these; the "Recommendation" text
below each original question is now the decision unless this list says otherwise.

1. Size cap: `P2PM_GATE_MAX_MIB`, unit MiB, default `8192`, accept only `^[1-9][0-9]{0,8}$`, otherwise fall back to the default.
2. Lock: use `flock` when it is available (any OS). Where it is missing, fall back to an atomic `mkdir "$repo/target/gate.lockdir"` lock that records the PID and is removed by the trap. Do **not** make the hook Linux-only.
3. File-name tokens: linux→`linux`, darwin→`macos`, win32→`windows`; x64→`x86_64`, arm64→`aarch64`; fail on anything else. `<version>` from `tauri.conf.json`.
4. Same-name binary already in `release/`: refuse with a clear message and publish nothing.
5. Symlinks: refuse a symlinked `target/gate` (and the lock and stamp files), but **allow a symlinked `target/`** (users may point it at another disk). This matches DD-06 and overrides the stricter `target` refusal in the snippet and Security table below. In `gate-cleanup.sh`, the `<repo>/target/` prefix check must compare against the resolved (`pwd -P`) `target` dir so a symlinked `target/` still passes.
6. Windows "no builder paths" check: implement it, but it stays unverified until a Windows machine is available; record that in the PR.

## Original open questions (now resolved above)

1. **Size-cap environment variable: name and unit.** DD-06 asks for "a documented environment variable" accepting only a positive integer, with a default of 8 GiB. The `gate_cleanup` test needs a *tiny* cap, so the unit cannot be GiB.
   - *Recommendation:* `P2PM_GATE_MAX_MIB`, unit MiB, default `8192`. Accept only `^[1-9][0-9]{0,8}$`. Anything else falls back to the default.
2. **Lock portability.** `flock(1)` is util-linux. It is missing on stock macOS and in Git Bash on Windows, but DD-06 targets Linux, macOS and Windows desktops.
   - *Recommendation:* require `flock` on Linux. Where it is absent, fall back to an atomic `mkdir target/gate.lockdir` lock that records the PID and is removed by the trap.
   - Alternatively, decide that the hook is Linux-only for now. (The existing `pre_commit_hook` test already uses `std::os::unix`.)
3. **Release file-name tokens.** DD-06 says `p2p-messenger-<version>-<os>-<arch>` but does not fix the tokens.
   - *Recommendation:* map `process.platform` linux→`linux`, darwin→`macos`, win32→`windows`, and `process.arch` x64→`x86_64`, arm64→`aarch64`. Fail on anything else.
   - Take `<version>` from `app/src-tauri/tauri.conf.json` `version`, which today equals the workspace version 0.1.0.
4. **Same-name binary already in `release/`.** DD-06 says "the script never deletes anything in `release/`", but an atomic `rename` over an existing same-version file would replace it.
   - *Recommendation:* refuse with a clear message (move or remove the old file yourself), and publish nothing.

---

## Sub-task 2A: Self-cleaning gate

### Files

- `.githooks/pre-commit` (modify)
- `.githooks/gate-cleanup.sh` (new, executable: `git update-index --chmod=+x`)
- `app/src-tauri/tests/integration/gate_cleanup.rs` (new) and its `mod gate_cleanup;` in `main.rs`
- `app/src-tauri/tests/integration/pre_commit_hook.rs` (extend)
- `README.md` (hook section, "Reclaiming disk space"), `AGENTS.md` (Commands hook sentence, Layout note)

### `.githooks/pre-commit` (skeleton; the gate sequence is unchanged)

```bash
#!/usr/bin/env bash
# Mandatory local gate (no CI). Install: git config core.hooksPath .githooks
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
repo="$(pwd -P)"

# Self-cleaning gate (DD-06 §4): own target dir, no incremental, one run at a time.
gate_dir="$repo/target/gate"
lock="$repo/target/gate.lock"          # outside gate_dir: a wipe must never delete a held lock
mkdir -p "$repo/target"
for p in "$gate_dir" "$lock" "$gate_dir/.gate-stamp"; do   # a symlinked target/ is allowed (decision 5)
  if [ -L "$p" ]; then echo "pre-commit: refusing symlinked ${p#"$repo"/}" >&2; exit 1; fi
done
exec 9>>"$lock"                        # append: never truncates
flock 9                                # waits for a concurrent gate run in this clone; mkdir-lock fallback when flock is missing (decision 2)
export CARGO_TARGET_DIR="$gate_dir"
export CARGO_INCREMENTAL=0

# Installed only after the lock is held: an interrupted *waiting* run must not
# clean another run's in-flight build.
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ! "$repo/.githooks/gate-cleanup.sh" "$gate_dir"; then
    echo "pre-commit: WARNING: gate cleanup failed; the next run retries it" >&2
  fi
  exit "$status"                       # the gate result, never the cleanup result
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Stamp: toolchain + lockfile. Any change wipes the gate dir before building.
stamp="$(rustc -vV; git hash-object Cargo.lock)"
if [ "$(cat "$gate_dir/.gate-stamp" 2>/dev/null || true)" != "$stamp" ]; then
  "$repo/.githooks/gate-cleanup.sh" --wipe "$gate_dir"
fi
mkdir -p "$gate_dir"
printf '%s\n' "$stamp" > "$gate_dir/.gate-stamp"

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build

# Exported-symbol gate (Linux, DD-06 §6). Fail closed: if nm fails, set -e aborts.
if [ "$(uname -s)" = "Linux" ]; then
  syms="$(nm -D --defined-only "$gate_dir/debug/app")"
  if printf '%s\n' "$syms" | grep -E '[[:space:]]sqlite3_'; then
    echo "pre-commit: app exports bundled sqlite3_* symbols (see app/src-tauri/build.rs)" >&2
    exit 1
  fi
fi

# ... existing secret / test-backend / clipboard greps, unchanged ...
cd app
npm run check
npm test
npm run build
```

Rules:

- Keep the order of the existing checks. The new symbol check goes right after
  `cargo build`, because that is where the binary it inspects is produced.
- `git hash-object` makes the stamp portable, since `sha256sum` and `shasum`
  differ by OS.
- Never use `set -x`, never echo env, and never `cat` user files. Output is limited
  to paths, sizes and check names.
- The trap uses absolute paths, because the hook `cd`s into `app/` at the end.
- Do not put `nm ... | grep` straight inside `if`. With `pipefail`, an `nm`
  failure would make the condition false, and the gate would silently pass.
- A plain `cargo clean` still removes `target/gate`, `target/gate.lock` and the stamp.

### `.githooks/gate-cleanup.sh` (new)

Interface: `gate-cleanup.sh [--wipe] <gate-dir>`. Without `--wipe` it runs
per-package clean and then the size cap. With `--wipe` it removes the whole gate
dir (used for the stamp change). Exit codes: `0` ok, `2` refused (nothing
deleted), `1` cleanup error.

```bash
#!/usr/bin/env bash
# Gate cleanup for .githooks/pre-commit (DD-06 §4). Deletes only Cargo outputs of
# this workspace's own packages inside <repo>/target/<gate>; keeps third-party
# libs. All refusals happen before anything is deleted or cargo is run.
set -euo pipefail
refuse() { echo "gate-cleanup: refusing ($1); nothing deleted" >&2; exit 2; }

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo="$(cd "$script_dir/.." && pwd -P)"

mode=clean
if [ "${1:-}" = "--wipe" ]; then mode=wipe; shift; fi
raw="${1:-}"
[ -n "$raw" ] || refuse "empty path"
while [ "$raw" != "/" ] && [ "${raw%/}" != "$raw" ]; do raw="${raw%/}"; done  # `-L dir/` would follow the link
[ "$raw" != "/" ] || refuse "/"
[ ! -L "$raw" ] || refuse "symlink"
[ -e "$raw" ] || exit 0                                   # nothing to clean
[ -d "$raw" ] || refuse "not a directory"
gate="$(cd "$raw" && pwd -P)"                             # resolves .. and symlinked parents
home="$(cd "${HOME:-/}" 2>/dev/null && pwd -P || true)"
[ "$gate" != "/" ] || refuse "/"
[ -z "$home" ] || [ "$gate" != "$home" ] || refuse "\$HOME"
[ "$gate" != "$repo" ] || refuse "repo root"
case "$gate" in "$repo/target/"?*) ;; *) refuse "outside <repo>/target/" ;; esac

cap="${P2PM_GATE_MAX_MIB:-}"                              # name: open question 1
[[ "$cap" =~ ^[1-9][0-9]{0,8}$ ]] || cap=8192             # invalid -> default, never eval'd

status=0
if [ "$mode" = clean ]; then
  for pkg in protocol core messenger-headless app; do
    "${CARGO:-cargo}" clean --offline --quiet --manifest-path "$repo/Cargo.toml" \
      --target-dir "$gate" -p "$pkg" || status=1
  done
fi
size_kib="$(du -sk "$gate" | cut -f1)"
if [ "$mode" = wipe ] || [ "$size_kib" -gt $((cap * 1024)) ]; then
  echo "gate-cleanup: wiping ${gate#"$repo"/} (${size_kib} KiB, cap ${cap} MiB)" >&2
  rm -rf -- "$gate" || status=1
fi
exit "$status"
```

- **No hand-written globs in `deps/`.** Only `cargo clean -p` decides which files
  belong to our packages. `rm -rf` is used only on the whole verified gate dir.
- `${CARGO:-cargo}`: under `cargo test`, `CARGO` points at the running cargo.
- `cargo clean -p` without `--profile` cleans the `debug` dir, which is where
  clippy, test and build all write in the gate. Check against a real gate run
  which uplifted files remain (`target/gate/debug/app`, `libapp_lib.rlib`,
  `messenger-headless`, `build/<pkg>-*`). If any of our packages' outputs
  survive, record them in the PR and ask before adding extra deletion. Do not
  invent globs.

### New test: `app/src-tauri/tests/integration/gate_cleanup.rs`

`#![cfg(unix)]`. Run the real `.githooks/gate-cleanup.sh` through `bash`. The
script only accepts gate dirs inside `<repo>/target/`, so create every temp gate
dir with `tempfile::Builder::new().prefix("gate-cleanup-test-").tempdir_in(<repo>/target)`.
Create `<repo>/target` first if missing. `tempfile` is already a dev-dep of `app`.

**Grep-gate caution:** this file lives under `app/`. It must not contain the
literal headless package or crate name, or the test-keyring type name. If a fake
headless artifact is needed, build its name with `concat()`, as
`no_test_backends.rs` does.

Cases:

1. **`removes_workspace_artifacts_and_keeps_third_party`**: in `<gate>/debug/deps/`
   create `core-0123456789abcdef`, `libcore-0123456789abcdef.rlib`,
   `libcore-0123456789abcdef.rmeta`, `app-0123456789abcdef`,
   `libapp_lib-0123456789abcdef.rlib`, `protocol-0123456789abcdef`, and a
   third-party `libserde-0123456789abcdef.rlib`. Add
   `<gate>/.fingerprint/core-0123456789abcdef/` too. Run the script, then assert
   that all workspace artifacts are gone, `libserde-*.rlib` remains, and the exit
   status is 0. (If cargo's real naming differs, mirror what a real gate dir
   contains. Do not loosen the assertion.)
2. **Refusals**: for each case, assert a non-zero exit (2), the reason on stderr,
   and that a sentinel file still exists:
   - empty path `""`;
   - `/`;
   - `$HOME`: run with `HOME=<repo>/target/gate-cleanup-test-x/home`, pass that dir, and put a sentinel inside it, so the `$HOME` rule is what fires;
   - the repo root (sentinel: `Cargo.toml` still exists);
   - a path outside `target/` (a `tempfile::tempdir()` in the OS temp dir, with a sentinel inside);
   - a symlinked gate dir: `<target>/gate-cleanup-test-y/link -> <target>/gate-cleanup-test-y/real`, with a sentinel and a fake `libcore-*.rlib` in `real`; both must survive. Also pass it as `link/` with a trailing slash.
3. **`size_cap_wipes_whole_gate_dir`**: a 2 MiB fake third-party file with
   `P2PM_GATE_MAX_MIB=1`. The gate dir must not exist afterwards.
4. **`invalid_cap_falls_back_to_default`**: for `0`, `-1`, `abc`, `1.5`, `" 1"`, `01` and
   `99999999999`, the same 2 MiB file remains, because the 8 GiB default applies.
5. **`wipe_mode_removes_gate_dir`**: `--wipe` removes it, and the same refusals apply
   (spot-check one: a symlinked dir with `--wipe` is refused).
6. The script exists and is executable (the same mode check as `pre_commit_hook`).

The test spawns cargo inside `cargo test`. That is safe because it uses its own
`--target-dir` and `--offline`. Remove the `CARGO_TARGET_DIR` env for the child
anyway, so only `--target-dir` applies.

### Extend `pre_commit_hook.rs`

Keep every existing needle, including `set -euo pipefail`, fmt, clippy (now
`--workspace`), `cargo test --workspace`, `cargo build`, the three greps and
`npm run check|test|build`. Add these:

```rust
let gate_needles = [
    "CARGO_TARGET_DIR=",                  // exported gate dir
    "target/gate",
    "CARGO_INCREMENTAL=0",
    "flock",
    "trap cleanup EXIT",
    "trap 'exit 130' INT",                // bare "INT"/"TERM" would match almost anything
    "trap 'exit 143' TERM",
    ".githooks/gate-cleanup.sh",
    "rustc -vV",
    "Cargo.lock",
    "nm -D --defined-only",
    "sqlite3_",
];
```

Also assert that `.githooks/gate-cleanup.sh` exists, is executable, and contains
`cargo`, ` clean `, `--target-dir` and `refuse`. Assert that the trap is
installed **after** `flock` (compare the `find()` offsets of the two needles), and
that the symbol check comes **after** the `cargo build` line.

### Docs (2A)

- README "Pre-commit hook (the test gate)":
  - The gate builds in `target/gate`, never `target/debug`.
  - Incremental compilation is off.
  - One run at a time per clone (lock).
  - A toolchain or `Cargo.lock` change wipes the gate dir and the next run is cold.
  - Cleanup always runs (pass, fail, Ctrl-C). Only our crates' outputs are deleted and third-party libs are kept.
  - The size cap and its env var (name per open question 1), positive integers only.
  - The Linux exported-symbol check.
  - A cleanup failure after a passing gate only warns, and the next run retries.
  - The existing verification instructions (planted secret) stay.
  - Never `--no-verify`.
- README "Reclaiming disk space" (§7):
  - `cargo clean` removes everything, including `target/gate`.
  - Optional: `cargo sweep --time 7` and `cargo sweep --installed`. Never required.
  - Manual `cargo test` writes to `target/debug` and is not auto-cleaned.
  - Folder meanings: `target/debug/` holds the dev app and manual runs, `target/gate/` is the hook's dependency cache, and `release/` holds end-user binaries.
- AGENTS.md: the Commands hook sentence lists the gate dir, the cleanup and the symbol check. Commands also gains the §7 cleanup commands (`cargo clean`; optional `cargo sweep --time 7` / `cargo sweep --installed`, never required; manual `cargo test` output in `target/debug` is not auto-cleaned). Layout notes say that `target/gate/` is hook-owned and that its workspace outputs are deleted after every run.

---

## Sub-task 2B: `npm run release`

### Files

- `package.json` (root): add `"release": "node scripts/build-release.mjs"`.
- `scripts/build-release.mjs` (new; Node builtins only, no new dependencies).
- `.gitignore`: add `release/`.
- `.dockerignore`: add `release/`. Release binaries should not enter the Docker build context. This file is not in DD-06's "Affected" list, so mention it in the PR.
- `README.md` ("Building the end-user binary", "Running the release binary"), `AGENTS.md` (Commands, Layout note).

### Script structure (`scripts/build-release.mjs`)

Steps run in this order and stop at the first failure (DD-06 §5):

1. **Preflight.**
   - `rustc --version` must be ≥ `rust-version` in the root `Cargo.toml` (parse both). `cargo` must be present.
   - `npm_execpath` must be set (that is, the script was launched through `npm run`). Otherwise exit with "run `npm run release`".
   - Linux: `pkg-config --exists webkit2gtk-4.1 gtk+-3.0 libsoup-3.0`. Check that `nm` and `readelf` (binutils) are present. If anything is missing, print the distro packages from the Tauri prerequisites page.
   - macOS: `xcode-select -p`.
   - Windows: print the MSVC Build Tools and WebView2 hint.
   - Platform-specific commands only run inside `process.platform === '...'` branches.
2. **Pinned inputs.** `npm ci --no-audit --no-fund` in `app/`.
3. **Throwaway target dir.** `realpath(await mkdtemp(join(os.tmpdir(), 'p2pm-release-')))`. Record the path. It is the **only** directory the script may delete.
4. **Child env.**
   - Copy `process.env`. **Delete** `MESSENGER_PROFILE`, `MESSENGER_RELAY_URL`, `RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS` and every `CARGO_PROFILE_RELEASE_*` (a local env override must not undo strip/LTO).
   - Set `CARGO_TARGET_DIR=<temp>`.
   - Set `CARGO_ENCODED_RUSTFLAGS` (`\x1f`-separated, so paths with spaces are safe) to the remaps: Cargo home → `/cargo`, rustup home → `/rustup`, the temp target dir → `/target`, the repo → `/src`.
     - Cargo home is `CARGO_HOME` or `~/.cargo`. Rustup home is `RUSTUP_HOME` or `~/.rustup`.
     - rustc applies the last matching remap, so put the more specific prefixes last.
     - The `/target` remap is needed on Windows, where the temp dir is under the user profile. Mention it in the PR as an addition to DD-06's three prefixes.
   - Dropping the inherited rustflags loses no required linker argument: the Linux
     `-Wl,--exclude-libs,ALL` comes from `app/src-tauri/build.rs`
     (`cargo::rustc-link-arg`), which RUSTFLAGS scrubbing does not affect, and the
     `nm -D` check in step 6 backstops it. Do not copy any linker flag into the
     script.
   - Known limits, do not paper over them:
     - The scrub covers env only. A `[profile.release]` override in the user's
       `~/.cargo/config.toml` still applies. The strip check catches the
       security-relevant one (`strip`); LTO/opt-level drift only changes size, so
       print the binary size and compare it with the expected ~19.5 MB.
     - `--remap-path-prefix` only affects rustc. C sources compiled through `cc`
       (vendored OpenSSL, SQLCipher) can embed `__FILE__` paths under the temp
       target dir. On Linux that dir is outside `$HOME` (measured: 0 home paths).
       On Windows it is under the user profile, so the "No builder paths" check
       may fire there. If it does, fix the input (e.g. a C-compiler prefix-map
       flag for `cc`) and record it. Never weaken or skip the check.
5. **Build.** `tauri build --no-bundle --ci -- --locked` with `cwd: app/`. That runs `beforeBuildCommand` (`npm run build`), embeds `frontendDist`, and builds only `app` with default features.
6. **Verify** (all checks below). Any failure means nothing is published.
7. **Publish.**
   - `mkdir release/`. If the final name exists, refuse (open question 4).
   - `copyFile` with `COPYFILE_EXCL` to `release/.<name>.partial-<random>` and `chmod 0o755` (not on Windows).
   - Hash the copy and compare it with the hash of the verified bytes, then `rename` to the final name.
   - Write `<name>.sha256` the same way (partial, then rename), in `sha256sum -c` format: `<hex>  <name>\n`.
   - Print the path, size and hash.
8. **Finally (always).** Delete the run's own `.partial` file if one exists, then delete the temp target dir, with the safety refusals.
   - The partial files are the one, narrow exception to "never deletes anything
     in `release/`": they are created by this run under a random name and must go
     so that an interrupt leaves no partial file (DD-06 Testing). Name this in the
     PR. Nothing else in `release/` is ever deleted or overwritten.

```js
#!/usr/bin/env node
// npm run release: builds the end-user binary (DD-06 §5). Node builtins only.
import { spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { constants as fsc } from 'node:fs';
import { mkdtemp, realpath, lstat, rm, readFile, copyFile, rename, chmod, unlink, mkdir, access } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = await realpath(path.join(path.dirname(fileURLToPath(import.meta.url)), '..'));
const appDir = path.join(repo, 'app');
const releaseDir = path.join(repo, 'release');
let tempTarget = null; // set only from mkdtemp in this run
let partials = [];     // the only files this run may delete in release/
let child = null;
let interrupted = false;

for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(sig, () => { interrupted = true; child?.kill(sig); }); // no exit here: finally must run
}

function run(cmd, args, { cwd = repo, env = process.env, capture = false } = {}) {
  if (interrupted) return Promise.reject(new Error('interrupted'));
  return new Promise((resolve, reject) => {
    // Argument arrays only, never a shell: no injection through paths or env.
    child = spawn(cmd, args, { cwd, env, shell: false, stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit' });
    let out = '';
    child.stdout?.on('data', (d) => { out += d; });
    child.on('error', reject);
    child.on('close', (code, signal) => {
      child = null;
      if (interrupted) reject(new Error('interrupted'));
      else if (code === 0) resolve(out);
      else reject(new Error(`${path.basename(cmd)} ${args[0] ?? ''} failed (${signal ?? code})`));
    });
  });
}

const npm = (args, opts) => run(process.execPath, [process.env.npm_execpath, ...args], opts);
const tauri = (args, opts) =>
  run(process.execPath, [path.join(appDir, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), ...args], opts);

async function main() {
  await preflight();
  await npm(['ci', '--no-audit', '--no-fund'], { cwd: appDir });
  tempTarget = await realpath(await mkdtemp(path.join(os.tmpdir(), 'p2pm-release-')));
  await tauri(['build', '--no-bundle', '--ci', '--', '--locked'], { cwd: appDir, env: releaseEnv(tempTarget) });
  const bin = await builtBinary(tempTarget); // <temp>/release/app[.exe]; confirm the name from the build output
  const bytes = await verify(bin);           // throws on any failed check
  await publish(bin, bytes);
}

try {
  await main();
} catch (e) {
  console.error(`release: ${e.message}`);
  process.exitCode = interrupted ? 130 : 1;
} finally {
  await cleanup(); // sets a non-zero exit code if a refusal or rm error happens
}
```

`cleanup()` deletes partials first (only paths in `partials`). It then removes
`tempTarget` only if **all** of these hold:

- it is non-null;
- `lstat` says it is a real directory, not a symlink;
- `path.dirname(tempTarget) === await realpath(os.tmpdir())`;
- the basename starts with `p2pm-release-`;
- it is none of: the filesystem root, `os.homedir()`, `repo`, `releaseDir`, `<repo>/target`, `<repo>/app/node_modules`.

If any condition fails, it refuses, prints the reason and exits non-zero. It
uses `rm(tempTarget, { recursive: true, force: true, maxRetries: 3 })`, which
does not follow symlinks inside. It never touches `release/` (other than this
run's partials), `target/`, the source tree, `node_modules`, the OS app-data dir
(`messenger.db`, logs) or the keychain.

### Binary verification (DD-06 Security, "End-user binary checks")

Read the binary into a Buffer once (about 20 MB) and run byte searches in Node,
which is portable.

| Check | How |
|---|---|
| No test code | None of `FileKeychain`, `MemoryContactStore`, `FailingContactStore`, `MemoryMessageStore`, `FailingMessageStore`, `ChatPeer`, `RecordingHistoryEraser`, `messenger_headless`, `messenger-headless` (`buf.indexOf`) |
| No builder paths | No `os.homedir()` and no `repo` (as given and as realpath; on Windows also with `/` separators). Skip the home check only if home is `/`, and print that it was skipped. |
| No exported SQLite symbols (Linux) | `nm -D --defined-only <bin>`, with output captured. A non-zero exit from `nm` fails the check. Any `sqlite3_` line fails. |
| Stripped | Linux: `readelf -S -W` has no `.symtab` and no `.debug_` sections. macOS: `otool -l` has no `__DWARF`. Windows: parse the PE section table in Node; no `.debug*` sections. |
| No devtools | `app/src-tauri/Cargo.toml` and the root `tauri =` workspace dependency do not mention `devtools`. Optionally also `cargo tree -p app -e features -i tauri --locked` (with output captured) has no `devtools` feature. |
| Release mode / frontend embedded | Every asset file name under `app/dist/assets/` (e.g. `index-<hash>.js`) occurs in the binary. The script never passes `--debug`. Do not treat the absence of `localhost:1420` as proof without checking it first. |
| No baked env | The child env never contains `MESSENGER_PROFILE` / `MESSENGER_RELAY_URL` (assert this in code before spawning). The code has no `option_env!`/`env!` for them (`grep` in preflight or review). |
| Pinned inputs | `npm ci` and `--locked` only. |

Run the manifest-only checks (devtools, no baked env in the source) both in
preflight, so a bad manifest fails before a 3-minute build, and again in step 6,
so they stay part of "verify before publish".

Keep the checks as small pure functions (e.g. `findForbidden(buf, needles)`) so
06-03 can re-run them on bad inputs. **No bypass flag** (no `--skip-verify`,
no "no remap" option). Proving that each check fires is done by temporary local
edits in 06-03, which are never committed.

### Docs (2B)

- README "Building the end-user binary":
  - `npm run release`.
  - Prerequisites per OS: Linux WebKitGTK 4.1 / GTK 3 / libsoup 3 dev packages, binutils and pkg-config; macOS Xcode CLT; Windows MSVC Build Tools.
  - Output: `release/p2p-messenger-<ver>-<os>-<arch>[.exe]` plus `.sha256`.
  - What it checks.
  - Verify the hash with `sha256sum -c <name>.sha256` (Linux), `shasum -a 256 -c` (macOS) or `Get-FileHash` (Windows).
  - Builds are not claimed to be reproducible.
  - The script never deletes anything in `release/`.
  - Interrupting the script leaves no temp dir and no partial file.
- README "Running the release binary":
  - Linux: WebKitGTK 4.1, GTK 3, libsoup 3, D-Bus and a Secret Service keychain.
  - Windows: the WebView2 runtime (preinstalled on 10/11).
  - macOS: nothing extra. A bare executable runs, and the `.app` is deferred.
  - `--profile` / `MESSENGER_PROFILE` / `MESSENGER_RELAY_URL` work at runtime, exactly as in dev.
- AGENTS.md: `npm run release` in Commands. Layout notes cover `release/` (git-ignored end-user binaries) and `scripts/build-release.mjs`.

---

## Quality gates (all must pass)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace            # includes gate_cleanup and the extended pre_commit_hook
cargo build
cd app && npm ci && npm test && npm run check && npm run build
npm run release                   # 2B: produces release/p2p-messenger-0.1.0-linux-x86_64 + .sha256
```

Commit through the hook. **Never use `--no-verify`.** After the 2A merge, the
commit itself is the first real self-cleaning run. Afterwards, check
`find target/gate/debug -maxdepth 2 -type f \( -name 'app' -o -name 'app-*' -o -name 'integration-*' -o -name 'core-*' -o -name 'protocol-*' -o -name 'libcore-*' -o -name 'libapp_lib-*' \)`
(prints nothing), and confirm that `target/debug` has the same mtime and size as
before. `npm start` must still launch the app.

## Guardrails (AGENTS.md; do not weaken)

- Never reference `SecretKeyBytes` / `DbKey` under `app/`, not even in comments or the new `gate_cleanup.rs`.
- Never reference `FileKeychain`, `messenger-headless` or `messenger_headless` literally under `app/`. Use `concat()` in tests. (`.githooks/` and `scripts/` are outside `app/`, so the release script's needle list may contain them.)
- Clipboard only in `app/src/views/MyInfo.svelte` (and tests).
- Logs and hook/release output never contain plaintext, keys, secrets or environment values. Output is limited to paths, sizes, hashes and check names.
- `core` keeps `#![forbid(unsafe_code)]` (not touched here).
- **No wire-format change, and no `PROTOCOL_VERSION` bump.**
- Only files with real content: no empty `scripts/` or `release/` placeholders. `release/` is created at runtime and ignored, so never commit it.
- Don't weaken any gate. Every old hook check and needle stays. Overrides (the cap) accept only positive integers. There is no verification-skip option.
- Never bypass the hook (`--no-verify`), and never document doing so.
- **Credentials and secrets:** never write credentials or secrets in code or config. All credentials are supplied at runtime (OS keychain, or env at runtime). **The release script must never set or bake `MESSENGER_PROFILE`, `MESSENGER_RELAY_URL` or any other environment value into the binary.** It strips them from the build env, and the binary keeps no defaults derived from the builder's environment.

## Security

| Risk | Prevention | How to check |
|---|---|---|
| Destructive deletion (`rm -rf` on `/`, `$HOME`, the repo, a typo path) | All refusals run before any deletion. The path is resolved with `pwd -P` and must start with `<repo>/target/`. Exit 2. | `gate_cleanup` refusal cases |
| Symlink attack: gate dir, `target/` or lock file replaced by a link (truncating or deleting elsewhere) | The hook refuses symlinked `target/gate`, the lock and the stamp (a symlinked `target/` is allowed, decision 5). The lock is opened with `>>`. The script refuses a symlinked gate dir, with trailing slashes stripped before `-L`. Symlinked parents fall outside the prefix after `pwd -P`. | `gate_cleanup` symlink case, plus a manual `ln -s` of `target/gate` making the hook refuse |
| TOCTOU / concurrent runs deleting each other's build | `flock` for the whole run. The trap is installed only after the lock is held. The lock lives outside the gate dir, so a wipe cannot delete a held lock. | Two concurrent `bash .githooks/pre-commit` runs: the second waits, and both pass |
| Cleanup error masking a gate failure (or the reverse) | `status=$?` is captured first and `exit "$status"`. A cleanup failure only warns. | Planted-secret commit: rejected **and** cleaned (06-03 manual check 2) |
| Env-var injection through the cap | Regex `^[1-9][0-9]{0,8}$`, never `eval`'d, and otherwise the default | `invalid_cap_falls_back_to_default` |
| Command injection in the Node script | `spawn` with argument arrays and `shell: false`. npm runs via `process.execPath` + `npm_execpath`, and tauri via its `tauri.js`. No `exec`/`execSync` strings. | Code review: `grep -n "exec(\|execSync\|shell: true" scripts/` is empty |
| Leaking builder home/repo paths into the binary | `--remap-path-prefix` for Cargo home, rustup home, the temp target and the repo. The binary is scanned. | The "No builder paths" check. A temporary no-remap build must fail (06-03). |
| Exported `sqlite3_*` (WebKit/libsoup interposition) | The `build.rs` linker arg, plus `nm -D` in both the hook and the release script, failing closed | Removing the arg temporarily is rejected by the hook and the release (06-03) |
| Test doubles or headless in the end-user binary | Default features only, `-p app` through tauri, and the forbidden-name scan | A temporary `test-support` build must fail (06-03) |
| Devtools in release | Manifest (and optional `cargo tree`) check | Plant `features = ["devtools"]` temporarily; the release must fail |
| Stale or unpinned inputs | `npm ci`, `--locked`, and a fresh target dir (no `target/`/`target/gate` reuse) | A stale-`Cargo.lock` run must fail (06-03) |
| Release env overrides weakening the profile | Strip `RUSTFLAGS*` and `CARGO_PROFILE_RELEASE_*` from the child env | `CARGO_PROFILE_RELEASE_STRIP=false npm run release` still produces a stripped binary (or fails the strip check) |
| Half-written or tampered output | Copy with `COPYFILE_EXCL` to a unique partial, compare hashes, then atomic `rename`. Partials are removed in `finally`. | Interrupt test in 06-03: no `.partial` left |
| Temp dir left behind with build intermediates (~1.9 GB) | `finally` plus signal handlers that never exit early | After success, failure and Ctrl-C: `ls "${TMPDIR:-/tmp}" \| grep p2pm-release-` is empty |
| Hook or release printing secrets/env | No `set -x`, no env dumps, fixed messages only | Read the hook and script output during runs |

## Definition of done (this step)

- [ ] Hook: `CARGO_TARGET_DIR=<repo>/target/gate`, `CARGO_INCREMENTAL=0`, `flock` lock, a stamp wipe on `rustc -vV` / `Cargo.lock` change, the unchanged gate sequence, the Linux `nm -D --defined-only` `sqlite3_*` check, and `gate-cleanup.sh` always run from the `EXIT`/`INT`/`TERM` trap with the exit status preserved. *(DD DoD 6)*
- [ ] `gate-cleanup.sh`: per-package `cargo clean -p` scoped to the gate dir for `protocol`, `core`, `messenger-headless` and `app`. No globbing in `deps/`. Path-safety refusals. A size cap (default 8 GiB, positive-integer override). Covered by the `gate_cleanup` test. *(DD DoD 7)*
- [ ] The `pre_commit_hook` self-test asserts every old needle plus the gate-dir, incremental, lock, trap, cleanup-script, stamp and symbol-check needles. *(DD DoD 8)*
- [ ] `npm run release` and `scripts/build-release.mjs` work as in §5: preflight, `npm ci`, `--locked`, a throwaway temp target dir, path remapping, `tauri build --no-bundle --ci`, every binary check, the atomic copy plus `.sha256`, and guaranteed temp-dir cleanup with refusals. `release/` is in `.gitignore`. *(DD DoD 11)*
- [ ] The release binary shows 0 `sqlite3_*` exports (`nm -D`). *(DD DoD 9, release half)*
- [ ] README: hook section, "Reclaiming disk space", "Building the end-user binary", "Running the release binary". AGENTS.md: Commands and Layout notes (`target/gate/`, `release/`, `scripts/`). *(DD DoD 15/16, partial)*
- [ ] The resolved decisions 1–6 are reflected in the code and docs.
- [ ] All quality gates pass. The commit went through the hook, which then left no workspace artifacts in `target/gate` and did not touch `target/debug`. `npm start` launches the app.
- [ ] PR follows the template. It names the affected areas (tooling: `.githooks`, `scripts`; `app` tests), states **"wire format unchanged, no `PROTOCOL_VERSION` bump"**, and lists the verification commands and the produced binary's size and hash.
