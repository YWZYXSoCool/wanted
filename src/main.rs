use std::path::PathBuf;

use clap::{Parser, Subcommand};
use wanted::cli::ToolSpec;
use wanted::cli::app;
use wanted::engine::DEFAULT_PARALLEL_WORKERS;
use wanted::fs_path::DirName;

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
    /// List available versions declared by a plugin's sources (`latest` picks the newest).
    #[command(alias = "avail")]
    Versions {
        /// Plugin tool name.
        tool: String,
        /// Show only a named source's versions (default: all sources).
        #[arg(long)]
        source: Option<String>,
    },
    /// Add wanted's own directory to PATH so `wanted` is callable directly.
    #[command(alias = "use")]
    Env,
}

fn main() {
    install_signal_handler();
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

/// Arrange for Ctrl+C to trigger a graceful rollback instead of an OS kill.
///
/// The handler sets the engine's shared cancel flag so in-flight downloads abort
/// and `execute` returns an error, routing through the normal compensation path.
/// Best-effort: a failed handler install (rare) leaves the default Ctrl+C
/// behavior intact, hence the ignored result.
fn install_signal_handler() {
    let _ = ctrlc::set_handler(|| {
        wanted::engine::cancel_flag().store(true, std::sync::atomic::Ordering::SeqCst)
    });
}

fn run() -> wanted::Result<()> {
    let _ = wanted::upgrade::Upgrader::cleanup_stale(&wanted::engine::RealFs);
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return Ok(());
    };
    match command {
        Commands::Add { plugin, registry } => app::add_plugin(&plugin, registry.as_deref()),
        Commands::Install {
            tools,
            source,
            asset_source,
            with,
            workers,
        } => {
            for tool in tools {
                app::install(
                    &tool,
                    source.clone(),
                    asset_source.as_deref(),
                    &with,
                    workers,
                )?;
            }
            Ok(())
        }
        Commands::Remove { name } => app::remove_plugin(&DirName::try_from(name)?),
        Commands::Uninstall { name } => app::uninstall(&DirName::try_from(name)?),
        Commands::Upgrade => app::upgrade(),
        Commands::List => app::list(),
        Commands::Versions { tool, source } => app::versions(&tool, source.as_deref()),
        Commands::Env => app::add_self_to_path(),
    }
}
