# Step 03-02 — Contact commands, contact list UI and all add/manage flows

**Design doc:** [DD-03 Contacts & Trust](../../03-contacts-and-trust.md)

**Depends on: 03-01 complete on its branch (sequential).** 03-01 will usually
**not** be merged into `main` yet: the DD-03 branches are stacked
(03-01 → 03-02 → 03-03). This step uses the following from 03-01:

- core: `core::contacts`, `ContactBook`, `ContactsState`, Accept-with-nickname,
  dial auto-add, `Node::last_seen_name`, `Node::disconnect`;
- app test helpers: `crate::test_util::{Started, started_with, …}`;
- frontend: `NicknameForm.svelte` (with its `initial` prop) and `nicknameMessages.ts`;
- the Accept prompt prefilled from `IncomingRequestDto.suggested_nickname`.

**Blocks:** 03-03 (audit).
**Branch:** `step-03-02-contact-commands-and-contact-list-ui`, created **from
`step-03-01-contacts-slice-and-trust-gate`** (never from `main`):

```sh
git fetch origin
git switch step-03-01-contacts-slice-and-trust-gate
git pull --ff-only     # pick up any review fixes pushed to 03-01
git switch -c step-03-02-contact-commands-and-contact-list-ui
git log --oneline main..HEAD   # must list the 03-01 commits; core/src/contacts/ must exist
```

If 03-01 has already been merged by the time this step starts, branch from its
merge target instead.
**PR base:** `step-03-01-contacts-slice-and-trust-gate` (stacked PR). The
sub-task worktrees (2A/2B/2C) branch from this step's branch and merge back
into it, never into `main`.

## Goal

When this step is done, the whole DD-03 feature works end to end.

**Backend**

- The Tauri commands `contacts_list`, `contacts_add`, `contacts_rename`,
  `contacts_set_verified`, `contacts_remove` (with `clear_history`) and
  `contacts_is_known` are registered and tested at command level.

**Contacts tab**

- **Contacts** is the first and default tab. Each row shows:
  - the **local nickname in bold**;
  - the **NodeId small and gray beneath it**: truncated, with the full value on
    hover (`title`) and on expand as selectable text;
  - an **Unverified** badge that peer data cannot suppress, or **"Verified by
    you"**.
- **User decision B:** rows show **only** the nickname. The peer's
  self-asserted name is not rendered in the list. It is stored as `self_name`,
  shown for context in the Accept and naming prompts, and kept for a future
  contact-info screen (code comment only, no placeholder).
- Row actions:
  - **Connect**: shown when the contact is on the LAN and not connected.
  - **Rename**.
  - **Mark verified…**: an honest attestation dialog. Once verified, the action
    becomes **Mark unverified**.
  - **Remove…**: a confirmation with an opt-in, default-off "Also delete our
    message history" checkbox. Its copy makes both outcomes unmistakable.
- **On this network** (inside the Contacts tab) lists LAN peers that are **not**
  contacts. They are always badged Unverified. **Add…** goes nickname →
  contact → connect.

**Ticket path (user decision E)**

- After a pasted-ticket dial connects, the backend has already auto-added the
  peer as an Unverified contact with the default nickname.
- A global **"Name this contact"** prompt then appears. It is prefilled with
  that default and shows the self-asserted name for context.
- **Save** keeps or renames the contact. **Not now** keeps the auto-added
  contact.
- In the rare case the auto-add failed (store full or unavailable), Network →
  Connections offers **Add contact…** for that connected non-contact.

**Other**

- Inbound Accept (from 03-01) refreshes the contact list.
- No "Open chat" action yet. That is DD-04, and no placeholder button is added.

### NodeId copy: user-accepted deviation from DD-03 (decision A)

DD-03 asks for "copy" on the NodeId. AGENTS.md permits `writeText` and
`clipboard` **only** in `app/src/views/MyInfo.svelte`. This is enforced by
`.githooks/pre-commit` and `app/src/clipboard-guard.test.ts`, which both match
the bare word `clipboard`, even in comments.

**The user decided to keep that rule.** So there is no copy button and no
clipboard plugin import outside `MyInfo.svelte`. Instead, `NodeIdLine.svelte`
provides:

1. the truncated NodeId, with the full value in `title`;
2. a **Show full NodeId** toggle (`aria-expanded`) that reveals the full value
   in a `<code>` with `user-select: all`. The user then copies it with the OS
   shortcut;
3. a hint, "Select it and use your system's copy shortcut.", which must
   **not** contain the word "clipboard".

Record this as a **user-accepted deviation** in the PR. 03-03 records it in
DD-03's status.

---

## Work breakdown and parallelism

```
 2A backend commands (Rust) ─────────────────────────────────────────┐
                                                                      ├─▶ 2D integration (sequential)
 2.0 shared frontend contract (sequential) ──┬─▶ 2B list & manage UI ─┤
                                             └─▶ 2C add-flow UI      ─┘
```

- **2A** can start immediately and runs in parallel with everything else. It is
  Rust only.
- **2.0** is a short sequential sub-step, committed on the step branch
  **before** the 2B and 2C worktrees are forked.
- **2B and 2C run in parallel** after 2.0. They code against the TypeScript
  contract below, with `invoke` mocked in Vitest. They do not need 2A.
- **2D** runs after 2A, 2B and 2C are merged.

### File ownership

