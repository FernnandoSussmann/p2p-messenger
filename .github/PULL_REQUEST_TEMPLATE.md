## Summary

<!-- What does this PR do, and why? Link the issue it closes: Closes #123 -->

## Changes

<!-- Bullet the notable changes. Name the crate/slice each one touches. -->

-

## Affected crates / slices

<!-- Check what this PR touches. -->

- [ ] `protocol` — wire types / protocol version
- [ ] `core` — identity · discovery · messaging · contacts · storage
- [ ] `app` — Tauri + Svelte shell
- [ ] tooling / CI / docs

## Protocol impact

<!-- E2E and the wire format are load-bearing. Answer explicitly. -->

- [ ] No change to the wire format
- [ ] Wire format changed **and** `PROTOCOL_VERSION` was bumped
- [ ] N/A

## Testing

<!-- How did you verify this? Commands, manual steps, platforms. -->

- [ ] `cargo test` passes
- [ ] `cargo fmt --check` and `cargo clippy` are clean
- [ ] Manually verified (describe below)

## Checklist

- [ ] No empty files, stub modules, or placeholder folders
- [ ] No message plaintext, private keys, or secrets added to logs
- [ ] Docs / README updated if behavior or structure changed
