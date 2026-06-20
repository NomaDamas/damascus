//! Damascus library: a verify-gated, test-time-scaling coding harness that makes
//! modest / local LLMs produce frontier-quality, verified changes.
//!
//! The binary in `main.rs` is a thin CLI over these modules. Exposing them as a
//! library lets the whole Fold Loop be integration-tested with an in-process
//! mock provider (see `tests/`).

pub mod config;
pub mod context;
pub mod edits;
pub mod generate;
pub mod ledger;
pub mod orchestrator;
pub mod plan;
pub mod prompts;
pub mod provider;
pub mod sandbox;
pub mod select;
pub mod ui;
pub mod verify;
