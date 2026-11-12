//! Static half of the `account_id` namespacing rule: every `SQL_*` constant
//! names `account_id`, every `MIGRATION_*` table declares it, and no inline
//! data SQL bypasses the constants. Allowlists are explicit and documented in
//! `core/src/storage/mod.rs`.

use std::fs;
use std::path::{Path, PathBuf};

/// FTS maintenance statement (see the `messages_fts` exception).
const SQL_ALLOWLIST: &[&str] = &["SQL_FTS_OPTIMIZE"];
/// External-content FTS index reached only through a join on `messages`.
const TABLE_ALLOWLIST: &[&str] = &["messages_fts"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "tests.rs"
        || name == "contract.rs"
        || path.contains("/tests/")
        || path.contains("/testing/")
}

/// The text of the item that starts right after `start` and ends at the first
/// `;` outside a string literal.
fn item_text(src: &str, start: usize) -> &str {
    let bytes = src.as_bytes();
    let (mut i, mut in_str) = (start, false);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b';' if !in_str => return &src[start..i],
            _ => {}
        }
        i += 1;
    }
    &src[start..]
}

fn consts<'a>(src: &'a str, prefix: &str) -> Vec<(String, &'a str)> {
    let needle = format!("const {prefix}");
    let mut out = Vec::new();
    for (at, _) in src.match_indices(&needle) {
        let rest = &src[at + "const ".len()..];
        let name_end = rest.find(':').unwrap_or(rest.len());
        let name = rest[..name_end].trim().to_owned();
        if let Some(eq) = src[at..].find('=') {
            out.push((name, item_text(src, at + eq + 1)));
        }
    }
    out
}

fn table_name(segment: &str) -> &str {
    let end = segment
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(segment.len());
    &segment[..end]
}

fn table_violations(path: &str, name: &str, sql: &str, out: &mut Vec<String>) {
    for (marker, virtual_table) in [("CREATE TABLE ", false), ("CREATE VIRTUAL TABLE ", true)] {
        for (at, _) in sql.match_indices(marker) {
            let rest = &sql[at + marker.len()..];
            let table = table_name(rest);
            let def = rest.split("\nCREATE ").next().unwrap_or(rest);
            let allowed = TABLE_ALLOWLIST.contains(&table);
            if !allowed && (virtual_table || !def.contains("account_id")) {
                out.push(format!("{path}: {name}: table {table} has no account_id"));
            }
        }
    }
}

fn inline_violations(path: &str, src: &str, out: &mut Vec<String>) {
    for call in [
        ".execute(\"",
        ".prepare(\"",
        ".prepare_cached(\"",
        ".query_row(\"",
    ] {
        for (at, _) in src.match_indices(call) {
            let lit = src[at + call.len()..].trim_start();
            let is_data = ["SELECT", "INSERT", "UPDATE", "DELETE"]
                .iter()
                .any(|v| lit.starts_with(v));
            let literal = lit.split('"').next().unwrap_or(lit);
            if is_data && !literal.contains("sqlite_master") {
                out.push(format!("{path}: inline data SQL: {literal}"));
            }
        }
    }
}

fn violations(files: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, src) in files.iter().filter(|(p, _)| !is_test_path(p)) {
        for (name, text) in consts(src, "SQL_") {
            if !SQL_ALLOWLIST.contains(&name.as_str()) && !text.contains("account_id") {
                out.push(format!("{path}: {name} does not mention account_id"));
            }
        }
        for (name, text) in consts(src, "MIGRATION_") {
            table_violations(path, &name, text, &mut out);
        }
        inline_violations(path, src, &mut out);
    }
    out
}

#[test]
fn core_sql_is_namespaced_by_account_id() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rust_files(&root, &mut paths);
    assert!(!paths.is_empty());
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            (
                p.to_string_lossy().replace('\\', "/"),
                fs::read_to_string(p).unwrap(),
            )
        })
        .collect();
    let found = violations(&files);
    assert!(found.is_empty(), "{found:#?}");
    let consts_seen: usize = files.iter().map(|(_, s)| consts(s, "SQL_").len()).sum();
    assert!(consts_seen > 5, "the scan must see the messaging constants");
}

fn planted(src: &str) -> Vec<String> {
    violations(&[("core/src/x.rs".to_owned(), src.to_owned())])
}

#[test]
fn guard_rejects_a_const_without_account_id() {
    let bad = "const SQL_ALL: &str = concat!(\"SELECT * \", \"FROM messages\");";
    assert_eq!(planted(bad).len(), 1);
    let good = "const SQL_ONE: &str = \"SELECT 1 FROM messages WHERE account_id = ?1\";";
    assert!(planted(good).is_empty());
    assert!(
        planted("const SQL_FTS_OPTIMIZE: &str = \"INSERT INTO f (f) VALUES ('optimize')\";")
            .is_empty()
    );
}

#[test]
fn guard_rejects_a_table_without_account_id() {
    let bad = "const MIGRATION_X_V1: &str = \"\nCREATE TABLE t (x TEXT);\nCREATE INDEX i ON t (account_id);\n\";";
    assert_eq!(planted(bad).len(), 1);
    let good = "const MIGRATION_X_V1: &str = \"\nCREATE TABLE t (account_id BLOB, x TEXT);\n\";";
    assert!(planted(good).is_empty());
    let fts = "const MIGRATION_X_V1: &str = \"\nCREATE VIRTUAL TABLE messages_fts USING fts5 (body);\n\";";
    assert!(planted(fts).is_empty());
    let other =
        "const MIGRATION_X_V1: &str = \"\nCREATE VIRTUAL TABLE other USING fts5 (body);\n\";";
    assert_eq!(planted(other).len(), 1);
}

#[test]
fn guard_rejects_inline_data_sql() {
    assert_eq!(planted("c.execute(\"DELETE FROM messages\", []);").len(), 1);
    assert_eq!(planted("c.prepare_cached(\"SELECT 1\");").len(), 1);
    assert!(planted("c.query_row(\"PRAGMA cipher_version\", [], f);").is_empty());
    assert!(planted("c.query_row(\"SELECT count(*) FROM sqlite_master\", [], f);").is_empty());
}
