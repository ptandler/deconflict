//! Python-compatible file metadata cache.
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Map;
pub use serde_json::Value as Json;

/// A lazy cache. Dry-run allows in-memory updates but never writes or deletes files.
pub struct FileCache {
    pub path: PathBuf,
    data: Map<String, Json>,
    dirty: bool,
    dry_run: bool,
}
impl FileCache {
    pub fn new(cache_dir: impl AsRef<Path>, key: &str, dry_run: bool) -> Self {
        Self {
            path: cache_dir.as_ref().join(format!("{key}.json")),
            data: Map::new(),
            dirty: false,
            dry_run,
        }
    }
    fn load(&mut self) {
        if self.data.is_empty() {
            if let Some(Json::Object(data)) = fs::read_to_string(&self.path)
                .ok()
                .and_then(|s| serde_json::from_str::<Json>(&s).ok())
            {
                self.data = data;
            }
        }
    }
    pub fn get(&mut self, path: &Path, size: u64, mtime: f64) -> Option<&Map<String, Json>> {
        self.load();
        let Json::Object(entry) = self.data.get(path.to_str()?)? else {
            return None;
        };
        let size_matches = entry.get("size").and_then(Json::as_u64) == Some(size)
            || entry
                .get("size")
                .filter(|v| v.is_f64())
                .and_then(Json::as_f64)
                .is_some_and(|s| s == size as f64 && (s as u128) == u128::from(size));
        let cached_mtime = entry.get("mtime")?.as_f64()?;
        (size_matches && (cached_mtime - mtime).abs() < 1e-6).then_some(entry)
    }
    pub fn put(
        &mut self,
        path: &Path,
        size: u64,
        mtime: f64,
        sha: &str,
        is_text: bool,
    ) -> io::Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
        if !mtime.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mtime is not finite",
            ));
        }
        self.load();
        self.data.insert(
            path.into(),
            Json::Object(Map::from_iter([
                ("size".into(), Json::from(size)),
                ("mtime".into(), Json::from(mtime)),
                ("sha".into(), Json::String(sha.into())),
                ("is_text".into(), Json::Bool(is_text)),
            ])),
        );
        self.dirty = true;
        Ok(())
    }
    pub fn save(&mut self) -> io::Result<()> {
        if !self.dirty || self.dry_run {
            return Ok(());
        }
        fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(&self.data)?)?;
        fs::rename(tmp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
    pub fn clear(&mut self) -> io::Result<()> {
        if !self.dry_run {
            for path in [&self.path, &self.count_path()] {
                match fs::remove_file(path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            }
        }
        self.data.clear();
        self.dirty = false;
        Ok(())
    }

    fn count_path(&self) -> PathBuf {
        self.path.with_extension("count")
    }

    pub fn file_count(&mut self) -> Option<u64> {
        fs::read_to_string(self.count_path())
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    pub fn save_file_count(&mut self, count: u64) -> io::Result<()> {
        if self.dry_run {
            return Ok(());
        }
        fs::create_dir_all(self.path.parent().unwrap_or(Path::new(".")))?;
        let path = self.count_path();
        let tmp = path.with_extension("count.tmp");
        fs::write(&tmp, count.to_string())?;
        fs::rename(tmp, path)
    }
}

/// Remove or report stale `.json` files, returning paths, reasons, and bytes.
pub fn clean_cache(
    dir: &Path,
    dry_run: bool,
    all: bool,
    older_than: Option<f64>,
) -> io::Result<(Vec<(PathBuf, String)>, u64)> {
    if !dir.is_dir() {
        return Ok((Vec::new(), 0));
    }
    let mut files: Vec<_> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let mut removed = Vec::new();
    let mut freed = 0;
    for path in files {
        let reason = if all {
            Some("explicit".to_string())
        } else if let Some(age) = older_than {
            let modified = fs::metadata(&path)?
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            (now - modified > age).then(|| format!("older than {age:.0}s"))
        } else {
            match fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<Json>(&s).ok())
            {
                Some(Json::Object(data)) => {
                    if data.keys().all(|p| {
                        !Path::new(p)
                            .parent()
                            .filter(|p| !p.as_os_str().is_empty())
                            .unwrap_or(Path::new("."))
                            .exists()
                    }) {
                        Some("scan dir no longer exists".into())
                    } else {
                        None
                    }
                }
                _ => Some("unreadable".into()),
            }
        };
        if let Some(reason) = reason {
            if dry_run {
                freed += fs::metadata(&path)?.len();
            } else if let Ok(meta) = fs::metadata(&path) {
                if fs::remove_file(&path).is_ok() {
                    freed += meta.len();
                }
            }
            removed.push((path, reason));
        }
    }
    Ok((removed, freed))
}

fn resolved(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => (),
            other => result.push(other.as_os_str()),
        }
        if let Ok(canonical) = fs::canonicalize(&result) {
            result = canonical;
        }
    }
    Ok(result)
}

