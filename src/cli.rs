//! Command-line interface definition (clap).

use clap::{Parser, Subcommand};

/// GET SVG — discover, inspect, and download SVG assets from Wikimedia Commons.
#[derive(Debug, Parser)]
#[command(
    name = "get-svg",
    bin_name = "get-svg",
    version,
    about = "GET SVG — Discover. Download. Ship SVGs.",
    long_about = "GET SVG is a terminal-first client for discovering, inspecting, and \
downloading SVG assets from Wikimedia Commons.\n\nRun `get-svg` with no arguments to \
launch the interactive interface.",
    propagate_version = true
)]
pub struct Cli {
    /// Enable debug logging and technical error details.
    #[arg(long, global = true)]
    pub debug: bool,

    /// Enable info logging.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Search Wikimedia Commons for SVG files.
    Search {
        /// Search query (terms, phrases, and categories all work).
        query: String,

        /// Maximum number of results to return.
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,

        /// Output format.
        #[arg(long, short = 'f', value_enum, default_value_t = crate::output::OutputFormat::Table)]
        format: crate::output::OutputFormat,

        /// Download matching results into this directory.
        #[arg(long, value_name = "DIR")]
        download: Option<std::path::PathBuf>,

        /// Pack matching results into a ZIP archive.
        #[arg(long, value_name = "FILE")]
        zip: Option<std::path::PathBuf>,

        /// Write attribution/license metadata into this directory.
        #[arg(long, value_name = "DIR")]
        metadata: Option<std::path::PathBuf>,

        /// Only include results whose license string contains this text.
        #[arg(long)]
        license: Option<String>,

        /// Page of results to fetch (page size = --limit).
        #[arg(long, default_value_t = 1)]
        page: u32,

        /// Skip confirmation prompts for bulk operations.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Overwrite existing files instead of skipping them.
        #[arg(long)]
        overwrite: bool,

        /// Maximum parallel downloads (1-32).
        #[arg(long)]
        concurrency: Option<usize>,
    },

    /// Download a single file by name or MediaWiki title.
    Download {
        /// File name or `File:Title.svg`.
        file: String,

        /// Destination directory (defaults to your configured download dir).
        #[arg(long, short = 'o', value_name = "DIR")]
        output: Option<std::path::PathBuf>,

        /// Output format for the result record.
        #[arg(long, short = 'f', value_enum, default_value_t = crate::output::OutputFormat::Table)]
        format: crate::output::OutputFormat,

        /// Overwrite an existing file.
        #[arg(long)]
        overwrite: bool,
    },

    /// List SVG files in a Wikimedia Commons category.
    Category {
        /// Category name, with or without the `Category:` prefix.
        category: String,

        /// Maximum number of results.
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,

        /// Output format.
        #[arg(long, short = 'f', value_enum, default_value_t = crate::output::OutputFormat::Table)]
        format: crate::output::OutputFormat,

        /// Download matching results into this directory.
        #[arg(long, value_name = "DIR")]
        download: Option<std::path::PathBuf>,

        /// Pack matching results into a ZIP archive.
        #[arg(long, value_name = "FILE")]
        zip: Option<std::path::PathBuf>,

        /// Skip confirmation prompts for bulk operations.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Overwrite existing files instead of skipping them.
        #[arg(long)]
        overwrite: bool,
    },

    /// Search and download everything matching in one step.
    Batch {
        /// Search query.
        query: String,

        /// Destination directory (defaults to your configured download dir).
        #[arg(long, short = 'o', value_name = "DIR")]
        download: Option<std::path::PathBuf>,

        /// Pack results into a ZIP archive.
        #[arg(long, value_name = "FILE")]
        zip: Option<std::path::PathBuf>,

        /// Write attribution/license metadata here.
        #[arg(long, value_name = "DIR")]
        metadata: Option<std::path::PathBuf>,

        /// Maximum number of files to download.
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,

        /// Skip confirmation prompts.
        #[arg(long, short = 'y')]
        yes: bool,

        /// Overwrite existing files instead of skipping them.
        #[arg(long)]
        overwrite: bool,
    },

    /// Inspect or manage the local cache.
    #[command(subcommand)]
    Cache(CacheCommand),

    /// Show (or initialize) the configuration file.
    Config {
        /// Write a default config file if missing.
        #[arg(long)]
        init: bool,
    },

    /// Diagnose environment, network, and configuration problems.
    Doctor {
        /// Emit the report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Print version information.
    Version,

    /// Completely remove GET SVG from this system.
    ///
    /// Deletes the config (settings, cache, recent searches), the default
    /// `get-svg` download folder, and the installed `get-svg`/`getsvg`
    /// binaries — leaving no trace. A custom download directory is left
    /// untouched unless its name is `get-svg`.
    Dlt {
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Download and install the latest release, replacing this binary.
    ///
    /// Verifies the download with SHA-256, swaps `get-svg`/`getsvg` in place,
    /// and removes old binaries and leftover `.old` files. Uses the GitHub
    /// releases API pointed at this project unless `GET_SVG_UPDATE_REPO` is set.
    Update,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Show cache location and disk usage.
    Status,
    /// Delete all cached responses.
    Clear,
}

impl Cli {
    /// Parse from `std::env::args_os` (clap's standard entry point).
    pub fn parse_args() -> Cli {
        <Cli as Parser>::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_search() {
        let cli = Cli::parse_from([
            "get-svg", "search", "github", "--limit", "10", "--format", "json",
        ]);
        match cli.command {
            Some(Command::Search {
                query,
                limit,
                format,
                ..
            }) => {
                assert_eq!(query, "github");
                assert_eq!(limit, 10);
                assert_eq!(format, crate::output::OutputFormat::Json);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_download_with_output() {
        let cli = Cli::parse_from(["get-svg", "download", "File:GitHub_Logo.svg", "-o", "/tmp"]);
        match cli.command {
            Some(Command::Download { file, output, .. }) => {
                assert_eq!(file, "File:GitHub_Logo.svg");
                assert_eq!(output.unwrap(), std::path::PathBuf::from("/tmp"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_cache_clear() {
        let cli = Cli::parse_from(["get-svg", "cache", "clear"]);
        assert!(matches!(
            cli.command,
            Some(Command::Cache(CacheCommand::Clear))
        ));
    }

    #[test]
    fn no_subcommand_means_interactive() {
        let cli = Cli::parse_from(["get-svg"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn global_debug_flag() {
        let cli = Cli::parse_from(["get-svg", "--debug", "version"]);
        assert!(cli.debug);
    }

    #[test]
    fn parses_dlt_with_yes() {
        let cli = Cli::parse_from(["get-svg", "dlt", "--yes"]);
        assert!(matches!(cli.command, Some(Command::Dlt { yes: true })));
    }
}
