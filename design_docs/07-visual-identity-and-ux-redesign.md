# DD-07 — Visual Identity & UX Redesign

**Feature slice:** `core::identity` (theme preference in account settings) + `app` (one shared stylesheet,
themes, setup screen, app shell and navigation, Discover tab, contact list, conversation, new contact info screen)
**Status:** Draft
**Affected:** `core/src/identity/` (settings trait, SQLCipher store, memory store, service),
`core/src/storage/schema.rs` (one new migration), `app/src-tauri/src/identity_cmd.rs`, `app/src-tauri/src/lib.rs`,
`app/index.html`, `app/src/main.ts`, a new global stylesheet, `app/src/lib/router.ts`, `app/src/lib/chatStore.ts`,
a new `lib/theme.ts`, every `app/src/views/*.svelte` (restyled, several re-laid out), new `ThemePicker.svelte` and
`ContactInfo.svelte`, their `*.test.ts`, the frontend guard tests, `README.md`, and the removal of `design/wireframes/`
**Wire format:** unchanged — no `PROTOCOL_VERSION` bump. The theme never leaves the device.
**Reference:** [`design/visual-identity.html`](../design/visual-identity.html) (approved component sheet, three themes; kept)
Every layout decision they carried is written down below, so this doc stands on its own once they are gone.
**Followed by:** [DD-08 Outgoing Delivery Time](08-outgoing-delivery-time.md) (adds the delivered time to the
outgoing bubbles designed here)
**Related:** [DD-01 Identity & My Info](01-identity-onboarding-and-my-info.md),
[DD-03 Contacts & Trust](03-contacts-and-trust.md), [DD-04 Messaging & Delivery](04-messaging-and-delivery.md),
[DD-05 Encrypted Storage](05-encrypted-storage-and-logging.md)

---

## TL;DR

The app has almost no styling (browser defaults plus a few hardcoded colours) and its screens grew
feature by feature. This doc gives it one visual system and a navigation built around the three user
paths:

- **First run:** setup screen → Contacts.
- **Normal launch:** opens on Contacts; a top menu reaches **Contacts | Discover | Network | My Info**.
- **Contact info:** Contacts → click a contact → conversation → **⋮** → contact info (rename with the
  pencil, remove with the trash, and every other per-contact action).

What changes:

1. **One stylesheet for the whole app.** All tokens, base styles and component classes live in a
   single global stylesheet ported from `design/visual-identity.html`. Svelte components carry **no
   `<style>` blocks and no inline `style` attributes**; they only use the shared classes. A guard
   test enforces it.
2. **Three themes, Midnight by default:** Midnight (`#888888` on `#101525`), Dark (`#B988EE` on
   `#000000`), Light (`#5A00BD` on `#FFFFFF`). The theme is a per-account setting in the encrypted
   `messenger.db` (new nullable `theme` column; `NULL` = Midnight), read and written through two
   new commands, `identity_theme_get` / `identity_theme_set`, taking a closed enum.
3. **Setup is one screen:** the existing "Welcome" text, a theme picker, the display-name field and
   an arrow button that appears once a name is typed. It provisions, names, saves the theme and
   lands on Contacts.
4. **A four-tab shell** with an app-name footer, plus full-screen "drill-in" screens (conversation,
   contact info) that have Back. Trust prompts stay visible on every screen.
5. **Discover** is a new tab hosting the existing "On this network" list with the nickname field
   and Add inline.
6. **Contact rows** read `badge nickname | direction | state | reason` (or "Not connected") plus an
   unread count, and open the conversation on click. The Network tab loses its Connections list.
7. **Conversation:** header with Back, name, badge, connection text and ⋮; "You" on your bubbles;
   *sent | received* times on incoming bubbles; slim composer with an arrow send button.
8. **New contact info screen** holds rename (pencil), remove (trash), NodeId, the self-chosen name,
   verify/unverify, Connect, Clear history and search.
9. **Existing text is kept verbatim.** Two sentences get a minimal edit because they point to
   places that move (§Text inventory).
10. **The wireframe images are deleted** when this doc is done.

## Context and scope

- The frontend is Svelte 5 + Vite in `app/src/`. `App.svelte` sets only `font-family: sans-serif;
  margin: 2rem`. Eight components have their own `<style>` blocks, and a few hardcode colours
  (`#e3f0ff` bubbles, `#0b6b2e` badges, `gray` NodeIds).
- `App.svelte` routes `loading | blocked | storage | wizard | main` through the `appView` store in
  `lib/router.ts`. `MainScreen` renders a `role="tablist"` of **Contacts, My Info, Network**
  (`activeTab`) and swaps the panel for `Chat` when `activeChat` (in `chatStore`) is set.
  `IncomingRequestPrompt` and `NamingPrompt` render above the tabs.
