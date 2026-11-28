use deconflict_core::cache::cache_key;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};

fn write_config(path: &Path, cache_dir: &Path) {
    fs::write(
        path,
        format!("[general]\ncache_dir = {:?}\n", cache_dir.to_string_lossy()),
    )
    .unwrap();
}

fn scan(config: &Path, root: &Path, no_cache: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_deconflict"));
    command.arg("--config").arg(config).arg("scan");
    if no_cache {
        command.arg("--no-cache");
    }
    command.arg(root).output().unwrap()
}

#[test]
fn scan_saves_and_reuses_file_count_estimate() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let cache_dir = temp.path().join("cache");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"base").unwrap();
    fs::write(root.join("file.pacnew"), b"copy").unwrap();
    write_config(&config, &cache_dir);

    let first = scan(&config, &root, false);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let key = cache_key([&root]).unwrap();
    let estimate = cache_dir.join(format!("{key}.count"));
    assert_eq!(fs::read_to_string(&estimate).unwrap(), "2");

    fs::write(root.join("extra"), b"another file").unwrap();
    let second = scan(&config, &root, false);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(fs::read_to_string(estimate).unwrap(), "3");
}

#[test]
fn no_cache_scan_does_not_read_or_write_estimate_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let cache_dir = temp.path().join("cache");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.pacnew"), b"copy").unwrap();
    write_config(&config, &cache_dir);

    let output = scan(&config, &root, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!cache_dir.exists());
}

#[test]
fn dry_run_resolve_does_not_write_estimate_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let cache_dir = temp.path().join("cache");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"base").unwrap();
    fs::write(root.join("file.pacnew"), b"copy").unwrap();
    write_config(&config, &cache_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--dry-run")
        .arg("--auto")
        .arg("base")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!cache_dir.exists());
    assert!(root.join("file").exists());
    assert!(root.join("file.pacnew").exists());
}

#[test]
fn interactive_resolve_prints_analysis_and_uses_recommendation_on_enter() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let cache_dir = temp.path().join("cache");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    let base = root.join("Readme.md");
    let copy = root.join("Readme.md.pacnew");
    fs::write(&base, "Title\n").unwrap();
    fs::write(&copy, "Title\nAdded line\n").unwrap();
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    fs::File::options()
        .write(true)
        .open(&base)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&copy)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified + Duration::from_secs(10)))
        .unwrap();
    write_config(&config, &cache_dir);

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--no-cache")
        .arg("--dry-run")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!stdout.contains("\u{1b}["));
    for expected in [
        "Conflict 1/1 — Readme.md",
        "files differ in content",
        "full paths:",
        "metadata diff",
        "size",
        "created",
        "modified",
        "17 B",
        "2023-",
        "newest is #1",
        "recommend keep #1",
        "(Enter)",
        "dry-run complete: 1 conflict",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
    assert!(base.exists());
    assert!(copy.exists());
    assert!(!cache_dir.exists());
}

#[test]
fn metadata_action_reprints_the_group_report() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("Readme.md"), b"base\n").unwrap();
    fs::write(root.join("Readme.md.pacnew"), b"copy\n").unwrap();
    write_config(&config, &temp.path().join("cache"));

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--no-cache")
        .arg("--dry-run")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"m\nq\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.matches("Conflict 1/1 — Readme.md").count() >= 2,
        "{stdout}"
    );
    assert!(stdout.matches("metadata diff").count() >= 2, "{stdout}");
}

#[test]
fn help_maps_actions_to_their_commands() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("song.mp3"), b"base").unwrap();
    fs::write(root.join("song.mp3.pacnew"), b"copy").unwrap();
    fs::write(
        &config,
        format!(
            "[general]\ncache_dir = {:?}\n[tools]\naudio_player = {:?}\n",
            temp.path().join("cache").to_string_lossy(),
            "/bin/echo"
        ),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--no-cache")
        .arg("--dry-run")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"?\nq\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for expected in [
        "[file type: audio]",
        "actions and commands:",
        "(v) listen ->",
        "/bin/echo",
        "(m) metadata -> built-in metadata report",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
}

