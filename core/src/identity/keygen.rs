//! Key generation for the identity slice.
//!
//! Both keys come from the OS CSPRNG and are generated independently: the DB
//! key is never derived from the identity key, so compromising one does not
//! expose the other.

use iroh_base::SecretKey;
use zeroize::Zeroize;

use super::{DbKey, NodeId, SecretKeyBytes};

/// Generates a fresh iroh identity key and its public NodeId.
pub(super) fn generate_identity_key() -> (SecretKeyBytes, NodeId) {
    let key = SecretKey::generate();
    let node_id = key.public();
    let mut bytes = key.to_bytes();
    let secret = SecretKeyBytes::from_bytes(bytes);
    bytes.zeroize();
    (secret, node_id)
}

/// Derives the NodeId matching a stored identity key.
pub(super) fn node_id_from_secret_bytes(bytes: &SecretKeyBytes) -> NodeId {
    SecretKey::from_bytes(bytes.as_bytes()).public()
}

/// Generates a fresh random 256-bit database key.
pub(super) fn generate_db_key() -> DbKey {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    let key = DbKey::from_bytes(bytes);
    bytes.zeroize();
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_matches_generated_secret() {
        let (secret, node_id) = generate_identity_key();
        assert_eq!(node_id_from_secret_bytes(&secret), node_id);
    }

    #[test]
    fn keys_are_random_and_independent() {
        let (a, _) = generate_identity_key();
        let (b, _) = generate_identity_key();
        let db = generate_db_key();
        assert_ne!(a.as_bytes(), b.as_bytes());
        assert_ne!(a.as_bytes(), db.as_bytes());
        assert_ne!(generate_db_key().as_bytes(), db.as_bytes());
    }
}
