//! [`ContactBook`]: one account's contacts plus DD-03's rules.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::history::HistoryEraser;
use super::model::{
    Contact, ContactError, HistoryChoice, NameUpdate, Nickname, Removal, TrustState,
};
use super::store::{ContactStore, ContactStoreError};
use crate::identity::{AccountId, DisplayName, NodeId};

impl From<ContactStoreError> for ContactError {
    fn from(e: ContactStoreError) -> Self {
        match e {
            ContactStoreError::NotFound => Self::NotFound,
            ContactStoreError::AlreadyExists => Self::AlreadyExists,
            ContactStoreError::Full => Self::Full,
            ContactStoreError::Corrupt | ContactStoreError::Io(_) => Self::Unavailable,
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Trust reads the NodeId only; names are never compared, looked up or used
/// for dedup.
pub struct ContactBook {
    account: AccountId,
    own: NodeId,
    store: Arc<dyn ContactStore>,
    history: Arc<dyn HistoryEraser>,
}

impl ContactBook {
    pub fn new(
        account: AccountId,
        own: NodeId,
        store: Arc<dyn ContactStore>,
        history: Arc<dyn HistoryEraser>,
    ) -> Self {
        Self {
            account,
            own,
            store,
            history,
        }
    }

    pub fn account(&self) -> &AccountId {
        &self.account
    }

    /// Sorted by nickname (case-insensitive), then NodeId string.
    pub fn list(&self) -> Result<Vec<Contact>, ContactError> {
        let mut contacts = self.store.list(&self.account)?;
        contacts
            .sort_by_cached_key(|c| (c.nickname.as_str().to_lowercase(), c.node_id.to_string()));
        Ok(contacts)
    }

    pub fn get(&self, id: &NodeId) -> Result<Option<Contact>, ContactError> {
        Ok(self.store.get(&self.account, id)?)
    }

    /// NodeId-only membership. Callers must treat `Err` as "unknown".
    pub fn is_known(&self, id: &NodeId) -> Result<bool, ContactError> {
        Ok(self.get(id)?.is_some())
    }

    /// User-typed nickname. Validates first (nothing written on failure);
    /// rejects `own` (`SelfContact`); always Unverified (there is no
    /// parameter to create a verified contact). `self_name` comes from the
    /// node's observation, never from webview input.
    pub fn add(
        &self,
        id: NodeId,
        nickname: &str,
        self_name: Option<DisplayName>,
    ) -> Result<Contact, ContactError> {
        let nickname = Nickname::parse(nickname).map_err(ContactError::InvalidNickname)?;
        if id == self.own {
            return Err(ContactError::SelfContact);
        }
        let contact = Contact {
            node_id: id,
            nickname,
            self_name,
            trust: TrustState::Unverified,
            added_at_ms: now_ms(),
            last_seen_ms: None,
        };
        self.store.insert(&self.account, contact.clone())?;
        Ok(contact)
    }

    /// A dialed peer answered with a valid Hello. If absent, inserts it
    /// Unverified with [`Nickname::default_for`]; if present, behaves like
    /// [`observe_hello`](Self::observe_hello) (nickname and trust untouched).
    pub fn remember_dialed(
        &self,
        id: NodeId,
        self_name: Option<&DisplayName>,
    ) -> Result<(), ContactError> {
        if id == self.own {
            return Err(ContactError::SelfContact);
        }
        if !self.is_known(&id)? {
            let now = now_ms();
            let contact = Contact {
                node_id: id,
                nickname: Nickname::default_for(&id, self_name),
                self_name: self_name.cloned(),
                trust: TrustState::Unverified,
                added_at_ms: now,
                last_seen_ms: Some(now),
            };
            match self.store.insert(&self.account, contact) {
                Ok(()) => return Ok(()),
                // Raced with another insert: fall through to an observation.
                Err(ContactStoreError::AlreadyExists) => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.observe_hello(&id, self_name)
    }

    pub fn rename(&self, id: &NodeId, nickname: &str) -> Result<Contact, ContactError> {
        let nickname = Nickname::parse(nickname).map_err(ContactError::InvalidNickname)?;
        Ok(self.store.set_nickname(&self.account, id, &nickname)?)
    }

    /// The only way trust changes: an explicit local user attestation.
    pub fn set_verified(&self, id: &NodeId, verified: bool) -> Result<Contact, ContactError> {
        let trust = if verified {
            TrustState::Verified
        } else {
            TrustState::Unverified
        };
        Ok(self.store.set_trust(&self.account, id, trust)?)
    }

    /// Hello on an authorized connection: `self_name` = `name` (`None`
    /// clears), `last_seen` = now (throttled by the store).
    pub fn observe_hello(
        &self,
        id: &NodeId,
        name: Option<&DisplayName>,
    ) -> Result<(), ContactError> {
        Ok(self
            .store
            .observe(&self.account, id, NameUpdate::Set(name), Some(now_ms()))?)
    }

    /// LAN record: `self_name` = `name`; `last_seen` untouched (not a connection).
    pub fn observe_lan(&self, id: &NodeId, name: &DisplayName) -> Result<(), ContactError> {
        Ok(self
            .store
            .observe(&self.account, id, NameUpdate::Set(Some(name)), None)?)
    }

    /// Removes the contact, which reverts trust: the peer is prompted again.
    ///
    /// Order is binding (03-02 copies it into the UI copy):
    /// 1. an absent contact is `NotFound` and nothing is erased;
    /// 2. with `Erase`, history is erased first; if that fails the contact is
    ///    KEPT (`HistoryEraseFailed`), so the erase can be retried from the
    ///    contact row. Removing the contact first would strand history the
    ///    contact UI can no longer reach;
    /// 3. the contact is removed; if that fails (`Unavailable`) history may
    ///    already be gone, but the contact exists and a retry is safe because
    ///    erase is idempotent.
    ///
    /// With `Keep`, the eraser is never called.
    pub fn remove(&self, id: &NodeId, history: HistoryChoice) -> Result<Removal, ContactError> {
        if self.get(id)?.is_none() {
            return Err(ContactError::NotFound);
        }
        if history == HistoryChoice::Erase {
            self.history
                .erase_peer_history(&self.account, id)
                .map_err(|_| ContactError::HistoryEraseFailed)?;
        }
        self.store.remove(&self.account, id)?;
        Ok(Removal {
            history_erased: history == HistoryChoice::Erase,
        })
    }
}

#[cfg(test)]
mod tests;
