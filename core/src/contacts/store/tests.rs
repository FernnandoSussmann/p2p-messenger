use std::path::PathBuf;

use super::*;
use crate::contacts::contract::{contact, name, node};

fn file(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(CONTACTS_FILE_NAME)
}

fn entry(account: &str, id: &str, fields: &str) -> String {
    format!(r#"{{"version":1,"accounts":{{"{account}":{{"{id}":{{{fields}}}}}}}}}"#)
}

const OK_FIELDS: &str =
    r#""nickname":"Bob","self_name":"Robert","trust":"verified","added_at_ms":7,"last_seen_ms":9"#;

#[test]
fn absent_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_legacy_contacts(dir.path()), Ok(None));
}

#[test]
fn valid_document_parses_every_field() {
    let dir = tempfile::tempdir().unwrap();
    let account = AccountId::new();
    let body = entry(
        &account.as_uuid().hyphenated().to_string(),
        &node(1).to_string(),
        OK_FIELDS,
    );
    fs::write(file(&dir), &body).unwrap();
    let parsed = read_legacy_contacts(dir.path()).unwrap().unwrap();
    let mut expected = contact(1, "Bob");
    expected.self_name = Some(name("Robert"));
    expected.trust = TrustState::Verified;
    expected.added_at_ms = 7;
    expected.last_seen_ms = Some(9);
    assert_eq!(parsed, vec![(account, expected)]);
    assert_eq!(fs::read_to_string(file(&dir)).unwrap(), body);
}

#[test]
fn corrupt_files_are_typed_errors_and_untouched() {
    let acct = AccountId::new().as_uuid().hyphenated().to_string();
    let id = node(1).to_string();
    let with = |from: &str, to: &str| entry(&acct, &id, &OK_FIELDS.replace(from, to));
    let long = "a".repeat(129);
    let cases = [
        "not json".to_owned(),
        r#"{"version":2,"accounts":{}}"#.to_owned(),
        r#"{"version":1,"accounts":{},"extra":1}"#.to_owned(),
        entry(&acct, &id, &format!("{OK_FIELDS},\"extra\":1")),
        entry("x", &id, OK_FIELDS),
        entry(&acct.to_uppercase(), &id, OK_FIELDS),
        entry(&acct, "nope", OK_FIELDS),
        entry(&acct, &id.to_uppercase(), OK_FIELDS),
        with(r#""Bob""#, r#""a\nb""#),
        with(r#""Bob""#, "\"a\\u202Eb\""),
        with(r#""Bob""#, "\"a\\u200Bb\""),
        with(r#""Bob""#, &format!(r#""{long}""#)),
        with(r#""Bob""#, r#"" a""#),
        with(r#""Robert""#, r#""\u0007""#),
        with(r#""Robert""#, r#"" x""#),
        with(r#""Robert""#, "\"x\\uFEFF\""),
        with("verified", "trusted"),
        with(r#""added_at_ms":7"#, r#""added_at_ms":-1"#),
        with(r#""last_seen_ms":9"#, r#""last_seen_ms":-5"#),
    ];
    for body in cases {
        let dir = tempfile::tempdir().unwrap();
        fs::write(file(&dir), &body).unwrap();
        assert_eq!(
            read_legacy_contacts(dir.path()),
            Err(LegacyError::Corrupt),
            "{body}"
        );
        assert_eq!(fs::read_to_string(file(&dir)).unwrap(), body);
    }
}

#[test]
fn over_the_cap_per_account_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let acct = AccountId::new().as_uuid().hyphenated().to_string();
    let contacts: BTreeMap<String, serde_json::Value> = (0..=MAX_CONTACTS_PER_ACCOUNT as u32)
        .map(|i| {
            let mut seed = [7u8; 32];
            seed[..4].copy_from_slice(&i.to_le_bytes());
            let id = iroh_base::SecretKey::from_bytes(&seed).public().to_string();
            (
                id,
                serde_json::json!({"nickname":"n","self_name":null,"trust":"unverified","added_at_ms":0,"last_seen_ms":null}),
            )
        })
        .collect();
    let doc = serde_json::json!({"version":1,"accounts":{acct: contacts}});
    fs::write(file(&dir), doc.to_string()).unwrap();
    assert_eq!(read_legacy_contacts(dir.path()), Err(LegacyError::Corrupt));
}

#[test]
fn oversized_file_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        file(&dir),
        vec![b' '; MAX_LEGACY_CONTACTS_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(read_legacy_contacts(dir.path()), Err(LegacyError::Corrupt));
}

#[cfg(unix)]
#[test]
fn symlinked_file_is_not_regular() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("elsewhere.json");
    fs::write(&target, r#"{"version":1,"accounts":{}}"#).unwrap();
    std::os::unix::fs::symlink(&target, file(&dir)).unwrap();
    assert_eq!(
        read_legacy_contacts(dir.path()),
        Err(LegacyError::NotRegularFile)
    );
}
