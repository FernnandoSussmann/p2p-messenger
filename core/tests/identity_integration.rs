//! Two in-process nodes with isolated keychains, settings and marker dirs.

use std::sync::Arc;

use core::identity::testing::FakeKeychain;
use core::identity::{
    FileSettingsStore, IdentityService, IdentityStatus, KeychainStore, MemorySettingsStore,
    MyInfoError, NodeIdTicketProvider, ProvisionError, SettingsStore, Ticket,
};

type Node = IdentityService<FakeKeychain, MemorySettingsStore, NodeIdTicketProvider>;

fn node(dir: &tempfile::TempDir) -> Node {
    IdentityService::new(
        FakeKeychain::new(),
        MemorySettingsStore::new(),
        NodeIdTicketProvider,
        dir.path(),
    )
}

#[test]
fn two_nodes_have_distinct_identities_and_parseable_tickets() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (node(&dir_a), node(&dir_b));
    let (prov_a, prov_b) = (a.provision().unwrap(), b.provision().unwrap());

    assert_ne!(prov_a.node_id, prov_b.node_id);
    assert_ne!(prov_a.account_id, prov_b.account_id);
    assert!(matches!(a.status(), IdentityStatus::Present { .. }));

    let ticket_b = b.my_info().unwrap().ticket;
    let parsed = Ticket::parse(ticket_b.as_str()).unwrap();
    assert_eq!(parsed.node_id, prov_b.node_id);
    assert_ne!(parsed.node_id, a.load().unwrap().node_id);
}

fn shared_node(
    keychain: &Arc<FakeKeychain>,
    dir: &tempfile::TempDir,
) -> IdentityService<Arc<FakeKeychain>, MemorySettingsStore, NodeIdTicketProvider> {
    IdentityService::new(
        Arc::clone(keychain),
        MemorySettingsStore::new(),
        NodeIdTicketProvider,
        dir.path(),
    )
}

#[test]
fn each_node_parses_the_others_ticket_and_gets_its_node_id() {
    // Live ticket content (relay and direct addresses) is deferred to DD-02;
    // this test covers the NodeId-only ticket and must be extended there.
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (kc_a, kc_b) = (Arc::new(FakeKeychain::new()), Arc::new(FakeKeychain::new()));
    let (a, b) = (shared_node(&kc_a, &dir_a), shared_node(&kc_b, &dir_b));
    let (pa, pb) = (a.provision().unwrap(), b.provision().unwrap());

    let ticket_a = a.my_info().unwrap().ticket;
    let ticket_b = b.my_info().unwrap().ticket;

    assert_eq!(
        Ticket::parse(ticket_b.as_str()).unwrap().node_id,
        pb.node_id
    );
    assert_eq!(
        Ticket::parse(ticket_a.as_str()).unwrap().node_id,
        pa.node_id
    );
    assert_ne!(pa.node_id, pb.node_id);

    assert_eq!(
        Ticket::parse(ticket_a.as_str()).unwrap().node_id,
        a.load().unwrap().node_id
    );

    let text = ticket_a.as_str();
    let tampered = [
        &text[..text.len() - 1],
        &text[..text.len() / 2],
        &format!("  {text}"),
        &format!(" \t{text}"),
        "",
    ];
    for bad in tampered {
        assert!(
            Ticket::parse(bad).is_err(),
            "accepted tampered ticket {bad:?}"
        );
    }

    let key = kc_a.load_identity_key(&pa.account_id).unwrap();
    let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    assert!(!text.contains(&hex));
    for window in key.as_bytes().windows(8) {
        let h: String = window.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!text.contains(&h));
    }
}

#[test]
fn restart_yields_same_node_id_repeated() {
    for _ in 0..32 {
        let dir = tempfile::tempdir().unwrap();
        let keychain = Arc::new(FakeKeychain::new());

        let first = shared_node(&keychain, &dir);
        let provisioned = first.provision().unwrap();
        drop(first);

        let second = shared_node(&keychain, &dir);
        let loaded = second.load().unwrap();
        assert_eq!(loaded.node_id, provisioned.node_id);
        assert_eq!(loaded.account_id, provisioned.account_id);
        assert_eq!(
            second.status(),
            IdentityStatus::Present {
                account_id: provisioned.account_id,
                node_id: provisioned.node_id,
            }
        );
        assert!(matches!(
            second.provision(),
            Err(ProvisionError::AlreadyProvisioned)
        ));
        assert_eq!(second.load().unwrap().node_id, provisioned.node_id);
    }
}

#[test]
fn display_name_and_provisioned_at_survive_restart() {
    type FileNode = IdentityService<Arc<FakeKeychain>, FileSettingsStore, NodeIdTicketProvider>;
    let build = |keychain: &Arc<FakeKeychain>, dir: &tempfile::TempDir| -> FileNode {
        IdentityService::new(
            Arc::clone(keychain),
            FileSettingsStore::new(dir.path()),
            NodeIdTicketProvider,
            dir.path(),
        )
    };
    let dir = tempfile::tempdir().unwrap();
    let keychain = Arc::new(FakeKeychain::new());

    let first = build(&keychain, &dir);
    let provisioned = first.provision().unwrap();
    first.set_display_name("Ada 🦀").unwrap();
    let before = FileSettingsStore::new(dir.path())
        .get_provisioned_at(&provisioned.account_id)
        .unwrap();
    assert!(before.is_some());
    drop(first);

    let second = build(&keychain, &dir);
    assert_eq!(
        second.status(),
        IdentityStatus::Present {
            account_id: provisioned.account_id,
            node_id: provisioned.node_id,
        }
    );
    let name = second.my_info().unwrap().display_name;
    assert_eq!(name.as_ref().map(|n| n.as_str()), Some("Ada 🦀"));
    let after = FileSettingsStore::new(dir.path())
        .get_provisioned_at(&provisioned.account_id)
        .unwrap();
    assert_eq!(before, after);
    drop(second);

    std::fs::write(dir.path().join("settings.json"), b"not json {{{").unwrap();
    let third = build(&keychain, &dir);
    assert!(matches!(third.my_info(), Err(MyInfoError::Settings(_))));
    assert!(matches!(third.status(), IdentityStatus::Present { .. }));
}
