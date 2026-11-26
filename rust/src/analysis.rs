use crate::cache::FileCache;
use crate::media::{self, Tools as MediaTools};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub sha256: String,
    pub is_text: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupAnalysis {
    pub base: Option<FileInfo>,
    pub copies: Vec<FileInfo>,
    pub all_equal: bool,
    pub all_text: bool,
    pub kind: String,
    pub meta: Vec<Option<bool>>,
    pub meta_diff: Vec<Option<String>>,
    pub meta_fields: Vec<BTreeMap<String, String>>,
}

/// A content-based recommendation; callers must classify media separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recommendation {
    Base,
    Copy(usize),
}

/// Diagnostics and the conservative recommendation for a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consideration {
    pub notes: Vec<String>,
    pub recommendation: Option<Recommendation>,
}

/// Analysis with filename stamps aligned with `analysis.copies`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampedGroupAnalysis {
    pub analysis: GroupAnalysis,
    pub copy_stamps: Vec<Option<String>>,
}

/// Analyze a scanned group without losing the stamps captured by its patterns.
pub fn analyze_scanned_group(group: &crate::scan::Group) -> io::Result<StampedGroupAnalysis> {
    analyze_scanned_group_with_media(group, &MediaTools::default())
}

pub fn analyze_scanned_group_with_media(
    group: &crate::scan::Group,
    tools: &MediaTools,
) -> io::Result<StampedGroupAnalysis> {
    let paths: Vec<_> = group
        .conflicts
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    Ok(StampedGroupAnalysis {
        analysis: analyze_group_with_media(&group.base, &paths, tools)?,
        copy_stamps: group
            .conflicts
            .iter()
            .map(|(_, m)| m.stamp.clone())
            .collect(),
    })
}

pub fn analyze_scanned_group_cached(
    group: &crate::scan::Group,
    cache: &mut FileCache,
    refresh: bool,
    tools: &MediaTools,
) -> io::Result<StampedGroupAnalysis> {
    let paths: Vec<_> = group
        .conflicts
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    Ok(StampedGroupAnalysis {
        analysis: analyze_group_cached(&group.base, &paths, cache, refresh, tools)?,
        copy_stamps: group
            .conflicts
            .iter()
            .map(|(_, m)| m.stamp.clone())
            .collect(),
    })
}

impl GroupAnalysis {
    /// Python's mtime hint: the first item wins ties, even without a base.
    pub fn winner_hint(&self) -> Option<Recommendation> {
        let mut winner = self.base.as_ref().map(|info| (Recommendation::Base, info));
        for (index, info) in self.copies.iter().enumerate() {
            if winner
                .as_ref()
                .is_none_or(|(_, current)| info.modified > current.modified)
            {
                winner = Some((Recommendation::Copy(index), info));
            }
        }
        winner.map(|(which, _)| which)
    }
}

fn file_for<'a>(group: &'a GroupAnalysis, which: &Recommendation) -> Option<&'a FileInfo> {
    match which {
        Recommendation::Base => group.base.as_ref(),
        Recommendation::Copy(index) => group.copies.get(*index),
    }
}

fn tag(which: &Recommendation) -> String {
    match which {
        Recommendation::Base => "#0".into(),
        Recommendation::Copy(index) => format!("#{}", index + 1),
    }
}

fn human(info: &FileInfo) -> String {
    format!(
        "'{}'",
        info.path.file_name().unwrap_or_default().to_string_lossy()
    )
}

/// Human-readable label matching Python's recommendation labels.
pub fn recommendation_label(group: &GroupAnalysis, which: &Recommendation) -> Option<String> {
    let info = file_for(group, which)?;
    if *which == Recommendation::Base && group.all_equal {
        return Some("#0 base (all files identical)".into());
    }
    let base = if *which == Recommendation::Base {
        " base"
    } else {
        ""
    };
    Some(format!("{}{base} {} (newest)", tag(which), human(info)))
}

