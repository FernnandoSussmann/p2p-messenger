//! All `core` integration tests in one binary (DD-06 §3): one link of the
//! dependency graph instead of sixteen. Modules keep their original file
//! names, so filters stay stable (e.g. `--test integration discovery_lan`).
//! Tests that install process-global state keep their own binary:
//! `logging_canary.rs` (global tracing subscriber) and `profile_isolation.rs`
//! (global keyring_core default store). Put any new test that sets a global
//! subscriber, default store, env var or working directory in its own binary.

mod contacts_two_nodes;
mod discovery_channels;
mod discovery_lan;
mod discovery_names;
mod discovery_parity;
mod discovery_relay;
mod discovery_two_nodes;
mod identity_integration;
mod log_redaction_guard;
mod messaging_frames;
mod messaging_store;
mod messaging_two_nodes;
mod sql_namespacing_guard;
mod storage_legacy_import;
mod storage_namespacing;
mod storage_open;
