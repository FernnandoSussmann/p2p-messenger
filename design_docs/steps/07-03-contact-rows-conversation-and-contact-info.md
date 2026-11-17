# Step 07-03 — Contact rows, conversation redesign, contact info screen, README; wireframes removed

**Size:** L — about +1,500 / −750 lines. Rust: none. Frontend ≈ +650 / −400 (new
`ContactInfo.svelte` ≈ +250, `ContactRow.svelte` ≈ −100 / +50, `Chat.svelte` ≈ ±150,
`MessageBubble.svelte` / `Composer.svelte` ≈ ±90, router/shell/stores ≈ +50, `app.css` ≈ +120).
Tests ≈ +650 / −300. README ≈ ±250. Deletes the 7 PNGs in `design/wireframes/`.

## TL;DR

Third of four sequential steps for [DD-07](../07-visual-identity-and-ux-redesign.md) (PR 3 of
"Suggested implementation order"; the Connections move already happened in 07-02). After this step
the feature is complete:

1. **Contact rows**: the whole row is one button opening the conversation — trust badge, nickname,
   `direction | state | reason` or "Not connected", and an "N unread" chip; no action buttons.
2. **Contacts tab**: list of those rows; the removal notice comes from a small store so contact info
   can post "Removed …" after navigating back.
3. **Conversation**: header Back / name / badge / connection text / ⋮ "Contact info"; "You" on
   outgoing bubbles; *sent | received* `<time>`s on incoming bubbles; slim auto-growing composer
   (placeholder "Type anything…", arrow Send); the log fills the height. Search and Clear history
   move out.
4. **New `ContactInfo.svelte`** (`contact-info` screen, Back → conversation): pencil rename, trash
   remove → Contacts with notice, `NodeIdLine`, self-name sentence, verify/unverify, connection +
   Connect, Clear history (+ cache reset), search.
5. Removed contacts are never displayed: a drill-in screen whose contact disappears returns to
   Contacts (user decision, see below); `render-guard` extended to five files with an
   exact count; the second allowed text edit; README rewritten for themes, screens, the contact-info
   path and the manual checklist; **`design/wireframes/` deleted** with every link to it.

DD-08 (outgoing delivered time) builds on the bubble layout made here — **do not implement DD-08**.

## Mandatory process rules

- After each step you implement and after each subagent execution completes, run the /usage-guard skill (Skill tool, skill: "usage-guard") to check plan usage; if it says to wait, wait until the limit resets before continuing. Pass this exact instruction on to every subagent you create, including subagents' subagents.
- If in doubt, ask; do not assume.
- Re-read DD-07 Part C, the Text inventory, "Security and privacy" and Tests before starting.
- Never write credentials or secrets in code, tests, fixtures or docs. Every credential must be
  provided at runtime; this work needs none.

## Prerequisites and branch

Steps are strictly sequential: **07-01 → 07-02 → 07-03 → 07-04**. 07-02 must be merged into the DD
branch first.

1. `git fetch origin --prune`; find the last worked branch for this DD (do not assume `main` is
   current): `git branch -a --sort=-committerdate | head`, `git log --oneline --all -15`,
   `gh pr list --state all --limit 10`. Normally `07-visual-identity-and-ux-redesign` with 07-02
   merged; if 07-02's PR is still open, stop and ask.
2. `git checkout 07-visual-identity-and-ux-redesign && git pull --ff-only`, then
   `git checkout -b step-07-03-contact-rows-conversation-and-contact-info`.
3. Hook installed: `git config core.hooksPath .githooks`.
4. PR into `07-visual-identity-and-ux-redesign` per `.github/PULL_REQUEST_TEMPLATE.md`.

## Resolved decision: removed contacts are not displayed (answered by the user)

