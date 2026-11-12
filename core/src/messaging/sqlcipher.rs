//! SQLCipher-backed [`MessageStore`].
//!
//! All SQL is constant text; every value goes through a bound parameter.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use protocol::{ChatMessage, MessageId};
use rusqlite::{params, Connection, ErrorCode, Row};
use zeroize::Zeroizing;

use super::model::{
    validate_body, HistoryCursor, Insert, MessageDirection, MessageStatus, MessageStoreError,
    StoredMessage,
};
use super::store::{clean_query, MessageStore, MAX_PAGE, MAX_SEARCH_RESULTS};
use crate::contacts::{HistoryEraseError, HistoryEraser};
use crate::identity::{
    AccountId, DbKey, IdentityService, KeychainStore, NodeId, SettingsStore, TicketProvider,
};

pub const DB_FILE: &str = "messenger.db";

const SCHEMA_VERSION: i64 = 1; // keep in step with the user_version statement in `migrate`

const SCHEMA_V1: &str = "
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

macro_rules! cols {
    () => {
        "seq, peer, message_id, direction, sent_at_ms, received_at_ms, body, status"
    };
}

macro_rules! cols_m {
    () => {
        "m.seq, m.peer, m.message_id, m.direction, m.sent_at_ms, m.received_at_ms, m.body, m.status"
    };
}

const SQL_INSERT_OUT: &str = "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, NULL, ?5, ?6)";
const SQL_INSERT_IN: &str = "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'in', ?4, ?5, ?6, 'received') ON CONFLICT (account_id, peer, message_id) DO NOTHING";
const SQL_SET_STATUS: &str = "UPDATE messages SET status = ?4 WHERE account_id = ?1 AND peer = ?2 AND message_id = ?3 AND direction = 'out' AND status <> 'delivered'";
const SQL_GET: &str = concat!(
    "SELECT ",
    cols!(),
    " FROM messages WHERE account_id = ?1 AND peer = ?2 AND message_id = ?3"
);
const SQL_PAGE: &str = concat!(
    "SELECT ",
    cols!(),
    " FROM messages WHERE account_id = ?1 AND peer = ?2 AND (?3 IS NULL OR (sent_at_ms, seq) < (?3, ?4)) ORDER BY sent_at_ms DESC, seq DESC LIMIT ?5"
);
const SQL_SEARCH_FTS: &str = concat!(
    "SELECT ",
    cols_m!(),
    " FROM messages_fts f JOIN messages m ON m.seq = f.rowid WHERE messages_fts MATCH ?2 AND m.account_id = ?1 AND (?3 IS NULL OR m.peer = ?3) ORDER BY m.sent_at_ms DESC, m.seq DESC LIMIT ?4"
);
const SQL_SEARCH_LIKE: &str = concat!(
    "SELECT ",
    cols_m!(),
    " FROM messages m WHERE m.account_id = ?1 AND m.body LIKE ?2 ESCAPE '\\' AND (?3 IS NULL OR m.peer = ?3) ORDER BY m.sent_at_ms DESC, m.seq DESC LIMIT ?4"
);
const SQL_UNDELIVERED: &str = concat!(
    "SELECT ",
    cols!(),
    " FROM messages WHERE account_id = ?1 AND direction = 'out' AND status IN ('sending', 'pending') ORDER BY seq ASC LIMIT ?2"
);
const SQL_ERASE: &str = "DELETE FROM messages WHERE account_id = ?1 AND peer = ?2";
const SQL_FTS_OPTIMIZE: &str = "INSERT INTO messages_fts (messages_fts) VALUES ('optimize')";

/// Opens (creating if absent) the encrypted store in the identity's data dir,
/// keyed with the keychain DB key. Never recreates or deletes an existing file.
pub fn open_message_store<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    identity: &IdentityService<K, S, T>,
) -> Result<(AccountId, SqlCipherMessageStore), MessageStoreError> {
    let (account, key) = identity
        .load_db_key()
        .map_err(|_| MessageStoreError::Unavailable)?;
    let store = SqlCipherMessageStore::open_with_key(&identity.data_dir().join(DB_FILE), &key)?;
    Ok((account, store))
}

