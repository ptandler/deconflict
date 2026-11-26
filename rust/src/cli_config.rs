use crate::cli::Cli;
use deconflict_core::{
    config::{Config, ScanConfig},
    media::Tools as MediaTools,
    patterns::Patterns,
    tools,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

pub(crate) fn load_config(path: Option<&Path>) -> Result<(Config, ScanConfig), String> {
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

pub(crate) fn effective_scan(
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
    let roots = scan_config.default_dirs.clone();
    Ok((scan_config, patterns, roots, config))
}

pub(crate) fn default_config_path() -> PathBuf {
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

pub(crate) fn general_cache(config: &Config) -> PathBuf {
    let value = config
        .general
        .get("cache_dir")
        .and_then(toml::Value::as_str)
        .unwrap_or("~/.cache/deconflict");
    expand(value)
}

pub(crate) fn general_backup(config: &Config) -> PathBuf {
    let value = config
        .general
        .get("backup_dir")
        .and_then(toml::Value::as_str)
        .unwrap_or("~/deconflict-backups");
    expand(value)
}

pub(crate) fn media_tools(config: &Config) -> MediaTools {
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
