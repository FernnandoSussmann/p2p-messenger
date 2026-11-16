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

fn page_count(store: &SqlCipherMessageStore) -> i64 {
    store
        .conn()
        .unwrap()
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap()
}

fn fill(store: &SqlCipherMessageStore, account: &AccountId, peer: &NodeId, n: u32, body: &str) {
    for i in 0..n {
        store
            .insert_outgoing(
                account,
                peer,
                &chat(i64::from(i) + 1, &format!("{body} {i}")),
                MessageStatus::Sending,
            )
            .unwrap();
    }
}

#[test]
fn erase_reclaims_free_pages() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account, gone) = (open(&dir, &key()), AccountId::new(), node());
    fill(&store, &account, &gone, 500, &"filler text ".repeat(20));
    let before = page_count(&store);
    store.erase_peer_history(&account, &gone).unwrap();
    assert_eq!(store.freelist_count().unwrap(), 0);
    assert!(page_count(&store) < before);
}

#[test]
fn vacuumed_db_reopens_with_key_and_rejects_wrong_key() {
    let dir = tempfile::tempdir().unwrap();
    let (k, account, keep, gone) = (key(), AccountId::new(), node(), node());
    let store = open(&dir, &k);
    fill(&store, &account, &keep, 3, "kept");
    fill(&store, &account, &gone, 50, "erased");
    store.erase_peer_history(&account, &gone).unwrap();
    drop(store);
    let reopened = open(&dir, &k);
    assert_eq!(reopened.page(&account, &keep, None, 10).unwrap().len(), 3);
    assert!(reopened.page(&account, &gone, None, 10).unwrap().is_empty());
    drop(reopened);
    assert!(Database::open(&db_path(&dir), &key(), MIGRATIONS, Create::Forbidden).is_err());
}

#[test]
fn erase_raw_file_has_no_canary() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account, keep, gone) = (open(&dir, &key()), AccountId::new(), node(), node());
    let secret = canary();
    fill(&store, &account, &keep, 5, "control");
    fill(&store, &account, &gone, 20, &secret);
    store.erase_peer_history(&account, &gone).unwrap();
    for bytes in files_with_prefix(dir.path(), "") {
        assert!(!contains(&bytes, secret.as_bytes()));
    }
}

fn peak_rss_kb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[test]
#[ignore = "manual measurement: cargo test -p core --features test-support --lib vacuum_cost -- --ignored --nocapture"]
fn vacuum_cost_on_100k_messages() {
    let dir = tempfile::tempdir().unwrap();
    let (store, account) = (open(&dir, &key()), AccountId::new());
    let peers: Vec<NodeId> = (0..50).map(|_| node()).collect();
    let body = "x".repeat(200);
    {
        let mut conn = store.conn().unwrap();
        let tx = conn.transaction().unwrap();
        for i in 0..100_000usize {
            tx.execute(
                "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, ?5, 'sent')",
                rusqlite::params![
                    account.as_uuid().as_bytes().as_slice(),
                    peers[i % 50].to_string(),
                    uuid::Uuid::new_v4().as_bytes().as_slice(),
                    i as i64,
                    format!("{body}{i}"),
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let size = fs::metadata(db_path(&dir)).unwrap().len();
    let rss_before = peak_rss_kb();
    let start = std::time::Instant::now();
    store.erase_peer_history(&account, &peers[0]).unwrap();
    let elapsed = start.elapsed();
    let delta = rss_before.zip(peak_rss_kb()).map(|(a, b)| b - a);
    println!("db_bytes={size} wall={elapsed:?} rss_delta_kb={delta:?}");
}