- `FirstRunWizard` has three steps: generate ("Welcome", "Create your identity on this device.",
  "Create identity"), name ("Choose a display name", "Display name", "Continue"), and land ("All
  set"). It calls `identity_provision`, then `identity_set_display_name`; `AlreadyProvisioned`
  short-circuits to `onDone`.
- `Contacts` renders `ContactRow`s and then `LanPeers` ("On this network"). Each `ContactRow` shows
  badge, nickname and `NodeIdLine` plus the buttons Message / Message (N), Connect, Rename, Mark
  verified… / Mark unverified and Remove…, opening `NicknameForm`, `VerifyContactDialog` and
  `RemoveContactDialog` inline. A code comment reserves `self_name` for "the future contact-info
  screen".
- `Network` renders `PresenceToggle`, the ticket form and a "Connections" list (`node_id`,
  `direction`, `state`, `reason`, `display_name`, with "Add contact…" for connected non-contacts).
- `Chat` shows Back, name, badge, "Connected"/"Not connected", "Clear history…", the not-connected
  explanation, a search box, "Load older messages", the log and the `Composer`. `MessageBubble`
  shows body, the `sent_at_ms` time, and for outgoing messages a status label and Resend.
- `MessageDto` carries `sent_at_ms` (sender's clock, advisory per DD-04) and `received_at_ms`
  (local receipt time for incoming, `null` for outgoing).
- Settings live in `account_settings` (DD-05) behind the `SettingsStore` trait, with
  `SqlCipherSettingsStore` and `MemorySettingsStore`.
- CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:`.
  `removeUnusedCommands: true`, so new commands must be registered and actually invoked.
- The Linux WebView is WebKitGTK, which can lag on newer CSS. The reference sheet uses
  `color-mix()` and `:has()`; the app must not.
- The wireframes are phone-shaped. Desktop is the first target, mobile comes later.

In scope: everything in the TL;DR. Out of scope: a delivered time for outgoing messages (DD-08).

## Goals

- First-run path **setup → Contacts**; normal path **launch → Contacts**; every tab one click away;
  contact info two clicks from the list.
- One source of styling truth: the app stylesheet, kept equal to `design/visual-identity.html`
  token values. Every screen looks like it belongs to the same app.
- Midnight on a fresh install, the loading screen, error screens and setup before any choice. The
  user can switch theme on setup and on My Info; the choice survives restarts and is per account
  (so per `--profile`).
- Every text colour meets **WCAG AA (≥ 4.5:1)** on the surfaces it is drawn on, in all three
  themes, enforced by a test.
- Trust prompts stay reachable on **every** screen. Trust state is visible wherever the contact is
  shown and never conveyed by colour alone.
- Message timing is honest: the sender's claimed time and the local receipt time are shown side by
  side, each labelled.
- Nothing that exists today becomes unreachable: every action, status and error text keeps a home
  (one deliberate exception: a removed contact's conversation, and so its "Clear history…", is
  hidden until the contact is re-added).
- Keyboard and screen-reader navigation work: real tablist, focus moved on screen change, Back
  reachable.
- A single-column layout that fits 900×640 and reflows down to phone widths.

## Non-goals

- **Following the OS light/dark preference.** The default is fixed to Midnight. A "System" option
  can be added later as a fourth enum value through a new migration.
- **User-defined colours, font sizes or accents; web fonts or icon fonts.** System font stack and
  inline SVG icons only.
- **Syncing the theme to other devices or peers.**
- **URL/hash routing or browser history.** One window, no deep links; an in-memory screen store is
  enough. OS back-gesture mapping is a mobile-phase concern.
- **Unread badges on the menu, avatars, presence dots, last-message previews or list re-sorting.**
- **Jumping from a search result to the message in the conversation.**
- **Changing trust, removal, history-erasure, provisioning, naming or presence semantics in
  `core`.** The only Rust change is the theme setting.
- **Copy-to-clipboard for a contact's NodeId.** Selection only, per the clipboard rule.
- **A delivered time on outgoing bubbles.** Covered by DD-08, which builds on the bubble layout
  defined here.

## The actual design

### Overview

```
<html data-theme="midnight">   ← static default in app/index.html, first paint already Midnight
App.svelte refresh()
  loading / blocked / storage ────────────────► Midnight
  wizard (setup screen) ──────────────────────► Midnight or the in-memory pick
  main: identity_theme_get ─► applyTheme ─► Shell
        Shell = menu [Contacts|Discover|Network|My Info] + global prompts + screen + footer
        screen: tab(contacts|discover|network|my-info) │ chat(nodeId) │ contact-info(nodeId)
```

### Part A — One stylesheet, tokens and themes

#### The single stylesheet

- One global stylesheet, imported once from `main.ts`, is the **only** place where CSS is written
  in the app. It contains, in order: theme token blocks, shared non-colour tokens, base element
  styles, layout primitives, and component classes.
- **Theme blocks** for `[data-theme="midnight"|"dark"|"light"]` with exactly the token names of the
  reference sheet: `--bg`, `--surface`, `--surface-2`, `--border`, `--border-strong`, `--text`,
  `--text-strong`, `--text-muted`, `--accent`, `--accent-hover`, `--accent-contrast`, `--success`,
  `--warning`, `--danger`, `--bubble-in`, `--bubble-in-text`, `--bubble-out`, `--bubble-out-text`,
  `--shadow`. Values are copied from `design/visual-identity.html`. `color-scheme` is set per theme
  so native controls match.
- **Soft tints** (`--accent-soft`, `--success-soft`, `--warning-soft`, `--danger-soft`, `--focus`)
  are **precomputed `rgba()` values per theme** instead of `color-mix()`: a few extra lines in
  exchange for compatibility with older WebKitGTK and WebView2.
- **Shared tokens:** radii, spacing scale, system and mono font stacks, easing.
- **Component classes** ported from the reference: `.btn` (+ `-primary`, `-secondary`, `-ghost`,
  `-danger`, `-sm`, `-icon`), `.input`, `.textarea`, `.select`, `.check`, `.radio`, `.switch`,
  `.tabs`, `.badge`, `.chip`, `.alert-*`, `.panel`, `.card`, `.list`, `.divider`, `.dialog`,
  `.bubble`, `.node-id`, `.spinner`, `.visually-hidden`, plus the few app-only layout classes the
  screens below need (shell, header bar, footer, row, swatch, QR frame). States that used `:has()`
  in the reference are expressed by a class or attribute the component sets on the parent.
- `prefers-reduced-motion` disables transitions and animations globally.

**Why no component styles:** the user asked for the same CSS everywhere. Svelte's scoped `<style>`
blocks make it easy for each component to drift a little (spacing, radii, colours). With one
stylesheet, a component can only look like the rest of the app, a redesign is one file, and the
guard tests can check every colour in one place. **Trade-off:** class names are global, so they
must be named carefully, and a component that needs something new adds a class to the shared file
rather than styling itself. Dynamic values that genuinely vary at runtime (the composer's
auto-grow height) are set from script on the element's style property, not written as CSS in a
component; that is the only allowed exception and it never sets a colour.

**The reference sheet** stays in `design/visual-identity.html` as the visual spec. The drift guard
keeps its token values and the app's equal, so changing a colour means changing both in the same
commit.

#### Theme state (frontend)

`lib/theme.ts` owns:

- `THEMES`: the readonly allowlist `["midnight", "dark", "light"]` and the type `Theme`.
- `applyTheme(t)`: sets `document.documentElement.dataset.theme` only if `t` is in the allowlist,
  otherwise Midnight. The only code that writes the attribute.
- `loadTheme()`: calls `identity_theme_get` and applies it; on any error applies Midnight without
  blocking startup.
- `chooseTheme(t)`: applies immediately (optimistic), then calls `identity_theme_set`; on failure
  re-applies the previous theme and returns the error for the caller to show.
- `pendingTheme`: an in-memory value used by the setup screen before an account exists.

`App.svelte` calls `loadTheme()` after status is Present and storage is OK, before showing the
shell. The loading screen stays up for that one call, so the shell never paints in the wrong theme.

**Trade-off:** a user whose saved theme is Dark or Light sees the Midnight loading screen for a
moment at launch. Caching the choice outside the encrypted DB (e.g. `localStorage`) would avoid it
but break the rule that all persisted user data lives in `messenger.db`. Accepted.

#### Theme picker

`ThemePicker.svelte` is a `role="radiogroup"` (accessible name "Theme") of three round swatch
buttons ("Midnight", "Dark", "Light"), each showing its theme's background with its text colour as
a ring. `aria-checked` marks the current one, arrow keys move between options, focus ring visible.
It takes the current theme and an `onChoose` callback and knows nothing about persistence, so the
setup screen and My Info both use it.

#### Core and storage

- **Migration (forward-only):** add a nullable `theme` column to `account_settings` with a `CHECK`
  that it is `NULL` or `'midnight'`, `'dark'`, `'light'`. Existing rows read `NULL` = Midnight. No
  legacy-import change (the old `settings.json` never had a theme).
- **`Theme` enum** in `core::identity` (`Midnight`, `Dark`, `Light`) with a fallible parse from the
  lowercase string; the only representation that reaches SQL, always as a bound parameter.
- **`SettingsStore`** gains `get_theme` (`Option<Theme>`) and `set_theme`, implemented for the
  SQLCipher store, `MemorySettingsStore` and any other implementation. An unparseable stored value
  (impossible given the `CHECK`, but defended) reads as `None`.
- **`IdentityService`** gains `theme()` (resolves `None` → Midnight) and `set_theme(Theme)`, using
  the marker-resolved `account_id` like `set_display_name`; without an account they return the
  existing "not provisioned" error.
- The column sits on a table that already carries `account_id`, so the SQL-namespacing guard covers
  it with no allowlist change.

#### Tauri commands

| Command | Input | Output | Errors |
|---|---|---|---|
| `identity_theme_get` | — | `{ theme: "midnight" \| "dark" \| "light" }` | existing identity/storage errors (`NotProvisioned`, `StoreUnavailable`, …) |
| `identity_theme_set` | `{ theme }` (serde enum, lowercase) | `{ theme }` as saved | `InvalidTheme` when deserialisation fails, plus the existing errors |

Thin wrappers over `IdentityService` in `identity_cmd.rs`, registered in `generate_handler!` and
invoked by `lib/theme.ts`.

### Part B — Setup, shell and navigation

#### Screen model

`lib/router.ts` replaces `activeTab` + the `activeChat` takeover with one `screen` store:

| `screen.kind` | Shows | Menu + footer | Back goes to |
|---|---|---|---|
| `tab` (`contacts` \| `discover` \| `network` \| `my-info`) | tab panel | visible | — |
| `chat` (`nodeId`) | conversation | hidden | Contacts |
| `contact-info` (`nodeId`) | contact info | hidden | the conversation |

- Default `{ tab: "contacts" }`: the screen after first run and after every launch.
- A menu click always goes to that tab and clears drill-in state.
- `chatStore.openChat`/`closeChat` keep their conversation-loading duties and delegate navigation
  to the router, so there is one source of truth for what is on screen.
- On every screen change, focus moves to the new screen's main heading (or Back on drill-in
  screens).
- If the contact disappears while a drill-in screen is open (removed here or elsewhere), the
  screen closes to Contacts. A removed contact's conversation is **never displayed**: if the
  history was kept, it stays in `messenger.db` and shows again only when the same contact (same
  NodeId) is re-added. "Clear history…" is therefore not offered for removed contacts; erasing
  history on removal is the remove dialog's explicit option.
- **Trade-off, Back placement:** the wireframes drew the phone's system navigation bar ("Go back")
  at the bottom. On desktop, Back sits at the left of the drill-in header, reusing the existing
  "Back" label. On mobile, the system back will call the same router action.

#### App shell

```
┌ Contacts │ Discover │ Network │ My Info ┐   ← role="tablist", equal-width items, aria-selected,
├─────────────────────────────────────────┤      roving tabindex, Left/Right arrows
│ IncomingRequestPrompt / NamingPrompt    │   ← global, on every screen (tab or drill-in)
│ tab panel  ── or ──  drill-in screen    │   ← scrolls; menu pinned at the top on tab screens,
│                                         │      hidden on drill-in screens (Back instead)
├─────────────────────────────────────────┤
│              p2p-messenger              │   ← <footer>, muted small text, tab screens only
└─────────────────────────────────────────┘
```

The content column is centred with a max width. The prompts render as cards above the content so a
drill-in screen can never hide them. The tab page headings ("My Info", "Network", …) stay in the DOM
but visually hidden, because the selected menu item already shows them.

#### Setup screen (first run)

```
Welcome                                   (◯ ◯ ◯)   ← ThemePicker
Create your identity on this device.      Choose a theme
Display name [______________]
                     [ → ]   ← rendered only when the trimmed name is non-empty; accessible name "Continue"
(role="alert" error area)
```

- The picker starts on Midnight. Choosing applies immediately and sets `pendingTheme`; nothing is
  persisted (no account yet).
- The arrow is **not rendered** while the trimmed name is empty. Enter on an empty name shows the
  existing `displayNameMessages.Empty` error.
- Pressing the arrow (field and button disabled while busy):
  1. `identity_provision`, skipped if this screen already provisioned on an earlier attempt.
     `AlreadyProvisioned` → `onDone()` without writing the name, so another instance's name is
     never overwritten.
  2. `identity_set_display_name(name)`. A validation error shows the existing message and stays;
     the retry skips step 1.
  3. `identity_theme_set(pendingTheme)`, best effort: on failure the theme still applies this
     session and can be set again on My Info. A cosmetic preference never blocks entry.
  4. `onDone()` → `App` re-reads status, loads the theme, shows the shell on Contacts.
- If the app closes between steps 1 and 2, the next launch goes to Contacts and My Info is where to
  set the name (existing behaviour).
- Other provision errors show `describeError` in the alert area, as today.

#### Discover tab

The existing `LanPeers` component, moved out of Contacts:

- Heading "On this network", its description and the empty text "No one new on this network." are
  unchanged.
- Each person is one `.list` row: the Unverified badge, "calls themselves “<name>”" or "(no name)",
  and on the right the **Add** button with its existing label "Add contact and connect". Below: the
  nickname field ("Your name for them") with the `NicknameForm` context line, then `NodeIdLine`, so
  the real identity is always beside the self-chosen name.
- The extra "Add…" click is removed; the form is always inline and Add stays disabled until the
  nickname is non-empty. Add-then-connect, its error texts and the list refresh are unchanged.
- **Connections without a contact:** under the existing heading "Connections", rendered only when a
  live connection's NodeId is not a contact (rare: dialled and accepted peers are saved
  automatically). Each entry: Unverified badge, `NodeIdLine`, `direction | state | reason`, and the
  existing "Add contact…" button when connected, which opens the existing `NamingPrompt` flow.

**Trade-off:** a form on every row is denser than click-to-expand, but adding becomes one action,
and the LAN list is small in practice.

#### Network tab

Top to bottom:

1. The existing switch "Visible on this network" with small "Off" / "On" captions either side, then
   the existing explanation paragraph.
2. Divider.
3. Connect by ticket: the existing label and full-width multi-line field, the button right-aligned
   ("Connect and remember this device"), then the existing paragraph "Once they reply, they're saved
   as an unverified contact…", moved below the button. Error alert unchanged.
4. The "Connections" list, "No connections yet." and their heading are **removed**: a contact's
   connection now shows on its row, a non-contact's on Discover.

#### My Info tab

Top to bottom:

1. "Current name: <name>" on the left, `ThemePicker` on the right (uses `chooseTheme`; a failed save
   shows the error in the page's existing `role="alert"` area).
2. The display name form: "Display name" label and field, "Save" on the right.
3. Divider. "Connection ticket" heading, "Network starting…" when applicable, the QR (centred, on a
   light quiet zone in every theme so it stays scannable), "Copy ticket", the full ticket in a muted
   mono block.
4. Divider. "NodeId" heading, the full NodeId in `.node-id`, "Copy NodeId".
5. Copy notices ("Ticket copied", "NodeId copied", "Copy failed") in a `role="status"` line.

### Part C — Contacts, conversation and contact info

#### Contact list (Contacts tab)

```
[Verified by you] Alice │ outbound │ connected                      (2)
[Unverified]      Bob   │ Not connected
[Unverified]      Carol │ inbound │ failed │ <reason>
```

- The whole row is one `<button>` opening the conversation; its accessible name is the nickname
  plus the visible details.
- Fields: trust badge (existing labels), nickname (`<bdi>`, ellipsis), then `direction | state |
  reason` verbatim from the live `connections` entry, or the existing "Not connected".
- An unread count chip on the right, only when > 0, labelled "N unread".
- The NodeId moves to contact info. Message / Message (N), Connect, Rename, Mark verified… / Mark
  unverified, Remove… and their inline forms and dialogs leave the row (all move to contact info).
- `LanPeers` leaves Contacts (now Discover). The error alert, the edited empty state and the
  `role="status"` notice stay; the notice comes from a small store so contact info can post
  "Removed …" after navigating back.

#### Conversation screen

```
[Back]  Alice  [Verified by you]  Connected                     [⋮]
(not-connected explanation, when applicable — edited text, see inventory)
[Load older messages]
┌────────────────────────────┐
│ Hi                         │
│      09:38 │ 09:39         │   ← incoming: sent (their clock) │ received (this device)
└────────────────────────────┘
                 ┌───────────────────────────────┐
                 │                          You  │
                 │ Hi                            │
                 │       09:40 · Delivered       │   ← outgoing: sent time · existing status label
                 └───────────────────────────────┘     (DD-08 adds the delivered time here)
[ Type anything…                                  ] [➤]
```

- **Header:** Back, the nickname (or the existing 8-character NodeId fallback), `TrustBadge`,
  "Connected"/"Not connected", and a **⋮** icon button named "Contact info".
- **Moved out to contact info:** "Clear history…" with its dialog and "History cleared.", and the
  search box with its results.
- **Bubbles:**
  - Outgoing bubbles show **"You"** (existing word, from today's search results) at the top right.
    Incoming bubbles carry no name; the header already identifies the peer.
  - Incoming meta: `sent | received`, each a `<time>` with a `title`, "Sent (their clock)" and
    "Received (this device)". If `received_at_ms` is missing or invalid, only the sent time shows.
  - Outgoing meta: sent time, then the existing status label ("Sending…", "Delivered", "Pending:
    will be sent when connected", "Not delivered"), plus "Resend" when failed.
  - Ordering stays `(sent_at_ms, seq)` per DD-04.
- **Composer:** one-line auto-growing textarea (existing accessible name "Message", new placeholder
  "Type anything…") and an arrow icon button named with the existing "Send". Ctrl/Cmd+Enter, the
  byte counter and the over-limit behaviour are unchanged.
- The log keeps `role="log"`, `aria-live="polite"`, stick-to-bottom scrolling and
  load-older-preserves-position, and fills the available height instead of `50vh`.

#### Contact info screen (new `ContactInfo.svelte`)

Top to bottom:

1. **Header:** Back; the nickname as heading (`<bdi>`) with `TrustBadge`; a **pencil** icon button
   (accessible name: existing "Rename") and a **trash** icon button (existing "Remove…").
   - Pencil opens the existing `NicknameForm` inline and saves via `contacts_rename`.
   - Trash opens the existing `RemoveContactDialog`, unchanged. On success: navigate to Contacts and
     post the existing "Removed “X”. Your message history with them was kept/was deleted."
2. **Identity:** `NodeIdLine` (short NodeId, "Show full NodeId" / "Hide full NodeId", "Select it and
   use your system's copy shortcut.", full NodeId). Below, the existing sentence "They call
   themselves “X” (chosen by them, not verified)" or "They did not send a name".
3. **Trust:** "Mark verified…" (existing `VerifyContactDialog`) or "Mark unverified".
4. **Connection:** `direction | state | reason` (or "Not connected"), and the existing "Connect"
   button when a LAN ticket is known and the contact is not connected (same condition as today).
5. **History:** "Clear history…" → existing `ClearHistoryDialog`; on success "History cleared." and
   the cached conversation is reset, so Back shows an empty chat.
6. **Search:** the existing "Search this conversation" box with the same behaviour (stale responses
   dropped, Esc clears, errors in `role="alert"`); results show "You"/"Them", local time, body, or
   "No matches.".

**Trade-off:** every action on contact info (the user's decision) makes "Mark verified" two clicks
further away than today. The trust *state* stays visible in the list and the conversation header;
only the action moves.

### Text inventory

Rule: **existing text is kept verbatim.** Wireframe labels that differ ("Rename:", "Connect and
Remember", "Visible:", "Welcome to p2p-messenger / Create your identity", "Display name:", "Node Id:
Id", "copy", "save", "ADD") were placeholders for the existing strings.

| Kind | Where | Text |
|---|---|---|
| **Edited (minimal)** | Contacts empty state | "No contacts yet. Add someone from ~~*On this network* below~~ ***Discover***, connect with a ticket in the Network tab, or accept an incoming request." |
| **Edited (minimal)** | Conversation, not connected | "…when they connect to you, or when you use ~~Contacts → Connect~~ **Connect in their contact info** or paste their ticket in Network." |
| Removed (step merged) | Setup | "Choose a display name", "Create identity", "All set" |
| Removed (click merged) | Discover row | "Add…" |
| Removed | Contact row | "Message", "Message (N)" |
| Removed | Network | "No connections yet." |
| Removed | Conversation | "This contact was removed." (removed contacts are never displayed) |
| Moved | Network → Discover | "Connections", "Add contact…" |
| Moved | Contacts → Discover | "On this network" section |
| Moved | Conversation → contact info | "Clear history…", "History cleared.", "Search this conversation", "No matches.", "You"/"Them" |
| Moved | Contact row → contact info | "Connect", "Rename", "Mark verified…", "Mark unverified", "Remove…" |
| Reused | Setup arrow / outgoing bubble / row | "Continue" (accessible name), "You", "Not connected" |
| New | Menu / footer | "Discover", "p2p-messenger" |
| New | Setup | "Choose a theme" |
| New | Theme picker | group "Theme"; options "Midnight", "Dark", "Light" |
| New | Network | switch captions "Off", "On" |
| New | Conversation | "Contact info" (⋮ name), "Sent (their clock)", "Received (this device)", "Type anything…" |
| New | Contact row | "N unread" |

### Restyling the remaining views

Every view not re-laid out above (dialogs, prompts, error screens, `TrustBadge`, `NodeIdLine`) keeps
its structure and text and switches to the shared classes:

- `TrustBadge`: "Verified by you" uses `--success` on `--success-soft`, "Unverified" `--warning` on
  `--warning-soft`; labels and tooltips unchanged.
- Dialogs (`VerifyContactDialog`, `RemoveContactDialog`, `ClearHistoryDialog`,
  `IncomingRequestPrompt`, `NamingPrompt`) use `.dialog`, secondary-first button order, destructive
  confirms `.btn-danger`.
- `ErrorBlocking` / `StorageUnavailable` use `.alert-danger` with a `.btn-primary` Retry.
- Every `role="alert"` uses `.alert-danger`; every `role="status"` uses `.alert-info` or muted text.
- Failed bubbles get the `--danger` border; the composer's over-limit counter uses `--danger`.

### Removing the wireframes

The images in `design/wireframes/` were inputs for this doc only. Their content is captured above
(layout sketches, element order, text mapping), so they are deleted in the final change of this
work, and every link to them is removed (this doc's header line and the README status paragraph).
`design/visual-identity.html` is **not** deleted: it is the living visual spec the drift guard
checks against.

## Security and privacy

- **Closed theme enum end to end:** TypeScript allowlist, serde enum, `core` enum and SQL `CHECK`.
  The theme is never interpolated into CSS, a style attribute, a class built from input, or HTML;
  `applyTheme` maps through the allowlist before touching `data-theme`.
- **Nothing outside the encrypted DB:** no `localStorage`, `sessionStorage`, cookies, IndexedDB or
  files for the theme. `pendingTheme` is a JS variable until the account exists. Guard test: no
  `localStorage`/`sessionStorage` under `app/src` outside tests.
- **No network fetches for styling:** no web fonts, CDN CSS or remote images. The CSP stays exactly
  as it is; `style-src 'unsafe-inline'` is already required by Svelte and must not be widened, and
  no `font-src` or remote origin is added. Moving to one stylesheet does not loosen the CSP.
- **Peer-controlled strings are text only:** message bodies, self-chosen names, nicknames and
  connection `reason` strings use text interpolation with `<bdi>` / `dir="auto"`, and never appear
  in `href`, `src`, `style`, a class built from input, a `title` built from a body, or `{@html}`.
  `render-guard` (no `<a`, `href`, `window.open`, `location`) is extended to `ContactInfo.svelte`
  (renders bodies in search results) and `ContactRow.svelte`.
- **Raw HTML:** the only `{@html}` stays the QR in `MyInfo.svelte`, generated by `core` from the
  local ticket (`html-guard`). Icons are inline SVG written in components, never injected strings.
- **Clipboard:** only in `MyInfo.svelte`, exactly the ticket and the NodeId (`clipboard-guard`).
  Contact NodeIds are selectable only.
- **Trust prompts can't be hidden by navigation:** both prompts live in the shell above every
  screen, tested across all tabs and drill-in screens.
- **Trust signals stay legible and labelled:** contrast is tested in every theme; verified /
  unverified keep their text labels, so a theme can never make an unverified contact look verified.
  `--success` is used only for positive trust or delivery states. Self-chosen names appear only
  inside the "(chosen by them, not verified)" sentence or next to the Unverified badge, never as a
  heading or in place of a nickname.
- **Timestamps labelled for what they are:** the sender's time says "their clock" and is never used
  to imply delivery or ordering guarantees beyond DD-04. Invalid values fall back through the
  existing `formatLocalTime` / `guardedIso` guards.
- **Setup can't create half-trusted state:** `AlreadyProvisioned` never overwrites a name, provision
  is not repeated on retry, and a theme-save failure blocks nothing.
- **Destructive actions keep their confirmations:** remove contact (optional, explicit history
  erasure with its guarantee text) and clear history (Cancel autofocused). The trash only opens the
  dialog. Add stays disabled for an empty nickname, ticket Connect for an empty field.
- **No stale access after removal:** the router leaves drill-in screens on removal, wherever it
  happens; a removed contact's conversation is never rendered, so no command can be sent for it
  from the UI. Kept history is only shown again after the same NodeId is re-added.
- **Search stays local:** existing `history_search` over the encrypted FTS index, scoped to the
  NodeId; results are not cached outside the component and are cleared on navigation and history
  clear.
- **Logging and secrets:** setting the theme needs no log line; if one is added it goes through
  `core::logging` with a reviewed field name in the redaction-guard allowlist. New `app/` code
  names no secret types (grep gate).
- **Profile isolation:** the theme lives in the per-profile DB; no global preference file.
- **Deleting the wireframes** removes images only; no runtime code references them (checked by a
  grep in the definition of done).

## Tests

Rust (`core`):

- Migration: a DB at the previous schema version upgrades, the old row reads `theme = NULL`, and
  `IdentityService::theme()` returns Midnight.
- SQLCipher round-trip for each value; an out-of-range raw value written with direct SQL fails the
  `CHECK`.
- One settings contract test run against both the SQL store and `MemorySettingsStore`.
- `theme` / `set_theme` without an account return the typed error; a second account's theme is
  independent; the namespacing guard stays green.

Rust (`app`):

- `identity_theme_get` on a fresh account returns `midnight`; set then get returns the set value.
- `identity_theme_set` with an unknown string returns `InvalidTheme` and changes nothing.
- `first_run_flow_end_to_end` gains the theme-set step after naming and asserts the stored value.
- `remove_keep_then_readd_restores_history_end_to_end`: remove without erase, re-add the same
  NodeId, the old messages are returned again.
- Existing `chat_flow_end_to_end`, `history_clear_end_to_end` and
  `remove_with_erase_clears_chat_history_end_to_end` keep covering the commands the new screens call.

Frontend guards (Vitest):

- **Single stylesheet:** no `.svelte` file under `app/src` contains a `<style>` block, a `style=`
  attribute or a `style:` directive; exactly one stylesheet is imported, from `main.ts`.
- **Contrast:** parse the stylesheet; for each theme assert ≥ 4.5:1 for `--text`, `--text-strong`,
  `--text-muted`, `--accent`, `--success`, `--warning`, `--danger` against `--bg` and `--surface`;
  each status colour against its `-soft` tint composited over `--surface`; each bubble text on its
  bubble; `--accent-contrast` on `--accent`.
- **Drift:** every theme token value in the app stylesheet equals the value in
  `design/visual-identity.html`.
- **No hardcoded colours:** no `#rgb`/`#rrggbb`, `rgb(` or named colours outside the theme blocks of
  the stylesheet; no `color-mix(` or `:has(` anywhere in app CSS.
- **No web storage:** no `localStorage` / `sessionStorage` under `app/src` outside tests.
- **render-guard** extended to `Chat`, `MessageBubble`, `Composer`, `ContactInfo`, `ContactRow`
  with an exact file-count assertion. `html-guard` and `clipboard-guard` stay green.

Frontend components (Vitest + Testing Library):

- **theme.ts:** `applyTheme` falls back to Midnight for values outside the allowlist; `loadTheme`
  falls back on rejection; `chooseTheme` reverts on failure.
- **ThemePicker:** three named options, current one checked, arrow keys move, `onChoose` value.
- **App:** Present + storage OK calls `identity_theme_get` before the shell renders; first-run
  completion and relaunch both land on Contacts.
- **Router:** default Contacts; menu click switches tab and clears drill-in; chat Back → Contacts;
  contact-info Back → chat; `showTab` from contact info goes to that tab; focus moves to heading or
  Back.
- **Shell:** four tabs in order with `aria-selected` and arrow keys; footer on tabs only; both
  prompts render on every tab, the chat and contact info.
- **Setup:** Midnight preselected, choosing Dark applies without `identity_theme_set`; arrow absent
  for empty/whitespace names; happy path calls provision → set name → set theme (chosen value) →
  `onDone`; `AlreadyProvisioned` skips set name; a name error shows the existing message and the
  retry does not re-provision; a theme-save failure still calls `onDone`.
- **Discover:** form inline without "Add…", Add disabled until a nickname, add-and-connect texts
  unchanged, contacts filtered out; the Connections group is hidden when every connection is a
  contact, shows "Add contact…" only when connected.
- **Contacts:** edited empty state; no LAN list; removal notice from contact info shown.
- **ContactRow:** badge, nickname, `direction | state | reason` or "Not connected"; unread chip only
  > 0 with "N unread"; click/Enter opens the chat; none of Rename, Mark verified, Remove, Connect,
  Message rendered.
- **Network:** order switch, captions, explanation, ticket field, button, paragraph; no Connections
  list; presence tests unchanged.
- **MyInfo:** picker present, order as specified, both copy buttons copy exactly ticket and NodeId,
  hidden heading exists.
- **Chat:** header content and ⋮ navigation; no search and no Clear history; edited not-connected
  sentence; outgoing bubble "You" + time + status (+ Resend on failure); incoming bubble two
  `<time>` with the right titles, one when `received_at_ms` is null; composer placeholder, Send by
  name, Ctrl+Enter and byte counter.
- **ContactInfo:** pencil renames via `contacts_rename`; trash removal navigates to Contacts with the
  right kept/deleted notice; verify dialog and unverify call; Connect only with a LAN ticket while
  disconnected; Clear history notice and cache reset; search results, stale drop, Esc, error alert;
  both self-name variants; external removal returns to Contacts and renders nothing for the
  removed contact (same on the chat screen).
- Existing `chatStore`, `contactsStore`, `netStore` and dialog tests stay green.

Manual (README checklist rewritten for the new paths):

- Fresh profile: Midnight setup, arrow appears only after typing, picking Light repaints instantly,
  Continue lands on Contacts in Light; relaunch opens Contacts in Light; a second `--profile` keeps
  its own theme.
- Switch all three themes on My Info: every tab, dialog, badge and bubble is readable.
- Keyboard only: menu arrows, picker arrows, row Enter, ⋮, Back; focus ring visible in every theme.
- Two instances: peer appears under Discover; nickname + "Add contact and connect" adds and
  connects; the row shows `outbound | connected`; the conversation shows "You" and *sent | received*;
  ⋮ opens contact info; pencil renames (list and header update); verify works through the dialog;
  trash removes and returns to Contacts with the notice.
- Clear history in contact info, then Back: the conversation is empty.
- Remove a contact keeping history: their conversation is gone from the app; re-add the same
  NodeId: the old messages are back.
- Stop the peer: the row shows the state and reason; the conversation shows the edited sentence.
- A pending incoming request stays visible while switching tabs and opening chat / contact info.

## Suggested implementation order

One doc, delivered in reviewable PRs, each leaving the app working:

1. Stylesheet, tokens, theme setting (migration, store, service, commands), `theme.ts`,
   `ThemePicker` on My Info, every view moved to shared classes with `<style>` blocks deleted.
2. Router, shell with four tabs, setup screen, Discover, Network and My Info layouts.
3. Contact rows, conversation, contact info, Connections move; then delete `design/wireframes/`.

## Definition of done

- [ ] One global stylesheet imported once; `index.html` defaults to `data-theme="midnight"`; no
      `.svelte` file has a `<style>` block, `style=` attribute or `style:` directive.
- [ ] Token values equal `design/visual-identity.html` (drift guard); no `color-mix()` / `:has()`.
- [ ] Migration adds `account_settings.theme` with the `CHECK`; `Theme` enum, both settings stores
      and `IdentityService` updated.
- [ ] `identity_theme_get` / `identity_theme_set` implemented, registered, invoked and tested.
- [ ] `theme.ts` and `ThemePicker` done; picker on setup and My Info; theme applied before the shell
      renders; Midnight on every pre-account screen.
- [ ] `screen` store with Back chains contact-info → chat → Contacts and focus management.
- [ ] Shell with Contacts | Discover | Network | My Info; menu and footer on tab screens only
      (hidden on drill-ins); prompts everywhere.
- [ ] Setup screen as specified (conditional arrow, provision → name → theme → Contacts, retry
      without re-provisioning).
- [ ] Discover hosts the LAN list (inline nickname + Add) and the non-contact Connections group.
- [ ] Network and My Info re-laid out; Network Connections list removed.
- [ ] Contact rows redesigned with no action buttons; conversation redesigned; `ContactInfo.svelte`
      with every per-contact action, reusing existing commands and dialogs.
- [ ] Text inventory matches the implementation exactly; the two listed edits are the only edits to
      existing text (reviewed in the PR).
- [ ] Single-stylesheet, contrast, drift, no-hardcoded-colour, no-web-storage and extended render
      guards green; `html-guard`, `clipboard-guard`, log-redaction, SQL-namespacing and secret grep
      gates green; CSP unchanged.
- [ ] All tests listed above added or updated and green.
- [ ] `design/wireframes/` deleted; no Markdown link, import or code reference points into it.
- [ ] README: themes, screen structure, contact info path and manual checklist updated.
- [ ] Pre-commit hook passes (fmt, clippy `-D warnings`, workspace tests, frontend
      check/test/build).