DD-07 disabled every control except Back on a removed contact's drill-in screens, which dropped
today's "Clear history… of a removed contact". **The user decided instead:** when a contact is
removed and their messages are kept (not erased), **the app must not display that conversation at
all.** The messages stay in `messenger.db`; the conversation shows again only if the user re-adds
the same contact (same NodeId). Consequences for this step:

- **No removed-contact view.** If the contact disappears from `$contacts` while its `chat` or
  `contact-info` screen is open (removed here or elsewhere), navigate to Contacts
  (`showTab("contacts")`, which clears the cached conversation per 07-02). The text "This contact
  was removed." is **removed** from the app (add it to the DD's Text inventory "Removed" rows), and
  no control is ever rendered for a removed contact, so no command can be sent for one.
- **No Clear history for removed contacts.** History erasure for a contact being removed stays
  available only through `RemoveContactDialog`'s explicit "Also delete our message history" option.
  README "### Chat" loses "also for removed contacts"; manual checklist item 27 is dropped/replaced
  by: "Remove a contact keeping history: their conversation is no longer shown; re-add the same
  NodeId: the old messages are back."
- **Nothing else may show a non-contact's conversation** (no row, no unread chip, no search result).
  `openChat`/`showContactInfo` are only called from a contact row / the chat header of an existing
  contact.
- **Re-add restores history:** add an `app` Rust flow test (e.g.
  `remove_keep_then_readd_restores_history_end_to_end` in `app/src-tauri/tests/integration/`) —
  send messages, `contacts_remove` without erase, `history_page` for that NodeId is not shown in
  the UI but the rows still exist; re-add the same NodeId; `history_page` returns the old messages.
  If re-adding does not bring them back today, fix it in the owning `core` slice (no wire change).
- Record this deviation from the DD in the DD-07 implementation notes (07-04), and update the DD
  text itself in this step (Screen model bullet, Security "No stale access after removal", Tests,
  Text inventory).

## Parallel vs sequential

Across steps: sequential. Inside this step:

1. **Sequential first (small): router contract.** In `lib/router.ts` add the `contact-info` kind and
   `showContactInfo(nodeId)`, extend `back()` (contact-info → chat of the same NodeId, chat →
   Contacts); in `lib/contactsStore.ts` add the `contactsNotice` store; commit. Everything below
   builds on these names.
2. Then in parallel (worktrees + sub-agents):

| Track | Owns | Notes |
|---|---|---|
| **A — Rows + Contacts** | `ContactRow.svelte`, `Contacts.svelte` (+ tests) | |
| **B — Conversation** | `Chat.svelte`, `MessageBubble.svelte`, `Composer.svelte` (+ tests) | deletes search/Clear history from Chat |
| **C — Contact info** | new `ContactInfo.svelte` (+ test), `MainScreen.svelte` (render the screen), `MainScreen.test.ts` | copy the search/clear logic from the **pre-change** `Chat.svelte` on the base branch, not from B's worktree |

3. Sequential at the end: **D — Integration**: `render-guard.test.ts` (exact count 5), the full
   suite, `README.md`, the wireframes deletion and link removal, the gate.

Conflict risks: `app.css` (each track only appends inside its own marked block: `/* contact row */`,
`/* conversation */`, `/* contact info */`); `Chat.svelte` is B's only — C must not edit it (the ⋮
button calling `showContactInfo` is B's); `MainScreen.svelte` is C's only; `contactsStore.ts` and
`router.ts` are frozen after phase 1 (ask before changing their API).

## Tasks

### Phase 1 — Router and notice store

```ts
// lib/router.ts
export type Screen =
  | { kind: "tab"; tab: TabId }
  | { kind: "chat"; nodeId: string }
  | { kind: "contact-info"; nodeId: string };
export const showContactInfo = (nodeId: string) => screen.set({ kind: "contact-info", nodeId });
// back(): contact-info(n) → chat(n); chat → Contacts
```
The conversation stays loaded while on contact info (07-02 only clears it on `tab` screens), so
Back from contact info shows the same conversation. `lib/contactsStore.ts`:
`export const contactsNotice = writable<string | null>(null);`.

