//! The `identity` slice: secret hygiene, the `account_id` marker, and
//! display-name validation.
//!
//! `AccountId` is a random v4 UUID. It is deliberately unrelated to any key
//! or to the NodeId, so it reveals nothing about the user.

mod display_name;
mod emoji;
mod keychain;
mod keygen;
mod marker;
mod profile;
mod secret;
mod service;
mod settings;
mod settings_sql;
mod theme;
mod ticket;

pub(crate) use display_name::{is_bidi_control, is_disallowed_control, is_invisible_break};
pub use display_name::{validate_display_name, DisplayName, DisplayNameError};
pub use keychain::{KeychainError, KeychainStore, OsKeychain, APP_KEYCHAIN_SERVICE};
pub use marker::{read_marker, write_marker, AccountId, MarkerError};
pub use profile::{Profile, ProfileError};
pub use secret::{DbKey, SecretKeyBytes};
pub use service::{
    IdentityService, IdentityStatus, LoadError, LoadedIdentity, MyInfo, MyInfoError,
    ProvisionError, Provisioned, SetNameError, StatusError, ThemeError,
};
pub(crate) use settings::{read_legacy_settings, LegacySettings};
pub use settings::{validate_relay_url, MemorySettingsStore, SettingsError, SettingsStore};
pub use settings_sql::SqlCipherSettingsStore;
pub(crate) use settings_sql::{
    insert_settings_if_absent_tx, MIGRATION_SETTINGS, MIGRATION_SETTINGS_THEME,
    SQL_SETTINGS_COUNT_OTHERS,
};
pub use theme::{InvalidTheme, Theme};
pub use ticket::{
    ticket_to_qr_svg, NodeId, NodeIdTicketProvider, ParsedTicket, Ticket, TicketError,
    TicketProvider,
};

#[cfg(any(test, feature = "test-support"))]
pub use keychain::testing;
