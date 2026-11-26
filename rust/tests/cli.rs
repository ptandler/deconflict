use deconflict_core::cache::cache_key;
use std::{fs, path::Path, process::Command};

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
