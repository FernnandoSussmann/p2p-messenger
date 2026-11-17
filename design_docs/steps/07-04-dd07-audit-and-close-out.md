# Step 07-04 — DD-07 audit and close-out

**Size:** S–M — audit-driven. Expected +0 to +250 / −50 lines (fixes for any gap found, mostly
frontend tests or text), plus docs: DD-07 Status/implementation notes, `design_docs/README.md`
status paragraph, and the deletion of the four `design_docs/steps/07-*.md` files (≈ −1,700 lines of
Markdown). Rust: none expected.

## TL;DR

Last of four sequential steps for [DD-07](../07-visual-identity-and-ux-redesign.md). Check every
item of the DD's **Goals**, **Security and privacy**, **Tests**, **Text inventory** and **Definition
of done** against the code on the DD branch; implement whatever is missing (no new scope); confirm
`design/wireframes/` is gone with no reference left; run the full pre-commit gate; record the
outcome in the DD (Status + short implementation notes with any deviations); then delete all DD-07
step files and open the DD branch → `main` PR.

## Mandatory process rules

- After each step you implement and after each subagent execution completes, run the /usage-guard skill (Skill tool, skill: "usage-guard") to check plan usage; if it says to wait, wait until the limit resets before continuing. Pass this exact instruction on to every subagent you create, including subagents' subagents.
- If in doubt, ask; do not assume.
- The DD is the spec. A gap is fixed only if the DD asks for it; anything else goes into the report
  as a question, not into code.
- Never write credentials or secrets in code, tests, fixtures or docs. Every credential must be
  provided at runtime; this work needs none.

## Prerequisites and branch — work on the last worked DD branch, NOT `main`

07-01, 07-02 and 07-03 must be merged into the DD branch. `main` may **not** contain any of this
work yet, so never start from `main`.

1. `git fetch origin --prune`, then determine the last worked branch for this DD:
   - `git branch -a --sort=-committerdate | head -15`
   - `git log --oneline --all --decorate -20`
   - `gh pr list --state all --limit 15` (look for the 07-01/07-02/07-03 step PRs and their base)
2. Normally this is `07-visual-identity-and-ux-redesign` with the three step PRs merged. If the most
   recent DD-07 work is on an unmerged step branch (e.g. `step-07-03-…` with an open PR), stop and ask
   whether to merge it first or to audit on top of it.
3. `git checkout 07-visual-identity-and-ux-redesign && git pull --ff-only`, then
   `git checkout -b step-07-04-dd07-audit-and-close-out`.
4. Hook installed: `git config core.hooksPath .githooks`.

## Parallel vs sequential

Across steps: this is the last of **07-01 → 07-02 → 07-03 → 07-04**, strictly after 07-03.
Inside this step:

- **Parallel (read-only audit):** the five checklists below are independent; sub-agents may audit
  them in parallel (Rust/storage; stylesheet/guards/themes; navigation/setup/shell; Contacts,
  conversation and contact info; text inventory + README). Each returns a list of gaps with file
  and line, and changes nothing.
- **Sequential:** fixes (one at a time, each with its test), then docs, then the gate, then deleting
  the step files, then the PRs. Do not delete the step files before the gate passes.

## Tasks

### 1. Audit — Definition of done (DD §Definition of done)

For each line, find the code/test that proves it and note the path:

- [ ] One global stylesheet (`app/src/app.css`) imported once from `app/src/main.ts`;
      `app/index.html` has `data-theme="midnight"`; no `.svelte` has `<style>`, `style=` or `style:`
      (`grep -rnE "<style|\sstyle=|\sstyle:" app/src --include=*.svelte` → empty).
- [ ] Token values equal `design/visual-identity.html` (drift guard); no `color-mix()` / `:has()`
      (`grep -rnE "color-mix\(|:has\(" app/src app/index.html` → empty).
- [ ] Migration 5 adds `account_settings.theme` with the `CHECK`; `Theme` enum; every
      `SettingsStore` impl (`grep -rn "impl.*SettingsStore for" --include=*.rs . | grep -v target`,
      each has `get_theme`/`set_theme`); `IdentityService::theme()/set_theme()`.
- [ ] `identity_theme_get` / `identity_theme_set` implemented, in `generate_handler!` in
      `app/src-tauri/src/lib.rs`, invoked from `app/src/identity.ts` → `lib/theme.ts`, and tested.
- [ ] `theme.ts` and `ThemePicker` done; picker on setup and My Info; theme applied before the shell
      renders; Midnight on every pre-account screen.
