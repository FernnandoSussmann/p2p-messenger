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

Requires Rust 1.95 or newer (the workspace MSRV), Node.js 20+, and the [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/).
A C compiler, `perl` and `make` are also required: SQLCipher and a vendored
OpenSSL are compiled from source (on Windows, install Strawberry Perl). See
[Installing the prerequisites](#installing-the-prerequisites) for the
`apt`, `dnf`, `pacman`, Homebrew and `winget` commands. The first build takes a
few minutes.

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

### Screens and themes

The visual spec is the reference sheet [`design/visual-identity.html`](design/visual-identity.html)
(components in all three themes).

- **Themes:** Midnight (default), Dark and Light. You pick one on the one-screen
  setup (name and theme) and can change it any time on **My Info**. It is stored
  per account in `messenger.db`, so each `--profile` keeps its own.
- **Screen structure:** setup lands on **Contacts**. The menu has four tabs:
  **Contacts**, **Discover**, **Network** and **My Info**. Drill-in screens hide the
  menu: Contacts -> a contact -> conversation -> the ⋮ menu's **Contact info**.
  **Back** goes from contact info to the conversation and from the conversation to
  Contacts; any menu tab clears the drill-in.
- **Where each per-contact action lives:**

  | Action | Where |
  | --- | --- |
  | Open the conversation | click the contact row |
  | Unread count | the "N unread" chip on the row |
  | Rename (pencil), Remove (trash) | contact info |
  | Verify / unverify | contact info |
  | Connect (LAN contacts), connection state | contact info (state also on the row and in the conversation header) |
  | Search, Clear history | contact info |
  | Add a contact from the LAN | Discover (nickname, then **Add contact and connect**) |

### Contacts

**Contacts** is the default tab. Each row is one button that opens the
conversation. It shows the trust badge, your own nickname (bold), the connection
as `direction | state | reason` (or "Not connected"), and an "N unread" chip. It
has no action buttons. What a peer calls itself appears only in the Accept,
naming and rename prompts and in contact info, never in the list.

- **Unverified** vs **Verified by you**: the latter is your own attestation
  (**Mark verified…**, undo with **Mark unverified**, both in contact info). The app
  does not check it cryptographically.
- **Rename** (pencil in contact info) changes only your nickname. **Connect**
  appears in contact info for contacts currently on the local network; they
  reconnect without a prompt.
- Names may contain emoji sequences (👨‍👩‍👧, 🏳️‍🌈, 🏴󠁧󠁢󠁥󠁮󠁧󠁿) but not stray
  zero-width characters.
- **Remove** (trash in contact info) always removes trust and closes any open
  connection, so their next connect prompts again, then returns to Contacts with
  a notice. Your message history is deleted only if you tick
  "Also delete our message history" (off by default). Ticking it deletes the
  stored messages with that contact from the encrypted message database. If you
  keep it, the conversation is no longer shown anywhere; it comes back if you
  re-add the same contact (same NodeId).
- Copying a NodeId: in contact info select it and use your system's copy shortcut.
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
database it just created, so running the setup screen again works.

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

Open **Contacts**, then click a contact row. **Back** (or any tab) returns to the
tabs; an "N unread" chip shows on the row while the chat is closed. Outgoing
bubbles read "You"; incoming bubbles show *sent | received* times. The header
has Back, the name, the trust badge, the connection text and a ⋮ menu with
**Contact info**.
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
- Search (in contact info) finds words in that conversation's history. History and search
  read the encrypted store, so they work even if the node failed to start.
  Storage is `messenger.db` (see Encrypted storage).
- **Clear history…** (in contact info) deletes the
  whole conversation after a confirmation. The messages are removed from this
  device's encrypted database and the space they used is overwritten. This cannot
  remove the other person's copy, backups or snapshots of this computer, or
  traces some storage hardware (such as SSDs) may keep. Messages still waiting to
  be sent are marked Not delivered. Remove contact with "Also delete our message
  history" uses the same path. A removed contact's conversation is hidden (no
  row, chip or search result) and returns if the same contact is re-added.

### On this network, presence and relay

**Discover** lists non-contact peers found via mDNS. Entries
are always marked **Unverified**: the name is self-asserted, only the NodeId
counts. Type your nickname for the peer and press **Add contact and connect**; it saves the contact and connects; the
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

`cargo build` and `npm start` build only `protocol`, `core` and `app`. Build the
test-only headless node explicitly with `cargo build -p messenger-headless`
(the Docker image already does).

