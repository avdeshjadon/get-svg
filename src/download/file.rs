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
    #[cfg(test)]
    let is_valid_url = security::validate_https_url(url)
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost");
    #[cfg(not(test))]
    let is_valid_url = security::validate_https_url(url);

    if !is_valid_url {
        return Err(Error::InvalidUrl(url.to_string()));
    }
    if path.exists() {
        return Ok(DownloadOutcome::Skipped);
    }

    const MAX_ATTEMPTS: u32 = 5;
    let mut last_error: Option<Error> = None;

    for attempt in 0..MAX_ATTEMPTS {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }

        // Apply backoff if this is a retry attempt
        if attempt > 0 {
            let backoff_duration = match &last_error {
                Some(Error::Http { status: 429, detail }) => {
                    crate::api::rate_limit::parse_retry_after(detail)
                        .unwrap_or_else(|| crate::api::rate_limit::backoff(attempt))
                        .min(std::time::Duration::from_secs(10))
                }
                _ => crate::api::rate_limit::backoff(attempt).min(std::time::Duration::from_secs(8)),
            };

            tracing::debug!(
                "Retrying download of {} in {:?} (attempt {}/{MAX_ATTEMPTS})",
                url,
                backoff_duration,
                attempt + 1
            );

            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                _ = tokio::time::sleep(backoff_duration) => {}
            }
        }

        let request = client
            .get(url)
            .header(reqwest::header::ACCEPT, "image/svg+xml,image/*,*/*;q=0.8")
            .header(reqwest::header::REFERER, "https://commons.wikimedia.org/")
            .header("Sec-Fetch-Dest", "image")
            .header("Sec-Fetch-Mode", "no-cors")
            .header("Sec-Fetch-Site", "cross-site")
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9");

        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            res = request.send() => match res {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some(Error::from(e));
                    continue;
                }
            },
        };

        let status = response.status();
        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            last_error = Some(Error::Http {
                status: 429,
                detail: retry_after,
            });
            continue;
        }

        if status.is_server_error() {
            last_error = Some(Error::Http {
                status: status.as_u16(),
                detail: format!("server error: {status}"),
            });
            continue;
        }

        if !status.is_success() {
            // Client errors (404, 400, etc.) are non-retryable
            return Err(Error::Http {
                status: status.as_u16(),
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
        // here means we landed on a transient CDN error/throttling notice — retry.
        if content_type.contains("text/html") {
            last_error = Some(Error::Download(format!(
                "unexpected HTML content type from server: {content_type}"
            )));
            continue;
        }

        let total = response.content_length().unwrap_or(0);
        if total > MAX_FILE_BYTES {
            return Err(Error::Download(format!(
                "file is larger than the {MAX_FILE_BYTES} byte limit"
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
                    Ok(()) => return Ok(DownloadOutcome::Written {
                        path: path.to_path_buf(),
                        bytes,
                    }),
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(Error::Io(e));
                    }
                }
            }
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                if matches!(err, Error::Cancelled) {
                    return Err(Error::Cancelled);
                }
                last_error = Some(err);
                continue;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        Error::Download("download failed after maximum retry attempts".into())
    }))
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

    #[tokio::test]
    async fn download_one_retries_on_429_and_succeeds() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            // First request: 429 Too Many Requests with Retry-After: 0
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }

            // Second request: 200 OK with valid SVG
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let body = "<svg><circle r='10'/></svg>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });

        let client = reqwest::Client::new();
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("icon.svg");
        let cancel = CancellationToken::new();
        let server_url = format!("http://127.0.0.1:{port}/icon.svg");

        let res = download_one(&client, &server_url, &out_path, &cancel, None).await;
        assert!(res.is_ok(), "download should succeed on retry: {:?}", res);
        assert!(out_path.exists(), "target file should be written");
        let content = std::fs::read_to_string(&out_path).unwrap();
        assert_eq!(content, "<svg><circle r='10'/></svg>");
    }
}

