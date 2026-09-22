# GET SVG

**GET SVG** is a terminal-first client for discovering, inspecting, and downloading SVG
assets from [Wikimedia Commons](https://commons.wikimedia.org). It ships both a
fast, keyboard-driven terminal interface and a scriptable CLI that speaks `table`, `JSON`,
and JSON-lines — with downloads, ZIP packing, per-file attribution metadata, caching,
and a rate limiter that stays polite toward Wikimedia's shared infrastructure.

> `GET SVG — Discover. Download. Ship SVGs.`

## Features

- **Interactive TUI** — search, preview, multi-select, live progress, attribution view.
- **Scriptable CLI** — `search`, `category`, `batch`, `download`, `config`, `cache`, `doctor`.
  `--format table|json|jsonl` for pipelines.
- **Bulk downloads** — parallel (polite, capped) downloads with atomic writes and
  collision-safe naming (`file.svg`, `file-1.svg`, …), case-insensitive on disk.
- **ZIP packing** — one archive per query with an included `ATTRIBUTION.md`.
- **Attribution metadata** — a `metadata/` folder with per-file license and credit
  records, so exporting art stays legally clean.
- **On-disk cache** — search results cached with TTL + size budget; `get-svg cache status|clear`.
- **Safety first** — path-traversal-proof filenames, HTTPS-only, response-size ceilings,
  429/retry backoff, and terminal escape stripping.

## Installation

### Pre-built binaries (recommended)

One-line installers fetch the [latest GitHub release](https://github.com/avdeshjadon/get-svg/releases)
for your OS + CPU and verify its SHA-256 checksum before installing to
`~/.local/bin`:

```sh
# macOS / Linux / Windows Git Bash / MSYS
curl -fsSL https://raw.githubusercontent.com/avdeshjadon/get-svg/main/install.sh | sh
```

```powershell
# Windows PowerShell (native)
Set-ExecutionPolicy -Scope Process Bypass
irm https://raw.githubusercontent.com/avdeshjadon/get-svg/main/install.ps1 | iex
```

- The latest release is resolved automatically — no version to remember. To pin
  one anyway, set `GET_SVG_VERSION=v0.1.0` (or `$env:GET_SVG_VERSION`).
- The installer puts **both** the `get-svg` binary and a `getsvg` shortcut into
  `~/.local/bin`. After it finishes (and you add the printed directory to your
  `PATH`) just type `getsvg` to launch the interactive interface, or
  `getsvg --help` for the full command list.
- Linux builds target the GNU C library (glibc ≥ 2.31, e.g. Ubuntu 20.04+,
  Debian 11+). macOS binaries are unsigned — install via the command line
  (curl) to avoid Gatekeeper prompts.
- Re-run with `--dir "$HOME/bin"` / `INSTALL_DIR` to install somewhere else.

### From source

```sh
cargo install get-svg --locked
```

This installs both the `get-svg` binary and a `getsvg` alias. Or build from source:

```sh
git clone https://github.com/avdeshjadon/get-svg.git
cd get-svg
cargo build --release
```

## Quick start

Launch the interactive interface (requires a real TTY):

```sh
get-svg
```

Non-interactive search:

```sh
get-svg search "github logo"
get-svg search "logos" --limit 50 --format json
get-svg category "Logos" --limit 100 --download ./logos
```

Download one file and its metadata:

```sh
get-svg download "File:GitHub_Logo.svg" -o ./assets
```

Search + download + zip + write attribution in one step:

```sh
get-svg batch "minimalism" --zip repo-svg.zip --metadata ./attribution -y
```

Inspect and maintain the cache:

```sh
get-svg cache status
get-svg cache clear
```

Self-diagnose environment, network, and config:

```sh
get-svg doctor
```

## Command reference

Run `get-svg <command> --help` for the authoritative flag list. Overview:

| Command      | Purpose                                            |
| ------------ | -------------------------------------------------- |
| `search`     | Search Commons for SVGs; optional download/zip/metadata. |
| `category`   | List SVGs in a category (with or without the `Category:` prefix). |
| `batch`      | Search and download/zip/metadata everything in one step. |
| `download`   | Download a single file by name or `File:` title.   |
| `cache`      | Show or clear the local response cache.            |
| `config`     | Show the effective config; `--init` writes a default file. |
| `doctor`     | Environment/network/config diagnostics (`--json`). |
| `version`    | Print version info.                                |

Interactive keyboard map (see the `?` help screen in-app):

| Key | Action |
| --- | ------ |
| `/` | Search / filter current list |
| `d` | Download selected file |
| `A` | Download all results |
| `z` | Pack results into a ZIP |
| `a` | Select / deselect all |
| ` ` | Toggle selection |
| `c` | Copy to clipboard |
| `o` | Open description page in browser |
| `r` | Refresh (bypass cache) |
| `q` | Quit |

## Configuration

`get-svg` reads TOML from your platform config dir:

- Linux: `~/.config/get-svg/config.toml`
- macOS: `~/Library/Application Support/get-svg/config.toml`
- Windows: `%APPDATA%\get-svg\config.toml`

Generate a file with all defaults:

```sh
get-svg config --init
```

`config --init` prints the current effective config; `get-svg config` (no flag) is
read-only. Available keys:

```toml
download_directory = "~/Downloads/get-svg"
max_concurrency = 4          # parallel downloads (1-32)
cache_enabled = true
cache_ttl_hours = 24
cache_max_mb = 100
theme = "default"
animations = true
min_request_interval_ms = 250  # minimum gap between API request starts
max_response_mb = 16           # hard ceiling per API response
contact = "your@email"         # appended to the User-Agent (recommended for heavy use)
```

The default User-Agent follows [Wikimedia policy](https://meta.wikimedia.org/wiki/User-Agent_policy):
`GET-SVG/<version> (<repository>; <contact>)`. Setting `contact` is recommended if you
run heavy automation so Wikimedia can reach you.

## License, attribution, and Wikimedia etiquette

- Files on Wikimedia Commons carry their **own** licenses. GET SVG never invents one:
  per-file license/credit info is written next to downloaded files (`metadata/`),
  and unknown licenses are reported as `Unknown` rather than guessed.
- The client enforces a conservative request rate, responds to `429 Retry-After`, and
  limits concurrency so bulk jobs do not hammer the API.

## Development

```sh
cargo fmt --check          # formatting
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all           # unit + property + mock-server integration tests
cargo bench                # security-path benchmarks
cargo audit                # dependency vulnerability scan
```

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md), and the
[CHANGELOG.md](CHANGELOG.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). You may choose
either license.