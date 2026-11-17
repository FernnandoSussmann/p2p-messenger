//! The identity orchestrator: provisioning, status, load, display name, My Info.
//!
//! Provisioning order is what makes it atomic from the point of view of
//! [`IdentityService::status`]: both keychain secrets are stored and confirmed
//! first, the settings store is initialised next, and the `account_id` marker
//! is written last. Until the marker exists the identity is invisible, so a
//! failure at any step is rolled back without leaving a half-identity behind.
//!
//! A present marker whose secrets cannot be read is an error state, never
//! "absent": silently provisioning a new identity would orphan the encrypted
//! database and change the user's address.
//!
//! No type here carries a secret; keys exist only as zeroizing newtypes that
//! are handed to the keychain and dropped.

use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::keygen::{generate_db_key, generate_identity_key, node_id_from_secret_bytes};
use super::{
    read_marker, ticket_to_qr_svg, validate_display_name, write_marker, AccountId, DbKey,
    DisplayName, DisplayNameError, KeychainError, KeychainStore, MarkerError, NodeId,
    SecretKeyBytes, SettingsError, SettingsStore, Theme, Ticket, TicketError, TicketProvider,
};

/// Why an existing identity could not be read. Recoverable by the user, so it
/// is surfaced rather than treated as a missing identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusError {
    /// The marker file exists but is not a valid account id.
    MarkerCorrupt,
    /// The marker file could not be read.
    MarkerUnreadable(String),
    /// A secret the marker points to is missing from the keychain.
    SecretMissing,
    /// The keychain is locked or the user refused access.
    AccessDenied,
    /// Any other keychain failure.
    Backend(String),
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatusError::MarkerCorrupt => write!(f, "account marker is corrupt"),
            StatusError::MarkerUnreadable(msg) => write!(f, "account marker unreadable: {msg}"),
            StatusError::SecretMissing => write!(f, "identity secret missing from keychain"),
            StatusError::AccessDenied => write!(f, "keychain access denied"),
            StatusError::Backend(msg) => write!(f, "keychain backend error: {msg}"),
        }
    }
}

impl std::error::Error for StatusError {}

impl From<MarkerError> for StatusError {
    fn from(err: MarkerError) -> Self {
        match err {
            MarkerError::Corrupt => StatusError::MarkerCorrupt,
            MarkerError::Io(e) => StatusError::MarkerUnreadable(e.to_string()),
        }
    }
}

impl From<KeychainError> for StatusError {
    fn from(err: KeychainError) -> Self {
        match err {
            KeychainError::NotFound => StatusError::SecretMissing,
            KeychainError::AccessDenied => StatusError::AccessDenied,
            KeychainError::Backend(msg) => StatusError::Backend(msg),
        }
    }
}

/// Whether an identity exists on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityStatus {
    /// No marker: first run.
    Absent,
    /// Marker present and both secrets loadable.
    Present {
        account_id: AccountId,
        node_id: NodeId,
    },
    /// An identity is expected but cannot be read.
    Error(StatusError),
}

/// Public result of a successful provisioning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioned {
    pub account_id: AccountId,
    pub node_id: NodeId,
}

/// Public view of an identity loaded from the keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedIdentity {
    pub account_id: AccountId,
    pub node_id: NodeId,
}

/// Everything the My Info screen shows. Public data only.
#[derive(Debug, Clone)]
pub struct MyInfo {
    pub display_name: Option<DisplayName>,
    pub node_id: NodeId,
    pub ticket: Ticket,
    pub qr_svg: String,
}

#[derive(Debug)]
pub enum ProvisionError {
    /// An identity already exists; provisioning never duplicates or overwrites it.
    AlreadyProvisioned,
    /// An identity is expected but unreadable; provisioning never replaces it.
    Unavailable(StatusError),
    Keychain(KeychainError),
    Settings(SettingsError),
    Marker(MarkerError),
}

