use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::security;

/// A single SVG asset discovered on Wikimedia Commons.
///
/// Every optional field reflects that Wikimedia metadata is heterogeneous:
/// some files carry rich extmetadata, some carry almost nothing. `None` is
/// rendered as "Unknown" in human output and omitted in JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Asset {
    /// Full MediaWiki title, e.g. `File:GitHub_Logo.svg`.
    pub title: String,
    /// MediaWiki page id.
    pub page_id: u64,
    /// Original name as stored on Wikimedia Commons.
    pub original_name: String,
    /// Filesystem-safe name used when writing to disk.
    pub file_name: String,
    /// Search relevance rank (page offset), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,

    /// Direct download URL (original file, served from upload.wikimedia.org).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    /// Wikimedia Commons file description page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_terms: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<DateTime<Utc>>,
}

impl Asset {
    /// Parse an Asset out of a MediaWiki `formatversion=2` JSON object.
    pub fn from_api_json(v: &serde_json::Value) -> Option<Asset> {
        let title = v.get("title")?.as_str()?.to_string();
        let page_id = v.get("pageid")?.as_u64()?;
        let file_name = security::file_name_from_title(&title);

        let ii = v.get("imageinfo");
        // formatversion=2 returns an object; be liberal and accept an array too.
        let info = match ii {
            Some(serde_json::Value::Object(map)) => Some(serde_json::Value::Object(map.clone())),
            Some(serde_json::Value::Array(arr)) => arr.first().cloned(),
            _ => None,
        };

        let get = |key: &str| -> Option<String> {
            info.as_ref()?
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let get_num = |key: &str| -> Option<u64> {
            info.as_ref()?.get(key).and_then(serde_json::Value::as_u64)
        };

        let ext = info
            .as_ref()
            .and_then(|i| i.get("extmetadata"))
            .and_then(serde_json::Value::as_object);
        let em_str = |key: &str| -> Option<String> {
            ext.and_then(|m| m.get(key))
                .and_then(|o| o.get("value"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(security::strip_markup)
        };

        let categories = em_str("Categories")
            .map(|raw| {
                raw.lines()
                    .map(str::trim)
                    .filter_map(|line| line.trim_start_matches('*').trim().to_string().into())
                    .filter(|c: &String| !c.is_empty())
                    .take(50)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let license = em_str("LicenseShortName");
        let license_url = em_str("LicenseUrl").or_else(|| {
            if license.is_some() {
                // Do not invent a license URL; only surface what the API gave us.
                None
            } else {
                None
            }
        });

        Some(Asset {
            title,
            page_id,
            file_name: security::sanitize_filename(&file_name),
            original_name: file_name,
            index: v.get("index").and_then(serde_json::Value::as_u64),
            size_bytes: get_num("size"),
            mime: get("mime"),
            width: get_num("width").and_then(|n| u32::try_from(n).ok()),
            height: get_num("height").and_then(|n| u32::try_from(n).ok()),
            mediatype: get("mediatype"),
            url: get("url"),
            thumb_url: get("thumburl").or_else(|| get("url")),
            description_url: get("descriptionurl"),
            author: em_str("Artist")
                .or_else(|| em_str("Author"))
                .or_else(|| get("user"))
                .map(|s| security::sanitize_text(&s)),
            uploader: get("user"),
            license: license
                .filter(|l| !l.eq_ignore_ascii_case("none"))
                .filter(|l| !l.is_empty()),
            license_url: license_url.filter(|u| security::validate_https_url(u)),
            usage_terms: em_str("UsageTerms"),
            attribution: em_str("Attribution"),
            credit: em_str("Credit").filter(|c| !c.is_empty()),
            description: em_str("ImageDescription")
                .or_else(|| em_str("ObjectName"))
                .map(|s| security::sanitize_text(&s)),
            categories,
            uploaded_at: get("timestamp")
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
            modified_at: get("timestamp")
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
        })
    }

    /// Human readable size, e.g. `12.4 KB`.
    pub fn size_human(&self) -> String {
        format_size(self.size_bytes.unwrap_or(0))
    }

    /// License display value; never claims a default license.
    pub fn license_or_unknown(&self) -> String {
        match &self.license {
            Some(l) => l.clone(),
            None => "Unknown".to_string(),
        }
    }

    /// Dimensions as `128x128`, or `Unknown`.
    pub fn dimensions_human(&self) -> String {
        match (self.width, self.height) {
            (Some(w), Some(h)) => format!("{w}x{h}"),
            _ => "Unknown".to_string(),
        }
    }

    /// The Commons file description page, if the API supplied one.
    pub fn page_url(&self) -> Option<&str> {
        self.description_url.as_deref()
    }
}

/// A single page of search results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchPage {
    pub query: String,
    pub offset: u64,
    pub per_page: u64,
    /// Total hits reported by the API (may be `None` when unavailable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_hits: Option<u64>,
    pub assets: Vec<Asset>,
}

impl SearchPage {
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

/// Format a byte count the way humans read it.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Format a duration in milliseconds compactly.
pub fn format_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_scales() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(12_698), "12.4 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn asset_parses_from_minimal_json() {
        let v = serde_json::json!({
            "pageid": 123,
            "ns": 6,
            "title": "File:Example.svg",
            "imageinfo": {
                "url": "https://upload.wikimedia.org/wikipedia/commons/1/1f/Example.svg",
                "size": 1024,
                "mime": "image/svg+xml",
                "width": 100,
                "height": 50,
                "descriptionurl": "https://commons.wikimedia.org/wiki/File:Example.svg",
                "timestamp": "2024-01-02T03:04:05Z"
            }
        });
        let asset = Asset::from_api_json(&v).expect("asset should parse");
        assert_eq!(asset.page_id, 123);
        assert_eq!(asset.title, "File:Example.svg");
        assert_eq!(asset.file_name, "Example.svg");
        assert_eq!(asset.size_bytes, Some(1024));
        assert_eq!(asset.width, Some(100));
        assert!(asset.url.is_some());
        assert!(asset.license.is_none());
    }

    #[test]
    fn asset_handles_missing_imageinfo() {
        let v = serde_json::json!({ "pageid": 1, "title": "File:X.svg" });
        let asset = Asset::from_api_json(&v).expect("should parse without imageinfo");
        assert!(asset.size_bytes.is_none());
        assert_eq!(asset.license_or_unknown(), "Unknown");
    }
}
