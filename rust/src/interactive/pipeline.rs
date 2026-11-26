use super::prompt::{prompt_choice, PromptChoice};
use crate::{
    cli::Cli,
    cli_config::{effective_scan, general_backup, general_cache, media_tools},
    progress::{perform_scan_with_progress, ScanProgress},
    report::group_report,
};
use deconflict_core::{
    analysis::{
        analyze_scanned_group_cached, analyze_scanned_group_with_media, StampedGroupAnalysis,
    },
    cache::{cache_key, FileCache},
    config::ScanConfig,
    media::Tools,
    patterns::Patterns,
    resolve::{prepare, Decision},
    scan::{Group, ScanResult},
};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

pub(crate) fn run_bare_cli(cli: &Cli) -> Result<(), String> {
    let (scan_config, patterns, roots, config) =
        effective_scan(cli.config.as_deref(), cli, &cli.dirs, None)?;
    let show_progress = std::io::stdout().is_terminal();
    run_interactive_scan(InteractiveScanRequest {
        roots: &roots,
        scan_config: &scan_config,
        patterns: &patterns,
        backup: &general_backup(&config),
        cache_dir: &general_cache(&config),
        media: &media_tools(&config),
        options: InteractiveOptions::default(),
        show_progress,
        no_conflicts_message: "no conflicts found",
    })
}

pub(crate) struct InteractiveScanRequest<'a> {
    pub(crate) roots: &'a [PathBuf],
    pub(crate) scan_config: &'a ScanConfig,
    pub(crate) patterns: &'a Patterns,
    pub(crate) backup: &'a Path,
    pub(crate) cache_dir: &'a Path,
    pub(crate) media: &'a Tools,
    pub(crate) options: InteractiveOptions,
    pub(crate) show_progress: bool,
    pub(crate) no_conflicts_message: &'a str,
}

