# Step 03-01 — `core::contacts` slice, contacts-backed trust gate, Accept-with-nickname

**Design doc:** [DD-03 Contacts & Trust](../../03-contacts-and-trust.md)
**Depends on:** DD-02 merged (it is, on `main`). Nothing in DD-03 comes before
this step.
**Blocks:** 03-02 (contact commands and UI) and 03-03 (audit).
**Branch:** `step-03-01-contacts-slice-and-trust-gate`, created from an
up-to-date `main`. It is the bottom of the DD-03 stack (03-01 → 03-02 → 03-03):
03-02 branches from this branch, so don't wait for this PR to merge before
starting 03-02.
**Commit this whole step folder (`design_docs/steps/03-contacts-and-trust/`)
with this step's PR**, so that 03-03 can `git rm` it later.

## Goal

When this step is done:

- A new `core::contacts` slice exists. It owns the contact record: NodeId, local
  nickname, last-seen self-asserted name (`self_name`), trust state and
  timestamps. Records are keyed by `(account_id, NodeId)`.
- Contacts are the **only** source of trust. DD-02's trust gate asks
  `ContactBook::is_known`, which is the same function the `contacts_is_known`
  command uses in 03-02. The interim `FileKnownPeers` / `known_peers.json` store
  is gone. Its entries are migrated into contacts at startup, so peers the user
  already accepted or dialed still connect without a prompt.
- **Inbound Accept is the "add contact" action** (user decision E, extended
  to Accept). The nickname field is **prefilled with the default nickname**,
  `Nickname::default_for(id, self_name)`. The user can edit it, and the peer's
  self-asserted name is still shown for context. Nothing happens until the user
  explicitly clicks Accept; there is no auto-accept. The contact is saved as
  **Unverified**, and only then is the prompt answered.
- **A successful ticket/LAN dial is remembered** (user decision E). When the
  peer replies with a valid Hello, the node asks the contacts slice to remember
  it. If no contact exists yet, the slice creates an **Unverified** contact with
  the **default nickname**. Naming and confirmation in the UI come in 03-02.
- **Default nickname** (user decision C): the peer's validated self-asserted
  name, falling back to `Peer <short NodeId>` when the name is absent or
  invalid. It is a snapshot taken once. Later peer renames update only
  `self_name`, never the nickname.
- **Name validation is tightened** (user decision D, refined). The shared
  DD-01 validator also rejects zero-width and invisible format characters. The
  one exception is well-formed emoji sequences: a ZWJ *between two emoji*, and
  a tag sequence `🏴 + tags + cancel tag`. This tightens DD-01 display names
  too. Names stored before the change never break startup.
- Advisory names are observed from both the **Hello** (on authorized
  connections) and **LAN/mDNS** records, but only for existing contacts. They
  update `self_name` only.
- `contacts_remove(…, clear_history)` support exists in core: a `HistoryEraser`
  seam, an honest production implementation (no message store exists yet) and
  a recording test double.
- The app still works end to end. The Accept prompt asks for a nickname,
  accepted and dialed peers reconnect silently (also after a restart), and the
  Network copy "this device is remembered" stays true. The contact list UI comes
  in 03-02.

### Storage deviation (decided by the user; document it everywhere)

DD-05's SQLCipher DB does not exist yet. Contacts go to an **interim
plaintext** file, `<data_dir>/contacts.json`, behind a `ContactStore` trait. The
file follows the existing `FileSettingsStore` / `FileKnownPeers` pattern:

- a versioned document with `deny_unknown_fields`;
- size and count caps;
- atomic writes, mode 0600, symlinks refused;
- a corrupt file is an error, never treated as empty (fail closed).

**Nicknames and self-asserted names are on disk in plaintext until DD-05
replaces this store with the encrypted `contacts` table.** State this in the
module docs of `core/src/contacts/store.rs` and in `README.md`.

---

## Work breakdown and parallelism

```
          ┌── 1A  core::contacts slice (pure, additive) ───────────┐
start ────┼── 1B  discovery seam additions (additive) ─────────────┼──▶ 1C integration (sequential)
          └── 1D  DD-01 validator tightening (identity + its UI) ──┘
```

**1A, 1B and 1D run in parallel** in separate git worktrees off this step's
branch. Each is additive and keeps every gate green on its own. **1C starts
only after all three are merged** into the step branch. 1C makes the breaking
switch (trust moves to contacts) and wires the app, headless and the frontend.

### File ownership (no two parallel sub-tasks touch the same file)

| Sub-task | Owns (creates / edits) | Must not touch |
|----------|------------------------|----------------|
| 1A | `core/src/contacts/**` (new); `core/src/lib.rs` (add `pub mod contacts;`); `core/src/identity/marker.rs` (add `pub(crate) fn from_uuid`); `core/tests/contacts_log_guard.rs` (new) | `core/src/discovery/**`, the other `core/src/identity/*` files, `app/**`, `headless/**` |
| 1B | `core/src/discovery/**`; `core/tests/discovery_*.rs`; `..`/`display_name: None` fixes at exactly `app/src-tauri/src/net_cmd.rs` (the `is_state` matcher in tests), `headless/src/node.rs` (test `wait_selectors_are_names_produced_by_event_json`, struct construction) and `headless/tests/control.rs` (struct construction) | `core/src/contacts/**`, `core/src/identity/**`, `core/src/lib.rs`, everything else in `app/` and `headless/` |
| 1D | `core/src/identity/display_name.rs`; `core/src/identity/emoji.rs` (new, `pub(crate)`); `core/src/identity/settings.rs`; `app/src-tauri/src/identity_cmd.rs` (`DisplayNameKind` only); `app/src/identity.ts` (`DisplayNameKind` type); `app/src/lib/displayNameMessages.ts`; the DD-01 tests these affect (`core/tests/identity_integration.rs`, `app/src/views/FirstRunWizard.test.ts`, `app/src/views/MyInfo.test.ts`) | `core/src/discovery/**`, `core/src/contacts/**`, everything else |
| 1C | everything else listed under 1C | — |

---

## Sub-task 1D — tighten the shared DD-01 name validator (parallel)

### Rule added to `validate_display_name`

After the existing bidi check, reject every **invisible format character**,
except inside the two well-formed emoji constructs below. Rejections use a new
variant, `DisplayNameError::InvisibleCharacter`.

**The candidate set** is all of Unicode general category Cf, plus U+180E:

| Range | What |
|-------|------|
| U+00AD | soft hyphen |
| U+0600–U+0605, U+06DD, U+070F, U+0890–U+0891, U+08E2 | Arabic/Syriac format marks |
| U+061C | Arabic letter mark |
| U+180E | Mongolian vowel separator |
| U+200B–U+200F | zero-width space, ZWNJ, ZWJ, LRM, RLM |
| U+2060–U+2064 | word joiner, invisible operators |
| U+206A–U+206F | deprecated format characters |
| U+FEFF | zero-width no-break space / BOM |
| U+FFF9–U+FFFB | interlinear annotation |
| U+110BD, U+110CD | Kaithi number signs |
| U+13430–U+1343F | Egyptian hieroglyph format controls |
| U+1BCA0–U+1BCA3 | shorthand format controls |
| U+1D173–U+1D17A | musical symbol format controls |
| U+E0001, U+E0020–U+E007F | language tag and tag characters |

The bidi set (U+202A–U+202E, U+2066–U+2069) keeps returning `BidiOverride`, so
error precedence is unchanged.

**Emoji exceptions** (user decision). Every other use of a candidate is
rejected.

1. **ZWJ (U+200D) between two emoji.** The character immediately after the ZWJ
   must be `Extended_Pictographic`. The *emoji base* before the ZWJ must also be
   `Extended_Pictographic`, where the emoji base is the preceding character
   after skipping **at most one** trailing modifier: either U+FE0F (variation
   selector 16) or one skin-tone modifier U+1F3FB–U+1F3FF. Consequences:
   - a ZWJ that is leading or trailing is rejected;
   - a doubled ZWJ is rejected, because the character after the first ZWJ is a
     ZWJ, not an emoji;
   - a ZWJ next to a letter is rejected.
2. **Tag sequence.** The sequence is U+1F3F4 (black flag), then **one or more**
   tag characters U+E0020–U+E007E, then exactly one U+E007F (cancel tag). Tag
   characters are allowed only inside such a run, and U+E007F only as its
   terminator. The following are rejected:
   - a lone tag character;
   - a tag run not directly preceded by U+1F3F4;
   - a run without the cancel tag;
   - a cancel tag with no run before it;
   - U+E0001.
3. Nothing else is exempt. U+200B, U+200C, U+2060–U+2064, U+FEFF, U+180E,
   U+00AD and every other row above are always rejected.

**Implementation.** In `validate_display_name`, scan with a `Peekable` over
`char_indices`, keeping the previous one or two characters. This is linear
time, does no allocation beyond the existing trimmed `String`, and uses no
`unsafe`.

**Classifying `Extended_Pictographic` without `unsafe`.** Add a new
`core/src/identity/emoji.rs` with
`pub(crate) fn is_extended_pictographic(c: char) -> bool`, a binary search over
a `const EXTENDED_PICTOGRAPHIC: &[(u32, u32)]` range table.

- **Source of the table:** copy it **verbatim** from Unicode 16.0
  `emoji-data.txt` (property `Extended_Pictographic`, about 80 ranges). Put the
  source URL and the Unicode version in the module doc.
- **Why a table and not a crate:** the table is small, auditable, adds no new
  dependency or supply-chain surface, is `const` data and needs no `unsafe`. A
  crate was considered and rejected; there is no `Cargo.toml` change.
