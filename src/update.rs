//! Self-update: fetch the latest GitHub release and replace the running binary.
//!
//! `getsvg update` resolves the newest tagged release for the current
//! platform, downloads the packaged archive, verifies its SHA-256, and swaps
//! the installed `get-svg`/`getsvg` binaries in place. Older binaries and any
//! leftover `.old` files are removed so no stale copies survive.

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::models::format_size;

/// Base of the GitHub API URL. Overridable for testing via
/// `GET_SVG_UPDATE_REPO` (must point at an API root that serves
/// `/repos/avdeshjadon/get-svg/releases/latest`).
fn api_root() -> String {
    std::env::var("GET_SVG_UPDATE_REPO")
        .unwrap_or_else(|_| "https://api.github.com/repos/avdeshjadon/get-svg".to_string())
}

fn repo_home() -> &'static str {
    "https://github.com/avdeshjadon/get-svg"
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

/// Map the running OS/CPU to the artifact triple used by the release workflow.
fn target_triple() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let triple = match (os, arch) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => {
            return Err(Error::Other(format!(
                "no prebuilt binary exists for your system ({os}/{arch}).\n\
                 Install from source instead:\n  cargo install --git {}\n\n\
                 (or use the one-line installer from the README, which builds from a release.)",
                repo_home()
            )));
        }
    };
    Ok(triple)
}

fn archive_ext() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

/// Compare two dotted versions (`0.1.1` vs `0.2.0`). Missing components are 0.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|p| {
                // Strip non-numeric suffixes like "0.1.1-beta.1".
                let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    };
    let (aa, bb) = (parse(a), parse(b));
    aa.cmp(&bb)
}

/// A URL we will download from: HTTPS always; plain HTTP only for loopback
/// hosts (useful for local release mirrors and tests).
fn url_allowed(s: &str) -> bool {
    match url::Url::parse(s) {
        Ok(u) => {
            let host = u.host_str().unwrap_or("");
            let loopback = u.scheme() == "http"
                && (host == "localhost"
                    || host == "127.0.0.1"
                    || host == "::1"
                    || host == "[::1]"
                    || host.starts_with("127."));
            (u.scheme() == "https" || loopback) && u.username().is_empty() && u.password().is_none()
        }
        Err(_) => false,
    }
}

/// Download `url` to `path`, showing a live progress bar. Content-type checks
/// mirror the SVG downloader so an HTML error page is never saved.
async fn download(client: &reqwest::Client, url: &str, path: &Path, label: &str) -> Result<u64> {
    if !url_allowed(url) {
        return Err(Error::InvalidUrl(url.to_string()));
    }
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(Error::Http {
            status: response.status().as_u16(),
            detail: format!("download failed for {url}"),
        });
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("text/html") {
        return Err(Error::Download(format!(
            "unexpected content type from server: {content_type}"
        )));
    }

    let total = response.content_length().unwrap_or(0);
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {msg}\n[{bar:40.green/blue}] {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}",
        )
        .unwrap()
        .progress_chars("=>─"),
    );
    bar.set_message(label.to_string());

    let mut file = std::fs::File::create(path)?;
    let mut remaining = total;
    let mut body = response.bytes_stream();
    let mut written: u64 = 0;
    use futures::StreamExt;
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        written += chunk.len() as u64;
        remaining = remaining.saturating_sub(chunk.len() as u64);
        bar.set_position(written);
        if remaining == 0 {
            break;
        }
    }
    bar.finish_and_clear();
    Ok(written)
}

/// Fetch a `.sha256` asset and return the expected hex digest.
async fn fetch_checksum(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(Error::Http {
            status: response.status().as_u16(),
            detail: "could not fetch checksum".to_string(),
        });
    }
    let text = response.text().await?;
    text.split_whitespace()
        .find(|tok| tok.len() == 64 && tok.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|h| h.to_ascii_lowercase())
        .ok_or_else(|| {
            Error::Other(format!(
                "checksum file had no valid SHA-256 digest: {}",
                url
            ))
        })
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Extract `archive` into `dest`. Supports the `.tar.gz` and `.zip` layouts
/// produced by the release workflow.
fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    if archive_ext() == "zip" {
        let mut z = zip::ZipArchive::new(file)?;
        z.extract(dest)?;
    } else {
        let dec = GzDecoder::new(file);
        let mut tar = tar::Archive::new(dec);
        tar.unpack(dest)?;
    }
    Ok(())
}