Debug builds trade debug info for build size: our crates keep line tables, so
panic backtraces show `file:line`; debuggers show locations but no local
variables. Dependency frames have names but no line numbers. Dependencies are
optimized (`opt-level = "s"`), so the first cold build is about 40 s slower and
later edit-rebuild is unchanged. For a local debugging session with full DWARF
run `CARGO_PROFILE_DEV_DEBUG=true cargo build` (or `npm start`); never commit it.

### Pre-commit hook (the test gate)

There is no CI, so the tracked hook in `.githooks/pre-commit` is the mandatory
local gate. Install it once per clone:

```sh
git config core.hooksPath .githooks
```

It blocks the commit on any failure of: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo test --workspace` (includes the secret-leak guards, the log redaction
guard `log_redaction_guard` and the SQL namespacing guard
`sql_namespacing_guard`), the secret grep
gates (no key types under `app/`; clipboard only in `MyInfo.svelte`), and the
frontend `npm run check`, `npm test` and `npm run build`.

The gate is self-cleaning:

- It builds in `target/gate`, never `target/debug`, with incremental
  compilation off.
- One run at a time per clone (a lock; `flock`, or an atomic `target/gate.lockdir`
  where `flock` is missing). A symlinked `target/gate` is refused.
- A toolchain (`rustc -vV`) or `Cargo.lock` change wipes the gate dir, so the
  next run is cold.
- After every run (pass, fail or Ctrl-C), `.githooks/gate-cleanup.sh` deletes
  only the outputs of our own crates (`cargo clean -p` for `protocol`, `core`,
  `messenger-headless` and `app`); third-party libraries stay cached.
- If `target/gate` still exceeds a size cap it is wiped entirely. The cap is
  `P2PM_GATE_MAX_MIB` (MiB, positive integer, default `8192`; anything else
  falls back to the default).
- On Linux the gate also fails if the `app` binary exports `sqlite3_*` symbols.
- A cleanup failure after a passing gate only warns; the next run retries.
- Never bypass the hook with `--no-verify`.

Verifying the gate by hand (after changing the hook), one at a time:

1. A passing commit leaves no workspace artifacts: `find target/gate/debug
   -maxdepth 2 -type f \( -name 'app' -o -name 'app-*' -o -name 'integration-*'
   -o -name 'core-*' -o -name 'protocol-*' -o -name 'libcore-*' -o -name
   'libprotocol-*' -o -name 'libapp_lib-*' -o -name 'logging_canary-*' -o -name
   'profile_isolation-*' \) | wc -l` prints `0`, `libserde-*` is still in
   `target/gate/debug/deps`, and `target/debug` is untouched.
2. Stage a file under `app/` containing a planted secret-looking identifier (e.g.
   a Tauri return type referencing the key type name). `git commit` must be
   rejected and the `find` above must still print `0`. Unstage and delete it.
3. `touch core/src/lib.rs`, start a commit and press Ctrl-C during `cargo test`:
   cleanup runs, `flock -n target/gate.lock true && echo released` prints
   `released`, and the `find` prints `0`.
4. `echo stale > target/gate/.gate-stamp`: the next run wipes the gate dir and
   builds cold.
5. Delete the `cargo::rustc-link-arg` line in `app/src-tauri/build.rs`, stage it
   and commit: the exported-symbol check must reject it listing `sqlite3_*`, and
   cleanup still runs. Restore it with `git checkout -- app/src-tauri/build.rs`.

Never commit these temporary edits.

### Reclaiming disk space

- `cargo clean` removes everything, including `target/gate`.
- Optional, never required: `cargo sweep --time 7` or `cargo sweep --installed`.
- Manual `cargo test` writes to `target/debug` and is not auto-cleaned.
- Folders: `target/debug/` holds the dev app and manual runs, `target/gate/` is
  the hook's dependency cache, and `release/` holds end-user binaries.

### Building the end-user binary

```sh
npm run release
```

Prerequisites: Rust 1.95+, Node.js 20+, and per OS: on Linux the WebKitGTK 4.1,
GTK 3 and libsoup 3 dev packages plus `pkg-config` and binutils; on macOS the
Xcode command line tools; on Windows the MSVC Build Tools. SQLCipher and
OpenSSL are compiled from source, so every OS also needs a C compiler, `perl`
and `make`. The script's preflight checks these and names any that are missing.

#### Installing the prerequisites

**Rust** (all OSes): install with [rustup](https://rustup.rs). Distribution
`rustc` packages are usually older than 1.95.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Linux / macOS
rustup update stable
```

