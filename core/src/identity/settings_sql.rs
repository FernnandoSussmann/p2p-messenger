//! SQLCipher-backed [`SettingsStore`]: the `account_settings` table of the
//! encrypted `messenger.db`. All SQL is constant text keyed by `account_id`;
//! every value goes through a bound parameter.

use std::sync::Arc;

use rusqlite::{params, OptionalExtension, Transaction};

use super::settings::LegacySettings;
use super::{
    validate_display_name, validate_relay_url, AccountId, DisplayName, DisplayNameError,
    SettingsError, SettingsStore,
};
use crate::storage::Storage;

pub(crate) const MIGRATION_SETTINGS: &str = "
CREATE TABLE account_settings (
  account_id       BLOB    PRIMARY KEY CHECK (length(account_id) = 16),
  display_name     TEXT,
  provisioned_at   INTEGER,
  presence_enabled INTEGER CHECK (presence_enabled IN (0, 1)),
  relay_url        TEXT    CHECK (relay_url IS NULL OR length(relay_url) <= 2048)
);
";

const SQL_SETTINGS_SET_NAME: &str = "INSERT INTO account_settings (account_id, display_name) VALUES (?1, ?2) ON CONFLICT (account_id) DO UPDATE SET display_name = excluded.display_name";
const SQL_SETTINGS_SET_PROVISIONED: &str = "INSERT INTO account_settings (account_id, provisioned_at) VALUES (?1, ?2) ON CONFLICT (account_id) DO UPDATE SET provisioned_at = excluded.provisioned_at";
const SQL_SETTINGS_SET_PRESENCE: &str = "INSERT INTO account_settings (account_id, presence_enabled) VALUES (?1, ?2) ON CONFLICT (account_id) DO UPDATE SET presence_enabled = excluded.presence_enabled";
const SQL_SETTINGS_SET_RELAY: &str = "INSERT INTO account_settings (account_id, relay_url) VALUES (?1, ?2) ON CONFLICT (account_id) DO UPDATE SET relay_url = excluded.relay_url";
const SQL_SETTINGS_GET: &str = "SELECT display_name, provisioned_at, presence_enabled, relay_url FROM account_settings WHERE account_id = ?1";
const SQL_SETTINGS_INSERT_IF_ABSENT: &str = "INSERT INTO account_settings (account_id, display_name, provisioned_at, presence_enabled, relay_url) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (account_id) DO NOTHING";
/// Rows of accounts other than `?1`: user data that provisioning must not discard.
pub(crate) const SQL_SETTINGS_COUNT_OTHERS: &str =
    "SELECT count(*) FROM account_settings WHERE account_id <> ?1";

pub struct SqlCipherSettingsStore {
    storage: Arc<Storage>,
}

impl SqlCipherSettingsStore {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }

    fn set<P: rusqlite::ToSql>(
        &self,
        id: &AccountId,
        sql: &str,
        value: P,
    ) -> Result<(), SettingsError> {
        let db = self.storage.database(id)?;
        let conn = db.conn()?;
        conn.execute(sql, params![id.as_uuid().as_bytes().as_slice(), value])
            .map(drop)
            .map_err(|_| SettingsError::Unavailable)
    }

    fn row(&self, id: &AccountId) -> Result<Option<LegacySettings>, SettingsError> {
        let db = self.storage.database(id)?;
        let conn = db.conn()?;
        conn.query_row(
            SQL_SETTINGS_GET,
            params![id.as_uuid().as_bytes().as_slice()],
            |r| {
                Ok(LegacySettings {
                    display_name: r.get(0)?,
                    provisioned_at: r.get(1)?,
                    presence_enabled: r.get(2)?,
                    relay_url: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|_| SettingsError::Unavailable)
    }
}

impl SettingsStore for SqlCipherSettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError> {
        self.set(id, SQL_SETTINGS_SET_NAME, name.as_str())
    }

    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
        match self.row(id)?.and_then(|r| r.display_name) {
            Some(raw) => match validate_display_name(&raw) {
                Ok(name) => Ok(Some(name)),
                // Saved under older, looser rules: treat as unset, never brick startup.
                Err(DisplayNameError::InvisibleCharacter) => Ok(None),
                Err(_) => Err(SettingsError::Unavailable),
            },
            None => Ok(None),
        }
    }

    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError> {
        self.set(id, SQL_SETTINGS_SET_PROVISIONED, unix_ms)
    }

    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
        Ok(self.row(id)?.and_then(|r| r.provisioned_at))
    }

    fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError> {
        Ok(self.row(id)?.and_then(|r| r.presence_enabled))
    }

    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        self.set(id, SQL_SETTINGS_SET_PRESENCE, on)
    }

    /// An invalid stored value is an error, so the node refuses to start
    /// rather than fall back to the default relay mesh.
    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        match self.row(id)?.and_then(|r| r.relay_url) {
            Some(raw) => validate_relay_url(&raw).map(Some),
            None => Ok(None),
        }
    }

    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        let canonical = url.map(validate_relay_url).transpose()?;
        self.set(id, SQL_SETTINGS_SET_RELAY, canonical)
    }

    fn discard_new_account(&self, id: &AccountId) -> Result<(), SettingsError> {
        Ok(self.storage.discard_provisioning(id)?)
    }
}

impl From<crate::storage::StorageError> for SettingsError {
    fn from(_: crate::storage::StorageError) -> Self {
        SettingsError::Unavailable
    }
}

/// Inserts one account's legacy settings unless a row exists. Runs on the
/// import's own transaction; returns whether a row was inserted.
pub(crate) fn insert_settings_if_absent_tx(
    tx: &Transaction,
    account: &AccountId,
    s: &LegacySettings,
) -> Result<bool, rusqlite::Error> {
    tx.execute(
        SQL_SETTINGS_INSERT_IF_ABSENT,
        params![
            account.as_uuid().as_bytes().as_slice(),
            s.display_name,
            s.provisioned_at,
            s.presence_enabled,
            s.relay_url
        ],
    )
    .map(|n| n > 0)
}

#[cfg(test)]
mod tests;
