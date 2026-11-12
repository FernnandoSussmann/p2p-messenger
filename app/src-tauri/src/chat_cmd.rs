//! The managed chat state (the encrypted message store and the delivery
//! engine) and the chat commands over it.
//!
//! The logic lives in plain functions over `ChatState` so it can be tested
//! without a Tauri runtime; the `#[tauri::command]` wrappers only forward the
//! managed state. History and search need only the store, so they work even
//! when the node never started. Nothing here logs a body or a query.

use std::sync::{Arc, OnceLock};

use messenger_core::contacts::{HistoryEraseError, HistoryEraser};
use messenger_core::discovery::{KnownPeers, Node};
use messenger_core::identity::{AccountId, NodeId};
use messenger_core::messaging::{
    chat_event_json, history_page, parse_message_id, ChatEvent, HistoryCursor, HistoryPageDto,
    MessageDto, MessageStatusDto, MessageStore, MessageStoreError, Messaging, MessagingConfig,
    MessagingDeps, SendError, SqlCipherMessageStore, CHAT_RESYNC, DEFAULT_SEARCH_RESULTS,
    MAX_SEARCH_RESULTS,
};
use messenger_core::storage::{Storage, StorageError};
use serde::Serialize;
use serde_json::Value;
use tauri::State;
use tokio::sync::broadcast::error::RecvError;

use crate::contacts_cmd::parse_node_id;

/// The encrypted message store, opened once the identity exists, and the
/// engine started over it. Until the store opens, history erase fails closed.
pub struct ChatState {
    store: OnceLock<(AccountId, Arc<SqlCipherMessageStore>)>,
    messaging: OnceLock<Arc<Messaging>>,
    starting: tokio::sync::Mutex<()>,
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            store: OnceLock::new(),
            messaging: OnceLock::new(),
            starting: tokio::sync::Mutex::new(()),
        }
    }

    /// Idempotent: once a store is open, later calls do nothing.
    pub fn open_store(&self, storage: &Storage, account: &AccountId) -> Result<(), StorageError> {
        if self.store.get().is_some() {
            return Ok(());
        }
        let store = SqlCipherMessageStore::new(storage.database(account)?);
        let _ = self.store.set((*account, Arc::new(store)));
        Ok(())
    }

    /// Starts the engine once, over the opened store. Later calls return the
    /// running engine.
    pub async fn start_messaging(
        &self,
        node: &Node,
        known: Arc<dyn KnownPeers>,
        config: MessagingConfig,
    ) -> Result<Arc<Messaging>, ChatCmdError> {
        let (_, store) = self.store_ref()?;
        let _guard = self.starting.lock().await;
        if let Some(running) = self.messaging.get() {
            return Ok(running.clone());
        }
        let deps = MessagingDeps { known, store };
        let messaging = Messaging::start(node, deps, config)
            .await
            .map_err(ChatCmdError::from)?;
        let messaging = Arc::new(messaging);
        let _ = self.messaging.set(messaging.clone());
        Ok(messaging)
    }

    /// `StoreUnavailable` if the store is absent; `NotReady` if it is open but
    /// messaging is not started.
    pub fn messaging(&self) -> Result<Arc<Messaging>, ChatCmdError> {
        self.store_ref()?;
        self.messaging.get().cloned().ok_or(ChatCmdError::NotReady)
    }

    pub fn store_ref(&self) -> Result<(AccountId, Arc<SqlCipherMessageStore>), ChatCmdError> {
        self.store
            .get()
            .cloned()
            .ok_or(ChatCmdError::StoreUnavailable)
    }

    pub fn shutdown(&self) {
        if let Some(messaging) = self.messaging.get() {
            messaging.shutdown();
        }
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

/// Webview-facing error: a type tag only, never a body, id or backend detail.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ChatCmdError {
    NotReady,
    StoreUnavailable,
    InvalidNodeId,
    InvalidMessageId,
    InvalidCursor,
    SelfMessage,
    NotAContact,
    EmptyBody,
    BodyTooLarge,
    OutboxFull,
    NotFound,
    NotResendable,
    InvalidQuery,
    ContactsUnavailable,
    Internal,
}

impl From<SendError> for ChatCmdError {
    fn from(err: SendError) -> Self {
        match err {
            SendError::EmptyBody => Self::EmptyBody,
            SendError::BodyTooLarge => Self::BodyTooLarge,
            SendError::SelfMessage => Self::SelfMessage,
            SendError::NotAContact => Self::NotAContact,
            SendError::ContactsUnavailable => Self::ContactsUnavailable,
            SendError::OutboxFull => Self::OutboxFull,
            SendError::NotFound => Self::NotFound,
            SendError::NotResendable => Self::NotResendable,
            SendError::StoreUnavailable => Self::StoreUnavailable,
            SendError::ShuttingDown => Self::NotReady,
        }
    }
}

