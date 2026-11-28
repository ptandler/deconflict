use console::{measure_text_width, Term};
use deconflict_core::{
    config::ScanConfig,
    patterns::Patterns,
    scan::{scan, Event, Group, ScanResult},
};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct ScanProgress {
    bar: Option<ProgressBar>,
    show_errors: bool,
    estimated_file_count: Option<u64>,
    finish_message: bool,
    clock: PauseClock,
}

impl ScanProgress {
    pub(crate) fn new(show_progress: bool, estimated_file_count: Option<u64>) -> Self {
        let bar = if show_progress && io::stderr().is_terminal() {
            let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(4));
            if let Some(estimate) = estimated_file_count.filter(|count| *count > 0) {
                bar.set_length(estimate);
            }
            bar.set_style(ProgressStyle::with_template("{msg}").expect("valid progress template"));
            Some(bar)
        } else {
            None
        };
        Self {
            bar,
            show_errors: show_progress,
            estimated_file_count,
            finish_message: true,
            clock: PauseClock::default(),
        }
    }

    pub(crate) fn without_finish_message(mut self) -> Self {
        self.finish_message = false;
        self
    }

    /// Keep stderr error reports even when the progress bar itself is disabled.
    pub(crate) fn with_error_reporting(mut self) -> Self {
        self.show_errors = true;
        self
    }

    pub(crate) fn summary(&self, timings: &ScanTimings) -> String {
        let total = timings
            .started
            .elapsed()
            .saturating_sub(self.clock.total_paused());
        let discovery = timings
            .discovery_last
            .map_or_else(Duration::default, |last| {
                last.duration_since(timings.started)
                    .saturating_sub(self.clock.paused_before(last))
            });
        let analysis = timings
            .analysis_started
            .zip(timings.analysis_last)
            .map(|(first, last)| {
                last.duration_since(first)
                    .saturating_sub(self.clock.paused_between(first, last))
            });
        let analysis_time = analysis
            .map(|duration| format!("{}s", format_duration(duration)))
            .unwrap_or_else(|| "n/a".to_string());
        format!(
            "scan complete: {} files, {} dirs, {} conflicts; discovery {}s, analysis {analysis_time} (overlapping windows; total {}s)",
            timings.files,
            timings.directories,
            timings.copies,
            format_duration(discovery),
            format_duration(total)
        )
    }

    /// Compact phase timings for the TUI status row.
    pub(crate) fn timing_breakdown(&self, timings: &ScanTimings) -> String {
        let total = timings
            .started
            .elapsed()
            .saturating_sub(self.clock.total_paused());
        let discovery = timings
            .discovery_last
            .map_or_else(Duration::default, |last| {
                last.duration_since(timings.started)
                    .saturating_sub(self.clock.paused_before(last))
            });
        let analysis = timings
            .analysis_started
            .zip(timings.analysis_last)
            .map(|(first, last)| {
                last.duration_since(first)
                    .saturating_sub(self.clock.paused_between(first, last))
            });
        let analysis_time = analysis
            .map(|duration| format!("{}s", format_duration(duration)))
            .unwrap_or_else(|| "n/a".to_string());
        let covered = discovery.max(analysis.unwrap_or_default());
        let waiting = total.saturating_sub(covered);
        format!(
            "discovery {}s; analysis {analysis_time}; waiting {}s; total {}s",
            format_duration(discovery),
            format_duration(waiting),
            format_duration(total)
        )
    }

    /// Refresh an already-visible CLI-style progress bar from external state.
    pub(crate) fn update_display(
        &self,
        files: usize,
        directories: usize,
        analyses_done: usize,
        analyses_queued: usize,
        path: Option<&std::path::Path>,
        elapsed: Duration,
    ) {
        if let Some(bar) = &self.bar {
            let (_, columns) = Term::stderr().size_checked().unwrap_or((24, 120));
            let status = live_progress_line(
                files,
                directories,
                analyses_done,
                analyses_queued,
                self.estimated_file_count,
                columns as usize,
                elapsed,
                "conflicts",
            );
            let path = path
                .map(|path| middle_ellipsis(&path.display().to_string(), columns as usize))
                .unwrap_or_default();
            bar.set_message(format!("{status}\n{path}"));
        }
    }

    pub(crate) fn finish_display(&self, message: &str) {
        if let Some(bar) = &self.bar {
            if self.finish_message {
                bar.finish_with_message(message.to_string());
            } else {
                bar.finish_and_clear();
            }
        }
    }

    pub(crate) fn suspend<R>(&self, action: impl FnOnce() -> R) -> R {
        if let Some(bar) = &self.bar {
            let elapsed = bar.elapsed();
            let result = self.clock.pause(|| bar.suspend(action));
            bar.set_elapsed(elapsed);
            result
        } else {
            self.clock.pause(action)
        }
    }
}

