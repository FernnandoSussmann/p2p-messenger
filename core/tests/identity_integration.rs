//! Two in-process nodes with isolated keychains, settings and marker dirs.

use core::identity::testing::FakeKeychain;
use core::identity::{
    IdentityService, IdentityStatus, MemorySettingsStore, NodeIdTicketProvider, Ticket,
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
