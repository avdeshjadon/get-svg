//! Wikimedia Commons API client.
//!
//! Uses the official MediaWiki Action API only — never HTML scraping.
//! Endpoints:
//! * `action=query&list=search` — relevance-ranked titles + total hit count
//! * `action=query&prop=imageinfo` — metadata, license, direct file URLs

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, RETRY_AFTER, USER_AGENT};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::api::provider::AssetProvider;
use crate::api::rate_limit::{backoff, parse_retry_after, RateLimiter};
use crate::config::Settings;
use crate::error::{Error, Result};
use crate::models::{Asset, SearchPage};

/// The official MediaWiki API endpoint for Wikimedia Commons.
pub const API_ENDPOINT: &str = "https://commons.wikimedia.org/w/api.php";

/// MediaWiki file namespace (all files live here).
const FILE_NAMESPACE: u32 = 6;

/// Hard cap on titles per `prop=imageinfo` request (keeps URLs short).
const TITLE_CHUNK: usize = 20;
/// Cap on bytes we will read from a single API response.
const MAX_API_BODY: usize = 64 * 1024 * 1024;

/// A handle to Wikimedia Commons. Cheap to clone; all state is shared.
#[derive(Clone)]
pub struct WikimediaClient {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    endpoint: String,
    limiter: RateLimiter,
    max_response_bytes: usize,
    max_retries: u32,
    /// Serializes backoff sleeps so parallel workers do not stampede.
    backoff_lock: Mutex<()>,
}

impl std::fmt::Debug for WikimediaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WikimediaClient")
            .field("endpoint", &self.inner.endpoint)
            .finish()
    }
}

impl WikimediaClient {
    pub fn new(settings: &Settings) -> Result<WikimediaClient> {
        let client = Client::builder()
            .user_agent(settings.user_agent())
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(4)
            .redirect(reqwest::redirect::Policy::limited(5))
            .https_only(true)
            .build()?;

        Ok(WikimediaClient {
            inner: Arc::new(Inner {
                client,
                endpoint: API_ENDPOINT.to_string(),
                limiter: RateLimiter::new(
                    settings.max_concurrency,
                    Duration::from_millis(settings.min_request_interval_ms),
                ),
                max_response_bytes: settings.max_response_bytes(),
                max_retries: 3,
                backoff_lock: Mutex::new(()),
            }),
        })
    }

    /// Override the API endpoint (integration tests only).
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> WikimediaClient {
        // Build a fresh inner with the new endpoint, reusing the client.
        let endpoint = endpoint.into();
        // Keep existing transport config identical to production.
        let client = Client::builder()
            .user_agent(format!("GET-SVG/test ({endpoint})"))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("test client");
        let _ = USER_AGENT;
        self.inner = Arc::new(Inner {
            client,
            endpoint,
            limiter: RateLimiter::new(4, Duration::from_millis(10)),
            max_response_bytes: MAX_API_BODY,
            max_retries: 2,
            backoff_lock: Mutex::new(()),
        });
        self
    }

    /// The shared transport client (clone, for download/zip use).
    pub fn client(&self) -> Client {
        self.inner.client.clone()
    }

    /// Tune the ranking of a single-token query: a bare word is usually meant as
    /// a file name or brand term (`github`, `amazon`), so scope it to titles.
    /// Multi-word phrases and explicit operators are left untouched.
    fn title_boost(q: &str) -> String {
        let trimmed = q.trim();
        let single = !trimmed.contains(char::is_whitespace);
        let operator = [
            "intitle:",
            "incategory:",
            "filetype:",
            "filemime:",
            "haswbstatement:",
            "deepcategory:",
        ]
        .iter()
        .any(|op| trimmed.to_lowercase().starts_with(op));
        let file = trimmed.to_lowercase().starts_with("file:");
        let slug = trimmed.to_lowercase().ends_with(".svg");
        if single && !operator && !file && !slug && trimmed.len() >= 2 {
            format!("intitle:{trimmed}")
        } else {
            trimmed.to_string()
        }
    }

