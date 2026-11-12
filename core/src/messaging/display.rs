//! Display-safe copies of untrusted chat text.

use crate::identity::{is_bidi_control, is_disallowed_control, is_invisible_break};

/// Display-safe copy of an untrusted chat body. Replaces with U+FFFD only bidi
/// controls, invisible breaks (ZWSP, U+2060-2064, BOM) and disallowed controls
/// (everything `char::is_control` except '\n' and '\t'). Keeps all else,
/// including ZWNJ, ZWJ, LRM, RLM, ALM, soft hyphen, Arabic number signs and
/// emoji sequences. Never trims.
pub fn neutralize_chat_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if is_bidi_control(c) || is_invisible_break(c) || is_disallowed_control(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unchanged(s: &str) {
        assert_eq!(neutralize_chat_text(s), s);
    }

    #[test]
    fn chat_text_keeps_persian_zwnj() {
        unchanged("می\u{200C}خواهم");
    }

    #[test]
    fn chat_text_keeps_hindi_zwj() {
        unchanged("क्\u{200D}ष");
    }

    #[test]
    fn chat_text_keeps_emoji_sequences() {
        unchanged("👨\u{200D}👩\u{200D}👧 🏳\u{FE0F}\u{200D}🌈 🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}");
    }

    #[test]
    fn chat_text_keeps_marks_soft_hyphen_and_arabic_signs() {
        unchanged("a\u{200E}b\u{200F}c\u{061C}d\u{00AD}e\u{0600}f");
    }

    #[test]
    fn chat_text_neutralizes_bidi_controls() {
        assert_eq!(
            neutralize_chat_text("a\u{202E}b\u{2066}c\u{2069}"),
            "a\u{FFFD}b\u{FFFD}c\u{FFFD}"
        );
    }

    #[test]
    fn chat_text_neutralizes_invisible_breaks() {
        assert_eq!(
            neutralize_chat_text("a\u{200B}b\u{2060}c\u{FEFF}"),
            "a\u{FFFD}b\u{FFFD}c\u{FFFD}"
        );
    }

    #[test]
    fn chat_text_neutralizes_controls_but_keeps_newline_and_tab() {
        assert_eq!(
            neutralize_chat_text("a\u{0007}b\r\u{0085}c\n\td"),
            "a\u{FFFD}b\u{FFFD}\u{FFFD}c\n\td"
        );
    }

    #[test]
    fn chat_text_is_identity_on_plain_text() {
        unchanged("  Hello, world! 123 \n\tçñ 你好 😀  ");
    }
}