pub(crate) fn run_interactive_scan(request: InteractiveScanRequest<'_>) -> Result<(), String> {
    let InteractiveScanRequest {
        roots,
        scan_config,
        patterns,
        backup,
        cache_dir,
        media,
        options,
        show_progress,
        no_conflicts_message,
    } = request;
    let key = cache_key(roots).map_err(|e| format!("cannot key cache: {e}"))?;
    let mut estimate_cache = FileCache::new(cache_dir, &key, options.no_cache || options.dry_run);
    let estimated_file_count = if options.no_cache || options.dry_run {
        None
    } else {
        estimate_cache.file_count()
    };
    let (group_tx, group_rx) = mpsc::channel::<Group>();
    let (message_tx, message_rx) = mpsc::channel();
    let scan_progress = ScanProgress::new(show_progress, estimated_file_count);
    let prompt_progress = scan_progress.clone();

    let analysis_tx = message_tx.clone();
    let analysis_dir = cache_dir.to_path_buf();
    let analysis_key = key.clone();
    let analysis_media = media.clone();
    let analysis_options = options;
    let analysis_thread = thread::Builder::new()
        .name("deconflict-group-analysis".into())
        .spawn(move || {
            let mut cache = FileCache::new(
                analysis_dir,
                &analysis_key,
                analysis_options.no_cache || analysis_options.dry_run,
            );
            while let Ok(group) = group_rx.recv() {
                let result = if analysis_options.no_cache {
                    analyze_scanned_group_with_media(&group, &analysis_media)
                } else {
                    analyze_scanned_group_cached(
                        &group,
                        &mut cache,
                        analysis_options.refresh,
                        &analysis_media,
                    )
                }
                .map_err(|error| error.to_string());
                if analysis_tx
                    .send(PipelineMessage::GroupAnalysisReady { group, result })
                    .is_err()
                {
                    return;
                }
            }
            let _ = analysis_tx.send(PipelineMessage::AnalysisFinished);
        })
        .map_err(|error| format!("cannot start group analysis worker: {error}"))?;

    let scan_tx = message_tx.clone();
    let scan_roots = roots.to_vec();
    let scan_config = scan_config.clone();
    let patterns = patterns.clone();
    let scanner_progress = scan_progress;
    let scanner_thread = match thread::Builder::new()
        .name("deconflict-scanner".into())
        .spawn(move || {
            let mut dispatch_failed = false;
            let result = perform_scan_with_progress(
                &scan_roots,
                &scan_config,
                &patterns,
                scanner_progress,
                |group| {
                    dispatch_failed |= group_tx.send(group.clone()).is_err();
                },
            );
            drop(group_tx);
            if dispatch_failed {
                let _ = scan_tx.send(PipelineMessage::WorkerError(
                    "group analysis worker stopped before scan completed".into(),
                ));
            }
            let _ = scan_tx.send(PipelineMessage::ScanFinished(result));
        }) {
        Ok(handle) => handle,
        Err(error) => {
            drop(message_tx);
            let _ = analysis_thread.join();
            return Err(format!("cannot start scanner worker: {error}"));
        }
    };
    drop(message_tx);

    let mut state = InteractiveState::default();
    collect_interactive_results(&message_rx, &mut state, options, &prompt_progress);
    let scanner_join = scanner_thread.join();
    let analysis_join = analysis_thread.join();
    scanner_join.map_err(|_| "scanner worker panicked".to_string())?;
    analysis_join.map_err(|_| "group analysis worker panicked".to_string())?;

    if let Some(error) = state.worker_error.take() {
        return Err(error);
    }
    let scan_result = state
        .scan_result
        .take()
        .ok_or_else(|| "scanner stopped without returning a result".to_string())?
        .map_err(|error| format!("scan failed: {error}"))?;
    ensure_scan_complete(&scan_result)?;
    if let Some(error) = state.interaction_error.take() {
        return Err(error);
    }
    if let Some(error) = state.analysis_error.take() {
        return Err(error);
    }
    ensure_group_analyses_complete(&scan_result, &state)?;
    if state.quit {
        println!(
            "aborted after {} of {} group(s); recorded choices are applied only after a complete scan",
            state.completed,
            scan_result.groups.len()
        );
    }
    if scan_result.groups.is_empty() {
        if !options.no_cache && !options.dry_run {
            if options.refresh {
                estimate_cache
                    .clear()
                    .map_err(|e| format!("cannot clear cache: {e}"))?;
            }
            estimate_cache
                .save_file_count(scan_result.file_count as u64)
                .map_err(|e| format!("cannot save scan estimate: {e}"))?;
        }
        println!("{no_conflicts_message}");
        return Ok(());
    }
    if !options.no_cache && !options.dry_run {
        if options.refresh {
            estimate_cache
                .clear()
                .map_err(|e| format!("cannot clear cache: {e}"))?;
        }
        estimate_cache
            .save_file_count(scan_result.file_count as u64)
            .map_err(|e| format!("cannot save scan estimate: {e}"))?;
    }

    let mut resolved = 0usize;
    for (index, group) in scan_result.groups.iter().enumerate() {
        let Some(decision) = state.decisions.remove(&group.base) else {
            continue;
        };
        let analysis = state
            .analyses
            .get(&group.base)
            .and_then(|result| result.as_ref().ok())
            .ok_or_else(|| format!("analysis missing for {}", group.base.display()))?;
        let prepared = prepare(&scan_result, index, &analysis.analysis, roots)
            .map_err(|e| format!("cannot prepare {}: {e}", group.base.display()))?;
        let outcome = prepared
            .resolve(decision, backup, options.dry_run)
            .map_err(|e| format!("resolution failed for {}: {e}", group.base.display()))?;
        resolved += 1;
        for (source, destination) in outcome.operations {
            println!(
                "{}: {} -> {}",
                if options.dry_run {
                    "would move"
                } else {
                    "moved"
                },
                source.display(),
                destination.display()
            );
        }
    }
    if options.dry_run {
        println!("\ndry-run complete: {resolved} group(s)");
    } else {
        println!("\nresolved {resolved} group(s)");
    }
    if state.skipped > 0 {
        println!("({} skipped)", state.skipped);
    }
    if !options.no_cache && !options.dry_run {
        estimate_cache
            .clear()
            .map_err(|e| format!("cannot invalidate cache: {e}"))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
pub(crate) struct InteractiveOptions {
    pub(crate) no_cache: bool,
    pub(crate) refresh: bool,
    pub(crate) dry_run: bool,
    pub(crate) wait_for_scan: bool,
    pub(crate) wait_for_analysis: bool,
}

enum PipelineMessage {
    GroupAnalysisReady {
        group: Group,
        result: Result<StampedGroupAnalysis, String>,
    },
    ScanFinished(Result<ScanResult, String>),
    AnalysisFinished,
    WorkerError(String),
}

#[derive(Default)]
struct InteractiveState {
    scan_result: Option<Result<ScanResult, String>>,
    analyses: BTreeMap<PathBuf, Result<StampedGroupAnalysis, String>>,
    analysis_finished: bool,
    decisions: BTreeMap<PathBuf, Decision>,
    skipped: usize,
    completed: usize,
    waiting_after_choice: bool,
    quit: bool,
    worker_error: Option<String>,
    analysis_error: Option<String>,
    interaction_error: Option<String>,
}

impl InteractiveState {
    fn record(&mut self, message: PipelineMessage) -> Option<Group> {
        match message {
            PipelineMessage::GroupAnalysisReady { group, result } => {
                self.analyses.insert(group.base.clone(), result);
                Some(group)
            }
            PipelineMessage::ScanFinished(result) => {
                self.scan_result = Some(result);
                None
            }
            PipelineMessage::AnalysisFinished => {
                self.analysis_finished = true;
                None
            }
            PipelineMessage::WorkerError(error) => {
                self.worker_error = Some(error);
                None
            }
        }
    }

    fn scan_failed(&self) -> bool {
        match self.scan_result.as_ref() {
            Some(Ok(result)) => result.cancelled || !result.errors.is_empty(),
            Some(Err(_)) => true,
            None => false,
        }
    }
}

fn collect_interactive_results(
    receiver: &Receiver<PipelineMessage>,
    state: &mut InteractiveState,
    options: InteractiveOptions,
    progress: &ScanProgress,
) {
    let mut last_status = None;
    if options.wait_for_analysis || options.wait_for_scan {
        while (state.scan_result.is_none()
            || (options.wait_for_analysis && !state.analysis_finished))
            && state.worker_error.is_none()
        {
            receive_and_record(
                receiver,
                state,
                progress,
                &mut last_status,
                Some(if options.wait_for_analysis {
                    "Waiting for scan and group analysis..."
                } else {
                    "Waiting for scan to finish..."
                }),
            );
        }
        if options.wait_for_analysis || !state.scan_failed() {
            if let Some(Ok(scan)) = state.scan_result.take() {
                if let Err(error) = ensure_scan_complete(&scan) {
                    state.scan_result = Some(Ok(scan));
                    state.worker_error = Some(error);
                } else {
                    prompt_sorted_groups(
                        &scan,
                        receiver,
                        state,
                        options,
                        progress,
                        &mut last_status,
                    );
                    state.scan_result = Some(Ok(scan));
                }
            }
        }
    } else {
        while (state.scan_result.is_none() || !state.analysis_finished)
            && state.worker_error.is_none()
        {
            let status = waiting_status(state);
            if let Some(group) =
                receive_and_record(receiver, state, progress, &mut last_status, status)
            {
                if !state.quit && !state.scan_failed() {
                    prompt_ready_group(&group, None, state, progress);
                }
            }
        }
    }

    while (state.scan_result.is_none() || !state.analysis_finished) && state.worker_error.is_none()
    {
        receive_and_record(
            receiver,
            state,
            progress,
            &mut last_status,
            waiting_status(state),
        );
    }
}

fn prompt_sorted_groups(
    scan: &ScanResult,
    receiver: &Receiver<PipelineMessage>,
    state: &mut InteractiveState,
    options: InteractiveOptions,
    progress: &ScanProgress,
    last_status: &mut Option<String>,
) {
    for (index, group) in scan.groups.iter().enumerate() {
        if state.quit {
            break;
        }
        while !state.analyses.contains_key(&group.base)
            && !state.analysis_finished
            && state.worker_error.is_none()
        {
            receive_and_record(
                receiver,
                state,
                progress,
                last_status,
                Some("Scan complete; waiting for remaining group analysis..."),
            );
        }
        if state.worker_error.is_some() {
            break;
        }
        if state.analysis_finished && !state.analyses.contains_key(&group.base) {
            state.analysis_error = Some(format!("analysis missing for {}", group.base.display()));
            break;
        }
        prompt_ready_group(group, Some((index + 1, scan.groups.len())), state, progress);
    }
    while !state.analysis_finished && state.worker_error.is_none() {
        receive_and_record(
            receiver,
            state,
            progress,
            last_status,
            Some(if options.wait_for_analysis {
                "Waiting for all group analyses..."
            } else {
                "Finishing group analysis..."
            }),
        );
    }
}

fn receive_and_record(
    receiver: &Receiver<PipelineMessage>,
    state: &mut InteractiveState,
    progress: &ScanProgress,
    last_status: &mut Option<String>,
    status: Option<&str>,
) -> Option<Group> {
    let message = match receiver.try_recv() {
        Ok(message) => message,
        Err(TryRecvError::Empty) => {
            match status {
                Some(status) if last_status.as_deref() != Some(status) => {
                    progress.suspend(|| println!("{status}"));
                    *last_status = Some(status.to_string());
                }
                None => *last_status = None,
                _ => (),
            }
            if status.is_some() {
                state.waiting_after_choice = false;
            }
            if let Err(error) = io::stdout().flush() {
                let error = format!("cannot flush status: {error}");
                state.interaction_error = Some(error.clone());
                state.worker_error = Some(error);
                return None;
            }
            match receiver.recv() {
                Ok(message) => message,
                Err(_) => {
                    state.worker_error =
                        Some("scan and analysis workers stopped unexpectedly".into());
                    return None;
                }
            }
        }
        Err(TryRecvError::Disconnected) => {
            state.worker_error = Some("scan and analysis workers stopped unexpectedly".into());
            return None;
        }
    };
    if matches!(&message, PipelineMessage::GroupAnalysisReady { .. }) {
        *last_status = None;
    }
    state.waiting_after_choice = false;
    state.record(message)
}

fn prompt_ready_group(
    group: &Group,
    position: Option<(usize, usize)>,
    state: &mut InteractiveState,
    progress: &ScanProgress,
) {
    if state.quit || state.interaction_error.is_some() {
        return;
    }
    let Some(result) = state.analyses.get(&group.base) else {
        state.analysis_error = Some(format!("analysis missing for {}", group.base.display()));
        return;
    };
    let stamped = match result {
        Ok(stamped) => stamped,
        Err(error) => {
            state.analysis_error = Some(format!(
                "analysis failed for {}: {error}",
                group.base.display()
            ));
            eprintln!("{}", state.analysis_error.as_deref().unwrap_or_default());
            state.quit = true;
            return;
        }
    };
    let report = group_report(group, &stamped.analysis, position);
    let choice = progress.suspend(|| {
        println!("{}", report.text);
        prompt_choice(&stamped.analysis, report.recommendation.as_ref())
    });
    match choice {
        Ok(PromptChoice::Resolve(decision)) => {
            state.decisions.insert(group.base.clone(), decision);
            state.completed += 1;
            state.waiting_after_choice = position.is_none();
        }
        Ok(PromptChoice::Skip) => {
            state.skipped += 1;
            state.completed += 1;
            state.waiting_after_choice = position.is_none();
        }
        Ok(PromptChoice::Quit) => {
            state.quit = true;
        }
        Err(error) => {
            state.interaction_error = Some(error);
            state.quit = true;
        }
    }
}

fn waiting_status(state: &InteractiveState) -> Option<&'static str> {
    let has_pending_work = state.scan_result.is_none() || has_pending_group_analysis(state);
    if !has_pending_work {
        return None;
    }
    if state.waiting_after_choice {
        return Some("Choice recorded; waiting for the next stable group...");
    }
    if state.quit {
        if state.scan_result.is_none() {
            Some("Quit requested; finishing the scan to preserve recorded choices...")
        } else if has_pending_group_analysis(state) {
            Some("Finishing pending group analysis; no further groups will be prompted...")
        } else {
            None
        }
    } else if state.scan_result.is_none() {
        Some("Scanning and analyzing groups; waiting for the next stable group...")
    } else if has_pending_group_analysis(state) {
        Some("Scan complete; waiting for remaining group analysis...")
    } else {
        None
    }
}

fn has_pending_group_analysis(state: &InteractiveState) -> bool {
    let Some(Ok(scan)) = state.scan_result.as_ref() else {
        return false;
    };
    scan.groups
        .iter()
        .any(|group| !state.analyses.contains_key(&group.base))
}

fn ensure_scan_complete(scan: &ScanResult) -> Result<(), String> {
    if scan.cancelled || !scan.errors.is_empty() {
        let detail = scan
            .errors
            .first()
            .map(|(path, error)| format!("; first error: {}: {error}", path.display()))
            .unwrap_or_default();
        return Err(format!(
            "scan incomplete; refusing to resolve ({} error(s){detail})",
            scan.errors.len()
        ));
    }
    Ok(())
}

fn ensure_group_analyses_complete(
    scan: &ScanResult,
    state: &InteractiveState,
) -> Result<(), String> {
    if !state.analysis_finished {
        return Err("group analysis did not finish".into());
    }
    for group in &scan.groups {
        match state.analyses.get(&group.base) {
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                return Err(format!(
                    "analysis failed for {}: {error}",
                    group.base.display()
                ));
            }
            None => return Err(format!("analysis missing for {}", group.base.display())),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_with_one_group() -> ScanResult {
        ScanResult {
            groups: vec![Group {
                base: PathBuf::from("base"),
                conflicts: Vec::new(),
            }],
            errors: Vec::new(),
            cancelled: false,
            file_count: 2,
            directory_count: 1,
            analysis_count: 1,
        }
    }

    #[test]
    fn choice_wait_status_is_hidden_when_no_group_analysis_is_pending() {
        let mut state = InteractiveState {
            scan_result: Some(Ok(scan_with_one_group())),
            waiting_after_choice: true,
            ..InteractiveState::default()
        };
        assert_eq!(
            waiting_status(&state),
            Some("Choice recorded; waiting for the next stable group...")
        );

        state
            .analyses
            .insert(PathBuf::from("base"), Err("analysis failed".into()));
        assert_eq!(waiting_status(&state), None);
    }

    #[test]
    fn scan_complete_status_requires_pending_group_analysis() {
        let state = InteractiveState {
            scan_result: Some(Ok(scan_with_one_group())),
            ..InteractiveState::default()
        };
        assert_eq!(
            waiting_status(&state),
            Some("Scan complete; waiting for remaining group analysis...")
        );
    }
}
