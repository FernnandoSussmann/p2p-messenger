//! Process-wide contacts state: exactly one store instance, bound to the
//! account once the identity exists.

use std::sync::{Arc, OnceLock};

use messenger_core::contacts::{ContactBook, ContactStore, HistoryEraser};
use messenger_core::identity::{AccountId, NodeId};

pub struct ContactsState {
    store: Arc<dyn ContactStore>,
    history: Arc<dyn HistoryEraser>,
    book: OnceLock<Arc<ContactBook>>,
}

impl ContactsState {
    pub fn new(store: Arc<dyn ContactStore>, history: Arc<dyn HistoryEraser>) -> Self {
        Self {
            store,
            history,
            book: OnceLock::new(),
        }
    }

    /// Idempotent: the first call wins.
    pub fn bind(&self, account: AccountId, own: NodeId) -> Arc<ContactBook> {
        self.book
            .get_or_init(|| {
                Arc::new(ContactBook::new(
                    account,
                    own,
                    Arc::clone(&self.store),
                    Arc::clone(&self.history),
                ))
            })
            .clone()
    }

    pub fn book(&self) -> Option<Arc<ContactBook>> {
        self.book.get().cloned()
    }
}

#[cfg(test)]
mod tests {
    use messenger_core::contacts::testing::{MemoryContactStore, RecordingHistoryEraser};

    use super::*;

    fn node(seed: u8) -> NodeId {
        iroh::SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn contacts_state_bind_is_idempotent() {
        let state = ContactsState::new(
            Arc::new(MemoryContactStore::new()),
            Arc::new(RecordingHistoryEraser::new()),
        );
        assert!(state.book().is_none());
        let first = state.bind(AccountId::new(), node(1));
        let second = state.bind(AccountId::new(), node(2));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.account(), second.account());
        assert!(Arc::ptr_eq(&state.book().unwrap(), &first));
    }
}
