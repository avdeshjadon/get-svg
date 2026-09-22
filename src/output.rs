//! Machine- and human-readable output. Serialization lives here so business
//! logic never formats its own output.

use std::io::Write;

use serde::Serialize;

use crate::error::Result;
use crate::models::{format_size, Asset};
use crate::security;

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Pretty table (default for non-interactive commands).
    Table,
    /// Single JSON document.
    Json,
    /// One JSON object per line.
    Jsonl,
}

impl OutputFormat {
    /// Resolve `--format auto` based on TTY detection.
    pub fn auto() -> OutputFormat {
        OutputFormat::Table
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OutputFormat::Table => "table",
            OutputFormat::Json => "json",
            OutputFormat::Jsonl => "jsonl",
        };
        f.write_str(s)
    }
}

/// Envelope for JSON output so scripts get stable, documented fields.
#[derive(Debug, Serialize)]
struct JsonEnvelope<'a> {
    tool: &'static str,
    version: &'static str,
    query: &'a str,
    total_hits: Option<u64>,
    count: usize,
    results: &'a [Asset],
}

/// Print assets in the requested format to `out`.
pub fn write_assets(
    out: &mut dyn Write,
    format: OutputFormat,
    query: &str,
    total_hits: Option<u64>,
    assets: &[Asset],
) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let envelope = JsonEnvelope {
                tool: "GET SVG",
                version: crate::VERSION,
                query,
                total_hits,
                count: assets.len(),
                results: assets,
            };
            writeln!(out, "{}", serde_json::to_string_pretty(&envelope)?)?;
        }
        OutputFormat::Jsonl => {
            for asset in assets {
                writeln!(out, "{}", serde_json::to_string(asset)?)?;
            }
        }
        OutputFormat::Table => {
            write_table(out, assets)?;
        }
    }
    Ok(())
}

/// A single asset as a one-line summary (used by `download`).
pub fn write_asset_record(out: &mut dyn Write, format: OutputFormat, asset: &Asset) -> Result<()> {
    let assets = [asset.clone()];
    write_assets(out, format, &asset.title, None, &assets)
}

fn write_table(out: &mut dyn Write, assets: &[Asset]) -> Result<()> {
    if assets.is_empty() {
        writeln!(out, "No results.")?;
        return Ok(());
    }

    // Columns: #, NAME, SIZE, TYPE, LICENSE
    let mut rows: Vec<[String; 5]> = Vec::with_capacity(assets.len());
    for (i, a) in assets.iter().enumerate() {
        rows.push([
            (i + 1).to_string(),
            security::sanitize_text(&a.original_name),
            a.size_human(),
            mime_short(a),
            security::sanitize_text(&a.license_or_unknown()),
        ]);
    }

    let headers = ["#", "NAME", "SIZE", "TYPE", "LICENSE"];
    let mut widths = [headers.iter().map(|h| h.len()).max().unwrap_or(1); 5];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(display_width(cell));
        }
    }
    // Clamp name column so 80-column terminals stay usable.
    widths[1] = widths[1].min(60);

    let mut sep = String::new();
    sep.push('+');
    for w in widths {
        sep.push_str(&"-".repeat(w + 2));
        sep.push('+');
    }
    sep.push('\n');

    writeln!(out, "{}", sep.trim_end())?;
    write_row(out, &headers.map(String::from), &widths)?;
    writeln!(out, "{}", sep.trim_end())?;
    for row in &rows {
        write_row(out, row, &widths)?;
    }
    writeln!(out, "{}", sep.trim_end())?;
    Ok(())
}

fn write_row(out: &mut dyn Write, cells: &[String], widths: &[usize]) -> Result<()> {
    let mut line = String::new();
    line.push_str("| ");
    for (i, cell) in cells.iter().enumerate() {
        let w = widths[i];
        let visible = truncate_visible(cell, w);
        line.push_str(&visible);
        for _ in visible.chars().count()..w {
            line.push(' ');
        }
        line.push_str(" | ");
    }
    writeln!(out, "{}", line.trim_end())?;
    Ok(())
}

