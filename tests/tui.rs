//! PTY-level contracts for the full-screen TUI.

use console::strip_ansi_codes;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    root: PathBuf,
    config: PathBuf,
    cache: PathBuf,
    backup: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("files");
        fs::create_dir_all(&root).expect("root dir");
        let base = root.join("note.txt");
        let copy = root.join("note (conflicted copy 2026-01-01).txt");
        fs::write(&base, "shared\nbase\n").expect("base");
        fs::write(&copy, "shared\ncopy with extra content\n").expect("copy");
        let older = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        fs::File::open(&base)
            .and_then(|file| file.set_modified(older))
            .expect("set base mtime");
        fs::File::open(&copy)
            .and_then(|file| file.set_modified(older + Duration::from_secs(3600)))
            .expect("set copy mtime");
        let config = temp.path().join("config.toml");
        let cache = temp.path().join("cache");
        let backup = temp.path().join("backups");
        let toml_path = |path: &Path| path.display().to_string().replace('\\', "/");
        fs::write(
            &config,
            format!(
                "[general]\ncache_dir = \"{}\"\nbackup_dir = \"{}\"\n",
                toml_path(&cache),
                toml_path(&backup)
            ),
        )
        .expect("config");
        Self {
            temp,
            root,
            config,
            cache,
            backup,
        }
    }

    fn empty() -> Self {
        let fixture = Self::new();
        fs::remove_file(fixture.root.join("note (conflicted copy 2026-01-01).txt"))
            .expect("remove conflict");
        fixture
    }

    fn three() -> Self {
        let fixture = Self::new();
        fs::write(
            fixture.root.join("note (conflicted copy 2026-01-02).txt"),
            "shared\nbase\nsecond copy\n",
        )
        .expect("second conflict");
        fixture
    }

    fn with_diff_tool() -> Self {
        let fixture = Self::new();
        let exe = std::env::current_exe()
            .expect("current exe")
            .display()
            .to_string()
            .replace('\\', "/");
        let mut config = fs::read_to_string(&fixture.config).expect("config");
        config.push_str(&format!("\n[tools]\ndiff = \"{exe}\"\n"));
        fs::write(&fixture.config, config).expect("rewrite config");
        fixture
    }

    fn large() -> Self {
        let fixture = Self::new();
        for index in 0..4000 {
            fs::write(
                fixture.root.join(format!("filler-{index:05}.txt")),
                format!("filler {index}\n"),
            )
            .expect("filler file");
        }
        fixture
    }

    fn command_args(&self) -> Vec<String> {
        vec![
            "resolve".into(),
            "--tui".into(),
            "--config".into(),
            self.config.display().to_string(),
            "--no-cache".into(),
            self.root.display().to_string(),
        ]
    }

    fn contents_unchanged(&self) {
        assert_eq!(
            fs::read_to_string(self.root.join("note.txt")).expect("base"),
            "shared\nbase\n"
        );
        assert!(self
            .root
            .join("note (conflicted copy 2026-01-01).txt")
            .exists());
    }
}

fn contains_text(output: &str, needle: &str) -> bool {
    let compact = |value: &str| -> String {
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    };
    compact(output).contains(&compact(needle))
}

fn run_pty(args: &[String], keys: &[u8], expected: &[&str]) -> String {
    run_pty_stages(args, &[(expected, keys)])
}

