//! Environment configuration layer.
//!
//! Decoupled from execution: `env::apply_deltas` produces **pure-value** deltas
//! (what is wanted), and the store handles **persistent write-back** (where to
//! write, capturing the old value for compensation). Swapping the write strategy
//! only means swapping the store implementation.

pub mod store;

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::Result;

/// An environment variable delta operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EnvOp {
    /// Set directly to the given value.
    Set,
    /// Insert in front of the current value (for prepending to PATH).
    Prepend,
    /// Append after the current value.
    Append,
}

/// The path-list separator: `;` on Windows, `:` on POSIX.
///
/// This is a property of the target platform and independent of the write
/// backend (all backends agree on the same platform).
pub const PATH_SEP: &str = if cfg!(windows) { ";" } else { ":" };

/// An environment variable name.
///
/// Equality is case-insensitive on Windows (where `Path`/`path`/`PATH` name the
/// same registry value) and byte-exact on POSIX. Call sites compare names or
/// test for PATH through this type, never a bare `String`, so the platform rule
/// cannot be forgotten at the point of use.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct EnvVar(String);

impl EnvVar {
    /// The verbatim name, as written by the caller.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The canonical form used for comparison: lowercased on Windows, verbatim on POSIX.
    fn canonical(&self) -> String {
        if cfg!(windows) {
            self.0.to_lowercase()
        } else {
            self.0.clone()
        }
    }

    /// Whether this is the PATH list variable. Matches case-insensitively on
    /// Windows, where the variable is looked up without case; byte-exact on POSIX.
    pub fn is_path(&self) -> bool {
        if cfg!(windows) {
            self.0.eq_ignore_ascii_case("PATH")
        } else {
            self.0 == "PATH"
        }
    }
}

impl From<String> for EnvVar {
    fn from(value: String) -> Self {
        EnvVar(value)
    }
}

impl From<&str> for EnvVar {
    fn from(value: &str) -> Self {
        EnvVar(value.to_owned())
    }
}

impl PartialEq for EnvVar {
    fn eq(&self, other: &Self) -> bool {
        self.canonical() == other.canonical()
    }
}

impl Eq for EnvVar {}

impl Hash for EnvVar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical().hash(state);
    }
}

impl std::fmt::Display for EnvVar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A delta description for one variable (pure value; unit-testable, usable in a
/// packed plan).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvDelta {
    /// Variable name.
    pub name: EnvVar,
    /// The value to join in (absolute path or full value).
    pub value: String,
    /// How to merge with the existing value.
    pub op: EnvOp,
}

/// Read/write backend for environment variables; `read`/`write` capture the old
/// value as a precondition for compensation.
pub trait EnvStore: Send {
    /// Read the current value; `None` when it does not exist.
    fn read(&self, name: &EnvVar) -> Result<Option<String>>;
    /// Write a new value.
    fn write(&self, name: &EnvVar, value: &str) -> Result<()>;
    /// Remove the variable.
    fn remove(&self, name: &EnvVar) -> Result<()>;
}

/// A PATH-like list of path entries joined by this platform's separator (`;` on
/// Windows, `:` on POSIX).
///
/// Entry identity is case-insensitive on Windows (where `C:\Foo` and `c:\foo`
/// name the same path), so a case-only variant is treated as the same entry and
/// never duplicated or double-removed. This is where the path-list merge and
/// removal rules live, so callers cannot hand-roll the separator.
#[derive(Clone, Debug)]
pub struct PathValue {
    string: String,
}

impl PathValue {
    /// Wrap a stored raw value for platform-aware list queries.
    pub fn new(value: String) -> Self {
        PathValue { string: value }
    }