- [ ] `screen` store with Back chains contact-info → chat → Contacts and focus management.
- [ ] Shell with Contacts | Discover | Network | My Info, footer on tab screens, prompts everywhere.
- [ ] Setup screen (conditional arrow, provision → name → theme → Contacts, retry without
      re-provisioning).
- [ ] Discover hosts the LAN list (inline nickname + Add) and the non-contact Connections group.
- [ ] Network and My Info re-laid out; Network Connections list removed.
- [ ] Contact rows without action buttons; conversation redesigned; `ContactInfo.svelte` with every
      per-contact action, reusing existing commands and dialogs.
- [ ] Text inventory matches exactly; the two listed edits are the only edits to existing text.
- [ ] Single-stylesheet, contrast, drift, no-hardcoded-colour, no-web-storage and extended render
      guards green; `html-guard`, `clipboard-guard`, log-redaction, SQL-namespacing and the hook's
      secret grep gates green; CSP unchanged.
- [ ] All tests listed in the DD added/updated and green.
- [ ] `design/wireframes/` deleted; no Markdown link, import or code reference points into it.
- [ ] README: themes, screen structure, contact-info path and manual checklist updated.
- [ ] Pre-commit hook passes.

### 2. Audit — Goals (DD §Goals)

- Paths: first run setup → Contacts; launch → Contacts; every tab one click from any tab screen;
  contact info two clicks from the list (row → ⋮).
- One source of styling truth; every screen uses the shared classes (spot-check every view).
- Midnight on fresh install, loading, error and storage screens and setup before a choice; theme
  switchable on setup and My Info; survives restart; per account / per `--profile`.
- Contrast ≥ 4.5:1 for the DD's pairs in all three themes (guard). Note any place where muted text
  sits on `--surface-2` or text uses `opacity` (not covered by the guard) and fix it.
- Trust prompts reachable on every screen; trust state visible wherever a contact is shown and never
  by colour alone (text labels on every badge).
- Message timing honest: *sent (their clock) | received (this device)* labelled.
- Nothing that existed before DD-07 became unreachable: walk every action/status/error text of the
  pre-DD-07 UI (`git show main:app/src/views/<File>.svelte` for each view) and find its new home.
- Keyboard and screen reader: real tablist with roving tabindex and arrows, picker arrows, focus
  moved on screen change, Back reachable, focus ring visible in every theme.
- Single column that fits 900×640 and reflows to phone widths (check in `npm start` by resizing).

### 3. Audit — Security and privacy (DD §Security and privacy)

Run and record:

```sh
grep -rnE "localStorage|sessionStorage|indexedDB|document\.cookie" app/src --include=*.ts --include=*.svelte | grep -v "\.test\.ts"   # empty
grep -rn "dataset.theme" app/src --include=*.ts --include=*.svelte | grep -v "\.test\.ts"                                           # only lib/theme.ts
grep -rn "{@html" app/src --include=*.svelte                                                                                       # only views/MyInfo.svelte
grep -rlE "writeText|clipboard" app/src --exclude='*.test.ts'                                                                       # only views/MyInfo.svelte
grep -rnE "SecretKeyBytes|DbKey" app --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=target --exclude-dir=gen         # empty
grep -rnE "FileKeychain|messenger-headless|messenger_headless" app --exclude-dir=node_modules --exclude-dir=dist --exclude-dir=target --exclude-dir=gen  # empty
grep -rnE "@import|url\(|https?://" app/src/app.css                                                                                 # empty
grep -rn "\.style\." app/src --include=*.svelte --include=*.ts | grep -v "\.test\.ts"                                              # only style.height in Composer.svelte
git diff main -- app/src-tauri/tauri.conf.json                                                                                      # empty (CSP unchanged)
git diff main -- protocol/                                                                                                          # empty (no wire change, no PROTOCOL_VERSION bump)
```
Then confirm by reading code/tests: closed theme enum end to end (TS allowlist, `Theme::parse`,
bound SQL parameter, `CHECK`); peer strings only as text with `<bdi>`/`dir="auto"` and never in
`href`/`src`/`style`/class/`title`/`{@html}`; self-chosen names never a heading; destructive actions
keep their confirmations (trash only opens the remove dialog, Clear history keeps Cancel
autofocused, Add disabled for an empty nickname, ticket Connect for an empty field); no stale access
after removal (navigation on removal, everything but Back disabled on external removal); search
local and uncached; setup never overwrites on `AlreadyProvisioned` and never re-provisions; no new
log line without a reviewed allowlisted field; theme only in the per-profile DB.

