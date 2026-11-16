use std::sync::atomic::Ordering;
use std::time::Duration;

use messenger_core::identity::testing::FakeKeychain;
use messenger_core::identity::{
    IdentityService, KeychainStore, MemorySettingsStore, NodeIdTicketProvider,
};
use messenger_core::messaging::{MessageStatus, DEFAULT_PAGE, MAX_BODY_BYTES, MAX_PAGE};
use messenger_core::storage::{Storage, StorageError, DB_FILE};
use protocol::{new_message_id, ChatMessage};
use serde_json::json;

use super::*;
use crate::test_util::{
    assert_forms_absent, connect_pair, started_with_chat, ChatLog, StartedChat, WAIT,
};

/// A provisioned account with its DB created, as the first-run flow leaves it.
fn provisioned(dir: &tempfile::TempDir) -> (Storage, AccountId) {
    let keychain = Arc::new(FakeKeychain::new());
    let identity = IdentityService::new(
        keychain.clone(),
        MemorySettingsStore::new(),
        NodeIdTicketProvider,
        dir.path(),
    );
    let account = identity.provision().unwrap().account_id;
    let storage = Storage::new(dir.path(), keychain);
    storage.database_creating(&account).unwrap();
    (storage, account)
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
    let (storage, account) = provisioned(&dir);
    let chat = ChatState::new();
    assert!(chat.store_ref().is_err());
    chat.open_store(&storage, &account).unwrap();
    let (stored, first) = chat.store_ref().unwrap();
    chat.open_store(&storage, &account).unwrap();
    let (again, second) = chat.store_ref().unwrap();
    assert_eq!((account, account), (stored, again));
    assert!(Arc::ptr_eq(&first, &second));
    assert!(dir.path().join(DB_FILE).exists());
}

#[test]
fn open_store_fails_without_identity() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path(), Arc::new(FakeKeychain::new()));
    let chat = ChatState::new();
    assert_eq!(
        chat.open_store(&storage, &AccountId::new()),
        Err(StorageError::KeyMissing)
    );
    assert!(chat.store_ref().is_err());
    assert!(!dir.path().join(DB_FILE).exists());
}

fn stranger() -> NodeId {
    iroh::SecretKey::generate().public()
}

fn seed(s: &StartedChat, peer: &NodeId, count: i64, sent_at: impl Fn(i64) -> i64) {
    let (account, store) = s.chat.store_ref().unwrap();
    for i in 0..count {
        let msg = ChatMessage {
            id: new_message_id(),
            sent_at_ms: sent_at(i),
            body: format!("seed {i}"),
        };
        store
            .insert_outgoing(&account, peer, &msg, MessageStatus::Delivered)
            .unwrap();
    }
}

async fn connected() -> (StartedChat, StartedChat) {
    let (a, b) = (
        started_with_chat("Alice").await,
        started_with_chat("Bob").await,
    );
    connect_pair(&a, &b).await;
    (a, b)
}

