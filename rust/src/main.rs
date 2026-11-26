use clap::{Args, Parser, Subcommand};
use deconflict_core::{
    analysis::analyze_scanned_group_cached,
    cache::{cache_key, clean_cache, FileCache},
    config::{Config, ScanConfig},
    media::{Tools as MediaTools},
    tools,
    patterns::Patterns,
    resolve::{prepare, Decision},
    scan::{scan, Event, ScanResult},
};
use std::{
    fs,
    io::{self, BufRead, IsTerminal},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Parser)]
#[command(name = "deconflict", about = "Resolve conflicted-copy-style sync files")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    workers: Option<usize>,
    #[arg(long, global = true)]
    max_in_flight: Option<usize>,
    #[arg(long)]
    version: bool,
    #[arg(help = "Directories for bare interactive mode")]
    dirs: Vec<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show the version
    Version,
    /// List or list enabled conflict patterns
    Patterns { #[arg(long)] enabled: bool },
    /// Write a documented default config
    InitConfig { #[arg(long)] path: Option<PathBuf> },
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
struct ConfigArgs {
    #[arg(long, global = true)]
    path: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<ConfigCommands>,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show tool detection and status
    Tools,
}

#[derive(Args)]
struct CacheArgs {
    #[command(subcommand)]
    command: CacheCommands,
}

#[derive(Subcommand)]
enum CacheCommands {
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
struct ScanArgs {
    #[arg(help = "Directories to scan (default: config default_dirs)")]
    dirs: Vec<PathBuf>,
    #[arg(long)]
    pattern: Option<String>,
    #[arg(long)]
    no_cache: bool,
    #[arg(long)]
    refresh: bool,
    #[arg(long)]
    progress: bool,
    #[arg(long)]
    no_progress: bool,
}

#[derive(Args)]
struct ResolveArgs {
    #[arg(help = "Directories to scan (default: config default_dirs)")]
    dirs: Vec<PathBuf>,
    #[arg(long)]
    pattern: Option<String>,
    #[arg(long, value_parser = ["base", "copy", "newest", "bigger", "recommended"])]
    auto: Option<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    no_cache: bool,
    #[arg(long)]
    refresh: bool,
    #[arg(long)]
    progress: bool,
    #[arg(long)]
    no_progress: bool,
    #[arg(long)]
    tui: bool,
    #[arg(long)]
    no_tui: bool,
}

fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn load_config(path: Option<&Path>) -> Result<(Config, ScanConfig), String> {
    let config = match path {
        Some(path) => {
            let text = fs::read_to_string(path)
                .map_err(|e| format!("could not read config {}: {e}", path.display()))?;
            Config::parse(&text).map_err(|e| format!("could not parse config: {e}"))?
        }
        None => {
            let xdg = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join(".config")
                });
            let path = xdg.join("deconflict/config.toml");
            match fs::read_to_string(&path) {
                Ok(text) => Config::parse(&text).unwrap_or_default(),
                Err(_) => Config::default(),
            }
        }
    };
    Ok((config.clone(), config.scan))
}

fn effective_scan(
    config_path: Option<&Path>,
    cli: &Cli,
    dirs: &[PathBuf],
    pattern: Option<&str>,
) -> Result<(ScanConfig, Patterns, Vec<PathBuf>, Config), String> {
    let (config, mut scan_config) = load_config(config_path)?;
    if let Some(workers) = cli.workers {
        scan_config.workers = workers;
    }
    if let Some(max) = cli.max_in_flight {
        scan_config.max_in_flight = max;
    }
    if !dirs.is_empty() {
        scan_config.default_dirs = dirs.to_vec();
    }
    if let Some(pattern) = pattern {
        scan_config.enabled_patterns = vec![pattern.to_string()];
        if pattern == "custom" {
            scan_config.patterns = config.scan.patterns.clone();
        }
    }
    if let Some(pattern) = pattern {
        if !matches!(
            pattern,
            "nextcloud" | "pacman" | "syncthing" | "dpkg" | "rpm" | "windows-copy"
        ) {
            let custom = scan_config.patterns.iter().find(|p| p.name == pattern);
            if custom.is_none() {
                return Err(format!("unknown pattern: {pattern}"));
            }
        }
    }
    let patterns = Patterns::build(&scan_config)?;
    let roots = if scan_config.default_dirs.is_empty() {
        Vec::new()
    } else {
        scan_config.default_dirs.clone()
    };
    Ok((scan_config, patterns, roots, config))
}

fn perform_scan(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    show_progress: bool,
) -> Result<ScanResult, String> {
    if roots.is_empty() {
        return Err("no directories. Pass dirs or set [scan].default_dirs".into());
    }
    let mut count = 0usize;
    scan(
        roots,
        scan_config,
        patterns,
        &AtomicBool::new(false),
        |event| {
            if show_progress {
                match event {
                    Event::File(path) => {
                        count += 1;
                        eprintln!("scanning ({count} files): {}", path.display());
                    }
                    Event::Provisional(_, _) => {}
                    Event::Error(path, error) => {
                        eprintln!("{}: {error}", path.display());
                    }
                }
            }
        },
    )
}

fn default_config_path() -> PathBuf {
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        });
    xdg.join("deconflict/config.toml")
}

