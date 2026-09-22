# Changelog

All notable changes to this project are documented in this file. This project
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Interactive TUI (`get-svg` with no arguments) with search, multi-select,
  live download/ZIP progress, and an attribution/details view.
- Provider abstraction (`AssetProvider`) designed so a second SVG
  source can be added without touching UI or download code.
- Non-interactive commands: `search`, `category`, `batch`, `download`,
  `config`, `cache status|clear`, `doctor`, `version`.
- Machine-readable output: `--format table|json|jsonl`.
- Bulk ZIP packing with an included `ATTRIBUTION.md` and per-file license
  metadata via `--metadata`.
- On-disk search-result cache with TTL and size budget; `cache status`/`clear`.
- Case-insensitive, collision-safe filename allocation (`file.svg`, `file-1.svg`).
- Association: `get-svg` and `getsvg` binaries from the same crate.

### Security
- Central sanitizers in `src/security.rs`: filename sanitization, safe path
  joining, ANSI/OSC/control-character stripping, markup flattening, and strict
  HTTPS URL validation.
- Response-size ceilings on API bodies (16 MB default / 64 MB hard) and 4 GiB
  per-file download cap.
- 429/`Retry-After` and exponential backoff, plus a per-process request
  interval limit.
- Filenames and `contact` headers are scrubbed so untrusted input cannot
  escape their container (no `..`, no control chars, no header injection).
- Japanese/multibyte-safe truncation that never splits UTF-8 and preserves
  file extensions.

### Changed
- Search queries automatically append `filemime:image/svg+xml` and constrain
  to the `File` namespace (6) unless already constrained.
- `create_zip` accepts an optional `FnMut` event callback so callers can render
  progress (breaking API change from the `&mut dyn` signature).

### Fixed
- Specialized handling of `File:`/`image:` wiki links so captions (not raw
  filenames) survive `strip_markup`.
- Trailing/leading dot abuse in user-supplied names can no longer leak `..`
  components into output paths.
- Query strings with quotes cannot break out of the `incategory:"…"` clause.

### Removed
- Dead `theme` configuration fields (name/title colors/high-contrast) and the
  unused `dv` render helper; keymap fields are now fully wired to handlers.

## [0.1.0] - 2026-09

Initial public release.

[0.1.0]: https://github.com/avdeshjadon/get-svg/releases/tag/v0.1.0