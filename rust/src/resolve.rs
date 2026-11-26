//! Conservative resolution of a finalized scan.
//!
//! Preparation captures identity in addition to the analysis snapshot. Paths must be regular
//! files, not symlinks. Operations are not transactional and cannot exclude concurrent writers;
//! callers must serialize access to scan roots and backup storage for the duration.

use crate::analysis::{analyze, GroupAnalysis};
use crate::scan::ScanResult;
use std::collections::HashSet;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static RESOLVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy)]
pub enum Decision {
    KeepBase,
    KeepCopy(usize),
    KeepBoth,
}

#[derive(Debug)]
pub struct Outcome {
    pub operations: Vec<(PathBuf, PathBuf)>,
    pub dry_run: bool,
}

#[derive(Debug)]
struct Snapshot {
    info: crate::analysis::FileInfo,
    metadata: Metadata,
}

#[derive(Debug)]
pub struct Prepared {
    base: PathBuf,
    files: Vec<Snapshot>,
    has_base: bool,
    roots: Vec<PathBuf>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn same_identity(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
            && a.len() == b.len()
            && a.modified().ok() == b.modified().ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

fn snapshot(info: &crate::analysis::FileInfo) -> io::Result<Snapshot> {
    let metadata = fs::symlink_metadata(&info.path)?;
    if !metadata.file_type().is_file() {
        return Err(invalid(
            "only regular files without symlinks may be resolved",
        ));
    }
    if analyze(&info.path)? != *info
        || !same_identity(&metadata, &fs::symlink_metadata(&info.path)?)
    {
        return Err(invalid(format!(
            "file changed since analysis: {}",
            info.path.display()
        )));
    }
    Ok(Snapshot {
        info: info.clone(),
        metadata,
    })
}

fn validate(s: &Snapshot) -> io::Result<()> {
    let meta = fs::symlink_metadata(&s.info.path)?;
    if !meta.file_type().is_file()
        || !same_identity(&s.metadata, &meta)
        || analyze(&s.info.path)? != s.info
        || !same_identity(&s.metadata, &fs::symlink_metadata(&s.info.path)?)
    {
        return Err(invalid(format!(
            "file changed since preparation: {}",
            s.info.path.display()
        )));
    }
    Ok(())
}

fn validate_after_link(s: &Snapshot) -> io::Result<()> {
    let now = snapshot(&s.info)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if now.metadata.dev() != s.metadata.dev() || now.metadata.ino() != s.metadata.ino() {
            return Err(invalid("file identity changed"));
        }
    }
    Ok(())
}

/// Bind an analysis to one final group. `roots` must be the same complete root set used by scan.
/// The scan API does not retain roots or identity, so callers must supply trustworthy inputs.
pub fn prepare(
    scan: &ScanResult,
    group_index: usize,
    analysis: &GroupAnalysis,
    roots: &[PathBuf],
) -> io::Result<Prepared> {
    if scan.cancelled || !scan.errors.is_empty() || roots.is_empty() {
        return Err(invalid("scan is incomplete or roots are missing"));
    }
    let group = scan
        .groups
        .get(group_index)
        .ok_or_else(|| invalid("invalid group index"))?;
    let roots = roots
        .iter()
        .map(fs::canonicalize)
        .collect::<io::Result<Vec<_>>>()?;
    let parent = fs::canonicalize(
        group
            .base
            .parent()
            .ok_or_else(|| invalid("missing base parent"))?,
    )?;
    let base = parent.join(
        group
            .base
            .file_name()
            .ok_or_else(|| invalid("missing base name"))?,
    );
    if !roots.iter().any(|r| base.starts_with(r) && base != *r) {
        return Err(invalid("group outside scan roots"));
    }
    if analysis.base.as_ref().is_some_and(|b| b.path != group.base)
        || analysis.copies.len() != group.conflicts.len()
        || analysis
            .copies
            .iter()
            .zip(&group.conflicts)
            .any(|(a, (p, _))| a.path != *p)
        || group.conflicts.is_empty()
    {
        return Err(invalid("analysis does not match scanned group"));
    }
    let mut paths = HashSet::new();
    paths.insert(base.clone());
    for (p, _) in &group.conflicts {
        if fs::canonicalize(p.parent().ok_or_else(|| invalid("missing parent"))?)? != parent
            || !paths.insert(parent.join(p.file_name().ok_or_else(|| invalid("missing name"))?))
        {
            return Err(invalid("duplicate or out-of-directory conflict"));
        }
    }
    if analysis.base.is_none() {
        match fs::symlink_metadata(&base) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Ok(_) => return Err(invalid("base appeared since analysis")),
            Err(e) => return Err(e),
        }
    }
    let files = analysis
        .base
        .iter()
        .chain(&analysis.copies)
        .map(snapshot)
        .collect::<io::Result<Vec<_>>>()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mut identities = HashSet::new();
        if files
            .iter()
            .any(|s| !identities.insert((s.metadata.dev(), s.metadata.ino())))
        {
            return Err(invalid("group contains hard-linked files"));
        }
    }
    Ok(Prepared {
        base,
        files,
        has_base: analysis.base.is_some(),
        roots,
    })
}

