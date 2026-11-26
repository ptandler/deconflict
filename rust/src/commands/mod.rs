mod resolve;
mod scan;

use crate::{
    cli::{CacheCommands, Cli, Commands, ConfigCommands},
    cli_config::{default_config_path, general_backup, general_cache, load_config},
};
use deconflict_core::{cache::clean_cache, patterns::Patterns, tools};
use std::fs;

pub(crate) fn run(mut cli: Cli) -> Result<(), String> {
    if cli.version {
        println!("deconflict 0.2.9");
        return Ok(());
    }
    let Some(command) = cli.command.take() else {
        return crate::interactive::run_bare_cli(&cli);
    };
    match command {
        Commands::Version => {
            println!("deconflict 0.2.9");
        }
        Commands::Patterns { enabled } => {
            let mut config = deconflict_core::config::ScanConfig::default();
            if enabled {
                config.enabled_patterns =
                    vec!["nextcloud".into(), "pacman".into(), "syncthing".into()];
            }
            let patterns = Patterns::build(&config)?;
            println!("name\tenabled\twinner\tmatches");
            for name in [
                "nextcloud",
                "pacman",
                "syncthing",
                "dpkg",
                "rpm",
                "windows-copy",
            ] {
                let is_enabled = config.enabled_patterns.iter().any(|p| p == name);
                if enabled && !is_enabled {
                    continue;
                }
                println!("{name}\t{is_enabled}\tcopy\t");
            }
            let _ = patterns;
        }
        Commands::Scan(args) => scan::run(&cli, args)?,
        Commands::Resolve(args) => resolve::run(&cli, args)?,
        Commands::InitConfig { path } => {
            let target = path.unwrap_or_else(default_config_path);
            if target.exists() {
                return Err(format!("exists: {}", target.display()));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create config directory: {e}"))?;
            }
            fs::write(
                &target,
                include_str!("../../../src/deconflict/config.example.toml"),
            )
            .map_err(|e| format!("cannot write config file {}: {e}", target.display()))?;
            println!("wrote config: {}", target.display());
        }
        Commands::Setup => {
            let (config, _) = load_config(cli.config.as_deref())?;
            println!("deconflict 0.2.9");
            println!(
                "  config file: {}",
                cli.config
                    .clone()
                    .unwrap_or_else(default_config_path)
                    .display()
            );
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
            let path = args.path.or_else(|| cli.config.clone());
            let (config, scan) = load_config(path.as_deref())?;
            if matches!(args.command, Some(ConfigCommands::Tools)) {
                let overrides = config
                    .tools
                    .iter()
                    .map(|(name, value)| {
                        (name.clone(), value.as_str().unwrap_or_default().to_string())
                    })
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
                println!(
                    "config file: {}",
                    path.unwrap_or_else(default_config_path).display()
                );
                println!("[scan]");
                println!("  default_dirs = {:?}", scan.default_dirs);
                println!("  enabled_patterns = {:?}", scan.enabled_patterns);
                println!("[general]");
                println!("  backup_dir = {:?}", general_backup(&config));
                println!("  cache_dir = {:?}", general_cache(&config));
            }
        }
        Commands::Cache(args) => match args.command {
            CacheCommands::Clean {
                config,
                dry_run,
                all,
                older_than_days,
            } => {
                let config_path = config.as_deref().or(cli.config.as_deref());
                let (cfg, _) = load_config(config_path)?;
                let cache_dir = general_cache(&cfg);
                let age = older_than_days.map(|days| days * 86400.0);
                let (removed, freed) = clean_cache(&cache_dir, dry_run, all, age)
                    .map_err(|e| format!("could not clean cache: {e}"))?;
                if removed.is_empty() {
                    println!("cache is clean");
                } else {
                    for (path, reason) in removed {
                        println!(
                            "{} {} ({reason})",
                            if dry_run { "would remove" } else { "removed" },
                            path.display()
                        );
                    }
                    println!("{freed} byte(s) freed");
                }
            }
        },
    }
    Ok(())
}