### Track A — Contact rows and the Contacts tab

1. **`ContactRow.svelte`** (rewrite; props `{ contact: ContactDto }`, drop `onNotice`):

   ```svelte
   <script lang="ts">
     import { connections } from "../lib/netStore";
     import { openChat, unread } from "../lib/chatStore";
     // …TrustBadge, ContactDto imports
     let { contact }: { contact: ContactDto } = $props();
     const conn = $derived($connections[contact.node_id]);
     const unreadCount = $derived($unread[contact.node_id] ?? 0);
   </script>

   <li>
     <button type="button" class="contact-row" data-testid="contact-row" onclick={() => openChat(contact.node_id)}>
       <TrustBadge verified={contact.verified} />
       <span class="contact-name" data-testid="contact-nickname"><bdi>{contact.nickname}</bdi></span>
       <span class="muted">{#if conn}{conn.direction} | {conn.state}{#if conn.reason} | {conn.reason}{/if}{:else}Not connected{/if}</span>
       {#if unreadCount > 0}<span class="chip">{unreadCount} unread</span>{/if}
     </button>
   </li>
   ```
   - The accessible name is the button's text (nickname + visible details); Enter/Space/click open
     the conversation. Nickname ellipsised via CSS, inside `<bdi>`.
   - Hover/focus styling must not switch the row background to `--surface-2` (the reference
     `.list li:hover` does): muted text on `--surface-2` is below 4.5:1 in every theme. Use a border
     or the focus ring instead.
   - Remove: "Message"/"Message (N)", "Connect", "Rename", "Mark verified…"/"Mark unverified",
     "Remove…", `NodeIdLine`, `NicknameForm`, `VerifyContactDialog`, `RemoveContactDialog` and their
     state (all move to contact info). Update the code comment that reserved `self_name` "for the
     future contact-info screen" (and the matching comment on `ContactDto.self_name` in
     `app/src/contacts.ts`): it is now shown on contact info only.
2. **`Contacts.svelte`**: `.list` of rows; keep the error alert, the edited empty state from 07-02,
   and the `role="status"` notice now read from `$contactsNotice` (clear it in `onDestroy` so it
   shows once, after the navigation back from contact info).

### Track B — Conversation

1. **`Chat.svelte`** (props stay `{ nodeId, onBack }`):
   - **Header**: Back (`data-screen-focus`, label "Back"); nickname `<bdi>` or the existing
     `nodeId.slice(0, 8)` fallback; `TrustBadge`; `data-testid="connection-text"` "Connected" /
     "Not connected"; a ⋮ icon button (`.btn .btn-ghost .btn-icon`, `aria-label="Contact info"`,
     inline SVG `aria-hidden="true"`) calling `showContactInfo(nodeId)`.
   - **Delete** "Clear history…", `ClearHistoryDialog`, "History cleared.", the search input,
     results and their state (moved to contact info).
   - **Not-connected note** — the second allowed text edit — exactly:
     `Not connected. Messages wait as Pending and are sent automatically once you are connected again: when they connect to you, or when you use Connect in their contact info or paste their ticket in Network.`
     (Was "…or when you use Contacts → Connect or paste…".)
   - "Load older messages", `conv.error` alert, the log (`role="log"`, `aria-live="polite"`,
     stick-to-bottom, load-older-preserves-position) unchanged in behaviour; the log **fills the
     available height** (flex column: shell `100vh`, drill-in screen `flex: 1; min-height: 0`, log
     `flex: 1; overflow-y: auto`) instead of a fixed height.
   - **Removed contact** (`!contact`): navigate to Contacts immediately (`showTab("contacts")` from
     an `$effect` once the contacts store has loaded and the NodeId is absent); render nothing for
     the missing contact — no "This contact was removed.", no controls (user decision above).
