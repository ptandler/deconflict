use crate::{
    cli::{Cli, ResolveArgs},
    cli_config::{detected_tools, effective_scan, general_backup, general_cache, media_tools},
    progress::{conflict_count, perform_scan},
};
use deconflict_core::{
    analysis::{
        analyze_scanned_group_cached, analyze_scanned_group_with_media, recommend, Recommendation,
    },
    cache::{cache_key, FileCache},
    resolve::{prepare, Decision},
};

pub(super) fn run(cli: &Cli, args: ResolveArgs) -> Result<(), String> {
    if args.tui {
        if args.auto.is_some() {
            return Err("--auto and --tui are mutually exclusive".into());
        }
        if cli.no_tui {
            return Err("--tui and --no-tui are mutually exclusive".into());
        }
        return crate::tui::run(
            cli,
            crate::tui::TuiOptions {
                dirs: &args.dirs,
                pattern: args.pattern.as_deref(),
                no_cache: args.no_cache,
                refresh: args.refresh,
                dry_run: args.dry_run,
                show_progress: args.progress && !args.no_progress,
            },
        );
    }
    let (scan_config, patterns, roots, config) = effective_scan(
        cli.config.as_deref(),
        cli,
        &args.dirs,
        args.pattern.as_deref(),
    )?;
    let backup = general_backup(&config);
    let cache_dir = general_cache(&config);
    let media = media_tools(&config);
    let external_tools = detected_tools(&config);
    if args.auto.is_none() {
        return crate::interactive::run_interactive_scan(
            crate::interactive::InteractiveScanRequest {
                roots: &roots,
                scan_config: &scan_config,
                patterns: &patterns,
                config: &config,
                external_tools: &external_tools,
                backup: &backup,
                cache_dir: &cache_dir,
                media: &media,
                options: crate::interactive::InteractiveOptions {
                    no_cache: args.no_cache,
                    refresh: args.refresh,
                    dry_run: args.dry_run,
                    wait_for_scan: args.wait_for_scan,
                    wait_for_analysis: args.wait_for_analysis,
                },
                show_progress: args.progress && !args.no_progress,
                no_conflicts_message: "no conflicts to resolve",
            },
        );
    }

    let key = cache_key(&roots).map_err(|e| format!("cannot key cache: {e}"))?;
    let mut file_cache = FileCache::new(&cache_dir, &key, args.dry_run || args.no_cache);
    let estimated_file_count = if args.no_cache || args.dry_run {
        None
    } else {
        file_cache.file_count()
    };
    let result = perform_scan(
        &roots,
        &scan_config,
        &patterns,
        args.progress && !args.no_progress,
        estimated_file_count,
    )?;
    if result.cancelled || !result.errors.is_empty() {
        return Err("scan incomplete; refusing to resolve".into());
    }
    if !args.no_cache && !args.dry_run {
        if args.refresh {
            file_cache
                .clear()
                .map_err(|e| format!("cannot clear cache: {e}"))?;
        }
        file_cache
            .save_file_count(result.file_count as u64)
            .map_err(|e| format!("cannot save scan estimate: {e}"))?;
    }
    if result.groups.is_empty() {
        println!("no conflicts to resolve");
        return Ok(());
    }

    let mut resolved = 0;
    for (index, group) in result.groups.iter().enumerate() {
        let stamped = if args.no_cache {
            analyze_scanned_group_with_media(group, &media)
        } else {
            analyze_scanned_group_cached(group, &mut file_cache, args.refresh, &media)
        }
        .map_err(|e| format!("analysis failed for {}: {e}", group.base.display()))?;
        let prepared = prepare(&result, index, &stamped.analysis, &roots)
            .map_err(|e| format!("cannot prepare {}: {e}", group.base.display()))?;
        let rule = args.auto.as_deref().unwrap_or("base");
        let decision = match rule {
            "base" => Decision::KeepBase,
            "copy" => Decision::KeepCopy(0),
            "newest" => match stamped.analysis.winner_hint() {
                Some(Recommendation::Copy(i)) => Decision::KeepCopy(i),
                _ => Decision::KeepBase,
            },
            "bigger" => {
                let base = stamped.analysis.base.as_ref();
                let biggest = stamped
                    .analysis
                    .copies
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| base.is_none_or(|b| c.size > b.size))
                    .max_by_key(|(_, c)| c.size)
                    .map(|(i, _)| i);
                biggest.map_or(Decision::KeepBase, Decision::KeepCopy)
            }
            "recommended" => match recommend(&stamped.analysis, stamped.analysis.kind == "text") {
                Some(Recommendation::Copy(i)) => Decision::KeepCopy(i),
                Some(Recommendation::Base) => Decision::KeepBase,
                None => continue,
            },
            _ => return Err(format!("unknown auto rule: {rule}")),
        };
        let outcome = prepared
            .resolve(decision, &backup, args.dry_run)
            .map_err(|e| format!("resolution failed for {}: {e}", group.base.display()))?;
        resolved += 1;
        if args.dry_run {
            println!(
                "dry-run: {} ({} operation(s))",
                group.base.display(),
                outcome.operations.len()
            );
        } else {
            println!("resolved: {}", group.base.display());
        }
    }
    if args.dry_run {
        println!("dry-run complete: {}", conflict_count(resolved));
    } else if !args.no_cache {
        file_cache
            .clear()
            .map_err(|e| format!("cannot invalidate cache: {e}"))?;
    }
    Ok(())
}
