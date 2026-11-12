//! Contact record and the small value types around it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::store::LAST_SEEN_RESOLUTION_MS;
use crate::identity::{validate_display_name, DisplayName, DisplayNameError, NodeId};

/// What the user calls a contact. DD-01 rules ([`validate_display_name`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nickname(DisplayName);

impl Nickname {
    /// Validates and trims `input` with the shared display-name rules.
    pub fn parse(input: &str) -> Result<Self, DisplayNameError> {
        validate_display_name(input).map(Self)
    }

    /// Default when the user has not typed one (migration, auto-added dialed
    /// peer, headless accept without a name): the peer's validated
    /// self-asserted name, else `Peer <id.fmt_short()>`. A snapshot: never
    /// re-derived later.
    pub fn default_for(id: &NodeId, self_name: Option<&DisplayName>) -> Self {
        match self_name {
            Some(name) => Self(name.clone()),
            // A short id is hex, so this always validates.
            None => Self::parse(&format!("Peer {}", id.fmt_short()))
                .expect("short NodeId label is a valid name"),
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Stored as `"unverified"` / `"verified"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustState {
    Unverified,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// Identity and primary key; immutable.
    pub node_id: NodeId,
    /// User-controlled; the only name shown in the contact list.
    pub nickname: Nickname,
    /// Peer's last-seen self-asserted name (Hello or LAN). Advisory only.
    /// Shown in naming/Accept prompts; kept for a future contact-info screen.
    pub self_name: Option<DisplayName>,
    pub trust: TrustState,
    pub added_at_ms: i64,
    /// Last authorized connection.
    pub last_seen_ms: Option<i64>,
}

/// How an observation changes `self_name`.
#[derive(Debug, Clone, Copy)]
pub enum NameUpdate<'a> {
    Keep,
    Set(Option<&'a DisplayName>),
}

/// Applies an observation to `c` and returns whether anything material
/// changed (so a store write is due). The only place the
/// [`LAST_SEEN_RESOLUTION_MS`] throttle is implemented.
pub(crate) fn apply_observation(
    c: &mut Contact,
    name: NameUpdate<'_>,
    seen_at_ms: Option<i64>,
) -> bool {
    let mut changed = false;
    if let NameUpdate::Set(new) = name {
        if c.self_name.as_ref() != new {
            c.self_name = new.cloned();
            changed = true;
        }
    }
    if let Some(seen) = seen_at_ms {
        let stale = c
            .last_seen_ms
            .is_none_or(|prev| seen.saturating_sub(prev).abs() >= LAST_SEEN_RESOLUTION_MS);
        if stale || changed {
            c.last_seen_ms = Some(seen);
        }
        changed |= stale;
    }
    changed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryChoice {
    #[default]
    Keep,
    Erase,
}

impl HistoryChoice {
    /// `None` and `Some(false)` keep the history.
    pub fn from_flag(flag: Option<bool>) -> Self {
        if flag == Some(true) {
            Self::Erase
        } else {
            Self::Keep
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removal {
    pub history_erased: bool,
}

/// Messages never embed names or paths.
#[derive(Debug)]
pub enum ContactError {
    InvalidNickname(DisplayNameError),
    SelfContact,
    AlreadyExists,
    NotFound,
    Full,
    /// Unreadable / corrupt / IO / blocking-task failure. Never "empty" or "unknown".
    Unavailable,
    /// History deletion failed; the contact was NOT removed.
    HistoryEraseFailed,
}

impl fmt::Display for ContactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNickname(e) => write!(f, "invalid nickname: {e}"),
            Self::SelfContact => f.write_str("cannot add your own node as a contact"),
            Self::AlreadyExists => f.write_str("contact already exists"),
            Self::NotFound => f.write_str("contact not found"),
            Self::Full => f.write_str("contact list is full"),
            Self::Unavailable => f.write_str("contact storage is unavailable"),
            Self::HistoryEraseFailed => {
                f.write_str("could not erase the conversation history; contact kept")
            }
        }
    }
}

impl std::error::Error for ContactError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(seed: u8) -> NodeId {
        iroh_base::SecretKey::from_bytes(&[seed; 32]).public()
    }

    #[test]
    fn nickname_rejects_control_newline_bidi_and_overlength() {
        let long = "a".repeat(129);
        let mut cases: Vec<(String, DisplayNameError)> = vec![
            ("".into(), DisplayNameError::Empty),
            ("   ".into(), DisplayNameError::Empty),
            ("a\nb".into(), DisplayNameError::ControlCharacter),
            ("a\tb".into(), DisplayNameError::ControlCharacter),
            (long, DisplayNameError::TooLong),
        ];
        for c in ('\u{202A}'..='\u{202E}').chain('\u{2066}'..='\u{2069}') {
            cases.push((format!("a{c}b"), DisplayNameError::BidiOverride));
        }
        for (input, expected) in cases {
            assert_eq!(Nickname::parse(&input), Err(expected), "{input:?}");
        }
    }

    #[test]
    fn nickname_is_trimmed_and_verbatim() {
        assert_eq!(Nickname::parse("  Bob  ").unwrap().as_str(), "Bob");
        assert_eq!(Nickname::parse("👋 日本語").unwrap().as_str(), "👋 日本語");
    }

    #[test]
    fn default_for_uses_self_name_else_peer_short_id() {
        let id = node(1);
        let bob = validate_display_name("Bob").unwrap();
        assert_eq!(Nickname::default_for(&id, Some(&bob)).as_str(), "Bob");
        let fallback = Nickname::default_for(&id, None);
        assert_eq!(fallback.as_str(), format!("Peer {}", id.fmt_short()));
        assert_eq!(Nickname::parse(fallback.as_str()).unwrap(), fallback);
    }

    #[test]
    fn history_choice_defaults_to_keep() {
        assert_eq!(HistoryChoice::default(), HistoryChoice::Keep);
        assert_eq!(HistoryChoice::from_flag(None), HistoryChoice::Keep);
        assert_eq!(HistoryChoice::from_flag(Some(false)), HistoryChoice::Keep);
        assert_eq!(HistoryChoice::from_flag(Some(true)), HistoryChoice::Erase);
    }
}