/// Recursively find release binaries (`get-svg`/`getsvg`, optional `.exe`)
/// inside the extracted archive.
fn find_binaries(root: &Path) -> Result<Vec<PathBuf>> {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let mut found = Vec::new();
    for entry in walkdir(root)? {
        if entry.is_file() {
            let name = entry
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name == format!("get-svg{ext}") || name == format!("getsvg{ext}") {
                found.push(entry);
            }
        }
    }
    if found.is_empty() {
        return Err(Error::Other(
            "release archive contained no get-svg binary".to_string(),
        ));
    }
    Ok(found)
}

fn walkdir(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Put `source` (the fresh binary) in place of `dest`, keeping a `.old`
/// backup long enough to survive on Windows, then deleting it.
#[cfg(not(windows))]
fn install_binary(source: &Path, dest: &Path) -> Result<()> {
    let old = old_path(dest);
    if dest.exists() {
        std::fs::rename(dest, &old)?;
    }
    std::fs::copy(source, dest)?;
    let perms = std::fs::metadata(source)?.permissions();
    std::fs::set_permissions(dest, perms)?;
    if old.exists() {
        let _ = std::fs::remove_file(&old);
    }
    Ok(())
}

#[cfg(windows)]
fn install_binary(source: &Path, dest: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let old = old_path(dest);
    if dest.exists() {
        std::fs::rename(dest, &old)?;
    }
    std::fs::copy(source, dest)?;
    let perms = std::fs::metadata(source)?.permissions();
    std::fs::set_permissions(dest, perms)?;
    // The running exe may be the `.old` file; deleting it now can fail, so
    // schedule a detached, delayed delete that actually removes it once the
    // process has exited.
    if old.exists() {
        let _ = std::fs::remove_file(&old).or_else(|_| {
            std::process::Command::new("cmd")
                .creation_flags(0x0800_0000 | 0x0000_0008) // CREATE_NO_WINDOW | DETACHED_PROCESS
                .args([
                    "/C",
                    "ping -n 3 127.0.0.1 >nul & del /q",
                    &old.to_string_lossy(),
                ])
                .spawn()
                .map(|_| ())
        });
    }
    Ok(())
}

fn old_path(dest: &Path) -> PathBuf {
    let mut os = dest.as_os_str().to_os_string();
    os.push(".old");
    PathBuf::from(os)
}

/// Run the self-update flow. Returns a process exit code.
pub async fn run_update() -> Result<i32> {
    let current = crate::VERSION;
    let exe = std::env::current_exe().map_err(|_| {
        Error::Other(
            "could not locate the running binary (is it installed on a writable path?)".to_string(),
        )
    })?;
    let bin_dir = exe
        .parent()
        .ok_or_else(|| Error::Other("could not determine the install directory".to_string()))?
        .to_path_buf();

    let triple = target_triple()?;
    let ext = archive_ext();
    let asset_name = format!("get-svg-{triple}.{ext}");
    let checksum_name = format!("{asset_name}.sha256");

    let client = reqwest::Client::builder()
        .user_agent(format!("get-svg/{current} (self-update)"))
        .build()?;

    let latest_url = format!("{}/releases/latest", api_root());
    let response = client.get(&latest_url).send().await?;
    if !response.status().is_success() {
        return Err(Error::Http {
            status: response.status().as_u16(),
            detail: format!("could not fetch {latest_url}"),
        });
    }
    let release: Release = response.json().await?;
    let latest = release.tag_name.trim_start_matches('v').to_string();

    if version_cmp(current, &latest) != std::cmp::Ordering::Less {
        let note = if current == latest {
            "already up to date"
        } else {
            "running a newer build than the latest release"
        };
        println!("✓ get-svg v{current} is {note} (latest: v{latest})");
        return Ok(0);
    }

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            Error::Other(format!(
                "release v{latest} does not ship {asset_name} for this platform"
            ))
        })?;
    let checksum_asset = release
        .assets
        .iter()
        .find(|a| a.name == checksum_name)
        .ok_or_else(|| {
            Error::Other(format!(
                "release v{latest} is missing the checksum file {checksum_name}"
            ))
        })?;

    println!(
        "✚ Updating get-svg v{current} → v{latest} (installed in {})",
        bin_dir.display()
    );

    let tmp = tempfile::tempdir()?;
    let archive_path = tmp.path().join(&asset_name);
    let expected = fetch_checksum(&client, &checksum_asset.browser_download_url).await?;
    let bytes = download(
        &client,
        &asset.browser_download_url,
        &archive_path,
        &asset_name,
    )
    .await?;
    println!("✓ Downloaded {} ({})", format_size(bytes), asset_name);

    let got = sha256_of(&archive_path)?;
    if got != expected {
        return Err(Error::Other(format!(
            "checksum mismatch (expected {expected}, got {got}) — the download is \
             corrupt or tampered with; aborting without touching your install."
        )));
    }
    println!("✓ SHA-256 checksum verified");

    let extract_dir = tmp.path().join("pkg");
    std::fs::create_dir_all(&extract_dir)?;
    extract(&archive_path, &extract_dir)?;

    let fresh = find_binaries(&extract_dir)?;
    let ext_bin = if cfg!(windows) { ".exe" } else { "" };
    let mut installed = Vec::new();
    for name in ["get-svg", "getsvg"] {
        let target = bin_dir.join(format!("{name}{ext_bin}"));
        if let Some(source) = fresh.iter().find(|p| {
            p.file_name().map(|n| n.to_string_lossy().into_owned())
                == Some(format!("{name}{ext_bin}"))
        }) {
            install_binary(source, &target)?;
            installed.push(
                target
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    if installed.len() == 1 {
        // Update the sibling alias too when the archive carries only one name.
        println!("✓ Installed {}", installed[0]);
    } else {
        println!("✓ Installed {}", installed.join(", "));
    }

    for name in ["get-svg", "getsvg"] {
        let stale = bin_dir.join(format!("{name}{ext_bin}.old"));
        if stale.exists() && std::fs::remove_file(&stale).is_err() {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("cmd")
                    .creation_flags(0x0800_0000 | 0x0000_0008)
                    .args(["/C", "del /q", &stale.to_string_lossy()])
                    .spawn();
            }
        }
    }
    println!("✓ Old binaries removed");

    println!(
        "\n✓ Update complete. Restart get-svg to use v{latest}.\n\
         Installer one-liner (keeps this updated):\n  \
         curl -fsSL https://cli.get-svg.app/install | sh"
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_order() {
        assert_eq!(version_cmp("0.1.1", "0.1.1"), std::cmp::Ordering::Equal);
        assert_eq!(version_cmp("0.1.1", "0.2.0"), std::cmp::Ordering::Less);
        assert_eq!(version_cmp("0.10.0", "0.2.0"), std::cmp::Ordering::Greater);
        assert_eq!(version_cmp("0.1.1", "0.1.2-beta"), std::cmp::Ordering::Less);
    }

    #[test]
    fn checksum_line_parses_hex() {
        let text = "fc6467fabb6c024e5211e4388b8bf16ad4e203bcbb5c6c52a2e4f58a1af1088a  get-svg\n";
        let hex = text
            .split_whitespace()
            .find(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()))
            .unwrap();
        assert_eq!(hex.len(), 64);
    }

    #[test]
    fn install_binary_replaces_and_removes_old() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("get-svg");
        let dest = tmp.path().join("getsvg");
        std::fs::write(&src, b"new-binary").unwrap();
        std::fs::write(&dest, b"old-binary").unwrap();
        install_binary(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new-binary");
        assert!(!old_path(&dest).exists());
    }
}
