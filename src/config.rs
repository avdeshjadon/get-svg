//! Configuration: `~/.config/get-svg/config.toml` (platform config dir).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Default batch concurrency. Conservative on purpose: GET SVG must stay
/// polite toward Wikimedia's shared infrastructure.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;
/// Minimum gap between the *starts* of two API requests, per process.
pub const DEFAULT_MIN_REQUEST_INTERVAL_MS: u64 = 250;
/// Default per-file cache TTL.
pub const DEFAULT_CACHE_TTL_HOURS: u64 = 24;
/// Upper bound on cached bytes on disk before the cache prunes itself.
pub const DEFAULT_CACHE_MAX_MB: u64 = 100;
/// Upper bound on a single API JSON response.
pub const DEFAULT_MAX_RESPONSE_MB: u64 = 16;

/// On-disk configuration file shape (all fields optional).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigFile {
    pub download_directory: Option<PathBuf>,
    pub max_concurrency: Option<usize>,
    pub cache_enabled: Option<bool>,
    pub cache_ttl_hours: Option<u64>,
    pub cache_max_mb: Option<u64>,
    pub theme: Option<String>,
    pub animations: Option<bool>,
    pub min_request_interval_ms: Option<u64>,
    pub max_response_mb: Option<u64>,
    /// Contact string appended to the User-Agent (recommended for heavy use).
    pub contact: Option<String>,
}

/// Effective settings after defaults are merged in.
#[derive(Debug, Clone)]
pub struct Settings {
    pub config_path: PathBuf,
    pub config_dir: PathBuf,
    pub download_dir: PathBuf,
    pub max_concurrency: usize,
    pub cache_enabled: bool,
    pub cache_ttl_hours: u64,
    pub cache_max_mb: u64,
    pub theme: String,
    pub animations: bool,
    pub min_request_interval_ms: u64,
    pub max_response_mb: u64,
    pub contact: Option<String>,
    /// Skip bulk download confirmations (driven by `--yes`).
    pub assume_yes: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let config_dir = config_dir();
        Settings {
            config_path: config_dir.join("config.toml"),
            config_dir: config_dir.clone(),
            download_dir: default_download_dir(),
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            cache_enabled: true,
            cache_ttl_hours: DEFAULT_CACHE_TTL_HOURS,
            cache_max_mb: DEFAULT_CACHE_MAX_MB,
            theme: "default".to_string(),
            animations: true,
            min_request_interval_ms: DEFAULT_MIN_REQUEST_INTERVAL_MS,
            max_response_mb: DEFAULT_MAX_RESPONSE_MB,
            contact: None,
            assume_yes: false,
        }
    }
}

/// `~/.config/get-svg` on Linux and `$XDG_CONFIG_HOME`; the platform config
/// directory elsewhere (e.g. `~/Library/Application Support` on macOS).
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("get-svg")
}

/// Default download directory: `~/Downloads/get-svg`.
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
        .join("get-svg")
}

impl Settings {
    /// Load config file if present, otherwise use defaults.
    pub fn load() -> Result<Settings> {
        Self::load_from(&config_dir().join("config.toml"))
    }

    pub fn load_from(path: &PathBuf) -> Result<Settings> {
        let mut s = Settings {
            config_path: path.clone(),
            config_dir: path.parent().map(PathBuf::from).unwrap_or_else(config_dir),
            ..Settings::default()
        };
        s.config_dir = path.parent().map(PathBuf::from).unwrap_or_else(config_dir);
        s.config_path = path.clone();

        if !path.exists() {
            return Ok(s);
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("could not read {}: {e}", path.display())))?;
        let file: ConfigFile = toml::from_str(&raw)
            .map_err(|e| Error::Config(format!("invalid TOML in {}: {e}", path.display())))?;

