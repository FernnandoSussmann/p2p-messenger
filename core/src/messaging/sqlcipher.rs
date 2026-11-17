//! SQLCipher-backed [`MessageStore`].
//!
//! All SQL is constant text; every value goes through a bound parameter.
//!
//! Erasing a peer deletes the rows (the `messages_ad` trigger drops their FTS
//! rows), merges the trigram postings with `optimize`, then runs `VACUUM`.
//! `secure_delete=ON` (verified at open) zero-fills freed content before a page
//! is encrypted and written, and `VACUUM` leaves the file with no free pages.
//! The rollback journal (default `DELETE` mode) holds only SQLCipher-encrypted
//! pages and is unlinked after commit. This does not cover old file blocks on
//! SSDs or copy-on-write file systems, OS backups and snapshots, or the peer's
//! own copy. It is not a "secure erase".
//!
//! `VACUUM` holds the connection for the whole rewrite, so erasing must run
//! off the async runtime. Cost, measured with the ignored test
//! `vacuum_cost_on_100k_messages`: 95 MB database (100 000 messages
//! of about 200 bytes over 50 peers, one peer cleared), debug build: 3.1 s wall,
//! 85 MB peak-RSS growth (under 2x the file size). Above the 2 s budget, so the
//! UI busy text says "This may take a moment."; `temp_store` stays `MEMORY`.

use std::sync::{Arc, MutexGuard};

use protocol::{ChatMessage, MessageId};
use rusqlite::{params, Connection, ErrorCode, Row};

use super::model::{
    validate_body, HistoryCursor, Insert, MessageDirection, MessageStatus, MessageStoreError,
    StoredMessage,
};
use super::store::{clean_query, MessageStore, MAX_PAGE, MAX_SEARCH_RESULTS};
use crate::contacts::{HistoryEraseError, HistoryEraser};
use crate::identity::{AccountId, NodeId};
use crate::storage::Database;

pub(crate) const MIGRATION_MESSAGES_V1: &str = "
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

pub(crate) const MIGRATION_MESSAGES_DELIVERED_AT: &str = "
ALTER TABLE messages ADD COLUMN delivered_at_ms INTEGER
  CHECK (delivered_at_ms IS NULL OR direction = 'out');
";

macro_rules! cols {
    () => {
        "seq, peer, message_id, direction, sent_at_ms, received_at_ms, body, status, delivered_at_ms"
    };
}

macro_rules! cols_m {
    () => {
        "m.seq, m.peer, m.message_id, m.direction, m.sent_at_ms, m.received_at_ms, m.body, m.status, m.delivered_at_ms"
    };
}

const SQL_INSERT_OUT: &str = "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'out', ?4, NULL, ?5, ?6)";
const SQL_INSERT_IN: &str = "INSERT INTO messages (account_id, peer, message_id, direction, sent_at_ms, received_at_ms, body, status) VALUES (?1, ?2, ?3, 'in', ?4, ?5, ?6, 'received') ON CONFLICT (account_id, peer, message_id) DO NOTHING";
const SQL_MARK_DELIVERED: &str = "UPDATE messages SET status = 'delivered', delivered_at_ms = ?4 WHERE account_id = ?1 AND peer = ?2 AND message_id = ?3 AND direction = 'out' AND status <> 'delivered'";
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

pub struct SqlCipherMessageStore {
    db: Arc<Database>,
}

impl SqlCipherMessageStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Free pages in the file; 0 after every successful erase.
    #[cfg(feature = "test-support")]
    pub fn freelist_count(&self) -> Result<i64, MessageStoreError> {
        self.conn()?
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .map_err(|_| MessageStoreError::Unavailable)
    }

    fn conn(&self) -> Result<MutexGuard<'_, Connection>, MessageStoreError> {
        self.db.conn().map_err(|_| MessageStoreError::Unavailable)
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
    Option<i64>,
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
        r.get(8)?,
    ))
}

/// Any unreadable column is corruption, reported as `Unavailable`.
fn into_stored(raw: Raw) -> Result<StoredMessage, MessageStoreError> {
    let (seq, peer, id, direction, sent_at_ms, received_at_ms, body, status, delivered_at_ms) = raw;
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
        delivered_at_ms,
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
            delivered_at_ms: None,
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
            delivered_at_ms: None,
        }))
    }

    fn set_status(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &MessageId,
        status: MessageStatus,
    ) -> Result<bool, MessageStoreError> {
        if matches!(status, MessageStatus::Received | MessageStatus::Delivered) {
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

    fn mark_delivered(
        &self,
        account: &AccountId,
        peer: &NodeId,
        id: &MessageId,
        delivered_at_ms: i64,
    ) -> Result<bool, MessageStoreError> {
        let changed = self
            .conn()?
            .execute(
                SQL_MARK_DELIVERED,
                params![
                    account.as_uuid().as_bytes().as_slice(),
                    peer.to_string(),
                    id.as_bytes().as_slice(),
                    delivered_at_ms,
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
        {
            let mut conn = self.conn().map_err(|_| HistoryEraseError)?;
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
            tx.commit().map_err(|_| HistoryEraseError)?;
        }
        // The rows are gone even if this fails; the caller may retry (idempotent).
        self.db.reclaim().map_err(|_| HistoryEraseError)
    }
}

#[cfg(test)]
mod tests;