| Sub-task | Owns (create / edit) | Reads only |
|----------|----------------------|-----------|
| 2A | `app/src-tauri/src/contacts_cmd.rs`; `app/src-tauri/src/lib.rs` (the `generate_handler!` list and its imports only) | `core/**`, `app/src-tauri/src/test_util.rs` (helpers moved there in 03-01), `net_cmd.rs` |
| 2.0 | `app/src/contacts.ts`; `app/src/lib/contactsStore.ts` (+ `.test.ts`); `app/src/lib/nodeId.ts` (+ test); `app/src/views/NodeIdLine.svelte` (+ test); `app/src/views/TrustBadge.svelte` (+ test); `app/src/html-guard.test.ts` | — |
| 2B | `views/Contacts.svelte`, `views/ContactRow.svelte`, `views/VerifyContactDialog.svelte`, `views/RemoveContactDialog.svelte` (each with its `.test.ts`); `lib/router.ts`; `views/MainScreen.svelte`; `views/MainScreen.test.ts` | 2.0 files, `LanPeers.svelte` (mounted, not edited), `lib/netStore.ts` |
| 2C | `views/LanPeers.svelte`, `views/Network.svelte`, `views/NamingPrompt.svelte` (new), `views/IncomingRequestPrompt.svelte` (each with its `.test.ts`) | 2.0 files, `lib/netStore.ts` |
| 2D | `views/MainScreen.svelte` + test (mount `<NamingPrompt />`), `README.md`, integration fix-ups | — |

**2A makes no `core` changes.** If 2A finds that core is missing something,
stop. Raise it in the PR and fix it in a follow-up, or in 2D if it is
mechanical, keeping core's guardrails. Do not edit `core/**` from the 2A
worktree.

---

## Sub-task 2A — Tauri contact commands (parallel)

Extend `app/src-tauri/src/contacts_cmd.rs`, which 03-01 created with
`ContactsState`. Follow the existing pattern: plain functions over the state,
plus thin `#[tauri::command]` wrappers. Every function that touches the store
runs in `tauri::async_runtime::spawn_blocking` because it does file I/O. A
`JoinError` maps to `StoreUnavailable`.

```rust
/// Webview-facing error. Never embeds names, paths or backend details.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ContactsCmdError {
    NotReady, InvalidNodeId, SelfContact, InvalidNickname { kind: DisplayNameKind },
    AlreadyExists, NotFound, Full, StoreUnavailable, HistoryEraseFailed,
}
impl From<ContactError> for ContactsCmdError { /* exhaustive; Unavailable → StoreUnavailable */ }

/// Rejects input longer than 128 bytes before parsing; anything else → InvalidNodeId.
fn parse_node_id(raw: &str) -> Result<NodeId, ContactsCmdError>;

pub async fn list(c: &ContactsState) -> Result<Vec<ContactDto>, ContactsCmdError>;
/// `self_name` from `net.node()?.last_seen_name(&peer)` (Hello, pending prompt
/// or LAN), never from the webview; None if the node is down.
pub async fn add(c: &ContactsState, net: &NetState, node_id: &str, nickname: &str) -> Result<ContactDto, ContactsCmdError>;
pub async fn rename(c: &ContactsState, node_id: &str, nickname: &str) -> Result<ContactDto, ContactsCmdError>;
pub async fn set_verified(c: &ContactsState, node_id: &str, verified: bool) -> Result<ContactDto, ContactsCmdError>;
/// `clear_history: None` ⇒ Keep. After a successful removal, if the node
/// runs, `node.disconnect(&peer)` so revocation takes effect now.
pub async fn remove(c: &ContactsState, net: &NetState, node_id: &str, clear_history: Option<bool>) -> Result<RemoveContactDto, ContactsCmdError>;
/// Same `ContactBook::is_known` the gate uses. A store error is StoreUnavailable, never false.
pub async fn is_known(c: &ContactsState, node_id: &str) -> Result<bool, ContactsCmdError>;
```

All commands take `contacts: State<'_, Arc<ContactsState>>`. They are `async`,
so they run off the main thread.

| Command | Args (Rust) | JS invoke args | Returns |
|---------|-------------|----------------|---------|
| `contacts_list` | — | — | `ContactDto[]` |
| `contacts_add` | `node_id: String, nickname: String` | `{ nodeId, nickname }` | `ContactDto` |
| `contacts_rename` | `node_id: String, nickname: String` | `{ nodeId, nickname }` | `ContactDto` |
| `contacts_set_verified` | `node_id: String, verified: bool` | `{ nodeId, verified }` | `ContactDto` |
| `contacts_remove` | `node_id: String, clear_history: Option<bool>` | `{ nodeId, clearHistory }` | `RemoveContactDto` |
| `contacts_is_known` | `node_id: String` | `{ nodeId }` | `boolean` |

Register all six commands in `app/src-tauri/src/lib.rs` `generate_handler![…]`.

**Tests** go in `#[cfg(test)] mod tests` inside `contacts_cmd.rs`. They use
`crate::test_util::{started, started_with, events, expect, is_state,
is_incoming, ticket_of, Started}`, which 03-01 moved there. `Started` carries
`contacts: Arc<ContactsState>` over `MemoryContactStore` and a
`RecordingHistoryEraser`.

