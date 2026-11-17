//! The UI colour theme: a closed set, the only representation that reaches SQL.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Midnight,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTheme;

impl fmt::Display for InvalidTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid theme")
    }
}

impl std::error::Error for InvalidTheme {}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Midnight, Theme::Dark, Theme::Light];

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Midnight => "midnight",
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    /// Exact lowercase match only: no trimming, no case folding.
    pub fn parse(raw: &str) -> Result<Self, InvalidTheme> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == raw)
            .ok_or(InvalidTheme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_round_trips() {
        for t in Theme::ALL {
            assert_eq!(Theme::parse(t.as_str()), Ok(t));
        }
        assert_eq!(Theme::default(), Theme::Midnight);
    }

    #[test]
    fn anything_else_is_rejected() {
        for bad in ["Dark", " dark", "dark ", "", "system", "midnight'--"] {
            assert_eq!(Theme::parse(bad), Err(InvalidTheme), "{bad:?}");
        }
    }
}
