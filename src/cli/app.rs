//! Command handlers behind the `wanted` CLI front end.
//!
//! Each `Commands` variant in `main.rs` dispatches to one handler here. Keeping
//! the front end (`main.rs`) to parsing and dispatch keeps a single binary under
//! the per-file line budget and lets the handlers stay unit-testable without a
//! `clap` parse.

use std::path::PathBuf;

use crate::Version;
use crate::cli::ToolSpec;
use crate::engine::execute;
use crate::engine::{Ctx, Fs, RealDownloader, RealFs, RealProcess};
use crate::env::store::RealEnvStore;
use crate::fs_path::{AppDir, DirName};
use crate::plugin::Manifest;
use crate::receipt::{Receipt, VarSnapshot};
use crate::report::terminal::TerminalReporter;
use crate::store::Store;

/// Register a plugin manifest into `plugins/` next to the executable, so
/// `install` can invoke it by name. The manifest is read from a local path, or
/// fetched as `<name>.toml` from a plugin registry when the argument is not an
/// existing file.
pub fn add_plugin(target: &str, registry: Option<&str>) -> crate::Result<()> {
    let source = crate::cli::resolve_add_source(target, registry);
    let (label, data) = match source {
        crate::cli::PluginSource::Local(path) => (
            path.display().to_string(),
            std::fs::read(&path).map_err(|e| crate::error::io_err(path.clone(), e))?,
        ),
        crate::cli::PluginSource::Registry { url, .. } => (url.clone(), fetch_plugin(&url)?),
    };
    let manifest = Manifest::parse(bytes_to_manifest(&data)?)?;
    let dest = plugins_dir().join(format!("{}.toml", manifest.meta.name));
    let fs = RealFs;
    fs.write(&dest, &data)?;
    println!("added plugin {} (from {label})", manifest.meta.name);
    Ok(())
}

/// Download a plugin manifest from a raw URL in full.
fn fetch_plugin(url: &str) -> crate::Result<Vec<u8>> {
    use std::io::Read;
    let response = ureq::get(url)
        .call()
        .map_err(|e| crate::Error::Network(format!("failed to fetch {url}: {e}")))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| crate::Error::Network(format!("failed to read {url}: {e}")))?;
    Ok(bytes)
}

/// Interpret plugin bytes as a manifest source string.
fn bytes_to_manifest(data: &[u8]) -> crate::Result<&str> {
    std::str::from_utf8(data)
        .map_err(|e| crate::Error::Other(format!("plugin is not valid UTF-8: {e}")))
}

/// Install a single tool: resolve `latest`, build the plan chain, execute it,
/// and persist the receipt so uninstall can reverse the effect.
pub fn install(
    spec: &ToolSpec,
    manifest_override: Option<PathBuf>,
    asset_source: Option<&str>,
    with: &[String],
    workers: usize,
) -> crate::Result<()> {
    let name = spec.name().clone();
    let store = store_at_cwd();
    let manifest_path =
        manifest_override.unwrap_or_else(|| plugins_dir().join(format!("{name}.toml")));
    let manifest = Manifest::load(&manifest_path)?;

    let mut version = spec.version().clone();
    if version == Version::Latest {
        version = resolve_latest(&manifest.install, asset_source)?;
    }

    let selection = crate::engine::plan::Selection {
        source: asset_source,
        components: with,
    };
    let plans = manifest.plan_chain(
        store.root(),
        &crate::plugin::Target::current(),
        &version,
        &selection,
    );
    if plans.is_empty() {
        return Err(crate::Error::Other(format!(
            "no install method available for {name} on this platform"
        )));
    }

    let fs = RealFs;
    let downloader = RealDownloader::with_workers(workers);
    let process = RealProcess;
    let env = RealEnvStore::new()?;
    let snapshots = env_snapshots_for_chain(&plans, &env)?;
    let reporter = TerminalReporter::new(plans[0].name.as_str());
    let ctx = Ctx {
        root: store.root().to_path_buf(),
        fs: &fs,
        downloader: &downloader,
        runner: &process,
        env: &env,
        reporter: &reporter,
    };
    let chosen = execute::execute_chain(&plans, &ctx)?;
    let plan = &plans[chosen];
    reporter.finish();

    let receipt = Receipt {
        name: plan.name.to_string(),
        version: plan.version.clone(),
        app_dir: AppDir::from_path(&plan.dest_dir),
        vars: snapshots,
    };
    receipt.write(&fs, &store.receipt_path(&plan.name))?;

    println!("installed {} {}", plan.name, plan.version);
    Ok(())
}

/// Resolve a bare `latest` to the newest version the source supports: its
/// inline `versions` list first, else its live `versions_source` endpoint. Keeps
/// `latest` (literal `{version}` substitution) when the source declares neither.
fn resolve_latest(
    install: &crate::plugin::Install,
    source: Option<&str>,
) -> crate::Result<Version> {
    if let Some(resolved) = install.latest_for(source) {
        return resolved;
    }
    match install.versions_source_for(source) {
        Some(versions_source) => {
            let list = crate::plugin::VersionsSource::fetch(versions_source)?;
            crate::version::pick_latest(&list)
        }
        None => Ok(Version::Latest),
    }
}

/// Snapshot the pre-apply value of every variable a plan chain will write, said
/// values being read before any execution mutates the environment.
///
/// The deltas are identical across a plan chain (every plan computes them from
/// the same manifest `env` section and base dir), so the chain's first plan is a
/// faithful stand-in even when a later fallback plan ultimately succeeds. This
/// lets the receipt record old values once, up front, while the receipt still
/// names the chosen plan afterwards.
fn env_snapshots_for_chain(
    plans: &[crate::engine::plan::Plan],
    env: &dyn crate::env::EnvStore,
) -> crate::Result<Vec<VarSnapshot>> {
    let mut out = Vec::new();
    for delta in plans[0].env_deltas() {
        out.push(VarSnapshot {
            name: delta.name.clone(),
            op: delta.op,
            value: delta.value.clone(),
            old: env.read(&delta.name)?,
        });
    }
    Ok(out)
}