| Test | Asserts |
|------|---------|
| `contacts_commands_not_ready_before_bind` | every command returns `NotReady` |
| `contacts_errors_serialize_with_type_tag` | each variant serializes to `{"type":…}`, and `InvalidNickname` also carries `"kind"` |
| `contact_dto_json_shape` | `ContactDto` keys are exactly `added_at_ms, last_seen_ms, node_id, nickname, self_name, verified`; `RemoveContactDto` keys are exactly `history_erased, node_id` |
| `contacts_add_then_list_shows_unverified` | the new contact has `verified: false` and the trimmed nickname |
| `contacts_add_takes_self_name_from_the_node_not_args` | B advertises `"Bob"` on the LAN; A adds B as `"Bobby"`; `self_name` is `"Bob"`; no parameter can set it |
| `contacts_add_without_running_node_has_no_self_name` | |
| `contacts_add_rejects_malformed_node_id` | `""`, `"x"`, `"\u{202E}"`, `"f".repeat(4096)` and a valid id with a trailing space all return `InvalidNodeId`, and nothing is stored |
| `contacts_add_rejects_self` | returns `SelfContact` |
| `contacts_add_invalid_nickname_kinds` | Empty, ControlCharacter, BidiOverride (`"a\u{2067}b"`), **InvisibleCharacter** (`"a\u{200B}b"`, `"a\u{FEFF}b"`) and TooLong each map to the matching kind |
| `contacts_add_existing_is_already_exists` | covers both a user-added contact and one auto-added by a dial |
| `contacts_rename_validates_and_keeps_node_id_and_self_name` | |
| `contacts_set_verified_round_trip_and_not_found` | |
| `contacts_remove_defaults_to_keeping_history` | for `None` and `Some(false)`: the recorder is empty and `history_erased` is false |
| `contacts_remove_with_clear_history_erases_then_removes` | |
| `contacts_remove_history_failure_keeps_contact` | |
| `contacts_remove_unknown_is_not_found` | |
| `contacts_remove_drops_live_connection_and_next_inbound_prompts` | B is a contact and connected; after `remove`, B sees `Disconnected`, and B redialing raises exactly one `incoming_request` |
| `contacts_is_known_tracks_add_and_remove` | malformed input returns `InvalidNodeId` |
| `contacts_is_known_store_failure_is_error_not_false` | |
| `contacts_are_namespaced_by_bound_account` | two `ContactsState` over one store, bound to different accounts, stay isolated |
| `contact_dtos_never_contain_key_material` | `assert_forms_absent` |

---

## Sub-task 2.0 — shared frontend contract (sequential, before 2B and 2C)

### `app/src/contacts.ts`

```ts
import { invoke } from "@tauri-apps/api/core";
import type { DisplayNameKind } from "./identity";
import { nicknameMessages } from "./lib/nicknameMessages";

export interface ContactDto {
  node_id: string;
  nickname: string;
  /** Not rendered in the contact list (user decision). For the naming prompt
   *  and a future contact-info screen. */
  self_name: string | null;
  verified: boolean;
  added_at_ms: number;
  last_seen_ms: number | null;
}
export interface RemoveContactDto { node_id: string; history_erased: boolean; }
export type ContactsCmdError =
  | { type: "NotReady" } | { type: "InvalidNodeId" } | { type: "SelfContact" }
  | { type: "InvalidNickname"; kind: DisplayNameKind } | { type: "AlreadyExists" }
  | { type: "NotFound" } | { type: "Full" } | { type: "StoreUnavailable" } | { type: "HistoryEraseFailed" };

export const contactsList = () => invoke<ContactDto[]>("contacts_list");
export const contactsAdd = (nodeId: string, nickname: string) => invoke<ContactDto>("contacts_add", { nodeId, nickname });
export const contactsRename = (nodeId: string, nickname: string) => invoke<ContactDto>("contacts_rename", { nodeId, nickname });
export const contactsSetVerified = (nodeId: string, verified: boolean) => invoke<ContactDto>("contacts_set_verified", { nodeId, verified });
export const contactsRemove = (nodeId: string, clearHistory: boolean) => invoke<RemoveContactDto>("contacts_remove", { nodeId, clearHistory });
export const contactsIsKnown = (nodeId: string) => invoke<boolean>("contacts_is_known", { nodeId });
export function describeContactsError(err: unknown): string;
```

`describeContactsError` copy (exact):

| Error | Message |
|-------|---------|
| NotReady | "Contacts are not available yet. Try again in a moment." |
| InvalidNodeId | "That is not a valid NodeId." |
| SelfContact | "That's you. You can't add yourself as a contact." |
| InvalidNickname | `nicknameMessages[kind]` |
| AlreadyExists | "This person is already a contact." |
| NotFound | "That contact no longer exists." |
| Full | "Your contact list is full." |
| StoreUnavailable | "Contacts could not be saved right now. Try again." |
| HistoryEraseFailed | "Your message history could not be deleted, so the contact was not removed. Try again, or remove them without deleting history." |
| default | "Something went wrong." |

### `app/src/lib/contactsStore.ts`

```ts
export const contacts = writable<ContactDto[]>([]);
export const contactIds = derived(contacts, (c) => new Set(c.map((x) => x.node_id)));
export const contactsError = writable<string | null>(null);
export async function refreshContacts(): Promise<void>;     // on error: keep the last list, set contactsError

export interface NamingRequest {
  node_id: string;
  self_name: string | null;   // context only
  nickname: string | null;    // current (auto-added default) nickname to prefill; null if no contact exists
}
export const namingQueue = writable<NamingRequest[]>([]);

/** Called after `netConnectTicket` resolves with the dialed node_id.
 *  Race-safe: if an outbound `connected` for that node arrived in the last
 *  RECENT_OUTBOUND_TTL_MS (60 s) it is handled immediately from the buffer;
 *  otherwise the node is remembered as expected (dropped after 120 s or on
 *  an outbound `failed`). */
export function expectNaming(nodeId: string): void;
export async function requestNaming(nodeId: string, selfName: string | null): Promise<void>; // refreshes, then enqueues (dedup by node_id) with the contact's nickname or null
export function dropNaming(nodeId: string): void;
/** connection_state handler:
 *  - outbound + connected: record in a bounded buffer (≤ 32 entries, 60 s TTL);
 *    if expected → requestNaming(node_id, display_name) and forget it
 *  - outbound + failed + expected → forget it
 *  - any connected → refreshContacts() */
export function onConnectionState(c: ConnectionStateDto): void;
export async function startContacts(): Promise<() => void>;  // listen("connection_state") + initial refresh; returns unlisten
```

