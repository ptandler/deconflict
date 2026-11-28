use deconflict_core::analysis::analyze_group;
use deconflict_core::config::ScanConfig;
use deconflict_core::patterns::Patterns;
use deconflict_core::scan::{scan, Event};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn io_counters() -> BTreeMap<String, u64> {
    std::fs::read_to_string("/proc/self/io")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.to_string(), value.trim().parse().ok()?))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: benchmark ROOT WORKERS".into());
    }
    let root = PathBuf::from(&args[1]);
    let config = ScanConfig {
        workers: args[2].parse()?,
        ..ScanConfig::default()
    };
    let patterns = Patterns::build(&config)?;
    let before_io = io_counters();
    let start = Instant::now();
    let mut provisional = 0;
    let result = scan(
        &[root],
        &config,
        &patterns,
        &AtomicBool::new(false),
        |event| {
            if let Event::Provisional(_, Ok(_)) = event {
                provisional += 1;
            }
        },
    )?;
    let scan_end = Instant::now();
    if result.cancelled || !result.errors.is_empty() {
        return Err(format!("scan failed: {:?}", result.errors).into());
    }
    let mut analyzed = 0;
    for group in &result.groups {
        let copies: Vec<_> = group
            .conflicts
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        let analysis = analyze_group(&group.base, &copies)?;
        analyzed += usize::from(analysis.base.is_some()) + analysis.copies.len();
    }
    let end = Instant::now();
    let after_io = io_counters();
    let peak_rss = std::fs::read_to_string("/proc/self/status")?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .ok_or("VmHWM unavailable")?;
    let io: Vec<_> = before_io
        .iter()
        .filter_map(|(key, before)| {
            after_io
                .get(key)
                .map(|after| format!("\"{key}\":{}", after - before))
        })
        .collect();
    println!(
        "{{\"scan_seconds\":{},\"analysis_seconds\":{},\"total_seconds\":{},\"groups\":{},\"analyzed_files\":{},\"provisional_files\":{},\"peak_rss_kib\":{},\"io_bytes\":{{{}}}}}",
        scan_end.duration_since(start).as_secs_f64(),
        end.duration_since(scan_end).as_secs_f64(),
        end.duration_since(start).as_secs_f64(),
        result.groups.len(),
        analyzed,
        provisional,
        peak_rss,
        io.join(",")
    );
    Ok(())
}