// Canonicalize existing ancestors without creating directories, rejecting ambiguous `..`.
fn prospective(path: &Path) -> io::Result<PathBuf> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid("parent components are not allowed"));
    }
    let mut p = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        match fs::canonicalize(&p) {
            Ok(mut parent) => {
                for part in tail.iter().rev() {
                    parent.push(part);
                }
                return Ok(parent);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                match fs::symlink_metadata(&p) {
                    Ok(_) => return Err(invalid("dangling or inaccessible backup path")),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
                tail.push(
                    p.file_name()
                        .ok_or_else(|| invalid("invalid backup path"))?
                        .to_os_string(),
                );
                p = p
                    .parent()
                    .ok_or_else(|| invalid("invalid backup parent"))?
                    .to_path_buf();
                if p.as_os_str().is_empty() {
                    p = PathBuf::from(".");
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn open_log(root: &Path) -> io::Result<File> {
    let path = root.join("decision-log.jsonl");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(invalid("decision log is not a regular file"));
    }
    let mut file = OpenOptions::new()
        .read(true)
        .create(true)
        .append(true)
        .open(path)?;
    if file.metadata()?.len() > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            return Err(invalid("decision log has an incomplete trailing record"));
        }
    }
    Ok(file)
}

fn log(
    file: &mut File,
    decision: Decision,
    base: &Path,
    chosen: Option<&Path>,
    operations: &[(PathBuf, PathBuf)],
) -> io::Result<()> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let action = match decision {
        Decision::KeepBase => "keep_base",
        Decision::KeepCopy(_) => "keep_copy",
        Decision::KeepBoth => "keep_both",
    };
    let entry = |action: &str, to: &Path| {
        format!(
            "{{\"action\":{},\"moved_to\":{},\"detail\":\"\"}}",
            json_string(action),
            json_string(&to.to_string_lossy())
        )
    };
    let decisions = operations
        .iter()
        .enumerate()
        .map(|(i, (_, to))| {
            let rename = matches!(decision, Decision::KeepBoth)
                || (chosen.is_some() && i + 1 == operations.len());
            entry(if rename { "rename" } else { "move" }, to)
        })
        .collect::<Vec<_>>()
        .join(",");
    let group = base
        .file_name()
        .ok_or_else(|| invalid("missing base name"))?
        .to_string_lossy();
    let mut record = format!(
        "{{\"ts\":{ts},\"group\":{},\"action\":{},\"base\":{},\"decisions\":[{decisions}]",
        json_string(&group),
        json_string(action),
        json_string(&base.to_string_lossy())
    );
    if let Some(copy) = chosen {
        record.push_str(&format!(
            ",\"copy\":{},\"renamed\":{}",
            json_string(&copy.to_string_lossy()),
            entry("rename", base)
        ));
    }
    record.push_str("}\n");
    file.write_all(record.as_bytes())?;
    file.sync_all()
}

fn sanitize(name: &str) -> String {
    let mut out = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>();
    if out.is_empty() || out == "." || out == ".." {
        out = "conflict".into();
    }
    out
}

fn free_name(dir: &Path, name: &str, reserved: &HashSet<PathBuf>) -> io::Result<PathBuf> {
    for i in 0..10_000 {
        let candidate = dir.join(if i == 0 {
            name.to_owned()
        } else {
            format!("{name}.{i}")
        });
        if reserved.contains(&candidate) {
            continue;
        }
        match fs::symlink_metadata(&candidate) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(e) => return Err(e),
            Ok(_) => (),
        }
    }
    Err(invalid("no free destination name"))
}

impl Prepared {
    /// Resolve a prepared group. Errors may occur after partial progress without a log entry;
    /// inspect sources and backups before retrying. Dry runs never create directories or logs.
    pub fn resolve(
        &self,
        decision: Decision,
        backup_root: &Path,
        dry_run: bool,
    ) -> io::Result<Outcome> {
        self.resolve_inner(decision, backup_root, dry_run, |_| Ok(()))
    }

