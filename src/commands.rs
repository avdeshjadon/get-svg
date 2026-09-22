//! Non-interactive command implementations.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::api::{AssetProvider, WikimediaClient};
use crate::archive::{create_zip, ZipEvent};
use crate::cache::Cache;
use crate::cli::{CacheCommand, Cli, Command};
use crate::config::Settings;
use crate::download::{
    download_many, ensure_space, estimated_bytes, BatchContext, BatchEvent, DownloadStats,
};
use crate::error::{Error, Result};
use crate::metadata;
use crate::models::{format_size, Asset};
use crate::output::{write_asset_record, write_assets, OutputFormat};
use crate::{archive, doctor, search};

/// Top-level dispatch. Returns a process exit code.
pub async fn dispatch(cli: Cli) -> Result<i32> {
    match cli.command {
        None => interactive(),
        Some(cmd) => match cmd {
            Command::Search {
                query,
                limit,
                format,
                download,
                zip,
                metadata,
                license,
                page,
                yes,
                overwrite,
                concurrency,
            } => {
                cmd_search(SearchArgs {
                    query,
                    limit,
                    format,
                    download,
                    zip,
                    metadata,
                    license,
                    page,
                    yes,
                    overwrite,
                    concurrency,
                })
                .await
            }
            Command::Download {
                file,
                output,
                format,
                overwrite,
            } => cmd_download(file, output, format, overwrite).await,
            Command::Category {
                category,
                limit,
                format,
                download,
                zip,
                yes,
                overwrite,
            } => {
                let query = compose_category_query(&category);
                cmd_search(SearchArgs {
                    query,
                    limit,
                    format,
                    download,
                    zip,
                    metadata: None,
                    license: None,
                    page: 1,
                    yes,
                    overwrite,
                    concurrency: None,
                })
                .await
            }
            Command::Batch {
                query,
                download,
                zip,
                metadata,
                limit,
                yes,
                overwrite,
            } => {
                let dest = download.or_else(|| zip.as_ref().map(|_| default_download_dir()));
                cmd_search(SearchArgs {
                    query,
                    limit,
                    format: OutputFormat::Table,
                    download: dest,
                    zip,
                    metadata,
                    license: None,
                    page: 1,
                    yes,
                    overwrite,
                    concurrency: None,
                })
                .await
            }
            Command::Cache(action) => cmd_cache(action).await,
            Command::Config { init } => cmd_config(init),
            Command::Doctor { json } => cmd_doctor(json).await,
            Command::Version => {
                cmd_version();
                Ok(0)
            }
        },
    }
}

fn default_download_dir() -> PathBuf {
    Settings::default().download_dir
}

fn compose_category_query(category: &str) -> String {
    let cat = category
        .trim()
        .trim_start_matches("Category:")
        .trim_start_matches("category:");
    let escaped = cat.replace('\\', "\\\\").replace('"', "\\\"");
    format!("incategory:\"{escaped}\"")
}

/// Interactive mode is only available on a real TTY.
fn interactive() -> Result<i32> {
    use crate::ui;
    ui::run()
}

struct SearchArgs {
    query: String,
    limit: usize,
    format: OutputFormat,
    download: Option<PathBuf>,
    zip: Option<PathBuf>,
    metadata: Option<PathBuf>,
    license: Option<String>,
    page: u32,
    yes: bool,
    overwrite: bool,
    concurrency: Option<usize>,
}

