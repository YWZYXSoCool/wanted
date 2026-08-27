//! Uninstall: roll back one install precisely from its receipt.
//!
//! Every side effect is reversible: remove the tool directory and write each
//! environment variable back to its pre-apply snapshot. The logic can be
//! unit-tested against the in-memory filesystem and env backend (same idea as
//! the execution layer).

use std::path::Path;

use crate::Result;
use crate::engine::fs::Fs;
use crate::env::{self, EnvDelta, EnvStore};
use crate::receipt::Receipt;

/// Roll back from a receipt: remove the tool directory, then reverse each
/// variable. PATH-like variables drop only the segment this install added; `Set`
/// restores the pre-apply value.
pub fn apply_receipt(receipt: &Receipt, fs: &dyn Fs, env: &dyn EnvStore) -> Result<()> {
    remove_app_dir(fs, Path::new(&receipt.app_dir))?;
    for var in &receipt.vars {
        let applied = EnvDelta {
            name: var.name.clone(),
            value: var.value.clone(),
            op: var.op,
        };
        env::undo_delta(&applied, var.old.as_deref(), env)?;
    }
    Ok(())
}

fn remove_app_dir(fs: &dyn Fs, dir: &Path) -> Result<()> {
    if fs.exists(dir)? {
        fs.remove_dir_all(dir)?;
    }
    Ok(())
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
