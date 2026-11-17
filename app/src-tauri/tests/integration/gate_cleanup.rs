#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HASH: &str = "0123456789abcdef";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn script() -> PathBuf {
    repo().join(".githooks/gate-cleanup.sh")
}

fn target_dir() -> PathBuf {
    let t = repo().join("target");
    fs::create_dir_all(&t).unwrap();
    t
}

fn temp_in_target() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("gate-cleanup-test-")
        .tempdir_in(target_dir())
        .unwrap()
}

fn run(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("bash");
    cmd.arg(script()).args(args).env_remove("CARGO_TARGET_DIR");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"x").unwrap();
}

fn new_gate(tmp: &tempfile::TempDir) -> PathBuf {
    let gate = tmp.path().join("gate");
    fs::create_dir_all(&gate).unwrap();
    fs::write(
        gate.join("CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n",
    )
    .unwrap();
    gate
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn assert_refused(o: &Output) {
    assert_eq!(o.status.code(), Some(2), "stderr: {}", stderr(o));
    assert!(stderr(o).contains("refusing"), "stderr: {}", stderr(o));
}

#[test]
fn script_exists_and_is_executable() {
    let mode = fs::metadata(script()).unwrap().permissions().mode();
    assert!(mode & 0o111 != 0);
}

#[test]
fn removes_workspace_artifacts_and_keeps_third_party() {
    let tmp = temp_in_target();
    let gate = new_gate(&tmp);
    let deps = gate.join("debug/deps");
    let ours = [
        format!("core-{HASH}"),
        format!("libcore-{HASH}.rlib"),
        format!("libcore-{HASH}.rmeta"),
        format!("app-{HASH}"),
        format!("libapp_lib-{HASH}.rlib"),
        format!("protocol-{HASH}"),
    ];
    for name in &ours {
        touch(&deps.join(name));
    }
    let third = deps.join(format!("libserde-{HASH}.rlib"));
    touch(&third);
    let fp = gate.join(format!("debug/.fingerprint/core-{HASH}"));
    touch(&fp.join("lib-core"));

    let o = run_gate(&gate, &[], &[]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert!(third.exists(), "third-party library must remain");
    for name in &ours {
        assert!(!deps.join(name).exists(), "{name} must be removed");
    }
    assert!(!fp.exists());
}

fn run_gate(gate: &Path, extra: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut args: Vec<&str> = extra.to_vec();
    args.push(gate.to_str().unwrap());
    run(&args, envs)
}

#[test]
fn refuses_unsafe_paths_and_deletes_nothing() {
    let tmp = temp_in_target();

    assert_refused(&run(&[""], &[]));
    assert_refused(&run(&["/"], &[]));

    let home = tmp.path().join("home");
    touch(&home.join("sentinel"));
    assert_refused(&run_gate(&home, &[], &[("HOME", home.to_str().unwrap())]));
    assert!(home.join("sentinel").exists());

    assert_refused(&run_gate(&repo(), &[], &[]));
    assert!(repo().join("Cargo.toml").exists());

    let outside = tempfile::tempdir().unwrap();
    touch(&outside.path().join("sentinel"));
    assert_refused(&run_gate(outside.path(), &[], &[]));
    assert!(outside.path().join("sentinel").exists());

    let real = tmp.path().join("real");
    touch(&real.join("sentinel"));
    touch(&real.join(format!("libcore-{HASH}.rlib")));
    let link = tmp.path().join("link");
    symlink(&real, &link).unwrap();
    assert_refused(&run_gate(&link, &[], &[]));
    let slashed = format!("{}/", link.display());
    assert_refused(&run(&[&slashed], &[]));
    assert_refused(&run_gate(&link, &["--wipe"], &[]));
    assert!(real.join("sentinel").exists());
    assert!(real.join(format!("libcore-{HASH}.rlib")).exists());
}

fn big_file(gate: &Path) -> PathBuf {
    let f = gate.join("debug/deps").join(format!("libbig-{HASH}.rlib"));
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, vec![0u8; 2 * 1024 * 1024]).unwrap();
    f
}

#[test]
fn size_cap_wipes_whole_gate_dir() {
    let tmp = temp_in_target();
    let gate = new_gate(&tmp);
    big_file(&gate);
    let o = run_gate(&gate, &[], &[("P2PM_GATE_MAX_MIB", "1")]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert!(!gate.exists());
}

#[test]
fn invalid_cap_falls_back_to_default() {
    let tmp = temp_in_target();
    let gate = new_gate(&tmp);
    let f = big_file(&gate);
    for cap in ["0", "-1", "abc", "1.5", " 1", "01", "99999999999"] {
        let o = run_gate(&gate, &[], &[("P2PM_GATE_MAX_MIB", cap)]);
        assert!(o.status.success(), "cap {cap:?}: {}", stderr(&o));
        assert!(f.exists(), "cap {cap:?} must fall back to the default");
    }
}

#[test]
fn wipe_mode_removes_gate_dir() {
    let tmp = temp_in_target();
    let gate = new_gate(&tmp);
    big_file(&gate);
    let o = run_gate(&gate, &["--wipe"], &[]);
    assert!(o.status.success(), "stderr: {}", stderr(&o));
    assert!(!gate.exists());
}
