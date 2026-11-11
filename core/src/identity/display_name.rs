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
//! 6. Invisible format characters are rejected
//!    ([`DisplayNameError::InvisibleCharacter`]): Unicode category Cf plus
//!    U+180E (soft hyphen, ZWSP/ZWNJ/ZWJ, LRM/RLM, ALM, word joiner and
//!    invisible operators, BOM, tag characters, ...). Two well-formed emoji
//!    constructs are exempt: a ZWJ between two `Extended_Pictographic`
//!    characters (the base before it may carry one U+FE0F or one skin-tone
//!    modifier), and a tag sequence U+1F3F4, one or more tag characters
//!    U+E0020-U+E007E, then U+E007F.
//!
//! Everything else, including emoji and other multi-byte UTF-8, is accepted
//! verbatim. Accepted by design: well-formed emoji ZWJ sequences and
//! subdivision flags. Only *structure* is checked, not membership in the RGI
//! emoji set or the ISO 3166-2 list, so two arbitrary emoji joined by a ZWJ
//! are accepted. Not handled: homoglyphs, visually blank non-Cf characters
//! such as U+3164 HANGUL FILLER, and Unicode normalization.

use super::emoji::is_extended_pictographic;
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
    /// Contains an invisible format character outside a well-formed emoji sequence.
    InvisibleCharacter,
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
            Self::InvisibleCharacter => {
                write!(f, "display name must not contain invisible characters")
            }
        }
    }
}

impl std::error::Error for DisplayNameError {}

/// The exact bidi set rejected: U+202A-U+202E and U+2066-U+2069.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Unicode category Cf plus U+180E, minus the bidi set (reported separately).
fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{0890}'..='\u{0891}' | '\u{08E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{2060}'..='\u{2064}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}'
        | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}' | '\u{E0001}'
        | '\u{E0020}'..='\u{E007F}')
}

const ZWJ: char = '\u{200D}';
const BLACK_FLAG: char = '\u{1F3F4}';
const CANCEL_TAG: char = '\u{E007F}';

fn is_tag_char(c: char) -> bool {
    matches!(c, '\u{E0020}'..='\u{E007E}')
}

/// Scans for invisible characters; `Ok` when every one sits in an allowed
/// emoji construct.
fn check_invisible(s: &str) -> Result<(), DisplayNameError> {
    let bad = Err(DisplayNameError::InvisibleCharacter);
    let mut prev: Option<char> = None;
    let mut prev2: Option<char> = None;
    // True while inside a tag run that started right after a black flag.
    let mut in_tags = false;
    let mut tags_seen = false;
    let mut iter = s.chars().peekable();
    while let Some(c) = iter.next() {
        if in_tags {
            if is_tag_char(c) {
                tags_seen = true;
            } else if c == CANCEL_TAG && tags_seen {
                in_tags = false;
            } else {
                return bad;
            }
        } else if is_tag_char(c) {
            if prev != Some(BLACK_FLAG) {
                return bad;
            }
            in_tags = true;
            tags_seen = true;
        } else if c == ZWJ {
            let base = match prev {
                Some('\u{FE0F}' | '\u{1F3FB}'..='\u{1F3FF}') => prev2,
                other => other,
            };
            let ok = base.is_some_and(is_extended_pictographic)
                && iter.peek().copied().is_some_and(is_extended_pictographic);
            if !ok {
                return bad;
            }
        } else if is_invisible_format(c) {
            return bad;
        }
        prev2 = prev;
        prev = Some(c);
    }
    if in_tags {
        return bad;
    }
    Ok(())
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
    check_invisible(trimmed)?;
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
            ("a\u{200B}b", DisplayNameError::InvisibleCharacter),
            ("a\u{200C}b", DisplayNameError::InvisibleCharacter),
            ("a\u{200E}b", DisplayNameError::InvisibleCharacter),
            ("a\u{200F}b", DisplayNameError::InvisibleCharacter),
            ("a\u{FEFF}b", DisplayNameError::InvisibleCharacter),
            ("a\u{2060}b", DisplayNameError::InvisibleCharacter),
            ("a\u{2064}b", DisplayNameError::InvisibleCharacter),
            ("a\u{206A}b", DisplayNameError::InvisibleCharacter),
            ("a\u{180E}b", DisplayNameError::InvisibleCharacter),
            ("a\u{00AD}b", DisplayNameError::InvisibleCharacter),
            ("a\u{061C}b", DisplayNameError::InvisibleCharacter),
            ("a\u{0600}b", DisplayNameError::InvisibleCharacter),
            ("a\u{06DD}b", DisplayNameError::InvisibleCharacter),
            ("a\u{070F}b", DisplayNameError::InvisibleCharacter),
            ("a\u{0890}b", DisplayNameError::InvisibleCharacter),
            ("a\u{08E2}b", DisplayNameError::InvisibleCharacter),
            ("a\u{FFF9}b", DisplayNameError::InvisibleCharacter),
            ("a\u{110BD}b", DisplayNameError::InvisibleCharacter),
            ("a\u{13430}b", DisplayNameError::InvisibleCharacter),
            ("a\u{1BCA0}b", DisplayNameError::InvisibleCharacter),
            ("a\u{1D173}b", DisplayNameError::InvisibleCharacter),
            ("a\u{E0001}b", DisplayNameError::InvisibleCharacter),
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
            ("❤\u{FE0F}", "❤\u{FE0F}"),
            ("e\u{0301}", "e\u{0301}"),
            ("日本語", "日本語"),
            ("👋", "👋"),
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
            DisplayNameError::InvisibleCharacter,
        ];
        let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        for (i, m) in messages.iter().enumerate() {
            assert!(!m.is_empty());
            assert!(!messages[..i].contains(m), "duplicate message: {m}");
        }
        let _: &dyn std::error::Error = &DisplayNameError::Empty;
    }

    fn assert_all(inputs: &[&str], expected: Result<(), DisplayNameError>) {
        for input in inputs {
            assert_eq!(
                validate_display_name(input).map(|_| ()),
                expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn zwj_outside_emoji_is_rejected() {
        assert_all(
            &[
                "a\u{200D}b",
                "\u{200D}x",
                "😀\u{200D}",
                "😀\u{200D}\u{200D}😀",
                "a\u{200D}😀",
                "😀\u{200D}a",
                "😀\u{FE0F}\u{FE0F}\u{200D}😀",
            ],
            Err(DisplayNameError::InvisibleCharacter),
        );
    }

    #[test]
    fn malformed_tag_sequences_are_rejected() {
        assert_all(
            &[
                "\u{E0041}",
                "a\u{E0041}b",
                "\u{1F3F4}\u{E0067}\u{E0062}",
                "😀\u{E0067}\u{E007F}",
                "\u{1F3F4}\u{E007F}",
                "a\u{E007F}",
            ],
            Err(DisplayNameError::InvisibleCharacter),
        );
    }

    #[test]
    fn well_formed_emoji_sequences_are_accepted() {
        let inputs = [
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}",
            "\u{1F44D}\u{1F3FD}",
            "\u{1F9D1}\u{1F3FD}\u{200D}\u{1F4BB}",
            "Ana \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        ];
        for input in inputs {
            assert_eq!(
                validate_display_name(input).unwrap().as_str(),
                input,
                "input: {input:?}"
            );
        }
    }
}
