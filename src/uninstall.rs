//! Uninstall: roll back one install precisely from its receipt.
//!
//! Every side effect is reversible: remove the tool directory and write each
//! environment variable back to its pre-apply snapshot. The logic can be
//! unit-tested against the in-memory filesystem and env backend (same idea as
//! the execution layer).

use std::path::Path;

use crate::Result;
use crate::engine::fs::Fs;
use crate::env::EnvStore;
use crate::receipt::Receipt;

/// Roll back from a receipt: remove the tool directory, then restore each
/// variable to its old value.
pub fn apply_receipt(receipt: &Receipt, fs: &dyn Fs, env: &dyn EnvStore) -> Result<()> {
    remove_app_dir(fs, Path::new(&receipt.app_dir))?;
    for var in &receipt.vars {
        restore_var(env, &var.name, var.old.as_deref())?;
    }
    Ok(())
}

fn remove_app_dir(fs: &dyn Fs, dir: &Path) -> Result<()> {
    if fs.exists(dir)? {
        fs.remove_dir_all(dir)?;
    }
    Ok(())
}

/// Restore one variable to its old value; `None` means it did not exist, so delete it.
fn restore_var(env: &dyn EnvStore, name: &str, old: Option<&str>) -> Result<()> {
    match old {
        Some(value) => env.write(name, value),
        None => env.remove(name),
    }
}

/// Remove the receipt and its directory after uninstalling.
pub fn remove_receipt(fs: &dyn Fs, receipt_path: &Path) -> Result<()> {
    if let Some(parent) = receipt_path.parent()
        && fs.exists(parent)?
    {
        fs.remove_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