#[derive(Clone, Default)]
struct PauseClock {
    state: Arc<Mutex<PauseState>>,
}

#[derive(Default)]
struct PauseState {
    total: Duration,
    current: Option<Instant>,
}

impl PauseClock {
    fn pause<R>(&self, action: impl FnOnce() -> R) -> R {
        if let Ok(mut state) = self.state.lock() {
            state.current = Some(Instant::now());
        }
        let result = action();
        if let Ok(mut state) = self.state.lock() {
            if let Some(started) = state.current.take() {
                state.total += started.elapsed();
            }
        }
        result
    }

    fn paused_before(&self, at: Instant) -> Duration {
        let Ok(state) = self.state.lock() else {
            return Duration::ZERO;
        };
        let current = state.current.map_or(Duration::ZERO, |started| {
            if started < at {
                at.saturating_duration_since(started)
            } else {
                Duration::ZERO
            }
        });
        state.total + current
    }

    fn paused_between(&self, start: Instant, end: Instant) -> Duration {
        self.paused_before(end)
            .saturating_sub(self.paused_before(start))
    }

    fn total_paused(&self) -> Duration {
        self.paused_before(Instant::now())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ScanTimings {
    started: Instant,
    discovery_last: Option<Instant>,
    analysis_started: Option<Instant>,
    analysis_last: Option<Instant>,
    files: usize,
    directories: usize,
    copies: usize,
}

impl ScanTimings {
    pub(crate) fn elapsed_since_start(&self) -> Duration {
        self.started.elapsed()
    }
}

pub(crate) struct ScanOutcome {
    pub(crate) result: Result<ScanResult, String>,
    pub(crate) timings: Option<ScanTimings>,
}

pub(crate) fn perform_scan(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    show_progress: bool,
    estimated_file_count: Option<u64>,
) -> Result<ScanResult, String> {
    perform_scan_with_groups(
        roots,
        scan_config,
        patterns,
        show_progress,
        estimated_file_count,
        |_| {},
    )
    .result
}

pub(crate) fn perform_scan_with_groups(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    show_progress: bool,
    estimated_file_count: Option<u64>,
    mut group_ready: impl FnMut(&Group),
) -> ScanOutcome {
    perform_scan_with_progress(
        roots,
        scan_config,
        patterns,
        ScanProgress::new(show_progress, estimated_file_count).with_error_reporting(),
        &mut group_ready,
    )
}

pub(crate) fn perform_scan_with_progress(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    progress: ScanProgress,
    group_ready: impl FnMut(&Group),
) -> ScanOutcome {
    let cancel = AtomicBool::new(false);
    perform_scan_with_progress_cancel(roots, scan_config, patterns, progress, &cancel, group_ready)
}

pub(crate) fn perform_scan_with_progress_cancel(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    progress: ScanProgress,
    cancel: &AtomicBool,
    mut group_ready: impl FnMut(&Group),
) -> ScanOutcome {
    perform_scan_with_progress_cancel_events(
        roots,
        scan_config,
        patterns,
        progress,
        cancel,
        |event| {
            if let Event::GroupReady(group) = event {
                group_ready(group);
            }
        },
    )
}

pub(crate) fn perform_scan_with_progress_cancel_events(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    progress: ScanProgress,
    cancel: &AtomicBool,
    mut observer: impl FnMut(&Event),
) -> ScanOutcome {
    if roots.is_empty() {
        return ScanOutcome {
            result: Err("no directories. Pass dirs or set [scan].default_dirs".into()),
            timings: None,
        };
    }
    let mut file_count = 0usize;
    let mut directory_count = 0usize;
    let mut analysis_queued = 0usize;
    let mut analysis_done = 0usize;
    let started = Instant::now();
    let mut discovery_last_event = None;
    let mut analysis_started = None;
    let mut analysis_last_event = None;
    let scan_result = scan(roots, scan_config, patterns, cancel, |event| {
        match event {
            Event::File(path) => {
                file_count += 1;
                discovery_last_event = Some(Instant::now());
                if let Some(bar) = &progress.bar {
                    bar.set_position(file_count as u64);
                    set_progress_message(
                        bar,
                        file_count,
                        directory_count,
                        analysis_done,
                        analysis_queued,
                        progress.estimated_file_count,
                        path,
                    );
                }
            }
            Event::Directory(path) => {
                directory_count += 1;
                discovery_last_event = Some(Instant::now());
                if let Some(bar) = &progress.bar {
                    set_progress_message(
                        bar,
                        file_count,
                        directory_count,
                        analysis_done,
                        analysis_queued,
                        progress.estimated_file_count,
                        path,
                    );
                }
            }
            Event::AnalysisQueued(path) => {
                analysis_queued += 1;
                analysis_started.get_or_insert_with(Instant::now);
                if let Some(bar) = &progress.bar {
                    set_progress_message(
                        bar,
                        file_count,
                        directory_count,
                        analysis_done,
                        analysis_queued,
                        progress.estimated_file_count,
                        path,
                    );
                }
            }
            Event::Provisional(path, _) => {
                analysis_done += 1;
                analysis_last_event = Some(Instant::now());
                if let Some(bar) = &progress.bar {
                    set_progress_message(
                        bar,
                        file_count,
                        directory_count,
                        analysis_done,
                        analysis_queued,
                        progress.estimated_file_count,
                        path,
                    );
                }
            }
            Event::Error(path, error) => {
                if progress.show_errors {
                    if let Some(bar) = &progress.bar {
                        bar.suspend(|| eprintln!("{}: {error}", path.display()));
                    } else {
                        eprintln!("{}: {error}", path.display());
                    }
                }
            }
            Event::DirectoryComplete(_) => (),
            Event::GroupReady(_) => (),
        }
        observer(event);
    });

    let timings = scan_result.as_ref().ok().map(|result| ScanTimings {
        started,
        discovery_last: discovery_last_event,
        analysis_started,
        analysis_last: analysis_last_event,
        files: result.file_count,
        directories: result.directory_count,
        copies: analysis_done,
    });
    let summary = timings
        .as_ref()
        .map_or_else(String::new, |timings| progress.summary(timings));
    if let Some(bar) = &progress.bar {
        if progress.finish_message {
            bar.finish_with_message(summary.clone());
        } else {
            bar.finish_and_clear();
        }
    }
    ScanOutcome {
        result: scan_result,
        timings,
    }
}

fn set_progress_message(
    bar: &ProgressBar,
    files: usize,
    directories: usize,
    analyses_done: usize,
    analyses_queued: usize,
    estimated_file_count: Option<u64>,
    path: &std::path::Path,
) {
    let (_, columns) = Term::stderr().size_checked().unwrap_or((24, 120));
    let status = live_progress_line(
        files,
        directories,
        analyses_done,
        analyses_queued,
        estimated_file_count,
        columns as usize,
        bar.elapsed(),
        "conflicts",
    );
    bar.set_message(format!(
        "{status}\n{}",
        middle_ellipsis(&path.display().to_string(), columns as usize)
    ));
}

/// Shared live counter line, used by the CLI progress bar and the TUI status row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn live_progress_line(
    files: usize,
    directories: usize,
    analyses_done: usize,
    analyses_queued: usize,
    estimated_file_count: Option<u64>,
    columns: usize,
    elapsed: Duration,
    analysis_unit: &str,
) -> String {
    let timing = precise_duration(elapsed);
    progress_line_with_timing(
        files,
        directories,
        analyses_done,
        analyses_queued,
        estimated_file_count,
        columns,
        &timing,
        analysis_unit,
    )
}

/// Final counter line: keeps the elapsed time but drops the ETA.
#[allow(clippy::too_many_arguments)]
pub(crate) fn final_progress_line(
    files: usize,
    directories: usize,
    analyses_done: usize,
    analyses_queued: usize,
    estimated_file_count: Option<u64>,
    columns: usize,
    elapsed: Duration,
    analysis_unit: &str,
) -> String {
    progress_line_with_timing(
        files,
        directories,
        analyses_done,
        analyses_queued,
        estimated_file_count,
        columns,
        &precise_duration(elapsed),
        analysis_unit,
    )
}

#[allow(clippy::too_many_arguments)]
fn progress_line_with_timing(
    files: usize,
    directories: usize,
    analyses_done: usize,
    analyses_queued: usize,
    estimated_file_count: Option<u64>,
    columns: usize,
    timing: &str,
    analysis_unit: &str,
) -> String {
    let status = status_text(
        files,
        directories,
        analyses_done,
        analyses_queued,
        estimated_file_count,
        columns,
        analysis_unit,
    );
    format!("{timing} | {status}")
}

fn precise_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

pub(crate) fn conflict_count(count: usize) -> String {
    format!("{count} conflict{}", if count == 1 { "" } else { "s" })
}

pub(crate) fn status_text(
    files: usize,
    directories: usize,
    analyses_done: usize,
    analyses_queued: usize,
    estimated_file_count: Option<u64>,
    columns: usize,
    analysis_unit: &str,
) -> String {
    let estimated = estimated_file_count.filter(|count| *count > 0);
    if columns >= 100 {
        let files = match estimated {
            Some(estimate) => format!("{files} files (est. {estimate})"),
            None => format!("{files} files"),
        };
        format!(
            "discovery {files}; dirs {directories}; analysis {analyses_done}/{analyses_queued} {analysis_unit}"
        )
    } else {
        let files = estimated.map_or_else(
            || files.to_string(),
            |estimate| format!("{files}/{estimate}"),
        );
        format!(
            "discovery {files} files; dirs {directories}; analysis {analyses_done}/{analyses_queued}"
        )
    }
}

fn format_duration(duration: Duration) -> String {
    format!("{:.1}", duration.as_secs_f64())
}

fn middle_ellipsis(path: &str, max_width: usize) -> String {
    if measure_text_width(path) <= max_width {
        return path.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let chars: Vec<char> = path.chars().collect();
    let head_count = (max_width - 3) / 2;
    let tail_count = max_width - 3 - head_count;
    let tail_start = chars.len().saturating_sub(tail_count);
    let mut out: String = chars[..head_count.min(chars.len())].iter().collect();
    out.push_str("...");
    out.extend(chars[tail_start.min(chars.len())..].iter());
    out
}

#[cfg(test)]
mod tests {
    use super::{
        final_progress_line, live_progress_line, middle_ellipsis, set_progress_message,
        status_text, ScanProgress, ScanTimings,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn middle_ellipsis_keeps_head_and_tail() {
        assert_eq!(
            middle_ellipsis("/tmp/long-directory/important-name.txt", 20),
            "/tmp/lon...-name.txt"
        );
    }

    #[test]
    fn middle_ellipsis_handles_narrow_terminals() {
        assert_eq!(middle_ellipsis("Readme.md", 2), "..");
    }

    #[test]
    fn progress_labels_distinguish_discovery_from_analysis() {
        let progress = indicatif::ProgressBar::hidden();
        set_progress_message(
            &progress,
            5525,
            862,
            43,
            45,
            Some(5525),
            std::path::Path::new("/scan/file"),
        );
        let message = progress.message();
        let mut lines = message.lines().map(str::to_string);
        let status = lines.next().unwrap();
        assert!(status.contains("discovery 5525 files (est. 5525)"));
        assert!(status.contains("dirs 862"));
        assert!(status.contains("analysis 43/45 conflicts"));
        assert_eq!(lines.next().as_deref(), Some("/scan/file"));
        assert!(lines.next().is_none());
    }

    #[test]
    fn live_progress_line_shares_timing_and_counters() {
        let line = live_progress_line(
            3,
            2,
            1,
            4,
            Some(10),
            120,
            Duration::from_secs(3),
            "conflicts",
        );
        assert_eq!(
            line,
            "00:00:03 | discovery 3 files (est. 10); dirs 2; analysis 1/4 conflicts"
        );

        let unknown =
            live_progress_line(0, 0, 0, 0, None, 120, Duration::from_secs(1), "conflicts");
        assert!(unknown.starts_with("00:00:01 | discovery 0 files"));
    }

    #[test]
    fn final_progress_line_keeps_elapsed_time_without_eta() {
        let line = final_progress_line(
            4,
            1,
            4,
            4,
            Some(4),
            120,
            Duration::from_secs(2),
            "conflicts",
        );
        assert_eq!(
            line,
            "00:00:02 | discovery 4 files (est. 4); dirs 1; analysis 4/4 conflicts"
        );
    }

    #[test]
    fn progress_status_compacts_for_narrow_terminals() {
        assert_eq!(
            status_text(5525, 862, 43, 45, Some(5525), 80, "conflicts"),
            "discovery 5525/5525 files; dirs 862; analysis 43/45"
        );
    }

    #[test]
    fn timing_breakdown_reports_discovery_and_analysis() {
        let progress = ScanProgress::new(false, None);
        let now = Instant::now();
        let timings = ScanTimings {
            started: now - Duration::from_secs(10),
            discovery_last: Some(now - Duration::from_secs(7)),
            analysis_started: Some(now - Duration::from_secs(5)),
            analysis_last: Some(now - Duration::from_secs(2)),
            files: 2,
            directories: 1,
            copies: 1,
        };
        let line = progress.timing_breakdown(&timings);
        assert!(line.contains("discovery 3.0s"), "{line}");
        assert!(line.contains("analysis 3.0s"), "{line}");
        assert!(line.contains("waiting 7.0s"), "{line}");
        assert!(line.contains("total 10.0s"), "{line}");
    }

    #[test]
    fn summary_excludes_time_spent_waiting_for_user_input() {
        let progress = ScanProgress::new(false, None);
        let timings = ScanTimings {
            started: Instant::now() - Duration::from_secs(10),
            discovery_last: None,
            analysis_started: None,
            analysis_last: None,
            files: 2,
            directories: 1,
            copies: 1,
        };
        progress
            .clock
            .pause(|| std::thread::sleep(Duration::from_millis(120)));
        let summary = progress.summary(&timings);
        assert!(summary.contains("total 10.0s"), "{summary}");
    }
}
