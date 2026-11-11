//! File-backed [`KeychainStore`] for the headless test node.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use messenger_core::identity::{AccountId, DbKey, KeychainError, KeychainStore, SecretKeyBytes};
use zeroize::{Zeroize, Zeroizing};

const IDENTITY_SUFFIX: &str = "identity-key";
const DB_SUFFIX: &str = "db-key";
const KEY_LEN: usize = 32;

/// CONTAINER/TEST ONLY. Secrets rest unencrypted on the container filesystem.
/// Never use for a real identity. The desktop app always uses core's OsKeychain.
pub struct FileKeychain {
    dir: PathBuf,
}

fn backend(msg: impl Into<String>) -> KeychainError {
    KeychainError::Backend(msg.into())
}

fn io_err(e: std::io::Error) -> KeychainError {
    match e.kind() {
        std::io::ErrorKind::NotFound => KeychainError::NotFound,
        std::io::ErrorKind::PermissionDenied => KeychainError::AccessDenied,
        _ => backend(e.to_string()),
    }
}

impl FileKeychain {
    /// Opens (creating, mode 0700 on unix) the keyring directory.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, KeychainError> {
        let dir = dir.into();
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(backend("keyring path is not a directory")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => create_private_dir(&dir)?,
            Err(e) => return Err(io_err(e)),
        }
        restrict_dir(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, id: &AccountId, suffix: &str) -> PathBuf {
        self.dir.join(format!("{}.{suffix}", id.as_uuid()))
    }

    fn store(
        &self,
        id: &AccountId,
        suffix: &str,
        key: &[u8; KEY_LEN],
    ) -> Result<(), KeychainError> {
        let path = self.path(id, suffix);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return Err(backend("keyring entry is not a regular file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(e)),
        }
        let tmp = path.with_extension(format!("{suffix}.tmp"));
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(e)),
        }
        let result = write_private(&tmp, key).and_then(|()| fs::rename(&tmp, &path));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.map_err(io_err)
    }

    fn load(
        &self,
        id: &AccountId,
        suffix: &str,
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, KeychainError> {
        let path = self.path(id, suffix);
        let meta = fs::symlink_metadata(&path).map_err(io_err)?;
        if !meta.is_file() {
            return Err(backend("keyring entry is not a regular file"));
        }
        let mut raw = Zeroizing::new(Vec::with_capacity(KEY_LEN + 1));
        fs::File::open(&path)
            .map_err(io_err)?
            .take(KEY_LEN as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(io_err)?;
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        if raw.len() != KEY_LEN {
            return Err(backend("stored key has wrong length"));
        }
        bytes.copy_from_slice(&raw);
        raw.zeroize();
        Ok(bytes)
    }

    fn delete(&self, id: &AccountId, suffix: &str) -> Result<(), KeychainError> {
        let path = self.path(id, suffix);
        let meta = fs::symlink_metadata(&path).map_err(io_err)?;
        if !meta.is_file() {
            return Err(backend("keyring entry is not a regular file"));
        }
        fs::remove_file(&path).map_err(io_err)
    }

    /// The keyring directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), KeychainError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io_err)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), KeychainError> {
    fs::create_dir_all(dir).map_err(io_err)
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) -> Result<(), KeychainError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io_err)
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) -> Result<(), KeychainError> {
    Ok(())
}

fn write_private(path: &Path, key: &[u8; KEY_LEN]) -> std::io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(key)?;
    file.sync_all()
}

impl KeychainStore for FileKeychain {
    fn store_identity_key(
        &self,
        id: &AccountId,
        key: &SecretKeyBytes,
    ) -> Result<(), KeychainError> {
        self.store(id, IDENTITY_SUFFIX, key.as_bytes())
    }

    fn store_db_key(&self, id: &AccountId, key: &DbKey) -> Result<(), KeychainError> {
        self.store(id, DB_SUFFIX, key.as_bytes())
    }

    fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError> {
        Ok(SecretKeyBytes::from_bytes(*self.load(id, IDENTITY_SUFFIX)?))
    }

    fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError> {
        Ok(DbKey::from_bytes(*self.load(id, DB_SUFFIX)?))
    }

    fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError> {
        self.delete(id, IDENTITY_SUFFIX)
    }

    fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError> {
        self.delete(id, DB_SUFFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::identity::{
        IdentityService, IdentityStatus, MemorySettingsStore, NodeIdTicketProvider,
    };

    // Throwaway test patterns, not real secrets.
    const ID_KEY: [u8; 32] = [7; 32];
    const DB_KEY: [u8; 32] = [9; 32];

    fn open(dir: &tempfile::TempDir) -> FileKeychain {
        FileKeychain::open(dir.path().join("keyring")).unwrap()
    }

    #[test]
    fn round_trip_and_delete_both_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let kc = open(&tmp);
        let id = AccountId::new();
        kc.store_identity_key(&id, &SecretKeyBytes::from_bytes(ID_KEY))
            .unwrap();
        kc.store_db_key(&id, &DbKey::from_bytes(DB_KEY)).unwrap();
        assert_eq!(kc.load_identity_key(&id).unwrap().as_bytes(), &ID_KEY);
        assert_eq!(kc.load_db_key(&id).unwrap().as_bytes(), &DB_KEY);

        kc.store_identity_key(&id, &SecretKeyBytes::from_bytes(DB_KEY))
            .unwrap();
        assert_eq!(kc.load_identity_key(&id).unwrap().as_bytes(), &DB_KEY);

        kc.delete_identity_key(&id).unwrap();
        kc.delete_db_key(&id).unwrap();
        assert!(matches!(
            kc.load_identity_key(&id),
            Err(KeychainError::NotFound)
        ));
        assert!(matches!(kc.load_db_key(&id), Err(KeychainError::NotFound)));
        assert!(fs::read_dir(kc.dir()).unwrap().next().is_none());
    }

    #[test]
    fn missing_entries_are_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let kc = open(&tmp);
        let id = AccountId::new();
        assert!(matches!(
            kc.load_identity_key(&id),
            Err(KeychainError::NotFound)
        ));
        assert!(matches!(kc.load_db_key(&id), Err(KeychainError::NotFound)));
        assert!(matches!(
            kc.delete_identity_key(&id),
            Err(KeychainError::NotFound)
        ));
        assert!(matches!(
            kc.delete_db_key(&id),
            Err(KeychainError::NotFound)
        ));
    }

    #[test]
    fn wrong_length_is_a_backend_error() {
        let tmp = tempfile::tempdir().unwrap();
        let kc = open(&tmp);
        let id = AccountId::new();
        for bad in [&[1u8; 31][..], &[1u8; 33][..], &[][..]] {
            fs::write(kc.path(&id, IDENTITY_SUFFIX), bad).unwrap();
            match kc.load_identity_key(&id) {
                Err(KeychainError::Backend(m)) => assert_eq!(m, "stored key has wrong length"),
                other => panic!("unexpected: {:?}", other.map(|_| ())),
            }
        }
    }

    #[test]
    fn accounts_are_isolated() {
        let tmp = tempfile::tempdir().unwrap();
        let kc = open(&tmp);
        let (a, b) = (AccountId::new(), AccountId::new());
        kc.store_db_key(&a, &DbKey::from_bytes(DB_KEY)).unwrap();
        assert!(matches!(kc.load_db_key(&b), Err(KeychainError::NotFound)));
        kc.store_db_key(&b, &DbKey::from_bytes(ID_KEY)).unwrap();
        assert_eq!(kc.load_db_key(&a).unwrap().as_bytes(), &DB_KEY);
        kc.delete_db_key(&b).unwrap();
        assert_eq!(kc.load_db_key(&a).unwrap().as_bytes(), &DB_KEY);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_entries_are_refused_and_target_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let kc = open(&tmp);
        let id = AccountId::new();
        let target = tmp.path().join("target");
        fs::write(&target, ID_KEY).unwrap();
        std::os::unix::fs::symlink(&target, kc.path(&id, IDENTITY_SUFFIX)).unwrap();

        assert!(matches!(
            kc.load_identity_key(&id),
            Err(KeychainError::Backend(_))
        ));
        assert!(matches!(
            kc.store_identity_key(&id, &SecretKeyBytes::from_bytes(DB_KEY)),
            Err(KeychainError::Backend(_))
        ));
        assert!(matches!(
            kc.delete_identity_key(&id),
            Err(KeychainError::Backend(_))
        ));
        assert_eq!(fs::read(&target).unwrap(), ID_KEY);
        assert!(fs::symlink_metadata(kc.path(&id, IDENTITY_SUFFIX)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn open_refuses_symlinked_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(FileKeychain::open(&link).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn modes_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let kc = open(&tmp);
        let id = AccountId::new();
        kc.store_identity_key(&id, &SecretKeyBytes::from_bytes(ID_KEY))
            .unwrap();
        kc.store_db_key(&id, &DbKey::from_bytes(DB_KEY)).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(kc.dir()), 0o700);
        assert_eq!(mode(&kc.path(&id, IDENTITY_SUFFIX)), 0o600);
        assert_eq!(mode(&kc.path(&id, DB_SUFFIX)), 0o600);
        assert_eq!(fs::read_dir(kc.dir()).unwrap().count(), 2);
    }

    #[test]
    fn identity_service_provisions_and_loads_over_file_keychain() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = IdentityService::new(
            open(&tmp),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            tmp.path(),
        );
        assert_eq!(svc.status(), IdentityStatus::Absent);
        let p = svc.provision().unwrap();
        let reopened = IdentityService::new(
            open(&tmp),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            tmp.path(),
        );
        assert_eq!(reopened.load().unwrap().node_id, p.node_id);
    }
}
