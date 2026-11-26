use crate::{
    cli::Cli,
    cli_config::{effective_scan, general_backup, general_cache, media_tools},
    progress::perform_scan,
    report::group_report,
};
use console::style;
use deconflict_core::{
    analysis::{
        analyze_scanned_group_cached, analyze_scanned_group_with_media, recommendation_label,
        GroupAnalysis, Recommendation,
    },
    cache::{cache_key, FileCache},
    media::Tools,
    resolve::{prepare, Decision},
    scan::ScanResult,
};
use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

pub(crate) fn run_bare_cli(cli: &Cli) -> Result<(), String> {
    let (scan_config, patterns, roots, config) =
        effective_scan(cli.config.as_deref(), cli, &cli.dirs, None)?;
    let key = cache_key(&roots).map_err(|e| format!("cannot key cache: {e}"))?;
    let mut file_cache = FileCache::new(general_cache(&config), &key, false);
    let estimated_file_count = file_cache.file_count();
    let show_progress = std::io::stdout().is_terminal();
    let result = perform_scan(
        &roots,
        &scan_config,
        &patterns,
        show_progress,
        estimated_file_count,
    )?;
    if result.cancelled || !result.errors.is_empty() {
        return Err("scan incomplete; refusing to resolve".into());
    }
    file_cache
        .save_file_count(result.file_count as u64)
        .map_err(|e| format!("cannot save scan estimate: {e}"))?;
    if result.groups.is_empty() {
        println!("no conflicts found");
        return Ok(());
    }

    let backup = general_backup(&config);
    let media = media_tools(&config);
    resolve_interactively(
        &result,
        &roots,
        &backup,
        &mut file_cache,
        &media,
        InteractiveOptions {
            no_cache: false,
            refresh: false,
            dry_run: false,
        },
    )?;
    file_cache
        .clear()
        .map_err(|e| format!("cannot invalidate cache: {e}"))?;
    Ok(())
}

pub(crate) fn resolve_interactively(
    result: &ScanResult,
    roots: &[PathBuf],
    backup: &Path,
    file_cache: &mut FileCache,
    media: &Tools,
    options: InteractiveOptions,
) -> Result<(), String> {
    let mut resolved = 0usize;
    let mut skipped = 0usize;
    for (index, group) in result.groups.iter().enumerate() {
        let stamped = if options.no_cache {
            analyze_scanned_group_with_media(group, media)
        } else {
            analyze_scanned_group_cached(group, file_cache, options.refresh, media)
        }
        .map_err(|e| format!("analysis failed for {}: {e}", group.base.display()))?;
        let report = group_report(group, &stamped.analysis, index + 1, result.groups.len());
        println!("{}", report.text);
        let choice = prompt_choice(&stamped.analysis, report.recommendation.as_ref())?;
        let decision = match choice {
            PromptChoice::Quit => {
                println!("aborted after {index} of {} group(s)", result.groups.len());
                break;
            }
            PromptChoice::Skip => {
                skipped += 1;
                continue;
            }
            PromptChoice::Resolve(decision) => decision,
        };
        let prepared = prepare(result, index, &stamped.analysis, roots)
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
    if skipped > 0 {
        println!("({skipped} skipped)");
    }
    Ok(())
}

pub(crate) struct InteractiveOptions {
    pub(crate) no_cache: bool,
    pub(crate) refresh: bool,
    pub(crate) dry_run: bool,
}

enum PromptChoice {
    Resolve(Decision),
    Skip,
    Quit,
}

fn prompt_choice(
    analysis: &GroupAnalysis,
    recommendation: Option<&Recommendation>,
) -> Result<PromptChoice, String> {
    let default = recommendation
        .and_then(|choice| recommendation_label(analysis, choice))
        .map(|label| format!("(Enter) {label} "));
    loop {
        let mut choices = Vec::new();
        if let Some(default) = &default {
            choices.push(style(default).cyan().bold().to_string());
        }
        if analysis.base.is_some() {
            choices.push("(0) keep base".into());
        }
        choices.extend(
            analysis
                .copies
                .iter()
                .enumerate()
                .map(|(index, _)| format!("({}) keep copy #{}", index + 1, index + 1)),
        );
        choices.extend(["(h) keep both".into(), "(s) skip".into(), "(q) quit".into()]);
        print!("{}: ", choices.join(" "));
        io::stdout()
            .flush()
            .map_err(|e| format!("cannot flush prompt: {e}"))?;

        let mut answer = String::new();
        let bytes = io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| format!("cannot read choice: {e}"))?;
        if bytes == 0 {
            return Err("stdin closed before a resolution choice".into());
        }
        let answer = answer.trim().to_lowercase();
        if answer.is_empty() {
            if let Some(recommendation) = recommendation {
                return Ok(PromptChoice::Resolve(match recommendation {
                    Recommendation::Base => Decision::KeepBase,
                    Recommendation::Copy(index) => Decision::KeepCopy(*index),
                }));
            }
        }
        match answer.as_str() {
            "q" | "quit" | "exit" => return Ok(PromptChoice::Quit),
            "s" | "skip" => return Ok(PromptChoice::Skip),
            "h" | "keep-both" => return Ok(PromptChoice::Resolve(Decision::KeepBoth)),
            "0" if analysis.base.is_some() => return Ok(PromptChoice::Resolve(Decision::KeepBase)),
            value => {
                if let Ok(copy_index) = value.parse::<usize>() {
                    if copy_index > 0 && copy_index <= analysis.copies.len() {
                        return Ok(PromptChoice::Resolve(Decision::KeepCopy(copy_index - 1)));
                    }
                }
                println!("invalid choice");
            }
        }
    }
}
