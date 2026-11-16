use messenger_core::messaging::MAX_BODY_BYTES;

#[test]
fn frontend_body_limit_matches_protocol() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/chat.ts");
    let src = std::fs::read_to_string(path).expect("read chat.ts");
    let expected = format!("MAX_BODY_BYTES = {MAX_BODY_BYTES}");
    assert!(src.contains(&expected), "chat.ts must contain `{expected}`");
}
