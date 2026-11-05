//! The `identity` slice: secret hygiene, the `account_id` marker, and
//! display-name validation.
//!
//! `AccountId` is a random v4 UUID. It is deliberately unrelated to any key
//! or to the NodeId, so it reveals nothing about the user.

mod display_name;
mod keychain;
mod keygen;
mod marker;
mod secret;
mod service;
mod settings;
mod ticket;

pub use display_name::{validate_display_name, DisplayName, DisplayNameError};
pub use keychain::{KeychainError, KeychainStore, OsKeychain, APP_KEYCHAIN_SERVICE};
pub use marker::{read_marker, write_marker, AccountId, MarkerError};
pub use secret::{DbKey, SecretKeyBytes};
pub use service::{
    IdentityService, IdentityStatus, LoadError, LoadedIdentity, MyInfo, MyInfoError,
    ProvisionError, Provisioned, SetNameError, StatusError,
};
pub use settings::{MemorySettingsStore, SettingsError, SettingsStore};
pub use ticket::{
    ticket_to_qr_svg, NodeId, NodeIdTicketProvider, ParsedTicket, Ticket, TicketError,
    TicketProvider,
};

#[cfg(any(test, feature = "test-support"))]
pub use keychain::testing;
