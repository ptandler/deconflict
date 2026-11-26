use deconflict_core::{
    analysis,
    config::{Config, CustomPattern, ScanConfig},
    patterns::Patterns,
    scan::{scan, Event},
};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

#[test]
fn patterns_and_custom_regex() {
    let mut cfg = ScanConfig::default();
    cfg.patterns.push(CustomPattern {
        name: "custom".into(),
        base_name: r"(?P<base>.+)(?=\.bak)\.bak".into(),
        winner: "base".into(),
    });
    let p = Patterns::build(&cfg).unwrap();
    assert_eq!(
        p.match_name("a (conflicted copy 2020).txt")
            .unwrap()
            .unwrap()
            .base_name,
        "a.txt"
    );
    assert_eq!(p.match_name("a.pacnew").unwrap().unwrap().base_name, "a");
    assert_eq!(
        p.match_name("a.sync-conflict-20200101-ABC.txt")
            .unwrap()
            .unwrap()
            .base_name,
        "a.txt"
    );
    assert_eq!(p.match_name("file.bak").unwrap().unwrap().winner, "base");
    cfg.patterns[0].base_name = "missing".into();
    assert!(Patterns::build(&cfg).is_err());
}

#[test]
fn hash_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    fs::write(&file, b"abc").unwrap();
    let info = analysis::analyze(&file).unwrap();
    assert_eq!(
        info.sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(info.is_text);
    let cfg = Config::parse("[scan]\nworkers = 1\nmax_in_flight = 2\n").unwrap();
    assert_eq!(cfg.scan.workers, 1);
    assert_eq!(cfg.scan.enabled_patterns.len(), 3);
}

#[test]
fn python_global_flags_in_custom_pattern() {
    let mut cfg = ScanConfig::default();
    cfg.patterns.push(CustomPattern {
        name: "case-insensitive".into(),
        base_name: r"(?i)(?P<base>.+)\.backup".into(),
        winner: "copy".into(),
    });
    let patterns = Patterns::build(&cfg).unwrap();
    assert_eq!(
        patterns
            .match_name("File.BACKUP")
            .unwrap()
            .unwrap()
            .base_name,
        "File"
    );
}

#[test]
fn group_analysis_compares_existing_base_and_copies() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("file");
    let copy = dir.path().join("file.pacnew");
    fs::write(&base, b"same").unwrap();
    fs::write(&copy, b"same").unwrap();
    let result = analysis::analyze_group(&base, std::slice::from_ref(&copy)).unwrap();
    assert!(result.base.is_some());
    assert!(result.all_equal);
    assert!(result.all_text);
    fs::write(&copy, b"different\0").unwrap();
    let result = analysis::analyze_group(&base, std::slice::from_ref(&copy)).unwrap();
    assert!(!result.all_equal);
    assert!(!result.all_text);
    fs::remove_file(&base).unwrap();
    let result = analysis::analyze_group(&base, &[copy]).unwrap();
    assert!(result.base.is_none());
    assert!(result.all_equal);
}

#[test]
fn deterministic_groups_and_provisional_results() {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["a", "b", "c"] {
        fs::create_dir(dir.path().join(sub)).unwrap();
        fs::write(dir.path().join(sub).join("x.pacnew"), b"new").unwrap();
        fs::write(dir.path().join(sub).join("x.pacsave"), b"old").unwrap();
    }
    let mut previous = None;
    for workers in [1, 4, 2, 4] {
        let cfg = ScanConfig {
            workers,
            max_in_flight: 2,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&cfg).unwrap();
        let mut events = 0;
        let result = scan(
            &[dir.path().to_path_buf(), dir.path().join("a")],
            &cfg,
            &patterns,
            &AtomicBool::new(false),
            |e| {
                if matches!(e, Event::Provisional(_, Ok(_))) {
                    events += 1
                }
            },
        )
        .unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(events, 6);
        assert_eq!(result.groups.len(), 3);
        if let Some(old) = &previous {
            assert_eq!(old, &result.groups);
        }
        previous = Some(result.groups);
    }
}