fn general_cache(config: &Config) -> PathBuf {
    let value = config
        .general
        .get("cache_dir")
        .and_then(toml::Value::as_str)
        .unwrap_or("~/.cache/deconflict");
    expand(value)
}

fn general_backup(config: &Config) -> PathBuf {
    let value = config
        .general
        .get("backup_dir")
        .and_then(toml::Value::as_str)
        .unwrap_or("~/deconflict-backups");
    expand(value)
}

fn media_tools(config: &Config) -> MediaTools {
    let overrides = config
        .tools
        .iter()
        .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
        .collect();
    let detected = tools::detect(&overrides);
    MediaTools {
        exiftool: detected.get("exiftool").map(Path::to_path_buf),
        video_probe: detected.get("video_probe").map(Path::to_path_buf),
        office_text: detected.get("office_text").map(Path::to_path_buf),
    }
}

fn run_bare_cli(cli: &Cli) -> Result<(), String> {
    let (scan_config, patterns, roots, config) = effective_scan(
        cli.config.as_deref(), cli, &cli.dirs, None,
    )?;
    let show_progress = std::io::stdout().is_terminal();
    let result = perform_scan(&roots, &scan_config, &patterns, show_progress)?;
    if result.cancelled || !result.errors.is_empty() {
        return Err("scan incomplete; refusing to resolve".into());
    }
    if result.groups.is_empty() {
        println!("no conflicts found");
        return Ok(());
    }

    let backup = general_backup(&config);
    let key = cache_key(&roots).map_err(|e| format!("cannot key cache: {e}"))?;
    let mut file_cache = FileCache::new(general_cache(&config), &key, false);
    let media = media_tools(&config);
    let mut resolved = 0usize;
    let mut skipped = 0usize;

    for (index, group) in result.groups.iter().enumerate() {
        let stamped = analyze_scanned_group_cached(group, &mut file_cache, false, &media)
            .map_err(|e| format!("analysis failed for {}: {e}", group.base.display()))?;
        println!("\nGroup {}/{}", index + 1, result.groups.len());
        println!("base: {}", group.base.display());
        for (copy_index, (copy, _)) in group.conflicts.iter().enumerate() {
            println!("{}: {}", copy_index + 1, copy.display());
        }
        print!("(0) keep base, (1..) keep copy, (h) keep both, (s) skip, (q) quit: ");
        let _ = io::Write::flush(&mut io::stdout());

        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer).map_err(|e| e.to_string())?;
        let answer = answer.trim().to_lowercase();
        let decision = match answer.as_str() {
            "q" | "quit" | "exit" => {
                println!("aborted after {index} of {} group(s)", result.groups.len());
                break;
            }
            "s" | "skip" => { skipped += 1; continue; }
            "h" | "keep-both" => Decision::KeepBoth,
            "0" => Decision::KeepBase,
            value => match value.parse::<usize>() {
                Ok(copy_index) if copy_index > 0 && copy_index <= group.conflicts.len() =>
                    Decision::KeepCopy(copy_index - 1),
                _ => { println!("invalid choice"); continue; }
            },
        };

        let prepared = prepare(&result, index, &stamped.analysis, &roots)
            .map_err(|e| format!("cannot prepare {}: {e}", group.base.display()))?;
        let outcome = prepared.resolve(decision, &backup, false)
            .map_err(|e| format!("resolution failed for {}: {e}", group.base.display()))?;
        resolved += 1;
        for (source, destination) in outcome.operations {
            println!("{} -> {}", source.display(), destination.display());
        }
    }

    file_cache.clear().map_err(|e| format!("cannot invalidate cache: {e}"))?;
    println!("\nresolved {resolved} group(s)");
    if skipped > 0 { println!("({skipped} skipped)"); }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = run(cli);
    if let Err(message) = result {
        eprintln!("{message}");
        std::process::exit(2);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    if cli.version {
        println!("deconflict 0.2.9");
        return Ok(());
    }
    let Some(command) = &cli.command else {
        return run_bare_cli(&cli);
    };
    match command {
        Commands::Version => {
            println!("deconflict 0.2.9");
        }
        Commands::Patterns { enabled } => {
            let mut config = ScanConfig::default();
            if *enabled {
                config.enabled_patterns = vec!["nextcloud".into(), "pacman".into(), "syncthing".into()];
            }
            let patterns = Patterns::build(&config)?;
            println!("name\tenabled\twinner\tmatches");
            for name in ["nextcloud", "pacman", "syncthing", "dpkg", "rpm", "windows-copy"] {
                let is_enabled = config.enabled_patterns.iter().any(|p| p == name);
                if *enabled && !is_enabled {
                    continue;
                }
                println!("{name}\t{is_enabled}\tcopy\t");
            }
            let _ = patterns;
        }
        Commands::Scan(args) => {
            let (scan_config, patterns, roots, config) = effective_scan(
                cli.config.as_deref(),
                &cli,
                &args.dirs,
                args.pattern.as_deref(),
            )?;
            let result = perform_scan(&roots, &scan_config, &patterns, args.progress)?;
            if !result.errors.is_empty() {
                for (path, error) in result.errors {
                    eprintln!("{}: {}", path.display(), error);
                }
                return Err("scan completed with errors".into());
            }

            if !args.no_cache {
                let key = cache_key(&roots).map_err(|e| format!("cannot key cache: {e}"))?;
                let mut cache = FileCache::new(general_cache(&config), &key, false);
                if args.refresh {
                    cache.clear().map_err(|e| format!("cannot clear cache: {e}"))?;
                }
                let media = media_tools(&config);
                for group in &result.groups {
                    let _stamped = analyze_scanned_group_cached(
                        group,
                        &mut cache,
                        args.refresh,
                        &media,
                    );
                }
                cache.save().map_err(|e| format!("cannot save cache: {e}"))?;
            }

            if result.groups.is_empty() {
                println!("no conflicts found");
            } else {
                for group in &result.groups {
                    let pattern_name = group
                        .conflicts
                        .first()
                        .map(|(_, m)| m.pattern.as_str())
                        .unwrap_or("unknown");
                    println!("{}\t{}\t{} files", group.base.display(), pattern_name, group.conflicts.len());
                }
                println!("{} conflict group(s). Run `deconflict resolve` to act.", result.groups.len());
            }
        }
        Commands::Resolve(args) => {
            if args.auto.as_deref() == Some("copy") && args.dry_run {
                // Copy resolution is allowed; retain the documented no-write dry-run contract.
            }
            let (scan_config, patterns, roots, config) = effective_scan(
                cli.config.as_deref(),
                &cli,
                &args.dirs,
                args.pattern.as_deref(),
            )?;
            let result = perform_scan(&roots, &scan_config, &patterns, args.progress)?;
            if result.cancelled || !result.errors.is_empty() {
                return Err("scan incomplete; refusing to resolve".into());
            }
            if result.groups.is_empty() {
                println!("no conflicts to resolve");
                return Ok(());
            }
            let backup = general_backup(&config);
            let cache_dir = general_cache(&config);
            let key = cache_key(&roots).map_err(|e| format!("cannot key cache: {e}"))?;
            let mut file_cache = FileCache::new(
                &cache_dir,
                &key,
                args.dry_run || args.no_cache,
            );
            if args.refresh && !args.no_cache && !args.dry_run {
                file_cache.clear().map_err(|e| format!("cannot clear cache: {e}"))?;
            }

            let media = media_tools(&config);
            let mut resolved = 0;
            for (index, group) in result.groups.iter().enumerate() {
                let stamped = if args.no_cache {
                    deconflict_core::analysis::analyze_scanned_group_with_media(
                        group,
                        &media,
                    )
                } else {
                    analyze_scanned_group_cached(
                        group,
                        &mut file_cache,
                        args.refresh,
                        &media,
                    )
                }
                .map_err(|e| format!("analysis failed for {}: {e}", group.base.display()))?;
                let prepared = prepare(&result, index, &stamped.analysis, &roots)
                    .map_err(|e| format!("cannot prepare {}: {e}", group.base.display()))?;
                let rule = args.auto.as_deref().unwrap_or("base");
                let decision = match rule {
                    "base" => Decision::KeepBase,
                    "copy" => Decision::KeepCopy(0),
                    "newest" => match stamped.analysis.winner_hint() {
                        Some(deconflict_core::analysis::Recommendation::Copy(i)) => Decision::KeepCopy(i),
                        _ => Decision::KeepBase,
                    },
                    "bigger" => {
                        let base = stamped.analysis.base.as_ref();
                        let biggest = stamped.analysis.copies.iter().enumerate()
                            .filter(|(_, c)| base.is_none_or(|b| c.size > b.size))
                            .max_by_key(|(_, c)| c.size)
                            .map(|(i, _)| i);
                        biggest.map_or(Decision::KeepBase, Decision::KeepCopy)
                    }
                    "recommended" => match deconflict_core::analysis::recommend(
                        &stamped.analysis,
                        stamped.analysis.kind == "text",
                    ) {
                        Some(deconflict_core::analysis::Recommendation::Copy(i)) => Decision::KeepCopy(i),
                        Some(deconflict_core::analysis::Recommendation::Base) => Decision::KeepBase,
                        None => continue,
                    },
                    _ => return Err(format!("unknown auto rule: {rule}")),
                };
                let outcome = prepared.resolve(decision, &backup, args.dry_run)
                    .map_err(|e| format!("resolution failed for {}: {e}", group.base.display()))?;
                resolved += 1;
                if args.dry_run {
                    println!("dry-run: {} ({} operation(s))", group.base.display(), outcome.operations.len());
                } else {
                    println!("resolved: {}", group.base.display());
                }
            }
            if args.dry_run {
                println!("dry-run complete: {resolved} group(s)");
            } else if !args.no_cache {
                file_cache.clear().map_err(|e| format!("cannot invalidate cache: {e}"))?;
            }
        }
        Commands::InitConfig { path } => {
            let target = path.clone().unwrap_or_else(default_config_path);
            if target.exists() {
                return Err(format!("exists: {}", target.display()));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create config directory: {e}"))?;
            }
            fs::write(&target, include_str!("../../src/deconflict/config.example.toml"))
                .map_err(|e| format!("cannot write config file {}: {e}", target.display()))?;
            println!("wrote config: {}", target.display());
        }
        Commands::Setup => {
            let (config, _) = load_config(cli.config.as_deref())?;
            println!("deconflict 0.2.9");
            println!("  config file: {}", cli.config.clone().unwrap_or_else(default_config_path).display());
            println!("  backup dir:  {}", general_backup(&config).display());
            println!("  cache dir:   {}", general_cache(&config).display());
            let overrides = config
                .tools
                .iter()
                .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
                .collect();
            let detected = tools::detect(&overrides);
            println!("\nexternal tools:");
            let mut missing = 0;
            for name in detected.names() {
                match detected.get(name) {
                    Some(path) => println!("  {name}: {}", path.display()),
                    None => {
                        missing += 1;
                        println!("  {name}: missing");
                    }
                }
            }
            println!("\n{missing} tool(s) missing.");
        }
        Commands::Config(args) => {
            let path = args.path.clone().or_else(|| cli.config.clone());
            let (config, scan) = load_config(path.as_deref())?;
            if matches!(&args.command, Some(ConfigCommands::Tools)) {
                let overrides = config
                    .tools
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
                    .collect();
                let detected = tools::detect(&overrides);
                for kind in ["text", "other", "audio", "image", "video", "office", "kdbx"] {
                    let role = match kind {
                        "audio" => "mp3_editor",
                        "image" => "image_viewer",
                        "video" => "viewer",
                        "office" => "office",
                        "kdbx" => "keepass",
                        _ => "editor",
                    };
                    match detected.get(role) {
                        Some(path) => println!("{kind}: {role}: {}", path.display()),
                        None => println!("{kind}: {role}: missing"),
                    }
                }
            } else {
                println!("config file: {}", path.unwrap_or_else(default_config_path).display());
                println!("[scan]");
                println!("  default_dirs = {:?}", scan.default_dirs);
                println!("  enabled_patterns = {:?}", scan.enabled_patterns);
                println!("[general]");
                println!("  backup_dir = {:?}", general_backup(&config));
                println!("  cache_dir = {:?}", general_cache(&config));
            }
        }
        Commands::Cache(args) => match &args.command {
            CacheCommands::Clean { config, dry_run, all, older_than_days } => {
                let config_path = config.as_deref().or_else(|| cli.config.as_deref());
                let (cfg, _) = load_config(config_path)?;
                let cache_dir = general_cache(&cfg);
                let age = older_than_days.map(|days| days * 86400.0);
                let (removed, freed) = clean_cache(&cache_dir, *dry_run, *all, age)
                    .map_err(|e| format!("could not clean cache: {e}"))?;
                if removed.is_empty() {
                    println!("cache is clean");
                } else {
                    for (path, reason) in removed {
                        println!("{} {} ({reason})", if *dry_run { "would remove" } else { "removed" }, path.display());
                    }
                    println!("{freed} byte(s) freed");
                }
            }
        },
    }
    Ok(())
}
