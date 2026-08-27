//! `wanted` — a plugin-driven development-tool installer.
//!
//! Core design is documented in [`docs/`](../docs). This crate carries only the
//! domain logic (parsing / planning / execution / environment wiring); the CLI
//! front end lives in `main.rs`.
//!
//! Key invariants: **planning is a pure function**, and **every side effect
//! carries its own undo action (compensation)**, so any step can be rolled back.

pub mod cli;
pub mod engine;
pub mod env;
pub mod error;
pub mod plugin;
pub mod receipt;
pub mod report;
pub mod store;
pub mod uninstall;
pub mod upgrade;
pub mod version;

pub use error::Error;
pub use receipt::{Receipt, VarSnapshot};
pub use report::{Progress, ProgressState, Reporter, SilentReporter};
pub use version::Version;

/// The crate-wide result type.
pub type Result<T> = std::result::Result<T, crate::error::Error>;
