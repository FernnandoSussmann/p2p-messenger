//! The `contacts` slice: the contact record, DD-03's trust rules and the
//! interim contact store.
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
//! - **Interim storage.** Contacts live in a plaintext `contacts.json` (see
//!   [`store`]), a documented deviation until DD-05's encrypted table.
//! - **Removal order.** History is erased first, then the contact is removed;
//!   a failed erase keeps the contact (see [`ContactBook::remove`]).

mod book;
#[cfg(test)]
mod contract;
mod dto;
mod history;
mod migrate;
mod model;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use book::ContactBook;
pub use dto::{ContactDto, RemoveContactDto};
pub use history::{HistoryEraseError, HistoryEraser, NoMessageStore};
pub use migrate::{migrate_legacy_known_peers, MigrationError, MigrationReport};
pub use model::{Contact, ContactError, HistoryChoice, NameUpdate, Nickname, Removal, TrustState};
pub use store::{
    BatchReport, ContactStore, ContactStoreError, FileContactStore, LAST_SEEN_RESOLUTION_MS,
    MAX_CONTACTS_BYTES, MAX_CONTACTS_PER_ACCOUNT,
};
