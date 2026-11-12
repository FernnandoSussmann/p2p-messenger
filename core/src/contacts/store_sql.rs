//! SQLCipher-backed [`ContactStore`]: the `contacts` table of the encrypted
//! `messenger.db`. All SQL is constant text scoped by `account_id`; every
//! value goes through a bound parameter.
//!
//! Locking: every method takes `storage.database(account)` once and locks
//! `db.conn()` once. Read-modify-write methods run their own IMMEDIATE
//! transaction on that guard through the `*_tx` helpers and never call another
//! trait method while holding it (the mutex is not reentrant).

use std::collections::BTreeMap;
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

use super::model::{apply_observation, Contact, NameUpdate, Nickname, TrustState};
use super::store::{BatchReport, ContactStore, ContactStoreError, MAX_CONTACTS_PER_ACCOUNT};
use crate::identity::{validate_display_name, AccountId, NodeId};
use crate::storage::{Database, Storage};

pub(crate) const MIGRATION_CONTACTS: &str = "
CREATE TABLE contacts (
  account_id   BLOB    NOT NULL CHECK (length(account_id) = 16),
  node_id      TEXT    NOT NULL,
  nickname     TEXT    NOT NULL,
  self_name    TEXT,
  trust        TEXT    NOT NULL CHECK (trust IN ('unverified', 'verified')),
  added_at_ms  INTEGER NOT NULL CHECK (added_at_ms >= 0),
  last_seen_ms INTEGER CHECK (last_seen_ms IS NULL OR last_seen_ms >= 0),
  PRIMARY KEY (account_id, node_id)
) WITHOUT ROWID;
";

macro_rules! cols {
    () => {
        "node_id, nickname, self_name, trust, added_at_ms, last_seen_ms"
    };
}

const SQL_CONTACTS_LIST: &str = concat!("SELECT ", cols!(), " FROM contacts WHERE account_id = ?1");
const SQL_CONTACTS_GET: &str = concat!(
    "SELECT ",
    cols!(),
    " FROM contacts WHERE account_id = ?1 AND node_id = ?2"
);
const SQL_CONTACTS_INSERT_IF_ABSENT: &str = "INSERT INTO contacts (account_id, node_id, nickname, self_name, trust, added_at_ms, last_seen_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (account_id, node_id) DO NOTHING";
const SQL_CONTACTS_COUNT: &str = "SELECT count(*) FROM contacts WHERE account_id = ?1";
/// Rows of any account: user data that provisioning must not discard. The
/// predicate is always true (the column is NOT NULL); it names the namespace
/// column to keep the scoping guard honest.
pub(crate) const SQL_CONTACTS_COUNT_ALL: &str =
    "SELECT count(*) FROM contacts WHERE account_id IS NOT NULL";
const SQL_CONTACTS_SET_NICKNAME: &str = concat!(
    "UPDATE contacts SET nickname = ?3 WHERE account_id = ?1 AND node_id = ?2 RETURNING ",
    cols!()
);
const SQL_CONTACTS_SET_TRUST: &str = concat!(
    "UPDATE contacts SET trust = ?3 WHERE account_id = ?1 AND node_id = ?2 RETURNING ",
    cols!()
);
const SQL_CONTACTS_UPDATE_OBSERVED: &str =
    "UPDATE contacts SET self_name = ?3, last_seen_ms = ?4 WHERE account_id = ?1 AND node_id = ?2";
const SQL_CONTACTS_REMOVE: &str = "DELETE FROM contacts WHERE account_id = ?1 AND node_id = ?2";

type Raw = (String, String, Option<String>, String, i64, Option<i64>);

pub struct SqlCipherContactStore {
    storage: Arc<Storage>,
}

impl SqlCipherContactStore {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    fn db(&self, account: &AccountId) -> Result<Arc<Database>, ContactStoreError> {
        self.storage
            .database(account)
            .map_err(|_| ContactStoreError::Unavailable)
    }
}

fn unavailable<E>(_: E) -> ContactStoreError {
    ContactStoreError::Unavailable
}

fn account_bytes(account: &AccountId) -> [u8; 16] {
    *account.as_uuid().as_bytes()
}

fn read_raw(r: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    ))
}