### 4. Audit — Tests (DD §Tests)

Map every bullet of the DD's Tests section (Rust `core`, Rust `app`, Frontend guards, Frontend
components) to a concrete test name and file; list missing ones and add them. Include: migration
from schema 4 with `theme = NULL` → Midnight; SQL round-trip and `CHECK` rejection; settings contract
on both stores; `NotProvisioned` and per-account independence; `identity_theme_get` fresh =
`midnight`, set/get, unknown → `InvalidTheme` with nothing changed; `first_run_flow_end_to_end`
theme step; render-guard exact count 5; every component bullet (theme.ts, ThemePicker, App, Router,
Shell, Setup, Discover, Contacts, ContactRow, Network, MyInfo, Chat, ContactInfo).

### 5. Audit — Text inventory (DD §Text inventory)

Compare the rendered strings with `main` (`git diff main -- app/src | grep -E '^[+-]'`): every
string removed from a view must be in the inventory's Removed/Moved rows; every new string must be
in its New rows; "Reused" strings appear at their new places; the two edits are exact:

- `No contacts yet. Add someone from <em>Discover</em>, connect with a ticket in the Network tab, or accept an incoming request.`
- `…or when you use Connect in their contact info or paste their ticket in Network.`

Fix any drift (no other wording changes) and check README examples use the same words.

### 6. Wireframes

If 07-03 did not do it: `git rm -r design/wireframes/` and remove the `**Wireframes:**` header
line of DD-07 and any link to the folder. Verify: `ls design/` → only `visual-identity.html`;
`git grep -n "wireframes" -- . ':!design_docs/07-visual-identity-and-ux-redesign.md' ':!design_docs/steps/07-*.md'`
→ empty; `git grep -nE "\]\([^)]*wireframes"` → empty. Repeat the first grep without the
`steps` exclusion after task 9 deleted the step files.

### 7. Gate

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cd app && npm ci && npm test && npm run check && npm run build
```
Then commit the fixes through the pre-commit hook (`git config core.hooksPath .githooks`). Never use
`--no-verify`; fix failures instead. Run the README manual checklist once with two instances
(`--profile a` / `--profile b`), at least in Midnight and Light, and note the result in the PR.

### 8. Close-out docs

- DD-07 header: set `**Status:**` to "Implemented" and add a short "Implementation notes" list of
  the deviations and interpretations actually taken. Only list what is true in the code;
  candidates from the step files:
  - Light `--success` changed to `#1C7347` (user decision) and the extra contrast pair (tint over `--bg`);
  - `identity_theme_set` taking a string so a bad value returns `InvalidTheme`;
  - theme errors use the existing `IdentityCmdError` mapping (`Internal` / `RecoverableErrorState`),
    no `NotProvisioned` / `StoreUnavailable` variants;
  - `loadTheme()` prefers an unsaved `pendingTheme` (failed save during setup applies this session);
  - `AlreadyProvisioned` on setup also skips the theme write (and clears `pendingTheme`);
  - the Connections move done with the Network re-layout in 07-02, not 07-03;
  - the menu hidden on drill-in screens (user decision);
  - focus stays on the tab for arrow-key tab changes;
  - `Discover.svelte` as a new file;
  - the `<em>Discover</em>` markup in the edited empty state;
  - the ` | ` / `·` separators;
  - removed contacts are never displayed (drill-ins redirect to Contacts, "This contact was
    removed." dropped, no Clear history for removed contacts); kept history returns when the same
    NodeId is re-added (user decision).
- `design_docs/README.md`: update the status paragraph ("DD-07 … and DD-08 … are drafts") to say
  DD-07 is implemented and DD-08 is the next draft.

### 9. Delete the DD-07 step files, then PRs

- Only after the gate passes: `git rm design_docs/steps/07-*.md` (this file included). Do **not**
  touch the unrelated files in `design_docs/steps/` (e.g. `02-*.md`). Commit through the hook.
- Open the step PR into `07-visual-identity-and-ux-redesign` per `.github/PULL_REQUEST_TEMPLATE.md`
  (affected: `core` identity + storage, `app`, docs; wire format unchanged, no `PROTOCOL_VERSION`
  bump; how verified).
- After it merges, open the DD PR `07-visual-identity-and-ux-redesign` → `main` (same template,
  summarising the whole DD). Ask the user before merging into `main`.

## Tests

No new scope. Every gap found in task 4 gets the test the DD's Tests section names (Rust in the
crate's existing test module or integration binary, frontend next to the component as
`*.test.ts`). The whole existing suite, all guards and the hook's grep gates must stay green; no
guard is relaxed to make a test pass.