### `app/src/lib/nodeId.ts`

`shortNodeId(id)` returns `id` unchanged when it has at most 20 characters.
Otherwise it returns `` `${id.slice(0, 10)}…${id.slice(-6)}` ``.

### `app/src/views/NodeIdLine.svelte`

Props: `{ nodeId: string }`.

```svelte
<div class="node-id-line">
  <small class="node-id" title={nodeId} data-testid="node-id-short">{shortNodeId(nodeId)}</small>
  <button type="button" class="link" aria-expanded={open} onclick={() => (open = !open)}>
    {open ? "Hide full NodeId" : "Show full NodeId"}</button>
  {#if open}
    <code class="selectable" data-testid="node-id-full">{nodeId}</code>
    <small class="hint">Select it and use your system's copy shortcut.</small>
  {/if}
</div>
<style> .node-id { color: gray; font-size: 0.8em; } .selectable { user-select: all; word-break: break-all; } </style>
```

### `app/src/views/TrustBadge.svelte`

Props: `{ verified: unknown }`. It fails closed: only the strict boolean `true`
counts as verified.

```svelte
{#if verified === true}
  <span class="badge verified" data-testid="badge-verified"
        title="You marked this contact as verified. The app did not check this cryptographically.">Verified by you</span>
{:else}
  <span class="badge unverified" data-testid="badge-unverified"
        title="Not verified: you have not confirmed this NodeId belongs to them.">Unverified</span>
{/if}
```

Style the badge as a bordered pill with `flex-shrink: 0; white-space: nowrap`.
The unverified pill uses a high-contrast warning colour. The badge never
receives props or slots carrying peer data.

### `NicknameForm.svelte` (from 03-01; read-only here)

03-01 already gives it an optional, always-editable `initial` prop. In this
step, three callers prefill:

- rename (2B), with the user's own nickname;
- the naming prompt (2C), with the stored default nickname (decision E);
- the Accept prompt (done in 03-01), with the backend `suggested_nickname`
  (decision E, extended to Accept).

**LAN Add stays empty**, because the user decisions did not extend the prefill
to it. A cleared field blocks submit everywhere.

### `app/src/html-guard.test.ts`

Using `import.meta.glob("/src/**/*.svelte", { query: "?raw" })`, assert that
the only file containing `{@html` is `/src/views/MyInfo.svelte`.

### 2.0 tests

**`nodeId.test.ts`**
- short ids are returned unchanged;
- a 64-character id is truncated;
- the 20/21-character boundary.

**`NodeIdLine.test.ts`**
- `shows a truncated id with the full id in title`;
- `Show full reveals the whole id as selectable text`;
- `renders a hostile id inert`.

**`TrustBadge.test.ts`**
- `Unverified for false, undefined, null, "true", 1`;
- `Verified by you only for true`;
- `verified copy disclaims cryptographic checking`.


**`contactsStore.test.ts`** (with `listen` and `invoke` mocked)
- `refreshContacts fills the store`;
- `refresh failure keeps the old list and sets an error`;
- `expected outbound connected enqueues naming prefilled with the contact's nickname`;
- **`connected event that arrives before expectNaming is still handled`**: emit
  the event first, then call `expectNaming`, and the queue has the entry;
- `buffered events older than the TTL are ignored`;
- `inbound connected never enqueues naming`;
- `unexpected outbound connected does not enqueue`;
- `failed outbound forgets the expectation`;
- `requestNaming with no contact enqueues nickname null`;
- `requestNaming dedups by node_id`;
- `any connected triggers a refresh`;
- `startContacts unlistens`.

---

## Sub-task 2B — contact list and management UI (parallel with 2C, after 2.0)

### Mounting and reachability

- **`lib/router.ts`**:
  - `TabId = "contacts" | "my-info" | "network"`;
  - Contacts is the first entry in `tabs`;
  - `activeTab` defaults to `"contacts"`.
- **`views/MainScreen.svelte`**: `onMount` starts both `startNetEvents()` and
  `startContacts()`, and unlistens both on destroy. The `<NamingPrompt />`
  mount is added later, in 2D.
- **`views/MainScreen.test.ts`**:
  - `vi.mock("../lib/contactsStore", …)` up front, with `startContacts`
    resolving to an unlisten spy;
  - update the tab cases: three tabs, with Contacts first and selected by
    default;
  - add `starts and stops the contacts listener`.

### `views/Contacts.svelte`

```svelte
<section>
  <h2>Contacts</h2>
  {#if $contactsError}<p role="alert">{$contactsError}</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if $contacts.length === 0}
    <p>No contacts yet. Add someone from <em>On this network</em> below, connect with a ticket
       in the Network tab, or accept an incoming request.</p>
  {:else}
    <ul class="contacts">{#each $contacts as c (c.node_id)}<ContactRow contact={c} onNotice={(m) => (notice = m)} />{/each}</ul>
  {/if}
  <LanPeers />   <!-- "On this network": reworked by 2C -->
</section>
```

The component calls `onMount(refreshContacts)`. The list is keyed by
`node_id`, never by name.

### `views/ContactRow.svelte`

Props: `{ contact: ContactDto; onNotice: (msg: string) => void }`. The
component reads `$lanPeers` and `$connections` from `lib/netStore` and derives:

- `lanTicket = $lanPeers.find(p => p.node_id === contact.node_id)?.ticket`;
- `isConnected = $connections[contact.node_id]?.state === "connected"`.

