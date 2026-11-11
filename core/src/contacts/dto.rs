//! String-only DTOs for the Tauri shell.

use serde::Serialize;

use super::model::{Contact, Removal, TrustState};
use crate::identity::NodeId;

#[derive(Debug, Serialize)]
pub struct ContactDto {
    pub node_id: String,
    pub nickname: String,
    /// Not rendered in the contact list (user decision). Carried for the
    /// naming prompt and a future contact-info screen.
    pub self_name: Option<String>,
    pub verified: bool,
    pub added_at_ms: i64,
    pub last_seen_ms: Option<i64>,
}

impl From<&Contact> for ContactDto {
    fn from(c: &Contact) -> Self {
        Self {
            node_id: c.node_id.to_string(),
            nickname: c.nickname.as_str().to_owned(),
            self_name: c.self_name.as_ref().map(|n| n.as_str().to_owned()),
            verified: c.trust == TrustState::Verified,
            added_at_ms: c.added_at_ms,
            last_seen_ms: c.last_seen_ms,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RemoveContactDto {
    pub node_id: String,
    pub history_erased: bool,
}

impl RemoveContactDto {
    pub fn new(id: &NodeId, removal: Removal) -> Self {
        Self {
            node_id: id.to_string(),
            history_erased: removal.history_erased,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::Nickname;

    #[test]
    fn contact_dto_json_shape() {
        let id = iroh_base::SecretKey::from_bytes(&[1; 32]).public();
        let contact = Contact {
            node_id: id,
            nickname: Nickname::parse("Bobby").unwrap(),
            self_name: None,
            trust: TrustState::Verified,
            added_at_ms: 5,
            last_seen_ms: None,
        };
        let json = serde_json::to_value(ContactDto::from(&contact)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"node_id": id.to_string(), "nickname": "Bobby",
                "self_name": null, "verified": true, "added_at_ms": 5, "last_seen_ms": null})
        );
        let removed = serde_json::to_value(RemoveContactDto::new(
            &id,
            Removal {
                history_erased: true,
            },
        ))
        .unwrap();
        assert_eq!(removed["history_erased"], true);
    }
}