fn run_pty_stages(args: &[String], stages: &[(&[&str], &[u8])]) -> String {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_deconflict"));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("NO_COLOR", "1");
    let mut child = pair.slave.spawn_command(command).expect("spawn TUI");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if output_tx.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut writer = pair.master.take_writer().expect("pty writer");
    let mut bytes = Vec::new();
    for (expected, keys) in stages {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            while let Ok(chunk) = output_rx.try_recv() {
                bytes.extend(chunk);
            }
            let plain = strip_ansi_codes(&String::from_utf8_lossy(&bytes)).into_owned();
            if expected.iter().all(|needle| contains_text(&plain, needle)) {
                break;
            }
            if Instant::now() >= deadline {
                panic!("TUI did not show {expected:?}; output:\n{plain}");
            }
            thread::sleep(Duration::from_millis(50));
        }
        writer.write_all(keys).expect("send keys");
    }

    let (status_tx, status_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = status_tx.send(child.wait());
    });
    let status = status_rx
        .recv_timeout(Duration::from_secs(15))
        .expect("TUI did not exit after keys")
        .expect("wait");
    assert!(status.success(), "TUI exited with {status}");

    drop(writer);
    while let Ok(chunk) = output_rx.recv_timeout(Duration::from_millis(200)) {
        bytes.extend(chunk);
    }
    strip_ansi_codes(&String::from_utf8_lossy(&bytes)).into_owned()
}

#[test]
fn tui_renders_conflicts_and_quits_when_scan_is_done_in_dry_run() {
    let fixture = Fixture::new();
    let mut args = fixture.command_args();
    args.push("--dry-run".into());
    run_pty(
        &args,
        b"Q",
        &[
            "conflicts:1",
            "note.txt",
            "keep base",
            "dry-run",
            "(q) quit",
            "field",
            "#0note",
        ],
    );

    fixture.contents_unchanged();
    assert!(!fixture.backup.exists());
    assert!(!fixture.cache.exists());
    drop(fixture.temp);
}

#[test]
fn tui_records_skip_before_applying() {
    let fixture = Fixture::new();
    let mut args = fixture.command_args();
    args.push("--dry-run".into());
    let output = run_pty(&args, b"sQ", &["conflicts:1", "(s) skip", "(q) quit"]);
    assert!(contains_text(&output, "1 skipped"), "output:\n{output}");
    fixture.contents_unchanged();
    assert!(!fixture.backup.exists());
    drop(fixture.temp);
}

#[test]
fn tui_applies_recorded_keep_base_in_real_run() {
    let fixture = Fixture::new();
    let output = run_pty(
        &fixture.command_args(),
        b"0Q",
        &["conflicts:1", "(0) keep base", "(q) quit"],
    );
    assert!(
        contains_text(&output, "resolved 1 conflict"),
        "output:\n{output}"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("note.txt")).expect("base"),
        "shared\nbase\n"
    );
    assert!(!fixture
        .root
        .join("note (conflicted copy 2026-01-01).txt")
        .exists());
    assert!(fs::read_dir(&fixture.backup)
        .expect("backup dir")
        .next()
        .is_some());
    drop(fixture.temp);
}

#[test]
fn quit_cancels_without_writes() {
    let fixture = Fixture::large();
    let output = run_pty(&fixture.command_args(), b"q", &["deconflict", "(q) quit"]);
    assert!(contains_text(&output, "quit"), "output:\n{output}");
    fixture.contents_unchanged();
    assert!(!fixture.backup.exists());
    assert!(!fixture.cache.exists());
    drop(fixture.temp);
}

