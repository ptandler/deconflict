use crate::config::ScanConfig;
use fancy_regex::Regex;

fn ascii_expression(expression: &str) -> Result<String, &'static str> {
    let mut result = String::new();
    let mut chars = expression.chars();
    while let Some(c) = chars.next() {
        match c {
            '[' => return Err("ASCII mode character classes are not supported"),
            '(' if chars.as_str().starts_with('?') && !chars.as_str().starts_with("?P<") => {
                return Err("ASCII mode inline flags and special groups are not supported");
            }
            '\\' => match chars.next() {
                Some('w') => result.push_str("[a-zA-Z0-9_]"),
                Some('d') => result.push_str("[0-9]"),
                Some('s') => result.push_str("[\\x09-\\x0d ]"),
                Some('W' | 'D' | 'S' | 'b' | 'B') => {
                    return Err("ASCII mode negated classes and word boundaries are not supported");
                }
                Some(next) => {
                    result.push('\\');
                    result.push(next);
                }
                None => result.push('\\'),
            },
            _ => result.push(c),
        }
    }
    Ok(result)
}

fn windows_equal(left: &str, right: &str) -> bool {
    fn fold(value: &str) -> String {
        let mut folded = String::new();
        for c in value.chars().flat_map(char::to_lowercase) {
            // Unicode 15.1 full casefold differences after lowercase. Cherokee folds to uppercase.
            if ('\u{ab70}'..='\u{abbf}').contains(&c) || ('\u{13f8}'..='\u{13fd}').contains(&c) {
                folded.extend(c.to_uppercase());
                continue;
            }
            if ('\u{1f80}'..='\u{1fa7}').contains(&c) {
                let offset = match c as u32 {
                    0x1f80..=0x1f87 => 0x80,
                    0x1f90..=0x1f97 => 0x70,
                    0x1fa0..=0x1fa7 => 0x40,
                    _ => 0,
                };
                if offset == 0 {
                    folded.push(c);
                    continue;
                }
                folded.push(char::from_u32(c as u32 - offset).unwrap());
                folded.push('ι');
                continue;
            }
            match c {
                'ß' => folded.push_str("ss"),
                'ς' => folded.push('σ'),
                'ſ' => folded.push('s'),
                'µ' => folded.push('μ'),
                'ŉ' => folded.push_str("ʼn"),
                'ǰ' => folded.push_str("j\u{30c}"),
                '\u{345}' | 'ι' => folded.push('ι'),
                'ΐ' => folded.push_str("ι\u{308}\u{301}"),
                'ΰ' => folded.push_str("υ\u{308}\u{301}"),
                'ϐ' => folded.push('β'),
                'ϑ' => folded.push('θ'),
                'ϕ' => folded.push('φ'),
                'ϖ' => folded.push('π'),
                'ϰ' => folded.push('κ'),
                'ϱ' => folded.push('ρ'),
                'ϵ' => folded.push('ε'),
                'և' => folded.push_str("եւ"),
                '\u{1c80}' => folded.push('в'),
                '\u{1c81}' => folded.push('д'),
                '\u{1c82}' => folded.push('о'),
                '\u{1c83}' => folded.push('с'),
                '\u{1c84}' | '\u{1c85}' => folded.push('т'),
                '\u{1c86}' => folded.push('ъ'),
                '\u{1c87}' => folded.push('ѣ'),
                '\u{1c88}' => folded.push('ꙋ'),
                'ẖ' => folded.push_str("h\u{331}"),
                'ẗ' => folded.push_str("t\u{308}"),
                'ẘ' => folded.push_str("w\u{30a}"),
                'ẙ' => folded.push_str("y\u{30a}"),
                'ẚ' => folded.push_str("aʾ"),
                'ẛ' => folded.push('ṡ'),
                'ὐ' => folded.push_str("υ\u{313}"),
                'ὒ' => folded.push_str("υ\u{313}\u{300}"),
                'ὔ' => folded.push_str("υ\u{313}\u{301}"),
                'ὖ' => folded.push_str("υ\u{313}\u{342}"),
                'ᾲ' => folded.push_str("ὰι"),
                'ᾳ' => folded.push_str("αι"),
                'ᾴ' => folded.push_str("άι"),
                'ᾶ' => folded.push_str("α\u{342}"),
                'ᾷ' => folded.push_str("α\u{342}ι"),
                'ῂ' => folded.push_str("ὴι"),
                'ῃ' => folded.push_str("ηι"),
                'ῄ' => folded.push_str("ήι"),
                'ῆ' => folded.push_str("η\u{342}"),
                'ῇ' => folded.push_str("η\u{342}ι"),
                'ῒ' => folded.push_str("ι\u{308}\u{300}"),
                'ΐ' => folded.push_str("ι\u{308}\u{301}"),
                'ῖ' => folded.push_str("ι\u{342}"),
                'ῗ' => folded.push_str("ι\u{308}\u{342}"),
                'ῢ' => folded.push_str("υ\u{308}\u{300}"),
                'ΰ' => folded.push_str("υ\u{308}\u{301}"),
                'ῤ' => folded.push_str("ρ\u{313}"),
                'ῦ' => folded.push_str("υ\u{342}"),
                'ῧ' => folded.push_str("υ\u{308}\u{342}"),
                'ῲ' => folded.push_str("ὼι"),
                'ῳ' => folded.push_str("ωι"),
                'ῴ' => folded.push_str("ώι"),
                'ῶ' => folded.push_str("ω\u{342}"),
                'ῷ' => folded.push_str("ω\u{342}ι"),
                'ﬀ' => folded.push_str("ff"),
                'ﬁ' => folded.push_str("fi"),
                'ﬂ' => folded.push_str("fl"),
                'ﬃ' => folded.push_str("ffi"),
                'ﬄ' => folded.push_str("ffl"),
                'ﬅ' | 'ﬆ' => folded.push_str("st"),
                'ﬓ' => folded.push_str("մն"),
                'ﬔ' => folded.push_str("մե"),
                'ﬕ' => folded.push_str("մի"),
                'ﬖ' => folded.push_str("վն"),
                'ﬗ' => folded.push_str("մխ"),
                _ => folded.push(c),
            }
        }
        folded
    }
    fold(left) == fold(right)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictMatch {
    pub pattern: String,
    pub base_name: String,
    pub winner: String,
    pub raw: String,
    pub stamp: Option<String>,
}

#[derive(Clone, Debug)]
struct Pattern {
    name: String,
    winner: String,
    regex: Regex,
    builtin: bool,
}

#[derive(Clone, Debug)]
pub struct Patterns(Vec<Pattern>);

impl Patterns {
    pub fn build(config: &ScanConfig) -> Result<Self, String> {
        let builtins = [
            (
                "nextcloud",
                r"(?P<stem>.*?) \(conflicted copy\s+(?P<stamp>[^)]*)\)(?P<ext>\.[^.]+)?",
            ),
            ("pacman", r"(?P<stem>(?s:.*))\.(?:pacnew|pacsave|pacorig)"),
            (
                "syncthing",
                r"(?P<stem>.*?)\.sync-conflict-(?P<stamp>[0-9A-Za-z:+_-]+)-(?P<id>[A-Za-z0-9]+)(?P<ext>\.[^.]+)?",
            ),
            (
                "dpkg",
                r"(?P<stem>(?s:.*))\.(?:dpkg-dist|dpkg-old|ucf-dist|ucf-old|ucf-new)",
            ),
            (
                "rpm",
                r"(?P<stem>(?s:.*))\.(?:rpmnew|rpmorig|rpmold|rpmignore)",
            ),
            (
                "windows-copy",
                r"(?P<stem>.*?)(?: - Copy| \(copy\))(?P<ext>\.[^.]+)?",
            ),
        ];
        let mut patterns = Vec::new();
        for (name, expression) in builtins {
            if config.enabled_patterns.iter().any(|e| {
                if cfg!(windows) {
                    windows_equal(e, name)
                } else {
                    e == name
                }
            }) {
                patterns.push(Pattern {
                    name: name.into(),
                    winner: "copy".into(),
                    regex: Regex::new(&format!(r"\A(?:{})\z", expression))
                        .map_err(|e| e.to_string())?,
                    builtin: true,
                });
            }
        }
        for p in &config.patterns {
            let verbose = p.base_name.strip_prefix("(?").is_some_and(|rest| {
                rest.split_once(')').is_some_and(|(flags, _)| {
                    flags.contains('x') && flags.chars().all(|c| "aiLmsux".contains(c))
                })
            });
            let terminator = if verbose { "\n" } else { "" };
            let mut expression = p.base_name.clone();
            if let Some((flags, rest)) = p
                .base_name
                .strip_prefix("(?")
                .and_then(|s| s.split_once(')'))
            {
                if flags.contains('a') && flags.chars().all(|c| "aimsx".contains(c)) {
                    if flags.contains('i') {
                        return Err(format!(
                            "{}: ASCII mode with case-insensitivity is not supported",
                            p.name
                        ));
                    }
                    let flags = flags.replace('a', "");
                    let rest = ascii_expression(rest).map_err(|e| format!("{}: {e}", p.name))?;
                    expression = if flags.is_empty() {
                        rest
                    } else {
                        format!("(?{flags}:{rest}{terminator})")
                    };
                }
            }
            let regex = Regex::new(&format!(r"\A(?:{expression}{terminator})\z"))
                .map_err(|e| format!("{}: {e}", p.name))?;
            if !regex.capture_names().flatten().any(|n| n == "base") {
                return Err(format!(
                    "custom pattern '{}' must define a named group 'base'",
                    p.name
                ));
            }
            patterns.push(Pattern {
                name: p.name.clone(),
                winner: p.winner.clone(),
                regex,
                builtin: false,
            });
        }
        Ok(Self(patterns))
    }

    pub fn match_name(&self, name: &str) -> Result<Option<ConflictMatch>, String> {
        for p in &self.0 {
            if let Some(c) = p
                .regex
                .captures(name)
                .map_err(|e| format!("{}: {e}", p.name))?
            {
                let base = if p.builtin {
                    format!(
                        "{}{}",
                        c.name("stem").map_or("", |m| m.as_str()),
                        c.name("ext").map_or("", |m| m.as_str())
                    )
                } else {
                    c.name("base")
                        .ok_or_else(|| {
                            format!("{}: named group 'base' did not participate", p.name)
                        })?
                        .as_str()
                        .to_string()
                };
                if !p.builtin && (base == name || (cfg!(windows) && windows_equal(&base, name))) {
                    continue;
                }
                return Ok(Some(ConflictMatch {
                    pattern: p.name.clone(),
                    base_name: base,
                    winner: p.winner.clone(),
                    raw: name.into(),
                    stamp: c.name("stamp").map(|m| m.as_str().to_string()),
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn builtin_names_are_case_sensitive_on_posix() {
        let config = ScanConfig {
            enabled_patterns: vec!["PACMAN".into()],
            ..ScanConfig::default()
        };
        assert!(Patterns::build(&config)
            .unwrap()
            .match_name("a.pacnew")
            .unwrap()
            .is_none());
    }

    #[test]
    fn python_custom_regex_and_builtin_priority() {
        let config = ScanConfig {
            patterns: vec![crate::config::CustomPattern {
                name: "custom".into(),
                base_name: r"(?i)^(?P<base>.+)\.(?P<stamp>bak)$".into(),
                winner: "base".into(),
            }],
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        let matched = patterns.match_name("config.BAK").unwrap().unwrap();
        assert_eq!(
            (
                matched.base_name.as_str(),
                matched.stamp.as_deref(),
                matched.winner.as_str()
            ),
            ("config", Some("BAK"), "base")
        );
        assert_eq!(
            patterns.match_name("a.pacnew").unwrap().unwrap().pattern,
            "pacman"
        );
        assert!(patterns.match_name("config.BAK.extra").unwrap().is_none());
    }

    #[test]
    fn custom_patterns_validate_and_do_not_create_self_conflicts() {
        let mut config = ScanConfig {
            enabled_patterns: vec![],
            ..ScanConfig::default()
        };
        let custom = |expression: &str| crate::config::CustomPattern {
            name: "custom".into(),
            base_name: expression.into(),
            winner: "copy".into(),
        };
        config.patterns = vec![custom(r"(.+)\.bak")];
        assert!(Patterns::build(&config)
            .unwrap_err()
            .contains("named group 'base'"));
        config.patterns = vec![custom(r"(?P<base>.+")];
        assert!(Patterns::build(&config).is_err());
        config.patterns = vec![custom(r"(?P<base>.+)")];
        assert!(Patterns::build(&config)
            .unwrap()
            .match_name("file")
            .unwrap()
            .is_none());
        config.patterns = vec![custom(r"(?P<base>a)?\.bak")];
        assert!(Patterns::build(&config)
            .unwrap()
            .match_name(".bak")
            .is_err());
    }

    #[test]
    fn python_named_backreferences() {
        let config = ScanConfig {
            enabled_patterns: vec![],
            patterns: vec![crate::config::CustomPattern {
                name: "repeat".into(),
                base_name: r"(?P<base>\w+)-(?P=base)\.bak".into(),
                winner: "copy".into(),
            }],
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        assert_eq!(
            patterns
                .match_name("café-café.bak")
                .unwrap()
                .unwrap()
                .base_name,
            "café"
        );
        assert!(patterns.match_name("café-cafe.bak").unwrap().is_none());
    }

    #[test]
    fn invalid_named_backreferences_are_rejected() {
        for expression in [r"(?P<base>.+)-(?P=missing)", r"(?P<base>.+)-(?P=base"] {
            let config = ScanConfig {
                enabled_patterns: vec![],
                patterns: vec![crate::config::CustomPattern {
                    name: "bad-reference".into(),
                    base_name: expression.into(),
                    winner: "copy".into(),
                }],
                ..ScanConfig::default()
            };
            assert!(Patterns::build(&config)
                .unwrap_err()
                .contains("bad-reference"));
        }
    }

    #[test]
    fn python_ascii_flag_supports_ascii_classes() {
        let config = ScanConfig {
            enabled_patterns: vec![],
            patterns: vec![crate::config::CustomPattern {
                name: "ascii".into(),
                base_name: r"(?a)(?P<base>\w+\s\d+)\.bak".into(),
                winner: "copy".into(),
            }],
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        assert!(patterns.match_name("File 42.bak").unwrap().is_some());
        assert!(patterns.match_name("café 42.bak").unwrap().is_none());
        assert!(patterns.match_name("File\u{a0}42.bak").unwrap().is_none());
        assert!(patterns.match_name("File ٤٢.bak").unwrap().is_none());
        for expression in [
            r"(?ai)(?P<base>\w+)\.bak",
            r"(?a)(?P<base>\W+)\.bak",
            r"(?a)(?P<base>[\w]+)\.bak",
            r"(?a:(?P<base>\w+))\.bak",
        ] {
            let mut config = config.clone();
            config.patterns[0].base_name = expression.into();
            assert!(Patterns::build(&config).is_err(), "{expression}");
        }
    }

    #[test]
    fn python_verbose_regex_with_trailing_comment() {
        let config = ScanConfig {
            enabled_patterns: vec![],
            patterns: vec![crate::config::CustomPattern {
                name: "verbose".into(),
                base_name: r"(?ix)(?P<base>.+) \.bak # trailing comment".into(),
                winner: "copy".into(),
            }],
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        assert_eq!(
            patterns.match_name("file.BAK").unwrap().unwrap().base_name,
            "file"
        );
        assert!(patterns.match_name("file.BAK.more").unwrap().is_none());
    }

    #[test]
    fn windows_comparison_handles_common_unicode_folds() {
        assert!(windows_equal("Straße", "STRASSE"));
        assert!(windows_equal("ΟΣ", "ος"));
        assert!(windows_equal("K", "k"));
        assert!(!windows_equal("café", "cafe"));
        assert!(windows_equal("ﬃ", "ffi"));
        assert!(windows_equal("ᾈ", "ἀι"));
        assert!(windows_equal("Ꭰ", "ꭰ"));
    }

    #[test]
    fn builtin_examples_and_defaults() {
        let patterns = Patterns::build(&ScanConfig::default()).unwrap();
        let nextcloud = patterns
            .match_name("Readme (conflicted copy 2024-12-01).md")
            .unwrap()
            .unwrap();
        assert_eq!(
            (nextcloud.base_name.as_str(), nextcloud.stamp.as_deref()),
            ("Readme.md", Some("2024-12-01"))
        );
        let syncthing = patterns
            .match_name("file.sync-conflict-20240427-150142-JKHLKL.txt")
            .unwrap()
            .unwrap();
        assert_eq!(syncthing.base_name, "file.txt");
        assert_eq!(syncthing.stamp.as_deref(), Some("20240427-150142"));
        assert!(patterns.match_name("foo - Copy.txt").unwrap().is_none());
        let patterns = Patterns::build(&ScanConfig {
            enabled_patterns: vec!["windows-copy".into(), "dpkg".into(), "rpm".into()],
            ..ScanConfig::default()
        })
        .unwrap();
        for (name, base) in [
            ("foo - Copy.txt", "foo.txt"),
            ("foo.dpkg-old", "foo"),
            ("foo.rpmnew", "foo"),
        ] {
            assert_eq!(patterns.match_name(name).unwrap().unwrap().base_name, base);
        }
    }

    #[test]
    fn package_suffixes_accept_newlines_in_base_name() {
        let config = ScanConfig {
            enabled_patterns: vec!["pacman".into(), "dpkg".into(), "rpm".into()],
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        for suffix in ["pacnew", "dpkg-old", "rpmnew"] {
            let matched = patterns
                .match_name(&format!("line\nbreak.{suffix}"))
                .unwrap()
                .unwrap();
            assert_eq!(matched.base_name, "line\nbreak");
        }
    }
}