/// First 16 hexadecimal digits of SHA-1 over ordered resolved paths joined by `_`.
pub fn cache_key<I, P>(dirs: I) -> io::Result<String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let paths = dirs
        .into_iter()
        .map(|p| {
            resolved(p.as_ref()).and_then(|p| {
                p.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut bytes = paths.join("_").into_bytes();
    let bits = (bytes.len() as u64).wrapping_mul(8);
    bytes.push(0x80);
    while bytes.len() % 64 != 56 {
        bytes.push(0);
    }
    bytes.extend_from_slice(&bits.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for chunk in bytes.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, c) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(c.try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a827999u32),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (v, n) in h.iter_mut().zip([a, b, c, d, e]) {
            *v = v.wrapping_add(n);
        }
    }
    Ok(format!("{:08x}{:08x}", h[0], h[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "deconflict-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn keys_and_json() {
        assert_eq!(
            cache_key(Vec::<PathBuf>::new()).unwrap(),
            "da39a3ee5e6b4b0d"
        );
        assert_eq!(cache_key([Path::new("/tmp")]).unwrap(), "8c393341f53683ee");
        let value: Json =
            serde_json::from_str(r#"{"\ud83d\ude00": [1, true, null, "é\n"]}"#).unwrap();
        assert_eq!(
            serde_json::from_str::<Json>(&serde_json::to_string(&value).unwrap()).unwrap(),
            value
        );
        assert!(serde_json::from_str::<Json>("{broken").is_err());
    }
    #[test]
    fn python_cache_unicode_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let script = r#"
import sys
from pathlib import Path
from deconflict.cache import FileCache
c = FileCache(sys.argv[1], 'python')
c.put(Path('/café/😀\nfile'), 9007199254740993, 1234.125, 'hash-é😀', True)
c.save()
"#;
        let status = Command::new("python3")
            .env("PYTHONPATH", project.join("src"))
            .args(["-c", script])
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let mut cache = FileCache::new(dir.path(), "python", false);
        let entry = cache
            .get(Path::new("/café/😀\nfile"), 9007199254740993, 1234.125)
            .unwrap();
        assert_eq!(entry["sha"], "hash-é😀");
        cache
            .put(Path::new("/日本/😺"), 42, 4.5, "rust-λ", false)
            .unwrap();
        cache.save().unwrap();
        let check = r#"
import sys
from pathlib import Path
from deconflict.cache import FileCache
c = FileCache(sys.argv[1], 'python')
assert c.get(Path('/café/😀\nfile'), 9007199254740993, 1234.125)['sha'] == 'hash-é😀'
assert c.get(Path('/日本/😺'), 42, 4.5)['sha'] == 'rust-λ'
"#;
        assert!(Command::new("python3")
            .env("PYTHONPATH", project.join("src"))
            .args(["-c", check])
            .arg(dir.path())
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn invalid_json_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        for (i, data) in [r#"{"x": 01}"#, r#"{"x": 1,}"#, r#"{"x": "\ud800"}"#]
            .iter()
            .enumerate()
        {
            fs::write(dir.path().join(format!("{i}.json")), data).unwrap();
        }
        let (removed, _) = clean_cache(dir.path(), true, false, None).unwrap();
        assert_eq!(removed.len(), 3);
        assert!(removed.iter().all(|(_, reason)| reason == "unreadable"));
    }
    #[test]
    fn file_cache_and_dry_run() {
        let dir = temp();
        let mut cache = FileCache::new(&dir, "one", false);
        fs::write(
            &cache.path,
            r#"{"/file": {"size": 12, "mtime": 1.5, "sha": "abc", "is_text": true, "extra": [1]}}"#,
        )
        .unwrap();
        assert!(cache
            .get(Path::new("/file"), 12, 1.5000005)
            .unwrap()
            .contains_key("extra"));
        assert!(cache.get(Path::new("/file"), 12, 1.500002).is_none());
        cache
            .put(Path::new("/other"), 3, 2.0, "hash", false)
            .unwrap();
        cache
            .put(
                Path::new("/large"),
                9_007_199_254_740_993,
                2.0,
                "hash",
                false,
            )
            .unwrap();
        cache.save().unwrap();
        let mut reloaded = FileCache::new(&dir, "one", false);
        assert!(reloaded
            .get(Path::new("/large"), 9_007_199_254_740_993, 2.0)
            .is_some());
        assert!(reloaded
            .get(Path::new("/large"), 9_007_199_254_740_992, 2.0)
            .is_none());
        assert!(FileCache::new(&dir, "one", false)
            .get(Path::new("/other"), 3, 2.0)
            .is_some());
        let before = fs::read(&cache.path).unwrap();
        let mut dry = FileCache::new(&dir, "one", true);
        dry.put(Path::new("/new"), 1, 1.0, "x", true).unwrap();
        dry.save().unwrap();
        dry.clear().unwrap();
        assert_eq!(fs::read(&cache.path).unwrap(), before);
        cache.clear().unwrap();
        assert!(!cache.path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_count_estimate_persists_and_clear_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = FileCache::new(dir.path(), "scan", false);
        assert_eq!(cache.file_count(), None);
        cache.save_file_count(5525).unwrap();

        let mut reloaded = FileCache::new(dir.path(), "scan", false);
        assert_eq!(reloaded.file_count(), Some(5525));
        reloaded.clear().unwrap();
        assert_eq!(FileCache::new(dir.path(), "scan", false).file_count(), None);
    }

    #[test]
    fn dry_run_does_not_write_file_count_estimate() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = FileCache::new(dir.path(), "scan", true);
        cache.save_file_count(5525).unwrap();
        assert!(!dir.path().join("scan.count").exists());
    }
    #[test]
    fn cleanup_modes() {
        let dir = temp();
        let live = dir.join("live");
        fs::create_dir(&live).unwrap();
        let cache = dir.join("cache");
        fs::create_dir(&cache).unwrap();
        fs::write(
            cache.join("live.json"),
            format!(
                "{{{}: {{}}}}",
                serde_json::to_string(&live.join("file").to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        fs::write(cache.join("dead.json"), "{}").unwrap();
        fs::write(cache.join("bad.json"), "broken").unwrap();
        fs::write(cache.join("pending.json.tmp"), "{}").unwrap();
        let (removed, bytes) = clean_cache(&cache, true, false, None).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(bytes > 0);
        assert!(cache.join("dead.json").exists());
        assert_eq!(clean_cache(&cache, false, false, None).unwrap().0.len(), 2);
        assert_eq!(
            clean_cache(&cache, false, false, Some(-1.0))
                .unwrap()
                .0
                .len(),
            1
        );
        assert!(cache.join("pending.json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