pub struct SqlCipherMessageStore {
    conn: Mutex<Connection>,
}

impl SqlCipherMessageStore {
    pub(crate) fn open_with_key(path: &Path, key: &DbKey) -> Result<Self, MessageStoreError> {
        create_private_file(path)?;
        let mut conn = Connection::open(path).map_err(|_| MessageStoreError::Unavailable)?;

        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut sql = Zeroizing::new(String::with_capacity(16 + 64 + 3));
        sql.push_str("PRAGMA key = \"x'");
        for b in key.as_bytes() {
            sql.push(HEX[usize::from(b >> 4)] as char);
            sql.push(HEX[usize::from(b & 0x0f)] as char);
        }
        sql.push_str("'\";");
        conn.execute_batch(&sql)
            .map_err(|_| MessageStoreError::Unavailable)?;
        drop(sql);

        let version: Option<String> = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .ok();
        if version.is_none_or(|v| v.is_empty()) {
            return Err(MessageStoreError::NotEncrypted);
        }
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| MessageStoreError::WrongKeyOrCorrupt)?;
        conn.execute_batch(
            "PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON; PRAGMA foreign_keys = ON;",
        )
        .map_err(|_| MessageStoreError::Unavailable)?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>, MessageStoreError> {
        self.conn.lock().map_err(|_| MessageStoreError::Unavailable)
    }
}

/// On Unix a new file is created `0600`; an existing file is left untouched.
fn create_private_file(path: &Path) -> Result<(), MessageStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(MessageStoreError::Unavailable),
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<(), MessageStoreError> {
    let unavailable = |_| MessageStoreError::Unavailable;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(unavailable)?;
    match version {
        0 => {
            let tx = conn.transaction().map_err(unavailable)?;
            tx.execute_batch(SCHEMA_V1).map_err(unavailable)?;
            tx.execute_batch("PRAGMA user_version = 1;")
                .map_err(unavailable)?;
            tx.commit().map_err(unavailable)
        }
        SCHEMA_VERSION => Ok(()),
        _ => Err(MessageStoreError::NewerSchema),
    }
}

type Raw = (
    i64,
    String,
    Vec<u8>,
    String,
    i64,
    Option<i64>,
    String,
    String,
);

fn read_raw(r: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}

/// Any unreadable column is corruption, reported as `Unavailable`.
fn into_stored(raw: Raw) -> Result<StoredMessage, MessageStoreError> {
    let (seq, peer, id, direction, sent_at_ms, received_at_ms, body, status) = raw;
    let bad = MessageStoreError::Unavailable;
    Ok(StoredMessage {
        seq,
        peer: peer.parse::<NodeId>().map_err(|_| bad)?,
        id: MessageId::from_slice(&id).map_err(|_| bad)?,
        direction: MessageDirection::parse(&direction).ok_or(bad)?,
        sent_at_ms,
        received_at_ms,
        body,
        status: MessageStatus::parse(&status).ok_or(bad)?,
    })
}

fn query_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<StoredMessage>, MessageStoreError> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|_| MessageStoreError::Unavailable)?;
    let raws = stmt
        .query_map(params, read_raw)
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|_| MessageStoreError::Unavailable)?;
    raws.into_iter().map(into_stored).collect()
}

fn sql_limit(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

fn fts_phrase(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('"');
    for c in query.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation)
}

