//! Nonblocking scan/analysis coordinator shared by the TUI event loop.
//!
//! The UI never mutates files directly: decisions are buffered here and applied
//! only after a complete, uncancelled scan and complete group analyses.

use crate::{
    interactive::ensure_scan_complete,
    progress::{
        conflict_count, perform_scan_with_progress_cancel_events, ScanProgress, ScanTimings,
    },
};
use deconflict_core::{
    analysis::{
        analyze_scanned_group_with_media, analyze_scanned_group_with_snapshots, FileInfo,
        StampedGroupAnalysis,
    },
    cache::{cache_key, FileCache},
    config::ScanConfig,
    media::Tools as MediaTools,
    patterns::Patterns,
    resolve::{prepare, Decision},
    scan::{Event, Group, ScanResult},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(super) struct SessionOptions {
    pub(super) no_cache: bool,
    pub(super) refresh: bool,
    pub(super) dry_run: bool,
}

pub(super) struct SessionRequest<'a> {
    pub(super) roots: &'a [PathBuf],
    pub(super) scan_config: &'a ScanConfig,
    pub(super) patterns: &'a Patterns,
    pub(super) backup: &'a Path,
    pub(super) cache_dir: &'a Path,
    pub(super) media: &'a MediaTools,
    pub(super) options: SessionOptions,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ScanStatus {
    pub(super) files: usize,
    pub(super) directories: usize,
    pub(super) copies_queued: usize,
    pub(super) copies_done: usize,
    pub(super) errors: usize,
    pub(super) error_messages: Vec<(PathBuf, String)>,
    pub(super) current_path: Option<PathBuf>,
    pub(super) started: Option<Instant>,
}

impl ScanStatus {
    fn observe(&mut self, event: &Event) {
        self.started.get_or_insert_with(Instant::now);
        match event {
            Event::File(path) => {
                self.files += 1;
                self.current_path = Some(path.clone());
            }
            Event::Directory(path) => {
                self.directories += 1;
                self.current_path = Some(path.clone());
            }
            Event::AnalysisQueued(path) => {
                self.copies_queued += 1;
                self.current_path = Some(path.clone());
            }
            Event::Provisional(path, _) => {
                self.copies_done += 1;
                self.current_path = Some(path.clone());
            }
            Event::Error(path, error) => {
                self.errors += 1;
                self.error_messages.push((path.clone(), error.clone()));
                self.current_path = Some(path.clone());
            }
            Event::DirectoryComplete(_) | Event::GroupReady(_) => {}
        }
    }
}

pub(super) struct GroupEntry {
    pub(super) group: Group,
    pub(super) label: String,
    pub(super) pattern: String,
    pub(super) analysis: Option<Result<StampedGroupAnalysis, String>>,
    pub(super) decision: Option<Decision>,
    pub(super) skipped: bool,
}

impl From<Group> for GroupEntry {
    fn from(group: Group) -> Self {
        let label = group
            .base
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let pattern = group
            .conflicts
            .first()
            .map(|(_, matched)| matched.pattern.clone())
            .unwrap_or_default();
        Self {
            group,
            label,
            pattern,
            analysis: None,
            decision: None,
            skipped: false,
        }
    }
}

impl GroupEntry {
    pub(super) fn is_decided(&self) -> bool {
        self.decision.is_some() || self.skipped
    }
}

fn analyzed_group_counts(groups: &[GroupEntry]) -> (usize, usize) {
    let done = groups
        .iter()
        .filter(|entry| entry.analysis.is_some())
        .count();
    (done, groups.len())
}

pub(super) struct Summary {
    pub(super) lines: Vec<String>,
}

enum Message {
    GroupReady(Group),
    GroupAnalysisReady {
        base: PathBuf,
        result: Result<StampedGroupAnalysis, String>,
    },
    ScanFinished {
        result: Result<ScanResult, String>,
        timings: Option<ScanTimings>,
    },
    AnalysisFinished,
    WorkerError(String),
}

pub(super) struct Session {
    receiver: Receiver<Message>,
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<ScanStatus>>,
    scanner: Option<JoinHandle<()>>,
    analysis: Option<JoinHandle<()>>,
    roots: Vec<PathBuf>,
    backup: PathBuf,
    cache: FileCache,
    media: MediaTools,
    options: SessionOptions,
    pub(super) groups: Vec<GroupEntry>,
    scan_result: Option<Result<ScanResult, String>>,
    timings: Option<ScanTimings>,
    final_elapsed: Option<Duration>,
    final_summary: Option<String>,
    analysis_finished: bool,
    estimated_file_count: Option<u64>,
    worker_error: Option<String>,
    quit: bool,
    force_quit: bool,
}

impl Session {
    pub(super) fn start(request: SessionRequest<'_>) -> Result<Self, String> {
        let key = cache_key(request.roots).map_err(|e| format!("cannot key cache: {e}"))?;
        let disabled = request.options.no_cache || request.options.dry_run;
        let mut estimate_cache = FileCache::new(request.cache_dir, &key, disabled);
        let estimated_file_count = if disabled {
            None
        } else {
            estimate_cache.file_count()
        };

        let (message_tx, receiver) = mpsc::channel();
        let (group_tx, group_rx) = mpsc::channel::<Group>();
        let status = Arc::new(Mutex::new(ScanStatus::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let snapshots = Arc::new(Mutex::new(HashMap::<PathBuf, FileInfo>::new()));

        let analysis_tx = message_tx.clone();
        let analysis_cancel = Arc::clone(&cancel);
        let analysis_snapshots = Arc::clone(&snapshots);
        let analysis_cache_dir = request.cache_dir.to_path_buf();
        let analysis_key = key.clone();
        let analysis_media = request.media.clone();
        let analysis_options = request.options;
        let analysis = thread::Builder::new()
            .name("deconflict-tui-analysis".into())
            .spawn(move || {
                let mut cache = FileCache::new(
                    &analysis_cache_dir,
                    &analysis_key,
                    analysis_options.no_cache || analysis_options.dry_run,
                );
                while let Ok(group) = group_rx.recv() {
                    if analysis_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let reused = {
                        let snapshots = analysis_snapshots
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        group
                            .conflicts
                            .iter()
                            .map(|(path, _)| snapshots.get(path).cloned())
                            .collect()
                    };
                    let cache = (!analysis_options.no_cache).then_some(&mut cache);
                    let result = analyze_scanned_group_with_snapshots(
                        &group,
                        reused,
                        cache,
                        analysis_options.refresh,
                        &analysis_media,
                    )
                    .map_err(|error| error.to_string());
                    if analysis_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let base = group.base.clone();
                    if analysis_tx
                        .send(Message::GroupAnalysisReady { base, result })
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = analysis_tx.send(Message::AnalysisFinished);
            })
            .map_err(|e| format!("cannot start TUI analysis worker: {e}"))?;

        let scan_tx = message_tx.clone();
        let group_event_tx = message_tx.clone();
        let scan_roots = request.roots.to_vec();
        let scan_config = request.scan_config.clone();
        let patterns = request.patterns.clone();
        let scan_status = Arc::clone(&status);
        let scan_cancel = Arc::clone(&cancel);
        let scan_snapshots = Arc::clone(&snapshots);
        let scanner = thread::Builder::new()
            .name("deconflict-tui-scanner".into())
            .spawn(move || {
                let mut dispatch_failed = false;
                let outcome = perform_scan_with_progress_cancel_events(
                    &scan_roots,
                    &scan_config,
                    &patterns,
                    ScanProgress::new(false, None).without_finish_message(),
                    &scan_cancel,
                    |event| {
                        scan_status
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .observe(event);
                        match event {
                            Event::Provisional(path, Ok(info)) => {
                                scan_snapshots
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .insert(path.clone(), info.clone());
                            }
                            Event::GroupReady(group) => {
                                let _ = group_event_tx.send(Message::GroupReady(group.clone()));
                                dispatch_failed |= group_tx.send(group.clone()).is_err();
                            }
                            _ => {}
                        }
                    },
                );
                drop(group_tx);
                if dispatch_failed {
                    let _ = scan_tx.send(Message::WorkerError(
                        "conflict analysis worker stopped before scan completed".into(),
                    ));
                }
                let _ = scan_tx.send(Message::ScanFinished {
                    result: outcome.result,
                    timings: outcome.timings,
                });
            });
        let scanner = match scanner {
            Ok(handle) => handle,
            Err(error) => {
                drop(message_tx);
                let _ = analysis.join();
                return Err(format!("cannot start TUI scanner worker: {error}"));
            }
        };
        drop(message_tx);

        Ok(Self {
            receiver,
            cancel,
            status,
            scanner: Some(scanner),
            analysis: Some(analysis),
            roots: request.roots.to_vec(),
            backup: request.backup.to_path_buf(),
            cache: estimate_cache,
            media: request.media.clone(),
            options: request.options,
            groups: Vec::new(),
            scan_result: None,
            timings: None,
            final_elapsed: None,
            final_summary: None,
            analysis_finished: false,
            estimated_file_count,
            worker_error: None,
            quit: false,
            force_quit: false,
        })
    }

    /// Drain pending messages without blocking the UI for longer than `timeout`.
    pub(super) fn poll(&mut self, timeout: Duration) {
        let first = match self.receiver.recv_timeout(timeout) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                if !(self.scan_complete() && self.analysis_finished) {
                    self.worker_error = Some("scan workers stopped unexpectedly".into());
                }
                None
            }
        };
        if let Some(message) = first {
            self.record(message);
        }
        while let Ok(message) = self.receiver.try_recv() {
            self.record(message);
        }
        self.finalize_timings();
    }

    fn finalize_timings(&mut self) {
        if self.final_elapsed.is_some() || !(self.scan_complete() && self.analysis_finished) {
            return;
        }
        if let Some(timings) = self.timings {
            self.final_elapsed = Some(timings.elapsed_since_start());
            self.final_summary = Some(ScanProgress::new(false, None).timing_breakdown(&timings));
        }
    }

    fn record(&mut self, message: Message) {
        match message {
            Message::GroupReady(group) => {
                if !self
                    .groups
                    .iter()
                    .any(|entry| entry.group.base == group.base)
                {
                    self.groups.push(GroupEntry::from(group));
                }
            }
            Message::GroupAnalysisReady { base, result } => {
                if let Some(entry) = self
                    .groups
                    .iter_mut()
                    .find(|entry| entry.group.base == base)
                {
                    entry.analysis = Some(result);
                }
            }
            Message::ScanFinished { result, timings } => {
                self.timings = timings;
                let mut previous: HashMap<_, _> = self
                    .groups
                    .drain(..)
                    .map(|entry| (entry.group.base.clone(), entry))
                    .collect();
                if let Ok(scan) = &result {
                    let mut status = self
                        .status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    status.files = scan.file_count;
                    status.directories = scan.directory_count;
                    status.copies_queued = scan.analysis_count;
                    self.groups = scan
                        .groups
                        .iter()
                        .map(|group| {
                            previous.remove(&group.base).unwrap_or_else(|| {
                                GroupEntry::from(Group {
                                    base: group.base.clone(),
                                    conflicts: group.conflicts.clone(),
                                })
                            })
                        })
                        .collect();
                }
                self.scan_result = Some(result);
            }
            Message::AnalysisFinished => self.analysis_finished = true,
            Message::WorkerError(error) => self.worker_error = Some(error),
        }
    }

    pub(super) fn status(&self) -> ScanStatus {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(super) fn scan_complete(&self) -> bool {
        self.scan_result.is_some()
    }

    pub(super) fn analysis_complete(&self) -> bool {
        self.analysis_finished
    }

    pub(super) fn timing_summary(&self) -> Option<String> {
        self.final_summary.clone().or_else(|| {
            self.timings
                .map(|timings| ScanProgress::new(false, None).summary(&timings))
        })
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.final_elapsed
            .or_else(|| self.status().started.map(|started| started.elapsed()))
            .unwrap_or_default()
    }

    pub(super) fn group_analysis_counts(&self) -> (usize, usize) {
        analyzed_group_counts(&self.groups)
    }

    pub(super) fn force_quit_requested(&self) -> bool {
        self.force_quit
    }

    pub(super) fn estimated_file_count(&self) -> Option<u64> {
        self.estimated_file_count
    }

    pub(super) fn worker_error(&self) -> Option<&str> {
        self.worker_error.as_deref()
    }

    pub(super) fn options_dry_run(&self) -> bool {
        self.options.dry_run
    }

    pub(super) fn record_decision(&mut self, base: &Path, decision: Decision) {
        if let Some(entry) = self
            .groups
            .iter_mut()
            .find(|entry| entry.group.base == base)
        {
            entry.decision = Some(decision);
            entry.skipped = false;
        }
    }

    pub(super) fn record_skip(&mut self, base: &Path) {
        if let Some(entry) = self
            .groups
            .iter_mut()
            .find(|entry| entry.group.base == base)
        {
            entry.decision = None;
            entry.skipped = true;
        }
    }

    pub(super) fn quit(&mut self) {
        self.quit = true;
    }

    pub(super) fn force_quit(&mut self) {
        self.quit = true;
        self.force_quit = true;
        for entry in &mut self.groups {
            entry.decision = None;
            entry.skipped = false;
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(super) fn refresh_analysis(&mut self, base: &Path) -> Result<(), String> {
        let Some(entry) = self.groups.iter().find(|entry| entry.group.base == base) else {
            return Ok(());
        };
        let analysis = analyze_scanned_group_with_media(&entry.group, &self.media)
            .map_err(|error| format!("analysis failed after tool action: {error}"))?;
        if let Some(entry) = self
            .groups
            .iter_mut()
            .find(|entry| entry.group.base == base)
        {
            entry.analysis = Some(Ok(analysis));
        }
        Ok(())
    }

    /// Wait for outstanding work, then apply buffered decisions through the safe resolver.
    pub(super) fn finish(mut self) -> Result<Summary, String> {
        if self.force_quit {
            self.cancel.store(true, Ordering::Relaxed);
        } else {
            while !(self.scan_complete() && self.analysis_complete()) && self.worker_error.is_none()
            {
                self.poll(Duration::from_millis(50));
            }
        }
        if let Some(scanner) = self.scanner.take() {
            let _ = scanner.join();
        }
        if let Some(analysis) = self.analysis.take() {
            let _ = analysis.join();
        }
        if self.force_quit {
            return Ok(Summary {
                lines: vec!["force quit; scan cancelled and no choices were applied".into()],
            });
        }
        if let Some(error) = self.worker_error.take() {
            return Err(error);
        }
        let scan = self
            .scan_result
            .take()
            .ok_or_else(|| "scanner stopped without returning a result".to_string())?
            .map_err(|error| format!("scan failed: {error}"))?;
        ensure_scan_complete(&scan)?;
        if !self.analysis_complete() {
            return Err("conflict analysis did not finish".into());
        }
        for entry in &self.groups {
            match &entry.analysis {
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    return Err(format!("analysis failed for {}: {error}", entry.label));
                }
                None => return Err(format!("analysis missing for {}", entry.label)),
            }
        }

        self.save_estimate(&scan)?;
        if scan.groups.is_empty() {
            return Ok(Summary {
                lines: vec!["no conflicts to resolve".into()],
            });
        }

        let mut operations = Vec::new();
        let mut resolved = 0usize;
        for (index, group) in scan.groups.iter().enumerate() {
            let Some(entry) = self
                .groups
                .iter()
                .find(|entry| entry.group.base == group.base)
            else {
                continue;
            };
            let Some(decision) = entry.decision else {
                continue;
            };
            let analysis = match entry
                .analysis
                .as_ref()
                .and_then(|result| result.as_ref().ok())
            {
                Some(analysis) => &analysis.analysis,
                None => continue,
            };
            let prepared = prepare(&scan, index, analysis, &self.roots)
                .map_err(|e| format!("cannot prepare {}: {e}", group.base.display()))?;
            let outcome = prepared
                .resolve(decision, &self.backup, self.options.dry_run)
                .map_err(|e| format!("resolution failed for {}: {e}", group.base.display()))?;
            resolved += 1;
            for (source, destination) in outcome.operations {
                operations.push(format!(
                    "{}: {} -> {}",
                    if self.options.dry_run {
                        "would move"
                    } else {
                        "moved"
                    },
                    source.display(),
                    destination.display()
                ));
            }
        }
        let skipped = self.groups.iter().filter(|entry| entry.skipped).count();
        if !self.options.no_cache && !self.options.dry_run {
            self.cache
                .clear()
                .map_err(|e| format!("cannot invalidate cache: {e}"))?;
        }
        let mut lines = operations;
        lines.push(if self.options.dry_run {
            format!(
                "dry-run complete: {} resolved, {skipped} skipped",
                conflict_count(resolved)
            )
        } else {
            format!("resolved {}, {skipped} skipped", conflict_count(resolved))
        });
        if self.quit {
            lines
                .push("quit requested; recorded choices were applied after a complete scan".into());
        }
        Ok(Summary { lines })
    }

    fn save_estimate(&mut self, scan: &ScanResult) -> Result<(), String> {
        if self.options.no_cache || self.options.dry_run {
            return Ok(());
        }
        if self.options.refresh {
            self.cache
                .clear()
                .map_err(|e| format!("cannot clear cache: {e}"))?;
        }
        self.cache
            .save_file_count(scan.file_count as u64)
            .map_err(|e| format!("cannot save scan estimate: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn event_status(events: &[Event]) -> ScanStatus {
        let mut status = ScanStatus::default();
        for event in events {
            status.observe(event);
        }
        status
    }

    #[test]
    fn status_tracks_events_without_authorizing_resolution() {
        let path = PathBuf::from("copy.txt");
        let status = event_status(&[
            Event::File(path.clone()),
            Event::AnalysisQueued(path.clone()),
            Event::Provisional(
                path.clone(),
                Ok(FileInfo {
                    path: path.clone(),
                    size: 1,
                    modified: SystemTime::UNIX_EPOCH,
                    sha256: String::new(),
                    is_text: true,
                }),
            ),
        ]);
        assert_eq!(status.files, 1);
        assert_eq!(status.copies_queued, 1);
        assert_eq!(status.copies_done, 1);
        assert_eq!(status.current_path, Some(path));
    }

    #[test]
    fn status_keeps_scan_error_messages() {
        let path = PathBuf::from("broken");
        let status = event_status(&[Event::Error(path.clone(), "cannot read".into())]);
        assert_eq!(status.errors, 1);
        assert_eq!(
            status.error_messages,
            vec![(path, "cannot read".to_string())]
        );
    }

    #[test]
    fn analysis_counter_uses_rich_group_results() {
        let groups = vec![
            GroupEntry::from(Group {
                base: PathBuf::from("a"),
                conflicts: Vec::new(),
            }),
            GroupEntry::from(Group {
                base: PathBuf::from("b"),
                conflicts: Vec::new(),
            }),
        ];
        assert_eq!(analyzed_group_counts(&groups), (0, 2));
        let mut groups = groups;
        groups[0].analysis = Some(Err("analysis failed".into()));
        assert_eq!(analyzed_group_counts(&groups), (1, 2));
    }

    #[test]
    fn group_entry_tracks_decisions() {
        let mut entry = GroupEntry::from(Group {
            base: PathBuf::from("base.txt"),
            conflicts: Vec::new(),
        });
        assert!(!entry.is_decided());
        entry.decision = Some(Decision::KeepBase);
        assert!(entry.is_decided());
    }
}