- **If the implementer prefers a crate:** it must be pinned in the root
  `[workspace.dependencies]`, expose `Extended_Pictographic` explicitly (not
  just "is emoji"), and contain no `unsafe` in the code paths used. Justify the
  choice in the PR.
- **Test:** `emoji::table_is_sorted_non_overlapping_and_spot_checked`. It
  asserts the ranges are sorted and disjoint. It also spot-checks:
  - included: U+00A9, U+203C, U+1F600, U+1F3F3, U+1F3F4, U+1F468, U+1FAFF;
  - excluded: `a`, U+1F3FB (skin tone), U+FE0F, U+200D, U+E0067.

**Known, accepted consequences** (state them in the module docs):

- Well-formed emoji ZWJ sequences and subdivision flags are accepted, for
  example 👨‍👩‍👧, 🏳️‍🌈, 🧑🏽‍💻 and 🏴󠁧󠁢󠁥󠁮󠁧󠁿 (England).
- The validator checks *structure*, not whether a sequence is listed in the
  RGI emoji set. A valid-shaped but unlisted sequence (two arbitrary emoji
  joined by ZWJ) is accepted and renders as two emoji. The tag letters inside a
  🏴 sequence are not checked against the ISO 3166-2 subdivision list either.
  This is residual risk R5 in 03-03.
- Single emoji, variation selectors (U+FE0F is category Mn, not Cf), skin-tone
  modifiers (Sk), combining marks and CJK text are accepted as before.
- Homoglyphs stay allowed. They are mitigated by the always-visible NodeId and
  the fixed trust badge.
- Visually blank letters such as U+3164 HANGUL FILLER are not Cf and are
  **not** rejected. This is residual risk R3.

Update the module doc's "Not handled" paragraph accordingly.

### Existing stored names must not brick startup

A display name saved under the old rules may now fail validation, for example
`"Ada\u{200B}"` or a malformed ZWJ use such as `"a\u{200D}b"`. The specified
fallback:

- **`FileSettingsStore::get_display_name`**: `Err(InvisibleCharacter)` maps to
  **`Ok(None)`**, meaning the name is treated as unset. Any other validation
  failure keeps returning `Unavailable` (tampering), as the current test
  `file_store_tampered_display_name_fails_validation` requires.
- Effects of an unset name:
  - My Info shows an empty name field and the user sets a new one.
  - Our Hello sends an empty name, and LAN advertising carries no name.
  - Presence, relay and the identity are unaffected, because only
    `get_display_name` reads the name.
- The rest of the settings document stays readable. `read_document` never
  validates names.
- `MemorySettingsStore` holds already-validated `DisplayName`s and needs no
  change.
- Peer names: an incoming Hello or LAN name that now fails decodes to `None`.
  Both `decode_hello` and `decode_lan_name` already call
  `validate_display_name(..).ok()`, so this needs no code change.
- The legacy `known_peers.json` names are handled by 1A's migration reader:
  `InvisibleCharacter` means the entry has no name, so the file is **not**
  treated as corrupt.

### UI surface

- `app/src-tauri/src/identity_cmd.rs`: `DisplayNameKind::InvisibleCharacter`,
  plus its `From<DisplayNameError>` arm.
- `app/src/identity.ts`: add `"InvisibleCharacter"` to `DisplayNameKind`.
- `app/src/lib/displayNameMessages.ts`: `InvisibleCharacter: "Display name
  cannot contain invisible characters (such as zero-width spaces or joiners)."`.
  `svelte-check` enforces the `Record` completeness.

### 1D tests

| Test | Asserts |
|------|---------|
| `display_name::rejected_inputs` (extend) | one sample from every always-rejected range (`"a\u{200B}b"`, `"a\u{200C}b"`, `"a\u{FEFF}b"`, `"a\u{2060}b"`, `"a\u{2064}b"`, `"a\u{180E}b"`, `"a\u{00AD}b"`, `"a\u{061C}b"`, `"a\u{E0001}b"`, …) → `InvisibleCharacter`; the existing bidi cases still → `BidiOverride` |
| `display_name::zwj_outside_emoji_is_rejected` | `"a\u{200D}b"`, `"\u{200D}x"`, `"😀\u{200D}"`, `"😀\u{200D}\u{200D}😀"`, `"a\u{200D}😀"`, `"😀\u{200D}a"`, `"😀\u{FE0F}\u{FE0F}\u{200D}😀"` (two modifiers skipped is not allowed) → `InvisibleCharacter` |
| `display_name::malformed_tag_sequences_are_rejected` | lone `"\u{E0041}"`; `"a\u{E0041}b"`; unterminated `"\u{1F3F4}\u{E0067}\u{E0062}"`; tags without a black flag `"😀\u{E0067}\u{E007F}"`; a bare cancel `"\u{1F3F4}\u{E007F}"`; cancel without a flag `"a\u{E007F}"` → `InvisibleCharacter` |
| `display_name::well_formed_emoji_sequences_are_accepted` | 👨‍👩‍👧 (`"\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"`), 🏳️‍🌈 (`"\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}"`), England 🏴󠁧󠁢󠁥󠁮󠁧󠁿 (`"\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"`), 👍🏽 (`"\u{1F44D}\u{1F3FD}"`), 🧑🏽‍💻 (`"\u{1F9D1}\u{1F3FD}\u{200D}\u{1F4BB}"`) and `"Ana 👨‍👩‍👧"` are accepted verbatim |
| `emoji::table_is_sorted_non_overlapping_and_spot_checked` | see above |
| `display_name::accepted_inputs_are_trimmed_and_verbatim` (extend) | `"❤\u{FE0F}"`, `"e\u{0301}"`, `"日本語"`, `"👋"` accepted |
| `display_name::error_messages_are_distinct_and_nonempty` (extend) | covers the new variant |
| `settings::stored_name_now_invisible_reads_as_unset` | a v1 file whose `display_name` is `"Ada\u{200B}"` gives `get_display_name → Ok(None)`. Presence and relay still read correctly. The file is not rewritten. |
| `settings::tampered_display_name_fails_validation` (existing) | still `Unavailable` for `"a\nb"` |
| `identity_cmd` serialization test | `InvalidDisplayName { kind: InvisibleCharacter }` → `{"type":"InvalidDisplayName","kind":"InvisibleCharacter"}` |
| `FirstRunWizard.test.ts` | `InvisibleCharacter shows the invisible-characters message` |

---

## Sub-task 1A — `core::contacts` slice (parallel)

Do not reference `crate::discovery` anywhere in 1A. The `KnownPeers` impl and
`accept_incoming` come in 1C.

### Files

```
core/src/contacts/
  mod.rs        // slice docs + curated re-exports
  model.rs      // Contact, Nickname, TrustState, HistoryChoice, Removal, NameUpdate, ContactError
  store.rs      // ContactStore trait, ContactStoreError, BatchReport, FileContactStore (interim)
  book.rs       // ContactBook: account-bound façade + business rules
  history.rs    // HistoryEraser trait, HistoryEraseError, NoMessageStore
  migrate.rs    // one-shot import of legacy known_peers.json
  dto.rs        // ContactDto, RemoveContactDto (string-only DTOs for the Tauri shell)
  testing.rs    // #[cfg(any(test, feature = "test-support"))] doubles
```

- `core/src/lib.rs`: add `pub mod contacts;` and extend the crate doc.
- `core/src/identity/marker.rs`: add
  `pub(crate) fn from_uuid(uuid: Uuid) -> Self { Self(uuid) }` on `AccountId`.
  It is crate-visible only, so the public API does not change.

### `model.rs`

```rust
/// What the user calls a contact. DD-01 rules (validate_display_name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nickname(DisplayName);

impl Nickname {
    pub fn parse(input: &str) -> Result<Self, DisplayNameError>;   // = validate_display_name
    /// Default when the user has not typed one (migration, auto-added dialed
    /// peer, headless accept without a name): the peer's validated
    /// self-asserted name, else "Peer <id.fmt_short()>". A snapshot: never
    /// re-derived later.
    pub fn default_for(id: &NodeId, self_name: Option<&DisplayName>) -> Self;
    pub fn as_str(&self) -> &str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustState { Unverified, Verified }   // stored as "unverified"/"verified"

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub node_id: NodeId,                 // identity, primary key, immutable
    pub nickname: Nickname,              // user-controlled; the only name shown in the contact list
    /// Peer's last-seen self-asserted name (Hello or LAN). Advisory only.
    /// Shown in naming/Accept prompts; kept for a future contact-info screen.
    pub self_name: Option<DisplayName>,
    pub trust: TrustState,
    pub added_at_ms: i64,
    pub last_seen_ms: Option<i64>,       // last authorized connection
}

/// How an observation changes `self_name`.
#[derive(Debug, Clone, Copy)]
pub enum NameUpdate<'a> { Keep, Set(Option<&'a DisplayName>) }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryChoice { #[default] Keep, Erase }
impl HistoryChoice { pub fn from_flag(flag: Option<bool>) -> Self; }   // None/false → Keep

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removal { pub history_erased: bool }

#[derive(Debug)]
pub enum ContactError {
    InvalidNickname(DisplayNameError),
    SelfContact,
    AlreadyExists,
    NotFound,
    Full,
    /// Unreadable / corrupt / IO / blocking-task failure. Never "empty" or "unknown".
    Unavailable,
    /// History deletion failed; the contact was NOT removed.
    HistoryEraseFailed,
}
```

`Display` messages never embed names or paths.

### `store.rs` — the trait and the interim file store

