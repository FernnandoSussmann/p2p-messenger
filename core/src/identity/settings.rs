//! Per-account settings: the [`SettingsStore`] trait, the in-memory test
//! double and the read-only parser of the pre-DD-05 `settings.json`.
//!
//! The durable store is [`SqlCipherSettingsStore`](super::SqlCipherSettingsStore)
//! in the encrypted `messenger.db`. [`MemorySettingsStore`] is the test double.
//! Both hold the display name, provisioning timestamp, LAN presence flag and
//! relay override only, and never any secret.
//!
//! [`read_legacy_settings`] only parses the old plaintext file; the one-shot
//! import into the database lives in `storage::legacy`, which also overwrites
//! and removes the file after the import committed.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use iroh::RelayUrl;
use serde::Deserialize;
use uuid::Uuid;

use super::{validate_display_name, AccountId, DisplayName, DisplayNameError};
use crate::storage::LegacyError;

#[derive(Debug)]
pub enum SettingsError {
    /// The backing store could not be read or written.
    Unavailable,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Unavailable => write!(f, "settings store unavailable"),
        }
    }
}

impl std::error::Error for SettingsError {}

pub trait SettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError>;
    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError>;
    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError>;
    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError>;
    /// `None` means the user never chose, which is ON.
    fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError>;
    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError>;
    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError>;
    /// Validates with [`validate_relay_url`] and stores the canonical form;
    /// `None` clears the override.
    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError>;
    /// Called best-effort when provisioning rolls back, so a store that
    /// created durable state for `id` can drop it. Never deletes anything
    /// that did not belong to the failed attempt.
    fn discard_new_account(&self, _id: &AccountId) -> Result<(), SettingsError> {
        Ok(())
    }
}

const MAX_RELAY_URL_BYTES: usize = 2048;

/// A relay URL is accepted if it parses, is at most 2048 bytes, and is
/// `https` (or `http` for a loopback host only). Returns the canonical string.
pub fn validate_relay_url(raw: &str) -> Result<String, SettingsError> {
    if raw.len() > MAX_RELAY_URL_BYTES {
        return Err(SettingsError::Unavailable);
    }
    let url = RelayUrl::from_str(raw).map_err(|_| SettingsError::Unavailable)?;
    let allowed = match url.scheme() {
        "https" => true,
        "http" => match url.host_str() {
            Some(host) => {
                let host = host.trim_start_matches('[').trim_end_matches(']');
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }
            None => false,
        },
        _ => false,
    };
    if allowed {
        Ok(url.to_string())
    } else {
        Err(SettingsError::Unavailable)
    }
}

impl<T: SettingsStore + ?Sized> SettingsStore for std::sync::Arc<T> {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError> {
        (**self).set_display_name(id, name)
    }
    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
        (**self).get_display_name(id)
    }
    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError> {
        (**self).set_provisioned_at(id, unix_ms)
    }
    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
        (**self).get_provisioned_at(id)
    }
    fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError> {
        (**self).get_presence(id)
    }
    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        (**self).set_presence(id, on)
    }
    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        (**self).get_relay_url(id)
    }
    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        (**self).set_relay_url(id, url)
    }
    fn discard_new_account(&self, id: &AccountId) -> Result<(), SettingsError> {
        (**self).discard_new_account(id)
    }
}

#[derive(Default)]
struct AccountSettings {
    display_name: Option<DisplayName>,
    provisioned_at: Option<i64>,
    presence_enabled: Option<bool>,
    relay_url: Option<String>,
}

#[derive(Default)]
pub struct MemorySettingsStore {
    accounts: Mutex<HashMap<uuid::Uuid, AccountSettings>>,
}

impl MemorySettingsStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_account<T>(
        &self,
        id: &AccountId,
        f: impl FnOnce(&mut AccountSettings) -> T,
    ) -> Result<T, SettingsError> {
        let mut accounts = self
            .accounts
            .lock()
            .map_err(|_| SettingsError::Unavailable)?;
        Ok(f(accounts.entry(id.as_uuid()).or_default()))
    }
}

impl SettingsStore for MemorySettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError> {
        self.with_account(id, |s| s.display_name = Some(name.clone()))
    }

    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
        self.with_account(id, |s| s.display_name.clone())
    }

    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError> {
        self.with_account(id, |s| s.provisioned_at = Some(unix_ms))
    }

    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
        self.with_account(id, |s| s.provisioned_at)
    }

    fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError> {
        self.with_account(id, |s| s.presence_enabled)
    }

    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        self.with_account(id, |s| s.presence_enabled = Some(on))
    }

    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        self.with_account(id, |s| s.relay_url.clone())
    }

    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        let canonical = url.map(validate_relay_url).transpose()?;
        self.with_account(id, |s| s.relay_url = canonical)
    }
}

const SETTINGS_FILE_NAME: &str = "settings.json";
const SETTINGS_VERSION: u32 = 1;
/// Upper bound on the settings file size read into memory.
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAccount {
    display_name: Option<String>,
    provisioned_at: Option<i64>,
    #[serde(default)]
    presence_enabled: Option<bool>,
    #[serde(default)]
    relay_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDocument {
    version: u32,
    accounts: BTreeMap<String, StoredAccount>,
}

/// One account's settings as read from the legacy file, already validated and
/// in canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacySettings {
    pub display_name: Option<String>,
    pub provisioned_at: Option<i64>,
    pub presence_enabled: Option<bool>,
    pub relay_url: Option<String>,
}

