use serde::{Deserialize, Deserializer};
use std::path::PathBuf;

fn enabled() -> Vec<String> {
    ["nextcloud", "pacman", "syncthing"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}
fn workers() -> usize {
    2
}
fn in_flight() -> usize {
    64
}

#[derive(Clone, Debug, Deserialize)]
pub struct CustomPattern {
    pub name: String,
    pub base_name: String,
    #[serde(default = "copy")]
    pub winner: String,
}
fn copy() -> String {
    "copy".into()
}

fn custom_patterns<'de, D>(deserializer: D) -> Result<Vec<CustomPattern>, D::Error>
where
    D: Deserializer<'de>,
{
    let entries = Vec::<toml::Value>::deserialize(deserializer)?;
    entries
        .into_iter()
        .filter(|entry| {
            entry
                .as_table()
                .is_some_and(|table| table.contains_key("name") && table.contains_key("base_name"))
        })
        .map(|entry| entry.try_into().map_err(serde::de::Error::custom))
        .collect()
}

#[derive(Clone, Debug, Deserialize)]
pub struct ScanConfig {
    #[serde(default)]
    pub default_dirs: Vec<PathBuf>,
    #[serde(default = "enabled")]
    pub enabled_patterns: Vec<String>,
    #[serde(default, deserialize_with = "custom_patterns")]
    pub patterns: Vec<CustomPattern>,
    #[serde(default = "workers")]
    pub workers: usize,
    #[serde(default = "in_flight")]
    pub max_in_flight: usize,
    #[serde(default)]
    pub follow_symlinks: bool,
}
impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            default_dirs: vec![],
            enabled_patterns: enabled(),
            patterns: vec![],
            workers: workers(),
            max_in_flight: in_flight(),
            follow_symlinks: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub scan: ScanConfig,
    #[serde(default)]
    pub general: toml::Table,
    #[serde(default)]
    pub tools: toml::Table,
    #[serde(default)]
    pub file_types: toml::Table,
    #[serde(default)]
    pub file_types_edit: toml::Table,
}
impl Config {
    pub fn parse(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_custom_entries_are_ignored() {
        let config = Config::parse(
            r#"
            [scan]
            patterns = [ { name = "incomplete" }, 42,
                         { name = "bak", base_name = '(?P<base>.+)\.bak' } ]
        "#,
        )
        .unwrap();
        assert_eq!(config.scan.patterns.len(), 1);
        assert_eq!(config.scan.patterns[0].name, "bak");
        assert_eq!(config.scan.patterns[0].winner, "copy");
    }

    #[test]
    fn defaults_empty_overrides_and_invalid_types() {
        let defaults = Config::parse("").unwrap();
        assert_eq!(
            defaults.scan.enabled_patterns,
            ["nextcloud", "pacman", "syncthing"]
        );
        let empty = Config::parse("[scan]\nenabled_patterns = []\npatterns = []").unwrap();
        assert!(empty.scan.enabled_patterns.is_empty());
        assert!(Config::parse("[scan]\npatterns = [{name = 1, base_name = 'x'}]").is_err());
    }
}
