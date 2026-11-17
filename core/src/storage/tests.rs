use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::migrate::{self, Migration};
use super::schema::{MIGRATIONS, SCHEMA_VERSION};
use super::*;
use crate::identity::testing::FakeKeychain;
use crate::identity::{write_marker, AccountId, DbKey, KeychainError, KeychainStore};
use crate::messaging::{MessageStatus, MessageStore, SqlCipherMessageStore};

fn key() -> DbKey {
    DbKey::from_bytes(rand::random::<[u8; 32]>())
}

fn db_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(DB_FILE)
}

fn open_with(
    dir: &tempfile::TempDir,
    key: &DbKey,
    list: &[Migration],
) -> Result<Database, StorageError> {
    Database::open(&db_path(dir), key, list, Create::Allowed)
}

fn open(dir: &tempfile::TempDir, key: &DbKey) -> Database {
    open_with(dir, key, MIGRATIONS).unwrap()
}

fn user_version(db: &Database) -> u32 {
    db.conn()
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn peer() -> String {
    iroh::SecretKey::generate().public().to_string()
}

fn insert_raw(db: &Database, account: &AccountId, peer: &str, n: u8) {
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, NULL, 'body', 'pending')",
            rusqlite::params![
                account.as_uuid().as_bytes().as_slice(),
                peer,
                uuid::Uuid::new_v4().as_bytes().as_slice(),
                i64::from(n)
            ],
        )
        .unwrap();
}

/// A keychain whose key loads are counted.
struct Counting {
    inner: FakeKeychain,
    loads: AtomicUsize,
}

impl KeySource for Counting {
    fn db_key(&self, account: &AccountId) -> Result<DbKey, KeychainError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load_db_key(account)
    }
}

/// A data dir with the marker written (an installed identity) and its DB key stored.
fn installed() -> (tempfile::TempDir, Arc<FakeKeychain>, AccountId, DbKey) {
    let dir = tempfile::tempdir().unwrap();
    let (keychain, account, k) = (Arc::new(FakeKeychain::new()), AccountId::new(), key());
    keychain.store_db_key(&account, &k).unwrap();
    write_marker(dir.path(), account).unwrap();
    (dir, keychain, account, k)
}

#[test]
fn create_then_reopen_with_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    let (account, p) = (AccountId::new(), peer());
    insert_raw(&open(&dir, &k), &account, &p, 1);
    let db = open(&dir, &k);
    let n: i64 = db
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn special_key_bytes_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = rand::random::<[u8; 32]>();
    bytes[..4].copy_from_slice(&[0x22, 0x27, 0x00, 0x22]);
    let k = DbKey::from_bytes(bytes);
    open(&dir, &k);
    open(&dir, &DbKey::from_bytes(bytes));
}

#[test]
fn wrong_key_is_reported_and_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    insert_raw(&open(&dir, &key()), &AccountId::new(), &peer(), 1);
    let before = fs::read(db_path(&dir)).unwrap();
    let err = open_with(&dir, &key(), MIGRATIONS).err().unwrap();
    assert_eq!(err, StorageError::WrongKeyOrCorrupt);
    assert_eq!(fs::read(db_path(&dir)).unwrap(), before);
}

#[test]
fn garbage_file_is_reported_and_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let garbage = rand::random::<[u8; 64]>().repeat(64);
    fs::write(db_path(&dir), &garbage).unwrap();
    let err = open_with(&dir, &key(), MIGRATIONS).err().unwrap();
    assert_eq!(err, StorageError::WrongKeyOrCorrupt);
    assert_eq!(fs::read(db_path(&dir)).unwrap(), garbage);
}

