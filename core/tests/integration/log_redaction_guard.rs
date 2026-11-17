//! Workspace-wide redaction guard for diagnostics. Every `tracing` call in
//! `core`, the app shell and the headless node may only record reviewed,
//! non-sensitive fields; the app and core never write to stdout/stderr.
//! Extend `ALLOWED_FIELDS` only with reviewed, non-sensitive names.

use std::fs;
use std::path::{Path, PathBuf};

const MACROS: [&str; 12] = [
    "trace!",
    "debug!",
    "info!",
    "warn!",
    "error!",
    "event!",
    "span!",
    "trace_span!",
    "debug_span!",
    "info_span!",
    "warn_span!",
    "error_span!",
];

const ALLOWED_FIELDS: [&str; 20] = [
    "peer",
    "node",
    "direction",
    "state",
    "reason",
    "error",
    "attempts",
    "count",
    "kind",
    "addr",
    "relay",
    "connected",
    "outcome",
    "direct_addrs",
    "bound",
    "lan",
    "presence",
    "version",
    "migration",
    "schema",
];

const FORBIDDEN: [&str; 22] = [
    "body",
    "text",
    "plaintext",
    "query",
    "nickname",
    "self_name",
    "display_name",
    "name",
    "ticket",
    "secret",
    "key",
    "db_key",
    "DbKey",
    "SecretKeyBytes",
    "as_bytes",
    "hex",
    "passphrase",
    "payload",
    "hello",
    "msg",
    "message",
    "contact",
];

const DIRECT_OUTPUT: [&str; 5] = ["println!", "eprintln!", "print!", "eprint!", "dbg!"];
const STD_STREAMS: [&str; 2] = ["stdout", "stderr"];

const FALLBACK_FILE: &str = "app/src-tauri/src/lib.rs";
const FALLBACK_STATEMENT: &str = r#"let _ = std::io::Write::write_all(
    &mut std::io::stderr(),
    b"p2p-messenger: diagnostic log file unavailable; continuing without file logging\n",
);"#;

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

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Source with comments blanked and string literals reduced to `"   "`, same
/// byte length. The ranges are the contents of the string literals.
struct Lexed {
    code: String,
    strings: Vec<(usize, usize)>,
}

fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut strings = Vec::new();
    let blank = |out: &mut [u8], from: usize, to: usize| {
        for byte in &mut out[from..to] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(b.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let (mut depth, mut j) = (1, i + 2);
                while j < b.len() && depth > 0 {
                    if b[j..].starts_with(b"/*") {
                        depth += 1;
                        j += 2;
                    } else if b[j..].starts_with(b"*/") {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j);
                i = j;
            }
            b'r' if (i == 0 || !is_ident_byte(b[i - 1]))
                && matches!(b.get(i + 1), Some(b'"' | b'#'))
                && raw_string_end(b, i).is_some() =>
            {
                let (content, end) = raw_string_end(b, i).unwrap();
                strings.push(content);
                blank(&mut out, i, end);
                out[i] = b'"';
                out[end - 1] = b'"';
                i = end;
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let end = (j + 1).min(b.len());
                strings.push((i + 1, j.min(b.len())));
                blank(&mut out, i + 1, end.saturating_sub(1).max(i + 1));
                i = end;
            }
            b'\'' => {
                let escaped = b.get(i + 1) == Some(&b'\\');
                let plain = b.get(i + 2) == Some(&b'\'') && b.get(i + 1) != Some(&b'\'');
                if escaped {
                    let end = src[i + 2..].find('\'').map_or(b.len(), |n| i + 2 + n + 1);
                    blank(&mut out, i + 1, end.saturating_sub(1).max(i + 1));
                    i = end;
                } else if plain {
                    out[i + 1] = b' ';
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    Lexed {
        code: String::from_utf8(out).expect("blanking preserves UTF-8"),
        strings,
    }
}

/// `((content_start, content_end), end)` of a raw string starting at `r`.
fn raw_string_end(b: &[u8], at: usize) -> Option<((usize, usize), usize)> {
    let mut j = at + 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    let hashes = j - at - 1;
    if b.get(j) != Some(&b'"') {
        return None;
    }
    let close: Vec<u8> = std::iter::once(b'"')
        .chain(std::iter::repeat_n(b'#', hashes))
        .collect();
    let start = j + 1;
    let rel = b[start..].windows(close.len()).position(|w| w == close)?;
    Some(((start, start + rel), start + rel + close.len()))
}

/// Index of the delimiter closing the one at `open`, scanning masked code.
fn argument_text(code: &str, open: usize) -> usize {
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    bytes.len() - 1
}

fn idents(code: &str) -> Vec<&str> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if is_ident_byte(b[i]) {
            let start = i;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            if !b[start].is_ascii_digit() {
                out.push(&code[start..i]);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// `{ident}` / `{ident:spec}` captures in a format string.
fn inline_captures(literal: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = literal;
    while let Some(at) = rest.find('{') {
        rest = &rest[at + 1..];
        if let Some(stripped) = rest.strip_prefix('{') {
            rest = stripped;
            continue;
        }
        let end = rest.find(['}', ':']).unwrap_or(rest.len());
        let capture = rest[..end].trim();
        if !capture.is_empty() && !capture.starts_with(|c: char| c.is_ascii_digit()) {
            out.push(capture);
        }
    }
    out
}

fn split_top_level(inner: &str) -> Vec<&str> {
    let (mut depth, mut start, mut parts) = (0usize, 0, Vec::new());
    for (i, b) in inner.bytes().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
}

fn field_violation(segment: &str) -> Option<String> {
    let seg = segment.trim();
    if seg.is_empty()
        || seg.starts_with('"')
        || ["Level::", "target:", "parent:"]
            .iter()
            .any(|p| seg.starts_with(p))
    {
        return None;
    }
    let seg = seg.trim_start_matches(['%', '?']);
    let end = seg
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(seg.len());
    let (path, rest) = (&seg[..end], seg[end..].trim_start());
    let is_field = rest.is_empty() || (rest.starts_with('=') && !rest.starts_with("=="));
    if path.is_empty() || !is_field {
        return Some(format!("unreviewed expression `{}`", segment.trim()));
    }
    (!ALLOWED_FIELDS.contains(&path)).then(|| format!("field `{path}` is not allowlisted"))
}

fn log_violations(src: &str) -> Vec<String> {
    let lexed = lex(src);
    let code = &lexed.code;
    let mut found = Vec::new();
    let report_args = |label: &str, open: usize, close: usize, with_fields: bool| {
        let mut found = Vec::new();
        let args = &code[open..=close];
        for ident in idents(args) {
            if FORBIDDEN.contains(&ident) {
                found.push(format!("{label} mentions `{ident}`"));
            }
        }
        for &(s, e) in lexed
            .strings
            .iter()
            .filter(|&&(s, _)| s > open && s < close)
        {
            for capture in inline_captures(&src[s..e]) {
                if FORBIDDEN.contains(&capture) {
                    found.push(format!("{label} captures `{{{capture}}}`"));
                }
            }
        }
        if with_fields {
            let inner = &args[1..args.len() - 1];
            found.extend(
                split_top_level(inner)
                    .into_iter()
                    .filter_map(field_violation)
                    .map(|v| format!("{label}: {v}")),
            );
        }
        found
    };
    for mac in MACROS {
        for (at, _) in code.match_indices(mac) {
            if at > 0 && is_ident_byte(code.as_bytes()[at - 1]) {
                continue;
            }
            let open = at + mac.len();
            let Some(open) = code[open..]
                .find(|c: char| !c.is_whitespace())
                .map(|n| open + n)
                .filter(|&o| code[o..].starts_with(['(', '[', '{']))
            else {
                continue;
            };
            let close = argument_text(code, open);
            let label = format!("{mac}{}", &src[open..=close]);
            found.extend(report_args(&label, open, close, true));
        }
    }
    for marker in ["#[instrument", "#[tracing::instrument"] {
        for (at, _) in code.match_indices(marker) {
            let close = argument_text(code, at + 1);
            let attr = &code[at..=close];
            let label = format!("{marker}...]");
            if !attr.contains("skip_all") {
                found.push(format!("{label} without skip_all"));
            }
            found.extend(report_args(&label, at + 1, close, false));
        }
    }
    found
}

fn output_violations(rel: &str, src: &str) -> Vec<String> {
    let mut text = src.to_owned();
    if rel == FALLBACK_FILE {
        if let Some((from, to)) = find_ignoring_whitespace(&text, FALLBACK_STATEMENT) {
            let blanked: String = text[from..to]
                .chars()
                .map(|c| if c == '\n' { '\n' } else { ' ' })
                .collect();
            text.replace_range(from..to, &blanked);
        }
    }
    let code = lex(&text).code;
    let mut found = Vec::new();
    for mac in DIRECT_OUTPUT {
        for (at, _) in code.match_indices(mac) {
            if at == 0 || !is_ident_byte(code.as_bytes()[at - 1]) {
                found.push(format!("direct output `{mac}`"));
            }
        }
    }
    for ident in idents(&code) {
        if STD_STREAMS.contains(&ident) {
            found.push(format!("direct use of std `{ident}`"));
        }
    }
    found
}

/// Byte span of the first occurrence of `needle` in `hay`, ignoring all
/// whitespace in both.
fn find_ignoring_whitespace(hay: &str, needle: &str) -> Option<(usize, usize)> {
    let squeezed: Vec<(usize, char)> = hay
        .char_indices()
        .filter(|(_, c)| !c.is_whitespace())
        .collect();
    let want: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    let at = squeezed
        .windows(want.len())
        .position(|w| w.iter().map(|&(_, c)| c).eq(want.iter().copied()))?;
    let (last, c) = squeezed[at + want.len() - 1];
    Some((squeezed[at].0, last + c.len_utf8()))
}

fn exempt_from_output_rule(rel: &str) -> bool {
    rel.ends_with("/tests.rs") || rel.contains("/tests/") || rel.contains("/testing/")
}

#[test]
fn guard_rejects_planted_violations() {
    for snippet in [
        "debug!(body = %m.body)",
        r#"info!("{}", chat.body)"#,
        "warn!(key = ?db_key)",
        "trace!(secret = ?k.as_bytes())",
        "info!(name = %display_name)",
        r#"info!("hi {nickname}")"#,
        "#[instrument] fn f(x: Y) {}",
        r#"tracing::info!(peer = %p, size = n, "x")"#,
        r#"tracing::info!("{}", count.to_string())"#,
    ] {
        assert!(!log_violations(snippet).is_empty(), "{snippet}");
    }
    assert!(!output_violations("core/src/x.rs", r#"eprintln!("{body}")"#).is_empty());
    assert!(!output_violations("core/src/x.rs", "let s = std::io::stdout();").is_empty());
}

#[test]
fn guard_accepts_reviewed_calls() {
    for snippet in [
        r#"info!(peer = %peer(&id), addr = %transport(&a), "path opened")"#,
        r#"warn!(error = %e, "storage unavailable")"#,
        r#"tracing::info!(count = r.migrated, outcome, "done {}", count)"#,
        r#"#[tracing::instrument(skip_all)] fn f() {}"#,
        r#"let nobody = keyed; // info!(body = x)"#,
        r#"let s = "warn!(body)";"#,
        r#"let unwarn!(x) = nickname;"#,
    ] {
        assert!(log_violations(snippet).is_empty(), "{snippet}");
    }
    assert!(
        output_violations("core/src/x.rs", "// eprintln!(1)\nlet s = \"println!\";").is_empty()
    );
}

#[test]
fn stderr_fallback_is_allowlisted_by_exact_match() {
    assert!(output_violations(FALLBACK_FILE, FALLBACK_STATEMENT).is_empty());
    let reflowed = FALLBACK_STATEMENT.replace('\n', " ");
    assert!(output_violations(FALLBACK_FILE, &reflowed).is_empty());
    assert!(!output_violations("app/src-tauri/src/net_cmd.rs", FALLBACK_STATEMENT).is_empty());
    let twice = format!("{FALLBACK_STATEMENT}\n{FALLBACK_STATEMENT}");
    assert!(!output_violations(FALLBACK_FILE, &twice).is_empty());
    let changed = FALLBACK_STATEMENT.replace("unavailable", "unavailable!");
    assert!(!output_violations(FALLBACK_FILE, &changed).is_empty());
}

#[test]
fn workspace_logs_never_carry_sensitive_data() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut all = Vec::new();
    for root in ["core/src", "app/src-tauri/src", "headless/src"] {
        let mut files = Vec::new();
        rust_files(&workspace.join(root), &mut files);
        assert!(!files.is_empty(), "{root} has no sources");
        for file in files {
            let rel = file
                .strip_prefix(&workspace)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let rel = rel.trim_start_matches("./").to_owned();
            let src = fs::read_to_string(&file).unwrap();
            let mut violations = log_violations(&src);
            if root != "headless/src" && !exempt_from_output_rule(&rel) {
                violations.extend(output_violations(&rel, &src));
            }
            all.extend(violations.into_iter().map(|v| format!("{rel}: {v}")));
        }
    }
    assert!(all.is_empty(), "{all:#?}");
}
