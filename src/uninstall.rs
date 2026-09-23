//! `getsvg dlt` — completely remove GET SVG from the system.
//!
//! Deletes everything the app owns so a fresh `getsvg` behaves like a brand
//! new install:
//! * the config directory (config.toml + cache + recent searches),
//! * the default download folder (`~/Downloads/get-svg`),
//! * the installed `get-svg` / `getsvg` binaries (including the running one).

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::config::Settings;
use crate::error::{Error, Result};

/// The download folder is only wiped automatically when it is the default
/// `get-svg` folder. A custom `download_directory` may point anywhere, so it
/// is left alone (and reported) instead of risking unrelated user data.
const DEFAULT_DOWNLOAD_NAME: &str = "get-svg";

/// What would be removed, computed up front so we can show it and confirm.
#[derive(Debug, Clone)]
pub struct UninstallPlan {
    pub config_dir: Option<PathBuf>,
    pub download_dir: Option<PathBuf>,
    pub binaries: Vec<PathBuf>,
}

impl UninstallPlan {
    /// Everything about to be deleted, oldest first (config → downloads → bins).
    pub fn targets(&self) -> Vec<&Path> {
        let mut out = Vec::new();
        if let Some(p) = &self.config_dir {
            out.push(p.as_path());
        }
        if let Some(p) = &self.download_dir {
            out.push(p.as_path());
        }
        out.extend(self.binaries.iter().map(PathBuf::as_path));
        out
    }
}

/// Derive the plan (no filesystem writes).
pub fn collect_plan(settings: &Settings) -> UninstallPlan {
    collect_plan_with_exe(settings, std::env::current_exe().ok().as_deref())
}

/// Same as [`collect_plan`], but the running binary is supplied explicitly so
/// tests can pin down binary discovery without depending on the ambient test
/// environment (which varies across platforms and toolchains).
fn collect_plan_with_exe(settings: &Settings, exe: Option<&Path>) -> UninstallPlan {
    let ext = if cfg!(windows) { ".exe" } else { "" };

    let config_dir = settings.config_dir.clone();
    let config_dir = (config_dir.exists()).then_some(config_dir);

    let download_dir = (settings.download_dir.exists()
        && settings.download_dir != settings.config_dir
        && settings
            .download_dir
            .file_name()
            .map(|n| n == DEFAULT_DOWNLOAD_NAME)
            .unwrap_or(false))
    .then_some(settings.download_dir.clone());

    let mut binaries = Vec::new();
    if let Some(exe) = exe {
        if let Some(dir) = exe.parent() {
            for name in ["get-svg", "getsvg"] {
                let path = dir.join(format!("{name}{ext}"));
                if path.exists() {
                    binaries.push(path);
                }
                let stale = dir.join(format!("{name}{ext}.old"));
                if stale.exists() {
                    binaries.push(stale);
                }
            }
        }
    }

    UninstallPlan {
        config_dir,
        download_dir,
        binaries,
    }
}

/// Run the full uninstall. Returns a process exit code.
pub fn run_uninstall(yes: bool) -> Result<i32> {
    let settings = Settings::load().unwrap_or_else(|_| Settings::default());
    let plan = collect_plan(&settings);

    if plan.targets().is_empty() {
        println!("Nothing to remove — GET SVG already leaves no trace on this system.");
        return Ok(0);
    }

    println!("✗ GET SVG uninstall");
    println!();
    if let Some(p) = &plan.config_dir {
        print_target("\u{2022} config", p, "settings, cache, recent searches");
    }
    if let Some(p) = &plan.download_dir {
        print_target("\u{2022} downloads", p, "SVGs downloaded by get-svg");
    }
    for b in &plan.binaries {
        print_target("\u{2022} binary", b, "get-svg / getsvg");
    }
    if plan.download_dir.is_none() && settings.download_dir.exists() {
        println!();
        println!(
            "  \u{23fa} custom download directory left untouched (not named \"get-svg\"):\n     {}",
            settings.download_dir.display()
        );
        println!("     Delete it manually if you also want those files removed.");
    }
    println!();

    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(Error::Other(
                "uninstall requires confirmation but stdin is not a TTY; pass --yes to proceed."
                    .into(),
            ));
        }
        eprint!("Are you sure? This permanently deletes the files above. [y/N] ");
        std::io::stderr().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Uninstall cancelled.");
            return Ok(0);
        }
    }

    remove_dir_report(&plan.config_dir, "config");
    remove_dir_report(&plan.download_dir, "downloads");

    if let Ok(cur) = std::env::current_exe() {
        for bin in &plan.binaries {
            if bin == &cur {
                remove_running_binary(bin);
            } else {
                let removed = std::fs::remove_file(bin).is_ok();
                println!(
                    "{} {}",
                    if removed {
                        "\u{2713} removed"
                    } else {
                        "\u{2013} note"
                    },
                    bin.display()
                );
            }
        }
    } else {
        for bin in &plan.binaries {
            let removed = std::fs::remove_file(bin).is_ok();
            println!(
                "{} {}",
                if removed {
                    "\u{2713} removed"
                } else {
                    "\u{2013} note"
                },
                bin.display()
            );
        }
    }

    println!();
    println!("✓ GET SVG has been completely removed from this system.");
    println!("  Reinstall anytime with:");
    println!(
        "    curl -fsSL https://raw.githubusercontent.com/avdeshjadon/get-svg/main/install.sh | sh"
    );
    Ok(0)
}