#[test]
fn mid_file_corruption_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    {
        let db = open(&dir, &k);
        let (account, p) = (AccountId::new(), peer());
        for n in 0..200u8 {
            insert_raw(&db, &account, &p, n);
        }
    }
    let mut bytes = fs::read(db_path(&dir)).unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 0xff;
    fs::write(db_path(&dir), &bytes).unwrap();
    let err = open_with(&dir, &k, MIGRATIONS).err().unwrap();
    assert_eq!(err, StorageError::Corrupt);
    assert_eq!(fs::read(db_path(&dir)).unwrap(), bytes);
}

#[test]
fn newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    open(&dir, &k)
        .conn()
        .unwrap()
        .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
        .unwrap();
    let before = fs::read(db_path(&dir)).unwrap();
    let err = open_with(&dir, &k, MIGRATIONS).err().unwrap();
    assert_eq!(err, StorageError::NewerSchema);
    assert_eq!(fs::read(db_path(&dir)).unwrap(), before);
}

#[test]
fn sqlcipher_reports_cipher_version() {
    let dir = tempfile::tempdir().unwrap();
    let version: String = open(&dir, &key())
        .conn()
        .unwrap()
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    assert!(!version.is_empty());
}

#[test]
fn fts5_trigram_is_available() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, &key());
    let conn = db.conn().unwrap();
    conn.execute_batch(
        "CREATE VIRTUAL TABLE temp.t USING fts5 (x, tokenize = 'trigram');
         INSERT INTO temp.t (x) VALUES ('hello world');",
    )
    .unwrap();
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM temp.t WHERE t MATCH '\"lo wo\"'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
}

#[test]
fn session_pragmas_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(&dir, &key());
    let conn = db.conn().unwrap();
    let read = |name: &str| -> i64 { conn.pragma_query_value(None, name, |r| r.get(0)).unwrap() };
    assert_eq!(read("secure_delete"), 1);
    assert_eq!(read("temp_store"), 2);
    assert_eq!(read("foreign_keys"), 1);
}

const SYNTHETIC: [Migration; 2] = [
    Migration {
        version: SCHEMA_VERSION + 1,
        label: "add-column",
        sql: "ALTER TABLE messages ADD COLUMN flags INTEGER NOT NULL DEFAULT 0;",
    },
    Migration {
        version: SCHEMA_VERSION + 2,
        label: "extras",
        sql: "CREATE TABLE extras (account_id BLOB NOT NULL, x TEXT);",
    },
];

fn extended() -> Vec<Migration> {
    MIGRATIONS.iter().chain(SYNTHETIC.iter()).copied().collect()
}

#[test]
fn forward_migration_keeps_rows() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    let (account, p) = (AccountId::new(), peer());
    let db = open_with(&dir, &k, &MIGRATIONS[..1]).unwrap();
    insert_raw(&db, &account, &p, 1);
    insert_raw(&db, &account, &p, 2);
    drop(db);

    let db = open_with(&dir, &k, &extended()).unwrap();
    assert_eq!(user_version(&db), SCHEMA_VERSION + 2);
    let (n, flags): (i64, i64) = db
        .conn()
        .unwrap()
        .query_row("SELECT count(*), sum(flags) FROM messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((n, flags), (2, 0));
    db.conn()
        .unwrap()
        .execute_batch("SELECT count(*) FROM extras;")
        .unwrap();
}

#[test]
fn theme_migration_upgrades_v4_database() {
    use crate::identity::{
        IdentityService, NodeIdTicketProvider, SettingsStore, SqlCipherSettingsStore, Theme,
    };
    let (dir, keychain, account, k) = installed();
    let db = open_with(&dir, &k, &MIGRATIONS[..4]).unwrap();
    assert_eq!(user_version(&db), 4);
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO account_settings (account_id, display_name) VALUES (?1, 'Ada')",
            rusqlite::params![account.as_uuid().as_bytes().as_slice()],
        )
        .unwrap();
    drop(db);

    let db = open(&dir, &k);
    assert_eq!(user_version(&db), 5);
    let theme: Option<String> = db
        .conn()
        .unwrap()
        .query_row("SELECT theme FROM account_settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(theme, None);
    drop(db);

    let storage = Arc::new(Storage::new(dir.path(), keychain.clone()));
    let store = SqlCipherSettingsStore::new(storage.clone());
    assert_eq!(store.get_theme(&account).unwrap(), None);
    let service = IdentityService::new(
        keychain,
        SqlCipherSettingsStore::new(storage),
        NodeIdTicketProvider,
        dir.path(),
    );
    assert_eq!(service.theme().unwrap(), Theme::Midnight);
}

