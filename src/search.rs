//! Search orchestration: cache-aware page fetching shared by the TUI and
//! the non-interactive commands.

use crate::api::provider::AssetProvider;
use crate::cache::{Cache, NS_SEARCH};
use crate::error::Result;
use crate::models::SearchPage;

/// A search page plus whether it came from the local cache.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub page: SearchPage,
    pub from_cache: bool,
}

/// Cache key for a query/offset pair.
fn cache_key(query: &str, offset: u64) -> String {
    // Normalize the *effective* search string (title-boost etc.) into the key
    // so that changing ranking semantics busts stale cache entries instead of
    // silently serving old results.
    let boost = api::wikimedia::WikimediaClient::compose_search(query, None)
        .trim()
        .to_lowercase();
    // Fall back through whitespace collapse just in case the boost was a no-op.
    let normalized = if boost.is_empty() { query.trim() } else { boost.as_str() };
    let joined = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{joined}@{offset}")
}

/// Fetch one page of results, reading from cache first when enabled.
pub async fn fetch_page<P: AssetProvider>(
    provider: &P,
    cache: &Cache,
    query: &str,
    offset: u64,
    limit: u32,
) -> Result<FetchedPage> {
    let key = cache_key(query, offset);
    if let Some(page) = cache.get::<SearchPage>(NS_SEARCH, &key) {
        return Ok(FetchedPage {
            page,
            from_cache: true,
        });
    }
    let page = provider.search(query, offset, limit).await?;
    cache.put(NS_SEARCH, &key, &page);
    Ok(FetchedPage {
        page,
        from_cache: false,
    })
}

/// Ignore the cache and force a network fetch (for `r` = refresh).
pub async fn fetch_page_fresh<P: AssetProvider>(
    provider: &P,
    cache: &Cache,
    query: &str,
    offset: u64,
    limit: u32,
) -> Result<FetchedPage> {
    let page = provider.search(query, offset, limit).await?;
    cache.put(NS_SEARCH, &cache_key(query, offset), &page);
    Ok(FetchedPage {
        page,
        from_cache: false,
    })
}

/// Collect up to `limit` assets across as many pages as needed.
///
/// Pagination is lazy: pages are fetched only while more assets are needed,
/// so a request for 50 results never pulls 500.
pub async fn collect_assets<P: AssetProvider>(
    provider: &P,
    cache: &Cache,
    query: &str,
    limit: usize,
    per_page: u32,
    mut on_page: impl FnMut(&SearchPage),
) -> Result<Vec<crate::models::Asset>> {
    let per_page = per_page.clamp(1, 500);
    let mut out: Vec<crate::models::Asset> = Vec::new();
    let mut offset = 0u64;
    let mut empty_streak = 0u8;

    while out.len() < limit {
        let fetched = fetch_page(provider, cache, query, offset, per_page).await?;
        let page = fetched.page;
        if page.assets.is_empty() {
            empty_streak += 1;
            if empty_streak >= 2 {
                break;
            }
        } else {
            empty_streak = 0;
        }
        let total = page.assets.len();
        let total_hits = page.total_hits;
        on_page(&page);
        for asset in page.assets {
            if out.len() >= limit {
                break;
            }
            out.push(asset);
        }
        offset += per_page as u64;
        if let Some(total) = total_hits {
            if offset >= total {
                break;
            }
        }
        if total < per_page as usize {
            break;
        }
    }
    Ok(out)
}

/// Case-insensitive license filter. Never invents a license: assets whose
/// license is unknown are excluded when a filter is active.
pub fn filter_by_license(
    assets: Vec<crate::models::Asset>,
    needle: Option<&str>,
) -> Vec<crate::models::Asset> {
    match needle {
        None => assets,
        Some(needle) => {
            let needle = needle.trim().to_lowercase();
            assets
                .into_iter()
                .filter(|a| {
                    a.license
                        .as_ref()
                        .map(|l| l.to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Asset;

    fn asset(name: &str, license: Option<&str>) -> Asset {
        Asset {
            title: format!("File:{name}"),
            page_id: 1,
            original_name: name.to_string(),
            file_name: name.to_string(),
            index: None,
            size_bytes: None,
            mime: None,
            width: None,
            height: None,
            mediatype: None,
            url: None,
            thumb_url: None,
            description_url: None,
            author: None,
            uploader: None,
            license: license.map(str::to_string),
            license_url: None,
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
    fn license_filter_excludes_unknown_when_active() {
        let assets = vec![
            asset("a.svg", Some("CC0")),
            asset("b.svg", None),
            asset("c.svg", Some("CC BY-SA 4.0")),
        ];
        let filtered = filter_by_license(assets, Some("cc by-sa"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_name, "c.svg");
    }

    #[test]
    fn license_filter_disabled_keeps_everything() {
        let assets = vec![asset("a.svg", None)];
        assert_eq!(filter_by_license(assets, None).len(), 1);
    }

    #[test]
    fn cache_key_is_normalized() {
        assert_eq!(cache_key(" GitHub ", 0), cache_key("github", 0));
    }
}
