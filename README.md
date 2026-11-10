# p2p-messenger

A fully decentralized, end-to-end-encrypted peer-to-peer messenger written in
Rust. There is no central server: each user's keypair *is* their identity, and
peers connect directly (addressed by public key) over [iroh], with relay
fallback for ciphertext when hole punching fails.

Desktop (Linux / macOS / Windows) is the v1 target. See [BLUEPRINT.MD](BLUEPRINT.MD)
for the full design, scope, and what is deferred to v2+.

## Workspace layout

This is a Cargo workspace built from vertical feature slices. Crates are added
only once they carry real code — no empty stubs or placeholder modules.

| Crate      | Status  | Responsibility                                               |
| ---------- | ------- | ------------------------------------------------------------ |
| `protocol` | present | Shared serde wire types (`Envelope`, `Payload`, …) + version |
| `core`     | present | UI-agnostic logic by slice: identity, discovery, messaging…  |
| `app`      | present | Thin Tauri v2 + Svelte shell over `core`                     |

## Run the app

Requires Rust 1.95 or newer (the workspace MSRV), Node.js 20+, and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
(on Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev`).

```sh
npm start
```

This installs the frontend dependencies, starts the Vite dev server, and opens
the desktop window. Running plain `cargo run` instead shows "connection refused"
because nothing serves the dev frontend.

The app stores its keys in your OS keychain, so the first launch offers to
create a real identity.

## Build & test

```sh
cargo test --workspace
cd app && npm ci && npm test && npm run check && npm run build
```

### Pre-commit hook (the test gate)

There is no CI, so the tracked hook in `.githooks/pre-commit` is the mandatory
local gate. Install it once per clone:

```sh
git config core.hooksPath .githooks
```

It blocks the commit on any failure of: `cargo fmt --all --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --workspace` (includes the secret-leak guards), the secret grep
gates (no key types under `app/`; clipboard only in `MyInfo.svelte`), and the
frontend `npm run check`, `npm test` and `npm run build`.

Verifying the gate: stage a file under `app/` containing a planted
secret-looking identifier (e.g. a Tauri return type referencing the key type
name) and run `git commit`; the hook must reject it. Do this manually after
changing the hook.

### Manual GUI checklist

No Tauri WebDriver driver is available in this environment, so the first-run
flow is automated at command-invoke level (`first_run_flow_end_to_end` in
`app/src-tauri/src/identity_cmd.rs`) and component level (vitest). Check by
hand with `cd app && npm run tauri dev` on a fresh profile and empty keychain:

1. First launch shows the wizard; empty or whitespace names are rejected.
2. A valid name lands on the main screen; relaunch skips the wizard.
3. My Info shows name, full NodeId, ticket and QR; both copy buttons paste
   exactly the ticket / NodeId.
4. Denying keychain access shows the blocking error screen, never the wizard.

## License

[GPL-3.0-only](LICENSE).

[iroh]: https://www.iroh.computer/
