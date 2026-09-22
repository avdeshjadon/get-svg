//! Attribution and license metadata export.
//!
//! GET SVG treats licensing as per-asset metadata. It never assumes two
//! Wikimedia Commons files share a license, and never invents one.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::models::Asset;
use crate::security;

/// Attribution record for one downloaded file, as stored next to assets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionEntry {
    pub filename: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub license: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    pub downloaded_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_terms: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Machine-readable record of a file's license.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseEntry {
    pub filename: String,
    pub license: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_terms: Option<String>,
    pub verified: bool,
}

/// Machine-readable record of a file's origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceEntry {
    pub filename: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_url: Option<String>,
    pub provider: String,
}

/// Manifest describing a batch/ZIP export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub tool: String,
    pub version: String,
    pub query: String,
    pub created_at: String,
    pub file_count: usize,
    pub note: String,
}

const MANIFEST_NOTE: &str = "Files are unmodified originals from Wikimedia Commons. \
License information is per-file and was recorded at download time; verify it on the \
source page before reuse. GET SVG does not own or license any Wikimedia content.";

impl AttributionEntry {
    pub fn for_asset(asset: &Asset) -> AttributionEntry {
        let license = asset.license_or_unknown();
        let note = if asset.license.is_none() {
            Some(
                "License could not be determined from API metadata. \
Verify licensing on the Wikimedia Commons page before reuse."
                    .to_string(),
            )
        } else if asset.license_url.is_none() {
            Some("No license URL was provided by the API; check the source page.".to_string())
        } else {
            None
        };

        AttributionEntry {
            filename: security::sanitize_filename(&asset.file_name),
            source: "Wikimedia Commons".to_string(),
            source_url: asset.description_url.clone(),
            author: asset.author.clone(),
            license,
            license_url: asset.license_url.clone(),
            downloaded_at: Utc::now().to_rfc3339(),
            original_file_url: asset.url.clone(),
            title: Some(asset.title.clone()),
            attribution_text: asset.attribution.clone(),
            usage_terms: asset.usage_terms.clone(),
            note,
        }
    }
}

fn license_entry(asset: &Asset) -> LicenseEntry {
    LicenseEntry {
        filename: security::sanitize_filename(&asset.file_name),
        license: asset.license_or_unknown(),
        license_url: asset.license_url.clone(),
        usage_terms: asset.usage_terms.clone(),
        verified: asset.license.is_some() && asset.license_url.is_some(),
    }
}

fn source_entry(asset: &Asset) -> SourceEntry {
    SourceEntry {
        filename: security::sanitize_filename(&asset.file_name),
        title: asset.title.clone(),
        page_url: asset.description_url.clone(),
        original_file_url: asset.url.clone(),
        provider: "Wikimedia Commons".to_string(),
    }
}

/// The JSON document bodies that accompany a batch/ZIP export.
pub fn metadata_documents(query: &str, assets: &[Asset]) -> Vec<(String, String)> {
    let attribution: Vec<AttributionEntry> =
        assets.iter().map(AttributionEntry::for_asset).collect();
    let licenses: Vec<LicenseEntry> = assets.iter().map(license_entry).collect();
    let sources: Vec<SourceEntry> = assets.iter().map(source_entry).collect();
    let manifest = Manifest {
        tool: "GET SVG".to_string(),
        version: crate::VERSION.to_string(),
        query: query.to_string(),
        created_at: Utc::now().to_rfc3339(),
        file_count: assets.len(),
        note: MANIFEST_NOTE.to_string(),
    };

    vec![
        (
            "attribution.json".to_string(),
            serde_json::to_string_pretty(&attribution).unwrap_or_else(|_| "[]".into()),
        ),
        (
            "licenses.json".to_string(),
            serde_json::to_string_pretty(&licenses).unwrap_or_else(|_| "[]".into()),
        ),
        (
            "sources.json".to_string(),
            serde_json::to_string_pretty(&sources).unwrap_or_else(|_| "[]".into()),
        ),
        (
            "manifest.json".to_string(),
            serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".into()),
        ),
    ]
}

