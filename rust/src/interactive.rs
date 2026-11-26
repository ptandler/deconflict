use crate::{
    cli::Cli,
    cli_config::{effective_scan, general_backup, general_cache, media_tools},
    progress::perform_scan,
};
use deconflict_core::{
    analysis::analyze_scanned_group_cached,
    cache::{cache_key, FileCache},
    resolve::{prepare, Decision},
};
use std::io::{self, BufRead, IsTerminal};

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
    let mut resolved = 0usize;
    let mut skipped = 0usize;

    for (index, group) in result.groups.iter().enumerate() {
        let stamped = analyze_scanned_group_cached(group, &mut file_cache, false, &media)
            .map_err(|e| format!("analysis failed for {}: {e}", group.base.display()))?;
        println!("\nGroup {}/{}", index + 1, result.groups.len());
        println!("base: {}", group.base.display());
        for (copy_index, (copy, _)) in group.conflicts.iter().enumerate() {
            println!("{}: {}", copy_index + 1, copy.display());
        }
        print!("(0) keep base, (1..) keep copy, (h) keep both, (s) skip, (q) quit: ");
        let _ = io::Write::flush(&mut io::stdout());

        let mut answer = String::new();
        io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| e.to_string())?;
        let answer = answer.trim().to_lowercase();
        let decision = match answer.as_str() {
            "q" | "quit" | "exit" => {
                println!("aborted after {index} of {} group(s)", result.groups.len());
                break;
            }
            "s" | "skip" => {
                skipped += 1;
                continue;
            }
            "h" | "keep-both" => Decision::KeepBoth,
            "0" => Decision::KeepBase,
            value => match value.parse::<usize>() {
                Ok(copy_index) if copy_index > 0 && copy_index <= group.conflicts.len() => {
                    Decision::KeepCopy(copy_index - 1)
                }
                _ => {
                    println!("invalid choice");
                    continue;
                }
            },
        };

        let prepared = prepare(&result, index, &stamped.analysis, &roots)
            .map_err(|e| format!("cannot prepare {}: {e}", group.base.display()))?;
        let outcome = prepared
            .resolve(decision, &backup, false)
            .map_err(|e| format!("resolution failed for {}: {e}", group.base.display()))?;
        resolved += 1;
        for (source, destination) in outcome.operations {
            println!("{} -> {}", source.display(), destination.display());
        }
    }

    file_cache
        .clear()
        .map_err(|e| format!("cannot invalidate cache: {e}"))?;
    println!("\nresolved {resolved} group(s)");
    if skipped > 0 {
        println!("({skipped} skipped)");
    }
    Ok(())
}