```svelte
<!-- Only the local nickname is shown here (user decision). contact.self_name is
     intentionally not rendered; it belongs to the future contact-info screen. -->
<li data-testid="contact-row" class="row">
  <div class="title">
    <TrustBadge verified={contact.verified} />
    <strong class="nickname" data-testid="contact-nickname"><bdi>{contact.nickname}</bdi></strong>
  </div>
  <NodeIdLine nodeId={contact.node_id} />
  <div class="actions">
    {#if lanTicket && !isConnected}<button type="button" onclick={connect}>Connect</button>{/if}
    <button type="button" onclick={…}>Rename</button>
    {#if contact.verified === true}
      <button type="button" onclick={unverify}>Mark unverified</button>
    {:else}
      <button type="button" onclick={() => (verifying = true)}>Mark verified…</button>
    {/if}
    <button type="button" onclick={() => (removing = true)}>Remove…</button>
  </div>
  {#if renaming}<NicknameForm initial={contact.nickname} selfName={contact.self_name} submitLabel="Save nickname" … />{/if}
  {#if verifying}<VerifyContactDialog {contact} … />{/if}
  {#if removing}<RemoveContactDialog {contact} … />{/if}
</li>
```

Behaviour:

- **Layout.** `.title { display: flex; gap: .5em; min-width: 0 }` and
  `.nickname { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0 }`.
  The badge comes first and never shrinks.
- **Rename form.** Its context line shows `self_name`. This is the only place
  `self_name` appears from the contact list, and only inside the explicit
  rename form.
- **Connect** calls `netConnectTicket(lanTicket)`. Because the peer is a
  contact, it reconnects without a prompt on our side. Failures show
  `describeNetError`.
- **Rename** calls `contactsRename`, then `refreshContacts()`. Errors show
  inline.
- **Unverify** calls `contactsSetVerified(id, false)` directly, without a
  confirmation.

### `views/VerifyContactDialog.svelte`

Props: `{ contact; onDone; onCancel }`, with `role="dialog"` and
`aria-label="Mark contact as verified"`. Exact copy:

> **Mark “{nickname}” as verified?**
> Only do this if you have confirmed, through a channel you trust (for example
> in person, or a ticket they handed you directly), that this NodeId belongs to
> the person you call “{nickname}”:
> `{full node_id}` (selectable `<code>`)
> This is your own statement. The app does not check it cryptographically and
> cannot tell if you were given the wrong ticket. You can undo it at any time.
> [Yes, I checked — mark verified] [Cancel]

Confirm calls `contactsSetVerified(id, true)`, then `refreshContacts()` and
`onDone()`. Cancel calls nothing.

### `views/RemoveContactDialog.svelte`

Props: `{ contact; onDone(r: RemoveContactDto); onCancel }`, with
`role="dialog"` and `aria-label="Remove contact"`. State:
`let eraseHistory = $state(false)`. A fresh instance is mounted every time the
dialog opens.

> **Remove “{nickname}”?**
> They will no longer be a contact. If they connect again, you will be asked to
> Accept or Decline them like someone new. Any open connection with them is
> closed now.
> ☐ **Also delete our message history with “{nickname}”**
> *(unchecked)* Your message history with them will be **kept**.
> *(checked)* Your message history with them will be **permanently deleted**.
> This cannot be undone.
> [Remove contact] / *(checked)* [Remove contact and delete history] [Cancel]

- **Confirm** calls `contactsRemove(id, eraseHistory)`, then
  `refreshContacts()` and `onDone(result)`.
- **Notice.** The row's `onNotice` then shows:
  - "Removed “X”. Your message history with them was kept." or
  - "Removed “X”. Your message history with them was deleted.",
  - depending on `history_erased`.
- **`HistoryEraseFailed`**: show the error message inside the dialog and keep
  the dialog open, with the checkbox state unchanged. The contact remains in
  the list.

### 2B tests (Vitest + Testing Library; `../contacts` and `../net` mocked)

| File | Test | Asserts |
|------|------|---------|
| `ContactRow.test.ts` | `renders nickname bold with the NodeId small and gray beneath` | `contact-nickname` sits inside `<strong>`; `node-id-short` follows it in DOM order with class `node-id`; `title` holds the full id |
| | `does not render the self-asserted name in the row` | `self_name: "Mallory"` → the text "Mallory" and the phrase "calls themselves" are absent from the row (unless the rename form is opened) |
| | `unverified contact shows the Unverified badge` | |
| | `badge cannot be spoofed by the nickname` | nicknames `"Verified by you"`, `"✓ Verified"`, `"<span data-testid=\"badge-verified\">x</span>"`, a 128-char string and `"a​b"` (the frontend does not trust the backend for this) with `verified: false` → exactly one `badge-unverified`, no `badge-verified`, and the badge is the first child of `.title` |
| | `non-boolean verified renders Unverified` | |
| | `verified contact shows "Verified by you" and Mark unverified` | |
| | `names render inert` | an `<img src=x onerror=…>` nickname → no `img` element |
| | `Connect appears only when the contact is on the LAN and not connected, and dials its LAN ticket` | `netConnectTicket("t-id")`; hidden when connected or not on the LAN |
| | `rename sends the new nickname`; `rename shows InvalidNickname message`; `Mark unverified calls set_verified(false) without a dialog` | |
| `VerifyContactDialog.test.ts` | `copy states it is the user's statement and not a cryptographic check` | contains "This is your own statement" and "does not check it cryptographically"; contains none of "secure", "guarantee", "cryptographically verified" (case-insensitive; the test uses one regex alternation over these three words) |
| | `shows the full NodeId`; `confirm calls contactsSetVerified(id, true)`; `cancel calls nothing` | |
| `RemoveContactDialog.test.ts` | `history checkbox is unchecked by default` | |
| | `default confirm keeps history` | label "Remove contact"; "will be kept"; `contactsRemove("id", false)` |
| | `checking shows the irreversible outcome and changes the button` | "permanently deleted"; "cannot be undone"; `("id", true)` |
| | `reopening resets the checkbox`; `HistoryEraseFailed keeps the dialog open with the not-removed message` | |
| `Contacts.test.ts` | `lists contacts from the store`; `shows the empty state with the three ways to add`; `shows the removal notice for kept / deleted history` | |
| `MainScreen.test.ts` | `Contacts is the first and default tab`; `starts and stops the contacts listener` | |

