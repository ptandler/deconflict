use crate::analysis::{analyze, FileInfo};
use crate::config::ScanConfig;
use crate::patterns::{ConflictMatch, Patterns};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub base: PathBuf,
    pub conflicts: Vec<(PathBuf, ConflictMatch)>,
}

#[derive(Debug)]
pub enum Event {
    /// A file snapshot, not an authorization to mutate or a finalized group.
    Provisional(PathBuf, Result<FileInfo, String>),
    Error(PathBuf, String),
}

#[derive(Debug)]
pub struct ScanResult {
    pub groups: Vec<Group>,
    pub errors: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

enum Job {
    Directory(Directory),
    Analysis(PathBuf),
}
enum Directory {
    New(PathBuf),
    Open(PathBuf, fs::ReadDir),
}
enum Message {
    Entry(PathBuf),
    Problem(PathBuf, String),
    Analysis(PathBuf, Result<FileInfo, String>),
    Done {
        directory: bool,
        continuation: Option<Directory>,
    },
}

struct DirectoryQueue {
    pending: VecDeque<Directory>,
    reserved: usize,
    limit: usize,
}

impl DirectoryQueue {
    fn new(roots: &[PathBuf], limit: usize) -> Self {
        Self {
            pending: roots.iter().cloned().map(Directory::New).collect(),
            reserved: 0,
            limit,
        }
    }

    fn pop(&mut self) -> Option<Directory> {
        let index = if self.reserved < self.limit {
            0
        } else {
            self.pending
                .iter()
                .position(|d| matches!(d, Directory::Open(..)))?
        };
        let directory = self.pending.remove(index)?;
        if matches!(directory, Directory::New(_)) {
            self.reserved += 1;
        }
        Some(directory)
    }

