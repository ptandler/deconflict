use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "deconflict",
    about = "Resolve conflicted-copy-style sync files"
)]
pub(crate) struct Cli {
    #[arg(long, global = true)]
    pub(crate) config: Option<PathBuf>,
    #[arg(long, global = true)]
    pub(crate) workers: Option<usize>,
    #[arg(long, global = true)]
    pub(crate) max_in_flight: Option<usize>,
    #[arg(long)]
    pub(crate) version: bool,
    #[arg(help = "Directories for bare interactive mode")]
    pub(crate) dirs: Vec<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Show the version
    Version,
    /// List or list enabled conflict patterns
    Patterns {
        #[arg(long)]
        enabled: bool,
    },
    /// Write a documented default config
    InitConfig {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Analyze setup and external tools
    Setup,
    /// Print effective configuration
    Config(ConfigArgs),
    /// Manage the file cache
    Cache(CacheArgs),
    /// Scan directories and list conflict groups
    Scan(ScanArgs),
    /// Resolve conflict groups
    Resolve(ResolveArgs),
}

#[derive(Args)]
pub(crate) struct ConfigArgs {
    #[arg(long, global = true)]
    pub(crate) path: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Option<ConfigCommands>,
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommands {
    /// Show tool detection and status
    Tools,
}

#[derive(Args)]
pub(crate) struct CacheArgs {
    #[command(subcommand)]
    pub(crate) command: CacheCommands,
}

#[derive(Subcommand)]
pub(crate) enum CacheCommands {
    /// Remove stale cache files
    Clean {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        older_than_days: Option<f64>,
    },
}

#[derive(Args)]
pub(crate) struct ScanArgs {
    #[arg(help = "Directories to scan (default: config default_dirs)")]
    pub(crate) dirs: Vec<PathBuf>,
    #[arg(long)]
    pub(crate) pattern: Option<String>,
    #[arg(long)]
    pub(crate) no_cache: bool,
    #[arg(long)]
    pub(crate) refresh: bool,
    #[arg(long)]
    pub(crate) progress: bool,
    #[arg(long)]
    pub(crate) no_progress: bool,
}

#[derive(Args)]
pub(crate) struct ResolveArgs {
    #[arg(help = "Directories to scan (default: config default_dirs)")]
    pub(crate) dirs: Vec<PathBuf>,
    #[arg(long)]
    pub(crate) pattern: Option<String>,
    #[arg(long, value_parser = ["base", "copy", "newest", "bigger", "recommended"])]
    pub(crate) auto: Option<String>,
    #[arg(long)]
    pub(crate) dry_run: bool,
    #[arg(long)]
    pub(crate) no_cache: bool,
    #[arg(long)]
    pub(crate) refresh: bool,
    #[arg(long)]
    pub(crate) progress: bool,
    #[arg(long)]
    pub(crate) no_progress: bool,
    #[arg(long)]
    pub(crate) tui: bool,
    #[arg(long)]
    pub(crate) no_tui: bool,
}