#[test]
fn failing_migration_rolls_back_and_keeps_version() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    open_with(&dir, &k, MIGRATIONS).unwrap();
    let failing = Migration {
        version: SCHEMA_VERSION + 1,
        label: "broken",
        sql: "CREATE TABLE t(x); INSERT INTO missing VALUES (1);",
    };
    let list: Vec<Migration> = MIGRATIONS.iter().copied().chain([failing]).collect();
    let err = open_with(&dir, &k, &list).err().unwrap();
    assert_eq!(
        err,
        StorageError::MigrationFailed {
            version: SCHEMA_VERSION + 1
        }
    );

    let db = open_with(&dir, &k, MIGRATIONS).unwrap();
    assert_eq!(user_version(&db), SCHEMA_VERSION);
    let tables: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 't'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

#[test]
fn rerun_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    drop(open(&dir, &k));
    let db = open(&dir, &k);
    assert_eq!(user_version(&db), SCHEMA_VERSION);
    let mut conn = db.conn().unwrap();
    assert_eq!(migrate::run(&mut conn, MIGRATIONS), Ok(SCHEMA_VERSION));
}

#[test]
fn migrations_are_contiguous_and_match_schema_version() {
    assert!(migrate::is_contiguous(MIGRATIONS));
    assert_eq!(MIGRATIONS.last().unwrap().version, SCHEMA_VERSION);
    assert!(!migrate::is_contiguous(&[SYNTHETIC[1]]));
}

/// The schema text exactly as DD-04 shipped it, with `user_version = 1`.
pub(super) const DD04_SCHEMA: &str = "
CREATE TABLE messages (
  seq            INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id     BLOB    NOT NULL CHECK (length(account_id) = 16),
  peer           TEXT    NOT NULL,
  message_id     BLOB    NOT NULL CHECK (length(message_id) = 16),
  direction      TEXT    NOT NULL CHECK (direction IN ('in','out')),
  sent_at_ms     INTEGER NOT NULL,
  received_at_ms INTEGER,
  body           TEXT    NOT NULL,
  status         TEXT    NOT NULL,
  UNIQUE (account_id, peer, message_id)
);
CREATE INDEX messages_by_peer_time ON messages (account_id, peer, sent_at_ms, seq);
CREATE INDEX messages_outbox       ON messages (account_id, status, seq) WHERE direction = 'out';
CREATE VIRTUAL TABLE messages_fts USING fts5 (body, content = 'messages', content_rowid = 'seq', tokenize = 'trigram');
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts (rowid, body) VALUES (new.seq, new.body); END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.seq, old.body); END;
";

#[test]
fn migration_one_is_byte_identical_to_dd04() {
    assert_eq!(MIGRATIONS[0].sql, DD04_SCHEMA);
}

