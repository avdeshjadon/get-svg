//! Local response cache: one JSON file per entry, TTL- and size-bounded.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::Settings;
use crate::error::Result;

/// Namespace for search pages so cache keys never collide across features.
pub const NS_SEARCH: &str = "search";
pub const NS_ASSET: &str = "asset";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStatus {
    pub directory: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
    pub enabled: bool,
    pub ttl_hours: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
    ttl: Duration,
    enabled: bool,
    max_bytes: u64,
}

impl Cache {
    pub fn new(settings: &Settings) -> Cache {
        Cache {
            dir: settings.cache_dir(),
            ttl: Duration::from_secs(settings.cache_ttl_hours.saturating_mul(3600)),
            enabled: settings.cache_enabled,
            max_bytes: settings.cache_max_mb.saturating_mul(1024 * 1024),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    fn key(ns: &str, source: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(ns.as_bytes());
        hasher.update(b"\0");
        hasher.update(source.as_bytes());
        format!("{ns}-{:x}.json", hasher.finalize())
    }

    fn path_for(&self, ns: &str, source: &str) -> PathBuf {
        self.dir.join(Cache::key(ns, source))
    }

    /// Read a cache entry. Expired or corrupt entries are treated as a miss
    /// (and lazily removed), never as an error.
    pub fn get<T: DeserializeOwned>(&self, ns: &str, source: &str) -> Option<T> {
        if !self.enabled {
            return None;
        }
        let path = self.path_for(ns, source);
        let meta = fs::metadata(&path).ok()?;
        let age = meta.modified().ok()?.elapsed().unwrap_or(Duration::MAX);
        if age > self.ttl {
            let _ = fs::remove_file(&path);
            return None;
        }
        let raw = fs::read(&path).ok()?;
        if raw.len() > self.max_bytes as usize {
            let _ = fs::remove_file(&path);
            return None;
        }
        serde_json::from_slice(&raw).ok()
    }

    /// Write a cache entry, then enforce the disk budget.
    pub fn put<T: Serialize>(&self, ns: &str, source: &str, value: &T) {
        if !self.enabled {
            return;
        }
        let Ok(raw) = serde_json::to_vec(value) else {
            return;
        };
        if raw.len() as u64 > self.max_bytes {
            return;
        }
        if fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let path = self.path_for(ns, source);
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, &raw).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
        self.prune();
    }

    /// Evict oldest entries until the cache fits in its byte budget.
    pub fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
        let mut total = 0u64;
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            total += meta.len();
            files.push((mtime, e.path(), meta.len()));
        }
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|(mtime, _, _)| *mtime); // oldest first
        for (_, path, len) in files {
            if total <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    /// Remove every cache entry.
    pub fn clear(&self) -> Result<usize> {
        let mut removed = 0usize;
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) == Some("json")
                    && fs::remove_file(&path).is_ok()
                {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// Snapshot of disk usage, for `get-svg cache status`.
    pub fn status(&self) -> CacheStatus {
        let mut file_count = 0usize;
        let mut total_bytes = 0u64;
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                if let Ok(meta) = e.metadata() {
                    if meta.is_file()
                        && e.path().extension().and_then(|x| x.to_str()) == Some("json")
                    {
                        file_count += 1;
                        total_bytes += meta.len();
                    }
                }
            }
        }
        CacheStatus {
            directory: self.dir.clone(),
            file_count,
            total_bytes,
            enabled: self.enabled,
            ttl_hours: self.ttl.as_secs() / 3600,
            max_bytes: self.max_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn test_cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            config_dir: dir.path().to_path_buf(),
            cache_enabled: true,
            cache_ttl_hours: 24,
            cache_max_mb: 1,
            ..Settings::default()
        };
        (dir, Cache::new(&s))
    }

    #[test]
    fn roundtrip() {
        let (_d, cache) = test_cache();
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct V(String);
        assert_eq!(cache.get::<V>(NS_SEARCH, "github:0"), None);
        cache.put(NS_SEARCH, "github:0", &V("ok".into()));
        assert_eq!(cache.get::<V>(NS_SEARCH, "github:0"), Some(V("ok".into())));
        assert_eq!(cache.get::<V>(NS_SEARCH, "other:0"), None);
    }

    #[test]
    fn expired_entry_is_a_miss() {
        let (_d, cache) = test_cache();
        #[derive(serde::Serialize, serde::Deserialize)]
        struct V(String);
        cache.put(NS_SEARCH, "q:0", &V("x".into()));
        let p = cache.path_for(NS_SEARCH, "q:0");
        // Backdate the file beyond the TTL.
        let old = SystemTime::now() - Duration::from_secs(25 * 3600);
        let f = fs::File::options().write(true).open(&p).unwrap();
        f.set_times(fs::FileTimes::new().set_modified(old)).unwrap();
        assert!(cache.get::<V>(NS_SEARCH, "q:0").is_none());
    }

    #[test]
    fn clear_removes_entries() {
        let (_d, cache) = test_cache();
        #[derive(serde::Serialize, serde::Deserialize)]
        struct V(String);
        cache.put(NS_SEARCH, "a:0", &V("x".into()));
        cache.put(NS_SEARCH, "b:0", &V("y".into()));
        assert_eq!(cache.status().file_count, 2);
        assert_eq!(cache.clear().unwrap(), 2);
        assert_eq!(cache.status().file_count, 0);
    }

    #[test]
    fn respects_disabled_cache() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            config_dir: dir.path().to_path_buf(),
            cache_enabled: false,
            ..Settings::default()
        };
        let cache = Cache::new(&s);
        #[derive(serde::Serialize, serde::Deserialize)]
        struct V(String);
        cache.put(NS_SEARCH, "q:0", &V("x".into()));
        assert!(cache.get::<V>(NS_SEARCH, "q:0").is_none());
        assert_eq!(cache.status().file_count, 0);
    }

    #[test]
    fn prune_enforces_budget() {
        let (_d, cache) = test_cache();
        #[derive(serde::Serialize, serde::Deserialize)]
        struct V(String);
        // 1 MB budget; write entries big enough to trip the pruner.
        for i in 0..30 {
            let payload = "x".repeat(80_000);
            cache.put(NS_SEARCH, &format!("q{i}:0"), &V(payload));
        }
        let status = cache.status();
        assert!(
            status.total_bytes <= cache.max_bytes,
            "{} > {}",
            status.total_bytes,
            cache.max_bytes
        );
    }
}