/// Explain checks even when they do not produce a recommendation.
pub fn consider(group: &GroupAnalysis, is_text_kind: bool) -> Consideration {
    let Some(base) = &group.base else {
        return Consideration {
            notes: vec!["no base file — nothing to recommend".into()],
            recommendation: None,
        };
    };
    let newest_id = group.winner_hint().expect("base exists");
    let newest = file_for(group, &newest_id).expect("winner belongs to group");
    let mut largest_id = Recommendation::Base;
    let mut largest = base;
    for (index, copy) in group.copies.iter().enumerate() {
        if copy.size > largest.size {
            largest = copy;
            largest_id = Recommendation::Copy(index);
        }
    }
    let seconds = match newest.modified.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    };
    let mut notes = vec![
        format!(
            "newest is {} {} ({})",
            tag(&newest_id),
            human(newest),
            seconds.round_ties_even()
        ),
        format!(
            "largest is {} {} ({} B; base is {} B)",
            tag(&largest_id),
            human(largest),
            largest.size,
            base.size
        ),
    ];
    if group.all_equal {
        notes.push("all files identical in content".into());
        return Consideration {
            notes,
            recommendation: Some(Recommendation::Base),
        };
    }
    if newest_id != Recommendation::Base {
        let tag = tag(&newest_id);
        if is_text_kind {
            if text_superset(base, newest).unwrap_or(false) {
                notes.push(format!(
                    "{tag} text is a superset of base (content added, nothing removed)"
                ));
            } else {
                notes.push(format!("{tag} text is NOT a superset of base (content changed/removed) — no recommendation"));
            }
        } else if newest.size <= base.size {
            notes.push(format!("{tag} is newer but smaller than base — possible content/metadata loss, no recommendation"));
        } else {
            notes.push(format!("{tag} is newer and larger than base"));
        }
    }
    let recommendation = recommend(group, is_text_kind);
    if let Some(which) = &recommendation {
        notes.push(format!(
            "recommend keep {}",
            recommendation_label(group, which).expect("winner belongs to group")
        ));
    } else {
        notes.push("no clear recommendation (no single file wins on all checks)".into());
    }
    Consideration {
        notes,
        recommendation,
    }
}

/// Apply the Python recommendation rules. `is_text_kind` is the group kind,
/// not merely the result of NUL-byte sniffing (media classification is separate).
pub fn recommend(group: &GroupAnalysis, is_text_kind: bool) -> Option<Recommendation> {
    let base = group.base.as_ref()?;
    if group.all_equal {
        return Some(Recommendation::Base);
    }
    let mut newest = base;
    let mut winner = Recommendation::Base;
    for (index, copy) in group.copies.iter().enumerate() {
        if copy.modified > newest.modified {
            newest = copy;
            winner = Recommendation::Copy(index);
        }
    }
    if base.iter_same_time(newest, &group.copies) {
        return None;
    }
    if winner == Recommendation::Base {
        return Some(winner);
    }
    if is_text_kind {
        if !text_superset(base, newest).unwrap_or(false) {
            return None;
        }
    } else if newest.size <= base.size {
        return None;
    }
    Some(winner)
}

impl FileInfo {
    fn iter_same_time(&self, newest: &FileInfo, copies: &[FileInfo]) -> bool {
        std::iter::once(self)
            .chain(copies)
            .filter(|item| item.modified == newest.modified)
            .count()
            != 1
    }
}

fn text_superset(base: &FileInfo, newest: &FileInfo) -> io::Result<bool> {
    fn lines(info: &FileInfo) -> io::Result<HashSet<String>> {
        let bytes = std::fs::read(&info.path)?;
        // Refuse to recommend from content that no longer matches the analyzed snapshot.
        if format!("{:x}", Sha256::digest(&bytes)) != info.sha256 {
            return Err(io::Error::other("file changed during recommendation"));
        }
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .split([
                '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
                '\u{2029}',
            ])
            .map(|line| {
                line.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
            })
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }
    let a = lines(base)?;
    let b = lines(newest)?;
    Ok(a != b && a.is_subset(&b))
}

/// Compare file contents and metadata using the default (no external tools) extractors.
pub fn analyze_group(base: &Path, copies: &[PathBuf]) -> io::Result<GroupAnalysis> {
    analyze_group_with_media(base, copies, &MediaTools::default())
}

pub fn analyze_group_with_media(
    base: &Path,
    copies: &[PathBuf],
    tools: &MediaTools,
) -> io::Result<GroupAnalysis> {
    let base = match File::open(base) {
        Ok(file) => Some(analyze_open_file(base, file)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    analyze_infos(base, copies, tools)
}

pub fn analyze_group_cached(
    base: &Path,
    copies: &[PathBuf],
    cache: &mut FileCache,
    refresh: bool,
    tools: &MediaTools,
) -> io::Result<GroupAnalysis> {
    let base = match base.exists() {
        true => Some(analyze_cached(base, cache, refresh)?),
        false => None,
    };
    let mut copies_info = Vec::with_capacity(copies.len());
    for copy in copies {
        copies_info.push(analyze_cached(copy, cache, refresh)?);
    }
    assemble_group(base, copies_info, tools)
}

fn analyze_infos(
    base: Option<FileInfo>,
    copies: &[PathBuf],
    tools: &MediaTools,
) -> io::Result<GroupAnalysis> {
    let mut copies_info = Vec::with_capacity(copies.len());
    for copy in copies {
        copies_info.push(analyze(copy)?);
    }
    assemble_group(base, copies_info, tools)
}

fn analyze_cached(path: &Path, cache: &mut FileCache, refresh: bool) -> io::Result<FileInfo> {
    let meta = path.metadata()?;
    let size = meta.len();
    let mtime = meta
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_else(|e| -e.duration().as_secs_f64());

    if !refresh {
        if let Some(entry) = cache.get(path, size, mtime) {
            if let (Some(sha), Some(is_text)) = (
                entry.get("sha").and_then(serde_json::Value::as_str),
                entry.get("is_text").and_then(serde_json::Value::as_bool),
            ) {
                return Ok(FileInfo {
                    path: path.into(),
                    size,
                    modified: meta.modified()?,
                    sha256: sha.to_owned(),
                    is_text,
                });
            }
        }
    }

    let info = analyze(path)?;
    cache.put(path, info.size, mtime, &info.sha256, info.is_text)?;
    Ok(info)
}

fn generic_fields(path: &Path, kind: &str, tools: &MediaTools) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    if let Ok(meta) = path.metadata() {
        fields.insert("size".into(), meta.len().to_string());
        if let Ok(modified) = meta.modified() {
            let seconds = modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or_else(|e| -e.duration().as_secs_f64());
            fields.insert("modified".into(), format!("{seconds:.6}"));
            fields.insert("created".into(), format!("{seconds:.6}"));
        }
    }
    if let Some(content) = media::content_fields(path, kind, tools) {
        fields.extend(content);
    }
    fields
}

fn assemble_group(
    base: Option<FileInfo>,
    copies: Vec<FileInfo>,
    tools: &MediaTools,
) -> io::Result<GroupAnalysis> {
    let mut items = base.iter().chain(copies.iter());
    let first = items.next();
    let all_equal = first.is_some_and(|first| items.all(|item| item.sha256 == first.sha256));
    let all_text = base.iter().chain(copies.iter()).all(|item| item.is_text);

    let representative = base
        .as_ref()
        .map(|item| &item.path)
        .or_else(|| copies.first().map(|item| &item.path));
    let kind = representative
        .map(|path| {
            let is_text = base
                .as_ref()
                .map(|item| item.is_text)
                .or_else(|| copies.first().map(|item| item.is_text));
            media::file_kind(path, is_text).to_owned()
        })
        .unwrap_or_else(|| "text".into());

    let mut meta_fields = Vec::with_capacity(base.iter().len() + copies.len());
    if let Some(base_info) = &base {
        meta_fields.push(generic_fields(&base_info.path, &kind, tools));
    }
    for copy in &copies {
        meta_fields.push(generic_fields(&copy.path, &kind, tools));
    }

    let mut meta = Vec::with_capacity(copies.len());
    let mut meta_diff = Vec::with_capacity(copies.len());
    if let Some(base_info) = &base {
        for copy in &copies {
            meta.push(media::metadata_equal(
                &base_info.path,
                &copy.path,
                &kind,
                tools,
            ));
            meta_diff.push(media::metadata_diff(
                &base_info.path,
                &copy.path,
                &kind,
                tools,
            ));
        }
    }

    Ok(GroupAnalysis {
        base,
        copies,
        all_equal,
        all_text,
        kind,
        meta,
        meta_diff,
        meta_fields,
    })
}

fn validate_snapshot(
    path: &Path,
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
) -> io::Result<()> {
    let current = std::fs::metadata(path)?;
    let changed = before.len() != after.len()
        || before.modified()? != after.modified()?
        || after.len() != current.len()
        || after.modified()? != current.modified()?;
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        changed
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || after.ctime() != current.ctime()
            || after.ctime_nsec() != current.ctime_nsec()
            || after.dev() != current.dev()
            || after.ino() != current.ino()
    };
    if changed {
        return Err(io::Error::other("file changed during analysis"));
    }
    Ok(())
}

pub fn analyze(path: &Path) -> io::Result<FileInfo> {
    analyze_open_file(path, File::open(path)?)
}

fn analyze_open_file(path: &Path, mut file: File) -> io::Result<FileInfo> {
    let before = file.metadata()?;
    let mut hash = Sha256::new();
    let mut buf = [0; 65536];
    let mut head_left = 8192usize;
    let mut is_text = true;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let head = n.min(head_left);
        is_text &= !buf[..head].contains(&0);
        head_left -= head;
        hash.update(&buf[..n]);
    }
    let after = file.metadata()?;
    validate_snapshot(path, &before, &after)?;
    Ok(FileInfo {
        path: path.into(),
        size: after.len(),
        modified: after.modified()?,
        sha256: format!("{:x}", hash.finalize()),
        is_text,
    })
}

