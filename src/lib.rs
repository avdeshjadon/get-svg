//! GET SVG — discover, inspect, and download SVG assets from Wikimedia Commons.
//!
//! This crate is the library half of the `get-svg` CLI. Everything the
//! binaries do lives here so it can be tested in-process.

// `async fn` in a public trait keeps the provider abstraction ergonomic; the
// trait methods return owned values so callers can still spawn them.
#![allow(async_fn_in_trait)]

pub mod api;
pub mod archive;
pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod doctor;
pub mod download;
pub mod error;
pub mod logging;
pub mod metadata;
pub mod models;
pub mod output;
pub mod search;
pub mod security;
pub mod ui;
pub mod uninstall;
pub mod update;

/// Crate version, used for User-Agent strings and `version` output.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Product name shown in branding and the terminal UI.
pub const APP_NAME: &str = "GET SVG";

/// Tagline used in branding and help output.
pub const TAGLINE: &str = "Discover. Download. Ship SVGs.";

/// Wire up the async runtime and dispatch CLI/UI.
///
/// Returns a process exit code.
pub async fn run() -> i32 {
    let cli = cli::Cli::parse_args();
    logging::init(cli.debug, cli.verbose);

    match commands::dispatch(cli).await {
        Ok(code) => code,
        Err(err) => {
            if logging::debug_enabled() {
                eprintln!("{err:?}");
                if let Some(source) = std::error::Error::source(&err) {
                    eprintln!("caused by: {source}");
                }
            }
            eprintln!("{}", err.friendly());
            1
        }
    }
}
