# Contributing to GET SVG

Thanks for wanting to help. This file covers how issues, patches, and behavior
are handled. Please also read [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Project ethos

- **Stay polite to Wikimedia.** The client is built around conservative request
  rates, `Retry-After` backoff, and capped concurrency. Any change that moves
  these toward more aggressive defaults will be rejected unless it is opt-in.
- **Safety is table stakes.** Filenames, URLs, and terminal output all flow
  through the sanitizers in `src/security.rs`. New code paths that touch
  untrusted input must reuse them, never re-implement them.
- **Tests prove behavior.** Every sanitizer has exhaustive tests, plus
  property tests in `tests/property.rs` and mock-server integration tests in
  `tests/wikimedia_http.rs`.

## Development environment

```sh
git clone https://github.com/avdeshjadon/get-svg.git
cd get-svg
cargo build
cargo test --all
```

The crate requires Rust 1.85+ (`rust-version` in `Cargo.toml`).

## What must pass

CI runs every check below; a PR that fails any of them will be asked to fix them.

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

Tests must be deterministic and offline:

- **Unit tests** use no network.
- **Integration tests** use `httpmock`, never the live Wikimedia API.
- **Property tests** (`tests/property.rs`) use `proptest`; shrinking output and
  regressions go into `tests/property.proptest-regressions`.

## Reporting a bug

Open an issue with:

1. Operating system and `get-svg --version`.
2. The exact command (or steps in the TUI) that triggered the problem.
3. Expected vs. actual behavior.
4. If it involves a specific file, a *safe* example — never a live upload URL
   from your own library that could be sensitive.

Security-sensitive bugs must be reported privately — see
[SECURITY.md](SECURITY.md). Do not open a public issue for them.

## Suggesting a feature

Open an issue and describe the workflow, not just the widget. Especially for TUI
changes, explain the key presses end-to-end. Large feature proposals go through
discussion before code lands.

## Submission checklist

- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --all` passes, including any new tests you added.
- [ ] No new public dependencies without justification in the PR body.
- [ ] New untrusted-input handling goes through `src/security.rs`.
- [ ] Default rate/limits are unchanged unless the change is opt-in.

## Architecture at a glance

```
src/
  cli.rs       clap definitions for the whole CLI surface
  commands.rs  non-interactive command dispatch
  ui/          the interactive terminal interface
  api/         provider trait + Wikimedia Commons implementation
  search.rs    cache-aware page fetching / license filtering
  download/    streaming downloads, naming, ZIP packing
  cache.rs     on-disk response cache
  security.rs  filename/path/terminal sanitizers shared everywhere
  output.rs    table / JSON / JSONL rendering
  doctor.rs    diagnostics
benches/       criterion benchmarks (security paths, harness=false)
tests/         property + httpmock integration tests
```

The provider abstraction in `src/api/provider.rs` means a second SVG source can
be added without touching the UI or download code. Reuse it rather than calling
Wikimedia directly from a new command.

## License

By contributing, you agree your contributions are licensed under the crate's
dual MIT/Apache-2.0 license, with the copyright held per [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE).