//! ZIP archive creation.
//!
//! Files are downloaded to a scratch directory first (streaming, one at a
//! time), then packed into the ZIP on a blocking thread. Nothing is held in
//! memory beyond a small copy buffer, and the archive is written to a
//! temporary path before being renamed into place.

use std::io::Write;
use std::path::{Path, PathBuf};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use zip::write::SimpleFileOptions;

use crate::error::{Error, Result};
use crate::metadata;
use crate::models::Asset;
use crate::security;

/// Progress emitted while a ZIP is being produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ZipPhase {
    Downloading,
    Packing,
    Finalizing,
}

#[derive(Debug, Clone)]
pub struct ZipEvent {
    pub phase: ZipPhase,
    pub done: usize,
    pub total: usize,
    pub current_name: Option<String>,
}

#[derive(Debug)]
pub struct ZipReport {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

/// Build a ZIP containing `svg/` (original files) and `metadata/`
/// (attribution, licenses, sources, manifest).
///
/// The archive layout:
/// ```text
/// <root>/
///   svg/<file>.svg
///   metadata/attribution.json
///   metadata/licenses.json
///   metadata/sources.json
///   metadata/manifest.json
/// ```
#[allow(clippy::too_many_arguments)]
pub async fn create_zip(
    client: &Client,
    assets: &[Asset],
    zip_path: &Path,
    root_name: &str,
    query: &str,
    concurrency: usize,
    cancel: &CancellationToken,
    mut on_event: Option<Box<dyn FnMut(ZipEvent) + Send + Sync>>,
) -> Result<ZipReport> {
    if assets.is_empty() {
        return Err(Error::Other("nothing to archive".into()));
    }
    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let root = security::sanitize_filename(root_name);
    let scratch = tempfile::tempdir()
        .map_err(|e| Error::Download(format!("could not create temp dir: {e}")))?;

    // Phase 1: download every file into the scratch directory.
    let svg_dir = scratch.path().join("svg");
    std::fs::create_dir_all(&svg_dir)?;
    let mut names = crate::download::NameAllocator::new();
    let mut downloaded: Vec<(String, PathBuf)> = Vec::with_capacity(assets.len());
    let total = assets.len();

    for (i, asset) in assets.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if let Some(cb) = on_event.as_deref_mut() {
            cb(ZipEvent {
                phase: ZipPhase::Downloading,
                done: i,
                total,
                current_name: Some(asset.original_name.clone()),
            });
        }
        let Some(url) = asset.url.as_deref() else {
            continue;
        };
        let Some(path) = names.allocate(&svg_dir, &asset.file_name, false) else {
            continue;
        };
        match crate::download::file::download_one(client, url, &path, cancel, None).await {
            Ok(crate::download::DownloadOutcome::Written { path, .. }) => {
                downloaded.push((security::sanitize_filename(&asset.file_name), path));
            }
            Ok(crate::download::DownloadOutcome::Skipped) => {
                if path.exists() {
                    downloaded.push((security::sanitize_filename(&asset.file_name), path));
                }
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                // A single bad file must not sink the archive; record it.
                tracing::warn!("skipping {} during zip: {e}", asset.original_name);
            }
        }
        let _ = concurrency;
    }

    if downloaded.is_empty() {
        return Err(Error::Download(
            "no files could be downloaded for the archive".into(),
        ));
    }

    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    if let Some(cb) = on_event.as_deref_mut() {
        cb(ZipEvent {
            phase: ZipPhase::Packing,
            done: 0,
            total: downloaded.len(),
            current_name: None,
        });
    }

    // Phase 2: pack on a blocking thread (zip requires sync Read+Write).
    let meta_docs = metadata::metadata_documents(query, assets);
    let zip_parent = zip_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let tmp_path = zip_path.with_extension("zip.part");
    let tmp_for_thread = tmp_path.clone();
    let zip_path_owned = zip_path.to_path_buf();
    let root_for_thread = root.clone();
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_flag_thread = cancel_flag.clone();

    let files_count = downloaded.len();
    let pack = tokio::task::spawn_blocking(move || {
        if cancel_flag_thread.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        pack_zip(&tmp_for_thread, &downloaded, &meta_docs, &root_for_thread)
    })
    .await;

