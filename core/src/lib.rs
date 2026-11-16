//! UI-agnostic feature slices of the p2p-messenger.
//!
//! This crate holds the application logic and knows nothing about Tauri or
//! the frontend; the `app` crate is a thin shell over it. Each slice owns its
//! own types and invariants. Trade-off: slices are public modules rather than
//! one flat API, so callers import by slice and a slice's internals stay
//! private behind its re-exports.
//!
//! Note: the package name `core` shadows the std `core` crate for dependents.
//! Dependents must rename the import (`package = "core"`).

#![forbid(unsafe_code)]

pub mod contacts;
pub mod discovery;
pub mod identity;
pub mod logging;
pub mod messaging;
pub mod storage;