```rust
pub const MAX_CONTACTS_PER_ACCOUNT: usize = 10_000;     // ≥ legacy known_peers cap
pub const MAX_CONTACTS_BYTES: u64 = 32 * 1024 * 1024;   // 10k × (2 names ≤ 512 B + id + fields) with headroom
/// `observe` does not rewrite the file just to move last_seen by less than this.
pub const LAST_SEEN_RESOLUTION_MS: i64 = 5 * 60 * 1000;

pub trait ContactStore: Send + Sync + 'static {
    fn list(&self, account: &AccountId) -> Result<Vec<Contact>, ContactStoreError>;
    fn get(&self, account: &AccountId, id: &NodeId) -> Result<Option<Contact>, ContactStoreError>;
    /// Never overwrites: existing NodeId → AlreadyExists.
    fn insert(&self, account: &AccountId, contact: Contact) -> Result<(), ContactStoreError>;
    /// Inserts every entry whose (account, NodeId) is absent, skips existing
    /// ones untouched, in ONE atomic write. Any cap violation → Full and
    /// nothing written. Used by migration.
    fn insert_missing(&self, entries: Vec<(AccountId, Contact)>) -> Result<BatchReport, ContactStoreError>;
    fn set_nickname(&self, account: &AccountId, id: &NodeId, nickname: &Nickname) -> Result<Contact, ContactStoreError>;
    /// The ONLY write path for `trust`.
    fn set_trust(&self, account: &AccountId, id: &NodeId, trust: TrustState) -> Result<Contact, ContactStoreError>;
    /// Advisory update of an EXISTING contact; Ok no-op for unknown ids.
    /// Never touches nickname or trust. Skips the write when `name` leaves
    /// self_name unchanged AND (`seen_at_ms` is None OR the stored
    /// last_seen_ms is within LAST_SEEN_RESOLUTION_MS of it).
    fn observe(&self, account: &AccountId, id: &NodeId, name: NameUpdate<'_>, seen_at_ms: Option<i64>) -> Result<(), ContactStoreError>;
    fn remove(&self, account: &AccountId, id: &NodeId) -> Result<bool, ContactStoreError>;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BatchReport { pub inserted: usize, pub already_present: usize }

#[derive(Debug)]
pub enum ContactStoreError { NotFound, AlreadyExists, Full, Corrupt, Io(std::io::Error) }
```

**`FileContactStore`** is built like today's `core/src/discovery/known_peers.rs`
store (`read_document` / `write_document` / `tmp_path`). Read that file before
1C deletes it.

- **Path and locking.** The path is `<data_dir>/contacts.json`, a constant file
  name and never built from account IDs or names. `new(data_dir)` does no I/O.
  One `Mutex<()>` serializes read-modify-write.
  - **Document that a process must construct exactly one instance per path**,
    because two instances can lose updates. The app and headless each build one
    and share it via `Arc`.
  - Under `#[cfg(test)]`, expose a `writes()` counter for throttle tests.
- **Document format.** All structs use `#[serde(deny_unknown_fields)]`:
  ```json
  {"version":1,"accounts":{"<uuid>":{"<node_id>":{"nickname":"Bobby","self_name":"Bob",
   "trust":"unverified","added_at_ms":1730000000000,"last_seen_ms":null}}}}
  ```
- **`validate()` runs on every read.** Any failure is `Corrupt`, and nothing is
  overwritten. It checks:
  - `version == 1`;
  - account keys are canonical lowercase hyphenated UUIDs;
  - NodeId keys parse **and equal `parsed.to_string()`**;
  - `nickname` passes `Nickname::parse` unchanged (already trimmed);
  - `self_name` passes `validate_display_name` unchanged;
  - `trust` is a known variant;
  - timestamps are `>= 0`;
  - the per-account count is `<= MAX_CONTACTS_PER_ACCOUNT`.
- **Caps.** The byte cap is checked via `symlink_metadata` before reading and
  again on the bytes read. Writing a document larger than the cap is refused
  with `Full`.
- **Missing and odd files.** A missing file is an empty document. A file that is
  not a regular file (a symlink or directory) is `Corrupt`.
- **Atomic write.** Write `contacts.json.tmp` (mode `0o600` on unix), then
  `write_all`, `sync_all`, `rename`, and a best-effort directory fsync. Remove
  the temp file on error.
- **Module doc**, required wording: *"Interim plaintext store, a documented
  deviation from DD-03/DD-05. Nicknames and peers' self-asserted names are
  stored unencrypted in `contacts.json` (mode 0600 in the 0700 app-data dir)
  until DD-05's encrypted `contacts` table replaces this type behind the same
  `ContactStore` trait."*

### `history.rs`

```rust
/// Deletes a peer's stored conversation. DD-05 owns the real implementation
/// (same path as `history_clear`, with page reclaim). Contract: idempotent
/// (erasing nothing is Ok) and all-or-nothing.
pub trait HistoryEraser: Send + Sync + 'static {
    fn erase_peer_history(&self, account: &AccountId, peer: &NodeId) -> Result<(), HistoryEraseError>;
}
#[derive(Debug)] pub struct HistoryEraseError;   // opaque: no paths, names or counts

/// Production eraser until DD-04/DD-05 ship a message store. This build never
/// persists a message, so a peer's history is always empty and erasing it is a
/// successful no-op. DD-05 replaces this type in the app wiring.
pub struct NoMessageStore;
```

`NoMessageStore` is the correct behaviour for a build that has no message
store, not a stub. Its doc comment must say so and name DD-05.

### `book.rs` — `ContactBook`

```rust
/// One account's contacts plus DD-03's rules. Trust reads the NodeId only;
/// names are never compared, looked up or used for dedup.
pub struct ContactBook { account: AccountId, own: NodeId,
    store: Arc<dyn ContactStore>, history: Arc<dyn HistoryEraser> }

impl ContactBook {
    pub fn new(account: AccountId, own: NodeId, store: Arc<dyn ContactStore>, history: Arc<dyn HistoryEraser>) -> Self;
    pub fn account(&self) -> &AccountId;
    /// Sorted by nickname (case-insensitive), then NodeId string.
    pub fn list(&self) -> Result<Vec<Contact>, ContactError>;
    pub fn get(&self, id: &NodeId) -> Result<Option<Contact>, ContactError>;
    /// NodeId-only membership. Err must be treated as "unknown" by callers.
    pub fn is_known(&self, id: &NodeId) -> Result<bool, ContactError>;
    /// User-typed nickname. Validates first (nothing written on failure);
    /// rejects `own` (SelfContact); always Unverified (no parameter to create
    /// a verified contact). `self_name` comes from the node's observation,
    /// never from webview input.
    pub fn add(&self, id: NodeId, nickname: &str, self_name: Option<DisplayName>) -> Result<Contact, ContactError>;
    /// Decision E: a dialed peer answered with a valid Hello. If absent, insert
    /// Unverified with `Nickname::default_for(id, self_name)`; if present,
    /// behave like `observe_hello` (nickname/trust untouched).
    pub fn remember_dialed(&self, id: NodeId, self_name: Option<&DisplayName>) -> Result<(), ContactError>;
    pub fn rename(&self, id: &NodeId, nickname: &str) -> Result<Contact, ContactError>;
    /// The only way trust changes. Explicit local user attestation.
    pub fn set_verified(&self, id: &NodeId, verified: bool) -> Result<Contact, ContactError>;
    /// Hello on an authorized connection: self_name = name (None clears),
    /// last_seen = now (throttled by the store).
    pub fn observe_hello(&self, id: &NodeId, name: Option<&DisplayName>) -> Result<(), ContactError>;
    /// LAN record: self_name = name; last_seen untouched (not a connection).
    pub fn observe_lan(&self, id: &NodeId, name: &DisplayName) -> Result<(), ContactError>;
    pub fn remove(&self, id: &NodeId, history: HistoryChoice) -> Result<Removal, ContactError>;
}
```

Store errors map as follows: `Corrupt` and `Io` become `Unavailable`; `Full`,
`NotFound` and `AlreadyExists` keep the same name.

**Removal order and failure semantics.** These are binding, and 03-02 copies
them into the UI copy. Put the rationale in the doc comment.

1. `get(id)`. If the contact is absent, return `NotFound` and erase nothing.
2. If `Erase`, call `erase_peer_history(account, id)`. If it fails, return
   `HistoryEraseFailed`; the **contact is kept** and trust is unchanged.
3. Call `store.remove(account, id)`. If it fails, return `Unavailable`. History
   may already be erased, which is irreversible. The contact still exists, so a
   retry is safe because erase is idempotent.
4. Return `Ok(Removal { history_erased: history == Erase })`.

With `Keep`, the eraser is **never called**.

Rationale: erasing first means a failed erase can be retried from the contact
row. If the contact were removed first, a failed erase would leave behind
history that the contact UI can no longer reach.

### `migrate.rs`

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport { pub migrated: usize, pub already_present: usize }
#[derive(Debug)]
pub enum MigrationError { LegacyCorrupt, Store(ContactStoreError), Io(std::io::Error) }

