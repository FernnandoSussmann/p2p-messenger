//! Test doubles for the contacts slice (behind `test-support`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::book::ContactBook;
use super::history::{HistoryEraseError, HistoryEraser};
use super::model::{Contact, NameUpdate, Nickname, TrustState};
use super::store::{
    BatchReport, ContactStore, ContactStoreError, Ledger, MAX_CONTACTS_PER_ACCOUNT,
};
use crate::discovery::testing::KnownFactory;
use crate::discovery::KnownPeers;
use crate::identity::{AccountId, NodeId};

/// In-memory store with the same contract as `SqlCipherContactStore`, including
/// `insert_missing` atomicity, caps and observe throttling.
pub struct MemoryContactStore {
    state: Mutex<Ledger>,
    cap: usize,
    writes: AtomicUsize,
}

impl MemoryContactStore {
    pub fn new() -> Self {
        Self::with_cap(MAX_CONTACTS_PER_ACCOUNT)
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            state: Mutex::new(Ledger::default()),
            cap,
            writes: AtomicUsize::new(0),
        }
    }

    /// Number of mutations that would have rewritten a file.
    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Ledger>, ContactStoreError> {
        self.state.lock().map_err(|_| ContactStoreError::Corrupt)
    }

    fn wrote(&self) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

impl Default for MemoryContactStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ContactStore for MemoryContactStore {
    fn list(&self, account: &AccountId) -> Result<Vec<Contact>, ContactStoreError> {
        Ok(self.lock()?.list(account))
    }

    fn get(&self, account: &AccountId, id: &NodeId) -> Result<Option<Contact>, ContactStoreError> {
        Ok(self.lock()?.get(account, id))
    }

    fn insert(&self, account: &AccountId, contact: Contact) -> Result<(), ContactStoreError> {
        self.lock()?.insert(account, contact, self.cap)?;
        self.wrote();
        Ok(())
    }

    fn insert_missing(
        &self,
        entries: Vec<(AccountId, Contact)>,
    ) -> Result<BatchReport, ContactStoreError> {
        let report = self.lock()?.insert_missing(entries, self.cap)?;
        if report.inserted > 0 {
            self.wrote();
        }
        Ok(report)
    }

    fn set_nickname(
        &self,
        account: &AccountId,
        id: &NodeId,
        nickname: &Nickname,
    ) -> Result<Contact, ContactStoreError> {
        let c = self.lock()?.set_nickname(account, id, nickname)?;
        self.wrote();
        Ok(c)
    }

    fn set_trust(
        &self,
        account: &AccountId,
        id: &NodeId,
        trust: TrustState,
    ) -> Result<Contact, ContactStoreError> {
        let c = self.lock()?.set_trust(account, id, trust)?;
        self.wrote();
        Ok(c)
    }

    fn observe(
        &self,
        account: &AccountId,
        id: &NodeId,
        name: NameUpdate<'_>,
        seen_at_ms: Option<i64>,
    ) -> Result<(), ContactStoreError> {
        if self.lock()?.observe(account, id, name, seen_at_ms) {
            self.wrote();
        }
        Ok(())
    }

    fn remove(&self, account: &AccountId, id: &NodeId) -> Result<bool, ContactStoreError> {
        let removed = self.lock()?.remove(account, id);
        if removed {
            self.wrote();
        }
        Ok(removed)
    }
}

/// Every call fails with `Corrupt`.
pub struct FailingContactStore;

impl ContactStore for FailingContactStore {
    fn list(&self, _: &AccountId) -> Result<Vec<Contact>, ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn get(&self, _: &AccountId, _: &NodeId) -> Result<Option<Contact>, ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn insert(&self, _: &AccountId, _: Contact) -> Result<(), ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn insert_missing(
        &self,
        _: Vec<(AccountId, Contact)>,
    ) -> Result<BatchReport, ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn set_nickname(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: &Nickname,
    ) -> Result<Contact, ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn set_trust(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: TrustState,
    ) -> Result<Contact, ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn observe(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: NameUpdate<'_>,
        _: Option<i64>,
    ) -> Result<(), ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }

    fn remove(&self, _: &AccountId, _: &NodeId) -> Result<bool, ContactStoreError> {
        Err(ContactStoreError::Corrupt)
    }
}

/// Records every erase request; `failing()` records the call, then errors.
pub struct RecordingHistoryEraser {
    calls: Mutex<Vec<(AccountId, NodeId)>>,
    fail: bool,
}

impl RecordingHistoryEraser {
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Self::new()
        }
    }

    pub fn calls(&self) -> Vec<(AccountId, NodeId)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Default for RecordingHistoryEraser {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryEraser for RecordingHistoryEraser {
    fn erase_peer_history(
        &self,
        account: &AccountId,
        peer: &NodeId,
    ) -> Result<(), HistoryEraseError> {
        self.calls.lock().unwrap().push((*account, *peer));
        if self.fail {
            Err(HistoryEraseError)
        } else {
            Ok(())
        }
    }
}

/// Shared slot holding the last [`ContactBook`] built by [`BookCell::factory`],
/// so a test can reach the book a node was started with.
#[derive(Clone, Default)]
pub struct BookCell(Arc<Mutex<Option<Arc<ContactBook>>>>);

impl BookCell {
    pub fn new() -> Self {
        Self::default()
    }

    /// A closure building a book over the same `store` and `history` each time
    /// it is called, and recording it here.
    pub fn factory(
        &self,
        store: Arc<dyn ContactStore>,
        history: Arc<dyn HistoryEraser>,
    ) -> impl Fn(AccountId, NodeId) -> Arc<ContactBook> + Send + Sync + 'static {
        let slot = Arc::clone(&self.0);
        move |account, own| {
            let book = Arc::new(ContactBook::new(
                account,
                own,
                Arc::clone(&store),
                Arc::clone(&history),
            ));
            *slot.lock().unwrap() = Some(Arc::clone(&book));
            book
        }
    }

    /// [`factory`](Self::factory) as a discovery [`KnownFactory`].
    pub fn known_factory(
        &self,
        store: Arc<dyn ContactStore>,
        history: Arc<dyn HistoryEraser>,
    ) -> KnownFactory {
        let build = self.factory(store, history);
        Arc::new(move |account, own| build(account, own) as Arc<dyn KnownPeers>)
    }

    /// The last book built.
    ///
    /// # Panics
    /// If no book has been built yet.
    pub fn get(&self) -> Arc<ContactBook> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .expect("no ContactBook built yet")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::contract::{concurrent_observers, contract_suite, isolation_suite};

    #[test]
    fn contract_memory() {
        let store = MemoryContactStore::new();
        contract_suite(&store, AccountId::new());
        isolation_suite(&store, AccountId::new(), &store, AccountId::new());
        concurrent_observers(&store, AccountId::new());
    }
}
