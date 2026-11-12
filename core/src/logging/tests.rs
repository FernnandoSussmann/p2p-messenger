use std::fs;
use std::io::Write;
use std::path::Path;

use tracing::level_filters::LevelFilter;
use tracing::Level;

use super::writer::{RollingFile, LOG_FILE};
use super::*;

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn write_line(w: &mut RollingFile, line: &str) {
    w.write_all(format!("{line}\n").as_bytes()).unwrap();
}

#[test]
fn level_from_env_accepts_known_and_rejects_others() {
    assert_eq!(level_from_env(None), Ok(DEFAULT_LEVEL));
    assert_eq!(level_from_env(Some("DeBuG")), Ok(LevelFilter::DEBUG));
    assert_eq!(level_from_env(Some("off")), Ok(LevelFilter::OFF));
    assert_eq!(level_from_env(Some("trace")), Ok(LevelFilter::TRACE));
    for bad in ["", "bogus", "info ", "5", "core=debug"] {
        assert_eq!(level_from_env(Some(bad)), Err(LogConfigError), "{bad:?}");
    }
}

#[test]
fn rotation_keeps_total_size_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = RollingFile::open(dir.path(), 4 * 1024, 3).unwrap();
    let total = 200 * 1024;
    let mut n = 0;
    while n * 100 < total {
        write_line(&mut w, &format!("event {n:06} {}", "x".repeat(80)));
        n += 1;
    }
    let names = files(dir.path());
    assert!(names.len() <= 3, "{names:?}");
    for name in &names {
        let len = fs::metadata(dir.path().join(name)).unwrap().len();
        assert!(len <= 4 * 1024 + MAX_LINE_BYTES as u64, "{name} {len}");
    }
    let active = fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
    assert!(active.contains(&format!("event {:06}", n - 1)));
}

#[test]
fn writer_neutralises_control_and_bidi_chars() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = RollingFile::open(dir.path(), 1 << 20, 2).unwrap();
    write_line(&mut w, "\x1b[31mred a\nb \u{202E}rtl \u{2066}x\r");
    let text = fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\u{202E}') && !text.contains('\u{2066}'));
    assert!(text.contains("a\\nb"));
    assert!(text.contains('\u{FFFD}'));
    assert!(text.ends_with('\n'));
}

#[test]
fn writer_caps_line_length() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = RollingFile::open(dir.path(), 1 << 20, 2).unwrap();
    write_line(&mut w, &"\u{e9}".repeat(MAX_LINE_BYTES));
    let text = fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
    assert!(text.len() <= MAX_LINE_BYTES);
    assert!(text.ends_with('\n'));
}

#[cfg(unix)]
#[test]
fn log_files_and_dir_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    let mut w = RollingFile::open(&logs, 64, 3).unwrap();
    for i in 0..10 {
        write_line(&mut w, &format!("line {i} padding padding padding"));
    }
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&logs), 0o700);
    for name in files(&logs) {
        assert_eq!(mode(&logs.join(name)), 0o600);
    }
}

#[cfg(unix)]
#[test]
fn symlinked_log_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::write(&target, "keep").unwrap();
    let logs = dir.path().join("logs");
    fs::create_dir(&logs).unwrap();
    std::os::unix::fs::symlink(&target, logs.join(LOG_FILE)).unwrap();
    assert!(RollingFile::open(&logs, 1024, 2).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");

    let link_dir = dir.path().join("linked");
    std::os::unix::fs::symlink(&logs, &link_dir).unwrap();
    assert!(RollingFile::open(&link_dir, 1024, 2).is_err());
}

#[test]
fn filter_caps_third_party_targets() {
    let f = filter(LevelFilter::TRACE);
    assert!(f.would_enable("core::discovery::node", &Level::TRACE));
    assert!(f.would_enable("app_lib::net_cmd", &Level::TRACE));
    assert!(!f.would_enable("iroh::socket", &Level::INFO));
    assert!(f.would_enable("iroh::socket", &Level::WARN));
    assert!(!f.would_enable("core_foundation::x", &Level::INFO));
    assert!(!f.would_enable("app_lib_other", &Level::INFO));
    let info = filter(LevelFilter::INFO);
    assert!(!info.would_enable("core::x", &Level::DEBUG));
    assert!(info.would_enable("core::x", &Level::INFO));
    let off = filter(LevelFilter::OFF);
    assert!(!off.would_enable("iroh::x", &Level::ERROR));
}

#[test]
fn off_level_creates_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    init(&LogConfig::new(&logs, LevelFilter::OFF)).unwrap();
    assert!(!logs.exists());
}

#[test]
fn subscriber_writes_events_without_ansi() {
    let dir = tempfile::tempdir().unwrap();
    let sub = subscriber(&LogConfig::new(dir.path(), LevelFilter::DEBUG)).unwrap();
    tracing::subscriber::with_default(sub, || {
        tracing::warn!(error = "x\ny", "unit");
    });
    let text = fs::read_to_string(dir.path().join(LOG_FILE)).unwrap();
    assert!(text.contains("unit") && !text.contains('\x1b'));
    assert_eq!(text.lines().count(), 1);
}

fn strip_comments(src: &str) -> String {
    src.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_token(src: &str, token: &str) -> bool {
    src.match_indices(token).any(|(at, _)| {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let before = src[..at].chars().next_back().is_some_and(word);
        let after = src[at + token.len()..].chars().next().is_some_and(word);
        !before && !after
    })
}

#[test]
fn logging_has_no_network_sink() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in ["writer.rs", "mod.rs", "config.rs"] {
        let src = strip_comments(&fs::read_to_string(root.join("src/logging").join(file)).unwrap());
        for token in [
            "std::net",
            "tokio::net",
            "TcpStream",
            "UdpSocket",
            "reqwest",
            "hyper",
            "ureq",
        ] {
            assert!(!has_token(&src, token), "{file} mentions {token}");
        }
    }
    let manifests = [
        root.join("../Cargo.toml"),
        root.join("Cargo.toml"),
        root.join("../app/src-tauri/Cargo.toml"),
    ];
    for path in manifests {
        let toml = fs::read_to_string(&path).unwrap();
        for dep in [
            "opentelemetry",
            "sentry",
            "loki",
            "gelf",
            "syslog",
            "journald",
            "datadog",
            "tracing-appender",
        ] {
            assert!(!toml.contains(dep), "{} lists {dep}", path.display());
        }
    }
}
