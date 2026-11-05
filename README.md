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
| `core`     | planned | UI-agnostic logic by slice: identity, discovery, messaging…  |
| `app`      | present | Thin Tauri v2 + Svelte shell over `core` (`app/`)            |

## Run the app

Requires Rust, Node.js 20+, and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
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
cargo test
```

## License

[GPL-3.0-only](LICENSE).

[iroh]: https://www.iroh.computer/