#[cfg(unix)]
#[test]
fn interactive_edit_reanalyzes_the_group_before_accepting_a_choice() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let config = temp.path().join("config.toml");
    let editor = temp.path().join("edit-copy.sh");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.txt"), "same\n").unwrap();
    fs::write(root.join("file.txt.pacnew"), "different\n").unwrap();
    fs::write(&editor, "#!/bin/sh\nprintf 'same\\n' > \"$1\"\n").unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        &config,
        format!(
            "[general]\ncache_dir = {:?}\n[tools]\neditor = {:?}\n",
            temp.path().join("cache").to_string_lossy(),
            editor.to_string_lossy(),
        ),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--no-cache")
        .arg("--dry-run")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"e\n1\n\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("files differ in content"));
    assert!(stdout.contains("all files identical in content"));
    assert_eq!(
        fs::read_to_string(root.join("file.txt.pacnew")).unwrap(),
        "same\n"
    );
}

#[test]
fn wait_modes_show_final_group_positions() {
    for (wait_option, mode) in [
        ("--wait-for-scan", "scan"),
        ("--wait-for-analysis", "analysis"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("scan");
        let config = temp.path().join("config.toml");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file"), b"base").unwrap();
        fs::write(root.join("file.pacnew"), b"copy").unwrap();
        write_config(&config, &temp.path().join("cache"));

        let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
            .arg("--config")
            .arg(&config)
            .arg("resolve")
            .arg(wait_option)
            .arg("--no-cache")
            .arg("--dry-run")
            .arg(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"0\n").unwrap();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{mode} wait failed; stdout: {}\nstderr: {}",
            stdout,
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(stdout.contains("Conflict 1/1 — file"));
        assert!(!stdout.contains("Group ready —"));
    }
}

#[test]
fn scan_and_analysis_wait_modes_are_mutually_exclusive() {
    let output = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .args(["resolve", "--wait-for-scan", "--wait-for-analysis"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[test]
fn interactive_resolve_does_not_apply_choices_after_incomplete_scan() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let missing_root = temp.path().join("missing");
    let cache_dir = temp.path().join("cache");
    let backup_dir = temp.path().join("backup");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    let base = root.join("file");
    let copy = root.join("file.pacnew");
    fs::write(&base, b"base").unwrap();
    fs::write(&copy, b"copy").unwrap();
    fs::write(
        &config,
        format!(
            "[general]\ncache_dir = {:?}\nbackup_dir = {:?}\n",
            cache_dir.to_string_lossy(),
            backup_dir.to_string_lossy(),
        ),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--no-cache")
        .arg(&root)
        .arg(&missing_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"1\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("scan incomplete; refusing to resolve")
    );
    assert_eq!(fs::read(base).unwrap(), b"base");
    assert_eq!(fs::read(copy).unwrap(), b"copy");
    assert!(!backup_dir.exists());
}

#[test]
fn interactive_resolve_can_skip_and_quit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    for name in ["first.txt", "second.txt"] {
        fs::write(root.join(name), b"base").unwrap();
        fs::write(root.join(format!("{name}.pacnew")), b"copy").unwrap();
    }
    write_config(&config, &temp.path().join("cache"));

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--no-cache")
        .arg("--dry-run")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"s\nQ\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("aborted after 1 of 2 conflicts"),
        "{stdout}"
    );
    assert!(
        stdout
            .matches("Finishing pending conflict analysis; no further conflicts will be prompted")
            .count()
            <= 1
    );
    assert!(stdout.contains("(1 skipped)"));
    for name in ["first.txt", "second.txt"] {
        assert_eq!(fs::read(root.join(name)).unwrap(), b"base");
        assert_eq!(
            fs::read(root.join(format!("{name}.pacnew"))).unwrap(),
            b"copy"
        );
    }
}

#[test]
fn interactive_force_quit_does_not_resolve_or_save_scan_estimate() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("scan");
    let cache_dir = temp.path().join("cache");
    let backup_dir = temp.path().join("backup");
    let config = temp.path().join("config.toml");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file"), b"base").unwrap();
    fs::write(root.join("file.pacnew"), b"copy").unwrap();
    fs::write(
        &config,
        format!(
            "[general]\ncache_dir = {:?}\nbackup_dir = {:?}\n",
            cache_dir.to_string_lossy(),
            backup_dir.to_string_lossy(),
        ),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_deconflict"))
        .arg("--config")
        .arg(&config)
        .arg("resolve")
        .arg("--wait-for-scan")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("quit; scan cancelled and no choices were applied"),
        "{stdout}"
    );
    assert!(root.join("file").exists());
    assert!(root.join("file.pacnew").exists());
    assert!(!backup_dir.exists());
    let key = cache_key([&root]).unwrap();
    assert!(!cache_dir.join(format!("{key}.count")).exists());
}
