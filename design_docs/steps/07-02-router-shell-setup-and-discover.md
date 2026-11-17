# Step 07-02 — Screen router, four-tab shell, one-screen setup, Discover, Network and My Info layouts

**Size:** L — about +1,350 / −550 lines. Rust ≈ +25 (the `first_run_flow_end_to_end` theme step).
Frontend ≈ +700 / −300 (`router.ts` and `chatStore.ts` ≈ ±120, `MainScreen.svelte` shell ≈ +120,
`FirstRunWizard.svelte` rewrite ≈ +100 / −76, new `Discover.svelte` ≈ +70, `LanPeers`/`Network`/
`Contacts`/`ContactRow`/`MyInfo` ≈ ±250, `app.css` ≈ +120). Tests ≈ +600 / −250. README ≈ ±60.

## TL;DR

Second of four sequential steps for [DD-07](../07-visual-identity-and-ux-redesign.md) (PR 2 of
"Suggested implementation order", plus the "Connections move" — see below). It:

1. Replaces `activeTab` + the `activeChat` takeover with one in-memory **`screen` store** in
   `app/src/lib/router.ts` (`tab` | `chat` in this step; `contact-info` arrives in 07-03).
   `chatStore.openChat`/`closeChat` keep loading the conversation and delegate navigation to the
   router. Focus moves on every screen change.
2. Turns `MainScreen.svelte` into the **shell**: menu **Contacts | Discover | Network | My Info**
   (`role="tablist"`, roving tabindex, Left/Right arrows), both trust prompts global on every
   screen, the footer "p2p-messenger" on tab screens only, visually hidden tab headings.
3. Rewrites `FirstRunWizard.svelte` as the **one-screen setup** (Welcome, theme picker, display
   name, conditional arrow "Continue"; provision → name → best-effort theme → `onDone`).
4. Adds the **Discover** tab (`LanPeers` moved out of Contacts with the nickname form inline and
   "Add…" removed, plus the non-contact "Connections" group moved out of Network).
5. Re-lays out **Network** (switch with "Off"/"On" captions, ticket form, Connections list removed)
   and **My Info** (order per the DD).