---

## Sub-task 2C — add flows (parallel with 2B, after 2.0)

### `views/LanPeers.svelte` → the "On this network" section

- **Header.** Heading `<h3>On this network</h3>`, with the intro "People nearby
  announcing themselves. Names are chosen by them and are not verified."
- **List.** Show `$lanPeers.filter(p => !$contactIds.has(p.node_id))`, keyed by
  `node_id`. The empty state is "No one new on this network." Contacts on the
  LAN are reached via **Connect** on their contact row (2B).
- **Each row:**
  - `<TrustBadge verified={false} />`. This is always shown and ignores DTO
    `verified`.
  - "calls themselves “<bdi>{display_name}</bdi>”", or "(no name)".
  - `<NodeIdLine>`.
  - An **Add…** button.
- **Add…** opens an inline `NicknameForm` with `selfName={display_name}`,
  `submitLabel="Add contact and connect"` and an empty field. On submit:
  1. `contactsAdd(id, nickname)`;
  2. `refreshContacts()`;
  3. `netConnectTicket(ticket)`.

  If the add fails, show the error and do not dial. If the dial fails, show
  "Added “X”, but connecting failed: …".
- Remove the old **Connect** button.

### `views/Network.svelte`

- Remove `<LanPeers />`. It is now mounted by `Contacts.svelte`.
- The ticket form keeps the button **"Connect and remember this device"**. This
  is still true under decision E. Replace the help text with: "Once they reply,
  they're saved as an unverified contact under the name they gave, which you
  can change, and they can reconnect without asking you again."
- On success: call `expectNaming(res.node_id)` and clear the field. The store
  handles events that arrive before this call.
- **Connections list.** Each row uses `<NodeIdLine>`. A row with
  `state === "connected"` whose peer is not in `$contactIds` (the auto-add
  failed) offers **Add contact…**. It calls
  `requestNaming(c.node_id, c.display_name)`.

### `views/NamingPrompt.svelte` (new, global)

```svelte
<div role="dialog" aria-label="Name this contact" data-testid="naming-prompt">
  <p><TrustBadge verified={false} /> Connected. What do you want to call them?</p>
  <NodeIdLine nodeId={head.node_id} />
  <NicknameForm initial={head.nickname ?? undefined} selfName={head.self_name}
                submitLabel="Save contact" cancelLabel="Not now" busy={busy} error={error}
                onSubmit={save} onCancel={() => dropNaming(head.node_id)} />
  {#if more > 0}<p>+{more} more</p>{/if}
</div>
```

`save(nickname)` behaves as follows:

- **Contact exists** (`head.nickname !== null`):
  - unchanged → just `dropNaming`;
  - changed → `contactsRename`, then refresh and drop.
- **No contact:** `contactsAdd`, then refresh and drop. `AlreadyExists` means
  refresh and drop quietly.
- **Errors** show inline and the prompt stays.

**Not now** keeps the auto-added contact.

### `views/IncomingRequestPrompt.svelte`

- After a successful Accept, call `refreshContacts()`.
- Keep the 03-01 prefill (`initial={first.suggested_nickname}`), the cleared
  field blocking Accept, and the explicit Accept click.
- Use `<TrustBadge verified={false} />`. The test id changes from
  `unverified-badge` to `badge-unverified`, so update the tests.

### 2C tests

| File | Test |
|------|------|
| `LanPeers.test.ts` | `hides peers that are already contacts` |
| | `always shows Unverified even if the DTO says verified` |
| | `Add sends the typed nickname then dials the peer's ticket` (call order) |
| | `Add failure does not dial` |
| | `dial failure after add reports both facts` |
| | `nickname field is not prefilled` |
| | kept: `renders HTML and bidi names inert`, `keeps same-named peers distinct by NodeId`, `shows (no name)` |
| `Network.test.ts` | `successful connect expects naming for the dialed node` |
| | `copy says the device is remembered as an unverified contact` |
| | `connected non-contact row offers Add contact and requests naming with its display_name` |
| | `contact rows do not offer Add contact` |
| | `LAN list is no longer on the Network tab` |
| | kept: ticket and error tests |
| `NamingPrompt.test.ts` | `prefills the auto-added default nickname and shows the self-asserted name as unverified context` |
| | `Save with unchanged nickname only dismisses` |
| | `Save with a new nickname renames` |
| | `Not now keeps the contact (no remove/rename call)` |
| | `no existing contact → Save adds` |
| | `AlreadyExists dequeues quietly` |
| | `InvalidNickname keeps the prompt` |
| | `hostile names inert` |
| `IncomingRequestPrompt.test.ts` | `successful accept refreshes contacts`, `prefill still comes from suggested_nickname`, plus the selector updates |

---

## Sub-task 2D — integration (sequential, after 2A + 2B + 2C)

1. Merge 2A, 2B and 2C.
2. Mount `<NamingPrompt />` next to `<IncomingRequestPrompt />` in
   `MainScreen.svelte`, and add the test
   `MainScreen.test.ts › naming prompt is shown on every tab`.
3. Run all quality gates and the security checks below.
4. **Manual end-to-end run** at Level 2 (two profiles on one machine). Go
   through every checklist item below and record the results in the PR.
