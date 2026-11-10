//! Developer/test profile override: isolates the data directory and the
//! keychain namespace so several app instances can run on one machine.

use std::fmt;
use std::path::{Path, PathBuf};

use super::APP_KEYCHAIN_SERVICE;

const MAX_PROFILE_LEN: usize = 32;
const RESERVED: &str = "default";

/// Why a profile name was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    Empty,
    TooLong,
    InvalidChar,
    Reserved,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileError::Empty => write!(f, "profile name is empty"),
            ProfileError::TooLong => {
                write!(
                    f,
                    "profile name is longer than {MAX_PROFILE_LEN} characters"
                )
            }
            ProfileError::InvalidChar => write!(
                f,
                "profile name may only contain a-z, 0-9 and '-', and must not start with '-'"
            ),
            ProfileError::Reserved => write!(f, "profile name \"{RESERVED}\" is reserved"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// A validated, lowercase profile name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile(String);

impl Profile {
    /// Accepts `[a-z0-9-]{1,32}` after trimming and ASCII-lowercasing, with no
    /// leading `-`. `default` is reserved for "no override".
    pub fn parse(raw: &str) -> Result<Profile, ProfileError> {
        let name = raw.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err(ProfileError::Empty);
        }
        if name.len() > MAX_PROFILE_LEN {
            return Err(ProfileError::TooLong);
        }
        let valid = name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid || name.starts_with('-') {
            return Err(ProfileError::InvalidChar);
        }
        if name == RESERVED {
            return Err(ProfileError::Reserved);
        }
        Ok(Profile(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn keychain_service(&self) -> String {
        format!("{APP_KEYCHAIN_SERVICE}.profile.{}", self.0)
    }

    pub fn data_dir(&self, base: &Path) -> PathBuf {
        base.join("profiles").join(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_are_accepted_and_lowercased() {
        assert_eq!(Profile::parse("b").unwrap().as_str(), "b");
        assert_eq!(Profile::parse("  Alice-2 ").unwrap().as_str(), "alice-2");
        assert_eq!(Profile::parse(&"a".repeat(32)).unwrap().as_str().len(), 32);
        assert_eq!(Profile::parse("A").unwrap(), Profile::parse("a").unwrap());
    }

    #[test]
    fn invalid_names_are_rejected() {
        assert_eq!(Profile::parse(""), Err(ProfileError::Empty));
        assert_eq!(Profile::parse("   "), Err(ProfileError::Empty));
        assert_eq!(Profile::parse(".."), Err(ProfileError::InvalidChar));
        assert_eq!(Profile::parse("a/b"), Err(ProfileError::InvalidChar));
        assert_eq!(Profile::parse("a\\b"), Err(ProfileError::InvalidChar));
        assert_eq!(Profile::parse("-a"), Err(ProfileError::InvalidChar));
        assert_eq!(Profile::parse(&"a".repeat(33)), Err(ProfileError::TooLong));
        assert_eq!(Profile::parse("défault"), Err(ProfileError::InvalidChar));
        assert_eq!(Profile::parse("default"), Err(ProfileError::Reserved));
        assert_eq!(Profile::parse("DEFAULT"), Err(ProfileError::Reserved));
        assert_eq!(Profile::parse("a b"), Err(ProfileError::InvalidChar));
        assert_eq!(Profile::parse("a\0b"), Err(ProfileError::InvalidChar));
    }

    #[test]
    fn paths_and_services_are_namespaced() {
        let p = Profile::parse("b").unwrap();
        assert_eq!(p.keychain_service(), "com.p2p-messenger.identity.profile.b");
        assert_eq!(p.data_dir(Path::new("/x")), Path::new("/x/profiles/b"));
    }
}
