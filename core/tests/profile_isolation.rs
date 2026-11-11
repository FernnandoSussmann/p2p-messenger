//! Two profiles on one machine: separate keychain namespaces and data dirs.
//!
//! `keyring_core::mock` keys entries by (service, user), so same-account
//! entries under different services are independent here as on real stores.

use std::path::Path;
use std::sync::Once;

use core::identity::{
    AccountId, DbKey, FileSettingsStore, IdentityService, IdentityStatus, KeychainError,
    KeychainStore, NodeIdTicketProvider, OsKeychain, Profile, APP_KEYCHAIN_SERVICE,
};

fn install_mock_store() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        keyring_core::set_default_store(keyring_core::mock::Store::new().expect("mock store"));
    });
}

fn profile(name: &str) -> Profile {
    Profile::parse(name).unwrap()
}

#[test]
fn same_account_id_is_independent_across_profiles() {
    install_mock_store();
    let (a, b) = (profile("a"), profile("b"));
    let kc_a = OsKeychain::native_for(Some(&a)).unwrap();
    let kc_b = OsKeychain::native_for(Some(&b)).unwrap();
    let id = AccountId::new();

    kc_a.store_db_key(&id, &DbKey::from_bytes([1; 32])).unwrap();
    assert!(matches!(
        kc_b.load_db_key(&id),
        Err(KeychainError::NotFound)
    ));
    kc_b.store_db_key(&id, &DbKey::from_bytes([2; 32])).unwrap();

    assert_eq!(kc_a.load_db_key(&id).unwrap().as_bytes(), &[1; 32]);
    assert_eq!(kc_b.load_db_key(&id).unwrap().as_bytes(), &[2; 32]);

    kc_a.delete_db_key(&id).unwrap();
    assert!(matches!(
        kc_a.load_db_key(&id),
        Err(KeychainError::NotFound)
    ));
    assert_eq!(kc_b.load_db_key(&id).unwrap().as_bytes(), &[2; 32]);
}

#[test]
fn native_for_none_uses_the_legacy_service() {
    install_mock_store();
    let id = AccountId::new();
    OsKeychain::native_for(None)
        .unwrap()
        .store_db_key(&id, &DbKey::from_bytes([7; 32]))
        .unwrap();

    let legacy =
        keyring_core::Entry::new(APP_KEYCHAIN_SERVICE, &format!("{}/db-key", id.as_uuid()))
            .unwrap();
    assert_eq!(legacy.get_secret().unwrap(), vec![7; 32]);
    assert_eq!(
        OsKeychain::native()
            .unwrap()
            .load_db_key(&id)
            .unwrap()
            .as_bytes(),
        &[7; 32]
    );
}

#[test]
fn profiles_provision_distinct_identities_in_separate_data_dirs() {
    install_mock_store();
    let base = tempfile::tempdir().unwrap();
    let service = |p: &Profile| {
        let dir = p.data_dir(base.path());
        std::fs::create_dir_all(&dir).unwrap();
        let svc = IdentityService::new(
            OsKeychain::native_for(Some(p)).unwrap(),
            FileSettingsStore::new(dir.clone()),
            NodeIdTicketProvider,
            dir.clone(),
        );
        (svc, dir)
    };
    let (a, b) = (profile("a"), profile("b"));
    let ((svc_a, dir_a), (svc_b, dir_b)) = (service(&a), service(&b));

    let (pa, pb) = (svc_a.provision().unwrap(), svc_b.provision().unwrap());
    assert_ne!(pa.account_id, pb.account_id);
    assert_ne!(pa.node_id, pb.node_id);
    assert_ne!(dir_a, dir_b);

    let contents = |dir: &Path| -> String {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (id_a, id_b) = (
        pa.account_id.as_uuid().to_string(),
        pb.account_id.as_uuid().to_string(),
    );
    let (a_files, b_files) = (contents(&dir_a), contents(&dir_b));
    assert!(a_files.contains(&id_a) && !a_files.contains(&id_b));
    assert!(b_files.contains(&id_b) && !b_files.contains(&id_a));

    assert!(matches!(svc_a.status(), IdentityStatus::Present { .. }));
    assert!(matches!(svc_b.status(), IdentityStatus::Present { .. }));
}

#[test]
fn profile_services_never_collide_with_the_default() {
    let svc = profile("a").keychain_service();
    assert_ne!(svc, APP_KEYCHAIN_SERVICE);
    assert_ne!(svc, profile("b").keychain_service());
}
