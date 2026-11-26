//! Optional external tool discovery, independent of any UI.

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

const NAMES: &[&str] = &[
    "diff",
    "editor",
    "image_viewer",
    "image_editor",
    "office",
    "office_text",
    "office_compare",
    "keepass",
    "mp3_editor",
    "audio_player",
    "video_probe",
    "exiftool",
    "viewer",
];

/// Resolved tool paths. Overrides are retained verbatim without checking that they exist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    pub found: HashMap<String, PathBuf>,
}

impl Tools {
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Path> {
        self.found.get(name).map(PathBuf::as_path)
    }

    pub fn names(&self) -> &'static [&'static str] {
        NAMES
    }

    pub fn candidates(&self, name: &str) -> Vec<String> {
        candidates(name)
    }

    pub fn hint(&self, name: &str) -> Option<&'static str> {
        hint(name)
    }

    pub fn require(&self, name: &str, action: &str) -> Result<&Path, ToolMissing> {
        self.get(name)
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| ToolMissing {
                name: name.to_owned(),
                action: action.to_owned(),
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolMissing {
    pub name: String,
    pub action: String,
}

impl fmt::Display for ToolMissing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} requires '{}', which was not found. Install it or set it in config.",
            self.action, self.name
        )
    }
}

impl Error for ToolMissing {}

fn candidates_with_editor(name: &str, editor: Option<&str>) -> Vec<String> {
    let values: &[&str] = match name {
        "diff" => &["meld", "WinMergeU.exe"],
        "editor" => &[editor.filter(|s| !s.is_empty()).unwrap_or("vi")],
        "image_viewer" if cfg!(windows) => &["mspaint", "start"],
        "image_viewer" => &["eog", "xdg-open"],
        "image_editor" => &["gimp", "pinta", "kolourpaint"],
        "office" | "office_compare" => &["soffice", "libreoffice"],
        "office_text" => &["odt2txt", "pandoc"],
        "keepass" => &["keepassxc-cli"],
        "mp3_editor" => &["kid3", "easytag", "picard", "kid3-cli"],
        "audio_player" => &["mpv", "vlc", "ffplay", "totem", "xdg-open"],
        "video_probe" => &["ffprobe"],
        "exiftool" => &["exiftool"],
        "viewer" => &["xdg-open", "start"],
        _ => &[name],
    };
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub fn candidates(name: &str) -> Vec<String> {
    candidates_with_editor(name, env::var("EDITOR").ok().as_deref())
}

pub fn hint(name: &str) -> Option<&'static str> {
    Some(match name {
        "diff" => "meld: apt install meld  ·  brew install meld",
        "editor" => "set $EDITOR, or override [tools] editor in config",
        "image_viewer" => "eog: apt install eog  ·  brew install eog",
        "image_editor" => "gimp: apt install gimp  ·  brew install --cask gimp",
        "office" => "soffice: apt install libreoffice  ·  brew install --cask libreoffice",
        "office_text" => "odt2txt: apt install odt2txt  ·  brew install odt2txt  ·  cand: pandoc",
        "office_compare" => "soffice File>Compare: libreoffice (use for a redline GUI diff)",
        "keepass" => "keepassxc-cli: apt install keepassxc  ·  brew install keepassxc",
        "mp3_editor" => {
            "kid3/easytag: apt install kid3 easytag  ·  brew install --cask kid3 easytag"
        }
        "audio_player" => "mpv/vlc: apt install mpv vlc  ·  brew install --cask vlc",
        "video_probe" => "ffprobe: apt install ffmpeg  ·  brew install ffmpeg",
        "exiftool" => {
            "exiftool -diff: apt install libimage-exiftool-perl  ·  brew install exiftool"
        }
        "viewer" => "xdg-open: your system default opener, no install needed",
        _ => return None,
    })
}

/// Detect known roles; unknown overrides are ignored and empty overrides fall back to discovery.
pub fn detect(overrides: &HashMap<String, String>) -> Tools {
    detect_with(overrides, candidates, which)
}

fn detect_with(
    overrides: &HashMap<String, String>,
    candidates: impl Fn(&str) -> Vec<String>,
    which: impl Fn(&str) -> Option<PathBuf>,
) -> Tools {
    let mut found = HashMap::new();
    for &name in NAMES {
        let path = overrides
            .get(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                candidates(name)
                    .iter()
                    .find_map(|candidate| which(candidate))
            });
        if let Some(path) = path {
            found.insert(name.to_owned(), path);
        }
    }
    Tools { found }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn which(command: &str) -> Option<PathBuf> {
    if command.is_empty() {
        return None;
    }
    let path = Path::new(command);
    let has_directory = path.is_absolute()
        || path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
    let dirs: Vec<PathBuf> = if has_directory {
        vec![PathBuf::new()]
    } else {
        env::var_os("PATH")
            .map(|value| env::split_paths(&value).collect())
            .unwrap_or_default()
    };
    #[cfg(windows)]
    let extensions: Vec<String> = env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    for dir in dirs {
        let candidate = dir.join(path);
        #[cfg(windows)]
        {
            if extensions
                .iter()
                .any(|ext| command.to_uppercase().ends_with(&ext.to_uppercase()))
                && executable(&candidate)
            {
                return Some(candidate);
            }
            for ext in &extensions {
                let mut extended = candidate.as_os_str().to_os_string();
                extended.push(ext);
                let extended = PathBuf::from(extended);
                if executable(&extended) {
                    return Some(extended);
                }
            }
        }
        #[cfg(not(windows))]
        if executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_respects_order_overrides_and_missing_tools() {
        let overrides = HashMap::from([
            ("diff".into(), "not-validated".into()),
            ("office".into(), "".into()),
            ("unknown".into(), "ignored".into()),
        ]);
        let tools = detect_with(
            &overrides,
            |name| candidates_with_editor(name, None),
            |candidate| match candidate {
                "meld" | "soffice" | "libreoffice" => {
                    Some(PathBuf::from(format!("/bin/{candidate}")))
                }
                _ => None,
            },
        );
        assert_eq!(tools.get("diff"), Some(Path::new("not-validated")));
        assert_eq!(tools.get("office"), Some(Path::new("/bin/soffice")));
        assert_eq!(tools.get("unknown"), None);
        assert_eq!(tools.get("video_probe"), None);
        assert!(!tools.is_empty());
    }

    #[test]
    fn metadata_and_errors_match_python() {
        let tools = Tools::default();
        assert!(tools.is_empty());
        assert_eq!(tools.names().len(), 13);
        assert!(tools.names().iter().all(|name| tools.hint(name).is_some()));
        assert_eq!(tools.hint("unknown"), None);
        assert_eq!(tools.candidates("custom"), ["custom"]);
        assert_eq!(candidates_with_editor("editor", None), ["vi"]);
        assert_eq!(candidates_with_editor("editor", Some("")), ["vi"]);
        assert_eq!(
            candidates_with_editor("editor", Some("my editor")),
            ["my editor"]
        );
        let error = tools.require("diff", "Compare").unwrap_err();
        assert_eq!(error.name, "diff");
        assert_eq!(
            error.to_string(),
            "Compare requires 'diff', which was not found. Install it or set it in config."
        );
    }

    #[test]
    fn which_accepts_real_executable_and_rejects_missing_or_directory() {
        let executable = env::current_exe().unwrap();
        assert_eq!(which(executable.to_str().unwrap()), Some(executable));
        assert_eq!(which(""), None);
        assert_eq!(which("."), None);
        assert_eq!(which("/definitely-missing-deconflict-tool"), None);
    }
}
