//! The trust seam: which NodeIds may connect without an Accept/Decline prompt.
//!
//! [`FileKnownPeers`] is the **interim** plaintext store (the same documented
//! deviation as `FileSettingsStore`). It holds public NodeIds and advisory
//! display names only. DD-03 replaces it with the encrypted `contacts` table.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::identity::{validate_display_name, AccountId, DisplayName, NodeId};

/// Persistent set of peers the user has already accepted or deliberately
/// dialed.
pub trait KnownPeers: Send + Sync + 'static {
    /// Trust check, keyed on NodeId only. An `Err` must be treated as
    /// "unknown" (fail closed): the caller prompts instead of connecting
    /// silently.
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError>;

    /// Records an accepted or dialed peer. `display_name` is the last-seen
    /// self-asserted name: advisory only, never a key, never used for lookup
    /// or dedup.
    fn add(&self, id: NodeId, display_name: Option<DisplayName>) -> Result<(), KnownPeersError>;

    /// Advisory name of an already-known peer. Hello (authorized connection):
    /// `name` replaces the stored name (None clears) and marks it seen.
    /// Lan: called only with Some(name); never marks "seen". Must never create
    /// an entry or change trust; a no-op for unknown ids.
    fn observe(
        &self,
        id: &NodeId,
        name: Option<&DisplayName>,
        source: NameSource,
    ) -> Result<(), KnownPeersError>;
}

/// Where an advisory name was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSource {
    Hello,
    Lan,
}

/// Why a [`KnownPeers`] operation failed.
#[derive(Debug)]
pub enum KnownPeersError {
    /// The store is full.
    Full,
    /// The stored data is unreadable, corrupt or of an unknown version. It is
    /// never treated as empty.
    Corrupt,
    /// The backing storage could not be read or written.
    Io(std::io::Error),
}

impl fmt::Display for KnownPeersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("known peers store is full"),
            Self::Corrupt => f.write_str("known peers store is corrupt"),
            Self::Io(e) => write!(f, "known peers store I/O error: {e}"),
        }
    }
}

impl std::error::Error for KnownPeersError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

const KNOWN_PEERS_FILE_NAME: &str = "known_peers.json";
const KNOWN_PEERS_VERSION: u32 = 1;
// Sized for MAX_PEERS_PER_ACCOUNT entries with maximum-length names.
const MAX_KNOWN_PEERS_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PEERS_PER_ACCOUNT: usize = 10_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPeer {
    name: Option<String>,
    added_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownPeersDocument {
    version: u32,
    accounts: BTreeMap<String, BTreeMap<String, StoredPeer>>,
}

impl KnownPeersDocument {
    fn empty() -> Self {
        Self {
            version: KNOWN_PEERS_VERSION,
            accounts: BTreeMap::new(),
        }
    }

    fn validate(&self) -> Result<(), KnownPeersError> {
        if self.version != KNOWN_PEERS_VERSION {
            return Err(KnownPeersError::Corrupt);
        }
        for (account, peers) in &self.accounts {
            uuid::Uuid::parse_str(account).map_err(|_| KnownPeersError::Corrupt)?;
            for (id, peer) in peers {
                id.parse::<NodeId>().map_err(|_| KnownPeersError::Corrupt)?;
                if let Some(name) = &peer.name {
                    validate_display_name(name).map_err(|_| KnownPeersError::Corrupt)?;
                }
            }
        }
        Ok(())
    }
}

/// Interim durable store: `<data_dir>/known_peers.json`, scoped to one account.
pub struct FileKnownPeers {
    path: PathBuf,
    account: String,
    lock: Mutex<()>,
}

impl FileKnownPeers {
    /// `data_dir` is the app-data directory. Performs no I/O.
    pub fn new(data_dir: impl Into<PathBuf>, account: AccountId) -> Self {
        Self {
            path: data_dir.into().join(KNOWN_PEERS_FILE_NAME),
            account: account.as_uuid().hyphenated().to_string(),
            lock: Mutex::new(()),
        }
    }

    fn tmp_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    }

    /// A missing file is an empty document; anything unreadable or invalid is
    /// an error and is never treated as empty.
    fn read_document(&self) -> Result<KnownPeersDocument, KnownPeersError> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(KnownPeersDocument::empty()),
            Err(e) => return Err(KnownPeersError::Io(e)),
        };
        if !meta.file_type().is_file() || meta.len() > MAX_KNOWN_PEERS_BYTES {
            return Err(KnownPeersError::Corrupt);
        }
        let bytes = fs::read(&self.path).map_err(KnownPeersError::Io)?;
        if bytes.len() as u64 > MAX_KNOWN_PEERS_BYTES {
            return Err(KnownPeersError::Corrupt);
        }
        let doc: KnownPeersDocument =
            serde_json::from_slice(&bytes).map_err(|_| KnownPeersError::Corrupt)?;
        doc.validate()?;
        Ok(doc)
    }

    fn write_document(&self, doc: &KnownPeersDocument) -> Result<(), KnownPeersError> {
        let bytes = serde_json::to_vec(doc).map_err(|_| KnownPeersError::Corrupt)?;
        // Never write a file the read cap would then refuse.
        if bytes.len() as u64 > MAX_KNOWN_PEERS_BYTES {
            return Err(KnownPeersError::Full);
        }
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
        result.map_err(KnownPeersError::Io)
    }
}

