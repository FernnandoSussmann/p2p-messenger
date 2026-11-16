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
}