#[test]
fn wide_tree_with_varied_scheduling_and_single_slot() {
    let dir = tempfile::tempdir().unwrap();
    for sub in 0..24 {
        let path = dir.path().join(format!("dir{sub:02}"));
        fs::create_dir(&path).unwrap();
        for file in 0..8 {
            fs::write(path.join(format!("file{file}.pacnew")), b"content").unwrap();
        }
    }
    let mut previous = None;
    for (workers, limit, mut seed) in [(1, 1, 3u32), (4, 1, 17), (4, 3, 29)] {
        let cfg = ScanConfig {
            workers,
            max_in_flight: limit,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&cfg).unwrap();
        let mut events = 0;
        let result = scan(
            &[dir.path().into()],
            &cfg,
            &patterns,
            &AtomicBool::new(false),
            |event| {
                if matches!(event, Event::Provisional(_, Ok(_))) {
                    events += 1;
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    if seed % 7 == 0 {
                        std::thread::yield_now();
                    }
                }
            },
        )
        .unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(events, 192);
        assert_eq!(result.groups.len(), 192);
        if let Some(old) = &previous {
            assert_eq!(old, &result.groups);
        }
        previous = Some(result.groups);
    }
}

#[test]
fn cancellation_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ScanConfig::default();
    let patterns = Patterns::build(&cfg).unwrap();
    let cancelled = AtomicBool::new(true);
    assert!(
        scan(&[dir.path().into()], &cfg, &patterns, &cancelled, |_| {})
            .unwrap()
            .cancelled
    );
    cancelled.store(false, Ordering::Relaxed);
    let result = scan(
        &[dir.path().join("missing")],
        &cfg,
        &patterns,
        &cancelled,
        |_| {},
    )
    .unwrap();
    assert_eq!(result.errors.len(), 1);
}

#[test]
fn cancellation_remains_latched_while_in_flight_errors_are_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let roots = [dir.path().join("first"), dir.path().join("second")];
    for root in &roots {
        fs::write(root, b"not a directory").unwrap();
    }
    let cfg = ScanConfig {
        workers: 2,
        max_in_flight: 2,
        ..ScanConfig::default()
    };
    let patterns = Patterns::build(&cfg).unwrap();
    let cancel = AtomicBool::new(false);
    let mut events = 0;
    let result = scan(&roots, &cfg, &patterns, &cancel, |event| {
        if matches!(event, Event::Error(..)) {
            events += 1;
            cancel.store(events == 1, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert_eq!(events, 2);
    assert_eq!(result.errors.len(), 2);
    assert!(result.cancelled);
}

#[test]
fn worker_count_is_limited_by_in_flight_capacity() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.pacnew"), b"new").unwrap();
    let cfg = ScanConfig {
        workers: usize::MAX,
        max_in_flight: 1,
        ..ScanConfig::default()
    };
    let patterns = Patterns::build(&cfg).unwrap();
    let result = scan(
        &[dir.path().to_path_buf()],
        &cfg,
        &patterns,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(result.errors.is_empty());
    assert_eq!(result.groups.len(), 1);
}

#[test]
fn root_removed_during_scan_is_reported_without_rescanning() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let removed = dir.path().join("removed");
    fs::create_dir(&removed).unwrap();
    fs::write(removed.join("file.pacnew"), b"new").unwrap();
    let cfg = ScanConfig {
        workers: 1,
        max_in_flight: 1,
        ..ScanConfig::default()
    };
    let patterns = Patterns::build(&cfg).unwrap();
    let mut errors = Vec::new();
    let result = scan(
        &[missing.clone(), removed.clone()],
        &cfg,
        &patterns,
        &AtomicBool::new(false),
        |event| {
            if let Event::Error(path, _) = event {
                errors.push(path.clone());
                if path == &missing {
                    fs::remove_dir_all(&removed).unwrap();
                }
            }
        },
    )
    .unwrap();
    assert_eq!(errors, vec![missing.clone(), removed.clone()]);
    assert_eq!(
        result.errors.iter().map(|e| &e.0).collect::<Vec<_>>(),
        vec![&missing, &removed]
    );
    assert!(result.groups.is_empty());
    assert!(!result.cancelled);
}

#[cfg(unix)]
#[test]
fn entry_errors_are_reported_and_sorted_across_scheduler_sizes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let invalid = dir
        .path()
        .join(OsString::from_vec(b"invalid-\xff".to_vec()));
    let dangling = dir.path().join("dangling.pacnew");
    fs::write(&invalid, b"invalid name").unwrap();
    symlink(dir.path().join("missing"), &dangling).unwrap();
    fs::write(dir.path().join("valid.pacnew"), b"new").unwrap();

    let mut previous = None;
    for (workers, max_in_flight) in [(1, 1), (4, 1), (4, 3)] {
        let cfg = ScanConfig {
            workers,
            max_in_flight,
            follow_symlinks: true,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&cfg).unwrap();
        let mut event_errors = Vec::new();
        let result = scan(
            &[dir.path().to_path_buf()],
            &cfg,
            &patterns,
            &AtomicBool::new(false),
            |event| {
                if let Event::Error(path, message) = event {
                    event_errors.push((path.clone(), message.clone()));
                }
            },
        )
        .unwrap();
        event_errors.sort();
        assert_eq!(event_errors, result.errors);
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.errors[0].0, dangling);
        assert_eq!(result.errors[1].0, invalid);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].base, dir.path().join("valid"));
        assert!(!result.cancelled);
        if let Some(old) = &previous {
            assert_eq!(old, &result.errors);
        }
        previous = Some(result.errors);
    }
}