impl fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvisionError::AlreadyProvisioned => write!(f, "an identity already exists"),
            ProvisionError::Unavailable(e) => write!(f, "existing identity unavailable: {e}"),
            ProvisionError::Keychain(e) => write!(f, "provisioning keychain failure: {e}"),
            ProvisionError::Settings(e) => write!(f, "provisioning settings failure: {e}"),
            ProvisionError::Marker(e) => write!(f, "provisioning marker failure: {e}"),
        }
    }
}

impl std::error::Error for ProvisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProvisionError::AlreadyProvisioned => None,
            ProvisionError::Unavailable(e) => Some(e),
            ProvisionError::Keychain(e) => Some(e),
            ProvisionError::Settings(e) => Some(e),
            ProvisionError::Marker(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum LoadError {
    /// No identity exists yet.
    NotProvisioned,
    Unavailable(StatusError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotProvisioned => write!(f, "no identity has been provisioned"),
            LoadError::Unavailable(e) => write!(f, "identity unavailable: {e}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::NotProvisioned => None,
            LoadError::Unavailable(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum SetNameError {
    Invalid(DisplayNameError),
    /// No identity exists to attach the name to.
    NotProvisioned,
    Marker(MarkerError),
    Settings(SettingsError),
}

impl fmt::Display for SetNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetNameError::Invalid(e) => write!(f, "invalid display name: {e}"),
            SetNameError::NotProvisioned => write!(f, "no identity has been provisioned"),
            SetNameError::Marker(e) => write!(f, "account marker error: {e}"),
            SetNameError::Settings(e) => write!(f, "settings error: {e}"),
        }
    }
}

impl std::error::Error for SetNameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SetNameError::Invalid(e) => Some(e),
            SetNameError::NotProvisioned => None,
            SetNameError::Marker(e) => Some(e),
            SetNameError::Settings(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum ThemeError {
    /// No identity exists to attach the theme to.
    NotProvisioned,
    Marker(MarkerError),
    Settings(SettingsError),
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeError::NotProvisioned => write!(f, "no identity has been provisioned"),
            ThemeError::Marker(e) => write!(f, "account marker error: {e}"),
            ThemeError::Settings(e) => write!(f, "settings error: {e}"),
        }
    }
}

impl std::error::Error for ThemeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ThemeError::NotProvisioned => None,
            ThemeError::Marker(e) => Some(e),
            ThemeError::Settings(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum MyInfoError {
    Load(LoadError),
    Settings(SettingsError),
    Ticket(TicketError),
}

impl fmt::Display for MyInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MyInfoError::Load(e) => write!(f, "{e}"),
            MyInfoError::Settings(e) => write!(f, "settings error: {e}"),
            MyInfoError::Ticket(e) => write!(f, "ticket error: {e}"),
        }
    }
}

impl std::error::Error for MyInfoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MyInfoError::Load(e) => Some(e),
            MyInfoError::Settings(e) => Some(e),
            MyInfoError::Ticket(e) => Some(e),
        }
    }
}

/// The identity orchestrator, generic over its three boundaries.
pub struct IdentityService<K, S, T> {
    keychain: K,
    settings: S,
    tickets: T,
    data_dir: PathBuf,
    generate_identity_key: fn() -> (SecretKeyBytes, NodeId),
    generate_db_key: fn() -> DbKey,
}

