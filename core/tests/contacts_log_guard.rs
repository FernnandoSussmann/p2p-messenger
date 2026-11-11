//! Diagnostics must never carry contact data: no `tracing` macro invocation in
//! the contacts or discovery slices may mention names.

use std::fs;
use std::path::{Path, PathBuf};

const MACROS: [&str; 5] = ["trace!", "debug!", "info!", "warn!", "error!"];
const FORBIDDEN: [&str; 5] = [
    "nickname",
    "self_name",
    "display_name",
    "name =",
    ".as_str()",
];

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

/// Text inside the balanced delimiters following a macro name, string
/// literals included (their contents are scanned too).
fn argument_text(src: &str, open: usize) -> &str {
    let bytes = src.as_bytes();
    let (mut depth, mut i, mut in_str) = (0usize, open, false);
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            match b {
                b'\\' => i += 1,
                b'"' => in_str = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..=i];
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    &src[open..]
}

fn violations(src: &str) -> Vec<String> {
    let mut found = Vec::new();
    for mac in MACROS {
        for (at, _) in src.match_indices(mac) {
            let before = src[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let open = at + mac.len();
            if !src[open..].starts_with(['(', '[', '{']) {
                continue;
            }
            let args = argument_text(src, open);
            for word in FORBIDDEN {
                if args.contains(word) {
                    found.push(format!("{mac}{args} mentions `{word}`"));
                }
            }
        }
    }
    found
}

#[test]
fn guard_detects_violations() {
    assert!(!violations(r#"tracing::debug!(nickname = %n, "x");"#).is_empty());
    assert!(!violations(r#"warn!("got {}", name.as_str())"#).is_empty());
    assert!(violations(r#"debug!(peer = %id.fmt_short(), "ok");"#).is_empty());
    assert!(violations(r#"let unwarn!(x) = nickname;"#).is_empty());
}

#[test]
fn contacts_and_discovery_never_log_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root.join("contacts"), &mut files);
    rust_files(&root.join("discovery"), &mut files);
    assert!(!files.is_empty());
    let mut all = Vec::new();
    for file in files {
        let src = fs::read_to_string(&file).unwrap();
        all.extend(
            violations(&src)
                .into_iter()
                .map(|v| format!("{}: {v}", file.display())),
        );
    }
    assert!(all.is_empty(), "{all:#?}");
}