/// Parses `<data_dir>/settings.json`. Never opens the database and never
/// writes. `Ok(None)` means the file is absent. A symlink or other non-regular
/// file is `NotRegularFile` and is never followed.
pub(crate) fn read_legacy_settings(
    data_dir: &Path,
) -> Result<Option<Vec<(AccountId, LegacySettings)>>, LegacyError> {
    let path = data_dir.join(SETTINGS_FILE_NAME);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LegacyError::Io),
    };
    if !meta.file_type().is_file() {
        return Err(LegacyError::NotRegularFile);
    }
    if meta.len() > MAX_SETTINGS_BYTES {
        return Err(LegacyError::Corrupt);
    }
    let bytes = fs::read(&path).map_err(|_| LegacyError::Io)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(LegacyError::Corrupt);
    }
    let doc: SettingsDocument = serde_json::from_slice(&bytes).map_err(|_| LegacyError::Corrupt)?;
    if doc.version != SETTINGS_VERSION {
        return Err(LegacyError::Corrupt);
    }
    doc.accounts
        .into_iter()
        .map(|(key, stored)| {
            let uuid = Uuid::parse_str(&key).map_err(|_| LegacyError::Corrupt)?;
            Ok((AccountId::from_uuid(uuid), legacy_settings(stored)?))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn legacy_settings(stored: StoredAccount) -> Result<LegacySettings, LegacyError> {
    let display_name = match stored.display_name.as_deref() {
        Some(raw) => match validate_display_name(raw) {
            Ok(name) => Some(name.as_str().to_owned()),
            // Saved under older, looser rules: treated as unset.
            Err(DisplayNameError::InvisibleCharacter) => None,
            Err(_) => return Err(LegacyError::Corrupt),
        },
        None => None,
    };
    let relay_url = stored
        .relay_url
        .as_deref()
        .map(|raw| validate_relay_url(raw).map_err(|_| LegacyError::Corrupt))
        .transpose()?;
    if stored.provisioned_at.is_some_and(|t| t < 0) {
        return Err(LegacyError::Corrupt);
    }
    Ok(LegacySettings {
        display_name,
        provisioned_at: stored.provisioned_at,
        presence_enabled: stored.presence_enabled,
        relay_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, body: &str) {
        fs::write(dir.path().join(SETTINGS_FILE_NAME), body).unwrap();
    }

    fn doc(id: &AccountId, fields: serde_json::Value) -> String {
        serde_json::json!({
            "version": 1,
            "accounts": { id.as_uuid().hyphenated().to_string(): fields }
        })
        .to_string()
    }

    #[test]
    fn absent_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_legacy_settings(dir.path()), Ok(None));
    }

    #[test]
    fn full_and_old_v1_documents_parse() {
        let dir = tempfile::tempdir().unwrap();
        let id = AccountId::new();
        write(
            &dir,
            &doc(
                &id,
                serde_json::json!({
                    "display_name": " Ada ", "provisioned_at": 5,
                    "presence_enabled": false, "relay_url": "https://relay.example"
                }),
            ),
        );
        let parsed = read_legacy_settings(dir.path()).unwrap().unwrap();
        assert_eq!(
            parsed,
            vec![(
                id,
                LegacySettings {
                    display_name: Some("Ada".into()),
                    provisioned_at: Some(5),
                    presence_enabled: Some(false),
                    relay_url: Some("https://relay.example/".into()),
                }
            )]
        );
        write(
            &dir,
            &doc(
                &id,
                serde_json::json!({"display_name": "Ada", "provisioned_at": 5}),
            ),
        );
        let parsed = read_legacy_settings(dir.path()).unwrap().unwrap();
        assert_eq!(parsed[0].1.presence_enabled, None);
        assert_eq!(parsed[0].1.relay_url, None);
    }

    #[test]
    fn invisible_stored_name_reads_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let id = AccountId::new();
        write(
            &dir,
            &doc(
                &id,
                serde_json::json!({"display_name": "Ada\u{200B}", "presence_enabled": false}),
            ),
        );
        let parsed = read_legacy_settings(dir.path()).unwrap().unwrap();
        assert_eq!(parsed[0].1.display_name, None);
        assert_eq!(parsed[0].1.presence_enabled, Some(false));
    }

    #[test]
    fn corrupt_files_are_typed_errors() {
        let id = AccountId::new();
        let too_long = "a".repeat(10_000);
        let padded = format!(
            r#"{{"version":1,"accounts":{{}}}}{}"#,
            " ".repeat(MAX_SETTINGS_BYTES as usize)
        );
        let cases = [
            "{not json \u{ff}".to_owned(),
            r#"{"version":2,"accounts":{}}"#.to_owned(),
            r#"{"version":1,"accounts":{"not-a-uuid":{}}}"#.to_owned(),
            doc(
                &id,
                serde_json::json!({"display_name": "Ada", "secret": "x"}),
            ),
            doc(&id, serde_json::json!({"display_name": "Ada\nEve"})),
            doc(&id, serde_json::json!({"display_name": "Ada\u{202E}Eve"})),
            doc(&id, serde_json::json!({"display_name": ""})),
            doc(&id, serde_json::json!({ "display_name": too_long })),
            doc(&id, serde_json::json!({"relay_url": "http://evil.example"})),
            doc(&id, serde_json::json!({"provisioned_at": -1})),
            padded,
        ];
        for body in cases {
            let dir = tempfile::tempdir().unwrap();
            write(&dir, &body);
            assert_eq!(
                read_legacy_settings(dir.path()),
                Err(LegacyError::Corrupt),
                "{body:.80}"
            );
            assert_eq!(
                fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap(),
                body
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_is_not_regular() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        fs::write(&target, r#"{"version":1,"accounts":{}}"#).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(SETTINGS_FILE_NAME)).unwrap();
        assert_eq!(
            read_legacy_settings(dir.path()),
            Err(LegacyError::NotRegularFile)
        );
    }
}