/// Imports `<data_dir>/known_peers.json` (DD-02 interim, v1) for every account
/// in it, in one atomic store write, then deletes the legacy file.
pub fn migrate_legacy_known_peers(data_dir: &Path, store: &dyn ContactStore) -> Result<MigrationReport, MigrationError>;
```

- **Legacy reader.** Copy only the legacy *reader* from today's
  `known_peers.rs`: version 1, `deny_unknown_fields`, UUID account keys, NodeId
  keys, 4 MiB cap, symlink refused. A **name** that fails with
  `InvisibleCharacter` (1D) counts as **no name**. Any other validation failure
  is `LegacyCorrupt`.
- **Missing file.** Return `Ok(default)` and create no `contacts.json`.
- **Entries.** Each entry becomes `Contact { nickname:
  Nickname::default_for(&id, name.as_ref()), self_name: name, trust:
  Unverified, added_at_ms, last_seen_ms: None }`, keyed by
  `AccountId::from_uuid`.
- **One write.** All entries go to the store in **one** `insert_missing` call.
  Existing contacts are never overwritten.
- **All or nothing.**
  - If the legacy file is corrupt, return `LegacyCorrupt` and touch nothing.
  - If the store write fails, return `Store(_)` and keep the legacy file.
  - The next start retries, which is idempotent.
  - `known_peers.json` is deleted only after `insert_missing` returns `Ok`.
- **Safe direction.** If migration fails, those peers are *prompted*, never
  silently admitted.

### `dto.rs`

DTOs for the Tauri shell, kept next to the types in core. They hold strings
only.

```rust
#[derive(Debug, Serialize)]
pub struct ContactDto {
    pub node_id: String,
    pub nickname: String,
    /// Not rendered in the contact list (user decision). Carried for the
    /// naming prompt and a future contact-info screen.
    pub self_name: Option<String>,
    pub verified: bool,
    pub added_at_ms: i64,
    pub last_seen_ms: Option<i64>,
}
#[derive(Debug, Serialize)]
pub struct RemoveContactDto { pub node_id: String, pub history_erased: bool }
```

### `testing.rs` (behind `test-support`)

- **`MemoryContactStore`**: implements the same contract, including
  `insert_missing` atomicity, caps (configurable via `with_cap(n)`) and observe
  throttling. It also has a `writes()` counter.
- **`FailingContactStore`**: every call returns `Corrupt`.
- **`RecordingHistoryEraser`**: `calls() -> Vec<(AccountId, NodeId)>`.
  `failing()` records the call and then returns `Err`.
- **`BookCell`**: a shared slot with `BookCell::factory(store, history)`. It
  returns a closure `Fn(AccountId, NodeId) -> Arc<ContactBook>` that builds the
  book over the **same** store and records it. `BookCell::get() ->
  Arc<ContactBook>` returns the last book built. Tests use it with 1B's
  `TestOptions::with_known_factory` (wrapped in 1C).

### `mod.rs`

The slice docs cover:

- NodeId-only trust;
- names being advisory;
- the default-nickname snapshot rule;
- the interim storage deviation;
- the removal order.

Re-export the public types and declare
`#[cfg(any(test, feature = "test-support"))] pub mod testing;`.

### 1A tests

| Test | Asserts | DD-03 Testing item |
|------|---------|--------------------|
| `model::nickname_rejects_control_newline_bidi_and_overlength` | `""`, `"   "`, `"a\nb"`, `"a\tb"`, every bidi char, 129 code points → matching error | name sanitization |
| `model::nickname_is_trimmed_and_verbatim` | `"  Bob  "` → `"Bob"`; emoji/CJK ok | |
| `model::default_for_uses_self_name_else_peer_short_id` | `Some("Bob")` → `"Bob"`; `None` → `"Peer <fmt_short>"`; both pass `Nickname::parse` | default nickname (C) |
| `model::history_choice_defaults_to_keep` | `default() == Keep`, `from_flag(None) == Keep`, `from_flag(Some(false)) == Keep` | flag defaults off |
| `store::contract_file` / `testing::contract_memory` | one shared `contract_suite(&dyn ContactStore)`, run on both stores. Covers insert/get; insert existing → `AlreadyExists` with the original kept; `set_nickname` only changes the nickname; `set_trust` only changes trust; `observe` only changes `self_name`/`last_seen`; `observe` on an unknown id → Ok with nothing created; `remove` returns a bool; `insert_missing` skips existing entries and writes once; accounts isolated | add/known, namespacing |
| `store::observe_skips_write_when_nothing_material_changed` | same name within `LAST_SEEN_RESOLUTION_MS` → `writes()` unchanged; new name → one write; last_seen older than the resolution → one write | store cost |
| `store::insert_missing_is_all_or_nothing_at_the_cap` | a batch that would exceed 10 000 for one account → `Full`, no write, earlier data intact | |
| `store::survives_reopen`, `store::same_nickname_different_nodes_are_distinct` | | |
| `store::corrupt_files_error_and_are_not_overwritten` | not JSON; `version: 2`; extra fields (top and contact); bad / uppercase UUID; bad / non-canonical NodeId; nickname `"a\nb"` / U+202E / U+200B / 129 chars / `" a"`; `self_name` `"\u{0007}"` / `"x\u{FEFF}"`; `trust: "trusted"`; negative timestamp → every op gives `Corrupt`, file bytes unchanged | corrupt ≠ empty, sanitization at rest |
| `store::symlinked_file_is_refused` (unix), `store::oversized_file_is_refused`, `store::full_store_reports_full_and_stays_readable`, `store::file_mode_is_0600_and_no_temp_file_remains` (unix) | | DoS / fs hardening |
| `store::file_contains_only_known_keys_and_no_key_material` | keys match the schema exactly; a `FakeKeychain` identity key (hex and base64) is absent | secrets |
| `book::add_makes_is_known_true_only_for_that_node` | | **unit add/known** |
| `book::add_starts_unverified`, `book::add_rejects_own_node_id`, `book::add_invalid_nickname_persists_nothing` (incl. `"a\u{200B}b"` once 1D is merged — add that case in 1C), `book::add_existing_is_already_exists_and_keeps_record` | | |
| `book::remember_dialed_creates_unverified_with_default_nickname` | absent + `Some("Alice")` → nickname `"Alice"`, `self_name` `"Alice"`, Unverified; absent + `None` → `"Peer …"` | decision E/C |
| `book::remember_dialed_keeps_existing_nickname_and_trust` | an existing `"Mine"`/Verified contact stays `"Mine"`/Verified; only `self_name` is updated | |
| `book::rename_keeps_node_id_self_name_and_trust` | | **nickname independence (1)** |
| `book::observe_hello_keeps_nickname_and_trust` | Hello `"Robert"` on a `"Bobby"`/Verified contact → nickname `"Bobby"`, Verified, `self_name` `"Robert"` | **nickname independence (2)** |
| `book::default_nickname_is_a_snapshot` | `remember_dialed(B, "Bob")`, then `observe_hello(B, "Robert")` → nickname still `"Bob"`, `self_name` `"Robert"` | decision C |
| `book::observe_lan_updates_self_name_only` | `last_seen_ms` unchanged; unknown id → nothing created | LAN observation |
| `book::only_set_verified_changes_trust` | add → rename → observe_hello/lan (names `"Verified"`, `"✓ Bob"`) → `remember_dialed` → second add → still Unverified; `set_verified(true)` → Verified; further observes keep Verified; `set_verified(false)` → Unverified | **verify toggle** |
| `book::observe_unknown_never_creates_a_contact` | | peer can't self-add |
| `book::set_verified_and_rename_unknown_are_not_found` | | |
| `book::remove_reverts_trust` | | **removal reverts trust** |
| `book::remove_keep_never_calls_eraser` / `book::remove_erase_calls_eraser_once_then_removes` / `book::remove_keeps_contact_when_history_erase_fails` / `book::remove_unknown_is_not_found_and_erases_nothing` | | **removal + clear-history flag** |
| `book::books_for_different_accounts_are_isolated` | | **namespacing** |
| `book::store_failure_is_unavailable_never_unknown` | `FailingContactStore` → `is_known` is `Err(Unavailable)` | fail closed |
| `book::list_is_sorted_by_nickname_then_node_id` | | |
| `migrate::migrates_entries_as_unverified_contacts` | the legacy `"Bob"` entry → nickname **`"Bob"`**, `self_name` `"Bob"`, `added_at_ms` preserved, Unverified | migration |
| `migrate::nameless_entry_gets_peer_short_id_nickname` | | |
| `migrate::invisible_legacy_name_counts_as_no_name` | `"Bob\u{200D}"` → nickname `"Peer …"`, `self_name` None, file not treated as corrupt (needs 1D; add in 1C if 1A merges first) | D fallback |
| `migrate::migrates_every_account_separately` | | namespacing |
| `migrate::is_idempotent_and_never_overwrites_contacts` | pre-existing `"Mine"`/Verified untouched; `already_present` counted | |
| `migrate::single_atomic_write_and_legacy_deleted_only_after_success` | store `writes() == 1`; with `FailingContactStore` the legacy file remains | all-or-nothing |
| `migrate::corrupt_legacy_file_is_left_untouched_and_nothing_migrated`, `migrate::missing_legacy_file_is_noop` | | fail closed |
| `core/tests/contacts_log_guard.rs::contacts_and_discovery_never_log_names` | reads every `.rs` file under `core/src/contacts/` and `core/src/discovery/`. For each `trace!/debug!/info!/warn!/error!` invocation (with or without `tracing::`), the macro's argument text contains none of `nickname`, `self_name`, `display_name`, `name =`, `.as_str()` | **no contact data in diagnostics** |

---

## Sub-task 1B — discovery seam additions (parallel)

These changes are additive. Keep `KnownPeers::add` and the node's existing
`add` calls in this sub-task (1C replaces them). All existing DD-02 tests must
still pass.

### Changes

1. **`known_peers.rs`.** Add to the trait:
   ```rust
   /// Where an advisory name was seen.
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum NameSource { Hello, Lan }
   /// Advisory name of an already-known peer. Hello (authorized connection):
   /// `name` replaces the stored name (None clears) and marks it seen.
   /// Lan: called only with Some(name); never marks "seen". Must never create
   /// an entry or change trust; a no-op for unknown ids.
   fn observe(&self, id: &NodeId, name: Option<&DisplayName>, source: NameSource) -> Result<(), KnownPeersError>;
   ```
   Implement it for `FileKnownPeers`, where it updates an existing entry's name
   only. Implement it for the doubles too:
   - `MemoryKnownPeers` gains `trust(&self, id: NodeId)` (pre-seed) and
     `observed(&self, id) -> Vec<(Option<String>, NameSource)>`. Its `observe`
     never creates entries.
   - `FailingKnownPeers::observe` returns `Err`.
   Re-export `NameSource`.