5. Update **README.md**:
   - **Ticket steps:** include the prefilled naming prompt.
   - **"Peers on this network…":** this now lives in **Contacts → On this
     network**, with **Add…**.
   - **New Contacts section:**
     - Unverified vs "Verified by you": the latter is the user's own
       attestation, not a cryptographic check.
     - The list shows only your nickname for each contact; what they call
       themselves appears only in the prompts.
     - Rename, and **Connect** for contacts on the LAN.
     - Names may contain emoji sequences (👨‍👩‍👧, 🏳️‍🌈, 🏴󠁧󠁢󠁥󠁮󠁧󠁿) but not stray
       zero-width characters.
     - Removal semantics: trust is always removed; history is removed only if
       the box is ticked; no message store exists yet, so there is nothing to
       delete until DD-04/DD-05.
     - Copying a NodeId: expand it and select it.
     - Plaintext storage until DD-05.
   - **Manual GUI checklist,** add:
     - Contacts is the default tab, and the empty state shows.
     - The Accept prompt's nickname is prefilled with their name (or
       "Peer <id>") and is editable. Clearing it disables Accept. After
       accepting, the row shows only the bold nickname, the NodeId gray and
       truncated beneath it, and Unverified. The peer's own name is not in the
       row.
     - Hovering shows the full NodeId. "Show full NodeId" reveals selectable
       text, and Ctrl/⌘+C copies it.
     - Ticket connect: once the other side accepts, the naming prompt appears,
       prefilled with their name. "Not now" leaves them in Contacts as
       Unverified with that name.
     - On this network: Add… on a non-contact makes it a contact and connects.
       The other side gets a prompt.
     - Rename changes only the nickname.
     - Mark verified: honest dialog, and the badge reads "Verified by you".
       Mark unverified reverts it.
     - Remove: the checkbox is unchecked by default and both outcomes are
       shown. After removal the connection drops and the peer's next connect
       prompts.
     - The peer renames itself and reconnects: your nickname for them is
       unchanged.
     - **Reconnect silently:** quit and relaunch both instances. On B, open
       Contacts. A's row shows **Connect** (A is visible on the LAN). Click it.
       A shows **no** Accept prompt, and both show Connected under Network →
       Connections. Repeat by pasting A's ticket into B's Network tab, again
       with no prompt on A.

---

## Guardrails (mandatory)

- **Never write credentials, keys, tokens or secrets in code, tests, fixtures or
  config. All credentials must be provided at runtime (OS keychain /
  environment).**
- Never reference `SecretKeyBytes` or `DbKey` anywhere under `app/`, including
  comments and names.
- Contact and known-peer test doubles are allowed only in `#[cfg(test)]` code.
  They are behind `test-support`, which the app enables only as a
  dev-dependency, so any non-test use fails `cargo build`. Never name
  `FileKeychain` or the headless crate under `app/`.
- `core` stays `#![forbid(unsafe_code)]`. There are no core edits in this step
  (see 2A).
- No nicknames, self-asserted names or message content in logs or `eprintln!`.
  NodeIds are fine.
- Never bypass `.githooks/pre-commit` with `--no-verify`.
- Only files with real content:
  - no placeholder components;
  - no "Open chat" button;
  - no contact-info route or stub, only the code comment.
- No wire-format change: `PROTOCOL_VERSION` stays as it is.
- **Clipboard:** no `writeText`, no clipboard plugin import, and not even the
  word "clipboard" in non-test files under `app/src/` other than
  `views/MyInfo.svelte`.
- **No `{@html}`** outside `MyInfo.svelte`.

## Security — threats, prevention, verification

| # | Threat | Prevention / fix | Check |
|---|--------|------------------|-------|
| T1 | Badge spoofing through nickname text ("Verified by you", ✓, HTML, zero-width, homoglyphs) | A fixed-text `TrustBadge` driven only by `verified === true`. It comes first in the row and never shrinks. Names are plain text inside `<bdi>`. The backend rejects Cf characters. LAN rows ignore the DTO's `verified` | `ContactRow.test.ts › badge cannot be spoofed…`, `TrustBadge.test.ts`, `LanPeers.test.ts › always shows Unverified…`, `contacts_add_invalid_nickname_kinds` |
| T2 | A peer-chosen name shown as if it were identity in the list | The list renders only the user's nickname (decision B) above an always-visible NodeId | `ContactRow.test.ts › does not render the self-asserted name in the row`; check **D1** |
| T3 | XSS via names or NodeIds | Text interpolation only | `html-guard.test.ts`; every `names render inert` test; check **D2** |
| T4 | "Verified" claims more than it means | The badge reads "Verified by you"; the dialog says "your own statement" and "does not check it cryptographically" | `VerifyContactDialog.test.ts` (negative regex), `TrustBadge.test.ts` |
| T5 | History deletion as the accidental default | The checkbox is off by default and resets on every open; `clear_history: None` means Keep; the confirm label and consequence text change with the checkbox | `RemoveContactDialog.test.ts`, `contacts_remove_defaults_to_keeping_history` |
| T6 | Removal leaves trust or a live connection behind | The row is removed, then `node.disconnect`, so the next connect prompts | `contacts_remove_drops_live_connection_and_next_inbound_prompts` |
| T7 | A partial failure is misreported | History is erased first, then the contact removed; on `HistoryEraseFailed` the contact is kept and the message says so | `contacts_remove_history_failure_keeps_contact`, `RemoveContactDialog.test.ts › HistoryEraseFailed…` |
| T8 | Forged IPC arguments | Length-capped NodeId parse, `SelfContact`, `Nickname::parse`, `Option<bool>` flag, and serde rejects a non-boolean `verified` | `contacts_add_rejects_malformed_node_id`, `contacts_add_rejects_self`, `contacts_add_invalid_nickname_kinds` |
| T9 | The webview sets the peer's self-asserted name | `contacts_add` has no `self_name` parameter; the backend reads it from the node | `contacts_add_takes_self_name_from_the_node_not_args` |
| T10 | Identity keyed by name in the UI | Every `each` key and store lookup uses `node_id` | `keeps same-named peers distinct by NodeId`; check **D3** |
| T11 | The peer's name becomes the nickname without the user noticing | The Accept prompt and the naming prompt prefill the default nickname by user decision E: the validated self-asserted name, else `Peer <id>`. The prefill is visibly editable and shown next to the separately labelled "chosen by them, not verified" context. Submitting needs an explicit click, an empty field blocks it, and the contact is always Unverified. LAN Add stays empty | `NamingPrompt.test.ts › prefills the auto-added default…`, `IncomingRequestPrompt.test.ts › prefill still comes from suggested_nickname`, 03-01's `clearing the field disables Accept`, `LanPeers.test.ts › nickname field is not prefilled` |
| T12 | Clipboard rule violation | `NodeIdLine` offers selection only | `clipboard-guard.test.ts`; check **D4** |
| T13 | Store errors look like "no contacts" | `refreshContacts` keeps the last list and shows an error; `is_known` errors are never reported as `false` | `contactsStore.test.ts › refresh failure…`, `contacts_is_known_store_failure_is_error_not_false` |
| T14 | Naming prompt lost to an event race | A bounded buffer of recent outbound-connected events, checked by `expectNaming` | `contactsStore.test.ts › connected event that arrives before expectNaming…` |
| T15 | Blocking I/O on the main thread or runtime | Commands are `async` and run store work in `spawn_blocking` | Code review of `contacts_cmd.rs`; check **D5** |
| T16 | Names in logs; key material in DTOs | No logging in `contacts_cmd.rs`; DTOs carry public strings only | check **D6**; `contact_dtos_never_contain_key_material` |

