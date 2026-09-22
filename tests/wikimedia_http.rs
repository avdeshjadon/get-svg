//! Integration tests for the Wikimedia provider, driven by a mock HTTP server.
//!
//! These exercise the real request pipeline (search -> imageinfo lookup, and
//! the single- and multi-title metadata helpers) without ever touching the
//! live Wikimedia API. Security-sensible assertions: relevance order,
//! offsets, and metadata extraction are pinned here on purpose.

use get_svg::api::AssetProvider as _;
use get_svg::api::WikimediaClient;
use get_svg::config::Settings;
use httpmock::prelude::*;

/// Match the plain `titles` existence-confirmation query, which carries no
/// `prop` parameter (the imageinfo lookup does carry one).
fn no_prop(req: &HttpMockRequest) -> bool {
    !req.query_params
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|(key, _)| key == "prop")
}

const SEARCH_HITS: &str = r#"{
  "query": {
    "searchinfo": { "totalhits": 42 },
    "search": [
      {
        "title": "File:GitHub_Logo.svg",
        "timestamp": "2024-01-01T00:00:00Z"
      }
    ]
  }
}"#;

/// One `formatversion=2` page object with full imageinfo, mirroring the real
/// `prop=imageinfo` response shape for a single title.
const INFO_PAGE: &str = r#"{
  "query": {
    "pages": [
      {
        "pageid": 7,
        "title": "File:GitHub_Logo.svg",
        "imageinfo": [
          {
            "url": "https://upload.wikimedia.org/wikipedia/commons/GitHub_Logo.svg",
            "thumburl": "https://upload.wikimedia.org/wikipedia/commons/thumb/GitHub_Logo.svg",
            "descriptionshorturl": "https://commons.wikimedia.org/wiki/File:GitHub_Logo.svg",
            "size": 1234,
            "width": 1024,
            "height": 1024,
            "mime": "image/svg+xml",
            "mediatype": "DRAWING",
            "user": "UploaderBot",
            "extmetadata": {
              "LicenseShortName": { "value": "CC BY-SA 4.0" },
              "LicenseUrl": { "value": "https://creativecommons.org/licenses/by-sa/4.0/" },
              "Artist": { "value": "[[User:Example|Example]]" }
            }
          }
        ]
      }
    ]
  }
}"#;

/// Point a client at the mock server instead of the live API.
fn client(server: &MockServer) -> WikimediaClient {
    let settings = Settings::default();
    let base = WikimediaClient::new(&settings).expect("client construction");
    base.with_endpoint(format!("{}/w/api.php", server.base_url()))
}

#[tokio::test]
async fn search_runs_full_pipeline_against_mock() {
    let server = MockServer::start();

    // 1. list=search returns a single hit.
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("list", "search");
        then.status(200)
            .header("content-type", "application/json")
            .body(SEARCH_HITS);
    });

    // 2. imageinfo lookup for the hit titles.
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("prop", "imageinfo");
        then.status(200)
            .header("content-type", "application/json")
            .body(INFO_PAGE);
    });

    let client = client(&server);
    let page = client.search("github logo", 0, 25).await.expect("search");

    assert_eq!(page.total_hits, Some(42));
    assert_eq!(page.offset, 0);
    assert_eq!(page.per_page, 25);
    assert_eq!(page.assets.len(), 1);

    let a = &page.assets[0];
    assert_eq!(a.title, "File:GitHub_Logo.svg");
    assert_eq!(a.original_name, "GitHub_Logo.svg");
    assert_eq!(a.file_name, "GitHub_Logo.svg");
    assert_eq!(a.page_id, 7);
    assert_eq!(a.size_bytes, Some(1234));
    assert_eq!(a.width, Some(1024));
    assert_eq!(a.height, Some(1024));
    assert_eq!(a.license.as_deref(), Some("CC BY-SA 4.0"));
    // extmetadata markup is flattened to display text.
    assert_eq!(a.author.as_deref(), Some("Example"));
    assert!(a.url.as_deref().unwrap().starts_with("https://"));
    // Relevance order from list=search is preserved and indexed.
    assert_eq!(a.index, Some(0));
}

#[tokio::test]
async fn search_preserves_relevance_order() {
    let server = MockServer::start();
    let hits = r#"{
      "query": {
        "searchinfo": { "totalhits": 2 },
        "search": [
          { "title": "File:B.svg", "timestamp": "2024-01-01T00:00:00Z" },
          { "title": "File:A.svg", "timestamp": "2024-01-01T00:00:00Z" }
        ]
      }
    }"#;
    let pages = r#"{
      "query": {
        "pages": [
          {
            "pageid": 2,
            "title": "File:A.svg",
            "imageinfo": [{ "url": "https://upload.wikimedia.org/A.svg", "size": 1 }]
          },
          {
            "pageid": 1,
            "title": "File:B.svg",
            "imageinfo": [{ "url": "https://upload.wikimedia.org/B.svg", "size": 2 }]
          }
        ]
      }
    }"#;
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("list", "search");
        then.status(200).body(hits);
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("prop", "imageinfo");
        then.status(200).body(pages);
    });

    let page = client(&server).search("logos", 0, 25).await.unwrap();
    let titles: Vec<&str> = page.assets.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["File:B.svg", "File:A.svg"]);
    assert_eq!(page.assets[0].index, Some(0));
    assert_eq!(page.assets[1].index, Some(1));
}

#[tokio::test]
async fn get_asset_resolves_existing_file() {
    let server = MockServer::start();

    // First request: the imageinfo lookup for the title.
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("prop", "imageinfo");
        then.status(200).body(INFO_PAGE);
    });

    // Second request: the existence confirmation (plain titles query, no
    // `prop`), which must answer "not missing".
    server.mock(|when, then| {
        when.method(GET).path("/w/api.php").matches(no_prop);
        then.status(200).body(r#"{"query":{"pages":[]}}"#);
    });

    let client = client(&server);
    let asset = client
        .get_asset("GitHub_Logo.svg")
        .await
        .expect("get_asset");
    assert!(asset.is_some());
    assert_eq!(asset.unwrap().title, "File:GitHub_Logo.svg");
}

#[tokio::test]
async fn get_asset_returns_none_for_missing_file() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("prop", "imageinfo");
        then.status(200).body(r#"{"query":{"pages":[]}}"#);
    });

    let client = client(&server);
    assert!(client
        .get_asset("Missing_File.svg")
        .await
        .expect("get_asset")
        .is_none());
}

#[tokio::test]
async fn get_assets_batches_titles_and_extracts_metadata() {
    let server = MockServer::start();
    // Two titles -> one imageinfo request carrying both (chunk size is 20+).
    server.mock(|when, then| {
        when.method(GET)
            .path("/w/api.php")
            .query_param("prop", "imageinfo");
        then.status(200).body(INFO_PAGE);
    });

    let client = client(&server);
    let titles = vec![
        "File:GitHub_Logo.svg".to_string(),
        "File:Missing.svg".to_string(),
    ];
    let assets = client.get_assets(&titles).await.expect("get_assets");
    assert_eq!(assets.len(), 1, "only well-formed pages become assets");
    assert_eq!(assets[0].title, "File:GitHub_Logo.svg");
}