2. **`MessageBubble.svelte`**:
   - Outgoing (`.bubble.out`): "You" at the top right (existing word), body, then meta: sent time
     (`<time datetime={guardedIso(sent_at_ms)}>`), the existing status label ("Sending…",
     "Delivered", "Pending: will be sent when connected", "Not delivered"), and "Resend" when
     failed (`.bubble.failed` → `--danger` border). Leave room in the meta row for DD-08's delivered
     time, but add nothing for it.
   - Incoming (`.bubble.in`): no name; body; meta `<time datetime=… title="Sent (their clock)">sent</time>
     | <time datetime=… title="Received (this device)">received</time>`, both via
     `formatLocalTime`. If `received_at_ms` is `null` or `guardedIso(received_at_ms)` is
     `undefined`, render only the sent `<time>`. Titles are these constant strings only — never
     built from a body or name.
   - Body stays `<bdi dir="auto">` text. Ordering stays `(sent_at_ms, seq)` (chatStore unchanged).
   - Meta text colour: the bubble's own text token (no `opacity`, no `--text-muted` on
     `--surface-2`; see 07-01's contrast note).
3. **`Composer.svelte`**: one-line auto-growing `<textarea class="textarea" rows="1"
   aria-label="Message" placeholder="Type anything…">` and an arrow icon button
   `aria-label="Send"` (inline SVG `aria-hidden="true"`). Ctrl/Cmd+Enter, the byte counter
   (`data-testid="byte-counter"`, `--danger` when over) and the over-limit/disabled behaviour
   unchanged. Auto-grow is the **only** allowed runtime style write: on input,
   `el.style.height = "auto"; el.style.height = `${Math.min(el.scrollHeight, MAX_PX)}px`;` — never a
   colour, never a `style=` attribute or `style:` directive.

### Track C — Contact info (`ContactInfo.svelte`) and the shell

1. **Shell**: `MainScreen.svelte` renders `<ContactInfo nodeId={$screen.nodeId} />` for
   `kind === "contact-info"` — menu and footer hidden, prompts shown, like the chat screen.
