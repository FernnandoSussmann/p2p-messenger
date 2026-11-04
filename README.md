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
| `app`      | planned | Thin Tauri v2 + Svelte shell over `core`                     |

## Build & test

```sh
cargo test
```

## License

[GPL-3.0-only](LICENSE).

[iroh]: https://www.iroh.computer/
