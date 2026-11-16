fn main() {
    // Linux only: never export symbols from static archives. Without this the
    // executable exports the bundled SQLCipher's sqlite3_* functions and the
    // system WebKitGTK/libsoup (which load libsqlite3) bind to our copy
    // (DD-06 §6). `cargo::rustc-link-arg` covers every linked target of this
    // package: bins and tests. CARGO_CFG_TARGET_OS is the *target* OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg=-Wl,--exclude-libs,ALL");
    }
    tauri_build::build();
}
