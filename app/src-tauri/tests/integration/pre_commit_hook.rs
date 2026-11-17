use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn hook_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.githooks/pre-commit")
}

#[test]
fn pre_commit_hook_exists_is_executable_and_runs_all_gates() {
    let path = hook_path();
    let mode = std::fs::metadata(&path)
        .expect("hook must exist")
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "hook must be executable");

    let script = std::fs::read_to_string(&path).unwrap();
    let secret_gate = ["SecretKey", "Bytes|Db", "Key"].concat();
    let backend_gate = [
        "File",
        "Keychain|messenger",
        "-headless|messenger",
        "_headless",
    ]
    .concat();
    let required = [
        "set -euo pipefail",
        "cargo fmt --all --check",
        "cargo clippy --workspace --all-targets --all-features -- -D warnings",
        "cargo test --workspace",
        "cargo build",
        secret_gate.as_str(),
        backend_gate.as_str(),
        "writeText|clipboard",
        "npm run check",
        "npm test",
        "npm run build",
    ];
    for needle in required {
        assert!(script.contains(needle), "hook missing: {needle}");
    }

    let gate_needles = [
        "CARGO_TARGET_DIR=",
        "target/gate",
        "CARGO_INCREMENTAL=0",
        "flock",
        "trap cleanup EXIT",
        "trap 'exit 130' INT",
        "trap 'exit 143' TERM",
        ".githooks/gate-cleanup.sh",
        "rustc -vV",
        "Cargo.lock",
        "nm -D --defined-only",
        "sqlite3_",
    ];
    for needle in gate_needles {
        assert!(script.contains(needle), "hook missing: {needle}");
    }
    assert!(
        script.find("flock 9").unwrap() < script.find("trap cleanup EXIT").unwrap(),
        "trap must be installed after the lock is held"
    );
    assert!(
        script.find("cargo build\n").unwrap() < script.find("nm -D --defined-only").unwrap(),
        "symbol check must follow cargo build"
    );

    let cleanup = hook_path().with_file_name("gate-cleanup.sh");
    let mode = std::fs::metadata(&cleanup)
        .expect("gate-cleanup.sh must exist")
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "gate-cleanup.sh must be executable");
    let body = std::fs::read_to_string(&cleanup).unwrap();
    for needle in ["cargo", " clean ", "--target-dir", "refuse"] {
        assert!(body.contains(needle), "gate-cleanup.sh missing: {needle}");
    }
}