    /// Build the srsearch string: SVG-only filter plus optional extras.
    pub fn compose_search(query: &str, extra: Option<&str>) -> String {
        let mut parts: Vec<String> = Vec::new();
        let q = query.trim();
        if !q.is_empty() {
            parts.push(WikimediaClient::title_boost(q));
        }
        if let Some(extra) = extra {
            let extra = extra.trim();
            if !extra.is_empty() {
                parts.push(extra.to_string());
            }
        }
        let joined = parts.join(" ");
        if joined.to_lowercase().contains("filemime:") {
            joined
        } else {
            format!("{joined} filemime:image/svg+xml")
                .trim()
                .to_string()
        }
    }

    /// GET `endpoint?params` as parsed JSON, with throttling and retries.
    async fn get_json(&self, params: &[(&str, String)]) -> Result<Value> {
        let mut attempt = 0u32;
        loop {
            if attempt > 0 {
                let delay = backoff(attempt - 1);
                // Collapse concurrent backoffs so we do not wake in lockstep.
                let _guard = self.inner.backoff_lock.lock().await;
                tokio::time::sleep(delay).await;
            }

            let permit = self.inner.limiter.acquire().await;
            let response = self
                .inner
                .client
                .get(&self.inner.endpoint)
                .query(params)
                .send()
                .await;
            drop(permit);

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    let net = Error::from(e);
                    if attempt < self.inner.max_retries && is_retryable(&net) {
                        attempt += 1;
                        continue;
                    }
                    return Err(net);
                }
            };

            let status = response.status();

            if status.as_u16() == 429 {
                if attempt < self.inner.max_retries {
                    let wait = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_retry_after)
                        .unwrap_or_else(|| backoff(attempt));
                    let _guard = self.inner.backoff_lock.lock().await;
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                    continue;
                }
                return Err(Error::RateLimited {
                    retry_after_secs: response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_retry_after)
                        .map(|d| d.as_secs())
                        .unwrap_or(5),
                });
            }

            if status.is_server_error() && attempt < self.inner.max_retries {
                attempt += 1;
                continue;
            }

            if !status.is_success() {
                return Err(Error::Http {
                    status: status.as_u16(),
                    detail: format!(
                        "request to {endpoint} failed",
                        endpoint = self.inner.endpoint
                    ),
                });
            }

            // Enforce a hard response-size ceiling before reading the body.
            let declared = response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            if declared as usize > self.inner.max_response_bytes || declared as usize > MAX_API_BODY
            {
                return Err(Error::ResponseTooLarge {
                    got: declared as usize,
                    limit: self.inner.max_response_bytes.min(MAX_API_BODY),
                });
            }

            let bytes = response.bytes().await?;
            if bytes.len() > self.inner.max_response_bytes || bytes.len() > MAX_API_BODY {
                return Err(Error::ResponseTooLarge {
                    got: bytes.len(),
                    limit: self.inner.max_response_bytes.min(MAX_API_BODY),
                });
            }

            let value: Value = serde_json::from_slice(&bytes)?;

            // Surface MediaWiki-level errors as structured failures.
            if let Some(err) = value.get("error") {
                let code = err
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let info = err
                    .get("info")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(Error::Malformed(format!("{code}: {info}")));
            }
            if let Some(warnings) = value.get("warnings") {
                tracing::debug!("API warnings: {warnings}");
            }

            return Ok(value);
        }
    }

    /// Fetch metadata for titles in bounded chunks.
    async fn imageinfo_for(&self, titles: &[String]) -> Result<Vec<Asset>> {
        let mut assets = Vec::with_capacity(titles.len());
        for chunk in titles.chunks(TITLE_CHUNK) {
            let joined = chunk.join("|");
            let params = [
                ("action", "query".to_string()),
                ("format", "json".to_string()),
                ("formatversion", "2".to_string()),
                ("prop", "imageinfo".to_string()),
                (
                    "iiprop",
                    "url|size|mime|extmetadata|timestamp|user|mediatype".to_string(),
                ),
                ("titles", joined),
            ];
            let json = self.get_json(&params).await?;
            let pages = json
                .pointer("/query/pages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for page in &pages {
                if let Some(asset) = Asset::from_api_json(page) {
                    assets.push(asset);
                }
            }
        }
        Ok(assets)
    }
}

fn is_retryable(err: &Error) -> bool {
    match err {
        Error::Network(net) => {
            net.is_timeout() || net.is_connect() || net.is_request() || net.is_redirect()
        }
        Error::Http { status, .. } => *status >= 500,
        _ => false,
    }
}