### Security checks (run from the repo root and review every hit)

```sh
# D1 the row never renders self_name (only the rename form passes it as selfName)
grep -n 'self_name' app/src/views/ContactRow.svelte        # expect: the comment and selfName={contact.self_name} on NicknameForm only

# D2 {@html} only in MyInfo
grep -rn '{@html' app/src --include=*.svelte               # expect only app/src/views/MyInfo.svelte

# D3 no keyed each by a name
grep -rnE '\((c|p|r|contact|peer)\.(nickname|display_name|self_name)\)' app/src --include=*.svelte   # expect no output

# D4 clipboard confined to MyInfo (same as the hook)
grep -RlE 'writeText|clipboard' app/src --exclude='*.test.ts'   # expect only app/src/views/MyInfo.svelte

# D5 every store-touching command uses spawn_blocking
grep -n 'spawn_blocking' app/src-tauri/src/contacts_cmd.rs   # expect one per store-touching fn

# D6 no prints/logs with names in the shell
grep -rnE '(eprintln|println|trace|debug|info|warn|error)!\(.*(nickname|self_name|display_name)' app/src-tauri/src   # expect no output

# D7 no placeholder chat action
grep -rniE 'open chat|openChat' app/src --include=*.svelte   # expect no output
```

## Tests (DD-03 mapping)

- **Command level:** the tests in the 2A table cover:
  - add/known: `contacts_add_then_list_shows_unverified` and
    `contacts_is_known_tracks_add_and_remove`;
  - nickname independence: `contacts_rename_…`;
  - verify toggle: `contacts_set_verified_…`;
  - sanitization: `contacts_add_invalid_nickname_kinds`;
  - removal reverts trust: `contacts_remove_drops_live_connection_…`;
  - removal plus the clear-history flag: `contacts_remove_defaults_…` and
    `…_with_clear_history_…`;
  - namespacing: `contacts_are_namespaced_by_bound_account`.
- **UI:** badge spoofing, honest copy, the default-off checkbox, and all three
  add paths plus Connect.
- The 03-01 core tests stay green.
- Keychain-dependent tests use the in-memory doubles.

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

Every commit goes through `.githooks/pre-commit`. Never use `--no-verify`.

## Definition of done

- [ ] 2A: all six `contacts_*` commands are registered, async, and use
      `spawn_blocking`, with no core edits. All 2A tests pass.
- [ ] 2.0: `contacts.ts`, `contactsStore` (with the race-safe naming buffer),
      `nodeId.ts`, `NodeIdLine`, `TrustBadge` and `html-guard.test.ts` are in
      place and tested.
- [ ] 2B: the Contacts tab is first and the default. Rows show only the bold
      nickname, the gray truncated NodeId (full on hover or expand, selectable)
      and the badge. Connect (when the contact is on the LAN), Rename, the
      honest verify dialog, Unverify, and the Remove dialog (opt-in, off by
      default, both outcomes shown) all work. The code comment about the future
      contact-info screen is present.
- [ ] 2C: On this network lists only non-contacts, always Unverified, with Add….
      Ticket connect auto-adds and shows the prefilled naming prompt; Not now
      keeps the contact. Connections offers Add contact… only when an auto-add
      failed. Accept refreshes the list.
- [ ] 2D: NamingPrompt is mounted globally, and the manual checklist (including
      the concrete reconnect-silently item) passes on two profiles.
- [ ] The clipboard rule holds. This is recorded as a user-accepted deviation in
      the PR.
- [ ] Security checks D1–D7 are clean.
- [ ] README is updated as listed in 2D.
- [ ] All quality gates pass through the hook, without `--no-verify`.
- [ ] PR follows `.github/PULL_REQUEST_TEMPLATE.md`:
  - crates: `app` (Tauri + Svelte) and docs;
  - **Wire format: no change; `PROTOCOL_VERSION` not bumped**;
  - list the new commands, the commands you ran and the checklist results;
  - state the deviations: copy-by-selection (A), nickname-only rows (B), and
    "Open chat" deferred to DD-04.
