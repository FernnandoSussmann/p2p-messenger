//! Accept == add contact (DD-03): the single path that persists trust for an
//! inbound peer.

use super::book::ContactBook;
use super::model::{Contact, ContactError, HistoryChoice, Nickname};
use crate::discovery::{Node, RespondError};
use crate::identity::{DisplayName, DisplayNameError, NodeId};

/// The open Accept/Decline prompts of a node.
pub trait IncomingPrompts {
    /// `Some(name)` while a prompt for `peer` is open (name = its Hello name).
    fn pending_name(&self, peer: &NodeId) -> Option<Option<DisplayName>>;
    fn answer(&self, peer: &NodeId, accept: bool) -> Result<(), RespondError>;
}

impl IncomingPrompts for Node {
    fn pending_name(&self, peer: &NodeId) -> Option<Option<DisplayName>> {
        self.pending_request(peer).map(|r| r.display_name)
    }

    fn answer(&self, peer: &NodeId, accept: bool) -> Result<(), RespondError> {
        self.respond_incoming(peer, accept)
    }
}

#[derive(Debug)]
pub enum AcceptError {
    InvalidNickname(DisplayNameError),
    NoSuchRequest,
    Contacts(ContactError),
}

/// Accept == add contact. Order:
/// 1. validate the nickname (the prompt stays open on failure);
/// 2. read the open prompt (`NoSuchRequest` if none); `self_name` is its
///    Hello name;
/// 3. `book.add`; `AlreadyExists` is fine (the record is kept, not ours),
///    other errors are returned, the prompt stays open and the peer is NOT
///    admitted;
/// 4. answer the prompt; on failure, if step 3 created the record, remove it
///    again (history untouched) and return `NoSuchRequest`.
///
/// Blocking (file I/O): async callers use `spawn_blocking`.
pub fn accept_incoming(
    book: &ContactBook,
    prompts: &impl IncomingPrompts,
    peer: &NodeId,
    nickname: &str,
) -> Result<Contact, AcceptError> {
    Nickname::parse(nickname).map_err(AcceptError::InvalidNickname)?;
    let self_name = prompts
        .pending_name(peer)
        .ok_or(AcceptError::NoSuchRequest)?;
    let (contact, created) = match book.add(*peer, nickname, self_name) {
        Ok(contact) => (contact, true),
        Err(ContactError::AlreadyExists) => match book.get(peer) {
            Ok(Some(existing)) => (existing, false),
            Ok(None) => return Err(AcceptError::Contacts(ContactError::NotFound)),
            Err(e) => return Err(AcceptError::Contacts(e)),
        },
        Err(e) => return Err(AcceptError::Contacts(e)),
    };
    if prompts.answer(peer, true).is_err() {
        if created {
            let _ = book.remove(peer, HistoryChoice::Keep);
        }
        return Err(AcceptError::NoSuchRequest);
    }
    Ok(contact)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::contacts::testing::{FailingContactStore, MemoryContactStore};
    use crate::contacts::NoMessageStore;
    use crate::identity::{validate_display_name, AccountId};

    struct Fake {
        open: bool,
        answer_ok: bool,
        answered: std::sync::Mutex<usize>,
    }

    impl Fake {
        fn new(open: bool, answer_ok: bool) -> Self {
            Self {
                open,
                answer_ok,
                answered: Default::default(),
            }
        }
    }

    impl IncomingPrompts for Fake {
        fn pending_name(&self, _: &NodeId) -> Option<Option<DisplayName>> {
            self.open
                .then(|| Some(validate_display_name("Bob").unwrap()))
        }

        fn answer(&self, _: &NodeId, _: bool) -> Result<(), RespondError> {
            *self.answered.lock().unwrap() += 1;
            if self.answer_ok {
                Ok(())
            } else {
                Err(RespondError::NoSuchRequest)
            }
        }
    }

    fn id(seed: u8) -> NodeId {
        iroh_base::SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn book(store: Arc<dyn crate::contacts::ContactStore>) -> ContactBook {
        ContactBook::new(AccountId::new(), id(9), store, Arc::new(NoMessageStore))
    }

    #[test]
    fn accept_rolls_back_new_contact_when_prompt_vanished() {
        let b = book(Arc::new(MemoryContactStore::new()));
        let err = accept_incoming(&b, &Fake::new(true, false), &id(1), "Bobby").unwrap_err();
        assert!(matches!(err, AcceptError::NoSuchRequest));
        assert!(!b.is_known(&id(1)).unwrap());

        b.add(id(2), "Mine", None).unwrap();
        let err = accept_incoming(&b, &Fake::new(true, false), &id(2), "Other").unwrap_err();
        assert!(matches!(err, AcceptError::NoSuchRequest));
        assert_eq!(b.get(&id(2)).unwrap().unwrap().nickname.as_str(), "Mine");
    }

    #[test]
    fn accept_without_prompt_or_with_bad_nickname_changes_nothing() {
        let b = book(Arc::new(MemoryContactStore::new()));
        let none = Fake::new(false, true);
        assert!(matches!(
            accept_incoming(&b, &none, &id(1), "Bobby"),
            Err(AcceptError::NoSuchRequest)
        ));
        let open = Fake::new(true, true);
        assert!(matches!(
            accept_incoming(&b, &open, &id(1), "\u{202E}x"),
            Err(AcceptError::InvalidNickname(_))
        ));
        assert_eq!(*open.answered.lock().unwrap(), 0);
        assert!(b.list().unwrap().is_empty());
    }

    #[test]
    fn accept_store_failure_does_not_admit_peer() {
        let b = book(Arc::new(FailingContactStore));
        let prompts = Fake::new(true, true);
        let err = accept_incoming(&b, &prompts, &id(1), "Bobby").unwrap_err();
        assert!(matches!(
            err,
            AcceptError::Contacts(ContactError::Unavailable)
        ));
        assert_eq!(*prompts.answered.lock().unwrap(), 0);
    }

    #[test]
    fn accept_persists_unverified_contact_with_hello_name() {
        let b = book(Arc::new(MemoryContactStore::new()));
        let c = accept_incoming(&b, &Fake::new(true, true), &id(1), "Bobby").unwrap();
        assert_eq!(c.nickname.as_str(), "Bobby");
        assert_eq!(c.self_name.unwrap().as_str(), "Bob");
        assert!(b.is_known(&id(1)).unwrap());
    }
}