async fn cmd_search(args: SearchArgs) -> Result<i32> {
    let has_download = args.download.is_some();
    let has_zip = args.zip.is_some();
    let interactive = std::io::stderr().is_terminal();
    let mut settings = Settings::load()?;
    if let Some(c) = args.concurrency {
        settings.max_concurrency = c.clamp(1, 32);
    }
    settings.assume_yes = args.yes;

    let provider = WikimediaClient::new(&settings)?;
    let cache = Cache::new(&settings);

    let per_page = args.limit.clamp(1, 500) as u32;
    let offset = (args.page.max(1) - 1) as u64 * per_page as u64;

    // Animated search status when stderr is a terminal; plain line otherwise.
    let spinner = interactive.then(|| {
        let s = ProgressBar::new_spinner();
        s.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        s.set_message(format!(
            "Searching Wikimedia Commons for \"{}\"",
            args.query.trim()
        ));
        s.enable_steady_tick(std::time::Duration::from_millis(80));
        s
    });
    let started = std::time::Instant::now();

    let mut total_hits: Option<u64> = None;
    let mut fetched = Vec::new();
    let mut cursor = offset;
    let want = args.limit;

    while fetched.len() < want {
        let page_limit = std::cmp::min(per_page as usize, want - fetched.len()) as u32;
        let page = search::fetch_page(&provider, &cache, &args.query, cursor, page_limit).await?;
        if total_hits.is_none() {
            total_hits = page.page.total_hits;
        }
        let batch = page.page.assets;
        if batch.is_empty() {
            break;
        }
        let got = batch.len() as u64;
        fetched.extend(batch);
        cursor += got;
        if total_hits.map(|t| cursor >= t).unwrap_or(false) {
            break;
        }
        if fetched.len() >= want {
            break;
        }
    }

    if let Some(s) = &spinner {
        s.finish_and_clear();
    }

    let assets = search::filter_by_license(fetched, args.license.as_deref());
    eprintln!(
        "\u{2713} {} result(s) in {}ms",
        assets.len(),
        crate::models::format_ms(started.elapsed().as_millis() as u64)
    );

    if let Some(dir) = &args.metadata {
        metadata::write_metadata_dir(dir, &args.query, &assets)?;
        eprintln!("Wrote attribution metadata to {}", dir.display());
    }

    let mut exit_code = 0;

    if let Some(dir) = args.download {
        let stats = run_downloads(
            &settings,
            &provider,
            &assets,
            &dir,
            args.overwrite,
            args.yes,
        )
        .await?;
        print_stats(&stats);
        if stats.failed > 0 {
            exit_code = 1;
        }
        metadata::write_metadata_dir(&dir, &args.query, &assets)?;
    }

    if let Some(zip_path) = args.zip {
        let report = run_zip(
            &settings,
            &provider,
            &assets,
            &zip_path,
            &args.query,
            args.yes,
        )
        .await?;
        eprintln!(
            "\u{2713} Created {} ({} files, {})",
            report.path.display(),
            report.files,
            format_size(report.bytes)
        );
    }

    // Results always go to stdout so `--json` piping works, even alongside
    // --download/--zip.
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    write_assets(&mut lock, args.format, &args.query, total_hits, &assets)?;
    lock.flush()?;

    // Guided download: when the user didn't pass --download/--zip, offer to save
    // the results right here — no extra flags to remember. Skipped when stdout
    // is redirected/automated (only stderr being a TTY means a human).
    if !has_download && !has_zip && !assets.is_empty() && interactive {
        let want_download =
            settings.assume_yes || ask_bool(&format!("\nDownload {} file(s)", assets.len()), true)?;
        if want_download {
            let dest = if settings.assume_yes {
                settings.download_dir.clone()
            } else {
                ask_dir(
                    "Save to folder (Enter = app download folder)",
                    &settings.download_dir,
                )?
            };
            let stats = run_downloads(
                &settings,
                &provider,
                &assets,
                &dest,
                args.overwrite,
                settings.assume_yes,
            )
            .await?;
            print_stats(&stats);
            if stats.failed > 0 {
                exit_code = 1;
            }
            metadata::write_metadata_dir(&dest, &args.query, &assets)?;
            eprintln!("\u{2192} Saved to {}", dest.display());
        } else {
            eprintln!(
                "\nTip: re-run with `get-svg search \"{}\" --download DIR`, or `--zip FILE`.",
                args.query.trim()
            );
        }
    }

    Ok(exit_code)
}

/// Read a y/n answer from stdin; piped input works too (`echo y | …`).
fn ask_bool(prompt: &str, default: bool) -> Result<bool> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    eprint!("{prompt} {hint} ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(default);
    }
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

/// Read a destination folder (Enter keeps the default; `~/…` expands).
fn ask_dir(prompt: &str, default: &std::path::Path) -> Result<PathBuf> {
    eprint!("{prompt} [{}]: ", default.display());
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let input = line.trim();
    if input.is_empty() {
        return Ok(default.to_path_buf());
    }
    Ok(crate::config::expand_tilde(PathBuf::from(input)))
}

