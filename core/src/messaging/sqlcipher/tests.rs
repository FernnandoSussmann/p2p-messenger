use std::fs;
use std::path::{Path, PathBuf};

use protocol::{new_message_id, ChatMessage, MAX_BODY_BYTES};

use super::*;
use crate::identity::{AccountId, DbKey};
use crate::messaging::contract::{chat, contract_suite, node};
use crate::storage::{Create, Database, DB_FILE, MIGRATIONS};

fn key() -> DbKey {
    DbKey::from_bytes(rand::random::<[u8; 32]>())
}

fn db_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(DB_FILE)
}

fn open(dir: &tempfile::TempDir, key: &DbKey) -> SqlCipherMessageStore {
    let db = Database::open(&db_path(dir), key, MIGRATIONS, Create::Allowed).unwrap();
    SqlCipherMessageStore::new(Arc::new(db))
}

fn canary() -> String {
    format!("canary-{}", uuid::Uuid::new_v4())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn files_with_prefix(dir: &Path, prefix: &str) -> Vec<Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .map(|e| fs::read(e.path()).unwrap())
        .collect()
}

fn terms(store: &SqlCipherMessageStore) -> Vec<String> {
    let conn = store.conn().unwrap();
    conn.execute_batch("CREATE VIRTUAL TABLE temp.v USING fts5vocab(main, messages_fts, 'row');")
        .unwrap();
    let mut stmt = conn
        .prepare("SELECT term FROM temp.v ORDER BY term")
        .unwrap();
    let out: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(stmt);
    conn.execute_batch("DROP TABLE temp.v;").unwrap();
    out
}

#[test]
fn sqlcipher_store_meets_contract() {
    let dir = tempfile::tempdir().unwrap();
    contract_suite(&open(&dir, &key()));
}

#[test]
fn reopen_with_same_key_reads_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (k, account, peer) = (key(), AccountId::new(), node());
    let row = {
        let store = open(&dir, &k);
        store
            .insert_outgoing(
                &account,
                &peer,
                &chat(1, "persisted"),
                MessageStatus::Pending,
            )
            .unwrap()
    };
    let store = open(&dir, &k);
    assert_eq!(store.page(&account, &peer, None, 10).unwrap(), vec![row]);
    assert_eq!(
        store.search(&account, None, "persisted", 10).unwrap().len(),
        1
    );
}

#[test]
fn unknown_status_text_reads_as_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account, peer) = (open(&dir, &key()), AccountId::new(), node());
    let row = store
        .insert_outgoing(&account, &peer, &chat(1, "x"), MessageStatus::Sending)
        .unwrap();
    store
        .conn()
        .unwrap()
        .execute_batch("UPDATE messages SET status = 'queued';")
        .unwrap();
    assert_eq!(
        store.get(&account, &peer, &row.id),
        Err(MessageStoreError::Unavailable)
    );
}

#[test]
fn sqlcipher_file_never_contains_plaintext_body() {
    let dir = tempfile::tempdir().unwrap();
    let (account, peer, secret) = (AccountId::new(), node(), canary());
    {
        let store = open(&dir, &key());
        store
            .insert_outgoing(&account, &peer, &chat(1, &secret), MessageStatus::Sending)
            .unwrap();
        store
            .insert_incoming(&account, &peer, &chat(2, &secret), 3)
            .unwrap();
    }
    let files = files_with_prefix(dir.path(), DB_FILE);
    assert!(!files.is_empty());
    for bytes in files {
        assert!(!contains(&bytes, secret.as_bytes()));
    }
}

#[test]
fn erase_leaves_no_fts_terms() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account, keep, gone) = (open(&dir, &key()), AccountId::new(), node(), node());
    store
        .insert_outgoing(
            &account,
            &keep,
            &chat(1, "control row"),
            MessageStatus::Sending,
        )
        .unwrap();
    let before = terms(&store);
    let secret = canary();
    store
        .insert_outgoing(&account, &gone, &chat(2, &secret), MessageStatus::Sending)
        .unwrap();
    assert!(terms(&store).len() > before.len());
    store.erase_peer_history(&account, &gone).unwrap();
    assert_eq!(terms(&store), before);
    let hits: i64 = store
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
            [format!("\"{secret}\"")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 0);
    assert!(store
        .search(&account, None, &secret, 10)
        .unwrap()
        .is_empty());
}

#[test]
fn erase_leaves_no_plaintext_in_export() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account, keep, gone) = (open(&dir, &key()), AccountId::new(), node(), node());
    let (control, secret) = (canary(), canary());
    store
        .insert_outgoing(&account, &keep, &chat(1, &control), MessageStatus::Sending)
        .unwrap();
    store
        .insert_outgoing(&account, &gone, &chat(2, &secret), MessageStatus::Sending)
        .unwrap();
    store.erase_peer_history(&account, &gone).unwrap();

    let export = dir.path().join("export.plain");
    // The connection is opened without SQLITE_OPEN_CREATE, so ATTACH needs the file.
    fs::write(&export, b"").unwrap();
    {
        let conn = store.conn().unwrap();
        conn.execute(
            "ATTACH DATABASE ?1 AS plaintext KEY ''",
            [export.to_str().unwrap()],
        )
        .unwrap();
        conn.query_row("SELECT sqlcipher_export('plaintext')", [], |_| Ok(()))
            .unwrap();
        conn.execute_batch("DETACH DATABASE plaintext;").unwrap();
    }
    let plain = fs::read(&export).unwrap();
    assert!(contains(&plain, control.as_bytes()));
    assert!(!contains(&plain, secret.as_bytes()));
}

#[test]
fn insert_rejects_empty_and_oversized_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account, peer) = (open(&dir, &key()), AccountId::new(), node());
    let oversized = "a".repeat(MAX_BODY_BYTES + 1);
    for bad in ["", "  \n\t ", oversized.as_str()] {
        let msg = ChatMessage {
            id: new_message_id(),
            sent_at_ms: 1,
            body: bad.to_owned(),
        };
        assert_eq!(
            store.insert_outgoing(&account, &peer, &msg, MessageStatus::Sending),
            Err(MessageStoreError::InvalidBody)
        );
        assert_eq!(
            store.insert_incoming(&account, &peer, &msg, 1),
            Err(MessageStoreError::InvalidBody)
        );
    }
    let exact = chat(1, &"a".repeat(MAX_BODY_BYTES));
    store
        .insert_outgoing(&account, &peer, &exact, MessageStatus::Sending)
        .unwrap();
    assert_eq!(store.page(&account, &peer, None, 10).unwrap().len(), 1);
}
