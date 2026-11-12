//! Test doubles for the messaging slice (behind `test-support`).

use std::sync::{Mutex, MutexGuard};

use protocol::{ChatMessage, MessageId};

use super::model::{
    validate_body, HistoryCursor, Insert, MessageDirection, MessageStatus, MessageStoreError,
    StoredMessage,
};
use super::store::{clean_query, MessageStore, MAX_PAGE, MAX_SEARCH_RESULTS};
use crate::contacts::{HistoryEraseError, HistoryEraser};
use crate::identity::{AccountId, NodeId};

#[derive(Default)]
struct Inner {
    rows: Vec<(AccountId, StoredMessage)>,
    last_seq: i64,
}

impl Inner {
    fn find(&self, account: &AccountId, peer: &NodeId, id: &MessageId) -> Option<usize> {
        self.rows
            .iter()
            .position(|(a, m)| a == account && m.peer == *peer && m.id == *id)
    }

    fn push(&mut self, account: &AccountId, mut row: StoredMessage) -> StoredMessage {
        self.last_seq += 1;
        row.seq = self.last_seq;
        self.rows.push((*account, row.clone()));
        row
    }
}

fn newest_first(rows: &mut [StoredMessage]) {
    rows.sort_by_key(|m| std::cmp::Reverse((m.sent_at_ms, m.seq)));
}

/// In-memory store with the same contract as the SQLCipher store, except that
/// search is ASCII-only case-insensitive.
#[derive(Default)]
pub struct MemoryMessageStore {
    inner: Mutex<Inner>,
}

impl MemoryMessageStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, MessageStoreError> {
        self.inner
            .lock()
            .map_err(|_| MessageStoreError::Unavailable)
    }
}

fn stored(
    peer: &NodeId,
    msg: &ChatMessage,
    direction: MessageDirection,
    received_at_ms: Option<i64>,
    status: MessageStatus,
) -> StoredMessage {
    StoredMessage {
        seq: 0,
        peer: *peer,
        id: msg.id,
        direction,
        sent_at_ms: msg.sent_at_ms,
        received_at_ms,
        body: msg.body.clone(),
        status,
    }
}

impl MessageStore for MemoryMessageStore {
    fn insert_outgoing(
        &self,
        account: &AccountId,
        peer: &NodeId,
        msg: &ChatMessage,
        status: MessageStatus,
    ) -> Result<StoredMessage, MessageStoreError> {
        validate_body(&msg.body)?;
        if status == MessageStatus::Received {
            return Err(MessageStoreError::InvalidQuery);
        }
        let mut inner = self.lock()?;
        if inner.find(account, peer, &msg.id).is_some() {
            return Err(MessageStoreError::AlreadyExists);
        }
        let row = stored(peer, msg, MessageDirection::Outgoing, None, status);
        Ok(inner.push(account, row))
    }

    fn insert_incoming(
        &self,
        account: &AccountId,
        peer: &NodeId,
        msg: &ChatMessage,
        received_at_ms: i64,
    ) -> Result<Insert, MessageStoreError> {
        validate_body(&msg.body)?;
        let mut inner = self.lock()?;
        if inner.find(account, peer, &msg.id).is_some() {
            return Ok(Insert::Duplicate);
        }
        let row = stored(
            peer,
            msg,
            MessageDirection::Incoming,
            Some(received_at_ms),
            MessageStatus::Received,
        );
        Ok(Insert::Inserted(inner.push(account, row)))
    }

    fn set_status(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &MessageId,
        status: MessageStatus,
    ) -> Result<bool, MessageStoreError> {
        if status == MessageStatus::Received {
            return Err(MessageStoreError::InvalidQuery);
        }
        let mut inner = self.lock()?;
        let Some(i) = inner.find(account, peer, id) else {
            return Ok(false);
        };
        let row = &mut inner.rows[i].1;
        if row.direction != MessageDirection::Outgoing || row.status == MessageStatus::Delivered {
            return Ok(false);
        }
        row.status = status;
        Ok(true)
    }

    fn get(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &MessageId,
    ) -> Result<Option<StoredMessage>, MessageStoreError> {
        let inner = self.lock()?;
        Ok(inner
            .find(account, peer, id)
            .map(|i| inner.rows[i].1.clone()))
    }

