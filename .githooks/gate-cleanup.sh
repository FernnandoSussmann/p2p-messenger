#!/usr/bin/env bash
# Gate cleanup for .githooks/pre-commit. Usage: gate-cleanup.sh [--wipe] <gate-dir>
# Deletes only Cargo outputs of this workspace's own packages inside
# <repo>/target/<gate>; keeps third-party libs. All refusals happen before
# anything is deleted or cargo is run. Exit: 0 ok, 2 refused, 1 cleanup error.
set -euo pipefail
refuse() { echo "gate-cleanup: refusing ($1); nothing deleted" >&2; exit 2; }

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo="$(cd "$script_dir/.." && pwd -P)"

mode=clean
if [ "${1:-}" = "--wipe" ]; then mode=wipe; shift; fi
raw="${1:-}"
[ -n "$raw" ] || refuse "empty path"
while [ "$raw" != "/" ] && [ "${raw%/}" != "$raw" ]; do raw="${raw%/}"; done # `-L dir/` would follow the link
[ "$raw" != "/" ] || refuse "/"
[ ! -L "$raw" ] || refuse "symlink"
[ -e "$raw" ] || exit 0
[ -d "$raw" ] || refuse "not a directory"
gate="$(cd "$raw" && pwd -P)"
home="$(cd "${HOME:-/}" 2>/dev/null && pwd -P || true)"
[ "$gate" != "/" ] || refuse "/"
[ -z "$home" ] || [ "$gate" != "$home" ] || refuse "\$HOME"
[ "$gate" != "$repo" ] || refuse "repo root"
target_real="$(mkdir -p "$repo/target" && cd "$repo/target" && pwd -P)" # a symlinked target/ is allowed
case "$gate" in "$target_real/"?*) ;; *) refuse "outside <repo>/target/" ;; esac

cap="${P2PM_GATE_MAX_MIB:-}"
[[ "$cap" =~ ^[1-9][0-9]{0,8}$ ]] || cap=8192 # invalid -> default, never eval'd

status=0
if [ "$mode" = clean ]; then
  for pkg in protocol core messenger-headless app; do
    env -u CARGO_TARGET_DIR "${CARGO:-cargo}" clean --offline --quiet \
      --manifest-path "$repo/Cargo.toml" --target-dir "$gate" -p "$pkg" || status=1
  done
fi
size_kib="$(du -sk "$gate" | cut -f1)"
if [ "$mode" = wipe ] || [ "$size_kib" -gt $((cap * 1024)) ]; then
  echo "gate-cleanup: wiping ${gate#"$repo"/} (${size_kib} KiB, cap ${cap} MiB)" >&2
  rm -rf -- "$gate" || status=1
fi
exit "$status"
