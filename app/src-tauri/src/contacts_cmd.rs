//! Process-wide contacts state (exactly one store instance, bound to the
//! account once the identity exists) and the contact commands over it.

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use messenger_core::contacts::{
    ContactBook, ContactDto, ContactError, ContactStore, HistoryChoice, HistoryEraser,
    RemoveContactDto,
};
use messenger_core::identity::{AccountId, NodeId};
use serde::Serialize;
use tauri::State;

use crate::chat_cmd::ChatState;
use crate::identity_cmd::DisplayNameKind;
use crate::net_cmd::NetState;

/// Webview-facing error. Never embeds names, paths or backend details.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ContactsCmdError {
    NotReady,
    InvalidNodeId,
    SelfContact,
    InvalidNickname { kind: DisplayNameKind },
    AlreadyExists,
    NotFound,
    Full,
    StoreUnavailable,
    HistoryEraseFailed,
}

impl From<ContactError> for ContactsCmdError {
    fn from(err: ContactError) -> Self {
        match err {
            ContactError::InvalidNickname(e) => Self::InvalidNickname { kind: e.into() },
            ContactError::SelfContact => Self::SelfContact,
            ContactError::AlreadyExists => Self::AlreadyExists,
            ContactError::NotFound => Self::NotFound,
            ContactError::Full => Self::Full,
            ContactError::Unavailable => Self::StoreUnavailable,
            ContactError::HistoryEraseFailed => Self::HistoryEraseFailed,
        }
    }
}

const MAX_NODE_ID_INPUT_BYTES: usize = 128;

pub(crate) fn parse_node_id(raw: &str) -> Result<NodeId, ContactsCmdError> {
    if raw.len() > MAX_NODE_ID_INPUT_BYTES {
        return Err(ContactsCmdError::InvalidNodeId);
    }
    NodeId::from_str(raw).map_err(|_| ContactsCmdError::InvalidNodeId)
}

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

fn book_of(c: &ContactsState) -> Result<Arc<ContactBook>, ContactsCmdError> {
    c.book().ok_or(ContactsCmdError::NotReady)
}

/// Store access does file I/O, so it never runs on the async runtime.
async fn blocking<T: Send + 'static>(
    book: Arc<ContactBook>,
    f: impl FnOnce(&ContactBook) -> Result<T, ContactError> + Send + 'static,
) -> Result<T, ContactsCmdError> {
    tauri::async_runtime::spawn_blocking(move || f(&book))
        .await
        .map_err(|_| ContactsCmdError::StoreUnavailable)?
        .map_err(ContactsCmdError::from)
}

pub async fn list(c: &ContactsState) -> Result<Vec<ContactDto>, ContactsCmdError> {
    let book = book_of(c)?;
    let contacts = blocking(book, |b| b.list()).await?;
    Ok(contacts.iter().map(ContactDto::from).collect())
}

/// `self_name` comes from the node's own observation, never from the webview.
pub async fn add(
    c: &ContactsState,
    net: &NetState,
    node_id: &str,
    nickname: &str,
) -> Result<ContactDto, ContactsCmdError> {
    let book = book_of(c)?;
    let peer = parse_node_id(node_id)?;
    let self_name = net.node().ok().and_then(|n| n.last_seen_name(&peer));
    let nickname = nickname.to_owned();
    let contact = blocking(book, move |b| b.add(peer, &nickname, self_name)).await?;
    Ok(ContactDto::from(&contact))
}

pub async fn rename(
    c: &ContactsState,
    node_id: &str,
    nickname: &str,
) -> Result<ContactDto, ContactsCmdError> {
    let book = book_of(c)?;
    let peer = parse_node_id(node_id)?;
    let nickname = nickname.to_owned();
    let contact = blocking(book, move |b| b.rename(&peer, &nickname)).await?;
    Ok(ContactDto::from(&contact))
}

pub async fn set_verified(
    c: &ContactsState,
    node_id: &str,
    verified: bool,
) -> Result<ContactDto, ContactsCmdError> {
    let book = book_of(c)?;
    let peer = parse_node_id(node_id)?;
    let contact = blocking(book, move |b| b.set_verified(&peer, verified)).await?;
    Ok(ContactDto::from(&contact))
}

/// `clear_history: None` keeps history. After a successful removal the live
/// connection is closed so the revocation takes effect immediately, and the
/// peer's queued messages become Not delivered.
pub async fn remove(
    c: &ContactsState,
    net: &NetState,
    chat: &ChatState,
    node_id: &str,
    clear_history: Option<bool>,
) -> Result<RemoveContactDto, ContactsCmdError> {
    let book = book_of(c)?;
    let peer = parse_node_id(node_id)?;
    let choice = if clear_history == Some(true) {
        HistoryChoice::Erase
    } else {
        HistoryChoice::Keep
    };
    let removal = blocking(book, move |b| b.remove(&peer, choice)).await?;
    if let Ok(node) = net.node() {
        node.disconnect(&peer);
    }
    if let Ok(messaging) = chat.messaging() {
        messaging.forget_peer(peer).await;
    }
    Ok(RemoveContactDto::new(&peer, removal))
}

/// A store error is `StoreUnavailable`, never `false`.
pub async fn is_known(c: &ContactsState, node_id: &str) -> Result<bool, ContactsCmdError> {
    let book = book_of(c)?;
    let peer = parse_node_id(node_id)?;
    blocking(book, move |b| b.is_known(&peer)).await
}

#[tauri::command]
pub async fn contacts_list(
    contacts: State<'_, Arc<ContactsState>>,
) -> Result<Vec<ContactDto>, ContactsCmdError> {
    list(&contacts).await
}

#[tauri::command]
pub async fn contacts_add(
    contacts: State<'_, Arc<ContactsState>>,
    net: State<'_, NetState>,
    node_id: String,
    nickname: String,
) -> Result<ContactDto, ContactsCmdError> {
    add(&contacts, &net, &node_id, &nickname).await
}

#[tauri::command]
pub async fn contacts_rename(
    contacts: State<'_, Arc<ContactsState>>,
    node_id: String,
    nickname: String,
) -> Result<ContactDto, ContactsCmdError> {
    rename(&contacts, &node_id, &nickname).await
}

#[tauri::command]
pub async fn contacts_set_verified(
    contacts: State<'_, Arc<ContactsState>>,
    node_id: String,
    verified: bool,
) -> Result<ContactDto, ContactsCmdError> {
    set_verified(&contacts, &node_id, verified).await
}

#[tauri::command]
pub async fn contacts_remove(
    contacts: State<'_, Arc<ContactsState>>,
    net: State<'_, NetState>,
    chat: State<'_, Arc<ChatState>>,
    node_id: String,
    clear_history: Option<bool>,
) -> Result<RemoveContactDto, ContactsCmdError> {
    remove(&contacts, &net, &chat, &node_id, clear_history).await
}

#[tauri::command]
pub async fn contacts_is_known(
    contacts: State<'_, Arc<ContactsState>>,
    node_id: String,
) -> Result<bool, ContactsCmdError> {
    is_known(&contacts, &node_id).await
}

#[cfg(test)]
mod tests;