#[test]
fn dd04_database_opens_unchanged() {
    let (dir, keychain, account, k) = installed();
    let p = iroh::SecretKey::generate().public();
    {
        let conn = rusqlite::Connection::open(db_path(&dir)).unwrap();
        let hex: String = k.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        conn.execute_batch(&["PRAGMA key = \"x'", &hex, "'\";"].concat())
            .unwrap();
        conn.execute_batch(DD04_SCHEMA).unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
        for n in 0..5 {
            conn.execute(
                "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, NULL, ?5, 'delivered')",
                rusqlite::params![
                    account.as_uuid().as_bytes().as_slice(),
                    p.to_string(),
                    uuid::Uuid::new_v4().as_bytes().as_slice(),
                    n,
                    format!("old message {n}"),
                ],
            )
            .unwrap();
        }
    }
    let storage = Storage::new(dir.path(), keychain);
    let store = SqlCipherMessageStore::new(storage.database(&account).unwrap());
    assert_eq!(storage.status(), StorageStatus::Ready);
    let rows = store.page(&account, &p, None, 10).unwrap();
    assert_eq!(rows.len(), 5);
    assert!(rows.iter().all(|r| r.status == MessageStatus::Delivered));
    assert_eq!(
        store
            .search(&account, None, "old message 3", 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn failure_is_latched_until_retry() {
    let (dir, keychain, account, k) = installed();
    let counting = Arc::new(Counting {
        inner: FakeKeychain::new(),
        loads: AtomicUsize::new(0),
    });
    counting.inner.store_db_key(&account, &k).unwrap();
    drop(keychain);
    let storage = Storage::new(dir.path(), counting.clone());
    assert_eq!(storage.status(), StorageStatus::NotOpened);
    counting.inner.replace_db_key_with_random(&account);
    fs::write(db_path(&dir), rand::random::<[u8; 64]>().repeat(64)).unwrap();

    let first = storage.database(&account).err().unwrap();
    assert_eq!(first, StorageError::WrongKeyOrCorrupt);
    assert_eq!(storage.database(&account).err().unwrap(), first);
    assert_eq!(counting.loads.load(Ordering::SeqCst), 1);
    assert_eq!(storage.status(), StorageStatus::Failed(first));

    storage.retry();
    assert_eq!(storage.status(), StorageStatus::NotOpened);
    assert!(storage.database(&account).is_err());
    assert_eq!(counting.loads.load(Ordering::SeqCst), 2);
}

#[test]
fn retry_never_modifies_the_file() {
    let (dir, keychain, account, k) = installed();
    let storage = Storage::new(dir.path(), keychain.clone());
    storage.database_creating(&account).unwrap();
    drop(storage);
    keychain.replace_db_key_with_random(&account);
    let before = fs::read(db_path(&dir)).unwrap();
    let storage = Storage::new(dir.path(), keychain.clone());
    for _ in 0..3 {
        assert!(storage.database(&account).is_err());
        storage.retry();
    }
    assert_eq!(fs::read(db_path(&dir)).unwrap(), before);
    keychain.store_db_key(&account, &k).unwrap();
    assert!(storage.database(&account).is_ok());
}

#[test]
fn other_account_is_refused() {
    let (dir, keychain, account, _) = installed();
    let storage = Storage::new(dir.path(), keychain);
    storage.database_creating(&account).unwrap();
    assert_eq!(
        storage.database(&AccountId::new()).err(),
        Some(StorageError::OtherAccount)
    );
    assert_eq!(storage.status(), StorageStatus::Ready);
}

#[test]
fn missing_db_with_identity_is_reported_not_recreated() {
    let (dir, keychain, account, _) = installed();
    let storage = Storage::new(dir.path(), keychain);
    assert_eq!(
        storage.database(&account).err(),
        Some(StorageError::Missing)
    );
    assert!(!db_path(&dir).exists());
    assert_eq!(
        storage.status(),
        StorageStatus::Failed(StorageError::Missing)
    );
}

#[test]
fn create_allowed_without_marker() {
    let dir = tempfile::tempdir().unwrap();
    let (keychain, account) = (Arc::new(FakeKeychain::new()), AccountId::new());
    keychain.store_db_key(&account, &key()).unwrap();
    Storage::new(dir.path(), keychain)
        .database(&account)
        .unwrap();
    assert!(db_path(&dir).exists());
}

#[test]
fn create_allowed_with_legacy_files() {
    for legacy in ["settings.json", "contacts.json", "known_peers.json"] {
        let (dir, keychain, account, _) = installed();
        fs::write(dir.path().join(legacy), r#"{"version":1,"accounts":{}}"#).unwrap();
        Storage::new(dir.path(), keychain)
            .database(&account)
            .unwrap();
        assert!(db_path(&dir).exists(), "{legacy}");
        assert!(!dir.path().join(legacy).exists(), "{legacy} imported");
    }
}

#[test]
fn missing_key_is_a_typed_error_before_any_file_access() {
    let (dir, keychain, account, _) = installed();
    keychain.delete_db_key(&account).unwrap();
    let storage = Storage::new(dir.path(), keychain);
    assert_eq!(
        storage.database(&account).err(),
        Some(StorageError::KeyMissing)
    );
    assert!(!db_path(&dir).exists());
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn db_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        open(&dir, &key());
        let mode = fs::metadata(db_path(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn loose_db_permissions_are_tightened() {
        let dir = tempfile::tempdir().unwrap();
        let k = key();
        drop(open(&dir, &k));
        fs::set_permissions(db_path(&dir), fs::Permissions::from_mode(0o644)).unwrap();
        drop(open(&dir, &k));
        let mode = fs::metadata(db_path(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn symlinked_db_is_refused_and_target_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.bin");
        fs::write(&target, b"untouched").unwrap();
        std::os::unix::fs::symlink(&target, db_path(&dir)).unwrap();
        assert_eq!(
            open_with(&dir, &key(), MIGRATIONS).err(),
            Some(StorageError::Io)
        );
        assert_eq!(fs::read(&target).unwrap(), b"untouched");
    }

    #[test]
    fn non_regular_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(db_path(&dir)).unwrap();
        assert_eq!(
            open_with(&dir, &key(), MIGRATIONS).err(),
            Some(StorageError::Io)
        );
    }
}

#[test]
fn storage_errors_never_embed_details() {
    let dir = tempfile::tempdir().unwrap();
    let path_text = dir.path().to_string_lossy().into_owned();
    let k = key();
    let hex: String = k.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    drop(open(&dir, &k));
    let real = open_with(&dir, &key(), MIGRATIONS).err().unwrap();
    for e in [
        real,
        StorageError::NotProvisioned,
        StorageError::KeyMissing,
        StorageError::Missing,
        StorageError::KeychainDenied,
        StorageError::KeychainUnavailable,
        StorageError::NotEncrypted,
        StorageError::WrongKeyOrCorrupt,
        StorageError::Corrupt,
        StorageError::NewerSchema,
        StorageError::MigrationFailed { version: 2 },
        StorageError::OtherAccount,
        StorageError::Io,
    ] {
        for shown in [e.to_string(), format!("{e:?}")] {
            let lower = shown.to_lowercase();
            assert!(!shown.contains(&path_text) && !shown.contains(&hex));
            assert!(!shown.contains(DB_FILE) && !shown.contains('/'));
            assert!(!lower.contains("sqlite") && !lower.contains("sqlcipher"));
        }
    }
}

/// Records the open-time cost (integrity checks included) on a large database;
/// the measurement and decision are in the module docs.
#[test]
#[ignore = "benchmark-style; run with --ignored --nocapture"]
fn open_time_with_100k_messages() {
    let dir = tempfile::tempdir().unwrap();
    let k = key();
    {
        let db = open(&dir, &k);
        let mut conn = db.conn().unwrap();
        let tx = conn.transaction().unwrap();
        let (account, p) = (AccountId::new(), peer());
        {
            let mut stmt = tx
                .prepare("INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, NULL, 'a typical chat message body', 'delivered')")
                .unwrap();
            for n in 0..100_000i64 {
                stmt.execute(rusqlite::params![
                    account.as_uuid().as_bytes().as_slice(),
                    p,
                    uuid::Uuid::new_v4().as_bytes().as_slice(),
                    n
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let started = std::time::Instant::now();
    drop(open(&dir, &k));
    println!("open with 100k messages: {:?}", started.elapsed());
}