    /// The separator used between entries on this platform.
    pub fn separator() -> &'static str {
        PATH_SEP
    }

    /// A joined value from the given entries, for callers building expectations.
    /// Empty segments are skipped.
    pub fn join<'a>(entries: impl IntoIterator<Item = &'a str>) -> String {
        entries
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(PATH_SEP)
    }

    /// The canonical identity of one entry, as compared across entries.
    fn canon(entry: &str) -> String {
        let trimmed = entry.trim();
        if cfg!(windows) {
            trimmed.to_lowercase()
        } else {
            trimmed.to_owned()
        }
    }

    /// Whether `entry` already appears in this list.
    fn contains(&self, entry: &str) -> bool {
        let want = Self::canon(entry);
        self.string
            .split(PATH_SEP)
            .any(|seg| Self::canon(seg) == want)
    }

    /// The list with `entry` prepended, de-duplicated against existing entries.
    pub fn prepend(&self, entry: &str) -> String {
        self.offer(entry, true)
    }

    /// The list with `entry` appended, de-duplicated against existing entries.
    pub fn append(&self, entry: &str) -> String {
        self.offer(entry, false)
    }

    /// The list with `entry` removed on this platform's identity rules; `None`
    /// when removing it would leave the list empty (the variable should vanish).
    pub fn remove(&self, entry: &str) -> Option<String> {
        let want = Self::canon(entry);
        let kept: Vec<&str> = self
            .string
            .split(PATH_SEP)
            .map(str::trim)
            .filter(|seg| !seg.is_empty() && Self::canon(seg) != want)
            .collect();
        if kept.is_empty() {
            None
        } else {
            Some(kept.join(PATH_SEP))
        }
    }

    /// Add `entry` at the front or back unless it is already present.
    fn offer(&self, entry: &str, front: bool) -> String {
        if self.contains(entry) {
            self.string.clone()
        } else if self.string.trim().is_empty() {
            entry.to_string()
        } else if front {
            format!("{}{}{}", entry, PATH_SEP, self.string)
        } else {
            format!("{}{}{}", self.string, PATH_SEP, entry)
        }
    }
}

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
) -> Result<Vec<(EnvVar, Option<String>)>> {
    let mut snapshots = Vec::new();
    for delta in deltas {
        let old = store.read(&delta.name)?;
        let next = merge(old.as_deref(), delta);
        store.write(&delta.name, &next)?;
        snapshots.push((delta.name.clone(), old));
    }
    Ok(snapshots)
}

/// Reverse an applied delta against the current value, so uninstall undoes exactly
/// what one install added. Returns what the variable should become; `None` means it
/// no longer exists and should be removed.
///
/// `Set` restores the pre-apply snapshot (`old`), because a `Set` owns the whole
/// value. `Prepend`/`Append` drop only the applied segment from the *current*
/// value, so segments added by later installs survive any uninstall order.
pub fn reverse_value(
    applied: &EnvDelta,
    old: Option<&str>,
    current: Option<&str>,
) -> Option<String> {
    match applied.op {
        EnvOp::Set => old.map(str::to_owned),
        EnvOp::Prepend | EnvOp::Append => {
            let current = current?.to_string();
            PathValue::new(current).remove(&applied.value)
        }
    }
}

/// Write the reverse of one applied delta to `store` (used by uninstall).
pub fn undo_delta(applied: &EnvDelta, old: Option<&str>, store: &dyn EnvStore) -> Result<()> {
    let current = store.read(&applied.name)?;
    match reverse_value(applied, old, current.as_deref()) {
        Some(next) => store.write(&applied.name, &next),
        None => store.remove(&applied.name),
    }
}

fn merge(current: Option<&str>, delta: &EnvDelta) -> String {
    let list = PathValue::new(current.unwrap_or("").to_string());
    match delta.op {
        EnvOp::Set => delta.value.clone(),
        EnvOp::Prepend => list.prepend(&delta.value),
        EnvOp::Append => list.append(&delta.value),
    }
}

/// A purely in-memory env backend, used to test that compensation really
/// restores the old values.
#[derive(Default, Debug)]
pub struct MemEnvStore {
    values: RefCell<HashMap<EnvVar, String>>,
}

impl MemEnvStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from the given snapshot, handy for assertions.
    #[cfg(test)]
    pub fn snapshot(&self) -> HashMap<EnvVar, String> {
        self.values.borrow().clone()
    }
}

impl EnvStore for MemEnvStore {
    fn read(&self, name: &EnvVar) -> Result<Option<String>> {
        Ok(self.values.borrow().get(name).cloned())
    }
    fn write(&self, name: &EnvVar, value: &str) -> Result<()> {
        self.values
            .borrow_mut()
            .insert(name.clone(), value.to_string());
        Ok(())
    }
    fn remove(&self, name: &EnvVar) -> Result<()> {
        self.values.borrow_mut().remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
