//! Contact persistence: the [`ContactStore`] trait and the interim file store.
//!
//! Interim plaintext store, a documented deviation from DD-03/DD-05. Nicknames
//! and peers' self-asserted names are stored unencrypted in `contacts.json`
//! (mode 0600 in the 0700 app-data dir) until DD-05's encrypted `contacts`
//! table replaces this type behind the same `ContactStore` trait.
//!
//! A process must construct exactly one [`FileContactStore`] per path: two
//! instances serialize on separate locks and can lose updates. Share it via
//! `Arc`.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::model::{Contact, NameUpdate, Nickname, TrustState};
use crate::identity::{validate_display_name, AccountId, NodeId};

pub const MAX_CONTACTS_PER_ACCOUNT: usize = 10_000;
/// 10k contacts with two names of at most 512 bytes each, plus headroom.
pub const MAX_CONTACTS_BYTES: u64 = 32 * 1024 * 1024;
/// `observe` does not rewrite the file just to move last_seen by less than this.
pub const LAST_SEEN_RESOLUTION_MS: i64 = 5 * 60 * 1000;

const CONTACTS_FILE_NAME: &str = "contacts.json";
const CONTACTS_VERSION: u32 = 1;

pub trait ContactStore: Send + Sync + 'static {
    fn list(&self, account: &AccountId) -> Result<Vec<Contact>, ContactStoreError>;
    fn get(&self, account: &AccountId, id: &NodeId) -> Result<Option<Contact>, ContactStoreError>;
    /// Never overwrites: existing NodeId -> `AlreadyExists`.
    fn insert(&self, account: &AccountId, contact: Contact) -> Result<(), ContactStoreError>;
    /// Inserts every entry whose (account, NodeId) is absent, skips existing
    /// ones untouched, in ONE atomic write. Any cap violation -> `Full` and
    /// nothing written. Used by migration.
    fn insert_missing(
        &self,
        entries: Vec<(AccountId, Contact)>,
    ) -> Result<BatchReport, ContactStoreError>;
    fn set_nickname(
        &self,
        account: &AccountId,
        id: &NodeId,
        nickname: &Nickname,
    ) -> Result<Contact, ContactStoreError>;
    /// The ONLY write path for `trust`.
    fn set_trust(
        &self,
        account: &AccountId,
        id: &NodeId,
        trust: TrustState,
    ) -> Result<Contact, ContactStoreError>;
    /// Advisory update of an EXISTING contact; Ok no-op for unknown ids.
    /// Never touches nickname or trust. Skips the write when `name` leaves
    /// self_name unchanged AND (`seen_at_ms` is None OR the stored
    /// last_seen_ms is within [`LAST_SEEN_RESOLUTION_MS`] of it).
    fn observe(
        &self,
        account: &AccountId,
        id: &NodeId,
        name: NameUpdate<'_>,
        seen_at_ms: Option<i64>,
    ) -> Result<(), ContactStoreError>;
    fn remove(&self, account: &AccountId, id: &NodeId) -> Result<bool, ContactStoreError>;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub inserted: usize,
    pub already_present: usize,
}

#[derive(Debug)]
pub enum ContactStoreError {
    NotFound,
    AlreadyExists,
    Full,
    Corrupt,
    Io(io::Error),
}

impl fmt::Display for ContactStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("contact not found"),
            Self::AlreadyExists => f.write_str("contact already exists"),
            Self::Full => f.write_str("contact store is full"),
            Self::Corrupt => f.write_str("contact store is corrupt"),
            Self::Io(e) => write!(f, "contact store I/O error: {e}"),
        }
    }
}

impl std::error::Error for ContactStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// In-memory contact set with the store contract's rules, shared by the file
/// store and the memory double so both obey one implementation.
#[derive(Debug, Default, Clone)]
pub(crate) struct Ledger(BTreeMap<Uuid, BTreeMap<NodeId, Contact>>);