impl AssetProvider for WikimediaClient {
    fn provider_name(&self) -> &'static str {
        "Wikimedia Commons"
    }

    async fn search(&self, query: &str, offset: u64, limit: u32) -> Result<SearchPage> {
        let srsearch = Self::compose_search(query, None);
        let limit = limit.clamp(1, 500);
        let params = [
            ("action", "query".to_string()),
            ("format", "json".to_string()),
            ("formatversion", "2".to_string()),
            ("list", "search".to_string()),
            ("srsearch", srsearch),
            ("srnamespace", FILE_NAMESPACE.to_string()),
            ("srlimit", limit.to_string()),
            ("sroffset", offset.to_string()),
            ("srqiprofile", "popular_inclinks_pv".to_string()),
            ("srprop", "timestamp".to_string()),
        ];

        let json = self.get_json(&params).await?;

        let total_hits = json
            .pointer("/query/searchinfo/totalhits")
            .and_then(Value::as_u64);
        let hits = json
            .pointer("/query/search")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut titles = Vec::with_capacity(hits.len());
        let mut order = Vec::with_capacity(hits.len());
        for (i, hit) in hits.iter().enumerate() {
            if let Some(title) = hit.get("title").and_then(Value::as_str) {
                titles.push(title.to_string());
                order.push((title.to_string(), i as u64));
            }
        }

        let mut assets = if titles.is_empty() {
            Vec::new()
        } else {
            self.imageinfo_for(&titles).await?
        };

        // Preserve relevance order from list=search.
        let mut rank: std::collections::HashMap<String, u64> = order.into_iter().collect();
        assets.sort_by_key(|a| rank.remove(&a.title).unwrap_or(u64::MAX));
        for (i, a) in assets.iter_mut().enumerate() {
            a.index = Some(offset + i as u64);
        }

        Ok(SearchPage {
            query: query.to_string(),
            offset,
            per_page: limit as u64,
            total_hits,
            assets,
        })
    }

    async fn get_asset(&self, title: &str) -> Result<Option<Asset>> {
        let title = if title.starts_with("File:") || title.starts_with("file:") {
            title.to_string()
        } else {
            format!("File:{title}")
        };
        let found = self.imageinfo_for(std::slice::from_ref(&title)).await?;
        // imageinfo_for returns a synthetic "missing" page only if parsed;
        // detect missing pages explicitly.
        if found.is_empty() {
            return Ok(None);
        }
        // MediaWiki marks missing pages with `missing: true`; we must detect
        // that before it becomes an Asset. Re-query cheaply to confirm.
        let params = [
            ("action", "query".to_string()),
            ("format", "json".to_string()),
            ("formatversion", "2".to_string()),
            ("titles", title),
        ];
        let json = self.get_json(&params).await?;
        let missing = json
            .pointer("/query/pages")
            .and_then(Value::as_array)
            .and_then(|p| p.first())
            .and_then(|p| p.get("missing"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if missing {
            return Ok(None);
        }
        Ok(found.into_iter().next())
    }

    async fn get_assets(&self, titles: &[String]) -> Result<Vec<Asset>> {
        self.imageinfo_for(titles).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_boost_scopes_single_bare_token() {
        assert_eq!(WikimediaClient::title_boost("github"), "intitle:github");
        assert_eq!(WikimediaClient::title_boost("Github Logo"), "Github Logo");
        assert_eq!(
            WikimediaClient::title_boost("intitle:logos"),
            "intitle:logos"
        );
        assert_eq!(
            WikimediaClient::title_boost("file:flag_of_india.svg"),
            "file:flag_of_india.svg"
        );
        assert_eq!(WikimediaClient::title_boost("github.svg"), "github.svg");
    }

    #[test]
    fn compose_search_appends_svg_filter() {
        assert_eq!(
            WikimediaClient::compose_search("github", None),
            "intitle:github filemime:image/svg+xml"
        );
        assert_eq!(
            WikimediaClient::compose_search("github", Some("incategory:Logos")),
            "intitle:github incategory:Logos filemime:image/svg+xml"
        );
    }

    #[test]
    fn compose_search_keeps_existing_filter() {
        let s = WikimediaClient::compose_search("github filemime:image/svg+xml", None);
        assert_eq!(s.matches("filemime:").count(), 1);
    }

    #[test]
    fn endpoint_is_https() {
        assert!(API_ENDPOINT.starts_with("https://"));
    }
}
