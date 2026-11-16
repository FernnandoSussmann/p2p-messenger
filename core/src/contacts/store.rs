//! Contact persistence: the [`ContactStore`] trait, the shared in-memory
//! [`Ledger`] rules and the read-only parser of the pre-DD-05 `contacts.json`.
//!
//! The durable store is [`SqlCipherContactStore`](super::SqlCipherContactStore)
//! in the encrypted `messenger.db`; `MemoryContactStore` (test-support) is the
//! double. [`read_legacy_contacts`] only parses the old plaintext file; the
//! one-shot import lives in `storage::legacy`, which also overwrites and
//! removes the file after the import committed.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use uuid::Uuid;

use super::model::{apply_observation, Contact, NameUpdate, Nickname, TrustState};
use crate::identity::{validate_display_name, AccountId, NodeId};
use crate::storage::LegacyError;

pub const MAX_CONTACTS_PER_ACCOUNT: usize = 10_000;
/// `observe` does not write just to move last_seen by less than this.
pub const LAST_SEEN_RESOLUTION_MS: i64 = 5 * 60 * 1000;

const CONTACTS_FILE_NAME: &str = "contacts.json";
const CONTACTS_VERSION: u32 = 1;
/// 10k contacts with two names of at most 512 bytes each, plus headroom.
const MAX_LEGACY_CONTACTS_BYTES: u64 = 32 * 1024 * 1024;

pub trait ContactStore: Send + Sync + 'static {
    fn list(&self, account: &AccountId) -> Result<Vec<Contact>, ContactStoreError>;
    fn get(&self, account: &AccountId, id: &NodeId) -> Result<Option<Contact>, ContactStoreError>;
    /// Never overwrites: existing NodeId -> `AlreadyExists`.
    fn insert(&self, account: &AccountId, contact: Contact) -> Result<(), ContactStoreError>;
    /// Inserts every entry whose (account, NodeId) is absent, skips existing
    /// ones untouched, atomically. Any cap violation -> `Full` and nothing
    /// written.
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
    /// A stored row failed validation.
    Corrupt,
    /// The encrypted store could not be reached. Opaque on purpose: backend
    /// errors can carry paths or SQL.
    Unavailable,
}

impl fmt::Display for ContactStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("contact not found"),
            Self::AlreadyExists => f.write_str("contact already exists"),
            Self::Full => f.write_str("contact store is full"),
            Self::Corrupt => f.write_str("contact store is corrupt"),
            Self::Unavailable => f.write_str("contact store is unavailable"),
        }
    }
}

impl std::error::Error for ContactStoreError {}

/// In-memory contact set with the store contract's rules: the memory double
/// and the legacy reader build on it.
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
        self.entry_mut(account, id)
            .is_ok_and(|c| apply_observation(c, name, seen_at_ms))
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredContact {
    nickname: String,
    self_name: Option<String>,
    trust: TrustState,
    added_at_ms: i64,
    last_seen_ms: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    accounts: BTreeMap<String, BTreeMap<String, StoredContact>>,
}

impl Document {
    /// Validates every field and converts; any failure is `Corrupt`.
    fn into_entries(self) -> Result<Vec<(AccountId, Contact)>, LegacyError> {
        let corrupt = || LegacyError::Corrupt;
        if self.version != CONTACTS_VERSION {
            return Err(corrupt());
        }
        let mut out = Vec::new();
        for (account, contacts) in self.accounts {
            let uuid = Uuid::parse_str(&account).map_err(|_| corrupt())?;
            if uuid.hyphenated().to_string() != account || contacts.len() > MAX_CONTACTS_PER_ACCOUNT
            {
                return Err(corrupt());
            }
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
                out.push((
                    AccountId::from_uuid(uuid),
                    Contact {
                        node_id,
                        nickname,
                        self_name,
                        trust: stored.trust,
                        added_at_ms: stored.added_at_ms,
                        last_seen_ms: stored.last_seen_ms,
                    },
                ));
            }
        }
        Ok(out)
    }
}

/// Parses `<data_dir>/contacts.json`. Never opens the database and never
/// writes. `Ok(None)` means the file is absent. A symlink or other non-regular
/// file is `NotRegularFile` and is never followed.
pub(crate) fn read_legacy_contacts(
    data_dir: &Path,
) -> Result<Option<Vec<(AccountId, Contact)>>, LegacyError> {
    let path = data_dir.join(CONTACTS_FILE_NAME);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LegacyError::Io),
    };
    if !meta.file_type().is_file() {
        return Err(LegacyError::NotRegularFile);
    }
    if meta.len() > MAX_LEGACY_CONTACTS_BYTES {
        return Err(LegacyError::Corrupt);
    }
    let bytes = fs::read(&path).map_err(|_| LegacyError::Io)?;
    if bytes.len() as u64 > MAX_LEGACY_CONTACTS_BYTES {
        return Err(LegacyError::Corrupt);
    }
    let doc: Document = serde_json::from_slice(&bytes).map_err(|_| LegacyError::Corrupt)?;
    doc.into_entries().map(Some)
}

#[cfg(test)]
mod tests;
