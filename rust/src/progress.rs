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
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct ScanProgress {
    bar: Option<ProgressBar>,
    show_errors: bool,
    estimated_file_count: Option<u64>,
}

impl ScanProgress {
    pub(crate) fn new(show_progress: bool, estimated_file_count: Option<u64>) -> Self {
        let bar = if show_progress && io::stderr().is_terminal() {
            let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(4));
            if let Some(estimate) = estimated_file_count.filter(|count| *count > 0) {
                bar.set_length(estimate);
            }
            bar.set_style(
                ProgressStyle::with_template("{elapsed_precise} ETA {eta_precise} | {msg}")
                    .expect("valid progress template"),
            );
            Some(bar)
        } else {
            None
        };
        Self {
            bar,
            show_errors: show_progress,
            estimated_file_count,
        }
    }

    pub(crate) fn suspend<R>(&self, action: impl FnOnce() -> R) -> R {
        match &self.bar {
            Some(bar) => bar.suspend(action),
            None => action(),
        }
    }
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
}

pub(crate) fn perform_scan_with_groups(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    show_progress: bool,
    estimated_file_count: Option<u64>,
    mut group_ready: impl FnMut(&Group),
) -> Result<ScanResult, String> {
    perform_scan_with_progress(
        roots,
        scan_config,
        patterns,
        ScanProgress::new(show_progress, estimated_file_count),
        &mut group_ready,
    )
}

pub(crate) fn perform_scan_with_progress(
    roots: &[PathBuf],
    scan_config: &ScanConfig,
    patterns: &Patterns,
    progress: ScanProgress,
    group_ready: impl FnMut(&Group),
) -> Result<ScanResult, String> {
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
) -> Result<ScanResult, String> {
    if roots.is_empty() {
        return Err("no directories. Pass dirs or set [scan].default_dirs".into());
    }
    let mut file_count = 0usize;
    let mut directory_count = 0usize;
    let mut analysis_queued = 0usize;
    let mut analysis_done = 0usize;
    let started = Instant::now();
    let mut discovery_last_event = None;
    let mut analysis_started = None;
    let mut analysis_last_event = None;
    let result = scan(roots, scan_config, patterns, cancel, |event| match event {
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
            if let Some(bar) = &progress.bar {
                bar.suspend(|| eprintln!("{}: {error}", path.display()));
            } else if progress.show_errors {
                eprintln!("{}: {error}", path.display());
            }
        }
        Event::DirectoryComplete(_) => (),
        Event::GroupReady(group) => group_ready(group),
    })?;

    if let Some(bar) = &progress.bar {
        let total = started.elapsed();
        let discovery = discovery_last_event
            .map(|last| last.duration_since(started))
            .unwrap_or_default();
        let analysis = analysis_started
            .zip(analysis_last_event)
            .map(|(first, last)| last.duration_since(first));
        let analysis_time = analysis
            .map(|duration| format!("{}s", format_duration(duration)))
            .unwrap_or_else(|| "n/a".to_string());
        bar.finish_with_message(format!(
            "scan complete: {} files, {} dirs, {analysis_done} copies; discovery {}s, analysis {analysis_time} (overlapping windows; total {}s)",
            result.file_count,
            result.directory_count,
            format_duration(discovery),
            format_duration(total)
        ));
    }
    Ok(result)
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
    let status = status_text(
        files,
        directories,
        analyses_done,
        analyses_queued,
        estimated_file_count,
        columns as usize,
    );
    bar.set_message(format!(
        "{status}\n{}",
        middle_ellipsis(&path.display().to_string(), columns as usize)
    ));
}

fn status_text(
    files: usize,
    directories: usize,
    analyses_done: usize,
    analyses_queued: usize,
    estimated_file_count: Option<u64>,
    columns: usize,
) -> String {
    let estimated = estimated_file_count.filter(|count| *count > 0);
    if columns >= 100 {
        let files = match estimated {
            Some(estimate) => format!("{files} files (est. {estimate})"),
            None => format!("{files} files"),
        };
        format!(
            "scan {files}; dirs {directories}; analysis {analyses_done}/{analyses_queued} copies"
        )
    } else {
        let files = estimated.map_or_else(
            || files.to_string(),
            |estimate| format!("{files}/{estimate}"),
        );
        format!(
            "scan {files} files; dirs {directories}; conflicts {analyses_done}/{analyses_queued}"
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
    use super::{middle_ellipsis, set_progress_message, status_text};

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
        assert!(status.contains("scan 5525 files (est. 5525)"));
        assert!(status.contains("dirs 862"));
        assert!(status.contains("analysis 43/45 copies"));
        assert_eq!(lines.next().as_deref(), Some("/scan/file"));
        assert!(lines.next().is_none());
    }

    #[test]
    fn progress_status_compacts_for_narrow_terminals() {
        assert_eq!(
            status_text(5525, 862, 43, 45, Some(5525), 80),
            "scan 5525/5525 files; dirs 862; conflicts 43/45"
        );
    }
}
