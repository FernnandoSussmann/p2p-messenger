# Step 07-01 — One stylesheet, three themes and the theme setting end to end

**Size:** L — about +1,900 / −300 lines. Rust ≈ +380 (core ≈ +260 including tests, app shell ≈ +120
including tests). Frontend ≈ +900 / −250 (new `app/src/app.css` ≈ 550, view restyles ≈ ±400 with
≈ 120 lines of `<style>` deleted, `theme.ts` + `ThemePicker.svelte` ≈ 140). Tests ≈ +600 (frontend
guards ≈ 300, component/unit ≈ 300).

## TL;DR

First of four sequential steps for [DD-07](../07-visual-identity-and-ux-redesign.md) ("Suggested
implementation order", PR 1). It:

1. Ports `design/visual-identity.html` into **one global stylesheet** (`app/src/app.css`) imported
   once from `app/src/main.ts`, with three theme token blocks, precomputed `rgba()` soft tints (no
   `color-mix()`, no `:has()`), shared tokens, base styles, layout primitives, component classes and
   a global `prefers-reduced-motion` rule. `app/index.html` gets `data-theme="midnight"`.
2. Moves **every existing view** onto the shared classes and deletes every `<style>` block,
   `style=` attribute and `style:` directive (structure and text unchanged).
3. Adds the **theme setting end to end**: forward-only migration 5 (nullable `account_settings.theme`
   with a `CHECK`), a closed `Theme` enum in `core::identity`, `SettingsStore::get_theme/set_theme`
   on every implementation, `IdentityService::theme()/set_theme()`, and the Tauri commands
   `identity_theme_get` / `identity_theme_set` (typed `InvalidTheme`), registered and invoked.
4. Adds `app/src/lib/theme.ts` (`THEMES`, `applyTheme`, `loadTheme`, `chooseTheme`,
   `pendingTheme`), `ThemePicker.svelte` on My Info, and makes `App.svelte` call `loadTheme()`
   before the main screen renders.
5. Adds the guards: single stylesheet, contrast (WCAG AA), token drift, no hardcoded colours, no web
   storage; plus Rust and component tests.

Navigation, the setup screen, Discover, contact info and the conversation layout are **not** in this
step (07-02 and 07-03). The app keeps today's three tabs and flows; it only looks different and
gains the theme picker on My Info.

## Mandatory process rules

- After each step you implement and after each subagent execution completes, run the /usage-guard skill (Skill tool, skill: "usage-guard") to check plan usage; if it says to wait, wait until the limit resets before continuing. Pass this exact instruction on to every subagent you create, including subagents' subagents.
- If in doubt, ask; do not assume.
- Read `AGENTS.md` and the whole of `design_docs/07-visual-identity-and-ux-redesign.md` before
  starting. The DD is the spec; this file maps it onto the code for this step.
- Never write credentials or secrets in code, tests, fixtures or docs. Every credential must be
  provided at runtime; this work needs none.

## Prerequisites and branch

This is the first DD-07 step. Steps are strictly sequential: **07-01 → 07-02 → 07-03 → 07-04**.

1. Make sure `main` is up to date: `git fetch origin && git checkout main && git pull --ff-only`.
   If the pull refuses because of the locally modified `design_docs/README.md`, run `git stash -u`,
   pull, then `git stash pop` (the untracked DD inputs come back with it).
2. Create the DD branch from it: `git checkout -b 07-visual-identity-and-ux-redesign`. (If it already
   exists locally or on `origin`, check it out and confirm with `git log --oneline -5` that it is
   based on current `main`; ask if it diverged.)
3. Install the hook once per clone: `git config core.hooksPath .githooks`.
4. **First commit on the DD branch: the DD inputs that are still uncommitted in the working tree.**
   Run `git status` and commit, through the hook (no `--no-verify`):
   - `design/` (the `design/visual-identity.html` reference sheet **and** `design/wireframes/`),
   - `design_docs/07-visual-identity-and-ux-redesign.md`,
   - `design_docs/08-outgoing-delivery-time.md`,
   - the modified `design_docs/README.md`,
   - the DD-07 step files `design_docs/steps/07-*.md` (this file and its three siblings).
   - the deletion of the obsolete `design_docs/steps/02-0[1-4]-*.md` files (already deleted in the
     working tree by the user; stage them with `git add -A design_docs/steps/` so the removal is
     part of this first commit — user decision, do not restore them).
   `design/wireframes/` stays until 07-03 deletes it.
5. Push the DD branch, then create the step branch from it:
   `git checkout -b step-07-01-stylesheet-themes-and-theme-setting`. Open the PR **into
   `07-visual-identity-and-ux-redesign`** (not `main`) when done, following
   `.github/PULL_REQUEST_TEMPLATE.md`.

## Resolved decision: Light `--success` (answered by the user)

The DD's contrast guard requires each status colour to reach ≥ 4.5:1 against its `-soft` tint
composited over `--surface`. With the original reference values this failed for the Light theme
(`#1F7A4D` on its tint over `#FAF7FE` is 4.16:1; over `--bg` 4.39:1). **The user chose: change Light
`--success` to `#1C7347`** (4.53:1 on its tint over `--surface`, 4.79:1 on its tint over `--bg`,
5.51:1 on `--surface`), and its `--success-soft` tint to the same RGB (`rgba(28,115,71,.14)`), in
**both** `design/visual-identity.html` and `app/src/app.css` in the same commit, so the drift guard
stays green. Do not lower the threshold or drop the pair. The contrast guard also checks each
status colour on its `-soft` tint composited over `--bg` (badges sit on both surfaces).

Also note (not guarded by the DD, but real): `--text-muted` on `--surface-2` is below 4.5:1 in all
three themes (4.27–4.40) and Midnight `--text` on `--surface-2` is 4.45. `--surface-2` is the
incoming bubble, read-only input and hover background. Never put `--text-muted` (or Midnight
`--text`) text on `--surface-2`; use `--bubble-in-text` / `--text-strong` there. Never put a badge
on `--surface-2` either: the Light success tint over `--surface-2` is 4.20:1 even with `#1C7347`.
Do not use
`opacity` on text (the reference's `.bubble .stamp { opacity: .7 }` must not be ported): the
contrast guard cannot see it.

## Parallel vs sequential

Across steps: sequential (07-01 → 07-02 → 07-03 → 07-04). Inside this step there are three tracks
that can run in parallel (separate git worktrees + sub-agents), then one sequential integration:

| Track | Owns (only this track edits these) | Depends on |
|---|---|---|
| **A — Rust theme setting** | `core/src/identity/{theme.rs,mod.rs,settings.rs,settings_sql.rs,settings_sql/tests.rs,service.rs}`, `core/src/storage/{schema.rs,tests.rs}`, `core/src/discovery/testing/scenarios.rs`, `core/tests/integration/{discovery_lan.rs,storage_namespacing.rs}`, `app/src-tauri/src/{identity_cmd.rs,lib.rs,net_cmd.rs}` | nothing |
| **B — Stylesheet + restyle + guards** | `app/src/app.css` (new), `app/src/main.ts`, `app/index.html`, every existing `app/src/views/*.svelte` **except** `MyInfo.svelte`, the style guard tests | the contrast decision above |
| **C — Theme state + picker** | `app/src/lib/theme.ts` (new), `app/src/lib/theme.test.ts`, `app/src/identity.ts`, `app/src/views/ThemePicker.svelte` (+ test) | the command contract (fixed by the DD; mock `invoke`) |

Then, **sequentially** after A, B and C are merged into the step branch:

- **D — Integration:** `App.svelte` (`loadTheme()` before main, remove its `<style>`, shell class)
  and `MyInfo.svelte` (restyle + `ThemePicker` + `chooseTheme`), `App.test.ts`, `MyInfo.test.ts`;
  then run the full gate.

Shared-file conflict risks and how to avoid them:

- `app/src/app.css` is **owned by B**. C needs the picker classes: agree the names up front (below)
  and let B write them. C must not create CSS.
- `App.svelte` and `MyInfo.svelte` are touched by B (restyle) and C (theme) → both deferred to D.
- `app/src-tauri/src/lib.rs` and `identity_cmd.rs` are only touched by A.
- `app/src/identity.ts` is only touched by C (A must not edit TypeScript).
- Agreed class names for the picker: `.theme-picker` (the radiogroup row), `.swatch` (round
  button), `aria-checked="true"` for the selected ring, and `data-theme="<t>"` **on each swatch**
  (from the constant `THEMES`, so the swatch shows that theme's `--bg` with its `--text` ring
  without any hardcoded colour; see Part 3).

## Tasks

### Track A — Rust: migration, enum, stores, service, commands

1. **`Theme` enum** — new `core/src/identity/theme.rs` (real content, re-exported from
   `core/src/identity/mod.rs` as `pub use theme::{Theme, InvalidTheme};`):

   ```rust
   /// A UI colour theme. Closed set; the only representation that reaches SQL.
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
   pub enum Theme {
       #[default]
       Midnight,
       Dark,
       Light,
   }

   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub struct InvalidTheme;

   impl Theme {
       pub const ALL: [Theme; 3] = [Theme::Midnight, Theme::Dark, Theme::Light];
       pub fn as_str(self) -> &'static str {
           match self { Theme::Midnight => "midnight", Theme::Dark => "dark", Theme::Light => "light" }
       }
       /// Exact lowercase match only: no trimming, no case folding.
       pub fn parse(raw: &str) -> Result<Self, InvalidTheme> {
           Self::ALL.into_iter().find(|t| t.as_str() == raw).ok_or(InvalidTheme)
       }
   }
   ```
   Implement `Display`/`Error` for `InvalidTheme`. Unit tests: every value round-trips; `"Dark"`,
   `" dark"`, `"dark "`, `""`, `"system"`, `"midnight'--"` are rejected.

2. **Migration 5** in `core/src/identity/settings_sql.rs` (the slice owns the SQL):

   ```rust
   pub(crate) const MIGRATION_SETTINGS_THEME: &str = "
   ALTER TABLE account_settings ADD COLUMN theme TEXT
     CHECK (theme IS NULL OR theme IN ('midnight', 'dark', 'light'));
   ";
   const SQL_SETTINGS_SET_THEME: &str = "INSERT INTO account_settings (account_id, theme) VALUES (?1, ?2) ON CONFLICT (account_id) DO UPDATE SET theme = excluded.theme";
   const SQL_SETTINGS_GET_THEME: &str = "SELECT theme FROM account_settings WHERE account_id = ?1";
   ```
   Export `MIGRATION_SETTINGS_THEME` through `identity/mod.rs` like `MIGRATION_SETTINGS`. In
   `core/src/storage/schema.rs` append `Migration { version: 5, label: "settings_theme", sql:
   crate::identity::MIGRATION_SETTINGS_THEME }` and set `SCHEMA_VERSION = 5` (DD-08 will take 6).
   Never edit migrations 1–4. Both `SQL_*` constants name `account_id`, so
   `core/tests/integration/sql_namespacing_guard.rs` stays green with **no allowlist change**. Do not
   add `theme` to `LegacySettings` or `SQL_SETTINGS_INSERT_IF_ABSENT` (no legacy-import change).

3. **`SettingsStore`** (`core/src/identity/settings.rs`): add required methods
   `fn get_theme(&self, id: &AccountId) -> Result<Option<Theme>, SettingsError>;` and
   `fn set_theme(&self, id: &AccountId, theme: Theme) -> Result<(), SettingsError>;` (doc: `None`
   means never chosen = Midnight). Implement them on:
   - the `Arc<T>` blanket impl (forward),
   - `MemorySettingsStore` (new `theme: Option<Theme>` field in `AccountSettings`),
   - `SqlCipherSettingsStore`: `set` binds `theme.as_str()` through the existing `set()` helper;
     `get` runs `SQL_SETTINGS_GET_THEME` with `query_row(...).optional()`, flattens the
     `Option<Option<String>>`, and maps an unparseable stored value to `Ok(None)` (defended, though
     the `CHECK` makes it impossible),
   - every other implementation: `SqlPresence` in `core/src/discovery/testing/scenarios.rs`,
     `SqlPresence` and `FailingPresence` in `core/tests/integration/discovery_lan.rs`, and
     `FlakyPresence` in `app/src-tauri/src/net_cmd.rs` (delegate to their inner memory store).
   Find them all with `grep -rn "impl.*SettingsStore for" --include=*.rs . | grep -v target`.

4. **`IdentityService`** (`core/src/identity/service.rs`): add a `ThemeError { NotProvisioned,
   Marker(MarkerError), Settings(SettingsError) }` (with `Display`/`Error` like `SetNameError`,
   exported from `mod.rs`) and

   ```rust
   /// The account's theme; never chosen reads as Midnight.
   pub fn theme(&self) -> Result<Theme, ThemeError>
   pub fn set_theme(&self, theme: Theme) -> Result<(), ThemeError>
   ```
   Resolve the account exactly like `set_display_name` (`read_marker(&self.data_dir)` →
   `ok_or(ThemeError::NotProvisioned)`). No keychain access, no logging needed. If you add a log
   line anyway, it goes through `core::logging` and the field name is added to `ALLOWED_FIELDS` in
   `core/tests/integration/log_redaction_guard.rs` only after review.

5. **Tauri commands** (`app/src-tauri/src/identity_cmd.rs`):
   - Add `InvalidTheme` to `IdentityCmdError` (unit variant, serialises as `{"type":"InvalidTheme"}`).
   - `#[derive(Debug, Serialize)] pub struct ThemeDto { pub theme: &'static str }` built from
     `Theme::as_str()` (so the output is always one of the three lowercase strings).
   - `impl From<ThemeError> for IdentityCmdError`, mirroring `From<SetNameError>`:
     `Marker(MarkerError::Corrupt)` → `StatusError::MarkerCorrupt.into()`, everything else →
     `Internal`. Do **not** invent `NotProvisioned` / `StoreUnavailable` variants (see Open
     questions).
   - Plain testable functions `theme_get(service)` and `theme_set(service, raw: &str)`, plus thin
     wrappers:

     ```rust
     #[tauri::command]
     pub fn identity_theme_get(service: State<'_, AppIdentity>) -> Result<ThemeDto, IdentityCmdError> {
         theme_get(&service)
     }

     /// `theme` arrives as a string so a bad value is the typed `InvalidTheme`,
     /// not Tauri's generic argument-deserialisation error.
     #[tauri::command]
     pub fn identity_theme_set(service: State<'_, AppIdentity>, theme: String) -> Result<ThemeDto, IdentityCmdError> {
         theme_set(&service, &theme)
     }
     ```
     `theme_set` parses with `Theme::parse` **before** touching the service, returning
     `InvalidTheme` and changing nothing on failure.
   - Register both in `tauri::generate_handler![...]` in `app/src-tauri/src/lib.rs` and add them to
     the `use identity_cmd::{...}` list. `removeUnusedCommands: true` is set in
     `tauri.conf.json`: both commands must be invoked from the frontend (`lib/theme.ts` through
     `identity.ts`, Track C).

### Track B — The single stylesheet and the restyle

1. **`app/src/app.css`** — the only CSS in the app, in this order:
   1. **Theme blocks** `[data-theme="midnight"]`, `[data-theme="dark"]`, `[data-theme="light"]`
      with exactly the 19 reference token names (`--bg`, `--surface`, `--surface-2`, `--border`,
      `--border-strong`, `--text`, `--text-strong`, `--text-muted`, `--accent`, `--accent-hover`,
      `--accent-contrast`, `--success`, `--warning`, `--danger`, `--bubble-in`, `--bubble-in-text`,
      `--bubble-out`, `--bubble-out-text`, `--shadow`) and values copied from
      `design/visual-identity.html` (lines ~28–96), plus `color-scheme` per theme.
   2. In the **same theme blocks**, the precomputed tints that replace every `color-mix()` of the
      reference: `--accent-soft`, `--success-soft`, `--warning-soft`, `--danger-soft` =
      `rgba(<that colour's r,g,b>, .14)`; `--focus: 0 0 0 3px rgba(<accent>, .40)`; and the app-only
      tokens needed for the other `color-mix()` uses in the reference (alert borders at 35 %,
      `.btn-danger` border at 45 %, dialog backdrop `--bg` at 70 %, avatar/accent border at 30 %),
      e.g. `--accent-line`, `--success-line`, `--warning-line`, `--danger-line`,
      `--danger-line-strong`, `--backdrop`, and `--qr-quiet: #FFFFFF` (the QR's light quiet zone in
      every theme). Example for Midnight: `--accent-soft: rgba(138, 155, 214, .14);`.
      Keep the list of app-only tokens short and identical across the three blocks.
   3. **Shared tokens** on `:root`: `--radius-sm`, `--radius`, `--radius-lg`, spacing scale,
      `--font` (system stack from the reference), `--mono`, `--ease`. No web fonts, no `@import`,
      no `url()` to anything remote.
   4. **Base**: box-sizing, `html`/`body` background and text, headings, `:focus-visible` ring with
      `var(--focus)`, `code`/`.mono`, `hr`/`.divider`, `::selection`.
   5. **Layout primitives**: `.row`, `.stack`, the centred max-width app column, `.muted`,
      `.visually-hidden`.
   6. **Component classes** ported from the reference: `.btn` (+ `-primary`, `-secondary`, `-ghost`,
      `-danger`, `-sm`, `-icon`), `.input`, `.textarea`, `.select`, `.check`, `.radio`, `.switch`,
      `.tabs`/`.tab`, `.badge` (+ `-success`, `-warning`), `.chip`, `.alert` (+ `-info`, `-success`,
      `-warning`, `-danger`), `.panel`, `.card`, `.list`, `.divider`, `.dialog`, `.bubble` (+ `.in`,
      `.out`, `.failed`), `.node-id`, `.spinner`, `.visually-hidden`, plus the picker classes
      (`.theme-picker`, `.swatch`) and the few app-only classes the restyled views need (QR frame,
      mono block). Every reference state written with `:has()` becomes a class or attribute the
      component sets on the parent (e.g. `.check[aria-disabled="true"]` instead of
      `.check:has(input:disabled)`). Do not port the reference page's own chrome (`.topbar`,
      `.intro`, `.swatches`, `.preview*`, `.ratio`, `.type-*`).
   7. `@media (prefers-reduced-motion: reduce) { *, *::before, *::after { transition: none
      !important; animation: none !important; } }`.

   Hard rules: no `color-mix(`, no `:has(`, no hex/`rgb(`/`rgba(`/`hsl(`/named colour outside the
   three theme blocks (only `transparent`, `currentColor`, `inherit` allowed elsewhere).

2. `app/src/main.ts`: add `import "./app.css";` — the only stylesheet import in the app.
3. `app/index.html`: `<html lang="en" data-theme="midnight">`. Nothing else (no `<style>`, no
   `<link rel="stylesheet">`, no inline script).
4. **Restyle every view except `MyInfo.svelte`/`App.svelte`** (Track D does those). Keep structure,
   text, `data-testid`s and roles; only add classes and delete `<style>` blocks / `style=`:
   - `TrustBadge.svelte`: `.badge .badge-success` for "Verified by you", `.badge .badge-warning`
     for "Unverified"; labels and `title`s unchanged; delete the hardcoded `#0b6b2e`, `#8a3b00`,
     `#fff3e0`.
   - `NodeIdLine.svelte`: `.node-id` / `.muted` instead of `color: gray`; the full NodeId keeps
     `user-select: all` through a shared class (e.g. `.selectable`, also used by
     `VerifyContactDialog.svelte`); the toggle is `.btn .btn-ghost .btn-sm`.
   - Dialogs (`VerifyContactDialog`, `RemoveContactDialog`, `ClearHistoryDialog`,
     `IncomingRequestPrompt`, `NamingPrompt`): `.dialog` on the existing `role="dialog"` element
     (keep it an inline element, not a modal `<dialog>`), buttons in **secondary-first order**,
     destructive confirms ("Remove contact" / "Remove contact and delete history", "Clear history")
     `.btn-danger`, the verify confirm `.btn-primary`. `ClearHistoryDialog` keeps Cancel
     `autofocus`. `IncomingRequestPrompt`: replace `style="word-break: break-all"` on the NodeId
     `<code>` with a class.
   - `ErrorBlocking.svelte` / `StorageUnavailable.svelte`: `.alert .alert-danger` and a
     `.btn .btn-primary` Retry.
   - Every `role="alert"` → `.alert .alert-danger`; every `role="status"` → `.alert .alert-info` or
     `.muted`.
   - `MessageBubble.svelte`: `.bubble .in` / `.bubble .out` (+ `.failed` → `--danger` border),
     delete the `#e3f0ff`/`#eee`/`#555` colours; meta text uses the bubble's text token, no opacity.
   - `Composer.svelte`: `.textarea`, `.btn-primary` Send, the over-limit counter uses a `--danger`
     class (keep `class:over`).
   - `Chat.svelte`: header as a `.row`; the log keeps `role="log"`; replace `max-height: 50vh` with
     a class (07-03 makes it fill the height).
   - `ContactRow.svelte`, `Contacts.svelte`, `LanPeers.svelte`, `Network.svelte`,
     `PresenceToggle.svelte` (`.switch`), `NicknameForm.svelte` (`.input`, buttons),
     `FirstRunWizard.svelte`, `MainScreen.svelte` (`.tabs`/`.tab` on the existing tablist, which
     keeps its three tabs in this step).
   - Inline SVG icons, if any are introduced, use `stroke="currentColor"`/`fill="none"`, never a
     colour literal.

### Track C — `theme.ts`, `identity.ts`, `ThemePicker`

1. `app/src/identity.ts`: add `| { type: "InvalidTheme" }` to `IdentityCmdError` and

   ```ts
   import type { Theme } from "./lib/theme";
   export interface ThemeDto { theme: Theme }
   export const identityThemeGet = () => invoke<ThemeDto>("identity_theme_get");
   export const identityThemeSet = (theme: Theme) => invoke<ThemeDto>("identity_theme_set", { theme });
   ```
   No new user-facing text: `describeError` already returns "Something went wrong." for unknown
   types; that is what a failed theme save shows.

2. `app/src/lib/theme.ts` (the only code that writes `document.documentElement.dataset.theme`):

   ```ts
   import { get, writable } from "svelte/store";
   import { identityThemeGet, identityThemeSet } from "../identity";

   export const THEMES = ["midnight", "dark", "light"] as const;
   export type Theme = (typeof THEMES)[number];
   const DEFAULT: Theme = "midnight";
   const isTheme = (v: unknown): v is Theme =>
     typeof v === "string" && (THEMES as readonly string[]).includes(v);

   /** The theme currently applied (for the pickers). */
   export const theme = writable<Theme>(DEFAULT);
   /** Setup-only, in memory: chosen before an account exists, or not yet saved. */
   export const pendingTheme = writable<Theme | null>(null);

   export function applyTheme(t: unknown): Theme {
     const next = isTheme(t) ? t : DEFAULT;          // allowlist, never the raw value
     document.documentElement.dataset.theme = next;
     theme.set(next);
     return next;
   }

   export async function loadTheme(): Promise<void> {
     const pending = get(pendingTheme);              // an unsaved setup pick wins this session
     try {
       applyTheme(pending ?? (await identityThemeGet()).theme);
     } catch {
       applyTheme(pending ?? DEFAULT);               // never blocks startup
     }
   }

   /** Optimistic: applies now, persists, reverts on failure and returns the error. */
   export async function chooseTheme(t: Theme): Promise<unknown | null> {
     const previous = get(theme);
     applyTheme(t);
     try {
       await identityThemeSet(t);
       pendingTheme.set(null);
       return null;
     } catch (e) {
       applyTheme(previous);
       return e;
     }
   }
   ```
   `pendingTheme` is consumed by `loadTheme()` now and by the setup screen in 07-02. It is a JS
   value only: no `localStorage`, `sessionStorage`, cookie, IndexedDB or file.

3. `app/src/views/ThemePicker.svelte`: props `{ current: Theme; onChoose: (t: Theme) => void;
   disabled?: boolean }`; knows nothing about persistence. Markup:
   `<div class="theme-picker" role="radiogroup" aria-label="Theme">` with one
   `<button type="button" role="radio" class="swatch" data-theme={t} aria-checked={t === current}
   aria-label={LABELS[t]} tabindex={t === current ? 0 : -1}>` per entry of `THEMES`, where
   `LABELS = { midnight: "Midnight", dark: "Dark", light: "Light" }`. Arrow Left/Up and Right/Down
   move to the previous/next option (wrapping), focus it and call `onChoose`; click calls
   `onChoose`. The swatch draws its own theme's `--bg` and a `--text` ring purely through the
   `data-theme` attribute + `.swatch` CSS (no colour in the component). Visible focus ring — but
   `data-theme` on the swatch re-scopes `--focus` (and every token) to the swatch's theme, so the
   focus and checked rings must not come from the swatch's own tokens: wrap each swatch in an
   element without `data-theme` that draws the ring (or use an outline whose colour is inherited
   from the page), and check focus/checked visibility of every swatch in every theme manually.

### Track D — Integration (sequential, after A + B + C)

1. `app/src/App.svelte`: delete the `<style>` block (use the app column class on `<main>`); in
   `refresh()`, after storage is OK and **before** `appView.set({ kind: "main" })`, `await
   loadTheme()`. The loading screen stays up for that one call. Pre-account screens (loading,
   blocked, storage, wizard) never call `applyTheme`, so they show the static Midnight from
   `index.html` until a choice is made.
2. `app/src/views/MyInfo.svelte`: restyle (replace both `style=` attributes with classes; QR inside
   a centred frame on `var(--qr-quiet)`; ticket in a muted mono block). Put "Current name: …" and
   the `ThemePicker` (`current={$theme}`, `onChoose` → `chooseTheme`) in one `.row` at the top. A
   failed save shows `describeError(err)` in a `role="alert"` `.alert-danger` line on the page.
   Render the picker **outside** the `{:else if info}` branch, so a My Info load error (or the
   "Loading…" state) never hides it.
   Keep `writeText(value)`, `copy(info.ticket, …)` and `copy(info.node_id, …)` exactly as they are
   (`clipboard-guard.test.ts` checks them). The full My Info re-layout is 07-02.

## Tests

### Rust (`core`)

- `core/src/identity/theme.rs`: parse/round-trip tests (above).
- `core/src/storage/tests.rs`, e.g. `theme_migration_upgrades_v4_database`: open with
  `open_with(&dir, &k, &MIGRATIONS[..4])`, insert an `account_settings` row, reopen with
  `MIGRATIONS`, assert `user_version == 5` and `theme IS NULL`. Then, over that upgraded DB (e.g. in
  `core/src/identity/settings_sql/tests.rs` or `service.rs` tests, with the marker written),
  assert `SqlCipherSettingsStore::get_theme` returns `None` and `IdentityService::theme()` returns
  `Theme::Midnight`. `migrations_are_contiguous_and_match_schema_version` and
  `dd04_database_opens_unchanged` stay green.
- `core/src/identity/settings_sql/tests.rs`: extend `settings_contract` (run by both
  `memory_store_meets_the_contract` and `sql_store_meets_the_contract`) with: default `None`; set
  and get each of `Theme::ALL`; setting the theme leaves display name, provisioned_at, presence and
  relay unchanged and vice versa. Add `sql_theme_check_rejects_out_of_range` (a raw
  `UPDATE account_settings SET theme = 'blue'` must fail — add a non-unwrapping variant of
  `raw_update`), `sql_theme_persists_across_instances`, and a hostile-value test showing an
  unparseable value reads `None` if the `CHECK` were bypassed (only if you can construct it; skip
  otherwise and say so in the PR).
- `core/src/identity/service.rs` tests: `theme_defaults_to_midnight_after_provision`,
  `set_theme_round_trips`, `theme_requires_an_identity` (both return `ThemeError::NotProvisioned`).
- `core/tests/integration/storage_namespacing.rs`: extend `write_all` with `set_theme(Dark)` and
  `assert_sees_nothing` with `get_theme == None` (a second account's theme is independent).
- `sql_namespacing_guard.rs` and `log_redaction_guard.rs` unchanged and green.

### Rust (`app`)

In `app/src-tauri/src/identity_cmd.rs` tests: `theme_get_on_fresh_account_is_midnight`,
`theme_set_then_get_returns_the_value` (each of the three), `theme_set_unknown_is_invalid_theme_and_changes_nothing`
(`"Dark"`, `"system"`, `""`, `"light "`, `"midnight'; DROP TABLE x;--"` → `InvalidTheme`, stored
value unchanged), `theme_before_provisioning_is_internal`, and extend `errors_serialize_with_type_tag`
with `{"type":"InvalidTheme"}`. Repeat get/set against `SqlCipherSettingsStore` in the style of
`first_run_flow_end_to_end_with_encrypted_settings` (persists across a rebuilt service). The
`first_run_flow_end_to_end` theme step is added in 07-02.

### Frontend guards (Vitest, `app/src/*.test.ts`)

Read sources with `import.meta.glob(..., { query: "?raw", import: "default", eager: true })` like
`html-guard.test.ts`.

- `stylesheet-guard.test.ts` — single stylesheet: no `.svelte` under `/src` contains `<style`,
  `/\sstyle=/` or `/\sstyle:/`; `import.meta.glob("/src/**/*.css")` keys equal `["/src/app.css"]`;
  among non-test `.ts`/`.svelte` files exactly one imports a `.css` file, and it is `/src/main.ts`;
  `index.html` contains `data-theme="midnight"` and no `<style` / `rel="stylesheet"`. Also the
  **no-hardcoded-colour** rule: strip the three `[data-theme="…"] { … }` blocks from `app.css`,
  then assert no `/#[0-9a-f]{3,8}\b/i`, `/\b(rgba?|hsla?)\(/`, or named colour word (at least
  `white|black|red|green|blue|gray|grey|orange|yellow|purple|pink|brown|navy|teal|silver|maroon|olive|lime|aqua|fuchsia`
  as a property value) remains; the same colour checks over every `.svelte` file; and no
  `color-mix(` or `:has(` anywhere in `app.css` or any `.svelte`.
- `theme-contrast-guard.test.ts` — parse the three theme blocks of `app.css`; for each theme
  assert ≥ 4.5 for `--text`, `--text-strong`, `--text-muted`, `--accent`, `--success`, `--warning`,
  `--danger` against `--bg` and `--surface`; each of `--accent`, `--success`, `--warning`, `--danger`
  against its `-soft` tint composited over `--surface` (and, as suggested above, over `--bg`);
  `--bubble-in-text` on `--bubble-in`,
  `--bubble-out-text` on `--bubble-out`; `--accent-contrast` on `--accent`. Helper:

  ```ts
  const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  const lin = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const lum = ([r, g, b]: number[]) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
  const ratio = (a: number[], b: number[]) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  // rgba(r, g, b, a) over an opaque background
  const over = (fg: number[], a: number, bg: number[]) => fg.map((v, i) => v * a + bg[i] * (1 - a));
  ```
  Also assert each `-soft` value equals `rgba(<that colour>, .14)` and `--focus` uses `.40`, so the
  precomputed tints cannot drift from the reference formula.
- `theme-drift-guard.test.ts` — every one of the 19 reference tokens in each theme block of
  `app.css` equals the value in `design/visual-identity.html` (case-insensitive hex, whitespace
  normalised for `--shadow`; note the reference's dark block starts with `:root,`). The app's
  theme blocks contain exactly the 19 reference names + the agreed app-only list, and the same set
  in all three themes. Load the reference with `import sheet from "../../design/visual-identity.html?raw"`
  (add `/// <reference types="vite/client" />`). If Vite refuses the file outside the app root,
  allow only that directory for tests (`server.fs.allow` in `vite.config.ts` under `mode === "test"`),
  not a broader path; ask if neither works.
- `web-storage-guard.test.ts` — no `localStorage`, `sessionStorage`, `document.cookie` or
  `indexedDB` in any non-test `.ts`/`.svelte` under `/src`.
- `html-guard.test.ts`, `clipboard-guard.test.ts`, `render-guard.test.ts` unchanged and green.

### Frontend components

- `app/src/lib/theme.test.ts`: `applyTheme` sets the attribute for each allowlisted value and falls
  back to Midnight for `"blue"`, `""`, `undefined`, `'midnight" onload="x'`; `loadTheme` applies the
  fetched value, falls back to Midnight on rejection, and prefers a set `pendingTheme`;
  `chooseTheme` applies immediately, calls `identity_theme_set` with the value, clears
  `pendingTheme` on success, reverts to the previous theme and returns the error on failure.
- `app/src/views/ThemePicker.test.ts`: a radiogroup named "Theme" with three radios "Midnight",
  "Dark", "Light"; the current one `aria-checked="true"`; ArrowRight/ArrowLeft move focus and call
  `onChoose` with the right value (with wrap); click calls `onChoose`.
- `app/src/App.test.ts`: add `identityThemeGet` (and `identityThemeSet`) to the existing
  `vi.mock("./identity", …)` factory; new test: Present + storage Ready calls `identityThemeGet`
  before the tab list appears, and a rejected `identityThemeGet` still shows the main screen in
  Midnight; Absent never calls it.
- `app/src/views/MyInfo.test.ts`: picker present; choosing Dark calls `identity_theme_set("dark")`;
  a failed save shows the alert and reverts `data-theme`; existing copy tests unchanged.
- Update any component test that asserted removed inline styles (e.g. `ContactRow.test.ts` "renders
  nickname bold with the NodeId small and gray beneath" may only assert structure now).

## Guardrails (AGENTS.md, enforced — do not weaken)

- Secret types (`SecretKeyBytes`, `DbKey`) are never named anywhere under `app/`, not even in
  comments or types; the pre-commit grep gate rejects it. The theme code needs neither.
- Clipboard (`writeText` / `clipboard`) only in `app/src/views/MyInfo.svelte` (and tests); the
  only `{@html}` stays the QR in `MyInfo.svelte`.
- The pre-commit clipboard gate greps `writeText|clipboard` over all of `app/src` except
  `*.test.ts` — including `app.css` and comments — so never write those words anywhere under
  `app/src` except `MyInfo.svelte` and tests. Likewise `render-guard` forbids the substrings
  `location`, `href`, `<a` and `window.open` in its guarded files, comments included.
- Logs never contain message plaintext, private keys or secrets; logging only through
  `core::logging`; the redaction guard stays green (no new fields without review).
- All persisted user data only in the SQLCipher `messenger.db` through `core::storage`. The theme is
  **never** in `localStorage`, `sessionStorage`, cookies, IndexedDB or any file.
- New table data is namespaced by `account_id` (the column sits on `account_settings`); migrations
  are forward-only; never edit an existing migration.
- `core` keeps `#![forbid(unsafe_code)]`.
- CSP in `app/src-tauri/tauri.conf.json` stays exactly
  `default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:` — no widening, no
  `font-src`, no remote origin, no web fonts, CDN CSS or remote images.
- Wire format unchanged; **no `PROTOCOL_VERSION` bump** (the theme never leaves the device).
- Keychain-dependent tests use the in-memory doubles (`FakeKeychain`, `MemorySettingsStore`),
  never the real OS keychain.
- The `headless` crate and `FileKeychain` are never referenced from `app/`.
- No empty files or folders, stub modules or placeholder files; only create files with real
  content. Every exported function is used or tested in this step.
- Never write credentials or secrets in code; all credentials must be provided at runtime (none
  are needed for this work).

## Security review

| Risk | Prevention | Verify |
|---|---|---|
| Theme value injected into CSS/attributes/HTML | `applyTheme` maps through the `THEMES` allowlist before writing `dataset.theme`; no class/style built from the value; the swatch `data-theme` comes from the constant `THEMES` | `theme.test.ts` hostile-value cases; `grep -rn "dataset.theme" app/src --include=*.ts --include=*.svelte` → only `lib/theme.ts` (+ tests) |
| Theme value injected into SQL | `Theme` enum is the only thing reaching SQL, bound as `?2`; `CHECK` constraint as a second wall | settings CHECK test; `sql_namespacing_guard`; `grep -n "format!" core/src/identity/settings_sql.rs` → none |
| Bad input producing an untyped/leaky error | `identity_theme_set` takes `String`, parses with `Theme::parse`, returns `InvalidTheme`; DTO only carries `&'static str` | `theme_set_unknown_is_invalid_theme_and_changes_nothing` |
| Theme persisted outside the encrypted DB | `pendingTheme` is a JS store only | `web-storage-guard.test.ts`; `grep -rnE "localStorage|sessionStorage|indexedDB|document\.cookie" app/src --include=*.ts --include=*.svelte | grep -v test` → empty |
| CSP widened / remote styling fetched | one local stylesheet bundled by Vite; no `@import`, `url(http`, font files | `git diff main -- app/src-tauri/tauri.conf.json` → empty; `grep -nE "@import|url\(|https?://" app/src/app.css` → empty |
| Unverified contact looking verified in some theme | badges keep text labels; contrast tested per theme; `--success` only for positive trust/delivery | `theme-contrast-guard.test.ts`; `TrustBadge.test.ts` unchanged and green |
| Destructive confirmation weakened by the restyle | dialog structure and texts unchanged, only order/classes; Clear history keeps Cancel `autofocus` | existing dialog tests green; read the diff of each dialog |
| Secret type named in app | none needed | the hook's grep gate; `grep -rnE "SecretKeyBytes|DbKey" app --exclude-dir=node_modules --exclude-dir=target --exclude-dir=gen` → empty |

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cd app && npm ci && npm test && npm run check && npm run build
```
Then commit through the pre-commit hook (`git config core.hooksPath .githooks`). Never use
`--no-verify`; fix any failure instead. Manually: `npm start` on a fresh `--profile`, create an
identity, switch the three themes on My Info, relaunch and confirm the theme persists; a second
profile keeps its own theme.

## Definition of done

- [ ] DD inputs (`design/`, DD-07, DD-08, `design_docs/README.md`, `design_docs/steps/07-*.md`)
      committed on `07-visual-identity-and-ux-redesign`; step PR targets that branch.
- [ ] Light `--success` set to `#1C7347` (user decision) and applied consistently to both
      the reference and the app stylesheet.
- [ ] `app/src/app.css` is the only stylesheet, imported once from `main.ts`; `index.html` has
      `data-theme="midnight"`; no `.svelte` has `<style>`, `style=` or `style:`.
- [ ] No `color-mix()` / `:has()`; no colour literal outside the theme blocks; reduced motion rule.
- [ ] Migration 5 with the `CHECK`; `SCHEMA_VERSION = 5`; `Theme` enum; `get_theme`/`set_theme` on
      every `SettingsStore` impl; `IdentityService::theme()/set_theme()`.
- [ ] `identity_theme_get` / `identity_theme_set` implemented, registered in `generate_handler!`,
      invoked by `lib/theme.ts` via `identity.ts`, and tested; `InvalidTheme` typed.
- [ ] `theme.ts` and `ThemePicker.svelte` done; picker on My Info; `App.svelte` loads the theme
      before the main screen.
- [ ] All listed Rust, guard and component tests added and green; `html-guard`,
      `clipboard-guard`, `render-guard`, log-redaction and SQL-namespacing guards green.
- [ ] Text unchanged everywhere (new strings only: "Theme", "Midnight", "Dark", "Light").
- [ ] README: a short note that the theme is chosen on My Info and stored per account in
      `messenger.db` (full README rewrite is 07-03).
- [ ] The app is usable at the end of this step and every screen is reachable (wizard, Contacts,
      My Info, Network, chat, every dialog and prompt, error/storage screens) in all three themes.
- [ ] Pre-commit hook passes; PR opened against the DD branch per the template (crates: `core`
      identity + storage, `app`; wire format unchanged, no `PROTOCOL_VERSION` bump; how verified).

## Open questions

1. **Light `--success`** — resolved: `#1C7347` in both files (user decision, see above).
2. The DD's command table lists errors `NotProvisioned` / `StoreUnavailable` for the theme
   commands, but `IdentityCmdError` has no such variants (identity errors map "not provisioned" and
   settings failures to `Internal`). This step follows the existing mapping and adds only
   `InvalidTheme`. Ask if new variants are wanted.
3. The DD says the input is a "serde enum" yet must return `InvalidTheme` "when deserialisation
   fails". A serde-enum command argument would fail inside Tauri before the handler runs and return
   Tauri's generic error, so the command takes a `String` and parses it into the closed enum.
4. A failed theme save during setup "still applies this session": implemented by `loadTheme()`
   preferring a still-set `pendingTheme` over the stored value.