impl<K, S, T> IdentityService<K, S, T>
where
    K: KeychainStore,
    S: SettingsStore,
    T: TicketProvider,
{
    /// `data_dir` is the app-data directory holding the `account_id` marker.
    pub fn new(keychain: K, settings: S, tickets: T, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            keychain,
            settings,
            tickets,
            data_dir: data_dir.into(),
            generate_identity_key,
            generate_db_key,
        }
    }

    #[cfg(test)]
    fn with_keygen(
        mut self,
        identity: fn() -> (SecretKeyBytes, NodeId),
        db: fn() -> DbKey,
    ) -> Self {
        self.generate_identity_key = identity;
        self.generate_db_key = db;
        self
    }

    pub fn status(&self) -> IdentityStatus {
        match self.inspect() {
            Ok(None) => IdentityStatus::Absent,
            Ok(Some(identity)) => IdentityStatus::Present {
                account_id: identity.account_id,
                node_id: identity.node_id,
            },
            Err(e) => IdentityStatus::Error(e),
        }
    }

    /// Generates both keys and persists a new identity. Rejected unless the
    /// status is [`IdentityStatus::Absent`].
    pub fn provision(&self) -> Result<Provisioned, ProvisionError> {
        match self.inspect() {
            Ok(None) => {}
            Ok(Some(_)) => return Err(ProvisionError::AlreadyProvisioned),
            Err(e) => return Err(ProvisionError::Unavailable(e)),
        }

        let account_id = AccountId::new();
        let (identity_key, node_id) = (self.generate_identity_key)();
        let db_key = (self.generate_db_key)();

        if let Err(e) = self.persist(&account_id, &identity_key, &db_key) {
            // Best effort: a failed delete must not mask the original error.
            let _ = self.keychain.delete_identity_key(&account_id);
            let _ = self.keychain.delete_db_key(&account_id);
            let _ = self.settings.discard_new_account(&account_id);
            return Err(e);
        }
        Ok(Provisioned {
            account_id,
            node_id,
        })
    }

    fn persist(
        &self,
        account_id: &AccountId,
        identity_key: &SecretKeyBytes,
        db_key: &DbKey,
    ) -> Result<(), ProvisionError> {
        self.keychain
            .store_identity_key(account_id, identity_key)
            .map_err(ProvisionError::Keychain)?;
        self.keychain
            .store_db_key(account_id, db_key)
            .map_err(ProvisionError::Keychain)?;
        self.settings
            .set_provisioned_at(account_id, unix_millis_now())
            .map_err(ProvisionError::Settings)?;
        write_marker(&self.data_dir, *account_id).map_err(ProvisionError::Marker)
    }

    pub fn load(&self) -> Result<LoadedIdentity, LoadError> {
        match self.inspect() {
            Ok(Some(identity)) => Ok(identity),
            Ok(None) => Err(LoadError::NotProvisioned),
            Err(e) => Err(LoadError::Unavailable(e)),
        }
    }

    /// Validates and stores the display name, overwriting any previous one.
    pub fn set_display_name(&self, name: &str) -> Result<DisplayName, SetNameError> {
        let name = validate_display_name(name).map_err(SetNameError::Invalid)?;
        let account_id = read_marker(&self.data_dir)
            .map_err(SetNameError::Marker)?
            .ok_or(SetNameError::NotProvisioned)?;
        self.settings
            .set_display_name(&account_id, &name)
            .map_err(SetNameError::Settings)?;
        Ok(name)
    }

    fn account_for_theme(&self) -> Result<AccountId, ThemeError> {
        read_marker(&self.data_dir)
            .map_err(ThemeError::Marker)?
            .ok_or(ThemeError::NotProvisioned)
    }

    /// The account's theme; never chosen reads as Midnight.
    pub fn theme(&self) -> Result<Theme, ThemeError> {
        let id = self.account_for_theme()?;
        let stored = self.settings.get_theme(&id).map_err(ThemeError::Settings)?;
        Ok(stored.unwrap_or_default())
    }

    pub fn set_theme(&self, theme: Theme) -> Result<(), ThemeError> {
        let id = self.account_for_theme()?;
        self.settings
            .set_theme(&id, theme)
            .map_err(ThemeError::Settings)
    }

    /// Assembles My Info with a freshly requested ticket.
    pub fn my_info(&self) -> Result<MyInfo, MyInfoError> {
        let identity = self.load().map_err(MyInfoError::Load)?;
        let display_name = self
            .settings
            .get_display_name(&identity.account_id)
            .map_err(MyInfoError::Settings)?;
        let ticket = self
            .tickets
            .current_ticket(&identity.node_id)
            .map_err(MyInfoError::Ticket)?;
        let qr_svg = ticket_to_qr_svg(&ticket).map_err(MyInfoError::Ticket)?;
        Ok(MyInfo {
            display_name,
            node_id: identity.node_id,
            ticket,
            qr_svg,
        })
    }

    /// Loads the identity key for the transport layer. Crate-private: the key
    /// is converted and dropped inside `core` and never reaches the app.
    pub(crate) fn load_identity_secret(&self) -> Result<(AccountId, SecretKeyBytes), LoadError> {
        let account_id = read_marker(&self.data_dir)
            .map_err(|e| LoadError::Unavailable(e.into()))?
            .ok_or(LoadError::NotProvisioned)?;
        let identity_key = self
            .keychain
            .load_identity_key(&account_id)
            .map_err(|e| LoadError::Unavailable(e.into()))?;
        self.keychain
            .load_db_key(&account_id)
            .map_err(|e| LoadError::Unavailable(e.into()))?;
        Ok((account_id, identity_key))
    }

    /// `Ok(None)` only when the marker is absent; every other failure is an error.
    fn inspect(&self) -> Result<Option<LoadedIdentity>, StatusError> {
        let Some(account_id) = read_marker(&self.data_dir)? else {
            return Ok(None);
        };
        let identity_key = self.keychain.load_identity_key(&account_id)?;
        // Confirms the DB key is present; the key itself is dropped right away.
        self.keychain.load_db_key(&account_id)?;
        Ok(Some(LoadedIdentity {
            account_id,
            node_id: node_id_from_secret_bytes(&identity_key),
        }))
    }
}