impl From<MessageStoreError> for ChatCmdError {
    fn from(err: MessageStoreError) -> Self {
        match err {
            MessageStoreError::InvalidQuery => Self::InvalidQuery,
            _ => Self::StoreUnavailable,
        }
    }
}

fn node_id_of(raw: &str) -> Result<NodeId, ChatCmdError> {
    parse_node_id(raw).map_err(|_| ChatCmdError::InvalidNodeId)
}

pub async fn send(
    chat: &ChatState,
    node_id: &str,
    body: String,
) -> Result<MessageDto, ChatCmdError> {
    let peer = node_id_of(node_id)?;
    let messaging = chat.messaging()?;
    let row = messaging.send(peer, body).await?;
    Ok(MessageDto::from(&row))
}

pub async fn resend(
    chat: &ChatState,
    node_id: &str,
    message_id: &str,
) -> Result<MessageStatusDto, ChatCmdError> {
    let peer = node_id_of(node_id)?;
    let id = parse_message_id(message_id).ok_or(ChatCmdError::InvalidMessageId)?;
    let messaging = chat.messaging()?;
    let status = messaging.resend(peer, id).await?;
    Ok(MessageStatusDto::new(&peer, &id, status))
}

pub async fn history(
    chat: &ChatState,
    node_id: &str,
    before: Option<String>,
    limit: Option<u32>,
) -> Result<HistoryPageDto, ChatCmdError> {
    let peer = node_id_of(node_id)?;
    let before = match before {
        Some(token) => Some(HistoryCursor::parse_token(&token).ok_or(ChatCmdError::InvalidCursor)?),
        None => None,
    };
    let (account, store) = chat.store_ref()?;
    let limit = limit.map(|l| l as usize);
    let page = tauri::async_runtime::spawn_blocking(move || {
        history_page(store.as_ref(), &account, &peer, before, limit)
    })
    .await
    .map_err(|_| ChatCmdError::Internal)??;
    Ok(HistoryPageDto::from(&page))
}

pub async fn search(
    chat: &ChatState,
    query: String,
    node_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<MessageDto>, ChatCmdError> {
    let peer = node_id.as_deref().map(node_id_of).transpose()?;
    let (account, store) = chat.store_ref()?;
    let limit = limit.map_or(DEFAULT_SEARCH_RESULTS, |l| {
        (l as usize).clamp(1, MAX_SEARCH_RESULTS)
    });
    let rows = tauri::async_runtime::spawn_blocking(move || {
        store.search(&account, peer.as_ref(), &query, limit)
    })
    .await
    .map_err(|_| ChatCmdError::Internal)??;
    Ok(rows.iter().map(MessageDto::from).collect())
}

/// Maps a chat event result to its external name and payload and hands it to
/// `emit`. A lagged stream asks the UI to reload. Returns false to stop.
pub fn forward_chat(ev: Result<ChatEvent, RecvError>, emit: impl Fn(&str, Value)) -> bool {
    match ev {
        Ok(ev) => {
            let (name, payload) = chat_event_json(&ev);
            emit(name, payload);
            true
        }
        Err(RecvError::Lagged(_)) => {
            emit(CHAT_RESYNC, Value::Null);
            true
        }
        Err(RecvError::Closed) => false,
    }
}

#[tauri::command]
pub async fn chat_send(
    chat: State<'_, Arc<ChatState>>,
    node_id: String,
    body: String,
) -> Result<MessageDto, ChatCmdError> {
    send(&chat, &node_id, body).await
}

#[tauri::command]
pub async fn chat_resend(
    chat: State<'_, Arc<ChatState>>,
    node_id: String,
    message_id: String,
) -> Result<MessageStatusDto, ChatCmdError> {
    resend(&chat, &node_id, &message_id).await
}

#[tauri::command]
pub async fn chat_history(
    chat: State<'_, Arc<ChatState>>,
    node_id: String,
    before: Option<String>,
    limit: Option<u32>,
) -> Result<HistoryPageDto, ChatCmdError> {
    history(&chat, &node_id, before, limit).await
}

#[tauri::command]
pub async fn history_search(
    chat: State<'_, Arc<ChatState>>,
    query: String,
    node_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<MessageDto>, ChatCmdError> {
    search(&chat, query, node_id, limit).await
}

#[cfg(test)]
mod tests;
