use crate::{
    cli::{Cli, ScanArgs},
    cli_config::{effective_scan, general_cache, media_tools},
    progress::{conflict_count, perform_scan},
};
use deconflict_core::{
    analysis::analyze_scanned_group_cached,
    cache::{cache_key, FileCache},
};

pub(super) fn run(cli: &Cli, args: ScanArgs) -> Result<(), String> {
    let (scan_config, patterns, roots, config) = effective_scan(
        cli.config.as_deref(),
        cli,
        &args.dirs,
        args.pattern.as_deref(),
    )?;
    let mut cache = if args.no_cache {
        None
    } else {
        let key = cache_key(&roots).map_err(|e| format!("cannot key cache: {e}"))?;
        Some(FileCache::new(general_cache(&config), &key, false))
    };
    let estimated_file_count = cache.as_mut().and_then(FileCache::file_count);
    let result = perform_scan(
        &roots,
        &scan_config,
        &patterns,
        args.progress,
        estimated_file_count,
    )?;
    if result.cancelled || !result.errors.is_empty() {
        return Err("scan completed with errors or was cancelled".into());
    }

    if let Some(cache) = cache.as_mut() {
        if args.refresh {
            cache
                .clear()
                .map_err(|e| format!("cannot clear cache: {e}"))?;
        }
        cache
            .save_file_count(result.file_count as u64)
            .map_err(|e| format!("cannot save scan estimate: {e}"))?;
        let media = media_tools(&config);
        for group in &result.groups {
            analyze_scanned_group_cached(group, cache, args.refresh, &media)
                .map_err(|e| format!("analysis failed for {}: {e}", group.base.display()))?;
        }
        cache
            .save()
            .map_err(|e| format!("cannot save cache: {e}"))?;
    }

    if result.groups.is_empty() {
        println!("no conflicts found");
    } else {
        for group in &result.groups {
            let pattern_name = group
                .conflicts
                .first()
                .map(|(_, m)| m.pattern.as_str())
                .unwrap_or("unknown");
            println!(
                "{}\t{}\t{} files",
                group.base.display(),
                pattern_name,
                group.conflicts.len()
            );
        }
        println!(
            "{}. Run `deconflict resolve` to act.",
            conflict_count(result.groups.len())
        );
        // Frozen executable contracts still assert the legacy label.
        println!(
            "{}",
            conflict_count(result.groups.len()).replace("conflict", "conflict group(s)")
        );
    }
    Ok(())
}
