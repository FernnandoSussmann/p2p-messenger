//! The `contacts` slice: the contact record, DD-03's trust rules and the
//! encrypted contact store.
//!
//! - **Trust is NodeId-only.** A peer is trusted exactly when a contact with
//!   its NodeId exists for the account ([`ContactBook::is_known`]). Names are
//!   never compared, looked up or used for dedup.
//! - **Names are advisory.** A peer's self-asserted name (`self_name`) is
//!   updated from Hello and LAN observations of existing contacts only, and
//!   never changes the nickname or trust. Trust changes only through
//!   [`ContactBook::set_verified`].
//! - **Default nickname is a snapshot.** [`Nickname::default_for`] is taken
//!   once, when a contact is created without a user-typed nickname; later
//!   peer renames update `self_name` only.
//! - **Storage.** Contacts live in the `contacts` table of the SQLCipher
//!   `messenger.db` ([`SqlCipherContactStore`]). Older plaintext
//!   `contacts.json` / `known_peers.json` files are imported once by
//!   `storage::legacy`; this slice only parses them.
//! - **Removal order.** History is erased first, then the contact is removed;
//!   a failed erase keeps the contact (see [`ContactBook::remove`]).

mod accept;
mod book;
#[cfg(test)]
mod contract;
mod dto;
mod history;
mod migrate;
mod model;
mod store;
mod store_sql;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use accept::{accept_incoming, AcceptError, IncomingPrompts};
pub use book::ContactBook;
pub use dto::{ContactDto, RemoveContactDto};
pub use history::{HistoryEraseError, HistoryEraser, NoMessageStore};
pub(crate) use migrate::read_legacy_known_peers;
pub use model::{Contact, ContactError, HistoryChoice, NameUpdate, Nickname, Removal, TrustState};
pub(crate) use store::read_legacy_contacts;
pub use store::{
    BatchReport, ContactStore, ContactStoreError, LAST_SEEN_RESOLUTION_MS, MAX_CONTACTS_PER_ACCOUNT,
};
pub use store_sql::SqlCipherContactStore;
pub(crate) use store_sql::{
    count_tx, insert_if_absent_tx, MIGRATION_CONTACTS, SQL_CONTACTS_COUNT_ALL,
};
