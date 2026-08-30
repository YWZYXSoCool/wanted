use std::path::PathBuf;

use clap::{Parser, Subcommand};
use wanted::cli::ToolSpec;
use wanted::engine::execute;
use wanted::engine::{Ctx, DEFAULT_PARALLEL_WORKERS, Fs, RealDownloader, RealFs, RealProcess};
use wanted::env::store::RealEnvStore;
use wanted::fs_path::{AppDir, DirName};
use wanted::plugin::Manifest;
use wanted::receipt::{Receipt, VarSnapshot};
use wanted::report::{Progress, Reporter};
use wanted::store::Store;

#[derive(Parser)]
#[command(name = "wanted")]
#[command(about = "Development environment installer.", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a plugin manifest, making a tool installable.
    #[command(alias = "a")]
    Add {
        /// A local plugin `.toml` path, or a tool name to fetch as `<name>.toml` from the registry.
        plugin: String,
        /// Registry base URL to fetch from (default: the GitHub `wanted-registry`).
        #[arg(long)]
        registry: Option<String>,
    },
    /// Install a tool by invoking its plugin (`name@version`).
    #[command(alias = "i")]
    Install {
        /// Tool spec, optionally pinned as `name@version`.
        tools: Vec<ToolSpec>,
        /// Override the plugin manifest path (default `plugins/<name>.toml`).
        #[arg(long)]
        source: Option<PathBuf>,
        /// Named asset source to download from (e.g. a mirror); defaults to the plugin's `default` source.
        #[arg(long)]
        asset_source: Option<String>,
        /// Optional component to also download (repeatable, e.g. `--with clang`).
        #[arg(long = "with", value_name = "COMPONENT")]
        with: Vec<String>,
        /// Number of concurrent connections used to download a large asset.
        #[arg(long, default_value_t = DEFAULT_PARALLEL_WORKERS)]
        workers: usize,
    },
    /// Remove a registered plugin manifest.
    #[command(alias = "rm")]
    Remove { name: String },
    /// Uninstall an installed tool and restore its environment.
    #[command(alias = "un")]
    Uninstall { name: String },
    /// Upgrade wanted itself.
    Upgrade,
    /// List installed tools.
    #[command(alias = "ls")]
    List,
    /// Add wanted's own directory to PATH so `wanted` is callable directly.
    #[command(alias = "use")]
    Env,
}

fn main() {
    // Ctrl+C must trigger a graceful rollback, not an OS kill: signal the shared
    // cancel flag so in-flight downloads abort and `execute` returns an error,
    // which routes through the normal compensation path. Best-effort — a failed
    // handler install (rare) leaves default Ctrl+C behavior intact.
    let _ = ctrlc::set_handler(|| {
        wanted::engine::cancel_flag().store(true, std::sync::atomic::Ordering::SeqCst)
    });
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> wanted::Result<()> {
    let _ = wanted::upgrade::Upgrader::cleanup_stale(&RealFs);
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return Ok(());
    };
    match command {
        Commands::Add { plugin, registry } => add_plugin(&plugin, registry.as_deref()),
        Commands::Install {
            tools,
            source,
            asset_source,
            with,
            workers,
        } => {
            for tool in tools {
                install(
                    &tool,
                    source.clone(),
                    asset_source.as_deref(),
                    &with,
                    workers,
                )?;
            }
            Ok(())
        }
        Commands::Remove { name } => remove_plugin(&DirName::try_from(name)?),
        Commands::Uninstall { name } => uninstall(&DirName::try_from(name)?),
        Commands::Upgrade => upgrade(),
        Commands::List => list(),
        Commands::Env => add_self_to_path(),
    }
}

/// Register a plugin manifest into `plugins/` next to the executable, so
/// `install` can invoke it by name. The manifest is read from a local path, or
/// fetched as `<name>.toml` from a plugin registry when the argument is not an
/// existing file.
fn add_plugin(target: &str, registry: Option<&str>) -> wanted::Result<()> {
    let source = wanted::cli::resolve_add_source(target, registry);
    let (label, data) = match source {
        wanted::cli::PluginSource::Local(path) => (
            path.display().to_string(),
            std::fs::read(&path).map_err(|e| wanted::error::io_err(path.clone(), e))?,
        ),
        wanted::cli::PluginSource::Registry { url, .. } => (url.clone(), fetch_plugin(&url)?),
    };
    let manifest = Manifest::parse(bytes_to_manifest(&data)?)?;
    let dest = plugins_dir().join(format!("{}.toml", manifest.meta.name));
    let fs = RealFs;
    fs.write(&dest, &data)?;
    println!("added plugin {} (from {label})", manifest.meta.name);
    Ok(())
}

/// Download a plugin manifest from a raw URL in full.
fn fetch_plugin(url: &str) -> wanted::Result<Vec<u8>> {
    use std::io::Read;
    let response = ureq::get(url)
        .call()
        .map_err(|e| wanted::Error::Network(format!("failed to fetch {url}: {e}")))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| wanted::Error::Network(format!("failed to read {url}: {e}")))?;
    Ok(bytes)
}

/// Interpret plugin bytes as a manifest source string.
fn bytes_to_manifest(data: &[u8]) -> wanted::Result<&str> {
    std::str::from_utf8(data)
        .map_err(|e| wanted::Error::Other(format!("plugin is not valid UTF-8: {e}")))
}