#[test]
fn ctrl_c_exits_tui() {
    let fixture = Fixture::new();
    let mut args = fixture.command_args();
    args.push("--dry-run".into());
    let output = run_pty(&args, b"\x03", &["conflicts:1", "keep base"]);
    assert!(
        contains_text(&output, "interrupt") || contains_text(&output, "quit"),
        "output:\n{output}"
    );
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_without_conflicts_starts_on_log_tab_and_offers_quit() {
    let fixture = Fixture::empty();
    let output = run_pty(
        &fixture.command_args(),
        b"q",
        &["no conflicts found", "(q) quit", " log "],
    );
    assert!(
        contains_text(&output, "ready")
            && contains_text(&output, "0 decided")
            && contains_text(&output, "log help")
            && !contains_text(&output, "metafilesdiff"),
        "output:\n{output}"
    );
    assert!(!fixture.backup.exists());
    drop(fixture.temp);
}

#[test]
fn tui_h_opens_keyboard_shortcuts() {
    let fixture = Fixture::new();
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"h"),
            (&["keyboard shortcuts", "Left/Right"], b"\x03"),
        ],
    );
    assert!(
        contains_text(&output, "press (h) to list keyboard shortcuts"),
        "output:\n{output}"
    );
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_question_mark_lists_commands() {
    let fixture = Fixture::new();
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"?"),
            (&["tools/config", "built-in", "(q) quit"], b"\x03"),
        ],
    );
    assert!(
        contains_text(&output, "file type: text"),
        "output:\n{output}"
    );
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_question_mark_lists_executable_commands() {
    let fixture = Fixture::with_diff_tool();
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"?"),
            (&["file type: text", "PlainTerminal"], b"\x03"),
        ],
    );
    assert!(
        contains_text(&output, "file type: text") && contains_text(&output, "PlainTerminal"),
        "output:\n{output}"
    );
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_group_list_shows_file_count() {
    let fixture = Fixture::three();
    let output = run_pty(
        &fixture.command_args(),
        b"q",
        &["note.txt(3x)", "nextcloud"],
    );
    assert!(contains_text(&output, "note.txt (3x)"), "output:\n{output}");
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_diff_uses_selected_file() {
    let fixture = Fixture::three();
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"\t"),
            (&["file detail", "content:base"], b"\x1b[B\x1b[B"),
            (&["size:↑12B"], b"d"),
            (&["--- ", "+++ ", "second copy"], b"\x03"),
        ],
    );
    assert!(
        contains_text(&output, "+second copy")
            && !contains_text(&output, "+copy with extra content"),
        "output:\n{output}"
    );
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_files_tab_shows_selected_file_relations() {
    let fixture = Fixture::new();
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"m"),
            (&["field", "#0note", "modified"], b"\t"),
            (
                &[
                    "file detail",
                    "content:base",
                    "selectacopytocomparewith#0",
                    "(↑19B)",
                ],
                b"\x1b[B",
            ),
            (
                &["copywithextracontent", "differs", "size:↑19B", "findings:"],
                b"\x03",
            ),
        ],
    );
    assert!(contains_text(&output, "file detail"), "output:\n{output}");
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_arrow_keys_switch_tabs() {
    let fixture = Fixture::new();
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"\x1b[C"),
            (&["file detail", "content:base"], b"\x1b[D"),
            (&["field", "#0note", "modified"], b"\x03"),
        ],
    );
    assert!(
        contains_text(&output, "file detail") && contains_text(&output, "#0note"),
        "output:\n{output}"
    );
    fixture.contents_unchanged();
    drop(fixture.temp);
}

#[test]
fn tui_file_detail_shows_analysis_findings() {
    let fixture = Fixture::new();
    let copy = fixture.root.join("note (conflicted copy 2026-01-01).txt");
    fs::write(&copy, "shared\nbase\nextra note\n").expect("rewrite copy");
    let output = run_pty_stages(
        &fixture.command_args(),
        &[
            (&["(b) keep both", "(q) quit"], b"m"),
            (&["field", "#0note"], b"\t"),
            (&["file detail", "selectacopytocomparewith#0"], b"\x1b[B"),
            (&["findings:"], b"\x1b[6;2~"),
            (&["recommended"], b"\x03"),
        ],
    );
    assert!(contains_text(&output, "findings:"), "output:\n{output}");
    assert_eq!(
        fs::read_to_string(fixture.root.join("note.txt")).expect("base"),
        "shared\nbase\n"
    );
    assert!(copy.exists());
    drop(fixture.temp);
}

#[test]
fn tui_requires_a_terminal() {
    let fixture = Fixture::new();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .args(["resolve", "--tui", "--config"])
        .arg(&fixture.config)
        .arg(&fixture.root)
        .output()
        .expect("run non-TTY TUI");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires an interactive terminal"),
        "{stderr}"
    );
}