    match pack {
        Ok(Ok(size)) => {
            if let Some(cb) = on_event.as_deref_mut() {
                cb(ZipEvent {
                    phase: ZipPhase::Finalizing,
                    done: files_count,
                    total: files_count,
                    current_name: None,
                });
            }
            std::fs::create_dir_all(&zip_parent)?;
            if zip_path.exists() {
                std::fs::remove_file(zip_path)?;
            }
            std::fs::rename(&tmp_path, zip_path)?;
            let _ = cancel_flag;
            Ok(ZipReport {
                path: zip_path_owned,
                files: files_count,
                bytes: size,
            })
        }
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(e)
        }
        Err(_) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(Error::Other("zip packing task failed".into()))
        }
    }
}

fn pack_zip(
    tmp: &Path,
    files: &[(String, PathBuf)],
    meta_docs: &[(String, String)],
    root: &str,
) -> Result<u64> {
    use std::fs::File;

    let file = File::create(tmp)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .unix_permissions(0o644);

    zip.start_file(format!("{root}/README.txt"), options)?;
    zip.write_all(README_TEXT.as_bytes())?;

    for (name, body) in meta_docs {
        zip.start_file(format!("{root}/metadata/{name}"), options)?;
        zip.write_all(body.as_bytes())?;
    }

    for (name, path) in files {
        // Re-check containment even though names came from our allocator.
        let safe = security::sanitize_filename(name);
        if safe.contains('/') || safe.contains('\\') || safe.contains("..") {
            continue;
        }
        let mut src = File::open(path)?;
        zip.start_file(format!("{root}/svg/{safe}"), options)?;
        std::io::copy(&mut src, &mut zip)?;
    }

    let file = zip.finish()?;
    file.sync_all()?;
    let meta = std::fs::metadata(tmp)?;
    Ok(meta.len())
}

const README_TEXT: &str =
    "This archive was created by GET SVG (https://github.com/avdeshjadon/get-svg)\n\n\
Contents:\n\
  svg/       Original SVG files, unmodified, as served by Wikimedia Commons\n\
  metadata/  Attribution, license, and source records captured at download time\n\n\
Licensing:\n\
Each file carries its own license. Read metadata/attribution.json and\n\
metadata/licenses.json — do not assume a shared license across files.\n\
License data may be incomplete when the source page did not expose it;\n\
verify on the Wikimedia Commons page linked in metadata/sources.json.\n\n\
GET SVG is an independent tool and does not own or license any content.\n";

/// Streaming download of many files into a ZIP is impractical without a
/// seekable writer; we stream each file to disk (never RAM) instead.
pub fn zip_wants_scratch() -> &'static str {
    "scratch directory"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_zip_creates_valid_archive() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.svg");
        std::fs::write(&a, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let b = dir.path().join("b.svg");
        std::fs::write(&b, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let zip_path = dir.path().join("out.zip");
        let meta = vec![("attribution.json".to_string(), "[]".to_string())];

        pack_zip(
            &zip_path,
            &[("a.svg".into(), a), ("b.svg".into(), b)],
            &meta,
            "root",
        )
        .unwrap();

        let f = std::fs::File::open(&zip_path).unwrap();
        let archive = zip::ZipArchive::new(f).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.name_for_index(i).unwrap().to_string())
            .collect();
        names.sort();
        assert!(names.iter().any(|n| n == "root/svg/a.svg"), "{names:?}");
        assert!(names.iter().any(|n| n == "root/svg/b.svg"), "{names:?}");
        assert!(names.iter().any(|n| n == "root/metadata/attribution.json"));
        assert!(names.iter().any(|n| n == "root/README.txt"));
    }

    #[test]
    fn pack_zip_blocks_traversal_entries() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.svg");
        std::fs::write(&a, "<svg/>").unwrap();
        let zip_path = dir.path().join("out.zip");
        // A crafted name that would escape the archive root.
        pack_zip(&zip_path, &[("../../evil.svg".into(), a)], &[], "root").unwrap();
        let f = std::fs::File::open(&zip_path).unwrap();
        let archive = zip::ZipArchive::new(f).unwrap();
        for i in 0..archive.len() {
            let name = archive.name_for_index(i).unwrap();
            assert!(!name.contains(".."), "path traversal in zip: {name}");
            assert!(!name.starts_with('/'), "absolute entry in zip: {name}");
        }
    }

    #[test]
    fn readme_mentions_per_file_licensing() {
        assert!(README_TEXT.contains("do not assume a shared license"));
    }
}