    fn resolve_inner(
        &self,
        decision: Decision,
        backup_root: &Path,
        dry_run: bool,
        mut after_operation: impl FnMut(usize) -> io::Result<()>,
    ) -> io::Result<Outcome> {
        let _guard = RESOLVE_LOCK
            .lock()
            .map_err(|_| io::Error::other("resolver lock poisoned"))?;
        let backup = prospective(backup_root)?;
        if self
            .roots
            .iter()
            .any(|r| backup.starts_with(r) || r.starts_with(&backup))
        {
            return Err(invalid("backup and scan roots overlap"));
        }
        let offset = usize::from(self.has_base);
        let chosen = match decision {
            Decision::KeepBase if !self.has_base => return Err(invalid("base is missing")),
            Decision::KeepCopy(i) if i >= self.files.len() - offset => {
                return Err(invalid("invalid copy index"))
            }
            Decision::KeepCopy(i) => Some(i + offset),
            _ => None,
        };
        for s in &self.files {
            validate(s)?;
        }
        if !self.has_base {
            match fs::symlink_metadata(&self.base) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Ok(_) => return Err(invalid("base appeared since preparation")),
                Err(e) => return Err(e),
            }
        }
        let mut operations = Vec::new();
        let mut reserved = HashSet::new();
        let mut log_file = if !dry_run {
            fs::create_dir_all(&backup)?;
            if fs::canonicalize(backup_root)? != backup {
                return Err(invalid("backup path changed during directory creation"));
            }
            Some(open_log(&backup)?)
        } else {
            None
        };
        for (index, s) in self.files.iter().enumerate() {
            if (self.has_base
                && index == 0
                && matches!(decision, Decision::KeepBase | Decision::KeepBoth))
                || chosen == Some(index)
            {
                continue;
            }
            let target = if matches!(decision, Decision::KeepBoth) {
                let name = self
                    .base
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| invalid("non-UTF-8 base name"))?;
                free_name(
                    self.base.parent().unwrap(),
                    &format!("{name} (copy {})", index + 1 - offset),
                    &reserved,
                )?
            } else {
                let group_name = sanitize(
                    self.base
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("conflict"),
                );
                let group_dir = backup.join(group_name);
                if !dry_run {
                    fs::create_dir_all(&group_dir)?;
                }
                let loser_name = sanitize(
                    s.info
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("backup"),
                );
                free_name(&group_dir, &loser_name, &reserved)?
            };
            reserved.insert(target.clone());
            if !dry_run {
                validate(s)?;
                if matches!(decision, Decision::KeepBoth) {
                    fs::hard_link(&s.info.path, &target)?;
                } else {
                    let mut source = File::open(&s.info.path)?;
                    if !same_identity(&s.metadata, &source.metadata()?) {
                        return Err(invalid("source changed while opening backup"));
                    }
                    let mut dest = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)?;
                    io::copy(&mut source, &mut dest)?;
                    dest.set_permissions(s.metadata.permissions())?;
                    dest.sync_all()?;
                    let copy = analyze(&target)?;
                    if copy.sha256 != s.info.sha256 || copy.size != s.info.size {
                        return Err(invalid(format!(
                            "backup verification failed: {}",
                            target.display()
                        )));
                    }
                }
                if matches!(decision, Decision::KeepBoth) {
                    validate_after_link(s)?;
                } else {
                    validate(s)?;
                }
                fs::remove_file(&s.info.path)?;
            }
            operations.push((s.info.path.clone(), target));
            if !dry_run {
                after_operation(operations.len())?;
            }
        }
        if let Some(index) = chosen {
            let s = &self.files[index];
            if !dry_run {
                validate(s)?;
                fs::hard_link(&s.info.path, &self.base)?;
                validate_after_link(s)?;
                fs::remove_file(&s.info.path)?;
            }
            operations.push((s.info.path.clone(), self.base.clone()));
            if !dry_run {
                after_operation(operations.len())?;
            }
        }
        if let Some(file) = &mut log_file {
            log(
                file,
                decision,
                &self.base,
                chosen.map(|i| self.files[i].info.path.as_path()),
                &operations,
            )?;
        }
        Ok(Outcome {
            operations,
            dry_run,
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::analysis::analyze_scanned_group;
    use crate::config::ScanConfig;
    use crate::patterns::Patterns;
    use crate::scan::scan;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "deconflict-resolve-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::create_dir(path.join("root")).unwrap();
            fs::write(path.join("root/file"), b"base").unwrap();
            fs::write(path.join("root/file.pacnew"), b"copy").unwrap();
            Self(path)
        }
        fn prepared(&self) -> Prepared {
            let config = ScanConfig::default();
            let root = self.0.join("root");
            let scan = scan(
                std::slice::from_ref(&root),
                &config,
                &Patterns::build(&config).unwrap(),
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            let analysis = analyze_scanned_group(&scan.groups[0]).unwrap().analysis;
            prepare(&scan, 0, &analysis, &[root]).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn keep_copy_backs_up_base_and_logs() {
        let f = Fixture::new();
        let result = f
            .prepared()
            .resolve(Decision::KeepCopy(0), &f.0.join("backup"), false)
            .unwrap();
        assert_eq!(fs::read(f.0.join("root/file")).unwrap(), b"copy");
        assert!(!f.0.join("root/file.pacnew").exists());
        assert_eq!(fs::read(&result.operations[0].1).unwrap(), b"base");
        let log = fs::read_to_string(f.0.join("backup/decision-log.jsonl")).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert!(log.contains("\"group\":\"file\""));
        assert!(log.contains("\"action\":\"keep_copy\""));
        assert!(log.contains("\"decisions\":[{"));
        assert!(log.contains("\"renamed\":{"));
        assert!(!log.contains("\"phase\""));
    }

    #[test]
    fn dry_run_and_root_protection() {
        let f = Fixture::new();
        let p = f.prepared();
        assert!(p
            .resolve(Decision::KeepBase, &f.0.join("root/backup"), false)
            .is_err());
        let out = p
            .resolve(Decision::KeepBase, &f.0.join("backup"), true)
            .unwrap();
        assert!(out.dry_run);
        assert!(!f.0.join("backup").exists());
        assert_eq!(fs::read(f.0.join("root/file.pacnew")).unwrap(), b"copy");
    }

    #[test]
    fn log_contains_python_decisions_for_each_action() {
        for decision in [
            Decision::KeepBase,
            Decision::KeepCopy(0),
            Decision::KeepBoth,
        ] {
            let f = Fixture::new();
            let result = f
                .prepared()
                .resolve(decision, &f.0.join("backup"), false)
                .unwrap();
            let text = fs::read_to_string(f.0.join("backup/decision-log.jsonl")).unwrap();
            let record: serde_json::Value = serde_json::from_str(&text).unwrap();
            let action = match decision {
                Decision::KeepBase => "keep_base",
                Decision::KeepCopy(_) => "keep_copy",
                Decision::KeepBoth => "keep_both",
            };
            assert!(text.contains(&format!("\"action\":{}", json_string(action))));
            assert!(record["ts"].is_number());
            assert_eq!(record["group"], "file");
            assert_eq!(record["action"], action);
            assert_eq!(
                record["decisions"].as_array().unwrap().len(),
                result.operations.len()
            );
            assert!(text.contains(&format!(
                "\"base\":{}",
                json_string(&f.0.join("root/file").to_string_lossy())
            )));
            for (i, (_, target)) in result.operations.iter().enumerate() {
                let kind = if matches!(decision, Decision::KeepBoth)
                    || matches!(decision, Decision::KeepCopy(_)) && i + 1 == result.operations.len()
                {
                    "rename"
                } else {
                    "move"
                };
                assert!(text.contains(&format!(
                    "{{\"action\":\"{kind}\",\"moved_to\":{},\"detail\":\"\"}}",
                    json_string(&target.to_string_lossy())
                )));
                assert_eq!(record["decisions"][i]["action"], kind);
                assert_eq!(
                    record["decisions"][i]["moved_to"],
                    target.to_string_lossy().as_ref()
                );
                assert_eq!(record["decisions"][i]["detail"], "");
            }
            if matches!(decision, Decision::KeepCopy(_)) {
                assert_eq!(
                    record["copy"],
                    f.0.join("root/file.pacnew").to_string_lossy().as_ref()
                );
                assert_eq!(
                    record["renamed"],
                    record["decisions"][result.operations.len() - 1]
                );
            }
            assert_eq!(
                text.contains("\"copy\":"),
                matches!(decision, Decision::KeepCopy(_))
            );
            assert_eq!(text.lines().count(), 1);
        }
    }

    #[test]
    fn partial_operation_has_no_success_record() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        let result = f
            .prepared()
            .resolve_inner(Decision::KeepCopy(0), &backup, false, |_| {
                Err(io::Error::other("injected failure"))
            });
        assert!(result.is_err());
        assert_eq!(fs::read(backup.join("file/file")).unwrap(), b"base");
        assert!(!f.0.join("root/file").exists());
        assert!(f.0.join("root/file.pacnew").exists());
        assert_eq!(fs::read(backup.join("decision-log.jsonl")).unwrap(), b"");
    }

    #[test]
    fn incomplete_existing_record_blocks_mutations() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("decision-log.jsonl"), b"{\"action\":").unwrap();
        assert!(f
            .prepared()
            .resolve(Decision::KeepBase, &backup, false)
            .is_err());
        assert!(f.0.join("root/file.pacnew").exists());
        assert_eq!(
            fs::read(backup.join("decision-log.jsonl")).unwrap(),
            b"{\"action\":"
        );
    }

    #[test]
    fn dangling_backup_symlink_is_rejected_even_in_preview() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        symlink(f.0.join("root/missing"), f.0.join("backup")).unwrap();
        assert!(f
            .prepared()
            .resolve(Decision::KeepBase, &f.0.join("backup"), true)
            .is_err());
    }

    #[test]
    fn log_symlink_is_rejected_without_touching_sources() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        let backup = f.0.join("backup");
        fs::create_dir(&backup).unwrap();
        symlink(f.0.join("root/file"), backup.join("decision-log.jsonl")).unwrap();
        assert!(f
            .prepared()
            .resolve(Decision::KeepBase, &backup, false)
            .is_err());
        assert_eq!(fs::read(f.0.join("root/file")).unwrap(), b"base");
        assert_eq!(fs::read(f.0.join("root/file.pacnew")).unwrap(), b"copy");
    }

    #[test]
    fn existing_backup_is_never_overwritten() {
        let f = Fixture::new();
        let backup = f.0.join("backup");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("backup"), b"existing").unwrap();
        let out = f
            .prepared()
            .resolve(Decision::KeepBase, &backup, false)
            .unwrap();
        assert_eq!(fs::read(backup.join("backup")).unwrap(), b"existing");
        assert_eq!(fs::read(&out.operations[0].1).unwrap(), b"copy");
        assert_ne!(out.operations[0].1, backup.join("backup"));
    }

    #[test]
    fn preview_reserves_distinct_backups_and_failed_log_preserves_sources() {
        let f = Fixture::new();
        fs::write(f.0.join("root/file.pacsave"), b"other").unwrap();
        let p = f.prepared();
        let backup = f.0.join("backup");
        let preview = p.resolve(Decision::KeepBase, &backup, true).unwrap();
        assert_eq!(preview.operations.len(), 2);
        assert_ne!(preview.operations[0].1, preview.operations[1].1);
        fs::create_dir(&backup).unwrap();
        fs::create_dir(backup.join("decision-log.jsonl")).unwrap();
        assert!(p.resolve(Decision::KeepBase, &backup, false).is_err());
        assert!(f.0.join("root/file.pacnew").exists());
        assert!(f.0.join("root/file.pacsave").exists());
    }

    #[test]
    fn keep_both_and_changed_file_rejected() {
        let f = Fixture::new();
        let p = f.prepared();
        fs::write(f.0.join("root/file.pacnew"), b"changed").unwrap();
        assert!(p
            .resolve(Decision::KeepBoth, &f.0.join("backup"), false)
            .is_err());
        assert!(!f.0.join("backup").exists());
        let p = f.prepared();
        let out = p
            .resolve(Decision::KeepBoth, &f.0.join("backup"), false)
            .unwrap();
        assert_eq!(fs::read(&out.operations[0].1).unwrap(), b"changed");
        assert_eq!(fs::read(f.0.join("root/file")).unwrap(), b"base");
    }

    #[test]
    fn incomplete_scan_is_rejected() {
        let f = Fixture::new();
        let config = ScanConfig::default();
        let root = f.0.join("root");
        let mut scan = scan(
            std::slice::from_ref(&root),
            &config,
            &Patterns::build(&config).unwrap(),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let analysis = analyze_scanned_group(&scan.groups[0]).unwrap().analysis;
        scan.cancelled = true;
        assert!(prepare(&scan, 0, &analysis, std::slice::from_ref(&root)).is_err());
        scan.cancelled = false;
        scan.errors.push((root.clone(), "error".into()));
        assert!(prepare(&scan, 0, &analysis, &[root]).is_err());
    }
}