impl KnownPeers for FileKnownPeers {
    fn is_known(&self, id: &NodeId) -> Result<bool, KnownPeersError> {
        let _guard = self.lock.lock().map_err(|_| KnownPeersError::Corrupt)?;
        let doc = self.read_document()?;
        Ok(doc
            .accounts
            .get(&self.account)
            .is_some_and(|peers| peers.contains_key(&id.to_string())))
    }

    fn add(&self, id: NodeId, display_name: Option<DisplayName>) -> Result<(), KnownPeersError> {
        let _guard = self.lock.lock().map_err(|_| KnownPeersError::Corrupt)?;
        let mut doc = self.read_document()?;
        let peers = doc.accounts.entry(self.account.clone()).or_default();
        let key = id.to_string();
        let name = display_name.map(|n| n.as_str().to_owned());
        match peers.get_mut(&key) {
            Some(existing) => {
                if name.is_none() || existing.name == name {
                    return Ok(());
                }
                existing.name = name;
            }
            None => {
                if peers.len() >= MAX_PEERS_PER_ACCOUNT {
                    return Err(KnownPeersError::Full);
                }
                let added_at_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                peers.insert(key, StoredPeer { name, added_at_ms });
            }
        }
        self.write_document(&doc)
    }

    fn observe(
        &self,
        id: &NodeId,
        name: Option<&DisplayName>,
        source: NameSource,
    ) -> Result<(), KnownPeersError> {
        let name = match (source, name) {
            (NameSource::Lan, None) => return Ok(()),
            (_, name) => name.map(|n| n.as_str().to_owned()),
        };
        let _guard = self.lock.lock().map_err(|_| KnownPeersError::Corrupt)?;
        let mut doc = self.read_document()?;
        let Some(existing) = doc
            .accounts
            .get_mut(&self.account)
            .and_then(|peers| peers.get_mut(&id.to_string()))
        else {
            return Ok(());
        };
        if existing.name == name {
            return Ok(());
        }
        existing.name = name;
        self.write_document(&doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(seed: u8) -> NodeId {
        iroh_base::SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn name(raw: &str) -> Option<DisplayName> {
        Some(validate_display_name(raw).unwrap())
    }

    fn store() -> (tempfile::TempDir, FileKnownPeers, AccountId) {
        let dir = tempfile::tempdir().unwrap();
        let account = AccountId::new();
        let store = FileKnownPeers::new(dir.path(), account);
        (dir, store, account)
    }

    fn file(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("known_peers.json")
    }

    fn only_account_peers(dir: &tempfile::TempDir) -> serde_json::Map<String, serde_json::Value> {
        let doc: serde_json::Value = serde_json::from_slice(&fs::read(file(dir)).unwrap()).unwrap();
        let accounts = doc["accounts"].as_object().unwrap();
        assert_eq!(accounts.len(), 1);
        accounts
            .values()
            .next()
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn add_then_is_known_and_others_are_not() {
        let (_dir, store, _) = store();
        assert!(!store.is_known(&node(1)).unwrap());
        store.add(node(1), name("Ada")).unwrap();
        assert!(store.is_known(&node(1)).unwrap());
        assert!(!store.is_known(&node(2)).unwrap());
    }

    #[test]
    fn survives_reopen() {
        let (dir, store, account) = store();
        store.add(node(1), None).unwrap();
        drop(store);
        let reopened = FileKnownPeers::new(dir.path(), account);
        assert!(reopened.is_known(&node(1)).unwrap());
    }

    #[test]
    fn accounts_are_isolated() {
        let (dir, store, _) = store();
        store.add(node(1), name("Ada")).unwrap();
        let other = FileKnownPeers::new(dir.path(), AccountId::new());
        assert!(!other.is_known(&node(1)).unwrap());
        other.add(node(2), None).unwrap();
        assert!(store.is_known(&node(1)).unwrap());
        assert!(!store.is_known(&node(2)).unwrap());
    }

    #[test]
    fn same_name_different_nodes_are_distinct() {
        let (dir, store, _) = store();
        store.add(node(1), name("Ada")).unwrap();
        store.add(node(2), name("Ada")).unwrap();
        assert!(store.is_known(&node(1)).unwrap());
        assert!(store.is_known(&node(2)).unwrap());
        assert!(!store.is_known(&node(3)).unwrap());
        assert_eq!(only_account_peers(&dir).len(), 2);
    }

    #[test]
    fn name_change_updates_in_place() {
        let (dir, store, _) = store();
        store.add(node(1), name("Ada")).unwrap();
        store.add(node(1), name("Ada L")).unwrap();
        store.add(node(1), None).unwrap();
        let peers = only_account_peers(&dir);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers.values().next().unwrap()["name"], "Ada L");
    }

    #[test]
    fn observe_updates_known_names_only_and_never_creates() {
        let (dir, store, _) = store();
        store
            .observe(&node(1), name("Ghost").as_ref(), NameSource::Hello)
            .unwrap();
        assert!(!file(&dir).exists());

        store.add(node(1), name("Ada")).unwrap();
        store
            .observe(&node(1), name("Ada L").as_ref(), NameSource::Lan)
            .unwrap();
        store.observe(&node(1), None, NameSource::Lan).unwrap();
        let peers = only_account_peers(&dir);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers.values().next().unwrap()["name"], "Ada L");

        store.observe(&node(1), None, NameSource::Hello).unwrap();
        let peers = only_account_peers(&dir);
        assert!(peers.values().next().unwrap()["name"].is_null());
        assert!(store.is_known(&node(1)).unwrap());
        assert!(!store.is_known(&node(2)).unwrap());
    }

    #[test]
    fn corrupt_files_error_and_are_not_overwritten() {
        let account = AccountId::new();
        let acct = account.as_uuid().hyphenated().to_string();
        let good = node(1).to_string();
        let cases = [
            "not json".to_owned(),
            r#"{"version":2,"accounts":{}}"#.to_owned(),
            r#"{"version":1,"accounts":{},"extra":1}"#.to_owned(),
            format!(
                r#"{{"version":1,"accounts":{{"{acct}":{{"nope":{{"name":null,"added_at_ms":1}}}}}}}}"#
            ),
            format!(
                r#"{{"version":1,"accounts":{{"{acct}":{{"{good}":{{"name":"a\nb","added_at_ms":1}}}}}}}}"#
            ),
            format!(
                r#"{{"version":1,"accounts":{{"x":{{"{good}":{{"name":null,"added_at_ms":1}}}}}}}}"#
            ),
        ];
        for body in cases {
            let dir = tempfile::tempdir().unwrap();
            fs::write(file(&dir), &body).unwrap();
            let store = FileKnownPeers::new(dir.path(), account);
            assert!(matches!(
                store.is_known(&node(1)),
                Err(KnownPeersError::Corrupt)
            ));
            assert!(matches!(
                store.add(node(2), None),
                Err(KnownPeersError::Corrupt)
            ));
            assert_eq!(fs::read_to_string(file(&dir)).unwrap(), body);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_is_refused() {
        let (dir, store, _) = store();
        let target = dir.path().join("elsewhere.json");
        fs::write(&target, r#"{"version":1,"accounts":{}}"#).unwrap();
        std::os::unix::fs::symlink(&target, file(&dir)).unwrap();
        assert!(store.is_known(&node(1)).is_err());
        assert!(store.add(node(1), None).is_err());
    }

    #[test]
    fn oversized_file_is_refused() {
        let (dir, store, _) = store();
        fs::write(file(&dir), vec![b' '; MAX_KNOWN_PEERS_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            store.is_known(&node(1)),
            Err(KnownPeersError::Corrupt)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn file_mode_is_0600_and_no_temp_file_remains() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store, _) = store();
        store.add(node(1), name("Ada")).unwrap();
        let mode = fs::metadata(file(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["known_peers.json"]);
    }

    #[test]
    fn full_store_reports_full_and_stays_readable() {
        let (dir, store, account) = store();
        let peers: BTreeMap<String, StoredPeer> = (0..10_000u32)
            .map(|i| {
                let mut seed = [0u8; 32];
                seed[..4].copy_from_slice(&(i + 1000).to_le_bytes());
                let id = iroh_base::SecretKey::from_bytes(&seed).public();
                (
                    id.to_string(),
                    StoredPeer {
                        name: None,
                        added_at_ms: 0,
                    },
                )
            })
            .collect();
        let existing = peers.keys().next().unwrap().parse::<NodeId>().unwrap();
        let mut doc = KnownPeersDocument::empty();
        doc.accounts
            .insert(account.as_uuid().hyphenated().to_string(), peers);
        fs::write(file(&dir), serde_json::to_vec(&doc).unwrap()).unwrap();
        let mut result = Ok(());
        for i in 0..200u32 {
            let mut seed = [1u8; 32];
            seed[..4].copy_from_slice(&i.to_le_bytes());
            result = store.add(iroh_base::SecretKey::from_bytes(&seed).public(), None);
            if result.is_err() {
                break;
            }
        }
        assert!(matches!(result, Err(KnownPeersError::Full)));
        assert!(store.is_known(&existing).unwrap());
    }
}
