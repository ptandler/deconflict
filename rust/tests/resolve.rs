use deconflict_core::{
    analysis::analyze_scanned_group,
    config::ScanConfig,
    patterns::Patterns,
    resolve::{prepare, Decision},
    scan::scan,
};
use std::sync::atomic::AtomicBool;

fn prepare_fixture() -> (tempfile::TempDir, deconflict_core::resolve::Prepared) {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"base").unwrap();
    std::fs::write(root.join("file.pacnew"), b"copy").unwrap();

    let config = ScanConfig::default();
    let patterns = Patterns::build(&config).unwrap();
    let result = scan(
        std::slice::from_ref(&root),
        &config,
        &patterns,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(result.errors.is_empty());
    assert_eq!(result.groups.len(), 1);
    let analysis = analyze_scanned_group(&result.groups[0]).unwrap().analysis;
    let prepared = prepare(&result, 0, &analysis, &[root]).unwrap();
    (fixture, prepared)
}

#[test]
fn public_api_resolves_copy_and_dry_run() {
    let (fixture, prepared) = prepare_fixture();
    let root = fixture.path().join("root");

    let preview = prepared
        .resolve(Decision::KeepCopy(0), &fixture.path().join("backup"), true)
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(std::fs::read(root.join("file")).unwrap(), b"base");
    assert_eq!(std::fs::read(root.join("file.pacnew")).unwrap(), b"copy");
    assert!(!fixture.path().join("backup").exists());

    prepared
        .resolve(Decision::KeepCopy(0), &fixture.path().join("backup"), false)
        .unwrap();
    assert_eq!(std::fs::read(root.join("file")).unwrap(), b"copy");
    assert!(!root.join("file.pacnew").exists());
    assert!(fixture.path().join("backup/decision-log.jsonl").is_file());
}