/// Write attribution files into `dir`, merging with any existing
/// `attribution.json` so repeated batches never drop earlier records.
pub fn write_metadata_dir(dir: &Path, query: &str, assets: &[Asset]) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;

    let attr_path = dir.join("attribution.json");
    let mut merged: Vec<AttributionEntry> = if attr_path.exists() {
        std::fs::read_to_string(&attr_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let new_entries: Vec<AttributionEntry> =
        assets.iter().map(AttributionEntry::for_asset).collect();
    for entry in &new_entries {
        merged.retain(|e| e.filename != entry.filename);
        merged.push(entry.clone());
    }

    let mut written = Vec::new();
    std::fs::write(&attr_path, serde_json::to_string_pretty(&merged)?)?;
    written.push(attr_path);

    for (name, body) in metadata_documents(query, assets) {
        if name == "attribution.json" {
            continue; // handled with merge semantics above
        }
        let path = dir.join(&name);
        std::fs::write(&path, body)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Asset;

    fn asset(name: &str, license: Option<&str>) -> Asset {
        Asset {
            title: format!("File:{name}"),
            page_id: 7,
            original_name: name.to_string(),
            file_name: name.to_string(),
            index: None,
            size_bytes: Some(100),
            mime: Some("image/svg+xml".into()),
            width: None,
            height: None,
            mediatype: None,
            url: Some(format!(
                "https://upload.wikimedia.org/wikipedia/commons/{name}"
            )),
            thumb_url: None,
            description_url: Some(format!("https://commons.wikimedia.org/wiki/File:{name}")),
            author: Some("Test Author".into()),
            uploader: None,
            license: license.map(str::to_string),
            license_url: license.map(|_| "https://creativecommons.org/licenses/by-sa/4.0/".into()),
            usage_terms: None,
            attribution: None,
            credit: None,
            description: None,
            categories: vec!["Icons".into()],
            uploaded_at: None,
            modified_at: None,
        }
    }

    #[test]
    fn attribution_never_invents_license() {
        let e = AttributionEntry::for_asset(&asset("x.svg", None));
        assert_eq!(e.license, "Unknown");
        assert!(e.note.is_some());
        assert!(e.license_url.is_none());
    }

    #[test]
    fn attribution_records_known_license() {
        let e = AttributionEntry::for_asset(&asset("x.svg", Some("CC BY-SA 4.0")));
        assert_eq!(e.license, "CC BY-SA 4.0");
        assert!(e.license_url.is_some());
        assert!(e.note.is_none());
        assert_eq!(e.source, "Wikimedia Commons");
        assert!(e.original_file_url.is_some());
    }

    #[test]
    fn documents_are_valid_json() {
        let docs = metadata_documents("github", &[asset("a.svg", Some("CC0"))]);
        assert_eq!(docs.len(), 4);
        for (name, body) in docs {
            assert!(
                serde_json::from_str::<serde_json::Value>(&body).is_ok(),
                "{name} invalid"
            );
        }
    }

    #[test]
    fn metadata_dir_merges_attribution() {
        let dir = tempfile::tempdir().unwrap();
        write_metadata_dir(dir.path(), "q1", &[asset("a.svg", None)]).unwrap();
        write_metadata_dir(
            dir.path(),
            "q2",
            &[asset("a.svg", Some("CC0")), asset("b.svg", None)],
        )
        .unwrap();
        let raw = std::fs::read_to_string(dir.path().join("attribution.json")).unwrap();
        let entries: Vec<AttributionEntry> = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            entries.len(),
            2,
            "duplicates must be replaced, not appended"
        );
        assert!(dir.path().join("licenses.json").exists());
        assert!(dir.path().join("manifest.json").exists());
    }
}
