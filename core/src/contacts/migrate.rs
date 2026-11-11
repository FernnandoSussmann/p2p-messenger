//! One-shot import of the DD-02 interim `known_peers.json` into contacts.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;
use uuid::Uuid;

use super::model::{Contact, Nickname, TrustState};
use super::store::{ContactStore, ContactStoreError};
use crate::identity::{validate_display_name, AccountId, DisplayName, DisplayNameError, NodeId};

const LEGACY_FILE_NAME: &str = "known_peers.json";
const LEGACY_VERSION: u32 = 1;
const MAX_LEGACY_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub migrated: usize,
    pub already_present: usize,
}

#[derive(Debug)]
pub enum MigrationError {
    LegacyCorrupt,
    Store(ContactStoreError),
    Io(io::Error),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LegacyCorrupt => f.write_str("legacy known peers file is corrupt"),
            Self::Store(e) => write!(f, "contact store error during migration: {e}"),
            Self::Io(e) => write!(f, "legacy known peers I/O error: {e}"),
        }
    }
}

impl std::error::Error for MigrationError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPeer {
    name: Option<String>,
    added_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDocument {
    version: u32,
    accounts: BTreeMap<String, BTreeMap<String, LegacyPeer>>,
}

fn legacy_name(raw: Option<&str>) -> Result<Option<DisplayName>, MigrationError> {
    match raw {
        None => Ok(None),
        Some(raw) => match validate_display_name(raw) {
            Ok(name) => Ok(Some(name)),
            Err(DisplayNameError::InvisibleCharacter) => Ok(None),
            Err(_) => Err(MigrationError::LegacyCorrupt),
        },
    }
}

fn read_legacy(path: &Path) -> Result<Option<Vec<(AccountId, Contact)>>, MigrationError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(MigrationError::Io(e)),
    };
    if !meta.file_type().is_file() || meta.len() > MAX_LEGACY_BYTES {
        return Err(MigrationError::LegacyCorrupt);
    }
    let bytes = fs::read(path).map_err(MigrationError::Io)?;
    if bytes.len() as u64 > MAX_LEGACY_BYTES {
        return Err(MigrationError::LegacyCorrupt);
    }
    let doc: LegacyDocument =
        serde_json::from_slice(&bytes).map_err(|_| MigrationError::LegacyCorrupt)?;
    if doc.version != LEGACY_VERSION {
        return Err(MigrationError::LegacyCorrupt);
    }
    let mut entries = Vec::new();
    for (account, peers) in doc.accounts {
        let account = AccountId::from_uuid(
            Uuid::parse_str(&account).map_err(|_| MigrationError::LegacyCorrupt)?,
        );
        for (key, peer) in peers {
            let node_id: NodeId = key.parse().map_err(|_| MigrationError::LegacyCorrupt)?;
            if peer.added_at_ms < 0 {
                return Err(MigrationError::LegacyCorrupt);
            }
            let self_name = legacy_name(peer.name.as_deref())?;
            entries.push((
                account,
                Contact {
                    node_id,
                    nickname: Nickname::default_for(&node_id, self_name.as_ref()),
                    self_name,
                    trust: TrustState::Unverified,
                    added_at_ms: peer.added_at_ms,
                    last_seen_ms: None,
                },
            ));
        }
    }
    Ok(Some(entries))
}

/// Imports `<data_dir>/known_peers.json` (DD-02 interim, v1) for every account
/// in it, in one atomic store write, then deletes the legacy file.
///
/// All or nothing: a corrupt legacy file is left untouched and nothing is
/// migrated; a failed store write keeps the legacy file so the next start
/// retries (idempotent, existing contacts are never overwritten). If migration
/// fails those peers are prompted, never silently admitted.
pub fn migrate_legacy_known_peers(
    data_dir: &Path,
    store: &dyn ContactStore,
) -> Result<MigrationReport, MigrationError> {
    let path = data_dir.join(LEGACY_FILE_NAME);
    let Some(entries) = read_legacy(&path)? else {
        return Ok(MigrationReport::default());
    };
    let report = store
        .insert_missing(entries)
        .map_err(MigrationError::Store)?;
    fs::remove_file(&path).map_err(MigrationError::Io)?;
    Ok(MigrationReport {
        migrated: report.inserted,
        already_present: report.already_present,
    })
}

#[cfg(test)]
mod tests;