6. Keeps every contact's connection visible: the existing contact row gains the
   `direction | state | reason` / "Not connected" line now (the row's full redesign is 07-03).

**Why the Connections move happens here and not in 07-03:** the DD's order puts the Network
re-layout in PR 2 and the "Connections move" in PR 3. Removing the Network list without a new home
would make a contact's direction/state/reason and the non-contact "Add contact…" unreachable between
steps. So this step removes the Network list **and** adds both new homes (Discover group, contact
row line) in the same change. Nothing becomes unreachable at any commit.

## Mandatory process rules

- After each step you implement and after each subagent execution completes, run the /usage-guard skill (Skill tool, skill: "usage-guard") to check plan usage; if it says to wait, wait until the limit resets before continuing. Pass this exact instruction on to every subagent you create, including subagents' subagents.
- If in doubt, ask; do not assume.
- Re-read DD-07 Part B ("Setup, shell and navigation"), the Text inventory and the Tests section
  before starting.
- Never write credentials or secrets in code, tests, fixtures or docs. Every credential must be
  provided at runtime; this work needs none.

## Prerequisites and branch

Steps are strictly sequential: **07-01 → 07-02 → 07-03 → 07-04**. 07-01 must be merged into the DD
branch first.

1. `git fetch origin --prune`. Find the last worked branch for this DD — do **not** assume `main`
   is current: `git branch -a --sort=-committerdate | head`, `git log --oneline --all -15`, and
   `gh pr list --state all --limit 10`. It is normally `07-visual-identity-and-ux-redesign` with the
   07-01 PR merged; if the 07-01 PR is still open, stop and ask.
2. `git checkout 07-visual-identity-and-ux-redesign && git pull --ff-only`, then
   `git checkout -b step-07-02-router-shell-setup-and-discover`.
3. Hook installed: `git config core.hooksPath .githooks`.
4. Open the PR into `07-visual-identity-and-ux-redesign` per `.github/PULL_REQUEST_TEMPLATE.md`.

## Resolved decision: menu hidden on drill-in screens (answered by the user)

The DD contradicted itself (screen-model table and TL;DR hide the menu on `chat` / `contact-info`;
the shell sketch, DoD and one router test implied it stays). **The user chose: hide it.** Drill-ins
are full-screen with Back; menu and footer are hidden there; the trust prompts
(`IncomingRequestPrompt` / `NamingPrompt`) still render on every screen. The router test "menu click
from contact info goes to that tab" becomes "calling `showTab` from a drill-in goes to that tab and
clears it". Shell tests assert the tablist is absent on `chat` and `contact-info`.

## Parallel vs sequential

Across steps: sequential. Inside this step:

| Track | Owns | Depends on |
|---|---|---|
| **A — Router + shell** | `lib/router.ts`, `lib/chatStore.ts` (+ tests), `views/MainScreen.svelte` (+ test), `App.svelte` | nothing |
| **B — Setup screen** | `views/FirstRunWizard.svelte` (+ test), `App.test.ts`, `app/src-tauri/src/identity_cmd.rs` (first-run test only) | `lib/theme.ts` (07-01); App.test needs A's App.svelte change → finish B's App.test after A merges |
| **C — Discover, Network, Contacts, row line** | new `views/Discover.svelte` (+ test), `LanPeers.svelte`, `Network.svelte`, `PresenceToggle.svelte`, `Contacts.svelte`, `ContactRow.svelte` (+ their tests) | nothing |
| **D — My Info layout** | `views/MyInfo.svelte` (+ test) | nothing |

Then **sequentially**: **E — Integration**: register `Discover` in the shell's tab table (A's file,
needs C's component), run the whole Vitest suite, update `README.md`, run the gate.

Conflict risks:

- `app/src/app.css` is touched by every track. Each track appends its classes only inside its own
  marked block (`/* shell */`, `/* setup */`, `/* discover + network */`, `/* my info */`) at the end
  of the component-classes section; merges are then trivial. Same rules as 07-01: no colour outside
  the theme blocks, no `color-mix()`/`:has()`.
- `App.svelte`: owned by A only. `App.test.ts`: owned by B, rebased on A.
- `MainScreen.svelte` imports the tab components: C must not edit it; E adds Discover.
- `lib/router.ts` must not import any view component (see A.1), so `chatStore.ts` can import it
  without an import cycle.

## Tasks

### Track A — Router, chatStore, shell, App

1. **`app/src/lib/router.ts`** — keep `appView`/`AppView`; remove `activeTab`, `tabs`, `wizardView`
   and **every component import** (today it imports `FirstRunWizard`, `Contacts`, `MyInfo`,
   `Network`; `chatStore` → `router` → components → `ContactRow` → `chatStore` would be a cycle).

   ```ts
   export type TabId = "contacts" | "discover" | "network" | "my-info";
   export type Screen = { kind: "tab"; tab: TabId } | { kind: "chat"; nodeId: string };
   const HOME: Screen = { kind: "tab", tab: "contacts" };
   export const screen = writable<Screen>(HOME);
   export const showTab = (tab: TabId) => screen.set({ kind: "tab", tab }); // clears drill-in
   export const showChat = (nodeId: string) => screen.set({ kind: "chat", nodeId });
   export const resetScreen = () => screen.set(HOME);
   /** Back from a drill-in screen: chat → Contacts (07-03 adds contact-info → chat). */
   export function back(): void { ... }
   ```
   `App.svelte` imports `FirstRunWizard` directly and calls `resetScreen()` when it enters `main`, so
   both first-run completion and every launch land on Contacts.

2. **`app/src/lib/chatStore.ts`** — delete `activeChat`. `openChat(nodeId)` calls `showChat(nodeId)`
   then `openConversation(nodeId)` (unchanged loading duty). `closeChat()` calls `back()`. Clear the
   cached conversation whenever the screen becomes a `tab` (subscribe to `screen` once at module
   level; `conversation.set(null)` when `kind === "tab"`), so a menu click from a conversation never
   leaves a stale conversation loaded and unread counting resumes. Update `chatStore.test.ts`
   (`open_and_close_chat_set_activeChat` becomes e.g. `open_and_close_chat_drive_the_screen`).

3. **`app/src/views/MainScreen.svelte` = the shell.** Tab table lives here:
   `[{ id: "contacts", label: "Contacts", component: Contacts }, { id: "discover", label: "Discover",
   … }, { id: "network", label: "Network", … }, { id: "my-info", label: "My Info", … }]` (Discover
   added in E). Layout per the DD's App shell sketch:
   - On **tab** screens: the menu `<div role="tablist" class="tabs">` pinned at the top, equal-width
     `<button role="tab" class="tab">` items with `aria-selected`, `aria-controls` → the panel,
     **roving tabindex** (selected = `0`, others = `-1`), ArrowLeft/ArrowRight (wrapping) select the
     previous/next tab and move focus to it; Home/End optional. Then the prompts, then
     `<div role="tabpanel" id=… aria-labelledby=…>` with the tab component, then
     `<footer class="app-footer">p2p-messenger</footer>`.
   - On the **chat** screen: no menu and no footer (DD screen-model table: "Menu + footer: hidden"
     on drill-ins); the prompts, then `<Chat nodeId={…} onBack={closeChat} />` (today's component,
     restyled in 07-01; its redesign is 07-03).
   - `IncomingRequestPrompt` and `NamingPrompt` render on **every** screen, above the content, as
     cards (so no drill-in can hide them).
   - Keep starting/stopping `startNetEvents`, `startContacts`, `startChatEvents` exactly as today.
   - **Focus management:** on every screen change after the first render, after `tick()`, focus the
     element marked `data-screen-focus` inside the new screen: the tab's visually hidden `<h2
     tabindex="-1">` or the drill-in's Back button. Exception: when the change came from arrow keys
     in the tablist, focus stays on the newly selected tab (WAI-ARIA tabs pattern). Add
     `data-screen-focus` to `Chat.svelte`'s Back button (one attribute; no other Chat change).
   - The content column is centred with a max width; the menu stays pinned while the content
     scrolls; single column that fits 900×640 and reflows down to phone widths.
   - Do not add an `aria-label` or any other text that is not in the DD's Text inventory (ask if a
     label seems necessary).

   - The shell's "four tabs in order" test can only be finalized in Integration E, once
     `Discover.svelte` exists; until then Track A tests the three existing tabs plus the router.

4. **Tab headings:** each tab component's `<h2>` ("Contacts", "Discover", "Network", "My Info")
   stays in the DOM with `class="visually-hidden"`, `tabindex="-1"` and `data-screen-focus`.

### Track B — One-screen setup (`FirstRunWizard.svelte`)

Rewrite the three steps (generate / name / land) into one form; texts kept verbatim, three removed:

```
<h2>Welcome</h2>                               ThemePicker   ← current={$pendingTheme ?? "midnight"}
<p>Create your identity on this device.</p>    <p>Choose a theme</p>
<label>Display name <input class="input" …></label>
{#if name.trim() !== ""}<button type="submit" aria-label="Continue" class="btn btn-primary btn-icon"><svg aria-hidden="true" …/></button>{/if}
{#if error}<p role="alert" class="alert alert-danger">{error}</p>{/if}
```

- Removed text: "Choose a display name", "Create identity", "All set". New: "Choose a theme".
- Picker `onChoose(t)`: `pendingTheme.set(t); applyTheme(t);` — applies immediately, persists
  nothing (no account yet), never calls `identity_theme_set`.
- The arrow is **not rendered** while the trimmed name is empty; Enter on an empty/whitespace name
  shows `displayNameMessages.Empty` without calling core. Field and button disabled while busy.
- Submit (guard `if (busy) return`):
  1. If this component has not provisioned yet: `identityProvision()`. On `AlreadyProvisioned` →
     `pendingTheme.set(null)` then `onDone()` immediately, writing **neither** the name nor the
     theme (another instance's settings are never overwritten; clearing the unsaved pick makes
     `loadTheme()` apply the existing account's stored theme). On any other error: `describeError(e)` in the alert, stop. On success
     remember `provisioned = true` (component state) so a retry skips this step.
  2. `identitySetDisplayName(name)`. `InvalidDisplayName` → `displayNameMessages[kind]`; other →
     `describeError`; stop and stay (retry skips step 1).
  3. `identityThemeSet(get(pendingTheme) ?? "midnight")`, best effort: on success
     `pendingTheme.set(null)`; on failure keep `pendingTheme` (07-01's `loadTheme()` then keeps
     applying it this session) and continue. A cosmetic preference never blocks entry.
  4. `onDone()` → `App` re-reads status, `loadTheme()`, `resetScreen()`, shell on Contacts.
- If the app closes between steps 1 and 2, the next launch goes to Contacts and My Info is where
  the name is set (existing behaviour; nothing to add).
- **Rust:** extend `first_run_flow_end_to_end` in `app/src-tauri/src/identity_cmd.rs` with
  `theme_set(&service, "light")` after `set_display_name` and assert `theme_get` returns `"light"`;
  in `first_run_flow_end_to_end_with_encrypted_settings` also assert the theme survives the
  rebuilt service.

### Track C — Discover, Network, Contacts, contact-row connection line

1. **New `app/src/views/Discover.svelte`** (the DD lists only `ThemePicker`/`ContactInfo` as new
   files; a Discover tab component is needed to host the hidden "Discover" heading and two
   sections — ask if you would rather render them elsewhere):
   - `<h2 class="visually-hidden" tabindex="-1" data-screen-focus>Discover</h2>`
   - `<LanPeers />`
   - **Connections without a contact**: `const others = Object.values($connections).filter((c) =>
     !$contactIds.has(c.node_id))`. Render only when `others.length > 0`: the existing heading
     "Connections" and per entry the Unverified `TrustBadge`, `NodeIdLine`, `direction | state |
     reason` (reason only when non-null; keep `data-testid`s `conn-direction`, `conn-state`,
     `conn-reason`), and the existing **"Add contact…"** button only when `state === "connected"`,
     calling `requestNaming(c.node_id, c.display_name)` (the existing `NamingPrompt` flow). Move this
     markup from `Network.svelte`; "No connections yet." is **removed** (not moved).
2. **`LanPeers.svelte`**: heading "On this network", its description and "No one new on this
   network." unchanged. Each peer is one `.list` row: Unverified badge, "calls themselves “<bdi>…</bdi>”"
   or "(no name)", and the Add button ("Add contact and connect") on the right; below it the
   `NicknameForm` context line and "Your name for them" field, then `NodeIdLine`. The form is
   **always inline** (delete the `adding` state and the "Add…" button); keep the Add button as the
   `NicknameForm` submit (so it stays disabled until the nickname is non-empty and Enter submits) and
   place it with CSS grid placement — do **not** build DOM ids or `form=` attributes from peer
   data, and keep the DOM order equal to the visual and tab order (no CSS `order` reordering; if
   the layout cannot be reached that way, keep Add after the field and ask). Pass no `onCancel` (no Cancel button on an always-inline form). Add-then-connect, the
   error texts (`Added “X”, but connecting failed: …`) and the list refresh are unchanged; contacts
   stay filtered out (`$contactIds`).
3. **`Network.svelte`** top to bottom: visually hidden `<h2>Network</h2>`; `PresenceToggle` (switch
   "Visible on this network" with small "Off" / "On" captions either side — captions
   `aria-hidden="true"` and outside the `<label>`, so the switch's accessible name is unchanged — then
   the existing explanation paragraph); `<hr class="divider">`; the ticket form: label "Connect by
   ticket", full-width multi-line `.textarea`, the button "Connect and remember this device"
   right-aligned, **then** the paragraph "Once they reply, they're saved as an unverified contact…"
   (moved below the button), error alert unchanged. Delete the "Connections" heading, list and "No
   connections yet." and the now-unused imports.
4. **`Contacts.svelte`**: remove `<LanPeers />`; heading "Contacts" visually hidden (as above). Edit
   the empty state — the first of the two allowed text edits — exactly to:
   `No contacts yet. Add someone from <em>Discover</em>, connect with a ticket in the Network tab, or accept an incoming request.`
   (Was "…from <em>On this network</em> below, …".) The error alert and `role="status"` notice stay.
5. **`ContactRow.svelte`**: keep everything (buttons, inline forms, dialogs) and add one line under
   the nickname: from `$connections[contact.node_id]` render `direction | state` (+ ` | reason` when
   non-null) as text, or the existing "Not connected" when there is no entry. This is the contact's
   new home for what the Network list showed; 07-03 redesigns the row around it.

### Track D — My Info layout (`MyInfo.svelte`)

Top to bottom (DD "My Info tab"): visually hidden `<h2>My Info</h2>`; a `.row` with "Current name:
<span data-testid="display-name">…</span>" on the left and `ThemePicker` on the right (07-01 wiring;
failures in the page's `role="alert"`); the display-name form ("Display name" label + field, "Save" on
the right); divider; "Connection ticket" heading, "Network starting…" when applicable, the QR
centred on the `--qr-quiet` frame, "Copy ticket", the full ticket in a muted mono block; divider;
"NodeId" heading, the full NodeId in `.node-id`, "Copy NodeId"; the copy notices ("Ticket copied",
"NodeId copied", "Copy failed") in one `role="status"` line. Keep `writeText(value)`,
`copy(info.ticket, …)` and `copy(info.node_id, …)` byte-for-byte (`clipboard-guard.test.ts`).

### Integration (E, sequential)

1. Add `Discover` to the shell's tab table in order Contacts | Discover | Network | My Info.
2. `README.md`: update every sentence that names a moved place (e.g. "Contacts → On this network" →
   "Discover"; "Connected under Network → Connections" → the contact's row; the wizard steps → the
   setup screen; Message button text stays until 07-03). Also the removed "Add…": the "On this
   network, presence and relay" section (~line 196, "**Add…** asks for your nickname…") and the
   manual checklist item 9 (~line 420, "Add… it: …") now describe typing the nickname inline and
   pressing "Add contact and connect". Keep it minimal; 07-03 rewrites the
   themes/screen-structure sections and the manual checklist.
3. Full test suite and the gate.

## Tests

- **Rust:** `first_run_flow_end_to_end` and `first_run_flow_end_to_end_with_encrypted_settings`
  gain the theme step (above). Existing `ticket_connect_accept_flow_end_to_end`,
  `chat_flow_end_to_end` stay green.
- **`lib/router` (new `app/src/lib/router.test.ts`):** default is `{ kind: "tab", tab: "contacts" }`;
  `showTab` switches and clears drill-in; `showChat` then `back()` → Contacts; `resetScreen()`.
- **`chatStore.test.ts`:** `openChat` sets the chat screen and loads; `closeChat` returns to
  Contacts; moving to a tab clears `conversation`; unread counting unchanged.
- **Shell (`MainScreen.test.ts`, rewrite the `activeTab`/`activeChat` cases):** four tabs in order
  Contacts, Discover, Network, My Info with `aria-selected`; roving tabindex; ArrowRight/ArrowLeft
  (with wrap) select and focus; menu click switches tab and clears the chat; footer
  "p2p-messenger" present on every tab and absent on the chat screen; menu absent on the chat
  screen; both prompts render on **every tab and on the chat screen**; focus moves to the hidden
  heading after a menu click and to Back when a chat opens; listeners started/stopped as before.
- **Setup (`FirstRunWizard.test.ts`, rewrite):** Midnight preselected; choosing Dark applies
  `data-theme="dark"` without calling `identity_theme_set`; the arrow is absent for empty and
  whitespace names and Enter shows the Empty message without calling core; happy path calls provision
  → set name → set theme (with the chosen value) → `onDone` in that order; `AlreadyProvisioned`
  calls `onDone`, skips both set name and set theme, and leaves `pendingTheme` `null` even after a
  pick; a name error shows the existing message and
  the retry does **not** re-provision; a theme-save failure still calls `onDone`; other provision
  errors show `describeError`; "Choose a display name", "Create identity" and "All set" are absent.
- **`App.test.ts`:** replace "Create identity" usage with the new flow; first-run completion lands
  on the Contacts tab (selected); relaunch with Present lands on Contacts; existing storage/blocked
  cases stay.
- **Discover (`Discover.test.ts` + `LanPeers.test.ts`):** the form is inline without "Add…"; Add
  ("Add contact and connect") disabled until a nickname is typed; add-and-connect calls and both
  error texts unchanged; contacts filtered out; the Connections group is hidden when every
  connection is a contact, lists a non-contact with its `direction | state | reason`, and shows
  "Add contact…" only when connected (calls `requestNaming` with its `display_name`); hostile names
  render inert.
- **Network (`Network.test.ts`):** order switch, "Off"/"On" captions, explanation, ticket field,
  button, paragraph; no "Connections" heading and no "No connections yet."; the switch's accessible
  name is still "Visible on this network"; existing submit/self-dial/InvalidTicket/expectNaming
  tests stay. Move all four connection tests to `Discover.test.ts` and adapt them to non-contacts:
  "updates the connection list when the store changes", "renders a hostile peer id as inert text",
  "connected non-contact row offers Add contact and requests naming with its display_name", and
  "contact rows do not offer Add contact" (becomes "contact connections are not listed on
  Discover"). `PresenceToggle.test.ts`
  unchanged and green.
- **Contacts (`Contacts.test.ts`):** the edited empty-state text exactly; no LAN list.
- **ContactRow (`ContactRow.test.ts`):** shows `outbound | connected`, `inbound | failed | timeout`
  and "Not connected" from the store (existing tests stay).
- **MyInfo (`MyInfo.test.ts`):** picker present; DOM order as specified; both copy buttons copy
  exactly the ticket and the NodeId; hidden heading "My Info" exists.
- Guards from 07-01 stay green (no `<style>`, colours, web storage; `html-guard`,
  `clipboard-guard`, `render-guard`).

## Guardrails (AGENTS.md, enforced — do not weaken)

- Secret types (`SecretKeyBytes`, `DbKey`) never named under `app/` (pre-commit grep gate).
- Clipboard only in `app/src/views/MyInfo.svelte` (and tests); only `{@html}` is the QR there.
- Logs never contain plaintext, keys or secrets; logging only via `core::logging`; redaction guard.
- All persisted data only in SQLCipher `messenger.db`; `pendingTheme` stays a JS value; no
  `localStorage`/`sessionStorage`/cookies/IndexedDB/files.
- `core` forbids unsafe; this step changes no `core` code (the only Rust change is an `app` test).
- CSP unchanged; no web fonts, CDN, remote images or new origins; icons are inline SVG written in
  the component with `currentColor`.
- Wire format unchanged; no `PROTOCOL_VERSION` bump.
- Tests use in-memory doubles, never the real keychain; `headless`/`FileKeychain` never referenced
  from `app/`.
- No empty files/folders, stub modules or placeholders; no component or export without a real use.
- The pre-commit clipboard gate greps `writeText|clipboard` over all of `app/src` except
  `*.test.ts` — including `app.css` and comments — so never write those words anywhere under
  `app/src` except `MyInfo.svelte` and tests. Likewise `render-guard` forbids the substrings
  `location`, `href`, `<a` and `window.open` in its guarded files, comments included.
- Peer-controlled strings (self-chosen names, nicknames, `reason`) are text interpolation only,
  wrapped in `<bdi>`/`dir="auto"`, never in `href`, `src`, `style`, `title`, a class, an `id`, a
  `form=` attribute or `{@html}`.
- Never write credentials or secrets in code; all credentials must be provided at runtime (none
  are needed for this work).

## Security review

| Risk | Prevention | Verify |
|---|---|---|
| Trust prompt hidden by navigation (clickjacking-like: user acts on a screen while a request waits unseen) | prompts rendered by the shell above every screen, including the chat screen | shell test "both prompts on every tab and on the chat screen" |
| Setup overwriting another instance's identity name/theme | `AlreadyProvisioned` → `onDone()` with no writes; provision never repeated on retry | setup tests; read the submit function |
| Half-created state from a failing theme save | theme save is last and best effort; never blocks `onDone` | "theme-save failure still calls onDone" |
| Self-chosen LAN name passed off as identity | name only inside "calls themselves “…”" / the "(chosen by them, not verified)" sentence, next to the Unverified badge, with `NodeIdLine` beside it; never a heading | `LanPeers.test.ts` "always shows Unverified", "renders HTML and bidi names inert" |
| Peer strings in attributes / DOM ids | no ids/`form=` built from NodeIds or names; Add stays the form's own submit | `grep -nE "id=\{|form=\{" app/src/views/LanPeers.svelte app/src/views/Discover.svelte` → empty |
| Adding a contact with an empty nickname | Add disabled until trimmed non-empty (NicknameForm) | Discover test |
| Stale conversation after leaving a chat | conversation cleared when the screen becomes a tab | chatStore test |
| Loss of connection visibility / "Add contact…" between steps | both new homes added in the same change that removes the Network list | Discover + ContactRow tests; manual check |
| Theme stored outside the DB / CSP widened | as 07-01 | `web-storage-guard.test.ts`; `git diff 07-visual-identity-and-ux-redesign -- app/src-tauri/tauri.conf.json` → empty |

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cd app && npm ci && npm test && npm run check && npm run build
```
Commit through the pre-commit hook (`git config core.hooksPath .githooks`); never `--no-verify`.
Manual: `npm start` with a fresh `--profile`: setup in Midnight, pick Light (repaints instantly),
the arrow appears only after typing, Continue lands on Contacts in Light; relaunch opens Contacts in
Light; keyboard-only menu arrows and picker arrows; two instances (README "Two-device simulation"):
the peer appears under Discover, nickname + "Add contact and connect" adds and connects, the row
shows `outbound | connected`; a pending incoming request stays visible on every tab and in a chat.

## Definition of done

- [ ] `screen` store replaces `activeTab`/`activeChat`; `router.ts` imports no components;
      `openChat`/`closeChat` delegate navigation; focus management as specified.
- [ ] Shell: Contacts | Discover | Network | My Info, roving tabindex and arrows, footer on tab
      screens only, menu hidden on the chat screen, both prompts on every screen, hidden headings.
- [ ] Setup screen as specified (conditional arrow "Continue", provision → name → theme → Contacts,
      retry without re-provisioning, `AlreadyProvisioned` writes nothing); `first_run_flow_end_to_end`
      has the theme step.
- [ ] Discover hosts the LAN list (inline nickname + Add, no "Add…") and the non-contact Connections
      group; Network re-laid out with its Connections list removed; contact rows show the connection
      line; My Info re-laid out.
- [ ] Text: only the Contacts empty-state edit is made to existing text; new strings are only
      "Discover", "p2p-messenger", "Choose a theme", "Off", "On" (and 07-01's picker labels).
- [ ] All tests above added/updated and green; 07-01 guards green.
- [ ] README paths updated for the moved places.
- [ ] The app is usable at the end of this step and every screen is reachable: setup, all four tabs,
      the chat (from a contact row's Message button) and Back, every dialog and both prompts, the
      error/storage screens; every action that existed before still has a home.
- [ ] Pre-commit hook passes; PR into the DD branch per the template (`app` only; wire format
      unchanged, no `PROTOCOL_VERSION` bump; how verified).

## Open questions

1. **Menu on drill-in screens** — resolved: hidden (user decision, see above).
2. **Arrow keys vs. "focus moves on every screen change".** Moving focus to the hidden heading on
   arrow-key tab changes would break keyboard navigation of the tablist; this step keeps focus on the
   tab for arrow keys only.
3. **Connections move timing** (deviation from the DD's PR split, explained in the TL;DR).
4. **`Discover.svelte`** is a new file the DD's "Affected" list does not name.
5. **Emphasis in the edited empty state:** the DD table marks "Discover" bold-italic as the
   inserted word; this step keeps the existing `<em>` markup (`<em>Discover</em>`).
6. **`AlreadyProvisioned` and the theme:** the DD says it skips writing the name; this step also
   skips the theme write, for the same reason (never overwrite another instance's settings).