async fn cmd_download(
    file: String,
    output: Option<PathBuf>,
    format: OutputFormat,
    overwrite: bool,
) -> Result<i32> {
    let settings = Settings::load()?;
    let provider = WikimediaClient::new(&settings)?;
    let cache = Cache::new(&settings);

    let title = if file.starts_with("File:") || file.starts_with("file:") {
        file.clone()
    } else if file.ends_with(".svg") && !file.contains('/') {
        format!("File:{file}")
    } else {
        file.clone()
    };

    eprintln!("Fetching metadata for {title}...");
    let asset = match provider.get_asset(&title).await? {
        Some(a) => a,
        None => {
            // Fall back to a title search so `download github-logo` works.
            let found = search::fetch_page(&provider, &cache, &title, 0, 5).await?;
            match found.page.assets.into_iter().find(|a| {
                a.original_name.eq_ignore_ascii_case(&title) || a.title.eq_ignore_ascii_case(&title)
            }) {
                Some(a) => a,
                None => {
                    return Err(Error::Other(format!(
                        "no file named \"{title}\" was found on Wikimedia Commons"
                    )));
                }
            }
        }
    };

    let dest = output.unwrap_or(settings.download_dir.clone());
    std::fs::create_dir_all(&dest)?;
    ensure_space(&dest, asset.size_bytes.unwrap_or(0))?;

    let bar = ProgressBar::new(asset.size_bytes.unwrap_or(0));
    bar.set_style(default_style());
    bar.set_message(format!("Downloading {}", asset.original_name));

    let ctx = Arc::new(BatchContext {
        client: provider.client(),
        dest: dest.clone(),
        overwrite,
        concurrency: 1,
        cancel: tokio_util::sync::CancellationToken::new(),
        events: None,
    });

    let stats = {
        let mut assets = vec![asset.clone()];
        // Reuse the batch path so naming/atomicity logic stays in one place.
        let names = std::sync::Mutex::new(crate::download::NameAllocator::new());
        let _ = names;
        download_many(ctx, std::mem::take(&mut assets)).await?
    };
    bar.finish_and_clear();

    metadata::write_metadata_dir(&dest, &asset.title, std::slice::from_ref(&asset))?;

    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    if stats.completed > 0 {
        let saved = dest.join(&asset.file_name);
        eprintln!("\u{2713} Saved {}", saved.display());
        write_asset_record(&mut lock, format, &asset)?;
        lock.flush()?;
        Ok(0)
    } else if stats.skipped > 0 {
        eprintln!(
            "File already exists: {} (use --overwrite to replace)",
            dest.join(&asset.file_name).display()
        );
        Ok(1)
    } else {
        Err(Error::Download(format!(
            "could not download {}",
            asset.original_name
        )))
    }
}

async fn cmd_cache(action: CacheCommand) -> Result<i32> {
    let settings = Settings::load()?;
    let cache = Cache::new(&settings);
    match action {
        CacheCommand::Status => {
            let st = cache.status();
            println!("Cache directory : {}", st.directory.display());
            println!("Enabled         : {}", st.enabled);
            println!("Entries         : {}", st.file_count);
            println!(
                "Size            : {} / {}",
                format_size(st.total_bytes),
                format_size(st.max_bytes)
            );
            println!("TTL             : {}h", st.ttl_hours);
            Ok(0)
        }
        CacheCommand::Clear => {
            let removed = cache.clear()?;
            println!(
                "Removed {removed} cache entr{}.",
                if removed == 1 { "y" } else { "ies" }
            );
            Ok(0)
        }
    }
}

fn cmd_config(init: bool) -> Result<i32> {
    let settings = Settings::load()?;
    if init {
        let path = Settings::write_default_config(&settings.config_path)?;
        println!("Wrote {}", path.display());
        return Ok(0);
    }
    println!("# {}", crate::APP_NAME);
    println!("# Config file: {}", settings.config_path.display());
    if !settings.config_path.exists() {
        println!("# (file does not exist yet; run `get-svg config --init` to create it)");
    }
    println!();
    print!("{}", settings.to_toml_string());
    Ok(0)
}

async fn cmd_doctor(json: bool) -> Result<i32> {
    let settings = Settings::load()?;
    let report = doctor::run(&settings).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", doctor::render(&report));
    }
    Ok(if report.healthy { 0 } else { 1 })
}

fn cmd_version() {
    println!("{} v{}", crate::APP_NAME, crate::VERSION);
    println!("{}", crate::TAGLINE);
}

// ---------------------------------------------------------------------------
// Shared bulk operations
// ---------------------------------------------------------------------------

fn default_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.green} {msg}\n[{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}",
    )
    .unwrap()
    .progress_chars("=>─")
}

fn bulk_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.green} {msg}\n[{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {bytes_per_sec} ETA {eta}",
    )
    .unwrap()
    .progress_chars("=>─")
}