fn trust_text(t: TrustState) -> &'static str {
    match t {
        TrustState::Unverified => "unverified",
        TrustState::Verified => "verified",
    }
}

/// Validates exactly like the legacy reader does; any failure is `Corrupt`.
fn decode(raw: Raw) -> Result<Contact, ContactStoreError> {
    let (node_id, nickname, self_name, trust, added_at_ms, last_seen_ms) = raw;
    let id: NodeId = node_id.parse().map_err(|_| corrupt_of())?;
    if id.to_string() != node_id || added_at_ms < 0 || last_seen_ms.is_some_and(|t| t < 0) {
        return Err(corrupt_of());
    }
    let nick = Nickname::parse(&nickname).map_err(|_| corrupt_of())?;
    if nick.as_str() != nickname {
        return Err(corrupt_of());
    }
    let self_name = match self_name {
        Some(raw) => {
            let name = validate_display_name(&raw).map_err(|_| corrupt_of())?;
            if name.as_str() != raw {
                return Err(corrupt_of());
            }
            Some(name)
        }
        None => None,
    };
    let trust = match trust.as_str() {
        "unverified" => TrustState::Unverified,
        "verified" => TrustState::Verified,
        _ => return Err(corrupt_of()),
    };
    Ok(Contact {
        node_id: id,
        nickname: nick,
        self_name,
        trust,
        added_at_ms,
        last_seen_ms,
    })
}

fn corrupt_of() -> ContactStoreError {
    ContactStoreError::Corrupt
}

fn immediate(conn: &mut Connection) -> Result<Transaction<'_>, ContactStoreError> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(unavailable)
}

pub(crate) fn get_tx(
    tx: &Transaction,
    account: &AccountId,
    id: &NodeId,
) -> Result<Option<Contact>, ContactStoreError> {
    tx.query_row(
        SQL_CONTACTS_GET,
        params![account_bytes(account).as_slice(), id.to_string()],
        read_raw,
    )
    .optional()
    .map_err(unavailable)?
    .map(decode)
    .transpose()
}

