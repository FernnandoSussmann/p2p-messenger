//! In-memory secret hygiene types for the identity slice.
//!
//! [`DbKey`] and [`SecretKeyBytes`] are the *serialized* byte forms of the two
//! secrets that cross into and out of the OS keychain. They exist so that raw
//! `[u8; 32]` secrets never travel as bare arrays: both wipe their memory on
//! drop, and neither can leak its bytes through `Debug`, `Display`, or serde.
//!
//! Trade-off: the types are deliberately inert. There is no `Clone`, so a
//! secret cannot be silently duplicated into memory that is never wiped, and
//! callers that need the bytes must borrow them via `as_bytes`.
//!
//! This module never persists anything; storage is the keychain's job.

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

/// The 256-bit key used to encrypt the local SQLCipher database.
///
/// Memory is zeroized on drop. `Debug` prints a redacted placeholder.
///
/// # Security
///
/// Never derive `Serialize`, `Deserialize`, `Display`, or `Clone` on this
/// type: any of them would make it easy to leak or duplicate the key.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DbKey([u8; 32]);

impl DbKey {
    /// Wrap raw key bytes. The caller should zeroize its own copy afterwards.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DbKey(<redacted>)")
    }
}

/// The raw 32-byte ed25519 secret key backing the node's iroh identity.
///
/// Memory is zeroized on drop. `Debug` prints a redacted placeholder.
///
/// # Security
///
/// Never derive `Serialize`, `Deserialize`, `Display`, or `Clone` on this
/// type: any of them would make it easy to leak or duplicate the key.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SecretKeyBytes([u8; 32]);

impl SecretKeyBytes {
    /// Wrap raw key bytes. The caller should zeroize its own copy afterwards.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the raw key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SecretKeyBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKeyBytes(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway in-memory pattern, not a real secret.
    const PATTERN: [u8; 32] = [0xAB; 32];

    #[test]
    fn debug_output_is_redacted() {
        let db = format!("{:?}", DbKey::from_bytes(PATTERN));
        let sk = format!("{:?}", SecretKeyBytes::from_bytes(PATTERN));
        assert_eq!(db, "DbKey(<redacted>)");
        assert_eq!(sk, "SecretKeyBytes(<redacted>)");
    }

    #[test]
    fn round_trip_bytes() {
        assert_eq!(DbKey::from_bytes(PATTERN).as_bytes(), &PATTERN);
        assert_eq!(SecretKeyBytes::from_bytes(PATTERN).as_bytes(), &PATTERN);
    }
}
