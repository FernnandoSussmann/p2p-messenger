//! The `storage` slice: the single encrypted database `messenger.db`.
//!
//! Invariants:
//! - one SQLCipher database per install;
//! - the key comes from the keychain only and is held only while the
//!   connection lives (it is dropped right after `PRAGMA key`);
//! - failures are typed ([`StorageError`], payload-free) and never reset,
//!   truncate or recreate the file; a failed open is latched until `retry`;
//! - migrations are forward-only and transactional, one per version, each
//!   together with its `user_version` bump;
//! - every row and every query is scoped by `account_id`. Documented
//!   exceptions: schema bookkeeping (`PRAGMA user_version`); the FTS5 index
//!   `messages_fts`, an external-content table of `rowid` + `body` reached only
//!   through a join on `messages.seq = messages_fts.rowid` filtered by
//!   `messages.account_id` (`SQL_SEARCH_FTS`), whose maintenance statement
//!   `SQL_FTS_OPTIMIZE` is allowlisted; and `legacy_imports`, which 05-02 adds.
//!
//! Feature slices own their table SQL as constant text; storage only orders
//! it (see `schema`). Every value goes through a bound parameter.
//!
//! Integrity check cost: both `PRAGMA cipher_integrity_check` and `PRAGMA
//! quick_check` run on every open of a file that already had data. Measured
//! with `open_time_with_100k_messages` (release build, dev machine): a full open
//! of a 100 000-message database took ~0.4 s including both checks, under the
//! ~1 s criterion, so the full check stays at startup (and on `retry`).

mod database;
mod error;
mod handle;
mod migrate;
mod schema;
#[cfg(test)]
mod tests;

pub use database::Database;
pub use error::StorageError;
pub use handle::{KeySource, Storage, StorageStatus};
pub use schema::SCHEMA_VERSION;

#[cfg(test)]
pub(crate) use database::Create;
#[cfg(test)]
pub(crate) use schema::MIGRATIONS;

/// The single encrypted database file in the (profile) data dir.
pub const DB_FILE: &str = "messenger.db";