fn print_target(label: &str, path: &Path, what: &str) {
    println!("  {label}  {:<34}  \u{2013} {what}", path.display());
}

fn remove_dir_report(dir: &Option<PathBuf>, label: &str) {
    match dir {
        None => println!("  \u{2013} {label} already gone"),
        Some(p) => {
            let res = std::fs::remove_dir_all(p);
            match res {
                Ok(()) => println!("\u{2713} removed {label}  {}", p.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("  \u{2013} {label} already gone")
                }
                Err(e) => println!("\u{2017} could not remove {label}  {}: {e}", p.display()),
            }
        }
    }
}

/// The running binary cannot be deleted while executing on Windows, so we
/// schedule a detached, delayed delete there. On Unix unlinking the file is
/// safe — the inode lives until this process exits.
fn remove_running_binary(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::fs::remove_file(path).or_else(|_| {
            std::process::Command::new("cmd")
                .creation_flags(0x0800_0000 | 0x0000_0008) // CREATE_NO_WINDOW | DETACHED_PROCESS
                .args([
                    "/C",
                    "ping -n 3 127.0.0.1 >nul & del /q",
                    &path.to_string_lossy(),
                ])
                .spawn()
                .map(|_| ())
        });
    }
    #[cfg(not(windows))]
    {
        let removed = std::fs::remove_file(path).is_ok();
        println!(
            "{} {}",
            if removed {
                "\u{2713} removed"
            } else {
                "\u{2013} note"
            },
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(config_dir: &Path, download_dir: &Path) -> Settings {
        Settings {
            config_dir: config_dir.to_path_buf(),
            download_dir: download_dir.to_path_buf(),
            ..Settings::default()
        }
    }

    #[test]
    fn default_folders_are_in_plan() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("get-svg");
        let dl = dir.path().join("get-svg");
        std::fs::create_dir_all(&cfg).unwrap();
        let plan = collect_plan_with_exe(&settings_with(&cfg, &dl), None);
        assert!(plan.config_dir.is_some());
        assert!(plan.download_dir.is_none()); // same path as config dir -> deduped
    }

    #[test]
    fn distinct_get_svg_downloads_included() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg").join("get-svg");
        let dl = dir.path().join("dl").join("get-svg");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(&dl).unwrap();
        let plan = collect_plan_with_exe(&settings_with(&cfg, &dl), None);
        assert!(plan.config_dir.is_some());
        assert!(plan.download_dir.is_some());
    }

    #[test]
    fn custom_download_dir_is_never_wiped() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("get-svg");
        let custom = dir.path().join("my-icons");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(&custom).unwrap();
        let plan = collect_plan_with_exe(&settings_with(&cfg, &custom), None);
        assert!(plan.download_dir.is_none());
    }

    #[test]
    fn no_trace_yields_empty_plan() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("get-svg");
        let plan = collect_plan_with_exe(&settings_with(&cfg, &cfg), None);
        assert!(plan.targets().is_empty());
    }

    #[test]
    fn running_binary_and_siblings_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let bindir = dir.path().join("bin");
        std::fs::create_dir_all(&bindir).unwrap();
        let ext = if cfg!(windows) { ".exe" } else { "" };
        for name in ["get-svg", "getsvg"] {
            std::fs::write(bindir.join(format!("{name}{ext}")), b"").unwrap();
        }
        let exe = bindir.join(format!("get-svg{ext}"));
        let plan = collect_plan_with_exe(&settings_with(&bindir, &bindir), Some(&exe));
        let names: Vec<String> = plan
            .binaries
            .iter()
            .map(|b| b.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![format!("get-svg{ext}"), format!("getsvg{ext}")]);
    }

    #[test]
    fn plan_targets_are_lowercased_safe() {
        let plan = UninstallPlan {
            config_dir: Some(PathBuf::from("/x/get-svg")),
            download_dir: None,
            binaries: vec![PathBuf::from("/x/bin/get-svg")],
        };
        let targets = plan.targets();
        assert!(targets
            .iter()
            .all(|p| p.to_string_lossy().contains("get-svg")));
    }
}