fn unix_millis_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use iroh_base::SecretKey;

    use super::*;
    use crate::identity::testing::FakeKeychain;
    use crate::identity::{MemorySettingsStore, NodeIdTicketProvider};

    type TestService = IdentityService<FakeKeychain, MemorySettingsStore, NodeIdTicketProvider>;

    fn service(dir: &tempfile::TempDir) -> TestService {
        IdentityService::new(
            FakeKeychain::new(),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            dir.path(),
        )
    }

    fn provisioned(dir: &tempfile::TempDir) -> (TestService, Provisioned) {
        let svc = service(dir);
        let result = svc.provision().unwrap();
        (svc, result)
    }

    #[test]
    fn provision_makes_identity_present() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(&dir);
        assert_eq!(svc.status(), IdentityStatus::Absent);
        let p = svc.provision().unwrap();
        assert_eq!(
            svc.status(),
            IdentityStatus::Present {
                account_id: p.account_id,
                node_id: p.node_id
            }
        );
        assert!(svc
            .settings
            .get_provisioned_at(&p.account_id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn second_provision_is_rejected_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, first) = provisioned(&dir);
        assert!(matches!(
            svc.provision(),
            Err(ProvisionError::AlreadyProvisioned)
        ));
        assert_eq!(read_marker(dir.path()).unwrap(), Some(first.account_id));
        assert_eq!(svc.load().unwrap().node_id, first.node_id);
    }

    #[test]
    fn marker_with_missing_identity_key_is_error_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, p) = provisioned(&dir);
        svc.keychain.delete_identity_key(&p.account_id).unwrap();
        assert_eq!(
            svc.status(),
            IdentityStatus::Error(StatusError::SecretMissing)
        );
    }

    #[test]
    fn marker_with_missing_db_key_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, p) = provisioned(&dir);
        svc.keychain.delete_db_key(&p.account_id).unwrap();
        assert_eq!(
            svc.status(),
            IdentityStatus::Error(StatusError::SecretMissing)
        );
    }

    #[test]
    fn marker_with_access_denied_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = provisioned(&dir);
        svc.keychain.fail_with(|| KeychainError::AccessDenied);
        assert_eq!(
            svc.status(),
            IdentityStatus::Error(StatusError::AccessDenied)
        );
        assert!(matches!(
            svc.load(),
            Err(LoadError::Unavailable(StatusError::AccessDenied))
        ));
    }

    #[test]
    fn marker_with_backend_failure_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = provisioned(&dir);
        svc.keychain
            .fail_with(|| KeychainError::Backend("boom".into()));
        assert_eq!(
            svc.status(),
            IdentityStatus::Error(StatusError::Backend("boom".into()))
        );
    }

    #[test]
    fn corrupt_marker_is_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("account_id"), "garbage").unwrap();
        let svc = service(&dir);
        assert_eq!(
            svc.status(),
            IdentityStatus::Error(StatusError::MarkerCorrupt)
        );
        assert!(matches!(
            svc.provision(),
            Err(ProvisionError::Unavailable(StatusError::MarkerCorrupt))
        ));
    }

    #[test]
    fn cleared_keychain_never_yields_a_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, p) = provisioned(&dir);
        svc.keychain.delete_identity_key(&p.account_id).unwrap();
        svc.keychain.delete_db_key(&p.account_id).unwrap();
        assert!(matches!(svc.status(), IdentityStatus::Error(_)));
        assert!(matches!(
            svc.provision(),
            Err(ProvisionError::Unavailable(StatusError::SecretMissing))
        ));
        assert_eq!(read_marker(dir.path()).unwrap(), Some(p.account_id));
        assert!(svc.keychain.is_empty());
    }

    /// Delegates to a [`FakeKeychain`] but rejects every DB-key write.
    #[derive(Default)]
    struct FailingDbKeyStore(FakeKeychain);

    impl KeychainStore for FailingDbKeyStore {
        fn store_identity_key(
            &self,
            id: &AccountId,
            key: &SecretKeyBytes,
        ) -> Result<(), KeychainError> {
            self.0.store_identity_key(id, key)
        }

        fn store_db_key(&self, _: &AccountId, _: &DbKey) -> Result<(), KeychainError> {
            Err(KeychainError::Backend("db key write failed".into()))
        }

        fn load_identity_key(&self, id: &AccountId) -> Result<SecretKeyBytes, KeychainError> {
            self.0.load_identity_key(id)
        }

        fn load_db_key(&self, id: &AccountId) -> Result<DbKey, KeychainError> {
            self.0.load_db_key(id)
        }

        fn delete_identity_key(&self, id: &AccountId) -> Result<(), KeychainError> {
            self.0.delete_identity_key(id)
        }

        fn delete_db_key(&self, id: &AccountId) -> Result<(), KeychainError> {
            self.0.delete_db_key(id)
        }
    }

    #[test]
    fn failure_on_second_secret_rolls_back_and_writes_no_marker() {
        let dir = tempfile::tempdir().unwrap();
        let svc = IdentityService::new(
            FailingDbKeyStore::default(),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            dir.path(),
        );
        assert!(matches!(svc.provision(), Err(ProvisionError::Keychain(_))));
        assert!(read_marker(dir.path()).unwrap().is_none());
        assert!(svc.keychain.0.is_empty());
        assert_eq!(svc.status(), IdentityStatus::Absent);
    }

    #[test]
    fn marker_write_failure_rolls_back_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let missing_dir = dir.path().join("does-not-exist");
        let svc = IdentityService::new(
            FakeKeychain::new(),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            missing_dir,
        );
        assert!(matches!(svc.provision(), Err(ProvisionError::Marker(_))));
        assert!(svc.keychain.is_empty());
    }

    #[test]
    fn display_name_rejects_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, p) = provisioned(&dir);
        for bad in [
            "",
            "   ",
            "a\nb",
            "a\u{0007}b",
            "a\u{202E}b",
            &"x".repeat(129),
        ] {
            assert!(
                matches!(svc.set_display_name(bad), Err(SetNameError::Invalid(_))),
                "should reject {bad:?}"
            );
        }
        assert!(svc
            .settings
            .get_display_name(&p.account_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn display_name_is_stored_verbatim_and_overwritable() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, p) = provisioned(&dir);
        for name in ["Ada \u{1F980}", "Grace"] {
            svc.set_display_name(name).unwrap();
            let stored = svc
                .settings
                .get_display_name(&p.account_id)
                .unwrap()
                .unwrap();
            assert_eq!(stored.as_str(), name);
        }
    }

    #[test]
    fn display_name_requires_an_identity() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            service(&dir).set_display_name("Ada"),
            Err(SetNameError::NotProvisioned)
        ));
    }

    #[test]
    fn theme_defaults_to_midnight_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _) = provisioned(&dir);
        assert_eq!(svc.theme().unwrap(), Theme::Midnight);
        for t in [Theme::Light, Theme::Dark, Theme::Midnight] {
            svc.set_theme(t).unwrap();
            assert_eq!(svc.theme().unwrap(), t);
        }
    }

    #[test]
    fn theme_requires_an_identity() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(&dir);
        assert!(matches!(svc.theme(), Err(ThemeError::NotProvisioned)));
        assert!(matches!(
            svc.set_theme(Theme::Dark),
            Err(ThemeError::NotProvisioned)
        ));
    }

    #[test]
    fn my_info_exposes_public_data() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, p) = provisioned(&dir);
        assert!(svc.my_info().unwrap().display_name.is_none());
        svc.set_display_name("Ada").unwrap();
        let info = svc.my_info().unwrap();
        assert_eq!(info.display_name.unwrap().as_str(), "Ada");
        assert_eq!(info.node_id, p.node_id);
        assert!(info.qr_svg.contains("<svg"));
        assert_eq!(
            crate::identity::Ticket::parse(info.ticket.as_str())
                .unwrap()
                .node_id,
            p.node_id
        );
    }

    #[test]
    fn my_info_before_provisioning_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            service(&dir).my_info(),
            Err(MyInfoError::Load(LoadError::NotProvisioned))
        ));
    }

    #[test]
    fn provision_then_load_yields_same_node_id_for_many_keys() {
        for _ in 0..64 {
            let dir = tempfile::tempdir().unwrap();
            let (svc, p) = provisioned(&dir);
            assert_eq!(svc.load().unwrap().node_id, p.node_id);
            let stored = svc.keychain.load_identity_key(&p.account_id).unwrap();
            assert_eq!(SecretKey::from_bytes(stored.as_bytes()).public(), p.node_id);
        }
    }

    const IDENTITY_CANARY: [u8; 32] = [
        0xC1, 0xA2, 0xB3, 0xD4, 0xE5, 0xF6, 0x17, 0x28, 0x39, 0x4A, 0x5B, 0x6C, 0x7D, 0x8E, 0x9F,
        0xA0, 0xB1, 0xC2, 0xD3, 0xE4, 0xF5, 0x06, 0x18, 0x29, 0x3A, 0x4B, 0x5C, 0x6D, 0x7E, 0x8F,
        0x90, 0xA1,
    ];
    const DB_CANARY: [u8; 32] = [
        0x31, 0x42, 0x53, 0x64, 0x75, 0x86, 0x97, 0xA8, 0xB9, 0xCA, 0xDB, 0xEC, 0xFD, 0x0E, 0x1F,
        0x20, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xA9, 0xBA, 0xCB, 0xDC, 0xED, 0xFE, 0x0F,
        0x21, 0x33,
    ];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn no_public_output_contains_key_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(&dir).with_keygen(
            || {
                let key = SecretKey::from_bytes(&IDENTITY_CANARY);
                (SecretKeyBytes::from_bytes(IDENTITY_CANARY), key.public())
            },
            || DbKey::from_bytes(DB_CANARY),
        );
        let provisioned = svc.provision().unwrap();
        svc.set_display_name("Ada").unwrap();
        let info = svc.my_info().unwrap();

        let outputs = [
            format!("{provisioned:?}"),
            format!("{:?}", svc.load().unwrap()),
            format!("{:?}", svc.status()),
            format!("{info:?}"),
            info.ticket.to_string(),
            info.qr_svg.clone(),
            info.node_id.to_string(),
        ];
        for canary in [IDENTITY_CANARY, DB_CANARY] {
            let forms = [hex(&canary), format!("{canary:?}")];
            for output in &outputs {
                for form in &forms {
                    assert!(!output.contains(form.as_str()), "leaked {form} in {output}");
                }
            }
        }
    }
}