    fn complete(&mut self, continuation: Option<Directory>) {
        if let Some(directory) = continuation {
            self.pending.push_back(directory);
        } else {
            self.reserved -= 1;
        }
    }
}

fn prefer_directory(analysis_streak: usize, pending_analyses: usize) -> bool {
    analysis_streak >= 3 || pending_analyses == 0
}

/// Bounds dispatched jobs, channel messages and open directory iterators, not pending directory
/// paths, analyses, visited identities, or results. When iterator capacity is full,
/// unopened directories wait for an open iterator to finish. Events are provisional; only the
/// returned groups are final, and only if
/// `cancelled` is false and `errors` is empty.
pub fn scan<F: FnMut(&Event)>(
    roots: &[PathBuf],
    config: &ScanConfig,
    patterns: &Patterns,
    cancel: &AtomicBool,
    mut event: F,
) -> Result<ScanResult, String> {
    if config.workers == 0 || config.max_in_flight == 0 {
        return Err("workers and max_in_flight must be positive".into());
    }
    let (job_tx, job_rx) = mpsc::sync_channel::<Job>(config.max_in_flight);
    let (msg_tx, msg_rx) = mpsc::sync_channel::<Message>(config.max_in_flight);
    let job_rx = Arc::new(Mutex::new(job_rx));
    let mut groups: BTreeMap<PathBuf, Vec<(PathBuf, ConflictMatch)>> = BTreeMap::new();
    let mut errors = Vec::new();
    let concurrency = config.workers.min(config.max_in_flight);
    // Two slots allow a single worker to interleave directories as well as analyses.
    let mut dirs = DirectoryQueue::new(roots, concurrency.max(2));
    let mut analyses = VecDeque::new();
    let mut visited = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut active = 0usize;
    let mut analysis_streak = 0usize;
    let mut cancelled = false;
    std::thread::scope(|scope| {
        for _ in 0..concurrency {
            let rx = Arc::clone(&job_rx);
            let tx = msg_tx.clone();
            scope.spawn(move || loop {
                let job = match rx.lock().unwrap().recv() {
                    Ok(job) => job,
                    Err(_) => break,
                };
                let directory_job = matches!(&job, Job::Directory(_));
                let continuation = match job {
                    Job::Directory(directory) => {
                        let (path, mut entries) = match directory {
                            Directory::New(path) => match fs::read_dir(&path) {
                                Ok(entries) => (path, entries),
                                Err(e) => {
                                    if tx.send(Message::Problem(path, e.to_string())).is_err()
                                        || tx
                                            .send(Message::Done {
                                                directory: true,
                                                continuation: None,
                                            })
                                            .is_err()
                                    {
                                        return;
                                    }
                                    continue;
                                }
                            },
                            Directory::Open(path, entries) => (path, entries),
                        };
                        match entries.next() {
                            Some(entry) => {
                                let message = match entry {
                                    Ok(entry) => Message::Entry(entry.path()),
                                    Err(e) => Message::Problem(path.clone(), e.to_string()),
                                };
                                if tx.send(message).is_err() {
                                    return;
                                }
                                Some(Directory::Open(path, entries))
                            }
                            None => None,
                        }
                    }
                    Job::Analysis(path) => {
                        let result = analyze(&path).map_err(|e| e.to_string());
                        if tx.send(Message::Analysis(path, result)).is_err() {
                            return;
                        }
                        None
                    }
                };
                if tx
                    .send(Message::Done {
                        directory: directory_job,
                        continuation,
                    })
                    .is_err()
                {
                    return;
                }
            });
        }
        drop(msg_tx);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
            cancelled |= cancel.load(Ordering::Relaxed);
            if cancelled {
                dirs.pending.clear();
                analyses.clear();
            }
            while active < concurrency && !cancelled && !cancel.load(Ordering::Relaxed) {
                let directory = if prefer_directory(analysis_streak, analyses.len()) {
                    dirs.pop()
                } else {
                    None
                };
                let job = if let Some(directory) = directory {
                    analysis_streak = 0;
                    match directory {
                        Directory::Open(path, entries) => {
                            Job::Directory(Directory::Open(path, entries))
                        }
                        Directory::New(path) => match fs::canonicalize(&path) {
                            Ok(identity) if visited.insert(identity.clone()) => {
                                Job::Directory(Directory::New(identity))
                            }
                            Ok(_) => {
                                dirs.complete(None);
                                continue;
                            }
                            Err(e) => {
                                dirs.complete(None);
                                let message = e.to_string();
                                event(&Event::Error(path.clone(), message.clone()));
                                errors.push((path, message));
                                continue;
                            }
                        },
                    }
                } else if !analyses.is_empty() {
                    analysis_streak = analysis_streak.saturating_add(1);
                    Job::Analysis(analyses.pop_front().unwrap())
                } else {
                    break;
                };
                // At most `min(workers, max_in_flight)` jobs exist, so this never blocks.
                job_tx.try_send(job).expect("bounded scheduler capacity");
                active += 1;
            }
            cancelled |= cancel.load(Ordering::Relaxed);
            if active == 0 {
                break;
            }
            match msg_rx.recv().expect("workers terminated unexpectedly") {
                Message::Done {
                    directory,
                    continuation,
                } => {
                    active -= 1;
                    if directory {
                        dirs.complete(if cancelled || cancel.load(Ordering::Relaxed) {
                            None
                        } else {
                            continuation
                        });
                    }
                }
                Message::Problem(path, message) => {
                    event(&Event::Error(path.clone(), message.clone()));
                    errors.push((path, message));
                }
                Message::Analysis(path, result) => {
                    if let Err(message) = &result {
                        errors.push((path.clone(), message.clone()));
                    }
                    event(&Event::Provisional(path, result));
                }
                Message::Entry(path) if !cancelled && !cancel.load(Ordering::Relaxed) => {
                    let meta = match fs::symlink_metadata(&path) {
                        Ok(m) => m,
                        Err(e) => {
                            let s = e.to_string();
                            event(&Event::Error(path.clone(), s.clone()));
                            errors.push((path, s));
                            continue;
                        }
                    };
                    if meta.file_type().is_symlink() && !config.follow_symlinks {
                        continue;
                    }
                    let meta = if meta.file_type().is_symlink() {
                        match fs::metadata(&path) {
                            Ok(m) => m,
                            Err(e) => {
                                let s = e.to_string();
                                event(&Event::Error(path.clone(), s.clone()));
                                errors.push((path, s));
                                continue;
                            }
                        }
                    } else {
                        meta
                    };
                    if meta.is_dir() {
                        dirs.pending.push_back(Directory::New(path));
                    } else if meta.is_file() && files.insert(path.clone()) {
                        match path.file_name().and_then(|n| n.to_str()) {
                            Some(name) => match patterns.match_name(name) {
                                Ok(Some(m)) => {
                                    if m.base_name.is_empty()
                                        || std::path::Path::new(&m.base_name)
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            != Some(m.base_name.as_str())
                                    {
                                        let message =
                                            "pattern base must be a non-empty filename".to_string();
                                        event(&Event::Error(path.clone(), message.clone()));
                                        errors.push((path, message));
                                        continue;
                                    }
                                    let base = path.with_file_name(&m.base_name);
                                    groups.entry(base).or_default().push((path.clone(), m));
                                    analyses.push_back(path);
                                }
                                Ok(None) => (),
                                Err(s) => {
                                    event(&Event::Error(path.clone(), s.clone()));
                                    errors.push((path, s));
                                }
                            },
                            None => {
                                let s = "non-UTF-8 filename cannot be matched".to_string();
                                event(&Event::Error(path.clone(), s.clone()));
                                errors.push((path, s));
                            }
                        }
                    }
                }
                Message::Entry(_) => (),
            }
        }));
        drop(job_tx);
        drop(msg_rx);
        if let Err(payload) = outcome {
            std::panic::resume_unwind(payload);
        }
    });
    let groups = groups
        .into_iter()
        .map(|(base, mut conflicts)| {
            conflicts.sort_by(|a, b| a.0.cmp(&b.0));
            Group { base, conflicts }
        })
        .collect();
    errors.sort();
    Ok(ScanResult {
        groups,
        errors,
        cancelled: cancelled || cancel.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_gets_a_turn_even_with_a_large_analysis_backlog() {
        assert!(!prefer_directory(2, 100));
        assert!(prefer_directory(3, 100));
        assert!(prefer_directory(0, 0));
    }

    #[test]
    fn cancellation_before_scan_does_not_dispatch_work() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.pacnew"), b"new").unwrap();
        let config = ScanConfig::default();
        let patterns = Patterns::build(&config).unwrap();
        let mut events = 0;
        let result = scan(
            &[dir.path().to_path_buf()],
            &config,
            &patterns,
            &AtomicBool::new(true),
            |_| events += 1,
        )
        .unwrap();
        assert!(result.cancelled);
        assert!(result.groups.is_empty());
        assert!(result.errors.is_empty());
        assert_eq!(events, 0);
    }

    #[test]
    fn cancellation_in_error_callback_stops_dispatching_roots() {
        let dir = tempfile::tempdir().unwrap();
        let roots = [dir.path().join("missing1"), dir.path().join("missing2")];
        let config = ScanConfig {
            workers: 1,
            max_in_flight: 1,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let mut events = 0;
        let result = scan(&roots, &config, &patterns, &cancel, |_| {
            events += 1;
            cancel.store(true, Ordering::Relaxed);
        })
        .unwrap();
        assert!(result.cancelled);
        assert_eq!(events, 1);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].0, roots[0]);
        assert!(result.groups.is_empty());
    }

    #[test]
    fn directory_reservations_bound_open_and_in_flight_handles() {
        let dir = tempfile::tempdir().unwrap();
        let roots: Vec<_> = (0..5)
            .map(|i| {
                let path = dir.path().join(i.to_string());
                fs::create_dir(&path).unwrap();
                path
            })
            .collect();
        let mut queue = DirectoryQueue::new(&roots, 2);
        let first = queue.pop().unwrap();
        let second = queue.pop().unwrap();
        assert_eq!(queue.reserved, 2);
        assert!(queue.pop().is_none());
        let Directory::New(path) = first else {
            panic!("expected new directory")
        };
        queue.complete(Some(Directory::Open(
            path.clone(),
            fs::read_dir(&path).unwrap(),
        )));
        assert!(matches!(queue.pop(), Some(Directory::Open(..))));
        assert!(queue.pop().is_none());
        assert!(matches!(second, Directory::New(_)));
        queue.complete(None);
        assert!(matches!(queue.pop(), Some(Directory::New(_))));
        assert_eq!(queue.reserved, 2);
    }

    #[test]
    fn full_directory_queue_resumes_open_iterators_before_new_roots() {
        let dir = tempfile::tempdir().unwrap();
        let roots: Vec<_> = (0..4)
            .map(|i| {
                let path = dir.path().join(i.to_string());
                fs::create_dir(&path).unwrap();
                path
            })
            .collect();
        let mut queue = DirectoryQueue::new(&roots, 1);
        let Directory::New(first) = queue.pop().unwrap() else {
            panic!("expected first root");
        };
        assert_eq!(first, roots[0]);
        queue.complete(Some(Directory::Open(
            first.clone(),
            fs::read_dir(&first).unwrap(),
        )));
        let Directory::Open(resumed, _) = queue.pop().unwrap() else {
            panic!("open iterator must bypass waiting roots");
        };
        assert_eq!(resumed, first);
        assert!(queue.pop().is_none());
        queue.complete(None);
        for expected in &roots[1..] {
            let Directory::New(path) = queue.pop().unwrap() else {
                panic!("expected waiting root");
            };
            assert_eq!(&path, expected);
            queue.complete(None);
        }
        assert_eq!(queue.reserved, 0);
        assert!(queue.pop().is_none());
    }

    #[test]
    fn overlapping_roots_are_scanned_once_under_handle_pressure() {
        let dir = tempfile::tempdir().unwrap();
        let mut roots = vec![dir.path().to_path_buf()];
        for i in 0..30 {
            let sub = dir.path().join(i.to_string());
            fs::create_dir(&sub).unwrap();
            fs::write(sub.join("file.pacnew"), b"new").unwrap();
            roots.push(sub);
        }
        let config = ScanConfig {
            workers: 4,
            max_in_flight: 2,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        let mut count = 0;
        let result = scan(
            &roots,
            &config,
            &patterns,
            &AtomicBool::new(false),
            |event| {
                if matches!(event, Event::Provisional(..)) {
                    count += 1;
                }
            },
        )
        .unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(result.groups.len(), 30);
        assert_eq!(count, 30);
    }

    #[test]
    fn callback_panic_does_not_deadlock_workers() {
        let dir = tempfile::tempdir().unwrap();
        let mut roots = Vec::new();
        for i in 0..100 {
            let root = dir.path().join(i.to_string());
            fs::create_dir(&root).unwrap();
            fs::write(root.join("file.pacnew"), b"new").unwrap();
            roots.push(root);
        }
        let config = ScanConfig {
            workers: 4,
            max_in_flight: 4,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        let panic = std::panic::catch_unwind(|| {
            let _ = scan(&roots, &config, &patterns, &AtomicBool::new(false), |_| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                panic!("callback panic");
            });
        });
        assert!(panic.is_err());
    }

    #[test]
    fn single_worker_analyses_before_finishing_large_directory() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..100 {
            fs::write(dir.path().join(format!("{i}.pacnew")), b"new").unwrap();
        }
        let config = ScanConfig {
            workers: 1,
            max_in_flight: 1,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let result = scan(
            &[dir.path().to_path_buf()],
            &config,
            &patterns,
            &cancel,
            |event| {
                if matches!(event, Event::Provisional(..)) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap();
        assert!(result.cancelled);
        assert!(result.errors.is_empty());
        assert!(!result.groups.is_empty());
        assert!(
            result.groups.len() < 100,
            "directory was exhausted before analysis"
        );
    }

    #[test]
    fn directories_and_analyses_both_make_progress() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        for i in 0..50 {
            fs::write(first.join(format!("{i}.pacnew")), b"new").unwrap();
        }
        fs::write(second.join("other.pacnew"), b"new").unwrap();
        let config = ScanConfig {
            workers: 1,
            max_in_flight: 1,
            ..ScanConfig::default()
        };
        let patterns = Patterns::build(&config).unwrap();
        let mut results = Vec::new();
        let result = scan(
            &[first, second.clone()],
            &config,
            &patterns,
            &AtomicBool::new(false),
            |event| {
                if let Event::Provisional(path, _) = event {
                    results.push(path.clone());
                }
            },
        )
        .unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(results.len(), 51);
        assert!(
            results
                .iter()
                .position(|path| path.starts_with(&second))
                .unwrap()
                < 5
        );
    }

    #[test]
    fn small_capacities_complete_wide_nested_trees() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            let sub = dir.path().join(i.to_string());
            fs::create_dir(&sub).unwrap();
            for j in 0..10 {
                fs::write(sub.join(format!("{j}.pacnew")), b"new").unwrap();
            }
        }
        for (workers, max_in_flight) in [(1, 1), (4, 1), (4, 2)] {
            let config = ScanConfig {
                workers,
                max_in_flight,
                ..ScanConfig::default()
            };
            let patterns = Patterns::build(&config).unwrap();
            let mut count = 0;
            let result = scan(
                &[dir.path().to_path_buf()],
                &config,
                &patterns,
                &AtomicBool::new(false),
                |event| {
                    if matches!(event, Event::Provisional(..)) {
                        count += 1;
                    }
                },
            )
            .unwrap();
            assert!(!result.cancelled);
            assert!(result.errors.is_empty());
            assert_eq!(result.groups.len(), 200);
            assert_eq!(count, 200);
        }
    }
}