async fn wait_no_channel(a: &StartedChat, b: &StartedChat) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while a.messaging.has_channel(&b.id) {
        assert!(tokio::time::Instant::now() < deadline, "channel stayed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[test]
fn chat_errors_serialize_with_type_tag() {
    for (err, tag) in [
        (ChatCmdError::NotReady, "NotReady"),
        (ChatCmdError::StoreUnavailable, "StoreUnavailable"),
        (ChatCmdError::InvalidNodeId, "InvalidNodeId"),
        (ChatCmdError::InvalidMessageId, "InvalidMessageId"),
        (ChatCmdError::InvalidCursor, "InvalidCursor"),
        (ChatCmdError::SelfMessage, "SelfMessage"),
        (ChatCmdError::NotAContact, "NotAContact"),
        (ChatCmdError::EmptyBody, "EmptyBody"),
        (ChatCmdError::BodyTooLarge, "BodyTooLarge"),
        (ChatCmdError::OutboxFull, "OutboxFull"),
        (ChatCmdError::NotFound, "NotFound"),
        (ChatCmdError::NotResendable, "NotResendable"),
        (ChatCmdError::InvalidQuery, "InvalidQuery"),
        (ChatCmdError::ContactsUnavailable, "ContactsUnavailable"),
        (ChatCmdError::NotConfirmed, "NotConfirmed"),
        (ChatCmdError::HistoryClearFailed, "HistoryClearFailed"),
        (ChatCmdError::Internal, "Internal"),
    ] {
        assert_eq!(serde_json::to_value(err).unwrap(), json!({ "type": tag }));
    }
}

#[test]
fn send_error_mapping_is_total() {
    for (err, want) in [
        (SendError::EmptyBody, ChatCmdError::EmptyBody),
        (SendError::BodyTooLarge, ChatCmdError::BodyTooLarge),
        (SendError::SelfMessage, ChatCmdError::SelfMessage),
        (SendError::NotAContact, ChatCmdError::NotAContact),
        (
            SendError::ContactsUnavailable,
            ChatCmdError::ContactsUnavailable,
        ),
        (SendError::OutboxFull, ChatCmdError::OutboxFull),
        (SendError::NotFound, ChatCmdError::NotFound),
        (SendError::NotResendable, ChatCmdError::NotResendable),
        (SendError::StoreUnavailable, ChatCmdError::StoreUnavailable),
        (SendError::ShuttingDown, ChatCmdError::NotReady),
    ] {
        assert_eq!(ChatCmdError::from(err), want);
    }
    assert_eq!(
        ChatCmdError::from(MessageStoreError::InvalidQuery),
        ChatCmdError::InvalidQuery
    );
    assert_eq!(
        ChatCmdError::from(MessageStoreError::Unavailable),
        ChatCmdError::StoreUnavailable
    );
}

#[tokio::test]
async fn readiness_codes() {
    let peer = stranger().to_string();
    let id = new_message_id().to_string();
    let fresh = ChatState::new();
    let gone = ChatCmdError::StoreUnavailable;
    assert_eq!(send(&fresh, &peer, "hi".into()).await.unwrap_err(), gone);
    assert_eq!(resend(&fresh, &peer, &id).await.unwrap_err(), gone);
    assert_eq!(history(&fresh, &peer, None, None).await.unwrap_err(), gone);
    assert_eq!(
        search(&fresh, "hi".into(), None, None).await.unwrap_err(),
        gone
    );

    let dir = tempfile::tempdir().unwrap();
    let (storage, account) = provisioned(&dir);
    let chat = ChatState::new();
    chat.open_store(&storage, &account).unwrap();
    assert_eq!(
        send(&chat, &peer, "hi".into()).await.unwrap_err(),
        ChatCmdError::NotReady
    );
    assert_eq!(
        resend(&chat, &peer, &id).await.unwrap_err(),
        ChatCmdError::NotReady
    );
    assert!(history(&chat, &peer, None, None)
        .await
        .unwrap()
        .messages
        .is_empty());
    assert!(search(&chat, "hi".into(), None, None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_flow_end_to_end() {
    let (a, b) = connected().await;
    let (log_a, log_b) = (ChatLog::record(&a.messaging), ChatLog::record(&b.messaging));
    let body = "hi 👋\nline 2";

    let sent = send(&a.chat, &b.id.to_string(), body.into()).await.unwrap();
    assert_eq!(sent.status, "sending");
    log_a.wait_status(&sent.id, "delivered").await;

    let deadline = tokio::time::Instant::now() + WAIT;
    while log_b.named("message_received").is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "never received");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let received = log_b.named("message_received");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0]["id"], sent.id.as_str());
    assert_eq!(received[0]["body"], body);

    let mine = history(&a.chat, &b.id.to_string(), None, None)
        .await
        .unwrap();
    let theirs = history(&b.chat, &a.id.to_string(), None, None)
        .await
        .unwrap();
    assert_eq!(mine.messages.len(), 1);
    assert_eq!(mine.messages[0].direction, "out");
    assert_eq!(theirs.messages.len(), 1);
    assert_eq!(theirs.messages[0].direction, "in");
    assert_eq!(theirs.messages[0].body, body);
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_then_delivered_after_reconnect() {
    let (a, b) = connected().await;
    let log_a = ChatLog::record(&a.messaging);
    a.state.node().unwrap().disconnect(&b.id);
    wait_no_channel(&a, &b).await;

    let sent = send(&a.chat, &b.id.to_string(), "later".into())
        .await
        .unwrap();
    assert_eq!(sent.status, "pending");
    connect_pair(&a, &b).await;
    log_a.wait_status(&sent.id, "delivered").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resend_flow_via_command() {
    let (a, b) = connected().await;
    let log_a = ChatLog::record(&a.messaging);
    b.faults.drop_acks.store(1000, Ordering::SeqCst);
    let sent = send(&a.chat, &b.id.to_string(), "retry me".into())
        .await
        .unwrap();
    log_a.wait_status(&sent.id, "failed").await;

    b.faults.drop_acks.store(0, Ordering::SeqCst);
    let status = resend(&a.chat, &b.id.to_string(), &sent.id).await.unwrap();
    assert_eq!(status.id, sent.id);
    log_a.wait_status(&sent.id, "delivered").await;

    assert_eq!(
        resend(&a.chat, &b.id.to_string(), &sent.id)
            .await
            .unwrap_err(),
        ChatCmdError::NotResendable
    );
    assert_eq!(
        resend(&a.chat, &b.id.to_string(), "garbage")
            .await
            .unwrap_err(),
        ChatCmdError::InvalidMessageId
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_send_rejects_non_contact_and_bad_input() {
    let a = started_with_chat("Alice").await;
    let other = stranger().to_string();
    assert_eq!(
        send(&a.chat, &other, "hi".into()).await.unwrap_err(),
        ChatCmdError::NotAContact
    );
    assert_eq!(
        send(&a.chat, &a.id.to_string(), "hi".into())
            .await
            .unwrap_err(),
        ChatCmdError::SelfMessage
    );
    assert_eq!(
        send(&a.chat, &other, " \n".into()).await.unwrap_err(),
        ChatCmdError::EmptyBody
    );
    assert_eq!(
        send(&a.chat, &other, "x".repeat(MAX_BODY_BYTES + 1))
            .await
            .unwrap_err(),
        ChatCmdError::BodyTooLarge
    );
    assert_eq!(
        send(&a.chat, &"9".repeat(1024), "hi".into())
            .await
            .unwrap_err(),
        ChatCmdError::InvalidNodeId
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn history_paging_via_command() {
    let a = started_with_chat("Alice").await;
    let peer = stranger();
    seed(&a, &peer, 60, |i| 1_000 + i);
    let first = history(&a.chat, &peer.to_string(), None, None)
        .await
        .unwrap();
    assert_eq!(first.messages.len(), 50);
    assert_eq!(first.messages[0].body, "seed 10");
    assert_eq!(first.messages[49].body, "seed 59");
    let token = first.next_before.unwrap();
    assert_eq!(token, first.messages[0].cursor);

    let rest = history(&a.chat, &peer.to_string(), Some(token), None)
        .await
        .unwrap();
    assert_eq!(rest.messages.len(), 10);
    assert_eq!(rest.messages[0].body, "seed 0");
    assert!(rest.next_before.is_none());

    let tied = stranger();
    seed(&a, &tied, 7, |_| 5_000);
    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let page = history(&a.chat, &tied.to_string(), before, Some(3))
            .await
            .unwrap();
        seen.splice(0..0, page.messages.iter().map(|m| m.id.clone()));
        match page.next_before {
            Some(token) => before = Some(token),
            None => break,
        }
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!((seen.len(), unique.len()), (7, 7));
}

#[tokio::test(flavor = "multi_thread")]
async fn history_limits_are_clamped() {
    let a = started_with_chat("Alice").await;
    let peer = stranger();
    seed(&a, &peer, (MAX_PAGE + 5) as i64, |i| i);
    let id = peer.to_string();
    let page = |limit| history(&a.chat, &id, None, limit);
    assert_eq!(page(Some(0)).await.unwrap().messages.len(), 1);
    assert_eq!(page(None).await.unwrap().messages.len(), DEFAULT_PAGE);
    assert_eq!(page(Some(u32::MAX)).await.unwrap().messages.len(), MAX_PAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn history_rejects_bad_cursor() {
    let a = started_with_chat("Alice").await;
    let peer = stranger().to_string();
    for bad in ["garbage", "1:2:3", "", &"1".repeat(49)] {
        assert_eq!(
            history(&a.chat, &peer, Some(bad.to_owned()), None)
                .await
                .unwrap_err(),
            ChatCmdError::InvalidCursor
        );
    }
    assert_eq!(
        history(&a.chat, "nope", None, None).await.unwrap_err(),
        ChatCmdError::InvalidNodeId
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_via_command() {
    let a = started_with_chat("Alice").await;
    let (p1, p2) = (stranger(), stranger());
    let (account, store) = a.chat.store_ref().unwrap();
    for (peer, body) in [(&p1, "needle one"), (&p2, "needle two"), (&p1, "other")] {
        let msg = ChatMessage {
            id: new_message_id(),
            sent_at_ms: 1,
            body: body.into(),
        };
        store
            .insert_outgoing(&account, peer, &msg, MessageStatus::Delivered)
            .unwrap();
    }
    let all = search(&a.chat, "NEEDLE".into(), None, None).await.unwrap();
    assert_eq!(all.len(), 2);
    let one = search(&a.chat, "needle".into(), Some(p1.to_string()), Some(10))
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].node_id, p1.to_string());
    for bad in [String::new(), "  ".into(), "x".repeat(129)] {
        assert_eq!(
            search(&a.chat, bad, None, None).await.unwrap_err(),
            ChatCmdError::InvalidQuery
        );
    }
    assert_eq!(
        search(&a.chat, "needle".into(), Some("bad".into()), None)
            .await
            .unwrap_err(),
        ChatCmdError::InvalidNodeId
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn received_dto_body_is_neutralized() {
    let (a, b) = connected().await;
    let log_b = ChatLog::record(&b.messaging);
    let persian = "می\u{200C}خواهم";
    for body in ["a\u{202E}b", persian] {
        send(&a.chat, &b.id.to_string(), body.into()).await.unwrap();
    }
    let deadline = tokio::time::Instant::now() + WAIT;
    while log_b.named("message_received").len() < 2 {
        assert!(tokio::time::Instant::now() < deadline, "never received");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let bodies: Vec<_> = log_b
        .named("message_received")
        .iter()
        .map(|v| v["body"].as_str().unwrap().to_owned())
        .collect();
    assert!(bodies.contains(&"a\u{FFFD}b".to_owned()));
    assert!(bodies.contains(&persian.to_owned()));
}

#[test]
fn forward_chat_emits_resync_on_lag() {
    let seen = std::sync::Mutex::new(Vec::new());
    let keep = forward_chat(Err(RecvError::Lagged(7)), |name, payload| {
        seen.lock().unwrap().push((name.to_owned(), payload));
    });
    assert!(keep);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(CHAT_RESYNC.to_owned(), Value::Null)]
    );
}

#[test]
fn forward_chat_stops_on_closed() {
    let emitted = std::cell::Cell::new(false);
    assert!(!forward_chat(Err(RecvError::Closed), |_, _| emitted.set(true)));
    assert!(!emitted.get());
}

#[tokio::test(flavor = "multi_thread")]
async fn dto_json_never_contains_db_key() {
    let (a, b) = connected().await;
    let log_b = ChatLog::record(&b.messaging);
    let sent = send(&a.chat, &b.id.to_string(), "secretless".into())
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    while log_b.named("message_received").is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "never received");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let account = a.identity.load().unwrap().account_id;
    let key = *a.keychain.load_db_key(&account).unwrap().as_bytes();
    let page = history(&a.chat, &b.id.to_string(), None, None)
        .await
        .unwrap();
    let found = search(&a.chat, "secretless".into(), None, None)
        .await
        .unwrap();
    let jsons = [
        serde_json::to_string(&sent).unwrap(),
        serde_json::to_string(&page).unwrap(),
        serde_json::to_string(&found).unwrap(),
        log_b.all()[0].1.to_string(),
    ];
    for json in &jsons {
        assert_forms_absent(json, &key);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_with_messaging_completes() {
    let (a, b) = connected().await;
    let node = a.state.node().unwrap();
    assert_eq!(std::sync::Arc::strong_count(&node), 2);
    drop(node);
    a.chat.shutdown();
    assert_eq!(
        send(&a.chat, &b.id.to_string(), "late".into())
            .await
            .unwrap_err(),
        ChatCmdError::NotReady
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        a.state.shutdown_with_timeout(Duration::from_secs(2)),
    )
    .await
    .expect("shutdown completes");
    assert!(a.state.node().is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn history_clear_requires_confirmation() {
    let a = started_with_chat("Alice").await;
    let peer = stranger();
    seed(&a, &peer, 3, |i| i);
    assert_eq!(
        clear(&a.chat, &peer.to_string(), false).await.unwrap_err(),
        ChatCmdError::NotConfirmed
    );
    let page = history(&a.chat, &peer.to_string(), None, None)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn history_clear_end_to_end() {
    let (a, b) = connected().await;
    let log_a = ChatLog::record(&a.messaging);
    let (a_id, b_id) = (a.id.to_string(), b.id.to_string());
    let sent = send(&a.chat, &b_id, "needle from alice".into())
        .await
        .unwrap();
    log_a.wait_status(&sent.id, "delivered").await;
    let deadline = tokio::time::Instant::now() + WAIT;
    while history(&b.chat, &a_id, None, None)
        .await
        .unwrap()
        .messages
        .is_empty()
    {
        assert!(tokio::time::Instant::now() < deadline, "never received");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    clear(&a.chat, &b_id, true).await.unwrap();
    assert!(history(&a.chat, &b_id, None, None)
        .await
        .unwrap()
        .messages
        .is_empty());
    assert!(search(&a.chat, "needle".into(), Some(b_id), None)
        .await
        .unwrap()
        .is_empty());
    let theirs = history(&b.chat, &a_id, None, None).await.unwrap();
    assert_eq!(theirs.messages.len(), 1);
    assert_eq!(theirs.messages[0].body, "needle from alice");
    let (_, store) = a.chat.store_ref().unwrap();
    assert_eq!(store.freelist_count().unwrap(), 0);
}

#[tokio::test]
async fn history_clear_without_store_fails_closed() {
    let peer = stranger().to_string();
    assert_eq!(
        clear(&ChatState::new(), &peer, true).await.unwrap_err(),
        ChatCmdError::StoreUnavailable
    );
}

#[tokio::test]
async fn history_clear_rejects_bad_node_id() {
    assert_eq!(
        clear(&ChatState::new(), "nope", true).await.unwrap_err(),
        ChatCmdError::InvalidNodeId
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn history_clear_fails_queued_messages() {
    let (a, b) = connected().await;
    let log_a = ChatLog::record(&a.messaging);
    a.state.node().unwrap().disconnect(&b.id);
    wait_no_channel(&a, &b).await;
    let sent = send(&a.chat, &b.id.to_string(), "queued".into())
        .await
        .unwrap();
    assert_eq!(sent.status, "pending");

    clear(&a.chat, &b.id.to_string(), true).await.unwrap();
    log_a.wait_status(&sent.id, "failed").await;
    connect_pair(&a, &b).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(history(&b.chat, &a.id.to_string(), None, None)
        .await
        .unwrap()
        .messages
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn history_clear_on_removed_contact_works() {
    let a = started_with_chat("Alice").await;
    let peer = stranger();
    seed(&a, &peer, 5, |i| i);
    assert!(!a.contacts.book().unwrap().is_known(&peer).unwrap());
    clear(&a.chat, &peer.to_string(), true).await.unwrap();
    assert!(history(&a.chat, &peer.to_string(), None, None)
        .await
        .unwrap()
        .messages
        .is_empty());
}
