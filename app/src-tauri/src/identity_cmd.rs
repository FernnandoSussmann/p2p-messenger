//! The identity Tauri commands, their DTOs and the typed error.
//!
//! The logic lives in plain functions generic over the service boundaries so it
//! can be tested without a Tauri runtime; the `#[tauri::command]` wrappers only
//! forward the managed state. DTOs carry public data only.

use std::sync::Arc;

use messenger_core::discovery::LiveTicketProvider;
use messenger_core::identity::KeychainStore;
use messenger_core::identity::{
    DisplayNameError, IdentityService, IdentityStatus, KeychainError, LoadError, MarkerError,
    MyInfoError, OsKeychain, ProvisionError, SetNameError, SettingsStore, SqlCipherSettingsStore,
    StatusError, Theme, ThemeError, TicketProvider,
};
use messenger_core::storage::Storage;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::chat_cmd::ChatState;
use crate::net_cmd;
use crate::storage_cmd;

/// The service as wired by the app.
pub type AppIdentity =
    IdentityService<Arc<OsKeychain>, Arc<SqlCipherSettingsStore>, LiveTicketProvider>;

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum StatusDto {
    Present { account_id: String, node_id: String },
    Absent,
    Error { reason: String },
}

#[derive(Debug, Serialize)]
pub struct ProvisionDto {
    pub node_id: String,
    pub account_id: String,
}

#[derive(Debug, Serialize)]
pub struct MyInfoDto {
    pub display_name: Option<String>,
    pub node_id: String,
    pub ticket: String,
    pub qr_svg: String,
}

#[derive(Debug, Serialize)]
pub struct ThemeDto {
    pub theme: &'static str,
}

/// Which validation rule a display name broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DisplayNameKind {
    Empty,
    TooLong,
    ControlCharacter,
    BidiOverride,
    InvisibleCharacter,
}

impl From<DisplayNameError> for DisplayNameKind {
    fn from(err: DisplayNameError) -> Self {
        match err {
            DisplayNameError::Empty => Self::Empty,
            DisplayNameError::TooLong => Self::TooLong,
            DisplayNameError::ControlCharacter => Self::ControlCharacter,
            DisplayNameError::BidiOverride => Self::BidiOverride,
            DisplayNameError::InvisibleCharacter => Self::InvisibleCharacter,
        }
    }
}

/// Webview-facing error. Messages never embed backend or secret details.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum IdentityCmdError {
    AlreadyProvisioned,
    RecoverableErrorState { reason: String },
    KeychainAccessDenied,
    InvalidDisplayName { kind: DisplayNameKind },
    InvalidTheme,
    Internal,
}

fn status_reason(err: &StatusError) -> &'static str {
    match err {
        StatusError::MarkerCorrupt => "The account marker file is corrupt.",
        StatusError::MarkerUnreadable(_) => "The account marker file could not be read.",
        StatusError::SecretMissing => "A secret for this identity is missing from the keychain.",
        StatusError::AccessDenied => "Access to the system keychain was denied.",
        StatusError::Backend(_) => "The system keychain reported an error.",
    }
}

impl From<StatusError> for IdentityCmdError {
    fn from(err: StatusError) -> Self {
        match err {
            StatusError::AccessDenied => Self::KeychainAccessDenied,
            other => Self::RecoverableErrorState {
                reason: status_reason(&other).to_owned(),
            },
        }
    }
}

impl From<ProvisionError> for IdentityCmdError {
    fn from(err: ProvisionError) -> Self {
        match err {
            ProvisionError::AlreadyProvisioned => Self::AlreadyProvisioned,
            ProvisionError::Unavailable(e) => e.into(),
            ProvisionError::Keychain(KeychainError::AccessDenied) => Self::KeychainAccessDenied,
            ProvisionError::Keychain(_)
            | ProvisionError::Settings(_)
            | ProvisionError::Marker(_) => Self::Internal,
        }
    }
}

