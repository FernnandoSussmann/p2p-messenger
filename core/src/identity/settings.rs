//! The DD-05 seam for per-account settings.
//!
//! [`MemorySettingsStore`] is the test double. [`FileSettingsStore`] is the
//! **interim** durable store. DD-05 replaces both with a SQLCipher-backed store
//! opened with the `DbKey`. Both hold the display name, provisioning
//! timestamp, LAN presence flag and relay override only, and never any secret.
//!
//! `FileSettingsStore` deliberately deviates from DD-01 ("encrypted DB"): it
//! keeps these low-sensitivity values in a plaintext JSON file in the app
//! data dir. DD-05 removes this deviation.
//!
//! Interim deviation from DD-02: presence and the relay override should live
//! in the DD-05 settings table keyed by `account_id`. That table does not
//! exist yet, so they sit in the same per-account entry; DD-05 migrates them
//! with the rest of the settings.
//!
//! The `DbKey` is created before the display name is captured, so the
//! encrypted store is already openable when the first setting is written.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use iroh::RelayUrl;
use serde::{Deserialize, Serialize};

use super::{validate_display_name, AccountId, DisplayName};

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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAccount {
    display_name: Option<String>,
    provisioned_at: Option<i64>,
    #[serde(default)]
    presence_enabled: Option<bool>,
    #[serde(default)]
    relay_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDocument {
    version: u32,
    accounts: BTreeMap<String, StoredAccount>,
}

impl SettingsDocument {
    fn empty() -> Self {
        Self {
            version: SETTINGS_VERSION,
            accounts: BTreeMap::new(),
        }
    }
}

/// Interim durable store: `<dir>/settings.json`, replaced by DD-05.
pub struct FileSettingsStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileSettingsStore {
    /// `dir` is the app-data directory. Performs no I/O.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            path: dir.into().join(SETTINGS_FILE_NAME),
            lock: Mutex::new(()),
        }
    }

    fn tmp_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    }

    /// Missing file is an empty document; anything unreadable or invalid is
    /// `Unavailable` and must never be treated as empty.
    fn read_document(&self) -> Result<SettingsDocument, SettingsError> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(SettingsDocument::empty()),
            Err(_) => return Err(SettingsError::Unavailable),
        };
        if !meta.file_type().is_file() || meta.len() > MAX_SETTINGS_BYTES {
            return Err(SettingsError::Unavailable);
        }
        let bytes = fs::read(&self.path).map_err(|_| SettingsError::Unavailable)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(SettingsError::Unavailable);
        }
        let doc: SettingsDocument =
            serde_json::from_slice(&bytes).map_err(|_| SettingsError::Unavailable)?;
        if doc.version != SETTINGS_VERSION {
            return Err(SettingsError::Unavailable);
        }
        Ok(doc)
    }

    fn write_document(&self, doc: &SettingsDocument) -> Result<(), SettingsError> {
        let bytes = serde_json::to_vec_pretty(doc).map_err(|_| SettingsError::Unavailable)?;
        let tmp_path = self.tmp_path();
        let result = (|| -> io::Result<()> {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            // Non-unix: confidentiality relies on the per-user app data dir ACLs.
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            let mut f = opts.open(&tmp_path)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp_path, &self.path)?;
            // Best-effort directory fsync so the rename is durable (unix).
            #[cfg(unix)]
            if let Some(dir) = self.path.parent() {
                if let Ok(d) = fs::File::open(dir) {
                    let _ = d.sync_all();
                }
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        result.map_err(|_| SettingsError::Unavailable)
    }

    fn read_account<T>(
        &self,
        id: &AccountId,
        f: impl FnOnce(Option<&StoredAccount>) -> Result<T, SettingsError>,
    ) -> Result<T, SettingsError> {
        let _guard = self.lock.lock().map_err(|_| SettingsError::Unavailable)?;
        let doc = self.read_document()?;
        f(doc.accounts.get(&id.as_uuid().hyphenated().to_string()))
    }

    fn update_account(
        &self,
        id: &AccountId,
        f: impl FnOnce(&mut StoredAccount),
    ) -> Result<(), SettingsError> {
        let _guard = self.lock.lock().map_err(|_| SettingsError::Unavailable)?;
        let mut doc = self.read_document()?;
        f(doc
            .accounts
            .entry(id.as_uuid().hyphenated().to_string())
            .or_insert(StoredAccount {
                display_name: None,
                provisioned_at: None,
                presence_enabled: None,
                relay_url: None,
            }));
        self.write_document(&doc)
    }
}

impl SettingsStore for FileSettingsStore {
    fn set_display_name(&self, id: &AccountId, name: &DisplayName) -> Result<(), SettingsError> {
        self.update_account(id, |a| a.display_name = Some(name.as_str().to_owned()))
    }

    fn get_display_name(&self, id: &AccountId) -> Result<Option<DisplayName>, SettingsError> {
        self.read_account(id, |a| match a.and_then(|a| a.display_name.as_deref()) {
            Some(raw) => validate_display_name(raw)
                .map(Some)
                .map_err(|_| SettingsError::Unavailable),
            None => Ok(None),
        })
    }

    fn set_provisioned_at(&self, id: &AccountId, unix_ms: i64) -> Result<(), SettingsError> {
        self.update_account(id, |a| a.provisioned_at = Some(unix_ms))
    }

    fn get_provisioned_at(&self, id: &AccountId) -> Result<Option<i64>, SettingsError> {
        self.read_account(id, |a| Ok(a.and_then(|a| a.provisioned_at)))
    }

    fn get_presence(&self, id: &AccountId) -> Result<Option<bool>, SettingsError> {
        self.read_account(id, |a| Ok(a.and_then(|a| a.presence_enabled)))
    }

    fn set_presence(&self, id: &AccountId, on: bool) -> Result<(), SettingsError> {
        self.update_account(id, |a| a.presence_enabled = Some(on))
    }

    fn get_relay_url(&self, id: &AccountId) -> Result<Option<String>, SettingsError> {
        self.read_account(id, |a| match a.and_then(|a| a.relay_url.as_deref()) {
            Some(raw) => validate_relay_url(raw).map(Some),
            None => Ok(None),
        })
    }

    fn set_relay_url(&self, id: &AccountId, url: Option<&str>) -> Result<(), SettingsError> {
        let canonical = url.map(validate_relay_url).transpose()?;
        self.update_account(id, |a| a.relay_url = canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::validate_display_name;

    fn name(raw: &str) -> DisplayName {
        validate_display_name(raw).unwrap()
    }

    #[test]
    fn set_get_display_name_round_trips() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        assert_eq!(
            store.get_display_name(&id).unwrap().unwrap().as_str(),
            "Ada"
        );
    }

    #[test]
    fn overwrite_display_name() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_display_name(&id, &name("Grace")).unwrap();
        assert_eq!(
            store.get_display_name(&id).unwrap().unwrap().as_str(),
            "Grace"
        );
    }

    #[test]
    fn provisioned_at_round_trips() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        store.set_provisioned_at(&id, 1_700_000_000_000).unwrap();
        assert_eq!(
            store.get_provisioned_at(&id).unwrap(),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn missing_returns_none() {
        let store = MemorySettingsStore::new();
        let id = AccountId::new();
        assert!(store.get_display_name(&id).unwrap().is_none());
        assert!(store.get_provisioned_at(&id).unwrap().is_none());
    }

    #[test]
    fn accounts_are_isolated() {
        let store = MemorySettingsStore::new();
        let (a, b) = (AccountId::new(), AccountId::new());
        store.set_display_name(&a, &name("Ada")).unwrap();
        assert!(store.get_display_name(&b).unwrap().is_none());
    }

    fn file_store() -> (tempfile::TempDir, FileSettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSettingsStore::new(dir.path());
        (dir, store)
    }

    fn settings_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("settings.json")
    }

    fn doc_with_name(id: &AccountId, name: &str) -> String {
        serde_json::json!({
            "version": 1,
            "accounts": { id.as_uuid().hyphenated().to_string(): { "display_name": name } }
        })
        .to_string()
    }

    #[test]
    fn file_store_round_trips_display_name_and_provisioned_at() {
        let (_dir, store) = file_store();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_provisioned_at(&id, 1_700_000_000_000).unwrap();
        assert_eq!(
            store.get_display_name(&id).unwrap().unwrap().as_str(),
            "Ada"
        );
        assert_eq!(
            store.get_provisioned_at(&id).unwrap(),
            Some(1_700_000_000_000)
        );
    }

    #[test]
    fn file_store_persists_across_instances() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_provisioned_at(&id, 42).unwrap();
        drop(store);
        let reopened = FileSettingsStore::new(dir.path());
        assert_eq!(
            reopened.get_display_name(&id).unwrap().unwrap().as_str(),
            "Ada"
        );
        assert_eq!(reopened.get_provisioned_at(&id).unwrap(), Some(42));
    }

    #[test]
    fn file_store_overwrite_display_name() {
        let (_dir, store) = file_store();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_display_name(&id, &name("Grace")).unwrap();
        assert_eq!(
            store.get_display_name(&id).unwrap().unwrap().as_str(),
            "Grace"
        );
    }

    #[test]
    fn file_store_missing_file_returns_none() {
        let (_dir, store) = file_store();
        let id = AccountId::new();
        assert!(store.get_display_name(&id).unwrap().is_none());
        assert!(store.get_provisioned_at(&id).unwrap().is_none());
    }

    #[test]
    fn file_store_accounts_are_isolated() {
        let (_dir, store) = file_store();
        let (a, b) = (AccountId::new(), AccountId::new());
        store.set_display_name(&a, &name("Ada")).unwrap();
        store.set_provisioned_at(&a, 1).unwrap();
        assert!(store.get_display_name(&b).unwrap().is_none());
        assert!(store.get_provisioned_at(&b).unwrap().is_none());
        store.set_display_name(&b, &name("Grace")).unwrap();
        assert_eq!(store.get_display_name(&a).unwrap().unwrap().as_str(), "Ada");
    }

    #[test]
    fn file_store_atomic_write_leaves_no_temp_file() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        for i in 0..4 {
            store.set_provisioned_at(&id, i).unwrap();
        }
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.json")]);
    }

    #[test]
    fn presence_defaults_to_none_and_round_trips_in_both_stores() {
        let (_dir, file) = file_store();
        let memory = MemorySettingsStore::new();
        let stores: [&dyn SettingsStore; 2] = [&file, &memory];
        for store in stores {
            let id = AccountId::new();
            assert_eq!(store.get_presence(&id).unwrap(), None);
            store.set_presence(&id, false).unwrap();
            assert_eq!(store.get_presence(&id).unwrap(), Some(false));
            store.set_presence(&id, true).unwrap();
            assert_eq!(store.get_presence(&id).unwrap(), Some(true));
        }
    }

    #[test]
    fn relay_url_round_trips_canonicalizes_and_clears_in_both_stores() {
        let (_dir, file) = file_store();
        let memory = MemorySettingsStore::new();
        let stores: [&dyn SettingsStore; 2] = [&file, &memory];
        for store in stores {
            let id = AccountId::new();
            assert_eq!(store.get_relay_url(&id).unwrap(), None);
            store
                .set_relay_url(&id, Some("https://relay.example"))
                .unwrap();
            let stored = store.get_relay_url(&id).unwrap().unwrap();
            assert_eq!(stored, "https://relay.example/");
            store.set_relay_url(&id, None).unwrap();
            assert_eq!(store.get_relay_url(&id).unwrap(), None);
        }
    }

    #[test]
    fn relay_url_validation_rejects_bad_schemes_and_sizes() {
        let (_dir, file) = file_store();
        let memory = MemorySettingsStore::new();
        let stores: [&dyn SettingsStore; 2] = [&file, &memory];
        let too_long = format!("https://{}.example", "a".repeat(2100));
        for store in stores {
            let id = AccountId::new();
            for bad in [
                "http://relay.example",
                "ftp://relay.example",
                "not a url",
                "",
                "http://192.168.1.5",
                too_long.as_str(),
            ] {
                assert!(
                    matches!(
                        store.set_relay_url(&id, Some(bad)),
                        Err(SettingsError::Unavailable)
                    ),
                    "should reject {bad:?}"
                );
            }
            for ok in [
                "http://localhost:3340",
                "http://127.0.0.1:3340",
                "http://[::1]:3340",
            ] {
                store.set_relay_url(&id, Some(ok)).unwrap();
            }
            assert!(store.get_relay_url(&id).unwrap().is_some());
        }
    }

    #[test]
    fn presence_and_relay_are_isolated_per_account() {
        let (_dir, store) = file_store();
        let (a, b) = (AccountId::new(), AccountId::new());
        store.set_presence(&a, false).unwrap();
        store
            .set_relay_url(&a, Some("https://relay.example"))
            .unwrap();
        assert_eq!(store.get_presence(&b).unwrap(), None);
        assert_eq!(store.get_relay_url(&b).unwrap(), None);
    }

    #[test]
    fn old_v1_file_without_new_fields_parses() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        let doc = serde_json::json!({
            "version": 1,
            "accounts": { id.as_uuid().hyphenated().to_string(): {
                "display_name": "Ada", "provisioned_at": 5
            } }
        });
        fs::write(settings_path(&dir), doc.to_string()).unwrap();
        assert_eq!(store.get_provisioned_at(&id).unwrap(), Some(5));
        assert_eq!(store.get_presence(&id).unwrap(), None);
        assert_eq!(store.get_relay_url(&id).unwrap(), None);
    }

    #[test]
    fn tampered_relay_url_in_file_is_unavailable() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        let doc = serde_json::json!({
            "version": 1,
            "accounts": { id.as_uuid().hyphenated().to_string(): {
                "relay_url": "http://evil.example"
            } }
        });
        fs::write(settings_path(&dir), doc.to_string()).unwrap();
        assert!(matches!(
            store.get_relay_url(&id),
            Err(SettingsError::Unavailable)
        ));
    }

    #[test]
    fn file_store_file_contains_no_secrets_and_only_known_keys() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        store.set_display_name(&id, &name("Ada")).unwrap();
        store.set_provisioned_at(&id, 7).unwrap();
        store.set_presence(&id, false).unwrap();
        store
            .set_relay_url(&id, Some("https://relay.example"))
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(settings_path(&dir)).unwrap()).unwrap();
        let mut top: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        top.sort();
        assert_eq!(top, vec!["accounts", "version"]);
        for account in v["accounts"].as_object().unwrap().values() {
            let mut keys: Vec<_> = account.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(
                keys,
                vec![
                    "display_name",
                    "presence_enabled",
                    "provisioned_at",
                    "relay_url"
                ]
            );
        }
    }

    #[test]
    fn file_store_corrupt_json_is_unavailable_and_not_overwritten() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        let garbage: &[u8] = b"{not json \xff";
        fs::write(settings_path(&dir), garbage).unwrap();
        assert!(matches!(
            store.get_display_name(&id),
            Err(SettingsError::Unavailable)
        ));
        assert!(matches!(
            store.get_provisioned_at(&id),
            Err(SettingsError::Unavailable)
        ));
        assert!(matches!(
            store.set_display_name(&id, &name("Ada")),
            Err(SettingsError::Unavailable)
        ));
        assert!(matches!(
            store.set_provisioned_at(&id, 1),
            Err(SettingsError::Unavailable)
        ));
        assert_eq!(fs::read(settings_path(&dir)).unwrap(), garbage);
    }

    #[test]
    fn file_store_wrong_version_is_unavailable() {
        let (dir, store) = file_store();
        fs::write(settings_path(&dir), r#"{"version":2,"accounts":{}}"#).unwrap();
        assert!(matches!(
            store.get_display_name(&AccountId::new()),
            Err(SettingsError::Unavailable)
        ));
    }

    #[test]
    fn file_store_unknown_field_in_account_is_unavailable() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        let doc = serde_json::json!({
            "version": 1,
            "accounts": { id.as_uuid().hyphenated().to_string(): {
                "display_name": "Ada", "secret": "x"
            } }
        });
        fs::write(settings_path(&dir), doc.to_string()).unwrap();
        assert!(matches!(
            store.get_display_name(&id),
            Err(SettingsError::Unavailable)
        ));
    }

    #[test]
    fn file_store_tampered_display_name_fails_validation() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        let too_long = "a".repeat(10_000);
        for bad in ["Ada\nEve", "Ada\u{202E}Eve", "", too_long.as_str()] {
            fs::write(settings_path(&dir), doc_with_name(&id, bad)).unwrap();
            assert!(
                matches!(store.get_display_name(&id), Err(SettingsError::Unavailable)),
                "should fail: {bad:?}"
            );
        }
    }

    #[test]
    fn file_store_oversized_file_is_unavailable() {
        let (dir, store) = file_store();
        let id = AccountId::new();
        let padding = " ".repeat(MAX_SETTINGS_BYTES as usize);
        fs::write(
            settings_path(&dir),
            format!(r#"{{"version":1,"accounts":{{}}}}{padding}"#),
        )
        .unwrap();
        assert!(matches!(
            store.get_display_name(&id),
            Err(SettingsError::Unavailable)
        ));
        assert!(matches!(
            store.set_provisioned_at(&id, 1),
            Err(SettingsError::Unavailable)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn file_store_rejects_symlinked_settings_file() {
        let (dir, store) = file_store();
        let target = dir.path().join("elsewhere");
        fs::write(&target, r#"{"version":1,"accounts":{}}"#).unwrap();
        std::os::unix::fs::symlink(&target, settings_path(&dir)).unwrap();
        let id = AccountId::new();
        assert!(matches!(
            store.get_display_name(&id),
            Err(SettingsError::Unavailable)
        ));
        assert!(matches!(
            store.set_provisioned_at(&id, 1),
            Err(SettingsError::Unavailable)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn file_store_file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store) = file_store();
        store.set_provisioned_at(&AccountId::new(), 1).unwrap();
        let mode = fs::metadata(settings_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn file_store_unicode_name_round_trips_verbatim() {
        let (_dir, store) = file_store();
        let id = AccountId::new();
        let n = "Ada 🦀 Łukasz ñ 日本語";
        store.set_display_name(&id, &name(n)).unwrap();
        assert_eq!(store.get_display_name(&id).unwrap().unwrap().as_str(), n);
    }

    #[test]
    fn file_store_concurrent_writers_do_not_lose_updates() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(FileSettingsStore::new(dir.path()));
        let ids: Vec<_> = (0..8).map(|_| AccountId::new()).collect();
        let handles: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let (store, id) = (std::sync::Arc::clone(&store), *id);
                std::thread::spawn(move || {
                    store
                        .set_display_name(&id, &name(&format!("User{i}")))
                        .unwrap();
                    store.set_provisioned_at(&id, i as i64).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                store.get_display_name(id).unwrap().unwrap().as_str(),
                format!("User{i}")
            );
            assert_eq!(store.get_provisioned_at(id).unwrap(), Some(i as i64));
        }
        let _: serde_json::Value =
            serde_json::from_slice(&fs::read(settings_path(&dir)).unwrap()).unwrap();
    }
}
