//! `get-svg doctor` — environment checks with actionable output.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::api::wikimedia::API_ENDPOINT;
use crate::config::Settings;
use crate::error::Result;

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn symbol(self) -> &'static str {
        match self {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
    pub healthy: bool,
}

/// Run all checks.
pub async fn run(settings: &Settings) -> Result<Report> {
    let mut checks = Vec::new();

    // 1. Config file
    let (status, detail, hint) = if settings.config_path.exists() {
        (Status::Ok, settings.config_path.display().to_string(), None)
    } else {
        (
            Status::Warn,
            format!(
                "{} (not found; using defaults)",
                settings.config_path.display()
            ),
            Some("Run `get-svg config --init` to create a config file.".into()),
        )
    };
    checks.push(Check {
        name: "Config file".into(),
        status,
        detail,
        hint,
    });

    // 2. Config directory writable
    let (status, detail, hint) = match std::fs::create_dir_all(&settings.config_dir) {
        Ok(()) => (Status::Ok, settings.config_dir.display().to_string(), None),
        Err(e) => (
            Status::Fail,
            format!("{} ({e})", settings.config_dir.display()),
            Some("Fix permissions on the config directory.".into()),
        ),
    };
    checks.push(Check {
        name: "Config directory".into(),
        status,
        detail,
        hint,
    });

    // 3. Download directory
    let (status, detail, hint) = match std::fs::create_dir_all(&settings.download_dir) {
        Ok(()) => {
            if let Some(space) = crate::security::available_space(&settings.download_dir) {
                let free = crate::models::format_size(space);
                (
                    Status::Ok,
                    format!("{} ({free} free)", settings.download_dir.display()),
                    None,
                )
            } else {
                (
                    Status::Ok,
                    settings.download_dir.display().to_string(),
                    None,
                )
            }
        }
        Err(e) => (
            Status::Fail,
            format!("{} ({e})", settings.download_dir.display()),
            Some("Set download_directory in config.toml to a writable path.".into()),
        ),
    };
    checks.push(Check {
        name: "Download directory".into(),
        status,
        detail,
        hint,
    });

    // 4. Cache
    let cache = crate::cache::Cache::new(settings);
    let st = cache.status();
    let (status, detail) = if st.enabled {
        (
            Status::Ok,
            format!(
                "{} ({} files, {} / {} used)",
                st.directory.display(),
                st.file_count,
                crate::models::format_size(st.total_bytes),
                crate::models::format_size(st.max_bytes)
            ),
        )
    } else {
        (Status::Warn, "disabled in config".to_string())
    };
    checks.push(Check {
        name: "Cache".into(),
        status,
        detail,
        hint: None,
    });

    // 5. Temp directory writable
    let (status, detail) = match tempfile::tempdir() {
        Ok(d) => (Status::Ok, d.path().display().to_string()),
        Err(e) => (Status::Fail, format!("temp dir not usable: {e}")),
    };
    checks.push(Check {
        name: "Temp directory".into(),
        status,
        detail,
        hint: None,
    });

    // 6. Terminal
    use std::io::IsTerminal;
    let (status, detail, hint) = if std::io::stdout().is_terminal() {
        (Status::Ok, "interactive TTY available".to_string(), None)
    } else {
        (
            Status::Warn,
            "stdout is not a TTY; interactive mode unavailable".to_string(),
            Some(
                "Run inside a terminal for the full experience; use `get-svg search` in scripts."
                    .into(),
            ),
        )
    };
    checks.push(Check {
        name: "Terminal".into(),
        status,
        detail,
        hint,
    });

    // 7. Network reachability
    let (status, detail, hint) = check_network(settings).await;
    checks.push(Check {
        name: "Wikimedia API".into(),
        status,
        detail,
        hint,
    });

    // 8. User-Agent
    checks.push(Check {
        name: "User-Agent".into(),
        status: Status::Ok,
        detail: settings.user_agent(),
        hint: Some(
            "Set `contact` in config.toml so Wikimedia can reach you about heavy usage.".into(),
        ),
    });

    let healthy = !checks.iter().any(|c| c.status == Status::Fail);
    Ok(Report { checks, healthy })
}

async fn check_network(settings: &Settings) -> (Status, String, Option<String>) {
    let client = match reqwest::Client::builder()
        .user_agent(settings.user_agent())
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .https_only(true)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                Status::Fail,
                format!("could not build HTTP client: {e}"),
                None,
            )
        }
    };

    // A tiny, cache-busted request that reports the API's view of us.
    let url = format!("{API_ENDPOINT}?action=query&format=json&formatversion=2&meta=siteinfo");
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            (Status::Ok, format!("reachable ({})", resp.status()), None)
        }
        Ok(resp) => (
            Status::Warn,
            format!("reachable but returned {}", resp.status()),
            Some("Wikimedia may be rate limiting or down; retry later.".into()),
        ),
        Err(e) if e.is_timeout() => (
            Status::Fail,
            "connection timed out".into(),
            Some("Check your internet connection, proxy, or firewall.".into()),
        ),
        Err(e) if e.is_connect() => (
            Status::Fail,
            "could not connect".into(),
            Some("Check your internet connection, proxy, or firewall.".into()),
        ),
        Err(e) => (Status::Fail, format!("request failed: {e}"), None),
    }
}

/// Render the report as a human-readable block.
pub fn render(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("GET SVG doctor\n");
    out.push_str(&"=".repeat(60));
    out.push('\n');
    for check in &report.checks {
        out.push_str(&format!("[{}] {}\n", check.status.symbol(), check.name));
        out.push_str(&format!("        {}\n", check.detail));
        if let Some(hint) = &check.hint {
            if check.status != Status::Ok {
                out.push_str(&format!("        hint: {hint}\n"));
            }
        }
    }
    out.push_str(&"=".repeat(60));
    out.push('\n');
    if report.healthy {
        out.push_str("All critical checks passed.\n");
    } else {
        out.push_str("Some checks failed. Fix the items marked FAIL above.\n");
    }
    out
}

/// Ensure a path exists (used by tests and setup helpers).
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_all_checks() {
        let report = Report {
            checks: vec![Check {
                name: "Config file".into(),
                status: Status::Warn,
                detail: "missing".into(),
                hint: Some("run --init".into()),
            }],
            healthy: true,
        };
        let text = render(&report);
        assert!(text.contains("Config file"));
        assert!(text.contains("run --init"));
        assert!(text.contains("All critical checks passed"));
    }
}