2. **`ContactInfo.svelte`** (props `{ nodeId: string }`; `contact` derived from `$contacts`), top to
   bottom:
   1. **Header**: Back (`data-screen-focus`, "Back", calls `back()` → the conversation); the
      nickname as the heading in `<bdi>` (fallback `nodeId.slice(0, 8)` when removed) with
      `TrustBadge`; a pencil icon button (`aria-label="Rename"`) and a trash icon button
      (`aria-label="Remove…"`).
      - Pencil toggles the existing `NicknameForm` inline (`initial={contact.nickname}`,
        `selfName={contact.self_name}`, `submitLabel="Save nickname"`, Cancel) and saves via
        `contactsRename` + `refreshContacts()`; errors via `describeContactsError` in the form.
      - Trash opens the existing `RemoveContactDialog` unchanged (only the trash opens it; the dialog
        keeps the explicit erase option and its guarantee text). **Snapshot the contact:**
        `RemoveContactDialog.confirm()` awaits `refreshContacts()` *before* `onDone(result)`, so by
        then `contact` (derived from `$contacts`) is already `undefined` — a dialog gated on
        `contact` would unmount, the removed-contact redirect would fire before the notice is set,
        and the nickname for the notice would be gone. So:

        ```svelte
        let removalTarget = $state<ContactDto | null>(null);   // set on the trash click
        // trash: onclick={() => (removalTarget = contact ?? null)}
        {#if removalTarget}
          <RemoveContactDialog
            contact={removalTarget}
            onCancel={() => (removalTarget = null)}
            onDone={(r) => {
              const name = removalTarget!.nickname;
              contactsNotice.set(`Removed “${name}”. Your message history with them ${r.history_erased ? "was deleted" : "was kept"}.`);
              showTab("contacts");                               // 07-02 clears the conversation
            }}
          />
        {/if}
        ```
        While `removalTarget` is set, the removed-contact redirect must not fire first (the dialog's
        `onDone` sets the notice and navigates).
   2. **Identity**: `NodeIdLine` (short NodeId, "Show full NodeId"/"Hide full NodeId", "Select it
      and use your system's copy shortcut.", full NodeId — selection only, no copy button). Below:
      "They call themselves “<bdi>X</bdi>” (chosen by them, not verified)" or "They did not send a
      name" (same strings as `NicknameForm`; extracting a tiny shared component is fine if both use
      it).
   3. **Trust**: "Mark verified…" opening the existing `VerifyContactDialog`, or "Mark unverified"
      calling `contactsSetVerified(nodeId, false)` + `refreshContacts()` (errors in `role="alert"`).
   4. **Connection**: `direction | state | reason` or "Not connected" (same rendering as the row),
      and "Connect" only when a LAN ticket is known and the contact is not connected — the exact
      condition the old row used: `lanTicket && !isConnected`, calling `netConnectTicket(lanTicket)`
      (errors via `describeNetError`).
   5. **History**: "Clear history…" → existing `ClearHistoryDialog` (Cancel autofocused). On done:
      `resetConversation(nodeId)`, clear the search query/results (bump the sequence), show "History
      cleared." (`role="status"`). Back then shows an empty chat.
   6. **Search**: the "Search this conversation" input (`type="search"`, same `aria-label` and
      placeholder) with the existing behaviour moved verbatim from Chat: `historySearch(q, nodeId)`,
      stale responses dropped by sequence number, Esc clears, errors in `role="alert"`; results list
      (`aria-label="Search results"`, `data-testid="search-results"`) showing "You"/"Them",
      `formatLocalTime(sent_at_ms)`, the body in `<bdi dir="auto">`, or "No matches.". Results live
      only in component state (cleared on navigation by unmount and on history clear).
   - **Removed contact** (no `contact`, removed elsewhere): navigate to Contacts immediately, as on
     the chat screen; nothing is rendered and no command is ever sent for a removed contact.
   - Must not contain `<a`, `href`, `window.open` or `location` anywhere (render-guard; note `<a`
     also matches `<aside`, `<abbr`, `<article>` — don't use them in the five guarded files).

### Integration (D, sequential)

1. **`app/src/render-guard.test.ts`**: glob
   `/src/views/{Chat,MessageBubble,Composer,ContactInfo,ContactRow}.svelte` and assert
   `toHaveLength(5)`; same forbidden list. Add to `stylesheet-guard.test.ts` (07-01) that the only
   `.style.` write in `/src` is `style.height` in `Composer.svelte`.
2. **Text inventory check** (`git diff 07-visual-identity-and-ux-redesign...` review): only the two
   listed edits changed existing text; every "Moved" string exists exactly once at its new home;
   "Message"/"Message (N)", "Add…", "No connections yet.", "Choose a display name", "Create
   identity", "All set" are gone; new strings are only those in the inventory ("Discover",
   "p2p-messenger", "Choose a theme", "Theme", "Midnight", "Dark", "Light", "Off", "On", "Contact
   info", "Sent (their clock)", "Received (this device)", "Type anything…", "N unread").
3. **README.md**: document the three themes (Midnight default, chosen on setup and My Info, stored
   per account in `messenger.db`, so per `--profile`), the screen structure (setup → Contacts;
   Contacts | Discover | Network | My Info; Contacts → contact → conversation → ⋮ → contact info),
   where each per-contact action now lives, and rewrite the "Manual GUI checklist" for the new paths
   using the DD's Manual list (fresh profile + Light; all three themes readable; keyboard only; two
   instances via Discover; "You" and *sent | received*; ⋮ → contact info; pencil rename; verify;
   trash → Contacts with notice; Clear history then Back is empty; stopped peer shows state/reason
   and the edited sentence; a pending request stays visible on every tab, chat and contact info).
   Keep the still-valid checklist items (storage, logs, keychain, profile, HTML-inert messages,
   16 KiB, grep of `messenger.db`). Item 27 ("Clear history… of a removed contact") is replaced by: "Remove a contact keeping
   history: their conversation is no longer shown; re-add the same NodeId: the old messages are
   back". Mention the
   reference sheet `design/visual-identity.html` as the visual spec.
   Explicitly rewrite the **"### Chat"** section: line ~163 ("Open **Contacts**, then a contact row's
   **Message** button … an unread count shows on the button") → click the contact row; the "N
   unread" chip on the row; line ~184 ("**Clear history…** (in the chat header, also for removed
   contacts)") → in contact info (drop "also for removed contacts"; say a removed contact's conversation is
   hidden and returns if the same contact is re-added); and the search
   sentence → contact info. In the checklist, rewrite items 20 (Contacts → **Message** opens the
   chat), 23 (unread count on B's Message button) and 26 (search in the chat) for the row click, the
   row's unread chip and search in contact info.
4. **Delete the wireframes**: `git rm -r design/wireframes/`. Remove every link to them: the
   `**Wireframes:** …` header line of `design_docs/07-visual-identity-and-ux-redesign.md`, and any
   link in `README.md` / `design_docs/README.md` (none existed when this step was written; check).
   `design/visual-identity.html` is **kept**. Verify:
   `git grep -n "wireframes" -- . ':!design_docs/07-visual-identity-and-ux-redesign.md' ':!design_docs/steps/07-*.md'`
   → empty (the step files are deleted in 07-04), and in the DD only prose remains (no
   `](…wireframes…)` link, no path a tool would follow): `git grep -nE "\]\([^)]*wireframes"` →
   empty; `ls design/` → only `visual-identity.html`.
5. Full suite and the gate.

## Tests

- **Rust (`app`):** new `remove_keep_then_readd_restores_history_end_to_end` (see the resolved
  decision); `chat_flow_end_to_end`, `history_clear_end_to_end` and
  `remove_with_erase_clears_chat_history_end_to_end` keep covering the commands the new screens call.
- **Router (`router.test.ts`):** contact-info Back → chat of the same NodeId; chat Back → Contacts;
  `showTab` from contact info goes to that tab and clears drill-in; focus moves to Back on both
  drill-ins (shell test).
- **Shell (`MainScreen.test.ts`):** both prompts render on contact info too; menu/footer absent there.
- **ContactRow:** badge, nickname, `direction | state | reason` or "Not connected"; the chip only
  when > 0 with "N unread" (e.g. "2 unread"); click and Enter open the chat (`openChat` called with
  the NodeId); none of Rename, Mark verified…, Mark unverified, Remove…, Connect, Message rendered;
  names inert; badge cannot be spoofed by a nickname like "Verified by you".
- **Contacts:** edited empty state; no LAN list; the removal notice from `contactsNotice` shown in
  `role="status"`.
- **Chat:** header content (Back, name, badge, connection text) and ⋮ "Contact info" navigates; no
  search box and no "Clear history…"; the edited not-connected sentence exactly; contact removed
  while the chat is open → navigates to Contacts and "This contact was removed." never renders; existing log tests
  (`renders_messages_newest_at_bottom`, `load_older_button_visible_while_cursor`, `back_calls_onBack`)
  adapted and green. Move `search_shows_results_as_text`, `clear_history_is_reachable_for_*` and
  `clear_history_empties_log_and_search` to `ContactInfo.test.ts`.
- **MessageBubble:** outgoing shows "You" + time + status (+ Resend on failure); incoming has two
  `<time>` elements with titles "Sent (their clock)" and "Received (this device)", and one when
  `received_at_ms` is `null` or invalid (e.g. `NaN`, `1e20`); existing inert-HTML/bidi/emoji tests
  stay.
- **Composer:** placeholder "Type anything…"; Send found by role name "Send"; Ctrl+Enter sends,
  Enter inserts a newline; byte counter and over-limit unchanged.
- **ContactInfo:** pencil renames via `contacts_rename`; trash → dialog → removal navigates to
  Contacts and posts the right kept/deleted notice built from the **pre-removal nickname** (mock
  `contacts_list` to return the list without the contact after `contacts_remove`; assert both the
  "was kept" and "was deleted" variants, and that the removed-contact redirect does not pre-empt the
  notice); verify dialog then `contacts_set_verified(true)`,
  and "Mark unverified" calls `contacts_set_verified(false)`; Connect only with a LAN ticket while
  disconnected (dials that ticket); Clear history shows "History cleared." and resets the cached
  conversation; search results ("You"/"Them", body inert), stale response dropped, Esc clears, error
  alert, "No matches."; both self-name variants; external removal (contact disappears from the store)
  navigates to Contacts and sends no command.
- **Guards:** `render-guard` with 5 files; `html-guard`, `clipboard-guard`, stylesheet/colour/
  contrast/drift/web-storage guards green. Existing `chatStore`, `contactsStore`, `netStore` and
  dialog tests green.

## Guardrails (AGENTS.md, enforced — do not weaken)

- Secret types (`SecretKeyBytes`, `DbKey`) never named under `app/` (pre-commit grep gate).
- Clipboard only in `MyInfo.svelte` (and tests): contact NodeIds are **selectable only**, no copy
  button on contact info. The only `{@html}` stays the QR in `MyInfo.svelte`; icons are inline SVG
  written in the components, never injected strings.
- The pre-commit clipboard gate greps `writeText|clipboard` over all of `app/src` except
  `*.test.ts` — including `app.css` and comments — so never write those words anywhere under
  `app/src` except `MyInfo.svelte` and tests. Likewise `render-guard` forbids the substrings
  `location`, `href`, `<a` and `window.open` in its guarded files (now including `ContactInfo` and
  `ContactRow`), comments included.
- Logs never contain plaintext, keys or secrets; logging only via `core::logging`.
- All persisted data only in SQLCipher `messenger.db`; search results are never cached outside the
  component; no web storage.
- `core` forbids unsafe (no `core` change in this step).
- CSP unchanged; no web fonts, CDN, remote images or new origins.
- Wire format unchanged; no `PROTOCOL_VERSION` bump. Do not implement DD-08.
- Test doubles only, never the real keychain; `headless`/`FileKeychain` never referenced from `app/`.
- No empty files/folders, stubs or placeholders; deleting `design/wireframes/` must not leave an empty
  `design/wireframes` directory.
- Peer-controlled strings (bodies, self-chosen names, nicknames, `reason`) are text interpolation
  only with `<bdi>`/`dir="auto"`; never in `href`, `src`, `style`, a class, a `title`, an id or
  `{@html}`. Self-chosen names never appear as a heading or in place of a nickname.
- Never write credentials or secrets in code; all credentials must be provided at runtime (none
  are needed for this work).

## Security review

| Risk | Prevention | Verify |
|---|---|---|
| Message bodies / names rendered as HTML or links | text interpolation in `<bdi>`; render-guard over the five files | `render-guard.test.ts` (count 5); MessageBubble/ContactInfo inert tests; `grep -n "{@html" app/src -r` → only `MyInfo.svelte` |
| Peer data in `title`/attributes (tooltip spoofing) | the bubble titles are constants; no attribute built from a body/name | `grep -nE "title=\{" app/src/views/{MessageBubble,ContactInfo,ContactRow,Chat}.svelte` → only constant/`nodeId` titles (`NodeIdLine`, `TrustBadge`) |
| Self-chosen name passed off as identity | contact-info heading is the local nickname; self name only in the "(chosen by them, not verified)" sentence | ContactInfo self-name tests |
| Stale actions after removal (commands sent for a removed contact) | successful removal navigates to Contacts; the dialog works on a snapshot (`removalTarget`), not the live derived contact; a contact removed elsewhere closes its chat/contact-info screen (redirect to Contacts); a removed contact's conversation is never rendered | ContactInfo/Chat removed-contact tests; pre-removal-nickname notice test |
| Removed confirmations on destructive actions | trash only opens `RemoveContactDialog` (explicit erase opt-in + guarantee text); Clear history keeps its dialog with Cancel autofocused | ContactInfo tests; diff of the dialogs → classes only |
| Verify made too easy / trust state hidden | verify still behind `VerifyContactDialog`; trust badge in the row, chat header and contact info, labelled text | ContactRow/Chat/ContactInfo badge tests; contrast guard |
| Timestamps over-trusted | sender time labelled "Sent (their clock)"; invalid values fall back via `formatLocalTime`/`guardedIso`; ordering unchanged | MessageBubble time tests |
| Search leaking/caching history | `history_search` scoped to the NodeId; results in component state only; cleared on navigation and clear | ContactInfo search tests |
| Composer style exception abused | only `style.height` in `Composer.svelte`, never colour | stylesheet guard `.style.` assertion |
| NodeId copied via clipboard outside My Info | selection only | `clipboard-guard.test.ts`; hook clipboard gate |
| Wireframes still referenced | deletion + link removal | the `git grep` commands in Integration step 4 |

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cd app && npm ci && npm test && npm run check && npm run build
```
Commit through the pre-commit hook (`git config core.hooksPath .githooks`); never `--no-verify`.
Manual (two instances, README "Two-device simulation"): run the new README checklist end to end in
at least Midnight and Light, keyboard only for one pass.

## Definition of done

- [ ] Contact rows redesigned with no action buttons; Contacts tab with the notice store.
- [ ] Conversation redesigned (header with ⋮, "You", *sent | received*, composer with placeholder and
      arrow Send, log fills height); search and Clear history moved out.
- [ ] `ContactInfo.svelte` with every per-contact action, reusing existing commands and dialogs;
      Back chain contact-info → chat → Contacts; removal returns to Contacts with the right notice.
- [ ] Removed contacts never displayed: both drill-in screens redirect to Contacts; history kept in
      the DB and shown again after re-adding the same NodeId (Rust flow test).
- [ ] Text inventory matches exactly; the two listed edits are the only edits to existing text.
- [ ] `render-guard` covers Chat, MessageBubble, Composer, ContactInfo, ContactRow with an exact
      count; all other guards green.
- [ ] All tests above green; README updated (themes, screen structure, contact-info path, manual
      checklist).
- [ ] `design/wireframes/` deleted; no Markdown link, import or code reference points into it.
- [ ] The app is usable at the end of this step and every screen is reachable: setup, all four
      tabs, conversation (row click), contact info (⋮), every dialog (rename form, verify, remove,
      clear history), both prompts on every screen, error/storage screens; every action that existed
      before DD-07 has a home (except Clear history for removed contacts, dropped by user decision).
- [ ] Pre-commit hook passes; PR into the DD branch per the template (`app` + docs; wire format
      unchanged, no `PROTOCOL_VERSION` bump; how verified).

## Open questions

1. **Removed contacts** — resolved: never displayed; history kept and restored on re-add (user
   decision, see above).
2. **Separators.** The DD sketches use `│` and `·`; this step uses ` | ` in text for
   `direction | state | reason` and *sent | received* (screen-reader friendly) and `·` only as a
   decorative `aria-hidden` separator in the outgoing meta. Ask if different glyphs are wanted.
3. **Wireframe mentions in DD-07's prose** (Affected list, "Removing the wireframes", Definition of
   done) are kept as history; only the header line pointing to the folder is removed. Ask if the
   prose should be reworded too.
