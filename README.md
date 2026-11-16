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
| `core`     | present | UI-agnostic logic by slices: `identity`, `discovery` (iroh node, LAN mDNS discovery + presence) and `contacts` (contact book, trust, encrypted contact store) and `messaging` (message store and delivery engine) and `storage` (the single encrypted `messenger.db`: keyed open, typed errors, migrations, one-shot import of the old plaintext files) and `logging` (rotating diagnostic log file) |
| `app`      | present | Thin Tauri v2 + Svelte shell over `core`                     |
| `headless` | present | Test-only headless node + file keyring for Level-3 simulation; never shipped, never used by `app` |

## Run the app

Requires Rust 1.95 or newer (the workspace MSRV), Node.js 20+, and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)
(on Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev`).
A C compiler, `perl` and `make` are also required: SQLCipher and a vendored
OpenSSL are compiled from source (on Windows, install Strawberry Perl). The
first build takes a few minutes.

```sh
npm start
```

This installs the frontend dependencies, starts the Vite dev server, and opens
the desktop window. Running plain `cargo run` instead shows "connection refused"
because nothing serves the dev frontend.

The app stores its keys in your OS keychain, so the first launch offers to
create a real identity.

### Connect two devices by ticket

1. On device A open **My Info** and copy the ticket (the long live form that
   carries the relay URL and current addresses).
2. On device B open **Network**, paste it and press **Connect and remember this
   device**.
3. Device A shows an Accept/Decline prompt marked **Unverified** (the name is
   self-asserted; trust is the NodeId). Accept, and both sides show Connected.
4. Once A accepts, device B shows a **Name this contact** prompt prefilled
   with the name A gave (editable). **Save** keeps or renames the contact;
   **Not now** keeps it under the prefilled name.

Trust rule: trust is the peer's NodeId, kept as a contact.

- **Accept** asks for your name for them and adds an **Unverified** contact.
  The field is prefilled with what the peer calls itself (or "Peer <id>" when
  it sent no name or an invalid one); edit it before clicking Accept. Nothing
  is saved until you click Accept.
- A peer you dial is remembered once it replies with a valid Hello, as an
  Unverified contact named after what it calls itself, or "Peer <id>". It may
  later connect to you without a prompt.
- The default nickname is a snapshot taken when the contact is created; later
  name changes by the peer never change your nickname.
- Two contacts may share a nickname; the NodeId tells them apart.
- Declined or expired requests leave no trace.
- Contacts are kept per account in the encrypted `messenger.db` (see
  Encrypted storage).
- An existing `known_peers.json` is imported once at startup as Unverified
  contacts and then removed; its peers still connect without a prompt. If the
  file is unreadable it is skipped, left on disk **unencrypted**, and those
  peers are prompted again; you can delete it yourself.

Names (yours and peers') may no longer contain invisible characters such as
zero-width spaces. Joiners are allowed only inside emoji sequences (👨‍👩‍👧,
🏳️‍🌈, subdivision flags such as 🏴󠁧󠁢󠁥󠁮󠁧󠁿). A saved name that now fails is treated as
unset, and My Info asks for a new one.

NAT traversal (hole punching, relay fallback) is provided by iroh; this app
adds no traversal logic of its own. Relays only ever carry ciphertext.

### Contacts

**Contacts** is the default tab. Each row shows only your own nickname (bold),
with the NodeId small and gray beneath it. What a peer calls itself appears only
in the Accept, naming and rename prompts, never in the list.

- **Unverified** vs **Verified by you**: the latter is your own attestation
  (**Mark verified…**, undo with **Mark unverified**). The app does not check it
  cryptographically.
- **Rename** changes only your nickname. **Connect** appears for contacts
  currently on the local network; they reconnect without a prompt.
- Names may contain emoji sequences (👨‍👩‍👧, 🏳️‍🌈, 🏴󠁧󠁢󠁥󠁮󠁧󠁿) but not stray
  zero-width characters.
- **Remove…** always removes trust and closes any open connection, so their
  next connect prompts again. Your message history is deleted only if you tick
  "Also delete our message history" (off by default). Ticking it deletes the
  stored messages with that contact from the encrypted message database.
- Copying a NodeId: choose **Show full NodeId**, select it and use your
  system's copy shortcut.
- Contacts, including nicknames, are stored encrypted in `messenger.db`.

### Encrypted storage

Settings (display name, presence, relay override), contacts and messages all
live in `messenger.db` in the (profile) data directory, encrypted with
SQLCipher under the database key held in your OS keychain. The `storage` slice
opens it, runs an integrity check, and applies forward-only, transactional
schema migrations (messages, settings, contacts and the import bookkeeping are
migrations 1 to 4). Nothing else on disk holds user data.

If the database cannot be opened (wrong or missing key, damaged file, newer
schema, failed migration, or the file is missing while an identity exists), the
app shows **Encrypted storage unavailable** with a **Retry** button, and
networking stays down until it opens. The file is never reset, deleted or
recreated, and there is no "reset database" action. A missing file is
recreated only during first-run provisioning or while importing legacy
plaintext data. A first-run provisioning that fails removes only the empty
database it just created, so running the wizard again works.

**Upgrading from the plaintext files.** Older versions kept `settings.json`,
`contacts.json` and `known_peers.json` in the data directory. On the first
start the app imports them into `messenger.db` in one transaction per file,
then overwrites them with zeros and deletes them. A contact you remove later
is never re-imported. If `settings.json` or `contacts.json` is corrupt or not a
regular file, the storage screen shows an error, nothing is imported and the
file is left untouched (fix or delete it, then Retry). A corrupt
`known_peers.json` does not block: it is skipped, those peers are prompted
again, and the file **stays on disk unencrypted** until you delete it.
Overwriting a file does not guarantee erasure on SSDs, copy-on-write file
systems (APFS, btrfs) or in backups and snapshots.

### Diagnostic logs

The app writes a local diagnostic log, `messenger.log`, to the OS app log
directory (a `profiles/<name>` subfolder under it for a `--profile`):

- Linux: `$XDG_DATA_HOME/<app identifier>/logs` (usually
  `~/.local/share/<app identifier>/logs`)
- macOS: `~/Library/Logs/<app identifier>`
- Windows: `%LOCALAPPDATA%\<app identifier>\logs`

The default level is `info`. Set `MESSENGER_LOG` to `off`, `error`, `warn`,
`info`, `debug` or `trace` to change it (our own crates only; third-party
crates are capped at `warn`). Any other value refuses to start. `off` creates
no file.

Size is bounded: at most 5 files of 2 MiB each (`messenger.log` plus four
rotated copies). On Unix the folder is `0700` and the files `0600`.

Logged: connection lifecycle, hole-punch and path events, relay status,
discovery events and errors, identified by short NodeId and address.
Never logged: message bodies, search queries, display names and nicknames, the
identity key, the database key and ticket strings. Logs are local only and
never sent anywhere.

Logs reveal *who* you connected to and *when* (short NodeIds, IP addresses).
Deleting the log folder is a privacy action and is safe at any time.

### Chat

Open **Contacts**, then a contact row's **Message** button. **Back** (or any tab)
returns to the tabs; an unread count shows on the button while the chat is closed.
Commands: `chat_send`, `chat_resend`, `chat_history`, `history_search` and `history_clear`; events:
`message_received`, `message_status` and `chat_resync`.

- Status: **Sending**, **Delivered**, **Pending** (waits for a connection and is
  retried this session, and again after a restart) and **Not delivered** with a
  **Resend** button.
- Removing a contact marks its waiting messages Not delivered.
- Delivery needs both peers online: there is no store-and-forward and no
  automatic dialing in v1. A Pending message goes out once you connect again.
- A message is limited to 16 KiB (UTF-8 bytes). Ctrl/Cmd+Enter sends; Enter
  inserts a newline.
- Messages are ordered by the sender's clock, then by arrival, so a wrong clock
  on the other side can misplace their messages. Times are shown in your local
  time zone.
- Text is shown literally: no links or formatting, and hidden direction,
  zero-width or control characters show as �.
- Search within a conversation finds words in its history. History and search
  read the encrypted store, so they work even if the node failed to start.
  Storage is `messenger.db` (see Encrypted storage).
- **Clear history…** (in the chat header, also for removed contacts) deletes the
  whole conversation after a confirmation. The messages are removed from this
  device's encrypted database and the space they used is overwritten. This cannot
  remove the other person's copy, backups or snapshots of this computer, or
  traces some storage hardware (such as SSDs) may keep. Messages still waiting to
  be sent are marked Not delivered. Remove contact with "Also delete our message
  history" uses the same path.

### On this network, presence and relay

**Contacts → On this network** lists non-contact peers found via mDNS. Entries
are always marked **Unverified**: the name is self-asserted, only the NodeId
counts. **Add…** asks for your nickname, saves the contact and connects; the
other side gets the Accept/Decline prompt. Discovered peers are kept in memory
only, never on disk.

**Visible on this network** (default on, saved per account) controls only
whether *you* advertise on the LAN. Off hides you from other devices' LAN lists
and withdraws your display name from the mDNS record; you still browse, and
anyone holding your ticket can still connect. It does not make you offline:
iroh's internet address lookup keeps publishing your addresses (never your
name) for ticket holders.

Relay override: set `MESSENGER_RELAY_URL` at launch to use your own relay
(`https`, or `http` for a loopback host only), or `default` to go back to the
default relay mesh. The value is saved per account and applied when the node
starts; an invalid value stops the app from starting. There is no GUI editor in
v1. Relays see NodeIds, addresses and timing, never plaintext; self-hosting a
relay is the way to keep that metadata off shared infrastructure.

## Build & test

```sh
cargo test --workspace   # includes the Level-1 two-node tests (real iroh endpoints on loopback)
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
`cargo test --workspace` (includes the secret-leak guards, the log redaction
guard `log_redaction_guard` and the SQL namespacing guard
`sql_namespacing_guard`), the secret grep
gates (no key types under `app/`; clipboard only in `MyInfo.svelte`), and the
frontend `npm run check`, `npm test` and `npm run build`.

Verifying the gate: stage a file under `app/` containing a planted
secret-looking identifier (e.g. a Tauri return type referencing the key type
name) and run `git commit`; the hook must reject it. Do this manually after
changing the hook.

### Manual GUI checklist

No Tauri WebDriver driver is available in this environment, so the first-run
flow is automated at command-invoke level (`first_run_flow_end_to_end` in
`app/src-tauri/src/identity_cmd.rs`, `ticket_connect_accept_flow_end_to_end` in
`app/src-tauri/src/net_cmd.rs`, `chat_flow_end_to_end` in
`app/src-tauri/src/chat_cmd/tests.rs`), the storage screen flows
(`app/src-tauri/src/storage_cmd/tests.rs`: `wrong_key_surfaces_error_and_keeps_file`,
`retry_recovers_after_key_restored`), `history_clear_end_to_end` and
`remove_with_erase_clears_chat_history_end_to_end`, and component level
(vitest). Check by
hand with `cd app && npm run tauri dev` on a fresh profile and empty keychain:

1. First launch shows the wizard; empty or whitespace names are rejected.
2. A valid name lands on the main screen; relaunch skips the wizard.
3. My Info shows name, full NodeId, ticket and QR; both copy buttons paste
   exactly the ticket / NodeId.
4. Denying keychain access shows the blocking error screen, never the wizard.
5. My Info shows the long live ticket (not just `p2pm:<nodeid>`); it updates
   shortly after launch once the relay connects.
6. Paste it on a second device (Network tab): the receiver gets an
   Accept/Decline prompt marked Unverified, on whichever tab is open. It shows
   a prefilled, editable "Your name for them" field and what the peer calls
   itself; an empty field disables Accept.
7. Accept (with the name you chose): both devices show Connected. Decline: the
   dialer shows Declined and the receiver keeps no entry.
8. Relaunch and reconnect from the same device: a known peer connects without
   a prompt.
   After upgrading a profile that has a `known_peers.json`, a previously
   accepted peer still connects without a prompt.
9. Two instances (see Two-device simulation): each appears in the other's
   Contacts → On this network list as Unverified, with its full NodeId. Add…
   it: it becomes a contact, connects, and the other side gets the prompt.
10. Turn Visible on this network off: the peer disappears from the other
    list within seconds, the other still shows in yours, and a ticket connect
    to you still works. Turn it on: you reappear with your current name.
11. Rename yourself while visible: the other instance shows the new name.
12. Contacts is the default tab; the empty state shows with no contacts.
13. The Accept prompt's nickname is prefilled with their name (or "Peer <id>") and is editable; clearing it disables Accept. After Accept, the row shows only the bold nickname, the gray truncated
    NodeId and Unverified; the peer's own name is not in the row. Hovering shows
    the full NodeId; Show full NodeId reveals selectable text and Ctrl/Cmd+C
    copies it.
14. Ticket connect: once the other side accepts, the naming prompt appears
    prefilled with their name. Not now leaves them in Contacts as Unverified
    under that name.
15. Rename changes only the nickname. Mark verified shows the honest dialog and
    the badge reads "Verified by you"; Mark unverified reverts it.
16. Remove: the checkbox is unchecked by default and both outcomes are shown.
    After removal the connection drops and the peer's next connect prompts.
17. The peer renames itself and reconnects: your nickname for them is unchanged.
18. Reconnect silently: quit and relaunch both instances. On B, open Contacts;
    A's row shows Connect. Click it: A shows no Accept prompt and both show
    Connected under Network → Connections. Repeat by pasting A's ticket into B's
    Network tab, again with no prompt on A.
19. `MESSENGER_PROFILE=../x` or `MESSENGER_RELAY_URL=bad` refuses to start.
20. Contacts -> **Message** opens the chat. **Back**, or any tab, returns to the tabs.
21. With both instances connected, multi-line + emoji text arrives once. The
    sender shows Sending -> Delivered, and times are local.
22. Quit B and send from A: the message is **Pending**, and the not-connected
    note explains the reconnect. Relaunch B and Connect: it becomes Delivered,
    and B shows it once.
23. An unread count appears on B's Message button while B's chat is closed, and
    clears when the chat opens.
24. Send `<b>x</b>`, `**x**`, `https://example.com`, a text containing U+202E,
    and a Persian word containing ZWNJ. All appear literally and nothing is
    clickable. The U+202E shows as �, and the Persian word renders correctly.
25. Restart A: the history reloads with its persisted statuses, the newest is at
    the bottom, and **Load older** pages back.
26. Search in the chat finds a word, and only from this conversation.
27. Remove the contact with "Also delete our message history", then re-add it:
    the chat is empty. Without the option, the history remains. Queued Pending
    messages for a removed contact show Not delivered.
    Clear history… while connected empties your chat and search, and the peer's
    history is unchanged. Clear history… of a removed contact works too, and
    search then finds nothing. `grep -c <cleared word> <profile data dir>/messenger.db`
    returns 0.
28. Empty or whitespace input keeps Send disabled. A paste over 16 KiB disables
    Send and shows the counter.
29. `grep -c "<a word you sent>" <profile data dir>/messenger.db` returns 0 (the
    file is encrypted).
30. Quit, then `head -c 8192 /dev/urandom > <profile data dir>/messenger.db`
    (after copying the real file aside) and launch: **Encrypted storage
    unavailable** shows, the file is untouched, and after restoring the backup
    copy **Retry** returns to the main screen.
31. Quit and move `messenger.db` away from a profile that has an identity, then
    launch: the same screen shows with the "missing" reason and no new
    `messenger.db` is created. Move it back and **Retry**.
32. With two instances connected, each profile's log folder contains
    `messenger.log` (`0600` inside a `0700` folder) with `connection state` and
    `path opened` lines. No name, nickname, message text or ticket appears in it.
33. `MESSENGER_LOG=bogus npm start` refuses to start with a clear message.
34. Upgrade a profile created by the previous version: launch it. Settings
    (display name, visibility, relay) and contacts are kept, `settings.json`
    and `contacts.json` are gone from the data dir, and no new prompts appear
    for accepted peers. A `known_peers.json` that could not be parsed may
    remain; it is plaintext and can be deleted.

## Two-device simulation

**Level 1** (in-process, runs in the pre-commit gate): `cargo test --workspace`
drives two real iroh nodes over loopback, with an in-process LAN bus standing in
for multicast. The real mDNS test is opt-in because it needs a
multicast-capable interface:

```sh
cargo test -p core --test discovery_lan -- --ignored
```

**Level 2** (two full app instances on one machine): `--profile <name>` or
`MESSENGER_PROFILE=<name>` (`[a-z0-9-]`, 1-32 chars, not `default`; the command
line wins) gives an instance its own data directory (`<app data>/profiles/<name>`)
and keychain entries, and adds "profile <name>" to the window title. An invalid
name stops the app from starting; it never falls back to the default identity.
The default (no profile) data and keychain are unchanged.

`tauri dev` twice would clash on the Vite port 1420, so run one dev instance and
start the second from the already built debug binary, which loads the same dev
server:

```sh
MESSENGER_PROFILE=a npm start                      # terminal 1: dev server + instance A
cargo build -p app                                 # once, if target/debug/app is missing
MESSENGER_PROFILE=b ./target/debug/app             # terminal 2: instance B
```

Then: complete first-run on each, find the other in **Contacts → On this network**
(or paste its ticket), Accept on the other side, see Connected, then turn
**Visible on this network** off on one: it vanishes from the other's list but
can still be connected to by ticket.

Same-host mDNS discovery needs a multicast-capable, non-loopback interface
(normal Wi-Fi or Ethernet). On a machine with only loopback, or with multicast
blocked by a VPN or firewall, the LAN list may stay empty; use the ticket path.

The profile override only relocates state. It never disables a prompt, trust
check or any other control.

### Level 3: containerized headless nodes (Docker)

`headless/` builds `messenger-headless`, a test-only node that runs the same
`core::discovery` stack with no webview, driven over a private Unix socket
(`messenger-headless ctl`). `docker/` runs two of them on one Docker network.
Prerequisites: Docker with Compose v2.

```sh
docker compose -f docker/compose.yaml build   # build the image
docker/smoke.sh                               # one command: up, LAN, ticket, reconnect, presence, isolation, teardown
```

Drive the nodes by hand:

```sh
docker compose -f docker/compose.yaml up -d
C="docker compose -f docker/compose.yaml exec -T"
$C node-a messenger-headless ctl --data-dir /data node-id
$C node-a messenger-headless ctl --data-dir /data ticket          # prints {"ok":true,"ticket":"..."}
$C node-b messenger-headless ctl --data-dir /data connect <TICKET>
$C node-a messenger-headless ctl --data-dir /data wait incoming_request --timeout-secs 30
$C node-a messenger-headless ctl --data-dir /data respond <NODE_B_ID> accept [NICKNAME]
$C node-a messenger-headless ctl --data-dir /data wait connection_state --state connected
$C node-a messenger-headless ctl --data-dir /data presence off
$C node-b messenger-headless ctl --data-dir /data peers
docker compose -f docker/compose.yaml down -v
```

`wait` consumes the event it returns: each event is delivered to at most one
`wait` call (a ring of the last 256 events is kept), and a timeout exits
non-zero. Teardown: `down -v` removes the per-node volumes; without `-v`,
identities persist until the next `down -v`. For more nodes, copy a service in
`docker/compose.yaml` together with its own volume; do not use `--scale`, since
replicas would share one volume.

Scope:

- The containers exercise mDNS discovery and the ticket path over direct
  addresses. They run `--relay disabled`, so the
  **relay** path is not tested by Docker; it is covered by the Level-1
  `core/tests/discovery_relay.rs`.
- The Docker bridge is not `internal: true`: mDNS needs the default route, so
  egress is open. Headless nodes always run with n0 internet lookup disabled
  (no n0 DNS/pkarr publishing or resolution); `--relay` is honored as given.
  The desktop app keeps internet lookup on.
- Parity: the same trust-gate and presence scenarios run against in-process
  nodes (`core/tests/discovery_parity.rs`) and against real headless processes
  (`headless/tests/parity.rs`). Host processes use `--lan off` (loopback
  multicast is unreliable), so the presence scenario proves presence
  **persistence and dialability** at both levels. Presence **visibility** is
  checked by `docker/smoke.sh` stage 5 (and at Level 1 by `discovery_lan.rs`).
- Measured on one run, with presence OFF on node A, node B's peer list dropped A
  after about 3 s (smoke.sh polls for up to 180 s; the goodbye is processed quickly, a silent peer would take its mDNS TTL).

Warnings: this is **test-only** with **throwaway identities**; never point a
real identity at a container. The file keyring stores secrets **unencrypted** in
the node's volume. The headless node never uses the OS keychain (`--keyring os`
is rejected). The control surface has no stability or security guarantees and is
not a product feature; it is not exposed in the GUI.

## License

[GPL-3.0-only](LICENSE).

[iroh]: https://www.iroh.computer/
