//! Environment configuration layer.
//!
//! Decoupled from execution: `env::apply_deltas` produces **pure-value** deltas
//! (what is wanted), and the store handles **persistent write-back** (where to
//! write, capturing the old value for compensation). Swapping the write strategy
//! only means swapping the store implementation.

pub mod store;

use std::cell::RefCell;
use std::collections::HashMap;

use crate::Result;

/// An environment variable delta operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvOp {
    /// Set directly to the given value.
    Set,
    /// Insert in front of the current value (for prepending to PATH).
    Prepend,
    /// Append after the current value.
    Append,
}

/// A delta description for one variable (pure value; unit-testable, usable in a
/// packed plan).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvDelta {
    /// Variable name.
    pub name: String,
    /// The value to join in (absolute path or full value).
    pub value: String,
    /// How to merge with the existing value.
    pub op: EnvOp,
}

/// Read/write backend for environment variables; `read`/`write` capture the old
/// value as a precondition for compensation.
pub trait EnvStore: Send {
    /// Read the current value; `None` when it does not exist.
    fn read(&self, name: &str) -> Result<Option<String>>;
    /// Write a new value.
    fn write(&self, name: &str, value: &str) -> Result<()>;
    /// Remove the variable.
    fn remove(&self, name: &str) -> Result<()>;
}

/// The path-list separator: `;` on Windows, `:` on POSIX.
///
/// This is a property of the target platform and independent of the write
/// backend (all backends agree on the same platform).
pub const PATH_SEP: &str = if cfg!(windows) { ";" } else { ":" };

/// User home directory (for `{user}` template expansion).
pub fn user_home() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default()
}

/// Apply a group of deltas to the store, snapshotting old values and returning
/// per-variable compensations.
///
/// The returned compensations are collected in "apply order"; the consumer
/// should replay them in reverse LIFO order.
pub fn apply_deltas(
    deltas: &[EnvDelta],
    store: &dyn EnvStore,
) -> Result<Vec<(String, Option<String>)>> {
    let mut snapshots = Vec::new();
    for delta in deltas {
        let old = store.read(&delta.name)?;
        let next = merge(old.as_deref(), delta, PATH_SEP);
        store.write(&delta.name, &next)?;
        snapshots.push((delta.name.clone(), old));
    }
    Ok(snapshots)
}

fn merge(current: Option<&str>, delta: &EnvDelta, sep: &str) -> String {
    match delta.op {
        EnvOp::Set => delta.value.clone(),
        EnvOp::Prepend => join_front(current, &delta.value, sep),
        EnvOp::Append => join_back(current, &delta.value, sep),
    }
}

fn join_front(current: Option<&str>, add: &str, sep: &str) -> String {
    match current.map(str::trim).filter(|s| !s.is_empty()) {
        None => add.to_string(),
        Some(existing) => {
            if existing_segments_exclude(existing, add, sep) {
                format!("{add}{sep}{existing}")
            } else {
                existing.to_string()
            }
        }
    }
}

fn join_back(current: Option<&str>, add: &str, sep: &str) -> String {
    match current.map(str::trim).filter(|s| !s.is_empty()) {
        None => add.to_string(),
        Some(existing) => {
            if existing_segments_exclude(existing, add, sep) {
                format!("{existing}{sep}{add}")
            } else {
                existing.to_string()
            }
        }
    }
}

/// Avoid the same segment appearing twice (deduplication when merging PATH).
fn existing_segments_exclude(existing: &str, add: &str, sep: &str) -> bool {
    !existing.split(sep).any(|seg| seg.trim() == add)
}

/// A purely in-memory env backend, used to test that compensation really
/// restores the old values.
#[derive(Default, Debug)]
pub struct MemEnvStore {
    values: RefCell<HashMap<String, String>>,
}

impl MemEnvStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from the given snapshot, handy for assertions.
    #[cfg(test)]
    pub fn snapshot(&self) -> HashMap<String, String> {
        self.values.borrow().clone()
    }
}

impl EnvStore for MemEnvStore {
    fn read(&self, name: &str) -> Result<Option<String>> {
        Ok(self.values.borrow().get(name).cloned())
    }
    fn write(&self, name: &str, value: &str) -> Result<()> {
        self.values
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn remove(&self, name: &str) -> Result<()> {
        self.values.borrow_mut().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
