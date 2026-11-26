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
        "Group 1/1 — Readme.md",
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
        "dry-run complete: 1 group(s)",
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
    child.stdin.take().unwrap().write_all(b"s\nq\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("aborted after 1 of 2 group(s)"));
    assert!(stdout.contains("(1 skipped)"));
    for name in ["first.txt", "second.txt"] {
        assert_eq!(fs::read(root.join(name)).unwrap(), b"base");
        assert_eq!(
            fs::read(root.join(format!("{name}.pacnew"))).unwrap(),
            b"copy"
        );
    }
}