#[cfg(test)]
mod recommendation_tests {
    use super::*;
    use std::time::Duration;

    #[cfg(unix)]
    #[test]
    fn snapshot_rejects_rewrite_with_restored_mtime() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"old").unwrap();
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let before = file.metadata().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        file.write_all(b"new").unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
            .unwrap();
        let after = file.metadata().unwrap();
        assert!(validate_snapshot(&path, &before, &after).is_err());
    }

    #[test]
    fn consideration_explains_recommendations_and_ties() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let copy = dir.path().join("copy");
        std::fs::write(&base, b"one\n").unwrap();
        std::fs::write(&copy, b"one\ntwo\n").unwrap();
        let mut group = analyze_group(&base, &[copy]).unwrap();
        group.base.as_mut().unwrap().modified = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        group.copies[0].modified = SystemTime::UNIX_EPOCH + Duration::from_secs(11);
        let result = consider(&group, true);
        assert_eq!(result.recommendation, Some(Recommendation::Copy(0)));
        assert_eq!(
            result.notes,
            [
                "newest is #1 'copy' (11)",
                "largest is #1 'copy' (8 B; base is 4 B)",
                "#1 text is a superset of base (content added, nothing removed)",
                "recommend keep #1 'copy' (newest)",
            ]
        );
        group.copies[0].modified = group.base.as_ref().unwrap().modified;
        assert_eq!(group.winner_hint(), Some(Recommendation::Base));
        assert_eq!(consider(&group, true).recommendation, None);
        group.base = None;
        assert_eq!(group.winner_hint(), Some(Recommendation::Copy(0)));
        assert_eq!(
            consider(&group, true).notes,
            ["no base file — nothing to recommend"]
        );
        group.copies.clear();
        assert_eq!(group.winner_hint(), None);
    }

    #[test]
    fn consideration_reports_identical_and_non_text_cases() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let copy = dir.path().join("copy");
        std::fs::write(&base, b"abc").unwrap();
        std::fs::write(&copy, b"abc").unwrap();
        let mut group = analyze_group(&base, std::slice::from_ref(&copy)).unwrap();
        let result = consider(&group, false);
        assert_eq!(
            result.notes.last().unwrap(),
            "all files identical in content"
        );
        assert_eq!(
            recommendation_label(&group, &Recommendation::Base).unwrap(),
            "#0 base (all files identical)"
        );
        std::fs::write(&copy, b"xy").unwrap();
        let mut changed = analyze_group(&base, &[copy]).unwrap();
        changed.copies[0].modified =
            changed.base.as_ref().unwrap().modified + Duration::from_secs(1);
        let result = consider(&changed, false);
        assert!(result.notes[2].contains("possible content/metadata loss"));
        assert_eq!(result.recommendation, None);
        group.copies.clear();
        assert_eq!(recommendation_label(&group, &Recommendation::Copy(0)), None);
    }

    #[test]
    fn scanned_group_preserves_optional_stamps_in_copy_order() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let one = dir.path().join("one");
        let two = dir.path().join("two");
        std::fs::write(&one, b"one").unwrap();
        std::fs::write(&two, b"two").unwrap();
        let matched = |stamp: Option<&str>| crate::patterns::ConflictMatch {
            pattern: "test".into(),
            base_name: "base".into(),
            winner: "copy".into(),
            raw: "copy".into(),
            stamp: stamp.map(str::to_owned),
        };
        let group = crate::scan::Group {
            base,
            conflicts: vec![
                (one.clone(), matched(Some("2024"))),
                (two.clone(), matched(None)),
            ],
        };
        let result = analyze_scanned_group(&group).unwrap();
        assert!(result.analysis.base.is_none());
        assert_eq!(
            result
                .analysis
                .copies
                .iter()
                .map(|c| &c.path)
                .collect::<Vec<_>>(),
            vec![&one, &two]
        );
        assert_eq!(result.copy_stamps, vec![Some("2024".into()), None]);
    }

    #[test]
    fn text_superset_uses_python_whitespace_rules() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let copy = dir.path().join("copy");
        std::fs::write(&base, b"\x1fone\x1f\n").unwrap();
        std::fs::write(&copy, b"one\ntwo\n").unwrap();
        let mut group = analyze_group(&base, &[copy]).unwrap();
        group.copies[0].modified = group.base.as_ref().unwrap().modified + Duration::from_secs(1);
        assert_eq!(recommend(&group, true), Some(Recommendation::Copy(0)));
    }

    #[test]
    fn analysis_hashes_all_bytes_but_sniffs_only_first_8192() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        let mut bytes = vec![b'a'; 8193];
        bytes[8192] = 0;
        std::fs::write(&path, &bytes).unwrap();
        let info = analyze(&path).unwrap();
        assert!(info.is_text);
        assert_eq!(info.size, 8193);
        assert_eq!(info.sha256, format!("{:x}", Sha256::digest(&bytes)));
        bytes[8191] = 0;
        std::fs::write(&path, &bytes).unwrap();
        assert!(!analyze(&path).unwrap().is_text);
    }

    #[test]
    fn tied_newest_copies_do_not_produce_a_recommendation() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::write(&base, b"one\n").unwrap();
        std::fs::write(&first, b"one\ntwo\n").unwrap();
        std::fs::write(&second, b"one\nthree\n").unwrap();
        let mut group = analyze_group(&base, &[first, second]).unwrap();
        let newest = group.base.as_ref().unwrap().modified + Duration::from_secs(1);
        group.copies[0].modified = newest;
        group.copies[1].modified = newest;
        assert_eq!(recommend(&group, true), None);
        group.copies[1].modified += Duration::from_secs(1);
        assert_eq!(recommend(&group, true), Some(Recommendation::Copy(1)));
    }

    #[test]
    fn recommendations_require_a_unique_winner_and_no_text_loss() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let copy = dir.path().join("copy");
        std::fs::write(&base, b"one\n").unwrap();
        std::fs::write(&copy, b"one\ntwo\n").unwrap();
        let mut group = analyze_group(&base, std::slice::from_ref(&copy)).unwrap();
        group.copies[0].modified = group.base.as_ref().unwrap().modified + Duration::from_secs(1);
        assert_eq!(recommend(&group, true), Some(Recommendation::Copy(0)));
        group.copies[0].modified = group.base.as_ref().unwrap().modified;
        assert_eq!(recommend(&group, true), None);
        group.copies[0].modified += Duration::from_secs(1);
        std::fs::write(&copy, b"replacement\n").unwrap();
        assert_eq!(recommend(&group, true), None);
        let mut group = analyze_group(&base, std::slice::from_ref(&copy)).unwrap();
        group.copies[0].modified = group.base.as_ref().unwrap().modified + Duration::from_secs(1);
        assert_eq!(recommend(&group, true), None);
        assert_eq!(recommend(&group, false), Some(Recommendation::Copy(0)));
        group.copies[0].size = group.base.as_ref().unwrap().size;
        assert_eq!(recommend(&group, false), None);
    }

    #[test]
    fn identical_and_missing_base_cases() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let copy = dir.path().join("copy");
        std::fs::write(&base, b"same").unwrap();
        std::fs::write(&copy, b"same").unwrap();
        let group = analyze_group(&base, std::slice::from_ref(&copy)).unwrap();
        assert_eq!(recommend(&group, true), Some(Recommendation::Base));
        std::fs::remove_file(base.clone()).unwrap();
        let group = analyze_group(&base, &[copy]).unwrap();
        assert_eq!(recommend(&group, true), None);
    }

    #[test]
    fn newest_base_and_blank_text_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let copy = dir.path().join("copy");
        std::fs::write(&base, b"  \n").unwrap();
        std::fs::write(&copy, b"new content\n").unwrap();
        let mut group = analyze_group(&base, &[copy]).unwrap();
        let original = group.base.as_ref().unwrap().modified;
        group.copies[0].modified = original + Duration::from_secs(1);
        assert_eq!(recommend(&group, true), Some(Recommendation::Copy(0)));
        group.base.as_mut().unwrap().modified = original + Duration::from_secs(2);
        assert_eq!(recommend(&group, true), Some(Recommendation::Base));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn disappearing_base_during_analysis_is_an_error() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("base");
        assert!(std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        std::thread::scope(|scope| {
            let analysis = scope.spawn(|| analyze_group(&path, &[]));
            let mut writer = File::create(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            writer.write_all(b"content").unwrap();
            drop(writer);
            assert_eq!(
                analysis.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
        });
    }

    #[test]
    fn detects_path_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, b"old").unwrap();
        let file = File::open(&path).unwrap();
        let before = file.metadata().unwrap();
        std::fs::rename(&path, dir.path().join("old")).unwrap();
        std::fs::write(&path, b"new").unwrap();
        assert!(validate_snapshot(&path, &before, &file.metadata().unwrap()).is_err());
    }
}
