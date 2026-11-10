//! Shared test helpers: checks that serialized DTOs never embed raw key bytes.

pub fn assert_forms_absent(json: &str, key: &[u8; 32]) {
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let decimal = format!("{key:?}");
    let decimal_compact = decimal.replace(' ', "");
    assert!(!json.to_lowercase().contains(&hex));
    assert!(!json.contains(&decimal));
    assert!(!json.contains(&decimal_compact));
    assert!(!json.contains(&base64_standard(key)));
}

pub fn base64_standard(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
