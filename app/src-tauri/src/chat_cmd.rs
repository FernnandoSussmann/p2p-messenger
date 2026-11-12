//! The managed chat state: the encrypted message store, opened once the
//! identity exists.

use std::sync::{Arc, OnceLock};

use messenger_core::contacts::{HistoryEraseError, HistoryEraser};
use messenger_core::identity::{
    AccountId, IdentityService, KeychainStore, NodeId, SettingsStore, TicketProvider,
};
use messenger_core::messaging::{open_message_store, MessageStoreError, SqlCipherMessageStore};

/// The encrypted message store, opened once the identity exists. Until then,
/// or if opening failed, history erase fails closed.
pub struct ChatState {
    store: OnceLock<(AccountId, Arc<SqlCipherMessageStore>)>,
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            store: OnceLock::new(),
        }
    }

    /// Idempotent: once a store is open, later calls do nothing.
    pub fn open_store<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
        &self,
        identity: &IdentityService<K, S, T>,
    ) -> Result<(), MessageStoreError> {
        if self.store.get().is_some() {
            return Ok(());
        }
        let (account, store) = open_message_store(identity)?;
        let _ = self.store.set((account, Arc::new(store)));
        Ok(())
    }

    // Read by the chat commands added in step 04-02; tests use it today.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn store(&self) -> Option<(AccountId, Arc<SqlCipherMessageStore>)> {
        self.store.get().cloned()
    }
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryEraser for ChatState {
    fn erase_peer_history(
        &self,
        account: &AccountId,
        peer: &NodeId,
    ) -> Result<(), HistoryEraseError> {
        match self.store.get() {
            Some((_, store)) => store.erase_peer_history(account, peer),
            None => Err(HistoryEraseError),
        }
    }
}

#[cfg(test)]
mod tests {
    use messenger_core::identity::testing::FakeKeychain;
    use messenger_core::identity::{MemorySettingsStore, NodeIdTicketProvider};
    use messenger_core::messaging::DB_FILE;

    use super::*;

    fn provisioned(
        dir: &tempfile::TempDir,
    ) -> IdentityService<FakeKeychain, MemorySettingsStore, NodeIdTicketProvider> {
        let identity = IdentityService::new(
            FakeKeychain::new(),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            dir.path(),
        );
        identity.provision().unwrap();
        identity
    }

    #[test]
    fn erase_without_store_fails_closed() {
        let peer = iroh::SecretKey::generate().public();
        assert!(ChatState::new()
            .erase_peer_history(&AccountId::new(), &peer)
            .is_err());
    }

    #[test]
    fn open_store_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let identity = provisioned(&dir);
        let chat = ChatState::new();
        assert!(chat.store().is_none());
        chat.open_store(&identity).unwrap();
        let (account, first) = chat.store().unwrap();
        chat.open_store(&identity).unwrap();
        let (again, second) = chat.store().unwrap();
        assert_eq!(account, again);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(dir.path().join(DB_FILE).exists());
    }

    #[test]
    fn open_store_fails_without_identity() {
        let dir = tempfile::tempdir().unwrap();
        let identity = IdentityService::new(
            FakeKeychain::new(),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            dir.path(),
        );
        let chat = ChatState::new();
        assert_eq!(
            chat.open_store(&identity),
            Err(MessageStoreError::Unavailable)
        );
        assert!(chat.store().is_none());
        assert!(!dir.path().join(DB_FILE).exists());
    }
}
