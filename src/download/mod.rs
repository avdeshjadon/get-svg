//! Streaming file downloads with progress, cancellation, and atomic rename.

pub mod file;

pub use file::{download_one, unique_path, DownloadOutcome, NameAllocator};

use std::path::PathBuf;
use std::sync::Arc;

use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::models::Asset;
use crate::security;

/// Aggregate outcome of a multi-file download.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DownloadStats {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub bytes_downloaded: u64,
}

impl DownloadStats {
    pub fn merge(&mut self, other: &DownloadStats) {
        self.total += other.total;
        self.completed += other.completed;
        self.failed += other.failed;
        self.skipped += other.skipped;
        self.bytes_downloaded += other.bytes_downloaded;
    }
}

/// Progress events emitted while a batch runs. The download layer stays
/// UI-agnostic: consumers receive these over a channel.
#[derive(Debug, Clone)]
pub enum BatchEvent {
    /// A file started downloading.
    Started {
        index: usize,
        name: String,
        total_bytes: Option<u64>,
    },
    /// Intra-file progress (only emitted when a total size is known).
    Progress { index: usize, done: u64, total: u64 },
    /// A file finished, failed, or was skipped.
    Finished {
        index: usize,
        name: String,
        status: FileStatus,
        bytes: u64,
    },
    /// Whole-batch snapshot after each file settles.
    Batch { stats: DownloadStats },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    Completed,
    Failed(String),
    Skipped,
}

impl FileStatus {
    pub fn label(&self) -> String {
        match self {
            FileStatus::Completed => "Completed".into(),
            FileStatus::Failed(e) => format!("Failed: {e}"),
            FileStatus::Skipped => "Skipped (exists)".into(),
        }
    }
}

/// A single finished file.
#[derive(Debug, Clone)]
pub struct FileResult {
    pub name: String,
    pub path: Option<PathBuf>,
    pub status: FileStatus,
    pub bytes: u64,
}

/// Context threaded through batch downloads.
pub struct BatchContext {
    pub client: Client,
    pub dest: PathBuf,
    pub overwrite: bool,
    pub concurrency: usize,
    pub cancel: CancellationToken,
    pub events: Option<mpsc::UnboundedSender<BatchEvent>>,
}

impl BatchContext {
    fn emit(&self, event: BatchEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }
}

