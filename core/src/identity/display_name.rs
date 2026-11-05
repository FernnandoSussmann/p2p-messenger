//! Display-name validation.
//!
//! A display name is the free-text label a user picks for themselves. It is
//! shown to peers, so validation guards against injection and spoofing, not
//! just bad UX.
//!
//! Rules, applied in this order to the input:
//! 1. Leading/trailing whitespace is trimmed ([`str::trim`], Unicode
//!    `White_Space`). The trimmed string is what is stored.
//! 2. An empty result is rejected ([`DisplayNameError::Empty`]).
//! 3. More than [`MAX_DISPLAY_NAME_CHARS`] Unicode scalar values (code points)
//!    is rejected ([`DisplayNameError::TooLong`]). Code points are counted, not
//!    bytes or grapheme clusters, so a long emoji sequence may count as several.
//! 4. Any character for which [`char::is_control`] is true (Unicode `Cc`:
//!    U+0000-U+001F, U+007F-U+009F) is rejected, which covers `\n`, `\r`, `\t`
//!    ([`DisplayNameError::ControlCharacter`]). Checked on the trimmed string,
//!    so interior control characters are caught.
//! 5. Bidirectional override/embedding/isolate characters are rejected
//!    ([`DisplayNameError::BidiOverride`]); the exact set is
//!    U+202A-U+202E (LRE, RLE, PDF, LRO, RLO) and U+2066-U+2069
//!    (LRI, RLI, FSI, PDI).
//!
//! Everything else, including emoji and other multi-byte UTF-8, is accepted
//! verbatim. Not handled: homoglyphs, other invisible format characters such
//! as U+200B-U+200F, and Unicode normalization.

use std::fmt;

/// Maximum display-name length, in Unicode code points, after trimming.
pub const MAX_DISPLAY_NAME_CHARS: usize = 128;

/// A validated display name. Holds the trimmed string that passed
/// [`validate_display_name`]; can only be built through that function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayName(String);

impl DisplayName {
    /// The validated, trimmed name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a display name was rejected. One variant per failure class so the UI
/// can show a specific message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayNameError {
    /// Empty or whitespace-only after trimming.
    Empty,
    /// Longer than [`MAX_DISPLAY_NAME_CHARS`] code points.
    TooLong,
    /// Contains a control character (`char::is_control`), including newlines and tabs.
    ControlCharacter,
    /// Contains a bidi override/embedding/isolate character (U+202A-U+202E, U+2066-U+2069).
    BidiOverride,
}

impl fmt::Display for DisplayNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "display name must not be empty"),
            Self::TooLong => write!(
                f,
                "display name must be at most {MAX_DISPLAY_NAME_CHARS} characters"
            ),
            Self::ControlCharacter => {
                write!(f, "display name must not contain control characters")
            }
            Self::BidiOverride => write!(
                f,
                "display name must not contain text-direction override characters"
            ),
        }
    }
}

impl std::error::Error for DisplayNameError {}

/// The exact bidi set rejected: U+202A-U+202E and U+2066-U+2069.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Validates `input` against the rules in the module docs and returns the
/// trimmed name on success.
pub fn validate_display_name(input: &str) -> Result<DisplayName, DisplayNameError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DisplayNameError::Empty);
    }
    if trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(DisplayNameError::TooLong);
    }
    for c in trimmed.chars() {
        if c.is_control() {
            return Err(DisplayNameError::ControlCharacter);
        }
        if is_bidi_control(c) {
            return Err(DisplayNameError::BidiOverride);
        }
    }
    Ok(DisplayName(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_inputs() {
        let too_long = "a".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        let too_long_multibyte = "é".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        let cases: Vec<(&str, DisplayNameError)> = vec![
            ("", DisplayNameError::Empty),
            ("   ", DisplayNameError::Empty),
            ("\t\n", DisplayNameError::Empty),
            ("\u{00A0}\u{2003}", DisplayNameError::Empty),
            (&too_long, DisplayNameError::TooLong),
            (&too_long_multibyte, DisplayNameError::TooLong),
            ("a\nb", DisplayNameError::ControlCharacter),
            ("a\rb", DisplayNameError::ControlCharacter),
            ("a\tb", DisplayNameError::ControlCharacter),
            ("a\u{0007}b", DisplayNameError::ControlCharacter),
            ("a\u{0000}b", DisplayNameError::ControlCharacter),
            ("a\u{007F}b", DisplayNameError::ControlCharacter),
            ("a\u{0085}b", DisplayNameError::ControlCharacter),
            ("a\u{202A}b", DisplayNameError::BidiOverride),
            ("a\u{202B}b", DisplayNameError::BidiOverride),
            ("a\u{202C}b", DisplayNameError::BidiOverride),
            ("a\u{202D}b", DisplayNameError::BidiOverride),
            ("a\u{202E}b", DisplayNameError::BidiOverride),
            ("a\u{2066}b", DisplayNameError::BidiOverride),
            ("a\u{2067}b", DisplayNameError::BidiOverride),
            ("a\u{2068}b", DisplayNameError::BidiOverride),
            ("a\u{2069}b", DisplayNameError::BidiOverride),
        ];
        for (input, expected) in cases {
            assert_eq!(
                validate_display_name(input),
                Err(expected),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn accepted_inputs_are_trimmed_and_verbatim() {
        let at_bound = "a".repeat(MAX_DISPLAY_NAME_CHARS);
        let at_bound_multibyte = "👋".repeat(MAX_DISPLAY_NAME_CHARS);
        let padded_at_bound = format!("  {at_bound}  ");
        let cases: Vec<(&str, &str)> = vec![
            ("Alice", "Alice"),
            ("  Bob  ", "Bob"),
            ("héllo 👋 world", "héllo 👋 world"),
            ("日本語の名前", "日本語の名前"),
            ("a b", "a b"),
            ("\u{202F}Zoe\u{3000}", "Zoe"),
            (&at_bound, &at_bound),
            (&at_bound_multibyte, &at_bound_multibyte),
            (&padded_at_bound, &at_bound),
        ];
        for (input, expected) in cases {
            let name = validate_display_name(input)
                .unwrap_or_else(|e| panic!("input {input:?} rejected: {e}"));
            assert_eq!(name.as_str(), expected, "input: {input:?}");
        }
    }

    #[test]
    fn error_messages_are_distinct_and_nonempty() {
        let errors = [
            DisplayNameError::Empty,
            DisplayNameError::TooLong,
            DisplayNameError::ControlCharacter,
            DisplayNameError::BidiOverride,
        ];
        let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        for (i, m) in messages.iter().enumerate() {
            assert!(!m.is_empty());
            assert!(!messages[..i].contains(m), "duplicate message: {m}");
        }
        let _: &dyn std::error::Error = &DisplayNameError::Empty;
    }
}