    fn page(
        &self,
        account: &AccountId,
        peer: &NodeId,
        before: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        let inner = self.lock()?;
        let mut rows: Vec<StoredMessage> = inner
            .rows
            .iter()
            .filter(|(a, m)| a == account && m.peer == *peer)
            .filter(|(_, m)| before.is_none_or(|c| (m.sent_at_ms, m.seq) < (c.sent_at_ms, c.seq)))
            .map(|(_, m)| m.clone())
            .collect();
        newest_first(&mut rows);
        rows.truncate(limit.clamp(1, MAX_PAGE));
        rows.reverse();
        Ok(rows)
    }

    fn search(
        &self,
        account: &AccountId,
        peer: Option<&NodeId>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        let needle = clean_query(query)?.to_ascii_lowercase();
        let inner = self.lock()?;
        let mut rows: Vec<StoredMessage> = inner
            .rows
            .iter()
            .filter(|(a, m)| a == account && peer.is_none_or(|p| m.peer == *p))
            .filter(|(_, m)| m.body.to_ascii_lowercase().contains(&needle))
            .map(|(_, m)| m.clone())
            .collect();
        newest_first(&mut rows);
        rows.truncate(limit.clamp(1, MAX_SEARCH_RESULTS));
        Ok(rows)
    }

    fn undelivered(
        &self,
        account: &AccountId,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        let inner = self.lock()?;
        let mut rows: Vec<StoredMessage> = inner
            .rows
            .iter()
            .filter(|(a, m)| {
                a == account
                    && m.direction == MessageDirection::Outgoing
                    && matches!(m.status, MessageStatus::Sending | MessageStatus::Pending)
            })
            .map(|(_, m)| m.clone())
            .collect();
        rows.sort_by_key(|m| m.seq);
        rows.truncate(limit);
        Ok(rows)
    }
}

impl HistoryEraser for MemoryMessageStore {
    fn erase_peer_history(
        &self,
        account: &AccountId,
        peer: &NodeId,
    ) -> Result<(), HistoryEraseError> {
        let mut inner = self.inner.lock().map_err(|_| HistoryEraseError)?;
        inner
            .rows
            .retain(|(a, m)| !(a == account && m.peer == *peer));
        Ok(())
    }
}

/// Fails every call with `Unavailable` / `HistoryEraseError`.
pub struct FailingMessageStore;

impl MessageStore for FailingMessageStore {
    fn insert_outgoing(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: &ChatMessage,
        _: MessageStatus,
    ) -> Result<StoredMessage, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }

    fn insert_incoming(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: &ChatMessage,
        _: i64,
    ) -> Result<Insert, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }

    fn set_status(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: &MessageId,
        _: MessageStatus,
    ) -> Result<bool, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }

    fn get(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: &MessageId,
    ) -> Result<Option<StoredMessage>, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }

    fn page(
        &self,
        _: &AccountId,
        _: &NodeId,
        _: Option<HistoryCursor>,
        _: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }

    fn search(
        &self,
        _: &AccountId,
        _: Option<&NodeId>,
        _: &str,
        _: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }

    fn undelivered(
        &self,
        _: &AccountId,
        _: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        Err(MessageStoreError::Unavailable)
    }
}

impl HistoryEraser for FailingMessageStore {
    fn erase_peer_history(&self, _: &AccountId, _: &NodeId) -> Result<(), HistoryEraseError> {
        Err(HistoryEraseError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::contract::contract_suite;

    #[test]
    fn memory_store_meets_contract() {
        contract_suite(&MemoryMessageStore::new());
    }

    #[test]
    fn failing_store_fails_every_call() {
        let (a, p) = (AccountId::new(), crate::messaging::contract::node());
        let msg = crate::messaging::contract::chat(1, "hi");
        let s = FailingMessageStore;
        assert_eq!(
            s.insert_outgoing(&a, &p, &msg, MessageStatus::Sending),
            Err(MessageStoreError::Unavailable)
        );
        assert_eq!(
            s.insert_incoming(&a, &p, &msg, 1),
            Err(MessageStoreError::Unavailable)
        );
        assert!(s.page(&a, &p, None, 1).is_err());
        assert!(s.search(&a, None, "hi", 1).is_err());
        assert!(s.undelivered(&a, 1).is_err());
        assert!(s.get(&a, &p, &msg.id).is_err());
        assert!(s
            .set_status(&a, &p, &msg.id, MessageStatus::Failed)
            .is_err());
        assert!(s.erase_peer_history(&a, &p).is_err());
    }
}