#[test]
fn concurrent_scans_have_independent_results() {
    use std::sync::{Arc, Barrier};

    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        fs::write(dir.path().join(format!("{i:02}.pacnew")), b"new").unwrap();
    }
    let barrier = Arc::new(Barrier::new(4));
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let barrier = Arc::clone(&barrier);
                let root = dir.path().to_path_buf();
                scope.spawn(move || {
                    let cfg = ScanConfig {
                        workers: i + 1,
                        max_in_flight: i % 3 + 1,
                        ..ScanConfig::default()
                    };
                    let patterns = Patterns::build(&cfg).unwrap();
                    let mut events = 0;
                    barrier.wait();
                    let result = scan(
                        &[root.clone(), root],
                        &cfg,
                        &patterns,
                        &AtomicBool::new(false),
                        |event| {
                            if let Event::Provisional(_, Ok(_)) = event {
                                events += 1;
                            }
                        },
                    )
                    .unwrap();
                    assert!(result.errors.is_empty(), "{:?}", result.errors);
                    assert!(!result.cancelled);
                    assert_eq!(events, 20);
                    result.groups
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results[0].len(), 20);
        assert!(results.iter().all(|groups| groups == &results[0]));
    });
}

#[cfg(unix)]
#[test]
fn overlapping_symlink_roots_and_dangling_entries() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    let alias = dir.path().join("alias");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("file.pacnew"), b"new").unwrap();
    symlink(&real, &alias).unwrap();
    symlink(real.join("missing"), real.join("dangling.pacnew")).unwrap();
    for follow_symlinks in [false, true] {
        let cfg = ScanConfig {
            follow_symlinks,
            workers: 2,
            max_in_flight: 2,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&cfg).unwrap();
        let mut events = 0;
        let result = scan(
            &[alias.clone(), real.clone(), real.clone()],
            &cfg,
            &patterns,
            &AtomicBool::new(false),
            |event| {
                if matches!(event, Event::Provisional(_, Ok(_))) {
                    events += 1;
                }
            },
        )
        .unwrap();
        assert_eq!(events, 1);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].base, real.join("file"));
        assert_eq!(result.errors.len(), usize::from(follow_symlinks));
        if follow_symlinks {
            assert_eq!(result.errors[0].0, real.join("dangling.pacnew"));
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_loop_terminates() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("x.pacnew"), b"x").unwrap();
    symlink(dir.path(), dir.path().join("loop")).unwrap();
    let cfg = ScanConfig {
        follow_symlinks: true,
        ..ScanConfig::default()
    };
    let p = Patterns::build(&cfg).unwrap();
    let result = scan(
        &[dir.path().into()],
        &cfg,
        &p,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.groups.len(), 1);
    assert!(result.errors.is_empty());
}
