use std::path::PathBuf;

use clap::{Parser, Subcommand};
use wanted::cli::ToolSpec;
use wanted::engine::execute;
use wanted::engine::{Ctx, Fs, RealDownloader, RealFs};
use wanted::env::store::RealEnvStore;
use wanted::plugin::Manifest;
use wanted::receipt::{Receipt, VarSnapshot};
use wanted::report::{Progress, Reporter};
use wanted::store::Store;

#[derive(Parser)]
#[command(name = "wanted")]
#[command(about = "Development environment installer.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a plugin manifest, making a tool installable.
    #[command(alias = "a")]
    Add {
        /// Path to a plugin `.toml` to register.
        plugin: PathBuf,
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
    },
    /// Update an installed tool or plugin.
    #[command(alias = "u")]
    Update {
        /// Target to update (`tool` or `plugin`).
        target: String,
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
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> wanted::Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return Ok(());
    };
    match command {
        Commands::Add { plugin } => add_plugin(&plugin),
        Commands::Install {
            tools,
            source,
            asset_source,
        } => {
            for tool in tools {
                install(&tool, source.clone(), asset_source.as_deref())?;
            }
            Ok(())
        }
        Commands::Update { target } => {
            println!("update {target}: not yet implemented in M0");
            Ok(())
        }
        Commands::Remove { name } => remove_plugin(&name),
        Commands::Uninstall { name } => uninstall(&name),
        Commands::Upgrade => {
            println!("self-upgrade: not yet implemented in M0");
            Ok(())
        }
        Commands::List => list(),
    }
}

/// Register a local plugin manifest into `plugins/` next to the executable, so
/// `install` can invoke it by name.
fn add_plugin(source: &PathBuf) -> wanted::Result<()> {
    let manifest = Manifest::load(source)?;
    let dest = plugins_dir().join(format!("{}.toml", manifest.meta.name));
    let data = std::fs::read(source).map_err(|e| wanted::error::io_err(source.clone(), e))?;
    let fs = RealFs;
    fs.write(&dest, &data)?;
    println!("added plugin {}", manifest.meta.name);
    Ok(())
}

fn install(
    spec: &ToolSpec,
    manifest_override: Option<PathBuf>,
    asset_source: Option<&str>,
) -> wanted::Result<()> {
    let name = spec.name();
    let version = spec.version();
    let store = store_at_cwd();

    let manifest_path =
        manifest_override.unwrap_or_else(|| plugins_dir().join(format!("{name}.toml")));
    let manifest = Manifest::load(&manifest_path)?;

    let plan = manifest.plan(
        store.root(),
        &wanted::plugin::Target::current(),
        version,
        asset_source,
    )?;

    let fs = RealFs;
    let downloader = RealDownloader;
    let env = RealEnvStore::new();
    let snapshots = env_snapshots(&plan, &env)?;
    let reporter = TerminalReporter::new(&plan.name);
    let ctx = Ctx {
        root: store.root().to_path_buf(),
        fs: &fs,
        downloader: &downloader,
        env: &env,
        reporter: &reporter,
    };
    execute::execute(&plan, &ctx)?;
    reporter.bar.finish_and_clear();

    let receipt = Receipt {
        name: plan.name.clone(),
        version: plan.version.clone(),
        app_dir: plan.dest_dir.to_string_lossy().into_owned(),
        vars: snapshots,
    };
    receipt.write(&fs, &store.receipt_path(&plan.name))?;

    println!("installed {} {}", plan.name, plan.version);
    Ok(())
}

/// Snapshot the pre-apply values of the variables the plan will write, to land
/// in the receipt for rollback.
fn env_snapshots(
    plan: &wanted::engine::plan::Plan,
    env: &dyn wanted::env::EnvStore,
) -> wanted::Result<Vec<VarSnapshot>> {
    let mut out = Vec::new();
    for delta in plan.env_deltas() {
        out.push(VarSnapshot {
            name: delta.name.clone(),
            old: env.read(&delta.name)?,
        });
    }
    Ok(out)
}

/// Remove a registered plugin manifest (the inverse of `add`).
fn remove_plugin(name: &str) -> wanted::Result<()> {
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
fn uninstall(name: &str) -> wanted::Result<()> {
    let store = store_at_cwd();
    let fs = RealFs;
    let env = RealEnvStore::new();
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

/// Render engine progress events as a terminal spinner.
///
/// Real downloads often lack `Content-Length`, so the total is unknown; forcing a
/// deterministic progress bar would render an empty "0 B/0 B" bar at full width.
/// Hence a fixed indeterminate spinner: the phase label as the message, plus a
/// live byte count.
struct TerminalReporter {
    bar: indicatif::ProgressBar,
}

impl TerminalReporter {
    fn new(tool: &str) -> Self {
        let bar = indicatif::ProgressBar::new_spinner().with_style(Self::style());
        bar.set_message(format!("installing {tool}"));
        Self { bar }
    }

    fn style() -> indicatif::ProgressStyle {
        indicatif::ProgressStyle::with_template("{msg} {spinner:.cyan} {bytes}")
            .expect("static spinner template is valid")
            .tick_chars("|/-\\")
    }
}

impl Reporter for TerminalReporter {
    fn report(&self, event: Progress) {
        match event {
            Progress::Phase(label) => {
                self.bar.reset();
                self.bar.set_message(label.to_string());
            }
            Progress::Bytes { done, .. } => {
                self.bar.set_position(done);
                self.bar.tick();
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