impl From<SetNameError> for IdentityCmdError {
    fn from(err: SetNameError) -> Self {
        match err {
            SetNameError::Invalid(e) => Self::InvalidDisplayName { kind: e.into() },
            SetNameError::Marker(MarkerError::Corrupt) => StatusError::MarkerCorrupt.into(),
            SetNameError::NotProvisioned | SetNameError::Marker(_) | SetNameError::Settings(_) => {
                Self::Internal
            }
        }
    }
}

impl From<ThemeError> for IdentityCmdError {
    fn from(err: ThemeError) -> Self {
        match err {
            ThemeError::Marker(MarkerError::Corrupt) => StatusError::MarkerCorrupt.into(),
            ThemeError::NotProvisioned | ThemeError::Marker(_) | ThemeError::Settings(_) => {
                Self::Internal
            }
        }
    }
}

impl From<MyInfoError> for IdentityCmdError {
    fn from(err: MyInfoError) -> Self {
        match err {
            MyInfoError::Load(LoadError::Unavailable(e)) => e.into(),
            MyInfoError::Load(LoadError::NotProvisioned)
            | MyInfoError::Settings(_)
            | MyInfoError::Ticket(_) => Self::Internal,
        }
    }
}

pub fn status<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
) -> StatusDto {
    match service.status() {
        IdentityStatus::Absent => StatusDto::Absent,
        IdentityStatus::Present {
            account_id,
            node_id,
        } => StatusDto::Present {
            account_id: account_id.as_uuid().to_string(),
            node_id: node_id.to_string(),
        },
        IdentityStatus::Error(e) => StatusDto::Error {
            reason: status_reason(&e).to_owned(),
        },
    }
}

pub fn provision<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
) -> Result<ProvisionDto, IdentityCmdError> {
    let provisioned = service.provision()?;
    Ok(ProvisionDto {
        node_id: provisioned.node_id.to_string(),
        account_id: provisioned.account_id.as_uuid().to_string(),
    })
}

pub fn set_display_name<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
    name: &str,
) -> Result<(), IdentityCmdError> {
    service.set_display_name(name)?;
    Ok(())
}

pub fn my_info<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
) -> Result<MyInfoDto, IdentityCmdError> {
    let info = service.my_info()?;
    Ok(MyInfoDto {
        display_name: info.display_name.map(|n| n.as_str().to_owned()),
        node_id: info.node_id.to_string(),
        ticket: info.ticket.to_string(),
        qr_svg: info.qr_svg,
    })
}

pub fn theme_get<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
) -> Result<ThemeDto, IdentityCmdError> {
    Ok(ThemeDto {
        theme: service.theme()?.as_str(),
    })
}

/// Parses before touching the service: a bad value changes nothing.
pub fn theme_set<K: KeychainStore, S: SettingsStore, T: TicketProvider>(
    service: &IdentityService<K, S, T>,
    raw: &str,
) -> Result<ThemeDto, IdentityCmdError> {
    let theme = Theme::parse(raw).map_err(|_| IdentityCmdError::InvalidTheme)?;
    service.set_theme(theme)?;
    Ok(ThemeDto {
        theme: theme.as_str(),
    })
}

#[tauri::command]
pub fn identity_status(service: State<'_, AppIdentity>) -> StatusDto {
    status(&service)
}

#[tauri::command]
pub fn identity_provision(
    app: AppHandle,
    service: State<'_, AppIdentity>,
    storage: State<'_, Arc<Storage>>,
    chat: State<'_, Arc<ChatState>>,
) -> Result<ProvisionDto, IdentityCmdError> {
    let provisioned = provision(&service)?;
    // Decide the storage state before the node starts; the wizard then shows a
    // failure through `storage_status`.
    if let IdentityStatus::Present { account_id, .. } = service.status() {
        let _ = storage_cmd::open_for(&storage, &chat, &account_id);
    }
    net_cmd::ensure_started(&app);
    Ok(provisioned)
}

#[tauri::command]
pub fn identity_set_display_name(
    service: State<'_, AppIdentity>,
    net: State<'_, net_cmd::NetState>,
    name: String,
) -> Result<(), IdentityCmdError> {
    net_cmd::set_display_name_and_refresh(&service, &net, &name)
}