2. **Blocking I/O off the async runtime.** Every `inner.known.*` call in
   `node.rs` goes through one helper:
   ```rust
   async fn known_call<T: Send + 'static>(inner: &Inner,
       f: impl FnOnce(&dyn KnownPeers) -> Result<T, KnownPeersError> + Send + 'static) -> Result<T, KnownPeersError>
   ```
   It clones the `Arc` and runs `tokio::task::spawn_blocking`. A `JoinError`
   becomes `Err`. The gate keeps `matches!(…, Ok(true))`, so a panic or failure
   means "unknown" and the peer is prompted (fail closed).
3. **`events.rs` / `dto.rs`.** `NetEvent::ConnectionState` gains
   `display_name: Option<DisplayName>`. It is `Some` only on `Connected` with a
   valid Hello name. Add `Inner::emit_connected(peer, dir, name)`;
   `emit_state` passes `None`. `ConnectionStateDto` gains
   `display_name: Option<String>`. This is a Tauri event payload change, not a
   wire change.
4. **`node.rs`.**
   - `Authorized` stores `display_name`.
   - `authorize(…, name)` emits `emit_connected`. It then calls
     `known.observe(peer, name, NameSource::Hello)` in the background (via
     `known_call`, spawned). On error it logs
     `debug!(peer = %peer.fmt_short(), "could not record peer name")`.
   - Pass the Hello name from all three sites: the known-silent inbound path,
     the accepted inbound path and `run_dial`.
   - `pub fn pending_request(&self, peer: &NodeId) -> Option<IncomingRequest>`.
   - `pub fn last_seen_name(&self, peer: &NodeId) -> Option<DisplayName>`:
     the authorized Hello name, else the pending prompt name, else the LAN
     record name. Memory only. Doc: advisory, for display/`self_name` only.
   - `pub fn disconnect(&self, peer: &NodeId) -> bool` closes the authorized
     connection with `CLOSE_NORMAL`. The existing `closed()` watcher removes it
     and emits `Disconnected`.
   - **LAN name observation in `lan_loop`.** Keep a local
     `HashMap<NodeId, DisplayName>` of the names already observed. After
     applying a `LanEvent::Discovered { addr, .. }`, read that peer's registry
     entry. If it has `Some(name)` that differs from the last observed name,
     spawn `known.observe(addr.id, Some(&name), NameSource::Lan)` and record
     it. On `Expired`, drop the peer from the map. This costs one observation
     per actual name change.
5. **`testing/mod.rs`, the known-store factory (fixes fixtures that need the
   account and own NodeId).**
   ```rust
   pub type KnownFactory = Arc<dyn Fn(AccountId, NodeId) -> Arc<dyn KnownPeers> + Send + Sync>;
   impl TestOptions { pub fn with_known_factory(mut self, f: KnownFactory) -> Self; }
   ```
   `start()` calls the factory **after provisioning**, using `account_id` and
   `node_id` from `identity.status()`, and stores the result in
   `TestNode.known`. When no factory is set, it uses `options.known` as today.
   On `restart` the factory runs again. Factories must therefore capture a
   shared store, which keeps persistence intact.
6. **Mechanical fixes outside `core/src/discovery`.** Only at the three sites in
   the ownership table:
   - `app/src-tauri/src/net_cmd.rs` `is_state`: this is a pattern, so add `..`.
   - `headless/src/node.rs` test and `headless/tests/control.rs`: these are
     struct constructions, so add `display_name: None`.

   Inside core, update every pattern or construction of
   `NetEvent::ConnectionState` (`dto.rs`, `discovery_two_nodes.rs`,
   `discovery_lan.rs`, `discovery_relay.rs`).

### 1B tests

| Test | Asserts |
|------|---------|
| `dto::connection_state_json_shape` (update) | key `display_name` is present: `null`, or `"Ada"` on a named `Connected`; the key set is exactly `direction, display_name, node_id, reason, state` |
| `connected_event_carries_hello_name_both_directions` | inbound Connected shows `"Bob"`, outbound shows `"Alice"` |
| `connected_event_name_is_none_for_invalid_hello_name` | `raw_dial` with a U+202E name → `None` |
| `known_peer_name_is_observed_on_every_authorized_connection` | `trust(B)`; B connects as `"Bob"`, renames to `"Robert"` and reconnects → `observed(B)` ends with `[(Some("Bob"),Hello),(Some("Robert"),Hello)]`; still one entry |
| `lan_names_are_observed_once_per_change` | LAN bus: B advertises `"Bob"`, then `"Robert"` → A's `MemoryKnownPeers` gets `[("Bob",Lan),("Robert",Lan)]`, no duplicates from re-announcements |
| `observe_is_never_called_for_declined_or_pending_peers` | |
| `failing_known_store_still_prompts` (existing `FailingKnownPeers` test) | still passes with `known_call` |
| `last_seen_name_prefers_authorized_then_pending_then_lan`, `pending_request_returns_open_prompt_only` | |
| `disconnect_closes_authorized_connection_and_emits_disconnected`, `disconnect_unknown_peer_is_false_and_harmless` | |
| `known_factory_receives_account_and_node_id_after_provisioning` | the factory gets the provisioned `AccountId`/`NodeId`; it runs again on `restart` |

---

## Sub-task 1C — integration: contacts become the trust store (sequential, after 1A + 1B + 1D)

### Core

1. **`known_peers.rs`: final shape.** Delete `FileKnownPeers`, its document,
   constants and tests. The trait becomes:
   ```rust
   pub trait KnownPeers: Send + Sync + 'static {
       fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError>;
       fn observe(&self, id: &NodeId, name: Option<&DisplayName>, source: NameSource) -> Result<(), KnownPeersError>;
       /// A peer we deliberately dialed replied with a valid Hello (DD-03,
       /// user decision E): remember it as an Unverified contact with the
       /// default nickname if it is not one yet.
       fn remember_dialed(&self, id: &NodeId, name: Option<&DisplayName>) -> Result<(), KnownPeersError>;
   }
   /// Opaque: the gate only needs "failed" (fail closed).
   #[derive(Debug)] pub struct KnownPeersError;
   ```
   `add` is gone. Module doc: "The trust seam the gate consults. The production
   implementation is `contacts::ContactBook`. The node never creates trust on
   **inbound** traffic; outbound dials are remembered via `remember_dialed`."
   Drop the `FileKnownPeers` re-export.
2. **`node.rs`.**
   - The accepted-inbound path: delete the `known.add` call. Persistence on
     Accept is `contacts::accept_incoming`.
   - `run_dial`: replace `known.add(peer, name)` with
     `known_call(remember_dialed(peer, name))`, still made only after a valid
     Hello reply. On error, log `debug!(peer = %peer.fmt_short(), "could not
     remember dialed peer")`. The connection is still authorized either way.
   - Document `Node::respond_incoming`: "Answers the prompt for this session
     only; `contacts::accept_incoming` persists trust."
3. **Doubles.**
   - `MemoryKnownPeers`: keep `trust()`, `observed()` and `len/is_empty`. Add
     `dialed() -> Vec<NodeId>`. `remember_dialed` records the call and trusts
     the id. Delete `name_of`.
   - `FailingKnownPeers`: implement the three methods, each returning `Err`.
     It has no `trust()`.
   Delete anything left without a caller (the real-content rule).
4. **`contacts/book.rs`: `impl KnownPeers for ContactBook`.**
   - `is_known` → `ContactBook::is_known`.
   - `observe` → `observe_hello` or `observe_lan`. A `Lan` call with `None` is
     ignored.
   - `remember_dialed` → `ContactBook::remember_dialed`.
   - Every `Err` maps to `KnownPeersError`.
   - Doc: "This is the function `contacts_is_known` exposes; the gate and the
     command share it."
5. **`core/src/contacts/accept.rs` (new).**
   ```rust
   pub trait IncomingPrompts {
       /// Some(name) while a prompt for `peer` is open (name = its Hello name).
       fn pending_name(&self, peer: &NodeId) -> Option<Option<DisplayName>>;
       fn answer(&self, peer: &NodeId, accept: bool) -> Result<(), RespondError>;
   }
   impl IncomingPrompts for Node { /* pending_request / respond_incoming */ }

   #[derive(Debug)]
   pub enum AcceptError { InvalidNickname(DisplayNameError), NoSuchRequest, Contacts(ContactError) }

   /// Accept == add contact (DD-03). Order:
   /// 1 validate nickname (prompt stays open on failure);
   /// 2 read the open prompt (NoSuchRequest if none), self_name = its Hello name;
   /// 3 book.add(peer, nickname, self_name) — AlreadyExists is fine (record kept,
   ///   not ours); other errors → returned, prompt stays open, peer NOT admitted;
   /// 4 prompts.answer(peer, true); on failure, if step 3 created the record,
   ///   remove it again (HistoryChoice::Keep) and return NoSuchRequest.
   /// Blocking (file I/O): async callers use spawn_blocking.
   pub fn accept_incoming(book: &ContactBook, prompts: &impl IncomingPrompts, peer: &NodeId, nickname: &str) -> Result<Contact, AcceptError>;
   ```
   Re-export `accept_incoming`, `AcceptError` and `IncomingPrompts`.
6. **`contacts/testing.rs`.** Add
   `BookCell::known_factory(&self) -> discovery::testing::KnownFactory`, which
   wraps `factory` and upcasts the book to `Arc<dyn KnownPeers>`.
