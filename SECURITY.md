# Security Policy

GET SVG downloads files from the internet and renders untrusted text in a
terminal, so a meaningful chunk of the crate exists purely to contain hostile
input. This policy describes how vulnerabilities are handled and what the
security boundaries are.

## Supported versions

Only the latest release is supported. Security fixes ship in the next release
and are backported only when they block a known, actively exploited class of bug
and the maintainer can produce a point release promptly.

## Reporting a vulnerability

**Do not open a public issue.** Email `avdeshyadav15@gmail.com` or open a
[GitHub security advisory](https://github.com/avdeshjadon/get-svg/security/advisories/new)
(privately). Include:

- The affected version(s).
- A minimal reproducer where possible (a crafted filename, a mocked API
  response, a terminal sequence).
- Your assessment of impact (worst case) and any suggested fix.

You will receive a response within **72 hours**. We will not disclose details
until a fix is released, and we will credit you in the advisory and changelog
unless you ask to remain anonymous.

## Scope — what is in scope

- **Path traversal**: filenames that escape the output directory
  (`../`, `..\\`, absolute paths, traversal via unicode).
- **Filename abuse**: control characters, Windows reserved names, trailing
  dots/spaces, 200+ byte names, case-collision collisions on case-insensitive
  filesystems.
- **URL validation**: non-HTTPS or credential-smuggled redirects that leak or
  rewrite downloads.
- **Response limits**: oversized API/download bodies, decompression bombs.
- **Terminal injection**: ANSI/OSC escape sequences or control characters in
  titles/descriptions that could manipulate the TUI.
- **Header smuggling**: the `contact` user-agent field introducing newlines or
  other header-breaking bytes.
- **Cache integrity**: cache-filename collisions or stale/poisoned entries.

## Out of scope

- Bugs in third-party crates unless GET SVG invokes them unsafely.
- Social engineering of Wikimedia editors via content in the result set.
- Vulnerabilities in the reverse-dependency `getsvg` alias binary beyond what
  the crate itself has.

## Security model notes

All untrusted-input handling is centralized in `src/security.rs`:
`sanitize_filename`, `safe_join`, `strip_ansi`/`sanitize_text`, `strip_markup`,
and `validate_https_url`. Safe joins are the only way untrusted names become
paths. If you change one of these, run:

```sh
cargo test --all       # includes property + mock-server tests
cargo clippy --all-targets --all-features -- -D warnings
```

New code must route untrusted input through these helpers rather than
duplicating the logic.