pub(crate) fn count_tx(tx: &Transaction, account: &AccountId) -> Result<usize, ContactStoreError> {
    tx.query_row(
        SQL_CONTACTS_COUNT,
        params![account_bytes(account).as_slice()],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
    .map_err(unavailable)
}

/// Inserts unless the (account, NodeId) exists; returns whether a row was inserted.
pub(crate) fn insert_if_absent_tx(
    tx: &Transaction,
    account: &AccountId,
    c: &Contact,
) -> Result<bool, ContactStoreError> {
    tx.execute(
        SQL_CONTACTS_INSERT_IF_ABSENT,
        params![
            account_bytes(account).as_slice(),
            c.node_id.to_string(),
            c.nickname.as_str(),
            c.self_name.as_ref().map(|n| n.as_str()),
            trust_text(c.trust),
            c.added_at_ms,
            c.last_seen_ms
        ],
    )
    .map(|n| n > 0)
    .map_err(unavailable)
}

fn update_returning(
    conn: &Connection,
    sql: &str,
    account: &AccountId,
    id: &NodeId,
    value: &str,
) -> Result<Contact, ContactStoreError> {
    conn.query_row(
        sql,
        params![account_bytes(account).as_slice(), id.to_string(), value],
        read_raw,
    )
    .optional()
    .map_err(unavailable)?
    .map(decode)
    .transpose()?
    .ok_or(ContactStoreError::NotFound)
}

impl ContactStore for SqlCipherContactStore {
    fn list(&self, account: &AccountId) -> Result<Vec<Contact>, ContactStoreError> {
        let db = self.db(account)?;
        let conn = db.conn().map_err(unavailable)?;
        let mut stmt = conn
            .prepare_cached(SQL_CONTACTS_LIST)
            .map_err(unavailable)?;
        let rows = stmt
            .query_map(params![account_bytes(account).as_slice()], read_raw)
            .map_err(unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(unavailable)?;
        rows.into_iter().map(decode).collect()
    }

    fn get(&self, account: &AccountId, id: &NodeId) -> Result<Option<Contact>, ContactStoreError> {
        let db = self.db(account)?;
        let conn = db.conn().map_err(unavailable)?;
        conn.query_row(
            SQL_CONTACTS_GET,
            params![account_bytes(account).as_slice(), id.to_string()],
            read_raw,
        )
        .optional()
        .map_err(unavailable)?
        .map(decode)
        .transpose()
    }

    fn insert(&self, account: &AccountId, contact: Contact) -> Result<(), ContactStoreError> {
        let db = self.db(account)?;
        let mut conn = db.conn().map_err(unavailable)?;
        let tx = immediate(&mut conn)?;
        if get_tx(&tx, account, &contact.node_id)?.is_some() {
            return Err(ContactStoreError::AlreadyExists);
        }
        if count_tx(&tx, account)? >= MAX_CONTACTS_PER_ACCOUNT {
            return Err(ContactStoreError::Full);
        }
        if !insert_if_absent_tx(&tx, account, &contact)? {
            return Err(ContactStoreError::AlreadyExists);
        }
        tx.commit().map_err(unavailable)
    }

    fn insert_missing(
        &self,
        entries: Vec<(AccountId, Contact)>,
    ) -> Result<BatchReport, ContactStoreError> {
        let Some((first, _)) = entries.first() else {
            return Ok(BatchReport::default());
        };
        // One database per process: every account must resolve to it.
        let db = self.db(first)?;
        let mut touched: BTreeMap<uuid::Uuid, AccountId> = BTreeMap::new();
        for (account, _) in &entries {
            if !Arc::ptr_eq(&db, &self.db(account)?) {
                return Err(ContactStoreError::Unavailable);
            }
            touched.insert(account.as_uuid(), *account);
        }
        let mut conn = db.conn().map_err(unavailable)?;
        let tx = immediate(&mut conn)?;
        let mut report = BatchReport::default();
        for (account, contact) in &entries {
            if insert_if_absent_tx(&tx, account, contact)? {
                report.inserted += 1;
            } else {
                report.already_present += 1;
            }
        }
        for account in touched.values() {
            if count_tx(&tx, account)? > MAX_CONTACTS_PER_ACCOUNT {
                return Err(ContactStoreError::Full);
            }
        }
        tx.commit().map_err(unavailable)?;
        Ok(report)
    }

    fn set_nickname(
        &self,
        account: &AccountId,
        id: &NodeId,
        nickname: &Nickname,
    ) -> Result<Contact, ContactStoreError> {
        let db = self.db(account)?;
        let conn = db.conn().map_err(unavailable)?;
        update_returning(
            &conn,
            SQL_CONTACTS_SET_NICKNAME,
            account,
            id,
            nickname.as_str(),
        )
    }

    fn set_trust(
        &self,
        account: &AccountId,
        id: &NodeId,
        trust: TrustState,
    ) -> Result<Contact, ContactStoreError> {
        let db = self.db(account)?;
        let conn = db.conn().map_err(unavailable)?;
        update_returning(
            &conn,
            SQL_CONTACTS_SET_TRUST,
            account,
            id,
            trust_text(trust),
        )
    }

    fn observe(
        &self,
        account: &AccountId,
        id: &NodeId,
        name: NameUpdate<'_>,
        seen_at_ms: Option<i64>,
    ) -> Result<(), ContactStoreError> {
        let db = self.db(account)?;
        let mut conn = db.conn().map_err(unavailable)?;
        let tx = immediate(&mut conn)?;
        let Some(mut contact) = get_tx(&tx, account, id)? else {
            return Ok(());
        };
        if !apply_observation(&mut contact, name, seen_at_ms) {
            return Ok(());
        }
        tx.execute(
            SQL_CONTACTS_UPDATE_OBSERVED,
            params![
                account_bytes(account).as_slice(),
                id.to_string(),
                contact.self_name.as_ref().map(|n| n.as_str()),
                contact.last_seen_ms
            ],
        )
        .map_err(unavailable)?;
        tx.commit().map_err(unavailable)
    }

    fn remove(&self, account: &AccountId, id: &NodeId) -> Result<bool, ContactStoreError> {
        let db = self.db(account)?;
        let conn = db.conn().map_err(unavailable)?;
        conn.execute(
            SQL_CONTACTS_REMOVE,
            params![account_bytes(account).as_slice(), id.to_string()],
        )
        .map(|n| n > 0)
        .map_err(unavailable)
    }
}

#[cfg(test)]
mod tests;
