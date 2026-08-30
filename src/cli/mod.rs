//! CLI-facing domain helpers shared between the `main.rs` front end and tests.

pub mod add;
pub mod app;
pub mod spec;

pub use add::{PluginSource, resolve_add_source};
pub use spec::{ToolSpec, ToolSpecError};