fn install(
    spec: &ToolSpec,
    manifest_override: Option<PathBuf>,
    asset_source: Option<&str>,
    with: &[String],
    workers: usize,
) -> wanted::Result<()> {
    let name = spec.name();
    let version = spec.version();
    let store = store_at_cwd();

    let manifest_path =
        manifest_override.unwrap_or_else(|| plugins_dir().join(format!("{name}.toml")));
    let manifest = Manifest::load(&manifest_path)?;

    let selection = wanted::engine::plan::Selection {
        source: asset_source,
        components: with,
    };
    let plans = manifest.plan_chain(
        store.root(),
        &wanted::plugin::Target::current(),
        version,
        &selection,
    );
    if plans.is_empty() {
        return Err(wanted::Error::Other(format!(
            "no install method available for {name} on this platform"
        )));
    }

    let fs = RealFs;
    let downloader = RealDownloader::with_workers(workers);
    let process = RealProcess;
    let env = RealEnvStore::new()?;
    let ref_plan = &plans[0];
    let snapshots = env_snapshots(ref_plan, &env)?;
    let reporter = TerminalReporter::new(ref_plan.name.as_str());
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

/// Capture the applied delta and pre-apply value of every variable the plan will
/// write, so the receipt can reverse each one precisely on uninstall.
fn env_snapshots(
    plan: &wanted::engine::plan::Plan,
    env: &dyn wanted::env::EnvStore,
) -> wanted::Result<Vec<VarSnapshot>> {
    let mut out = Vec::new();
    for delta in plan.env_deltas() {
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
fn remove_plugin(name: &DirName) -> wanted::Result<()> {
    let fs = RealFs;
    let path = plugins_dir().join(format!("{name}.toml"));
    if !fs.exists(&path)? {
        return Err(wanted::Error::Other(format!(
            "plugin {name} not registered"
        )));
    }
    fs.remove_file(&path)?;
    println!("removed plugin {name}");
    Ok(())
}

/// Uninstall a tool: restore its environment from the receipt and clear the app
/// directory.
fn uninstall(name: &DirName) -> wanted::Result<()> {
    let store = store_at_cwd();
    let fs = RealFs;
    let env = RealEnvStore::new()?;
    let receipt_path = store.receipt_path(name);
    match Receipt::read(&fs, &receipt_path)? {
        None => {
            let fallback = store.apps_dir().join(name);
            if fs.exists(&fallback)? {
                fs.remove_dir_all(&fallback)?;
            }
            println!("uninstalled {name} (no receipt; environment left untouched)");
            Ok(())
        }
        Some(receipt) => {
            wanted::uninstall::apply_receipt(&receipt, &fs, &env)?;
            wanted::uninstall::remove_receipt(&fs, &receipt_path)?;
            println!("uninstalled {} {}", receipt.name, receipt.version);
            Ok(())
        }
    }
}

/// Upgrade `wanted` itself from the latest GitHub release.
fn upgrade() -> wanted::Result<()> {
    let fs = RealFs;
    let downloader = RealDownloader::default();
    let reporter = TerminalReporter::new("wanted");
    match wanted::upgrade::Upgrader::upgrade(&fs, &downloader, &reporter) {
        Ok(wanted::upgrade::UpgradeOutcome::UpToDate) => {
            println!("wanted is up to date");
            Ok(())
        }
        Ok(wanted::upgrade::UpgradeOutcome::Upgraded(version)) => {
            println!("upgraded wanted to {version}");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Render engine progress events as a multi-line terminal display.
///
/// Real downloads often lack `Content-Length`, so the total is unknown; forcing a
/// deterministic progress bar would render an empty "0 B/0 B" bar at full width.
/// Hence an indeterminate spinner line plus a live byte count, shown alongside a
/// persistent status line (tool name, then the active phase) — two concurrently
/// visible progress texts, decoupled from the single aggregated byte total the
/// engine reports.
struct TerminalReporter {
    panel: indicatif::MultiProgress,
    status: indicatif::ProgressBar,
    spinner: indicatif::ProgressBar,
}

impl TerminalReporter {
    fn new(tool: &str) -> Self {
        let panel = indicatif::MultiProgress::new();
        let status = panel.add(indicatif::ProgressBar::new(0).with_style(Self::status_style()));
        status.set_message(format!("installing {tool}"));
        let spinner =
            panel.add(indicatif::ProgressBar::new_spinner().with_style(Self::spinner_style()));
        Self {
            panel,
            status,
            spinner,
        }
    }

    /// A plain status line with no bar or spinner.
    fn status_style() -> indicatif::ProgressStyle {
        indicatif::ProgressStyle::with_template("{msg}").expect("static status template is valid")
    }

    /// The live-byte spinner line.
    fn spinner_style() -> indicatif::ProgressStyle {
        indicatif::ProgressStyle::with_template("{spinner:.cyan} {bytes}")
            .expect("static spinner template is valid")
            .tick_chars("|/-\\")
    }

    /// Clear every progress line so the final message prints cleanly.
    fn finish(&self) {
        let _ = self.panel.clear();
    }
}

impl Reporter for TerminalReporter {
    fn report(&self, event: Progress) {
        match event {
            Progress::Phase(label) => {
                self.status.set_message(label.to_string());
                self.spinner.set_position(0);
            }
            Progress::Bytes { done, .. } => {
                self.spinner.set_position(done);
                self.spinner.tick();
            }
        }
    }
}

fn list() -> wanted::Result<()> {
    let store = store_at_cwd();
    for name in store.list_installed(&RealFs)? {
        println!("{name}");
    }
    Ok(())
}

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
fn self_dir() -> wanted::Result<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .ok_or_else(|| wanted::Error::Other("cannot locate the wanted executable".into()))
}

/// Put `wanted` itself on PATH so it can be called directly from any shell.
fn add_self_to_path() -> wanted::Result<()> {
    let dir = self_dir()?;
    let path = dir.to_string_lossy();
    let store = RealEnvStore::new()?;
    if wanted::env::add_to_path(&path, &store)? {
        println!("added {path} to PATH");
    } else {
        println!("{path} is already on PATH");
    }
    Ok(())
}