/// Download many assets concurrently with bounded parallelism.
///
/// Never loads whole files into memory; each download streams to a temporary
/// file and is atomically renamed into place.
pub async fn download_many(ctx: Arc<BatchContext>, assets: Vec<Asset>) -> Result<DownloadStats> {
    std::fs::create_dir_all(&ctx.dest)?;
    let names = Arc::new(Mutex::new(NameAllocator::new()));
    let sem = Arc::new(Semaphore::new(ctx.concurrency.max(1)));
    let mut stats = DownloadStats {
        total: assets.len(),
        ..DownloadStats::default()
    };

    let mut tasks = Vec::with_capacity(assets.len());
    for (index, asset) in assets.into_iter().enumerate() {
        let ctx = ctx.clone();
        let names = names.clone();
        let sem = sem.clone();
        tasks.push(async move {
            if ctx.cancel.is_cancelled() {
                return (index, asset.file_name, FileStatus::Skipped, 0u64);
            }
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return (index, asset.file_name, FileStatus::Skipped, 0u64),
            };

            let display = asset.original_name.clone();
            ctx.emit(BatchEvent::Started {
                index,
                name: display.clone(),
                total_bytes: asset.size_bytes,
            });

            if asset.url.is_none() {
                ctx.emit(BatchEvent::Finished {
                    index,
                    name: display.clone(),
                    status: FileStatus::Failed("no download URL available".into()),
                    bytes: 0,
                });
                return (
                    index,
                    display,
                    FileStatus::Failed("no download URL".into()),
                    0,
                );
            }

            let outcome = {
                let mut guard = names.lock().await;
                let preferred = guard.allocate(&ctx.dest, &asset.file_name, ctx.overwrite);
                match preferred {
                    None => Ok(DownloadOutcome::Skipped),
                    Some(path) => {
                        drop(guard);
                        download_one(
                            &ctx.client,
                            asset.url.as_deref().unwrap(),
                            &path,
                            &ctx.cancel,
                            None,
                        )
                        .await
                    }
                }
            };

            let (status, bytes) = match outcome {
                Ok(DownloadOutcome::Written { bytes, .. }) => (FileStatus::Completed, bytes),
                Ok(DownloadOutcome::Skipped) => (FileStatus::Skipped, 0),
                Err(Error::Cancelled) => (FileStatus::Skipped, 0),
                Err(e) => (FileStatus::Failed(e.to_string()), 0),
            };
            ctx.emit(BatchEvent::Finished {
                index,
                name: display.clone(),
                status: status.clone(),
                bytes,
            });
            (index, display, status, bytes)
        });
    }

    let mut stream = futures::stream::iter(tasks).buffer_unordered(ctx.concurrency.max(1));
    let mut results: Vec<(usize, String, FileStatus, u64)> = Vec::new();

    while let Some(res) = stream.next().await {
        results.push(res);
        let snapshot = DownloadStats {
            total: stats.total,
            completed: results
                .iter()
                .filter(|r| matches!(r.2, FileStatus::Completed))
                .count(),
            failed: results
                .iter()
                .filter(|r| matches!(r.2, FileStatus::Failed(_)))
                .count(),
            skipped: results
                .iter()
                .filter(|r| matches!(r.2, FileStatus::Skipped))
                .count(),
            bytes_downloaded: results.iter().map(|r| r.3).sum(),
        };
        ctx.emit(BatchEvent::Batch {
            stats: snapshot.clone(),
        });
        stats = snapshot;
    }

    if ctx.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(stats)
}

/// Estimate total bytes for a set of assets (unknown sizes count as 0).
pub fn estimated_bytes(assets: &[Asset]) -> u64 {
    assets.iter().filter_map(|a| a.size_bytes).sum()
}

/// Best-effort disk-space precondition check. Returns an error only when we
/// both know the free space and it is clearly insufficient.
pub fn ensure_space(dest: &std::path::Path, needed: u64) -> Result<()> {
    if needed == 0 {
        return Ok(());
    }
    if let Some(available) = security::available_space(dest) {
        // Leave 64 MB of headroom for the OS.
        if available < needed.saturating_add(64 * 1024 * 1024) {
            return Err(Error::InsufficientSpace { needed });
        }
    }
    Ok(())
}

/// Quick helper for UI/CLI prompts: human label for a batch.
pub fn batch_label(count: usize, bytes: u64) -> String {
    format!(
        "{count} file(s), estimated size {}",
        crate::models::format_size(bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimated_bytes_sums_known_sizes() {
        let assets = vec![
            Asset {
                title: "File:A.svg".into(),
                page_id: 1,
                original_name: "A.svg".into(),
                file_name: "A.svg".into(),
                index: None,
                size_bytes: Some(100),
                mime: None,
                width: None,
                height: None,
                mediatype: None,
                url: None,
                thumb_url: None,
                description_url: None,
                author: None,
                uploader: None,
                license: None,
                license_url: None,
                usage_terms: None,
                attribution: None,
                credit: None,
                description: None,
                categories: vec![],
                uploaded_at: None,
                modified_at: None,
            },
            Asset {
                size_bytes: Some(50),
                ..Default::default()
            },
        ];
        assert_eq!(estimated_bytes(&assets), 150);
    }

    #[test]
    fn file_status_labels_are_human() {
        assert_eq!(FileStatus::Completed.label(), "Completed");
        assert_eq!(FileStatus::Skipped.label(), "Skipped (exists)");
        assert!(FileStatus::Failed("boom".into()).label().contains("boom"));
    }
}