7. **`InProcessDriver` (`discovery/testing/scenarios.rs`).**
   - Use a `FileContactStore` in the driver's temp dir, `NoMessageStore` and a
     `BookCell`, via `TestOptions::with_known_factory(cell.known_factory())`.
   - `respond(id, true)` → `accept_incoming(&cell.get(), &node, &peer,
     Nickname::default_for(&peer, pending_name).as_str())`.
   - `respond(id, false)` → `respond_incoming(false)`.
   - The scenarios keep their semantics.
8. **DD-02 test rewrites.** Rewrite these; do not delete them.
   - `discovery_two_nodes::ticket_round_trip_yields_live_channel`. Drop the
     `name_of` asserts. Assert instead: A's `MemoryKnownPeers` is still empty
     after a plain `respond_incoming(true)` (inbound grants nothing without
     contacts), and B's `dialed()` contains A.
   - `discovery_two_nodes::known_peer_reconnects_silently_across_restart`. Use a
     `BookCell` over `FileContactStore` and `accept_incoming`. Assert
     `contacts.json` exists.
   - `discovery_two_nodes::dialed_peer_is_remembered_only_after_a_valid_reply`.
     The semantics stay. Use a `BookCell` for B. After A declines, B's book is
     empty. Add the positive half: A accepts, then B's book has A as Unverified,
     nickname `"Alice"` (the default).
   - `discovery_lan::lan_peers_are_never_persisted`. Use a book over a
     `FileContactStore`, so the disk check covers `contacts.json`.
   - `discovery_lan::lan_pick_uses_the_same_trust_gate`. Give both nodes a
     `BookCell`. A accepts B via `accept_incoming(…, "Bobby")`. B's second LAN
     pick connects with no prompt on A. Also assert that B's book has A, which
     B auto-added by dialing.
9. **Accept prefill: `discovery/dto.rs`.** `IncomingRequestDto` gains
   `pub suggested_nickname: String`, computed as
   `Nickname::default_for(&req.node_id, req.display_name.as_ref()).as_str()`.
   This is the only source of the Accept prefill, so the frontend never derives
   it. Update `dto::incoming_request_json_shape`: with a name it is `"Ada"`,
   without one it is `"Peer <fmt_short>"`, and the key set is exactly
   `display_name, node_id, suggested_nickname`. The headless stdout and
   `pending_incoming` carry the same field.

### Integration tests — `core/tests/contacts_two_nodes.rs` (new, Level 1, loopback)

All tests use `BookCell` + `with_known_factory`. Restarts reuse the same
`FileContactStore` dir.

| Test | Asserts | DD-03 Testing item |
|------|---------|--------------------|
| `accept_with_nickname_lists_unverified_contact_and_reconnects_silently` | B dials A; A accepts with `"Bobby"` → A's `list()` == [B, `"Bobby"`, `self_name` `"Bob"`, Unverified]; restart A; B reconnects → **no** `incoming_request` on A within `QUIET`; both Connected | **Integration (two nodes)** |
| `dialed_peer_becomes_unverified_contact_with_default_nickname` | B dials A, A accepts → B's book: A, nickname `"Alice"`, Unverified; restart B; A dials B → **no** prompt on B | decision E |
| `peer_rename_updates_self_name_not_nickname` | B renames to `"Robert"` and reconnects → A: `self_name` `"Robert"`, nickname `"Bobby"`; B's auto-added nickname for A stays `"Alice"` after A renames to `"Alicia"` | nickname independence + C snapshot |
| `lan_name_change_updates_self_name_of_existing_contact_only` | in-process LAN bus: the contact renames → `self_name` updates; a non-contact on the LAN creates nothing | LAN observation |
| `removed_contact_is_prompted_again` | `remove(B, Keep)`, then B reconnects → exactly one `incoming_request` | **removal reverts trust** |
| `decline_creates_no_contact` | | |
| `accept_with_invalid_nickname_keeps_prompt_open` | `"\u{202E}x"` → BidiOverride and `"a\u{200B}b"` → InvisibleCharacter; prompt still pending; book empty; B not connected | sanitization |
| `hello_name_with_invisible_char_is_dropped` | B's name set via a raw Hello `"Bob\u{200B}"` → pending `display_name` None → contact `self_name` None | D |
| `accept_rolls_back_new_contact_when_prompt_vanished` (unit, `accept.rs`) | fake `IncomingPrompts` whose `answer` fails → `NoSuchRequest`, `is_known` false; a pre-existing contact is kept | TOCTOU |
| `accept_store_failure_does_not_admit_peer` (unit) | `FailingContactStore` → `Contacts(Unavailable)`; `answer` never called | fail closed |
| `corrupt_contacts_file_fails_closed_and_prompts` | garbage in `contacts.json` → a previously accepted B is prompted; bytes unchanged | corrupt ≠ empty |
| `legacy_known_peers_reconnect_silently_after_migration` | a v1 `known_peers.json` with B as `"Bob"`; migrate; start A → B connects with no prompt; nickname `"Bob"`, Unverified; legacy file gone | migration |
| `contacts_are_namespaced_by_account_end_to_end` | two books (different accounts) over one store: B, added in book 1, is prompted by a node gated by book 2 | **namespacing** |

Parity (`core/tests/discovery_parity.rs`, `headless/tests/parity.rs`) must
pass unchanged.

### Headless (`headless/`)

- `src/node.rs`:
  - Build one `Arc<FileContactStore>` over `args.data_dir`.
  - Call `migrate_legacy_known_peers` before starting. Log counts only.
  - Build `ContactBook::new(account_id, node_id, store, Arc::new(NoMessageStore))`
    from `IdentityStatus::Present` and use it as `NodeDeps.known`.
  - Keep the `Arc<ContactBook>` for the respond handler.
- `src/control.rs`: `Request::Respond { node_id, accept, #[serde(default)] nickname: Option<String> }`.
- `src/cli.rs`: the syntax is `respond <NODE_ID> accept [NICKNAME]` and
  `respond <NODE_ID> decline`. An accept with no nickname uses
  `Nickname::default_for(&peer, pending_name)`. Update the usage text and parse
  tests, and reject extra arguments after `decline`.
- Respond runs `accept_incoming` (inside `spawn_blocking`). Error replies are
  `"InvalidNickname"`, `"NoSuchRequest"` and `"ContactsUnavailable"`.
- `headless/tests/control.rs`: add `respond_accept_with_and_without_nickname`.
- Check by reasoning that `docker/smoke.sh` stage 4 ("known reconnect") still
  holds. Run it if Docker is available.

### App shell (`app/src-tauri/`)

1. **`src/test_util.rs`** (owned by 1C; this fixes 03-02's access to the
   helpers).
   - Move the two-node helpers out of `net_cmd.rs`'s private `mod tests` into
     `test_util.rs` as `pub(crate)` items: `Started`, `started_with`,
     `started`, `events`, `expect`, `is_state`, `is_incoming`, `ticket_of` and
     `WAIT`.
   - `Started` gains `contacts: Arc<ContactsState>` over `MemoryContactStore` +
     `RecordingHistoryEraser` (keep `history: Arc<RecordingHistoryEraser>` as a
     field too).
   - `started_with` builds the node with
     `TestOptions::with_known_factory(|a, n| contacts.bind(a, n))`.
   - `net_cmd.rs` tests import them from `crate::test_util`.
2. **`src/contacts_cmd.rs`** (new; 03-02 adds the commands):
   ```rust
   /// Process-wide contacts state: exactly one store instance, bound to the
   /// account once the identity exists.
   pub struct ContactsState { store: Arc<dyn ContactStore>, history: Arc<dyn HistoryEraser>, book: OnceLock<Arc<ContactBook>> }
   impl ContactsState {
       pub fn new(store: Arc<dyn ContactStore>, history: Arc<dyn HistoryEraser>) -> Self;
       /// Idempotent: first call wins.
       pub fn bind(&self, account: AccountId, own: NodeId) -> Arc<ContactBook>;
       pub fn book(&self) -> Option<Arc<ContactBook>>;
   }
   ```
3. **`src/lib.rs` setup.** After `create_dir_all` and the `0o700` chmod:
   ```rust
   let contacts_store = Arc::new(FileContactStore::new(data_dir.clone()));
   // The app crate has no tracing dependency; counts only, never names.
   match migrate_legacy_known_peers(&data_dir, contacts_store.as_ref()) {
       Ok(r) if r.migrated > 0 => eprintln!("migrated {} known peers to contacts", r.migrated),
       Ok(_) => {}
       Err(_) => eprintln!("legacy known peers not migrated; those peers will be prompted again"),
   }
   let contacts = Arc::new(ContactsState::new(contacts_store, Arc::new(NoMessageStore)));
   app.manage(contacts);   // managed as Arc<ContactsState>
   ```
   Register `mod contacts_cmd;`.
4. **`src/net_cmd.rs`.**
   - **`start_if_present`.** The `known` parameter becomes
     `contacts: &ContactsState`. **Bind right after reading
     `IdentityStatus::Present { account_id, node_id }`, before and outside the
     start future.** That way a relay error or a bind failure still leaves
     contacts usable. The node gets `known = contacts.bind(account_id,
     node_id)`.
   - `respond_incoming(state, contacts, node_id, accept, nickname: Option<String>)` is `async`:
     - a malformed id → `NoSuchRequest`;
     - decline → `node.respond_incoming(&peer, false)`;
     - accept with `nickname` `None` → `InvalidNickname { kind: Empty }`;
     - accept before the book is bound → `ContactsUnavailable`;
     - otherwise run `accept_incoming` inside
       `tauri::async_runtime::spawn_blocking`. A `JoinError` maps to
       `Internal`.
   - **`NetCmdError`** gains `InvalidNickname { kind: DisplayNameKind }`,
     `ContactsFull` and `ContactsUnavailable`.
   - **`From<AcceptError>`** is exhaustive:
     - `InvalidNickname(e)` and `Contacts(InvalidNickname(e))` → `InvalidNickname { kind: e.into() }`;
     - `NoSuchRequest` and `Contacts(NotFound)` → `NoSuchRequest`;
     - `Contacts(Full)` → `ContactsFull`;
     - `Contacts(SelfContact | AlreadyExists | Unavailable | HistoryEraseFailed)`
       → `ContactsUnavailable`. `AlreadyExists` is already absorbed by
       `accept_incoming`; the arm is there for exhaustiveness.
   - The command:
     ```rust
     #[tauri::command]
     pub async fn net_respond_incoming(state: State<'_, NetState>, contacts: State<'_, Arc<ContactsState>>,
         node_id: String, accept: bool, nickname: Option<String>) -> Result<(), NetCmdError>
     ```
