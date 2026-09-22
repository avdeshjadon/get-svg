//! Single-file download: streaming to a temp file, then atomic rename.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::security;

/// Upper bound for a single downloaded file (4 GiB) — an SVG larger than
/// this is almost certainly a mislabeled response; refuse rather than fill
/// the user's disk.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// A pluggable progress callback, invoked with (bytes written, total bytes).
/// The total is 0 while it is unknown. Must be callable from spawned tasks.
pub type ProgressFn = Box<dyn FnMut(u64, u64) + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadOutcome {
    Written { path: PathBuf, bytes: u64 },
    Skipped,
}

/// Allocates collision-free file names inside a directory.
///
/// Handles: existing files, case-insensitive collisions, and names already
/// claimed by other files in the same batch.
#[derive(Debug, Default)]
pub struct NameAllocator {
    used: std::collections::HashSet<String>,
}

impl NameAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve `name` in `dest`. Returns the full path to write to, or
    /// `None` when the file exists and overwriting is disabled.
    pub fn allocate(&mut self, dest: &Path, name: &str, overwrite: bool) -> Option<PathBuf> {
        let base = security::sanitize_filename(name);
        let (stem, ext) = split_ext(&base);

        for attempt in 0..10_000u32 {
            let candidate = if attempt == 0 {
                base.clone()
            } else {
                format!("{stem}-{attempt}{ext}")
            };
            let key = security::casefold(&candidate);
            if self.used.contains(&key) {
                continue;
            }
            let path = match security::safe_join(dest, &candidate) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if path.exists() && !overwrite {
                // Claim it so a later duplicate does not loop over it too.
                self.used.insert(key);
                return None;
            }
            self.used.insert(key);
            return Some(path);
        }
        None
    }
}

fn split_ext(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// Resolve a unique, safe path for `name` in `dir` without allocating.
pub fn unique_path(dir: &Path, name: &str, overwrite: bool) -> Option<PathBuf> {
    NameAllocator::new().allocate(dir, name, overwrite)
}

/// Stream a URL to `path`.
///
/// * writes to `<path>.part-<pid>` first, then renames (atomic on POSIX),
/// * honors cancellation at every chunk,
/// * enforces a size ceiling and content-type sanity check,
/// * reports progress through `progress` when a total size is known.
pub async fn download_one(
    client: &Client,
    url: &str,
    path: &Path,
    cancel: &CancellationToken,
    mut progress: Option<ProgressFn>,
) -> Result<DownloadOutcome> {
    if !security::validate_https_url(url) {
        return Err(Error::InvalidUrl(url.to_string()));
    }
    if path.exists() {
        return Ok(DownloadOutcome::Skipped);
    }

    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        res = client.get(url).send() => res?,
    };

    if !response.status().is_success() {
        return Err(Error::Http {
            status: response.status().as_u16(),
            detail: "download request failed".to_string(),
        });
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Wikimedia serves SVGs as image/svg+xml or octet-stream; an HTML body
    // here means we landed on an error page — do not save it as an SVG.
    if content_type.contains("text/html") {
        return Err(Error::Download(format!(
            "unexpected content type from server: {content_type}"
        )));
    }

    let total = response.content_length().unwrap_or(0);
    if total > MAX_FILE_BYTES {
        return Err(Error::Download(format!(
            "file is larger than the {} byte limit",
            MAX_FILE_BYTES
        )));
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let tmp = temp_path_for(path);
    let write = write_stream(response, &tmp, total, cancel, &mut progress).await;

    match write {
        Ok(bytes) => {
            // Atomic publish: only fully-written files get their final name.
            match std::fs::rename(&tmp, path) {
                Ok(()) => Ok(DownloadOutcome::Written {
                    path: path.to_path_buf(),
                    bytes,
                }),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    Err(Error::Io(e))
                }
            }
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(format!(".part-{}", std::process::id()));
    path.with_file_name(name)
}

async fn write_stream(
    response: reqwest::Response,
    tmp: &Path,
    total: u64,
    cancel: &CancellationToken,
    progress: &mut Option<ProgressFn>,
) -> Result<u64> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(tmp).await?;
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();

    loop {
        let chunk = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            next = stream.next() => match next {
                None => break,
                Some(Err(e)) => return Err(Error::from(e)),
                Some(Ok(bytes)) => bytes,
            }
        };
        written += chunk.len() as u64;
        if written > MAX_FILE_BYTES {
            return Err(Error::Download(
                "file exceeded size limit mid-download".into(),
            ));
        }
        file.write_all(&chunk).await?;
        if let Some(cb) = progress.as_mut() {
            cb(written, total);
        }
    }

    file.flush().await?;
    file.sync_all().await?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_handles_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = NameAllocator::new();
        let p1 = a.allocate(dir.path(), "GitHub.svg", false).unwrap();
        assert!(p1.ends_with("GitHub.svg"));
        let p2 = a.allocate(dir.path(), "GitHub.svg", false).unwrap();
        assert!(p2.ends_with("GitHub-1.svg"), "{p2:?}");
        let p3 = a.allocate(dir.path(), "GitHub.svg", false).unwrap();
        assert!(p3.ends_with("GitHub-2.svg"), "{p3:?}");
    }

    #[test]
    fn allocator_skips_existing_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exists.svg"), "x").unwrap();
        let mut a = NameAllocator::new();
        assert!(a.allocate(dir.path(), "exists.svg", false).is_none());
        // With overwrite, the original name is returned.
        let mut a = NameAllocator::new();
        assert!(a.allocate(dir.path(), "exists.svg", true).is_some());
    }

    #[test]
    fn allocator_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = NameAllocator::new();
        let _ = a.allocate(dir.path(), "GitHub.svg", false);
        let p = a.allocate(dir.path(), "github.svg", false).unwrap();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert_ne!(
            name.to_lowercase(),
            "github.svg",
            "casefold collision not handled: {name}"
        );
    }

    #[test]
    fn allocator_survives_traversal_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = NameAllocator::new();
        let p = a.allocate(dir.path(), "../../evil.svg", false).unwrap();
        assert!(p.starts_with(dir.path()));
        assert!(!p.to_string_lossy().contains(".."));
    }

    #[test]
    fn rejects_non_https_urls() {
        assert!(!security::validate_https_url("http://x.test/a.svg"));
        assert!(!security::validate_https_url("ftp://x.test/a.svg"));
    }
}
