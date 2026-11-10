//! Test-only harness: a headless node that runs the shipping `core` discovery
//! stack with a file-backed keyring, used to simulate devices in Level-3 tests.
//! It is never shipped and the desktop app never depends on it.
#![forbid(unsafe_code)]

pub mod cli;
pub mod file_keychain;

// ── track B modules ──
#[cfg(unix)]
pub mod control;
#[cfg(unix)]
pub mod node;

// ── integration ──