impl Ledger {
    pub(crate) fn list(&self, account: &AccountId) -> Vec<Contact> {
        self.0
            .get(&account.as_uuid())
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn get(&self, account: &AccountId, id: &NodeId) -> Option<Contact> {
        self.0.get(&account.as_uuid())?.get(id).cloned()
    }

    pub(crate) fn insert(
        &mut self,
        account: &AccountId,
        contact: Contact,
        cap: usize,
    ) -> Result<(), ContactStoreError> {
        let map = self.0.entry(account.as_uuid()).or_default();
        let result = if map.contains_key(&contact.node_id) {
            Err(ContactStoreError::AlreadyExists)
        } else if map.len() >= cap {
            Err(ContactStoreError::Full)
        } else {
            map.insert(contact.node_id, contact);
            Ok(())
        };
        self.prune(account);
        result
    }

    pub(crate) fn insert_missing(
        &mut self,
        entries: Vec<(AccountId, Contact)>,
        cap: usize,
    ) -> Result<BatchReport, ContactStoreError> {
        let mut report = BatchReport::default();
        let mut fresh: BTreeMap<(Uuid, NodeId), Contact> = BTreeMap::new();
        let mut added: BTreeMap<Uuid, usize> = BTreeMap::new();
        for (account, contact) in entries {
            let key = (account.as_uuid(), contact.node_id);
            let present = self.0.get(&key.0).is_some_and(|m| m.contains_key(&key.1));
            if present || fresh.contains_key(&key) {
                report.already_present += 1;
                continue;
            }
            *added.entry(key.0).or_default() += 1;
            fresh.insert(key, contact);
        }
        for (uuid, count) in &added {
            let existing = self.0.get(uuid).map_or(0, BTreeMap::len);
            if existing + count > cap {
                return Err(ContactStoreError::Full);
            }
        }
        report.inserted = fresh.len();
        for ((uuid, id), contact) in fresh {
            self.0.entry(uuid).or_default().insert(id, contact);
        }
        Ok(report)
    }

    fn entry_mut(
        &mut self,
        account: &AccountId,
        id: &NodeId,
    ) -> Result<&mut Contact, ContactStoreError> {
        self.0
            .get_mut(&account.as_uuid())
            .and_then(|m| m.get_mut(id))
            .ok_or(ContactStoreError::NotFound)
    }

    pub(crate) fn set_nickname(
        &mut self,
        account: &AccountId,
        id: &NodeId,
        nickname: &Nickname,
    ) -> Result<Contact, ContactStoreError> {
        let c = self.entry_mut(account, id)?;
        c.nickname = nickname.clone();
        Ok(c.clone())
    }

    pub(crate) fn set_trust(
        &mut self,
        account: &AccountId,
        id: &NodeId,
        trust: TrustState,
    ) -> Result<Contact, ContactStoreError> {
        let c = self.entry_mut(account, id)?;
        c.trust = trust;
        Ok(c.clone())
    }

    /// Returns whether anything material changed (i.e. a write is due).
    pub(crate) fn observe(
        &mut self,
        account: &AccountId,
        id: &NodeId,
        name: NameUpdate<'_>,
        seen_at_ms: Option<i64>,
    ) -> bool {
        let Ok(c) = self.entry_mut(account, id) else {
            return false;
        };
        let mut changed = false;
        if let NameUpdate::Set(new) = name {
            if c.self_name.as_ref() != new {
                c.self_name = new.cloned();
                changed = true;
            }
        }
        if let Some(seen) = seen_at_ms {
            let stale = c
                .last_seen_ms
                .is_none_or(|prev| seen.saturating_sub(prev).abs() >= LAST_SEEN_RESOLUTION_MS);
            if stale || changed {
                c.last_seen_ms = Some(seen);
            }
            changed |= stale;
        }
        changed
    }

    pub(crate) fn remove(&mut self, account: &AccountId, id: &NodeId) -> bool {
        let removed = self
            .0
            .get_mut(&account.as_uuid())
            .is_some_and(|m| m.remove(id).is_some());
        self.prune(account);
        removed
    }

    fn prune(&mut self, account: &AccountId) {
        if self
            .0
            .get(&account.as_uuid())
            .is_some_and(BTreeMap::is_empty)
        {
            self.0.remove(&account.as_uuid());
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredContact {
    nickname: String,
    self_name: Option<String>,
    trust: TrustState,
    added_at_ms: i64,
    last_seen_ms: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    accounts: BTreeMap<String, BTreeMap<String, StoredContact>>,
}

impl Document {
    /// Validates every field and converts; any failure is `Corrupt`.
    fn into_ledger(self) -> Result<Ledger, ContactStoreError> {
        let corrupt = || ContactStoreError::Corrupt;
        if self.version != CONTACTS_VERSION {
            return Err(corrupt());
        }
        let mut ledger = Ledger::default();
        for (account, contacts) in self.accounts {
            let uuid = Uuid::parse_str(&account).map_err(|_| corrupt())?;
            if uuid.hyphenated().to_string() != account || contacts.len() > MAX_CONTACTS_PER_ACCOUNT
            {
                return Err(corrupt());
            }
            let mut map = BTreeMap::new();
            for (key, stored) in contacts {
                let node_id: NodeId = key.parse().map_err(|_| corrupt())?;
                if node_id.to_string() != key
                    || stored.added_at_ms < 0
                    || stored.last_seen_ms.is_some_and(|t| t < 0)
                {
                    return Err(corrupt());
                }
                let nickname = Nickname::parse(&stored.nickname).map_err(|_| corrupt())?;
                if nickname.as_str() != stored.nickname {
                    return Err(corrupt());
                }
                let self_name = match &stored.self_name {
                    Some(raw) => {
                        let name = validate_display_name(raw).map_err(|_| corrupt())?;
                        if name.as_str() != raw {
                            return Err(corrupt());
                        }
                        Some(name)
                    }
                    None => None,
                };
                map.insert(
                    node_id,
                    Contact {
                        node_id,
                        nickname,
                        self_name,
                        trust: stored.trust,
                        added_at_ms: stored.added_at_ms,
                        last_seen_ms: stored.last_seen_ms,
                    },
                );
            }
            if !map.is_empty() {
                ledger.0.insert(uuid, map);
            }
        }
        Ok(ledger)
    }

    fn from_ledger(ledger: &Ledger) -> Self {
        let accounts = ledger
            .0
            .iter()
            .map(|(uuid, contacts)| {
                let stored = contacts
                    .iter()
                    .map(|(id, c)| {
                        (
                            id.to_string(),
                            StoredContact {
                                nickname: c.nickname.as_str().to_owned(),
                                self_name: c.self_name.as_ref().map(|n| n.as_str().to_owned()),
                                trust: c.trust,
                                added_at_ms: c.added_at_ms,
                                last_seen_ms: c.last_seen_ms,
                            },
                        )
                    })
                    .collect();
                (uuid.hyphenated().to_string(), stored)
            })
            .collect();
        Self {
            version: CONTACTS_VERSION,
            accounts,
        }
    }
}

/// Interim durable store: `<data_dir>/contacts.json`, all accounts in one
/// versioned document. The path is a constant, never built from account ids
/// or names. A corrupt file is an error, never treated as empty.
pub struct FileContactStore {
    path: PathBuf,
    lock: Mutex<()>,
    #[cfg(test)]
    writes: std::sync::atomic::AtomicUsize,
}

impl FileContactStore {
    /// `data_dir` is the app-data directory. Performs no I/O.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: data_dir.into().join(CONTACTS_FILE_NAME),
            lock: Mutex::new(()),
            #[cfg(test)]
            writes: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(crate) fn writes(&self) -> usize {
        self.writes.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn tmp_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    }

    /// A missing file is an empty ledger; anything else unreadable or invalid
    /// is an error.
    fn read_ledger(&self) -> Result<Ledger, ContactStoreError> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Ledger::default()),
            Err(e) => return Err(ContactStoreError::Io(e)),
        };
        if !meta.file_type().is_file() || meta.len() > MAX_CONTACTS_BYTES {
            return Err(ContactStoreError::Corrupt);
        }
        let bytes = fs::read(&self.path).map_err(ContactStoreError::Io)?;
        if bytes.len() as u64 > MAX_CONTACTS_BYTES {
            return Err(ContactStoreError::Corrupt);
        }
        let doc: Document =
            serde_json::from_slice(&bytes).map_err(|_| ContactStoreError::Corrupt)?;
        doc.into_ledger()
    }

    fn write_ledger(&self, ledger: &Ledger) -> Result<(), ContactStoreError> {
        let bytes = serde_json::to_vec(&Document::from_ledger(ledger))
            .map_err(|_| ContactStoreError::Corrupt)?;
        // Never write a file the read cap would then refuse.
        if bytes.len() as u64 > MAX_CONTACTS_BYTES {
            return Err(ContactStoreError::Full);
        }
        let tmp_path = self.tmp_path();
        let result = (|| -> io::Result<()> {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            // Non-unix: confidentiality relies on the per-user app data dir ACLs.
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            let mut f = opts.open(&tmp_path)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp_path, &self.path)?;
            // Best-effort directory fsync so the rename is durable (unix).
            #[cfg(unix)]
            if let Some(dir) = self.path.parent() {
                if let Ok(d) = fs::File::open(dir) {
                    let _ = d.sync_all();
                }
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        #[cfg(test)]
        if result.is_ok() {
            self.writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        result.map_err(ContactStoreError::Io)
    }

    fn read(&self) -> Result<Ledger, ContactStoreError> {
        let _guard = self.lock.lock().map_err(|_| ContactStoreError::Corrupt)?;
        self.read_ledger()
    }

    /// Read-modify-write under the lock. `f` returns its result and whether a
    /// write is due.
    fn mutate<T>(
        &self,
        f: impl FnOnce(&mut Ledger) -> Result<(T, bool), ContactStoreError>,
    ) -> Result<T, ContactStoreError> {
        let _guard = self.lock.lock().map_err(|_| ContactStoreError::Corrupt)?;
        let mut ledger = self.read_ledger()?;
        let (value, write) = f(&mut ledger)?;
        if write {
            self.write_ledger(&ledger)?;
        }
        Ok(value)
    }
}

impl ContactStore for FileContactStore {
    fn list(&self, account: &AccountId) -> Result<Vec<Contact>, ContactStoreError> {
        Ok(self.read()?.list(account))
    }

    fn get(&self, account: &AccountId, id: &NodeId) -> Result<Option<Contact>, ContactStoreError> {
        Ok(self.read()?.get(account, id))
    }

    fn insert(&self, account: &AccountId, contact: Contact) -> Result<(), ContactStoreError> {
        self.mutate(|l| {
            l.insert(account, contact, MAX_CONTACTS_PER_ACCOUNT)?;
            Ok(((), true))
        })
    }

    fn insert_missing(
        &self,
        entries: Vec<(AccountId, Contact)>,
    ) -> Result<BatchReport, ContactStoreError> {
        self.mutate(|l| {
            let report = l.insert_missing(entries, MAX_CONTACTS_PER_ACCOUNT)?;
            let write = report.inserted > 0;
            Ok((report, write))
        })
    }

    fn set_nickname(
        &self,
        account: &AccountId,
        id: &NodeId,
        nickname: &Nickname,
    ) -> Result<Contact, ContactStoreError> {
        self.mutate(|l| Ok((l.set_nickname(account, id, nickname)?, true)))
    }

    fn set_trust(
        &self,
        account: &AccountId,
        id: &NodeId,
        trust: TrustState,
    ) -> Result<Contact, ContactStoreError> {
        self.mutate(|l| Ok((l.set_trust(account, id, trust)?, true)))
    }

    fn observe(
        &self,
        account: &AccountId,
        id: &NodeId,
        name: NameUpdate<'_>,
        seen_at_ms: Option<i64>,
    ) -> Result<(), ContactStoreError> {
        self.mutate(|l| Ok(((), l.observe(account, id, name, seen_at_ms))))
    }

    fn remove(&self, account: &AccountId, id: &NodeId) -> Result<bool, ContactStoreError> {
        self.mutate(|l| {
            let removed = l.remove(account, id);
            Ok((removed, removed))
        })
    }
}

#[cfg(test)]
mod tests;