## Guardrails (AGENTS.md, enforced — do not weaken)

- Secret types (`SecretKeyBytes`, `DbKey`) never named under `app/` (pre-commit grep gate).
- Clipboard only in `app/src/views/MyInfo.svelte` (and tests); the only `{@html}` is the QR there.
  The hook's clipboard gate greps `writeText|clipboard` over all of `app/src` except `*.test.ts`
  (CSS and comments included); `render-guard` forbids `location`, `href`, `<a`, `window.open` in its
  five files, comments included — keep that in mind for any audit fix.
- Logs never contain message plaintext, private keys or secrets; logging only via `core::logging`;
  the redaction guard's allowlist only grows by reviewed, non-sensitive names.
- All persisted user data only in the SQLCipher `messenger.db`; no `localStorage`,
  `sessionStorage`, cookies, IndexedDB or files for the theme or anything else.
- `core` forbids unsafe; migrations forward-only; every row namespaced by `account_id`.
- CSP unchanged; no web fonts, CDN CSS, remote images or new origins.
- Wire format unchanged; no `PROTOCOL_VERSION` bump. DD-08 is not part of this work.
- Keychain-dependent tests use the in-memory doubles; `headless`/`FileKeychain` never referenced
  from `app/`.
- No empty files or folders, stub modules or placeholders (also: no empty `design/wireframes/` or
  `design_docs/steps/` left behind by deletions if they become empty — `design_docs/steps/` keeps the
  unrelated files).
- Never write credentials or secrets in code; all credentials must be provided at runtime (none are
  needed for this work).

## Security review

Specific to an audit/close-out step:

| Risk | Prevention | Verify |
|---|---|---|
| A "quick fix" during the audit weakens a guard (lowering the contrast threshold, widening a glob, adding an allowlist entry, skipping a test) | fix the code, never the guard; any guard change needs the user's approval | `git diff 07-visual-identity-and-ux-redesign -- app/src/*-guard.test.ts core/tests/integration/*guard*.rs .githooks/` reviewed line by line |
| CSP or wire format changed somewhere in the DD | compare against `main` | the two `git diff main -- …` commands in task 3 → empty |
| Destructive actions lost their confirmation during the redesign | read the remove/clear paths end to end | ContactInfo tests; manual checklist |
| Trust prompts hidden on some screen | shell renders them on every screen | Shell tests across 4 tabs + chat + contact info; manual "pending request while navigating" |
| Theme or other data leaking outside `messenger.db` | web-storage guard + grep | task 3 greps |
| Step files deleted before the work is verified | delete only after the gate passes | order of tasks 7 → 9 |
| Merging an unaudited branch into `main` | DD → main PR only after the step PR merges; ask before merging | PR history |

## Quality gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cd app && npm ci && npm test && npm run check && npm run build
```
Every commit goes through the pre-commit hook; never `--no-verify`.

## Definition of done

- [ ] Worked on a step branch off the last worked DD branch (not `main`).
- [ ] Every item of the DD's Definition of done, Goals, Security and privacy, Tests and Text
      inventory checked, with evidence (path or test name) listed in the PR description; every gap
      fixed with a test, or raised as a question.
- [ ] `design/wireframes/` deleted; no reference remains (greps above).
- [ ] Full pre-commit gate passes; manual checklist run with two instances.
- [ ] DD-07 Status "Implemented" with accurate implementation notes; `design_docs/README.md` status
      paragraph updated.
- [ ] All `design_docs/steps/07-*.md` deleted; unrelated step files untouched.
- [ ] The app is usable at the end of this step and every screen is reachable (setup, Contacts,
      Discover, Network, My Info, conversation, contact info, every dialog and both prompts, error
      and storage screens) in all three themes.
- [ ] Step PR into the DD branch, then the DD → `main` PR opened (merge to `main` only with the
      user's go-ahead).

## Open questions

None new. Carry forward any still-open question from 07-01 to 07-03 that the user has not answered
and list it in the final PR description, in particular:

(Already answered by the user and not to be re-asked: Light `--success` = `#1C7347`; menu hidden
on drill-in screens; removed contacts never displayed, history restored on re-add.)

1. Theme command errors: the DD names `NotProvisioned` / `StoreUnavailable`, the code uses the
   existing `IdentityCmdError` mapping (07-01 OQ2).
2. Whether DD-07's prose mentions of the deleted wireframes should be reworded (07-03 OQ3).