5. **Tests in `net_cmd.rs`.**
   - `errors_serialize_with_type_tag`: add the new variants.
   - `ticket_connect_accept_flow_end_to_end`: respond with `Some("Bobby")`. A's
     book has B as Unverified `"Bobby"` with `self_name` `"Bob"`. **B's book has
     A as Unverified with the default nickname `"Alice"`.**
   - `dial_adds_known_only_after_hello_reply` → rename to
     `dial_remembers_peer_only_after_valid_reply`. After a decline the dialer's
     book is empty; after an accept it holds A with nickname `"Alice"`.
   - `accept_requires_a_valid_nickname`: `None`, `""`, `"a\nb"`, `"\u{202E}x"`,
     `"a\u{200B}b"` and 129 characters each give `InvalidNickname` with the
     matching kind. The prompt is still pending.
   - `malformed_node_id_is_no_such_request`: pass a nickname. Also check that
     decline ignores the nickname.
   - `accept_maps_every_contact_error`: a unit test over the `From<AcceptError>`
     table.
   - `contacts_bound_even_if_node_start_fails`: a failing start future still
     leaves `contacts.book().is_some()`.
   - `contacts_state_bind_is_idempotent`.
   - `dtos_never_contain_key_material` is unchanged.

### Frontend (`app/src/`): the minimal change that keeps the app usable

- **`net.ts`**
  - `netRespondIncoming(nodeId, accept, nickname?)` →
    `invoke("net_respond_incoming", { nodeId, accept, nickname: nickname ?? null })`.
  - `ConnectionStateDto.display_name: string | null`.
  - `NetCmdError` gains `InvalidNickname { kind }`, `ContactsFull` and
    `ContactsUnavailable`. `describeNetError` maps them using
    `nicknameMessages[kind]`, "Your contact list is full." and "Contacts are
    unavailable right now. The request is still open."
- **`lib/nicknameMessages.ts`** (new): a `Record<DisplayNameKind, string>`
  with one message per kind, including `InvisibleCharacter`.
- **`net.ts`, continued:** `IncomingRequestDto` gains `suggested_nickname: string`.
- **`views/NicknameForm.svelte`** (new; reused in 03-02). Props:
  `{ initial?: string; selfName: string | null; submitLabel: string; busy?: boolean; error?: string | null; onSubmit: (nickname: string) => void; onCancel?: () => void; cancelLabel?: string }`.
  - A context line: `They call themselves “<bdi>{selfName}</bdi>” (chosen by
    them, not verified)` or "They did not send a name".
  - `<label>Your name for them <input maxlength="256"></label>`. It is
    prefilled with `initial` when given, otherwise empty, and always editable.
  - Submit is disabled while the trimmed value is empty or `busy`.
- **`views/IncomingRequestPrompt.svelte`**
  - Embed `NicknameForm` with `initial={first.suggested_nickname}`,
    `selfName={first.display_name}` and `submitLabel="Accept and add
    contact"`, plus a separate **Decline** button. The prefill is only a
    suggestion: nothing is sent until the user clicks Accept.
  - **Cleared field: Accept is blocked.** If the user empties the field, Accept
    is disabled. The default is *not* silently re-applied, because the user
    deliberately removed it. The backend independently rejects an empty
    nickname with `InvalidNickname { kind: Empty }`.
  - Accept → `netRespondIncoming(id, true, nickname)`. On `InvalidNickname`,
    `ContactsFull` or `ContactsUnavailable`, show the error and keep the
    prompt. On success or `NoSuchRequest`, call `dropIncoming`.
  - Decline → `netRespondIncoming(id, false)`.
  - Reset the form when the head of the queue changes.
- **`views/Network.svelte`**: no change. Its copy ("Once they reply, this
  device is remembered and can reconnect without asking you again" / "Connect
  and remember this device") **stays true**, because the dial auto-adds a
  contact. Verify this as part of DoD.
- **Tests**
  - `IncomingRequestPrompt.test.ts`: update the existing cases and add:
    - `the nickname field is prefilled with suggested_nickname` (`"Eve"`, or
      `"Peer abc…"` when there is no name);
    - `the prefill is editable and the edited value is sent` (clear, type
      `"Bobby"`, click Accept → `("n1", true, "Bobby")`);
    - `accepting the untouched prefill sends it` (`("n1", true, "Eve")`);
    - `clearing the field disables Accept` (whitespace-only too; no
      `netRespondIncoming` call);
    - `nothing is sent until Accept is clicked` (render only → no call);
    - `the self-asserted name is shown as context`;
    - `InvalidNickname keeps the prompt and shows the message`;
    - `Decline sends no nickname`;
    - `hostile self-asserted names render inert`.
  - `NicknameForm.test.ts`: trims and submits; cancel works; shows "(chosen by
    them, not verified)"; `initial pre-fills only when given` and stays
    editable.
  - Fixture-only updates for the new `display_name` field in `netStore.test.ts`
    and `MainScreen.test.ts`.

### README (in 1C)

- Workspace table: `core` lists the `identity`, `discovery` and `contacts`
  slices.
- Replace the "Trust rule (interim…)" paragraph with:
  - **Accept** asks for your name for them and adds an **Unverified** contact.
  - A peer you dial is remembered once it replies with a valid Hello, as an
    Unverified contact named after what it calls itself, or "Peer <id>".
  - Later name changes by the peer never change your nickname.
  - Contacts are kept in `contacts.json` **in plaintext (nicknames included)
    until DD-05's encrypted DB**.
  - An existing `known_peers.json` is migrated once at startup and deleted.