impl MessageStore for SqlCipherMessageStore {
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
        let conn = self.conn()?;
        conn.execute(
            SQL_INSERT_OUT,
            params![
                account.as_uuid().as_bytes().as_slice(),
                peer.to_string(),
                msg.id.as_bytes().as_slice(),
                msg.sent_at_ms,
                msg.body,
                status.as_str(),
            ],
        )
        .map_err(|e| {
            if is_unique_violation(&e) {
                MessageStoreError::AlreadyExists
            } else {
                MessageStoreError::Unavailable
            }
        })?;
        Ok(StoredMessage {
            seq: conn.last_insert_rowid(),
            peer: *peer,
            id: msg.id,
            direction: MessageDirection::Outgoing,
            sent_at_ms: msg.sent_at_ms,
            received_at_ms: None,
            body: msg.body.clone(),
            status,
        })
    }

    fn insert_incoming(
        &self,
        account: &AccountId,
        peer: &NodeId,
        msg: &ChatMessage,
        received_at_ms: i64,
    ) -> Result<Insert, MessageStoreError> {
        validate_body(&msg.body)?;
        let conn = self.conn()?;
        let changed = conn
            .execute(
                SQL_INSERT_IN,
                params![
                    account.as_uuid().as_bytes().as_slice(),
                    peer.to_string(),
                    msg.id.as_bytes().as_slice(),
                    msg.sent_at_ms,
                    received_at_ms,
                    msg.body,
                ],
            )
            .map_err(|_| MessageStoreError::Unavailable)?;
        if changed == 0 {
            return Ok(Insert::Duplicate);
        }
        Ok(Insert::Inserted(StoredMessage {
            seq: conn.last_insert_rowid(),
            peer: *peer,
            id: msg.id,
            direction: MessageDirection::Incoming,
            sent_at_ms: msg.sent_at_ms,
            received_at_ms: Some(received_at_ms),
            body: msg.body.clone(),
            status: MessageStatus::Received,
        }))
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
        let changed = self
            .conn()?
            .execute(
                SQL_SET_STATUS,
                params![
                    account.as_uuid().as_bytes().as_slice(),
                    peer.to_string(),
                    id.as_bytes().as_slice(),
                    status.as_str(),
                ],
            )
            .map_err(|_| MessageStoreError::Unavailable)?;
        Ok(changed > 0)
    }

    fn get(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &MessageId,
    ) -> Result<Option<StoredMessage>, MessageStoreError> {
        let rows = query_rows(
            &*self.conn()?,
            SQL_GET,
            params![
                account.as_uuid().as_bytes().as_slice(),
                peer.to_string(),
                id.as_bytes().as_slice(),
            ],
        )?;
        Ok(rows.into_iter().next())
    }

    fn page(
        &self,
        account: &AccountId,
        peer: &NodeId,
        before: Option<HistoryCursor>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        let limit = sql_limit(limit.clamp(1, MAX_PAGE));
        let mut rows = query_rows(
            &*self.conn()?,
            SQL_PAGE,
            params![
                account.as_uuid().as_bytes().as_slice(),
                peer.to_string(),
                before.map(|c| c.sent_at_ms),
                before.map(|c| c.seq),
                limit,
            ],
        )?;
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
        let q = clean_query(query)?;
        let limit = sql_limit(limit.clamp(1, MAX_SEARCH_RESULTS));
        let (sql, pattern) = if q.chars().nth(2).is_some() {
            (SQL_SEARCH_FTS, fts_phrase(q))
        } else {
            (SQL_SEARCH_LIKE, like_pattern(q))
        };
        query_rows(
            &*self.conn()?,
            sql,
            params![
                account.as_uuid().as_bytes().as_slice(),
                pattern,
                peer.map(|p| p.to_string()),
                limit,
            ],
        )
    }

    fn undelivered(
        &self,
        account: &AccountId,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MessageStoreError> {
        query_rows(
            &*self.conn()?,
            SQL_UNDELIVERED,
            params![account.as_uuid().as_bytes().as_slice(), sql_limit(limit)],
        )
    }
}

impl HistoryEraser for SqlCipherMessageStore {
    fn erase_peer_history(
        &self,
        account: &AccountId,
        peer: &NodeId,
    ) -> Result<(), HistoryEraseError> {
        let mut conn = self.conn.lock().map_err(|_| HistoryEraseError)?;
        let tx = conn.transaction().map_err(|_| HistoryEraseError)?;
        tx.execute(
            SQL_ERASE,
            params![account.as_uuid().as_bytes().as_slice(), peer.to_string()],
        )
        .map_err(|_| HistoryEraseError)?;
        // The external-content 'delete' only adds tombstones; optimize merges
        // the segments so the erased trigram postings are gone.
        tx.execute(SQL_FTS_OPTIMIZE, [])
            .map_err(|_| HistoryEraseError)?;
        tx.commit().map_err(|_| HistoryEraseError)
    }
}

#[cfg(test)]
mod tests;
