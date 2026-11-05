//! The identity Tauri commands, their DTOs and the typed error.
//!
//! The logic lives in plain functions generic over the service boundaries so it
//! can be tested without a Tauri runtime; the `#[tauri::command]` wrappers only
//! forward the managed state. DTOs carry public data only.

use messenger_core::identity::KeychainStore;
use messenger_core::identity::{
    DisplayNameError, IdentityService, IdentityStatus, KeychainError, LoadError, MarkerError,
    MemorySettingsStore, MyInfoError, NodeIdTicketProvider, OsKeychain, ProvisionError,
    SetNameError, SettingsStore, StatusError, TicketProvider,
};
use serde::Serialize;
use tauri::State;

/// The service as wired by the app.
pub type AppIdentity = IdentityService<OsKeychain, MemorySettingsStore, NodeIdTicketProvider>;

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

/// Which validation rule a display name broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DisplayNameKind {
    Empty,
    TooLong,
    ControlCharacter,
    BidiOverride,
}

impl From<DisplayNameError> for DisplayNameKind {
    fn from(err: DisplayNameError) -> Self {
        match err {
            DisplayNameError::Empty => Self::Empty,
            DisplayNameError::TooLong => Self::TooLong,
            DisplayNameError::ControlCharacter => Self::ControlCharacter,
            DisplayNameError::BidiOverride => Self::BidiOverride,
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

#[tauri::command]
pub fn identity_status(service: State<'_, AppIdentity>) -> StatusDto {
    status(&service)
}

#[tauri::command]
pub fn identity_provision(
    service: State<'_, AppIdentity>,
) -> Result<ProvisionDto, IdentityCmdError> {
    provision(&service)
}

#[tauri::command]
pub fn identity_set_display_name(
    service: State<'_, AppIdentity>,
    name: String,
) -> Result<(), IdentityCmdError> {
    set_display_name(&service, &name)
}

#[tauri::command]
pub fn identity_my_info(service: State<'_, AppIdentity>) -> Result<MyInfoDto, IdentityCmdError> {
    my_info(&service)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use messenger_core::identity::testing::FakeKeychain;

    use super::*;

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

    fn assert_forms_absent(json: &str, key: &[u8; 32]) {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let decimal = format!("{key:?}");
        let decimal_compact = decimal.replace(' ', "");
        assert!(!json.to_lowercase().contains(&hex));
        assert!(!json.contains(&decimal));
        assert!(!json.contains(&decimal_compact));
        assert!(!json.contains(&base64_standard(key)));
    }

    fn base64_standard(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}