fn display_width(s: &str) -> usize {
    // Treat everything as single-width; good enough for identifiers/filenames
    // and avoids pulling in a unicode-width dependency.
    s.chars().count()
}

fn truncate_visible(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let take = max.saturating_sub(1);
        let mut out: String = s.chars().take(take).collect();
        out.push('…');
        out
    }
}

fn mime_short(asset: &Asset) -> String {
    match asset.mime.as_deref() {
        Some("image/svg+xml") => "SVG".to_string(),
        Some(m) if m.starts_with("image/") => m.trim_start_matches("image/").to_ascii_uppercase(),
        _ => "SVG".to_string(),
    }
}

/// Friendly one-line progress bar used in non-interactive mode.
pub fn progress_line(done: u64, total: u64) -> String {
    if total == 0 {
        return "Working...".to_string();
    }
    let pct = ((done as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
    let width = 30usize;
    let filled = (pct / 100.0 * width as f64).round() as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(width - filled);
    format!(
        "{bar} {pct:3.0}%  {}/{}",
        format_size(done),
        format_size(total)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Asset {
        Asset {
            title: "File:GitHub_Logo.svg".into(),
            page_id: 1,
            original_name: "GitHub_Logo.svg".into(),
            file_name: "GitHub_Logo.svg".into(),
            index: Some(0),
            size_bytes: Some(12_400),
            mime: Some("image/svg+xml".into()),
            width: Some(120),
            height: Some(120),
            mediatype: None,
            url: Some("https://upload.wikimedia.org/x/GitHub_Logo.svg".into()),
            thumb_url: None,
            description_url: Some("https://commons.wikimedia.org/wiki/File:GitHub_Logo.svg".into()),
            author: Some("GitHub".into()),
            uploader: None,
            license: Some("CC BY-SA 4.0".into()),
            license_url: Some("https://creativecommons.org/licenses/by-sa/4.0/".into()),
            usage_terms: None,
            attribution: None,
            credit: None,
            description: None,
            categories: vec![],
            uploaded_at: None,
            modified_at: None,
        }
    }

    #[test]
    fn json_is_machine_readable_and_ansi_free() {
        let mut buf = Vec::new();
        write_assets(
            &mut buf,
            OutputFormat::Json,
            "github",
            Some(127),
            &[sample()],
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains('\u{1b}'), "ANSI leaked into JSON");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["total_hits"], 127);
        assert_eq!(v["results"][0]["original_name"], "GitHub_Logo.svg");
    }

    #[test]
    fn jsonl_is_one_object_per_line() {
        let mut buf = Vec::new();
        write_assets(
            &mut buf,
            OutputFormat::Jsonl,
            "q",
            None,
            &[sample(), sample()],
        )
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for l in lines {
            serde_json::from_str::<serde_json::Value>(l).unwrap();
        }
    }

    #[test]
    fn table_contains_expected_columns() {
        let mut buf = Vec::new();
        write_assets(&mut buf, OutputFormat::Table, "q", None, &[sample()]).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("NAME"));
        assert!(text.contains("LICENSE"));
        assert!(text.contains("GitHub_Logo.svg"));
        assert!(text.contains("12.1 KB"));
    }

    #[test]
    fn empty_table_is_graceful() {
        let mut buf = Vec::new();
        write_assets(&mut buf, OutputFormat::Table, "q", None, &[]).unwrap();
        assert!(String::from_utf8(buf).unwrap().contains("No results"));
    }

    #[test]
    fn sanitizes_terminal_escapes_in_table() {
        let mut evil = sample();
        evil.original_name = "\x1b[31mEvil\x1b[0m.svg".into();
        let mut buf = Vec::new();
        write_assets(&mut buf, OutputFormat::Table, "q", None, &[evil]).unwrap();
        assert!(!String::from_utf8(buf).unwrap().contains('\u{1b}'));
    }
}