/// Remove a registered plugin manifest (the inverse of `add`).
pub fn remove_plugin(name: &DirName) -> crate::Result<()> {
    let fs = RealFs;
    let path = plugins_dir().join(format!("{name}.toml"));
    if !fs.exists(&path)? {
        return Err(crate::Error::Other(format!("plugin {name} not registered")));
    }
    fs.remove_file(&path)?;
    println!("removed plugin {name}");
    Ok(())
}

/// Uninstall a tool: restore its environment from the receipt and clear the app
/// directory.
pub fn uninstall(name: &DirName) -> crate::Result<()> {
    let store = store_at_cwd();
    let fs = RealFs;
    let env = RealEnvStore::new()?;
    let receipt_path = store.receipt_path(name);
    match Receipt::read(&fs, &receipt_path)? {
        None => {
            let fallback = store.root().join(name.as_str());
            if fs.exists(&fallback)? {
                fs.remove_dir_all(&fallback)?;
            }
            println!("uninstalled {name} (no receipt; environment left untouched)");
            Ok(())
        }
        Some(receipt) => {
            crate::uninstall::apply_receipt(&receipt, &fs, &env)?;
            crate::uninstall::remove_receipt(&fs, &receipt_path)?;
            println!("uninstalled {} {}", receipt.name, receipt.version);
            Ok(())
        }
    }
}

/// Upgrade `wanted` itself from the latest GitHub release.
pub fn upgrade() -> crate::Result<()> {
    let fs = RealFs;
    let downloader = RealDownloader::default();
    let reporter = TerminalReporter::new("wanted");
    match crate::upgrade::Upgrader::upgrade(&fs, &downloader, &reporter) {
        Ok(crate::upgrade::UpgradeOutcome::UpToDate) => {
            println!("wanted is up to date");
            Ok(())
        }
        Ok(crate::upgrade::UpgradeOutcome::Upgraded(version)) => {
            println!("upgraded wanted to {version}");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Print the installed tool names, one per line.
pub fn list() -> crate::Result<()> {
    let store = store_at_cwd();
    for name in store.list_installed(&RealFs)? {
        println!("{name}");
    }
    Ok(())
}

/// Print the versions each plugin source supports, marking the newest (`latest`).
/// Sources with an inline list are shown from it; sources declaring a
/// `versions_source` endpoint are fetched live. With `--source`, only that
/// source's versions are shown.
pub fn versions(tool: &str, source: Option<&str>) -> crate::Result<()> {
    let manifest = Manifest::load(&plugins_dir().join(format!("{tool}.toml")))?;
    let install = &manifest.install;
    let print = |name: &str, list: &[String]| {
        println!("{name}:");
        for version in list {
            println!("  {version}");
        }
        match crate::version::pick_latest(list) {
            Ok(latest) => println!("  (latest → {latest})"),
            Err(error) => eprintln!("  ({error})"),
        }
    };
    match source {
        Some(name) => {
            let list = versions_for(install, Some(name), tool)?;
            print(name, &list);
        }
        None => {
            let names = declared_sources(install);
            if names.is_empty() {
                println!("{tool} declares no versions");
                return Ok(());
            }
            for name in names {
                let list = versions_for(install, Some(name), tool)?;
                print(name, &list);
            }
        }
    }
    Ok(())
}

/// The supported version strings for one source: its inline list, else its
/// `versions_source` endpoint fetched live when declared.
fn versions_for(
    install: &crate::plugin::Install,
    source: Option<&str>,
    tool: &str,
) -> crate::Result<Vec<String>> {
    if let Some(list) = install.versions_for(source) {
        return Ok(list.clone());
    }
    let name = source.unwrap_or("default");
    let versions_source = install
        .versions_source_for(source)
        .ok_or_else(|| crate::Error::Other(format!("no source {name} for {tool}")))?;
    crate::plugin::VersionsSource::fetch(versions_source)
}

/// The sorted union of source names that declare versions (inline or remote).
fn declared_sources(install: &crate::plugin::Install) -> Vec<&str> {
    let mut names = Vec::new();
    for name in install.versions.keys() {
        names.push(name.as_str());
    }
    for name in install.versions_source.keys() {
        if !names.contains(&name.as_str()) {
            names.push(name.as_str());
        }
    }
    names.sort_unstable();
    names
}

/// The `.wanted` record directory beside the current working directory.
fn store_at_cwd() -> Store {
    let root = std::env::current_dir().unwrap_or_default();
    Store::new(root)
}

/// The directory holding plugin manifests: a `plugins` subdirectory next to the
/// executable, so it can ship with the binary.
fn plugins_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("plugins")))
        .unwrap_or_else(|| PathBuf::from("plugins"))
}

/// The directory holding this executable, added to PATH by `wanted env`.
fn self_dir() -> crate::Result<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .ok_or_else(|| crate::Error::Other("cannot locate the wanted executable".into()))
}

/// Put `wanted` itself on PATH so it can be called directly from any shell.
pub fn add_self_to_path() -> crate::Result<()> {
    let dir = self_dir()?;
    let path = dir.to_string_lossy();
    let store = RealEnvStore::new()?;
    if crate::env::add_to_path(&path, &store)? {
        println!("added {path} to PATH");
    } else {
        println!("{path} is already on PATH");
    }
    Ok(())
}
