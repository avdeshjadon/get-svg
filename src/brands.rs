//! Curated brand registry and resolution for direct, single-logo downloads.

use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrandInfo {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrandFile {
    #[serde(default)]
    brands: HashMap<String, BrandInfo>,
}

pub struct BrandRegistry {
    brands: HashMap<String, BrandInfo>,
}

impl BrandRegistry {
    /// Load the built-in curated brands and merge any user-specific overrides.
    pub fn load() -> Self {
        let mut map: HashMap<String, BrandInfo> = HashMap::new();

        // 1. Built-in curated registry
        const EMBEDDED: &str = include_str!("../brands.json");
        if let Ok(parsed) = serde_json::from_str::<BrandFile>(EMBEDDED) {
            map.extend(parsed.brands);
        }

        // 2. Check user's custom local ~/.config/get-svg/brands.json if present
        if let Some(user_config) = user_brands_path() {
            if user_config.exists() {
                if let Ok(content) = std::fs::read_to_string(&user_config) {
                    if let Ok(parsed) = serde_json::from_str::<BrandFile>(&content) {
                        map.extend(parsed.brands);
                    }
                }
            }
        }

        Self { brands: map }
    }

    /// Resolve a query string to a BrandInfo.
    /// Matches exact key, or any alias, case-insensitively.
    pub fn resolve(&self, query: &str) -> Option<&BrandInfo> {
        let norm = normalize(query);
        if norm.is_empty() {
            return None;
        }

        // 1. Exact normalized key match
        if let Some(b) = self.brands.get(&norm) {
            return Some(b);
        }

        // 2. Check aliases across all brands
        for (_, brand) in &self.brands {
            for alias in &brand.aliases {
                if normalize(alias) == norm {
                    return Some(brand);
                }
            }
        }

        // 3. Normalized brand name match
        for (_, brand) in &self.brands {
            if normalize(&brand.name) == norm {
                return Some(brand);
            }
        }

        None
    }
}

/// Normalize queries and brand names for robust matching.
pub fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .replace(['-', '_', '.', '\'', '"'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate a clean filesystem slug for downloaded SVG filenames (e.g. "amazon-prime.svg").
pub fn slugify(name: &str) -> String {
    let clean: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    
    // Collapse consecutive dashes
    let mut out = String::new();
    let mut prev_dash = false;
    for c in clean.chars() {
        if c == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(c);
            prev_dash = false;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "logo".to_string()
    } else {
        trimmed.to_string()
    }
}

fn user_brands_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("get-svg").join("brands.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_resolves_amazon() {
        let reg = BrandRegistry::load();
        let amazon = reg.resolve("amazon").expect("amazon should resolve");
        assert_eq!(amazon.name, "Amazon");
        assert_eq!(amazon.file, "File:Amazon logo.svg");
    }

    #[test]
    fn test_registry_resolves_amazon_prime() {
        let reg = BrandRegistry::load();
        let prime = reg.resolve("amazon prime").expect("amazon prime should resolve");
        assert_eq!(prime.name, "Amazon Prime");
        assert_eq!(prime.file, "File:Amazon Prime Logo.svg");

        // Alias test
        let prime_alias = reg.resolve("prime video").expect("prime video should resolve");
        assert_eq!(prime_alias.name, "Amazon Prime");
    }

    #[test]
    fn test_registry_resolves_amazon_music() {
        let reg = BrandRegistry::load();
        let music = reg.resolve("amazon music").expect("amazon music should resolve");
        assert_eq!(music.name, "Amazon Music");
        assert_eq!(music.file, "File:Amazon Music (Logo).svg");

        // Dashed format
        let music_dash = reg.resolve("amazon-music").expect("amazon-music should resolve");
        assert_eq!(music_dash.name, "Amazon Music");
    }

    #[test]
    fn test_registry_resolves_aws() {
        let reg = BrandRegistry::load();
        let aws = reg.resolve("aws").expect("aws alias should resolve");
        assert_eq!(aws.name, "Amazon Web Services");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Amazon"), "amazon");
        assert_eq!(slugify("Amazon Prime"), "amazon-prime");
        assert_eq!(slugify("Amazon Music (Logo)"), "amazon-music-logo");
    }
}
