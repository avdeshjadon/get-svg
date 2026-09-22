//! Provider abstraction.
//!
//! The rest of the application only talks to this trait, so another public
//! SVG source can be added later without touching the UI or download code.

use crate::error::Result;
use crate::models::{Asset, SearchPage};

/// A source of SVG assets.
pub trait AssetProvider: Send + Sync + 'static {
    /// Provider display name, e.g. `Wikimedia Commons`.
    fn provider_name(&self) -> &'static str;

    /// Search for SVG assets. `offset` is a stable pagination cursor.
    async fn search(&self, query: &str, offset: u64, limit: u32) -> Result<SearchPage>;

    /// Fetch full metadata for a single title (`File:Name.svg`).
    async fn get_asset(&self, title: &str) -> Result<Option<Asset>>;

    /// Fetch full metadata for many titles in as few requests as practical.
    async fn get_assets(&self, titles: &[String]) -> Result<Vec<Asset>>;
}