fn confirm(prompt: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(Error::Other(format!(
            "{prompt} requires confirmation but stdin is not a TTY; pass --yes to proceed."
        )));
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

fn confirm_bulk(count: usize, bytes: u64, assume_yes: bool) -> Result<bool> {
    if count <= 20 && bytes <= 64 * 1024 * 1024 {
        return Ok(true);
    }
    let msg = if bytes > 1024 * 1024 * 1024 {
        format!(
            "You are about to download {count} files ({})\nThis may consume significant bandwidth and disk space.\nContinue?",
            format_size(bytes)
        )
    } else {
        format!(
            "About to download {count} files ({}). Continue?",
            format_size(bytes)
        )
    };
    confirm(&msg, assume_yes)
}

async fn run_downloads(
    settings: &Settings,
    provider: &WikimediaClient,
    assets: &[Asset],
    dest: &PathBuf,
    overwrite: bool,
    yes: bool,
) -> Result<DownloadStats> {
    if assets.is_empty() {
        eprintln!("Nothing to download.");
        return Ok(DownloadStats::default());
    }
    std::fs::create_dir_all(dest)?;
    let bytes = estimated_bytes(assets);
    ensure_space(dest, bytes)?;
    if !confirm_bulk(assets.len(), bytes, yes)? {
        eprintln!("Cancelled.");
        return Ok(DownloadStats::default());
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();
    let ctx = Arc::new(BatchContext {
        client: provider.client(),
        dest: dest.clone(),
        overwrite,
        concurrency: settings.max_concurrency,
        cancel: tokio_util::sync::CancellationToken::new(),
        events: Some(tx),
    });

    let multi = MultiProgress::new();
    let overall = multi.add(ProgressBar::new(assets.len() as u64));
    overall.set_style(bulk_style());
    overall.set_message(format!("Downloading {} files", assets.len()));

    let download = download_many(ctx, assets.to_vec());
    let ui_loop = async {
        while let Some(event) = rx.recv().await {
            match event {
                BatchEvent::Batch { stats } => {
                    overall.set_position((stats.completed + stats.failed + stats.skipped) as u64);
                    overall.set_message(format!(
                        "Completed: {}  Failed: {}  Skipped: {}",
                        stats.completed, stats.failed, stats.skipped
                    ));
                }
                BatchEvent::Finished {
                    name,
                    status: crate::download::FileStatus::Failed(e),
                    ..
                } => {
                    tracing::warn!("{name}: {e}");
                }
                _ => {}
            }
        }
    };

    let (stats, _) = tokio::join!(download, ui_loop);
    overall.finish_and_clear();
    stats
}

fn print_stats(stats: &DownloadStats) {
    if stats.total == 0 {
        return;
    }
    eprintln!(
        "\u{2713} Completed: {}   \u{2717} Failed: {}   \u{2013} Skipped: {}   ({})",
        stats.completed,
        stats.failed,
        stats.skipped,
        format_size(stats.bytes_downloaded)
    );
}

async fn run_zip(
    settings: &Settings,
    provider: &WikimediaClient,
    assets: &[Asset],
    zip_path: &Path,
    query: &str,
    yes: bool,
) -> Result<archive::ZipReport> {
    if assets.is_empty() {
        return Err(Error::Other("no files to archive".into()));
    }
    let bytes = estimated_bytes(assets);
    if !confirm_bulk(assets.len(), bytes, yes)? {
        return Err(Error::Other("cancelled".into()));
    }

    let bar = ProgressBar::new(assets.len() as u64);
    bar.set_style(bulk_style());
    bar.set_message("Preparing archive");

    let bar_clone = bar.clone();
    let callback = move |ev: ZipEvent| {
        let label = match ev.phase {
            archive::ZipPhase::Downloading => format!(
                "Downloading {}",
                ev.current_name.clone().unwrap_or_default()
            ),
            archive::ZipPhase::Packing => "Packing ZIP".to_string(),
            archive::ZipPhase::Finalizing => "Finalizing".to_string(),
        };
        bar_clone.set_message(label);
        if matches!(ev.phase, archive::ZipPhase::Packing) {
            bar_clone.set_length(ev.total as u64);
            bar_clone.set_position(ev.done as u64);
        } else {
            bar_clone.set_position(ev.done as u64);
        }
    };

    let report = create_zip(
        &provider.client(),
        assets,
        zip_path,
        &zip_stem(query),
        query,
        settings.max_concurrency,
        &tokio_util::sync::CancellationToken::new(),
        Some(Box::new(callback)),
    )
    .await;
    bar.finish_and_clear();
    report
}

fn zip_stem(query: &str) -> String {
    let cleaned: String = query
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    if cleaned.is_empty() {
        "get-svg".to_string()
    } else {
        format!("{cleaned}-svg")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_query_is_escaped() {
        let q = compose_category_query("Category:Icons");
        assert_eq!(q, "incategory:\"Icons\"");
        let q = compose_category_query("icons");
        assert_eq!(q, "incategory:\"icons\"");
        // Quotes in user input cannot break out of the incategory clause.
        let q = compose_category_query("x\" filemime:png");
        assert!(q.starts_with("incategory:\"x\\\""));
    }

    #[test]
    fn zip_stem_is_filesystem_safe() {
        let s = zip_stem("github logos & icons!");
        assert!(!s.contains('/'));
        assert!(!s.contains('!'));
        assert!(s.ends_with("-svg"));
        assert_eq!(zip_stem(""), "get-svg");
    }

    #[test]
    fn confirm_without_tty_requires_yes() {
        // When stdin is not a terminal (as in tests), --yes is mandatory.
        let err = confirm("Continue?", false);
        if !std::io::stdin().is_terminal() {
            assert!(err.is_err());
        }
        assert!(confirm("Continue?", true).unwrap());
    }

    #[test]
    fn debug_logging_is_off_by_default() {
        assert!(!crate::logging::debug_enabled());
    }
}