- A note: names (yours and peers') may no longer contain invisible characters
  such as zero-width spaces. Joiners are allowed only inside emoji sequences
  (👨‍👩‍👧, 🏳️‍🌈, subdivision flags such as 🏴󠁧󠁢󠁥󠁮󠁧󠁿). A saved name that now
  fails is treated as unset, and My Info asks for a new one.
- Accept prefills your name for them with what they call themselves, or
  "Peer <id>". Edit it before clicking Accept.
- Manual checklist:
  - steps 6/7: the Accept prompt shows a prefilled, editable nickname;
  - add: "after upgrading a profile with `known_peers.json`, a previously
    accepted peer still connects without a prompt".
- Level-3 section: `ctl … respond <ID> accept [NICKNAME]`.

---

## Guardrails (mandatory)

- **Never write credentials, keys, tokens or secrets in code, tests, fixtures or
  config. All credentials must be provided at runtime (OS keychain /
  environment).**
- **Key types.** No `SecretKeyBytes` or `DbKey` under `app/`, not even in
  comments or names. The pre-commit hook greps for them.
- **Test backends.**
  - Never name `FileKeychain`, `messenger-headless` or `messenger_headless`
    under `app/`.
  - Contact and known-peer test doubles may be used only from `#[cfg(test)]`
    code. They sit behind `core`'s `test-support` feature, which
    `app/src-tauri/Cargo.toml` enables only in `[dev-dependencies]`
    (`tests/no_test_backends.rs` checks this). A non-test use therefore fails
    `cargo build`.
- **Unsafe.** `core` keeps `#![forbid(unsafe_code)]`.
- **Logs.** NodeIds (`fmt_short()`) and error classes are fine. **Never log
  nicknames, self-asserted names, message content, keys or secrets.** Never
  format `Contact`, `Nickname`, `DisplayName` or DTOs with `{:?}` in a log.
- **Pre-commit hook.** Never bypass it with `--no-verify`.
- **Real content only.** No empty modules, placeholder files or empty folders.
  `NoMessageStore` is a documented, correct implementation. Delete dead test
  helpers (`name_of`, unused error variants) in the same change.
- **Wire format.** No wire-format change: `protocol` is untouched, so
  `PROTOCOL_VERSION` stays.
- **Clipboard.** No `writeText` and no word `clipboard` in non-test files under
  `app/src/` except `views/MyInfo.svelte`.

## Security — threats, prevention, verification

| # | Threat | Prevention / fix | Check |
|---|--------|------------------|-------|
| S1 | Name-based authorization | Trust reads only the NodeId key; no API looks up by name; dedup by canonical NodeId | tests `book::add_makes_is_known_true_only_for_that_node`, `store::same_nickname_different_nodes_are_distinct`; grep **C1** |
| S2 | Peer data flips trust or self-adds via inbound traffic | Inbound never creates contacts (Accept goes through `accept_incoming` with a user-typed nickname); `observe` cannot create or touch trust/nickname; `set_trust` is the only trust writer and nothing in `discovery` reaches it | `book::only_set_verified_changes_trust`, `book::observe_unknown_never_creates_a_contact`, `lan_name_change_updates_self_name_of_existing_contact_only`; grep **C2** |
| S3 | Dial path over-trusts | `remember_dialed` runs only after a valid Hello reply to a dial the user started, and always creates **Unverified**; it never changes trust or an existing nickname | `dialed_peer_is_remembered_only_after_a_valid_reply`, `book::remember_dialed_keeps_existing_nickname_and_trust`; grep **C3** |
| S4 | Bidi/control/zero-width/over-length names stored or rendered | The DD-01 validator (now with the Cf set, and an exception only for structurally valid emoji ZWJ and tag sequences) applies to nicknames and Hello/LAN names; the file store re-validates on read | `display_name::rejected_inputs`, `display_name::zwj_outside_emoji_is_rejected`, `display_name::malformed_tag_sequences_are_rejected`, `model::nickname_rejects_…`, `store::corrupt_files_…`, `hello_name_with_invisible_char_is_dropped`, `accept_with_invalid_nickname_keeps_prompt_open` |
| S5 | Validator tightening bricks existing installs | `get_display_name` treats `InvisibleCharacter` as unset; the legacy migration treats it as no name; the rest of each document stays readable | `settings::stored_name_now_invisible_reads_as_unset`, `migrate::invisible_legacy_name_counts_as_no_name` |
| S6 | Peer name adopted as nickname without the user noticing (Accept prefill, decision E) | The prefill is computed by the backend (`suggested_nickname`) from the *validated* name, and is visibly editable. The self-asserted name is shown separately, labelled "chosen by them, not verified". The Unverified badge and the full NodeId stay on the prompt. An explicit Accept click is required, and an empty field blocks Accept. The contact is always Unverified | Vitest `the nickname field is prefilled…`, `the prefill is editable…`, `clearing the field disables Accept`, `nothing is sent until Accept is clicked`; `dto::incoming_request_json_shape` |
| S7 | XSS via names | Text interpolation only; no `{@html}` outside `MyInfo.svelte`; `<bdi>` | Vitest `hostile self-asserted names render inert`; grep **C4** |
| S8 | Corrupt store treated as empty or as known | Corrupt → `Err` → the gate treats it as unknown → prompt; migration aborts and leaves files untouched | `store::corrupt_files_…`, `book::store_failure_is_unavailable_never_unknown`, `corrupt_contacts_file_fails_closed_and_prompts`, `migrate::corrupt_legacy_file_…` |
| S9 | DoS of the store (size, count, write amplification from observations) | Byte and count caps; `Full`; symlink refused; observe throttled (`LAST_SEEN_RESOLUTION_MS`, LAN only on name change); file I/O in `spawn_blocking` | `store::oversized_file_is_refused`, `store::full_store_…`, `store::observe_skips_write_…`, `lan_names_are_observed_once_per_change` |
| S10 | Path traversal via account_id / profile | Fixed file name under the validated profile `data_dir`; account keys validated as canonical UUIDs | `store::corrupt_files_…` (UUID cases); grep **C5** |
| S11 | Accept racing prompt expiry | Validate → read prompt → add → answer → roll back a self-created record | `accept_rolls_back_new_contact_when_prompt_vanished`, `accept_store_failure_does_not_admit_peer` |
| S12 | Lost updates from two store instances | Exactly one `FileContactStore` per process, shared via `Arc` | grep **C6** |
| S13 | Removal doesn't revert trust, or history deletion defaults on | `remove` deletes the row; `HistoryChoice::default() == Keep` | `book::remove_reverts_trust`, `book::remove_keep_never_calls_eraser`, `model::history_choice_defaults_to_keep`, `removed_contact_is_prompted_again` |
| S14 | Names in logs | No name fields in any log; startup prints counts only | `core/tests/contacts_log_guard.rs`; grep **C7** |
| S15 | Forged IPC args | `NodeId::from_str` → `NoSuchRequest`; a missing nickname → `InvalidNickname{Empty}`; exhaustive error mapping | `malformed_node_id_is_no_such_request`, `accept_requires_a_valid_nickname`, `accept_maps_every_contact_error` |
| S16 | Migration silently trusts or drops peers | Only previously accepted or dialed peers, as Unverified; one atomic write; the legacy file is kept on failure (those peers are prompted) | `migrate::*`, `legacy_known_peers_reconnect_silently_after_migration` |
| S17 | Blocking file I/O stalls the runtime (gate DoS) | `known_call` / `spawn_blocking` everywhere the node or a command touches the store | `failing_known_store_still_prompts`; grep **C8** |

### Security checks (run from the repo root; review every hit)

```sh
# C1 no name comparisons used as identity (hits must be test code or display-only; review each)
grep -rnE 'nickname *==|self_name *==|display_name *==|as_str\(\) *==' core/src app/src-tauri/src headless/src

# C2 nothing in discovery can change trust
grep -rnE 'set_trust|set_verified' core/src/discovery              # expect no output

# C3 the node's only trust-store calls are is_known / observe / remember_dialed
grep -nE 'known\.(add|insert)\(' core/src/discovery/node.rs         # expect no output
grep -nE '\.(is_known|observe|remember_dialed)\(' core/src/discovery/node.rs   # expect: gate, authorize, lan_loop, run_dial only

# C4 {@html} only in MyInfo
grep -rn '{@html' app/src --include=*.svelte                       # expect only app/src/views/MyInfo.svelte

# C5 the store path is a constant file name
grep -n 'join(' core/src/contacts/store.rs                          # expect only CONTACTS_FILE_NAME / tmp name

# C6 exactly one production store per binary
grep -rn 'FileContactStore::new' app/src-tauri/src headless/src     # expect one hit in app lib.rs, one in headless node.rs (plus tests)

# C7 no names in log/print macros
grep -rnE '(trace|debug|info|warn|error|eprintln|println)!\(.*(nickname|self_name|display_name)' core/src app/src-tauri/src headless/src   # expect no output

# C8 no direct blocking store call on the async path in node.rs (all go through known_call)
grep -nE 'inner\.known\.' core/src/discovery/node.rs                # expect hits only inside fn known_call
```

## Tests (summary, mapped to the DD-03 Testing list)

| DD-03 item | Tests |
|------------|-------|
| add/known | `book::add_makes_is_known_true_only_for_that_node` |
| nickname independence | `book::rename_keeps_…`, `book::observe_hello_keeps_…`, `book::default_nickname_is_a_snapshot`, `peer_rename_updates_self_name_not_nickname` |
| verify toggle | `book::only_set_verified_changes_trust` |
| sanitization | `display_name::rejected_inputs`, `model::nickname_rejects_…`, `store::corrupt_files_…`, `hello_name_with_invisible_char_is_dropped` |
| removal reverts trust | `book::remove_reverts_trust`, `removed_contact_is_prompted_again` |
| removal + clear-history | `book::remove_*` |
| integration | `accept_with_nickname_lists_unverified_contact_and_reconnects_silently`, `dialed_peer_becomes_unverified_contact_with_default_nickname` |
| namespacing | `book::books_for_different_accounts_are_isolated`, `contacts_are_namespaced_by_account_end_to_end` |

All keychain-dependent tests use `FakeKeychain` / `MemorySettingsStore`, never
the OS keychain.

## Quality gates

Every sub-task commit and the integration commit must pass all of these:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build
cd app && npm ci && npm test && npm run check && npm run build
```

Commit through `.githooks/pre-commit` (`git config core.hooksPath .githooks`),
never with `--no-verify`. Run `docker/smoke.sh` if Docker is available and note
the result.

## Definition of done

- [ ] 1D: the shared validator rejects the specified Cf set
      (`InvisibleCharacter`), except structurally valid emoji ZWJ and tag
      sequences. `Extended_Pictographic` is classified by the pinned Unicode
      16.0 range table, with no new dependency and no `unsafe`. A stored name
      that now fails reads as unset and never bricks startup. The new kind is
      surfaced in the app and frontend.
- [ ] 1A: `core::contacts` is complete: model, `default_for`, store with caps of
      10 000 per account and 32 MiB, throttled observe, atomic `insert_missing`,
      book, history seam, migration, DTOs and doubles. All 1A tests pass.
- [ ] 1B: `observe` + `NameSource` (Hello and LAN), `known_call` /
      `spawn_blocking`, `display_name` on Connected, `pending_request`,
      `last_seen_name`, `disconnect` and `with_known_factory` are in. The DD-02
      suite is green.
- [ ] 1C: `FileKnownPeers` and `add` are gone, and `KnownPeersError` is opaque.
      The gate consults `ContactBook`. Inbound never creates trust; Accept
      persists via `accept_incoming`. Dials are remembered via
      `remember_dialed` with the default nickname.
- [ ] Migration runs at app and headless startup in one atomic write.
      Previously accepted peers connect silently. The legacy file is deleted
      only on success.
- [ ] Contacts are bound before and independently of node start.
- [ ] The Accept prompt prefills `suggested_nickname`, which stays editable.
      An empty field blocks Accept, and there is no auto-accept. The Network
      copy "remembered" is verified true.
- [ ] `net_cmd` test helpers live in `test_util.rs`.
- [ ] Headless `respond … accept [NICKNAME]` works, and parity passes at Level 1
      and Level 3.
- [ ] Security checks C1–C8 come back clean.
- [ ] README is updated: structure, trust rule, plaintext storage, migration,
      the name rule change, the checklist and the headless syntax.
- [ ] All quality gates pass through the hook without `--no-verify`.
- [ ] This step folder is committed in this PR.
- [ ] PR per `.github/PULL_REQUEST_TEMPLATE.md`. Crates: `core` (contacts,
      discovery, identity), `app`, `headless`, docs. **Wire format: no change;
      `PROTOCOL_VERSION` not bumped.** Note that the Tauri `connection_state`
      payload gains `display_name`, `net_respond_incoming` gains `nickname`, and
      DD-01 names are tightened. List the commands and checklist items run.