**Node.js 20+**: distribution packages are often older (Ubuntu 22.04 ships 12,
Debian 12 ships 18). Check with `node --version`. If it is too old, use the
installer from [nodejs.org](https://nodejs.org) or a version manager such as
`nvm` or `fnm`.

**Debian / Ubuntu / Linux Mint (`apt`):**

```sh
sudo apt update
sudo apt install build-essential perl pkg-config binutils curl \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libdbus-1-dev
```

**Fedora / RHEL family (`dnf`; `yum` is an alias for it on current RHEL):**

```sh
sudo dnf install gcc gcc-c++ make perl pkgconf-pkg-config binutils curl \
  webkit2gtk4.1-devel gtk3-devel libsoup3-devel dbus-devel
```

Install the full `perl` package, not just `perl-interpreter`: the OpenSSL build
needs its standard modules. WebKitGTK 4.1 is not packaged on older releases
such as RHEL 8.

**Arch Linux (`pacman`):**

```sh
sudo pacman -S --needed base-devel perl curl webkit2gtk-4.1 gtk3 libsoup3 dbus
```

**macOS (Homebrew):**

```sh
xcode-select --install   # C compiler, make, perl, otool
brew install node
```

**Windows (`winget`, in PowerShell):**

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Rustlang.Rustup
winget install OpenJS.NodeJS.LTS
winget install StrawberryPerl.StrawberryPerl
```

The WebView2 runtime is preinstalled on Windows 10/11. Open a new terminal
after installing so the new `PATH` takes effect.

The script runs `npm ci` and `tauri build --no-bundle --ci -- --locked` in a
throwaway temp target dir (with build paths remapped), then checks the binary:
no test doubles, no builder paths, no exported `sqlite3_*` (Linux), stripped, no
devtools, frontend embedded. Any failed check publishes nothing. The Windows
builder-path check is implemented but not yet verified on Windows.

Output: `release/p2p-messenger-<ver>-<os>-<arch>[.exe]` plus a `.sha256` file.
Verify it with `sha256sum -c <name>.sha256` (Linux), `shasum -a 256 -c
<name>.sha256` (macOS) or `Get-FileHash` (Windows). Builds are not claimed to be
reproducible. The script never deletes or overwrites anything in `release/` (an
existing same-name binary is refused); it only removes its own partial files.
Interrupting it leaves no temp dir and no partial file.

### Running the release binary

- Linux: WebKitGTK 4.1, GTK 3, libsoup 3, D-Bus and a Secret Service keychain.
- Windows: the WebView2 runtime (preinstalled on Windows 10/11).
- macOS: nothing extra; a bare executable runs (an `.app` bundle is deferred).
- `--profile`, `MESSENGER_PROFILE` and `MESSENGER_RELAY_URL` work at runtime,
  exactly as in dev.

### Manual GUI checklist

No Tauri WebDriver driver is available in this environment, so the first-run
flow is automated at command-invoke level (`first_run_flow_end_to_end` in
`app/src-tauri/src/identity_cmd.rs`, `ticket_connect_accept_flow_end_to_end` in
`app/src-tauri/src/net_cmd.rs`, `chat_flow_end_to_end` in
`app/src-tauri/src/chat_cmd/tests.rs`), the storage screen flows
(`app/src-tauri/src/storage_cmd/tests.rs`: `wrong_key_surfaces_error_and_keeps_file`,
`retry_recovers_after_key_restored`), `history_clear_end_to_end` and
`remove_with_erase_clears_chat_history_end_to_end`,
`remove_keep_then_readd_restores_history_end_to_end`, and component level
(vitest). Check by
hand with `cd app && npm run tauri dev` on a fresh profile and empty keychain:

1. First launch shows the one-screen setup (name and theme); empty or whitespace names are rejected.
2. A valid name lands on the main screen; relaunch skips the setup screen.
3. My Info shows name, full NodeId, ticket and QR; both copy buttons paste
   exactly the ticket / NodeId. The theme (Midnight, Dark, Light) is chosen
   there too, stored per account in `messenger.db`, and survives a relaunch.
4. Denying keychain access shows the blocking error screen, never the setup screen.
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
   Discover tab as Unverified, with its full NodeId. Type a nickname and press
   Add contact and connect: it becomes a contact, connects, and the other side gets the prompt.
10. Turn Visible on this network off: the peer disappears from the other
    list within seconds, the other still shows in yours, and a ticket connect
    to you still works. Turn it on: you reappear with your current name.
11. Rename yourself while visible: the other instance shows the new name.
12. Contacts is the default tab; the empty state shows with no contacts.
13. The Accept prompt's nickname is prefilled with their name (or "Peer <id>") and is editable; clearing it disables Accept. After Accept, the row shows the Unverified badge and the bold nickname;
    the peer's own name is not in the row. Contact info
    shows the full NodeId as selectable text, and Ctrl/Cmd+C copies it.
14. Ticket connect: once the other side accepts, the naming prompt appears
    prefilled with their name. Not now leaves them in Contacts as Unverified
    under that name.
15. In contact info, rename (pencil) changes only the nickname. Mark verified shows the honest dialog and
    the badge reads "Verified by you"; Mark unverified reverts it.
16. Remove (trash in contact info): the checkbox is unchecked by default and both outcomes are shown.
    After removal the connection drops and the peer's next connect prompts.
17. The peer renames itself and reconnects: your nickname for them is unchanged.
18. Reconnect silently: quit and relaunch both instances. On B, open Contacts;
    open A's contact info and click Connect: A shows no Accept prompt and both show
    Connected on A's contact row. Repeat by pasting A's ticket into B's
    Network tab, again with no prompt on A.
19. `MESSENGER_PROFILE=../x` or `MESSENGER_RELAY_URL=bad` refuses to start.
20. Contacts -> clicking the contact row opens the chat. **Back**, or any tab, returns to the tabs.
21. With both instances connected, multi-line + emoji text arrives once. The
    sender shows Sending -> Delivered, and times are local.
22. Quit B and send from A: the message is **Pending**, and the not-connected
    note explains the reconnect. Relaunch B and Connect: it becomes Delivered,
    and B shows it once.
23. An "N unread" chip appears on B's contact row while B's chat is closed, and
    clears when the chat opens.
24. Send `<b>x</b>`, `**x**`, `https://example.com`, a text containing U+202E,
    and a Persian word containing ZWNJ. All appear literally and nothing is
    clickable. The U+202E shows as �, and the Persian word renders correctly.
25. Restart A: the history reloads with its persisted statuses, the newest is at
    the bottom, and **Load older** pages back.
26. Search in contact info finds a word, and only from this conversation.
27. Remove a contact keeping history: their conversation is no longer shown;
    re-add the same NodeId: the old messages are back. With "Also delete our
    message history" the re-added chat is empty. Queued Pending messages for a
    removed contact show Not delivered.
    Clear history… in contact info while connected empties your chat and search,
    and the peer's history is unchanged; after **Back** the conversation is empty.
    `grep -c <cleared word> <profile data dir>/messenger.db` returns 0.
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
35. Fresh profile: setup is Midnight; the arrow appears only after typing; picking
    Light repaints instantly and Continue lands on Contacts in Light. Relaunch opens
    Contacts in Light; a second `--profile` keeps its own theme.
36. Switch all three themes on My Info: every tab, dialog, badge and bubble is readable.
37. Keyboard only: menu arrows, picker arrows, row Enter, ⋮, Back; the focus ring is
    visible in every theme.
38. Two instances via Discover: add and connect; the row shows `outbound | connected`;
    the conversation shows "You" and *sent | received*; ⋮ opens contact info.
39. Stop the peer: the row shows the state and reason, and the conversation shows
    the edited not-connected sentence.
40. A pending incoming request stays visible on every tab, in the chat and in
    contact info.

## Two-device simulation

**Level 1** (in-process, runs in the pre-commit gate): `cargo test --workspace`
drives two real iroh nodes over loopback, with an in-process LAN bus standing in
for multicast. The real mDNS test is opt-in because it needs a
multicast-capable interface:

```sh
cargo test -p core --test integration discovery_lan -- --ignored
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

Then: complete first-run on each, find the other in **Discover**
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
  `core/tests/integration/discovery_relay.rs`.
- The Docker bridge is not `internal: true`: mDNS needs the default route, so
  egress is open. Headless nodes always run with n0 internet lookup disabled
  (no n0 DNS/pkarr publishing or resolution); `--relay` is honored as given.
  The desktop app keeps internet lookup on.
- Parity: the same trust-gate and presence scenarios run against in-process
  nodes (`core/tests/integration/discovery_parity.rs`) and against real headless processes
  (`headless/tests/integration/parity.rs`). Host processes use `--lan off` (loopback
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