#[tauri::command]
pub fn identity_my_info(service: State<'_, AppIdentity>) -> Result<MyInfoDto, IdentityCmdError> {
    my_info(&service)
}

#[tauri::command]
pub fn identity_theme_get(service: State<'_, AppIdentity>) -> Result<ThemeDto, IdentityCmdError> {
    theme_get(&service)
}

/// `theme` arrives as a string so a bad value is the typed `InvalidTheme`,
/// not Tauri's generic argument-deserialisation error.
#[tauri::command]
pub fn identity_theme_set(
    service: State<'_, AppIdentity>,
    theme: String,
) -> Result<ThemeDto, IdentityCmdError> {
    theme_set(&service, &theme)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use messenger_core::identity::testing::FakeKeychain;
    use messenger_core::identity::{MemorySettingsStore, NodeIdTicketProvider};

    use super::*;
    use crate::test_util::assert_forms_absent;

    type TestService =
        IdentityService<Arc<FakeKeychain>, MemorySettingsStore, NodeIdTicketProvider>;

    fn setup() -> (TestService, Arc<FakeKeychain>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(FakeKeychain::new());
        let service = IdentityService::new(
            Arc::clone(&keychain),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            dir.path(),
        );
        (service, keychain, dir)
    }

    #[test]
    fn status_maps_absent_and_present() {
        let (service, _kc, _dir) = setup();
        assert!(matches!(status(&service), StatusDto::Absent));

        let provisioned = provision(&service).unwrap();
        match status(&service) {
            StatusDto::Present {
                account_id,
                node_id,
            } => {
                assert_eq!(account_id, provisioned.account_id);
                assert_eq!(node_id, provisioned.node_id);
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn double_provision_maps_to_already_provisioned() {
        let (service, _kc, _dir) = setup();
        provision(&service).unwrap();
        assert_eq!(
            provision(&service).unwrap_err(),
            IdentityCmdError::AlreadyProvisioned
        );
    }

    #[test]
    fn corrupt_marker_maps_to_recoverable_error_state() {
        let (service, _kc, dir) = setup();
        std::fs::write(dir.path().join("account_id"), "not-a-uuid").unwrap();

        assert!(matches!(status(&service), StatusDto::Error { .. }));
        assert!(matches!(
            provision(&service).unwrap_err(),
            IdentityCmdError::RecoverableErrorState { .. }
        ));
        assert!(matches!(
            my_info(&service).unwrap_err(),
            IdentityCmdError::RecoverableErrorState { .. }
        ));
    }

    #[test]
    fn missing_secret_maps_to_recoverable_error_state() {
        let (service, _kc, dir) = setup();
        provision(&service).unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::copy(
            dir.path().join("account_id"),
            other.path().join("account_id"),
        )
        .unwrap();
        let orphaned = IdentityService::new(
            FakeKeychain::new(),
            MemorySettingsStore::new(),
            NodeIdTicketProvider,
            other.path(),
        );

        assert!(matches!(status(&orphaned), StatusDto::Error { .. }));
        assert!(matches!(
            provision(&orphaned).unwrap_err(),
            IdentityCmdError::RecoverableErrorState { .. }
        ));
        assert!(matches!(
            my_info(&orphaned).unwrap_err(),
            IdentityCmdError::RecoverableErrorState { .. }
        ));
    }

    #[test]
    fn access_denied_maps_to_keychain_access_denied() {
        let (service, kc, _dir) = setup();
        kc.fail_with(|| KeychainError::AccessDenied);
        assert_eq!(
            provision(&service).unwrap_err(),
            IdentityCmdError::KeychainAccessDenied
        );
        assert!(kc.is_empty());

        let (service, kc, _dir) = setup();
        provision(&service).unwrap();
        kc.fail_with(|| KeychainError::AccessDenied);
        assert_eq!(
            my_info(&service).unwrap_err(),
            IdentityCmdError::KeychainAccessDenied
        );
        assert_eq!(
            provision(&service).unwrap_err(),
            IdentityCmdError::KeychainAccessDenied
        );
        assert!(matches!(status(&service), StatusDto::Error { .. }));
    }

    #[test]
    fn other_keychain_failure_during_provision_is_internal_and_rolled_back() {
        let (service, kc, _dir) = setup();
        kc.fail_with(|| KeychainError::Backend("boom".into()));
        assert_eq!(provision(&service).unwrap_err(), IdentityCmdError::Internal);
        assert!(kc.is_empty());
    }

    #[test]
    fn invalid_names_map_to_their_kind() {
        let (service, _kc, _dir) = setup();
        provision(&service).unwrap();
        let cases = [
            ("   ", DisplayNameKind::Empty),
            (&"a".repeat(129), DisplayNameKind::TooLong),
            ("bad\nname", DisplayNameKind::ControlCharacter),
            ("evil\u{202E}name", DisplayNameKind::BidiOverride),
            ("zero\u{200B}width", DisplayNameKind::InvisibleCharacter),
        ];
        for (input, kind) in cases {
            assert_eq!(
                set_display_name(&service, input).unwrap_err(),
                IdentityCmdError::InvalidDisplayName { kind },
                "input {input:?}"
            );
        }
    }

    #[test]
    fn display_name_round_trips_through_my_info() {
        let (service, _kc, _dir) = setup();
        let provisioned = provision(&service).unwrap();
        assert_eq!(my_info(&service).unwrap().display_name, None);

        set_display_name(&service, "  Alice ").unwrap();
        let info = my_info(&service).unwrap();
        assert_eq!(info.display_name.as_deref(), Some("Alice"));
        assert_eq!(info.node_id, provisioned.node_id);
        assert!(info.ticket.contains(&info.node_id));
        assert!(info.qr_svg.contains("<svg"));
    }

    #[test]
    fn theme_defaults_round_trips_and_rejects_bad_values() {
        let (service, _kc, _dir) = setup();
        provision(&service).unwrap();
        assert_eq!(theme_get(&service).unwrap().theme, "midnight");
        assert_eq!(theme_set(&service, "light").unwrap().theme, "light");
        assert_eq!(theme_get(&service).unwrap().theme, "light");
        for bad in [
            "Dark",
            " dark",
            "light ",
            "",
            "system",
            "dark'--",
            "midnight'; DROP TABLE x;--",
        ] {
            assert_eq!(
                theme_set(&service, bad).unwrap_err(),
                IdentityCmdError::InvalidTheme
            );
        }
        assert_eq!(theme_get(&service).unwrap().theme, "light");
        assert_eq!(
            serde_json::to_string(&IdentityCmdError::InvalidTheme).unwrap(),
            r#"{"type":"InvalidTheme"}"#
        );
    }

    #[test]
    fn theme_without_identity_is_internal() {
        let (service, _kc, _dir) = setup();
        assert_eq!(theme_get(&service).unwrap_err(), IdentityCmdError::Internal);
        assert_eq!(
            theme_set(&service, "dark").unwrap_err(),
            IdentityCmdError::Internal
        );
    }

    #[test]
    fn first_run_flow_end_to_end() {
        let (service, _kc, _dir) = setup();
        assert!(matches!(status(&service), StatusDto::Absent));

        let provisioned = provision(&service).unwrap();
        set_display_name(&service, "Alice").unwrap();
        let info = my_info(&service).unwrap();
        assert_eq!(info.display_name.as_deref(), Some("Alice"));
        assert_eq!(info.node_id, provisioned.node_id);
        assert!(info.ticket.contains(&info.node_id));
        assert!(info.qr_svg.contains("<svg"));

        match status(&service) {
            StatusDto::Present { node_id, .. } => assert_eq!(node_id, provisioned.node_id),
            other => panic!("expected Present, got {other:?}"),
        }
        assert_eq!(
            provision(&service).unwrap_err(),
            IdentityCmdError::AlreadyProvisioned
        );
    }

    #[test]
    fn first_run_flow_end_to_end_with_encrypted_settings() {
        use messenger_core::identity::SqlCipherSettingsStore;

        type EncryptedService =
            IdentityService<Arc<FakeKeychain>, SqlCipherSettingsStore, NodeIdTicketProvider>;
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(FakeKeychain::new());
        let build = || -> EncryptedService {
            let storage = Arc::new(Storage::new(dir.path(), keychain.clone()));
            IdentityService::new(
                keychain.clone(),
                SqlCipherSettingsStore::new(storage),
                NodeIdTicketProvider,
                dir.path(),
            )
        };

        let service = build();
        let provisioned = provision(&service).unwrap();
        set_display_name(&service, "Alice").unwrap();
        assert_eq!(
            my_info(&service).unwrap().display_name.as_deref(),
            Some("Alice")
        );
        drop(service);

        let restarted = build();
        match status(&restarted) {
            StatusDto::Present { node_id, .. } => assert_eq!(node_id, provisioned.node_id),
            other => panic!("expected Present, got {other:?}"),
        }
        assert_eq!(
            my_info(&restarted).unwrap().display_name.as_deref(),
            Some("Alice")
        );
        assert!(dir.path().join(messenger_core::storage::DB_FILE).exists());
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn theme_persists_across_rebuilt_encrypted_service() {
        use messenger_core::identity::SqlCipherSettingsStore;

        type EncryptedService =
            IdentityService<Arc<FakeKeychain>, SqlCipherSettingsStore, NodeIdTicketProvider>;
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(FakeKeychain::new());
        let build = || -> EncryptedService {
            let storage = Arc::new(Storage::new(dir.path(), keychain.clone()));
            IdentityService::new(
                keychain.clone(),
                SqlCipherSettingsStore::new(storage),
                NodeIdTicketProvider,
                dir.path(),
            )
        };

        let service = build();
        provision(&service).unwrap();
        assert_eq!(theme_get(&service).unwrap().theme, "midnight");
        assert_eq!(theme_set(&service, "light").unwrap().theme, "light");
        for bad in ["light ", "midnight'; DROP TABLE x;--"] {
            assert_eq!(
                theme_set(&service, bad).unwrap_err(),
                IdentityCmdError::InvalidTheme
            );
        }
        drop(service);

        let restarted = build();
        assert_eq!(theme_get(&restarted).unwrap().theme, "light");
    }

    #[test]
    fn set_display_name_before_provisioning_is_internal() {
        let (service, _kc, _dir) = setup();
        assert_eq!(
            set_display_name(&service, "Alice").unwrap_err(),
            IdentityCmdError::Internal
        );
    }

    #[test]
    fn errors_serialize_with_type_tag() {
        let value = serde_json::to_value(IdentityCmdError::InvalidDisplayName {
            kind: DisplayNameKind::TooLong,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"type": "InvalidDisplayName", "kind": "TooLong"})
        );
        let value = serde_json::to_value(IdentityCmdError::InvalidDisplayName {
            kind: DisplayNameKind::InvisibleCharacter,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"type": "InvalidDisplayName", "kind": "InvisibleCharacter"})
        );
        let value = serde_json::to_value(IdentityCmdError::AlreadyProvisioned).unwrap();
        assert_eq!(value, serde_json::json!({"type": "AlreadyProvisioned"}));
    }

    #[test]
    fn dtos_never_contain_key_material() {
        let (service, kc, _dir) = setup();
        let provisioned = provision(&service).unwrap();
        set_display_name(&service, "Alice").unwrap();
        let account_id = service.load().unwrap().account_id;

        let identity_key = *kc.load_identity_key(&account_id).unwrap().as_bytes();
        let db_key = *kc.load_db_key(&account_id).unwrap().as_bytes();

        let jsons = [
            serde_json::to_string(&status(&service)).unwrap(),
            serde_json::to_string(&provisioned).unwrap(),
            serde_json::to_string(&my_info(&service).unwrap()).unwrap(),
        ];
        for json in &jsons {
            assert!(
                json.contains(&provisioned.node_id) || json.contains("Alice"),
                "{json}"
            );
            for key in [identity_key, db_key] {
                assert_forms_absent(json, &key);
            }
        }
        assert!(jsons[0].contains(&provisioned.node_id));
        assert!(jsons[0].contains(&provisioned.account_id));
        assert!(jsons[1].contains(&provisioned.node_id));
        assert!(jsons[2].contains(&provisioned.node_id));
    }
}