        if let Some(dir) = file.download_directory {
            s.download_dir = expand_tilde(dir);
        }
        if let Some(n) = file.max_concurrency {
            s.max_concurrency = n.clamp(1, 32);
        }
        if let Some(b) = file.cache_enabled {
            s.cache_enabled = b;
        }
        if let Some(h) = file.cache_ttl_hours {
            s.cache_ttl_hours = h.clamp(1, 24 * 365);
        }
        if let Some(mb) = file.cache_max_mb {
            s.cache_max_mb = mb.clamp(1, 10_240);
        }
        if let Some(t) = file.theme {
            s.theme = t;
        }
        if let Some(a) = file.animations {
            s.animations = a;
        }
        if let Some(ms) = file.min_request_interval_ms {
            s.min_request_interval_ms = ms.clamp(50, 60_000);
        }
        if let Some(mb) = file.max_response_mb {
            s.max_response_mb = mb.clamp(1, 1024);
        }
        if let Some(c) = file.contact {
            s.contact = Some(security_sanitize_contact(&c));
        }
        Ok(s)
    }

    /// Write the default configuration file if it does not exist.
    pub fn write_default_config(path: &PathBuf) -> Result<PathBuf> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if path.exists() {
            return Ok(path.clone());
        }
        let example = ConfigFile {
            download_directory: Some(default_download_dir()),
            max_concurrency: Some(DEFAULT_MAX_CONCURRENCY),
            cache_enabled: Some(true),
            cache_ttl_hours: Some(DEFAULT_CACHE_TTL_HOURS),
            cache_max_mb: Some(DEFAULT_CACHE_MAX_MB),
            theme: Some("default".to_string()),
            animations: Some(true),
            min_request_interval_ms: Some(DEFAULT_MIN_REQUEST_INTERVAL_MS),
            max_response_mb: Some(DEFAULT_MAX_RESPONSE_MB),
            contact: None,
        };
        let body = toml::to_string_pretty(&example)?;
        std::fs::write(path, body)?;
        Ok(path.clone())
    }

    /// The User-Agent every Wikimedia request must carry.
    ///
    /// Format per Wikimedia policy: `<client name>/<version> (<contact>)`.
    pub fn user_agent(&self) -> String {
        let repo = env!("CARGO_PKG_REPOSITORY");
        let version = env!("CARGO_PKG_VERSION");
        let contact = self
            .contact
            .clone()
            .unwrap_or_else(|| format!("{repo}; contact via repository issue tracker"));
        format!("GET-SVG/{version} ({repo}; {contact})")
    }

    /// The cache directory, created on demand.
    pub fn cache_dir(&self) -> PathBuf {
        self.config_dir.join("cache")
    }

    /// Recent searches file.
    pub fn recent_path(&self) -> PathBuf {
        self.config_dir.join("recent.json")
    }

    /// Absolute max bytes a single API response may occupy.
    pub fn max_response_bytes(&self) -> usize {
        (self.max_response_mb as usize).saturating_mul(1024 * 1024)
    }

    /// Config as TOML, for `get-svg config`.
    pub fn to_toml_string(&self) -> String {
        let file = ConfigFile {
            download_directory: Some(self.download_dir.clone()),
            max_concurrency: Some(self.max_concurrency),
            cache_enabled: Some(self.cache_enabled),
            cache_ttl_hours: Some(self.cache_ttl_hours),
            cache_max_mb: Some(self.cache_max_mb),
            theme: Some(self.theme.clone()),
            animations: Some(self.animations),
            min_request_interval_ms: Some(self.min_request_interval_ms),
            max_response_mb: Some(self.max_response_mb),
            contact: self.contact.clone(),
        };
        toml::to_string_pretty(&file).unwrap_or_default()
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    path
}

/// Contacts end up in HTTP headers; strip anything that could smuggle
/// control characters or newlines into a header value.
fn security_sanitize_contact(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_control() || c == '<' || c == '>' {
                '_'
            } else {
                c
            }
        })
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.max_concurrency, 4);
        assert!(s.cache_enabled);
        assert!(s.user_agent().starts_with("GET-SVG/"));
        assert!(s
            .user_agent()
            .contains("https://github.com/avdeshjadon/get-svg"));
    }

    #[test]
    fn parses_partial_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "max_concurrency = 8\ntheme = \"high_contrast\"\nanimations = false\n",
        )
        .unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s.max_concurrency, 8);
        assert_eq!(s.theme, "high_contrast");
        assert!(!s.animations);
        assert!(s.cache_enabled);
    }

    #[test]
    fn rejects_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "max_concurrency = \"lots\"").unwrap();
        assert!(Settings::load_from(&path).is_err());
    }

    #[test]
    fn contact_cannot_smuggle_headers() {
        let dirty = "me@x\nX-Evil: 1<>";
        let clean = security_sanitize_contact(dirty);
        assert!(!clean.contains('\n'));
        assert!(!clean.contains('\r'));
        assert!(!clean.contains('<'));
    }

    #[test]
    fn concurrency_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "max_concurrency = 9999\n").unwrap();
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s.max_concurrency, 32);
    }
}
