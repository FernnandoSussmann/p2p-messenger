//! Parser of the DD-02 interim `known_peers.json`. The one-shot import into
//! contacts lives in `storage::legacy`.
//!
//! A corrupt `known_peers.json` is skipped by the import and **stays on disk
//! as plaintext**; those peers are prompted again. This is deliberate and
//! audited in 05-03.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use uuid::Uuid;

use super::model::{Contact, Nickname, TrustState};
use crate::identity::{validate_display_name, AccountId, DisplayName, DisplayNameError, NodeId};
use crate::storage::LegacyError;

const LEGACY_FILE_NAME: &str = "known_peers.json";
const LEGACY_VERSION: u32 = 1;
const MAX_LEGACY_BYTES: u64 = 4 * 1024 * 1024;

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

fn legacy_name(raw: Option<&str>) -> Result<Option<DisplayName>, LegacyError> {
    match raw {
        None => Ok(None),
        Some(raw) => match validate_display_name(raw) {
            Ok(name) => Ok(Some(name)),
            Err(DisplayNameError::InvisibleCharacter) => Ok(None),
            Err(_) => Err(LegacyError::Corrupt),
        },
    }
}

/// Parses `<data_dir>/known_peers.json`. Never opens the database and never
/// writes. `Ok(None)` means the file is absent. Every entry becomes an
/// `Unverified` contact. A symlink or other non-regular file is
/// `NotRegularFile` and is never followed.
pub(crate) fn read_legacy_known_peers(
    data_dir: &Path,
) -> Result<Option<Vec<(AccountId, Contact)>>, LegacyError> {
    let path = data_dir.join(LEGACY_FILE_NAME);
    let meta = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LegacyError::Io),
    };
    if !meta.file_type().is_file() {
        return Err(LegacyError::NotRegularFile);
    }
    if meta.len() > MAX_LEGACY_BYTES {
        return Err(LegacyError::Corrupt);
    }
    let bytes = fs::read(&path).map_err(|_| LegacyError::Io)?;
    if bytes.len() as u64 > MAX_LEGACY_BYTES {
        return Err(LegacyError::Corrupt);
    }
    let doc: LegacyDocument = serde_json::from_slice(&bytes).map_err(|_| LegacyError::Corrupt)?;
    if doc.version != LEGACY_VERSION {
        return Err(LegacyError::Corrupt);
    }
    let mut entries = Vec::new();
    for (account, peers) in doc.accounts {
        let account =
            AccountId::from_uuid(Uuid::parse_str(&account).map_err(|_| LegacyError::Corrupt)?);
        for (key, peer) in peers {
            let node_id: NodeId = key.parse().map_err(|_| LegacyError::Corrupt)?;
            if peer.added_at_ms < 0 {
                return Err(LegacyError::Corrupt);
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

#[cfg(test)]
mod tests;
