//! Defense-in-depth helpers: filename sanitization, path containment,
//! terminal escape sanitization, URL validation, and markup stripping.
//!
//! Everything in here is pure and side-effect free so it can be unit tested
//! exhaustively and reused by every layer that touches untrusted input.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::error::{Error, Result};

/// Maximum length (in bytes) of a generated file name, extension included.
pub const MAX_FILENAME_BYTES: usize = 200;
/// Maximum length of any single-line value rendered into the terminal.
pub const MAX_TEXT_CHARS: usize = 400;

fn windows_reserved() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|$)").unwrap())
}

fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // CSI, OSC (BEL or ST terminated), and other two-byte escape sequences.
    RE.get_or_init(|| {
        Regex::new(r"\x1b(?:\][^\x07\x1b]*(?:\x07|\x1b\\)|\[[0-?]*[ -/]*[@-~]|[@-Z\\-_])").unwrap()
    })
}

fn wiki_bracket_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\[[^\]]*\]\]").unwrap())
}

/// Sanitize an untrusted string into a safe, filesystem-friendly file name.
///
/// Handles: path separators, control characters, Windows reserved device
/// names, leading dots (hidden files), trailing dots/spaces (Windows),
/// over-long names, and names that are empty or `.`/`..`.
pub fn sanitize_filename(name: &str) -> String {
    let name = name.trim();
    let mut out = String::with_capacity(name.len());

    for c in name.chars() {
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
            out.push('_');
        } else {
            out.push(c);
        }
    }

    // Neutralize dot abuse: a run of dots at the very start (hidden files,
    // `.`, `..`) and any run of two or more dots (traversal remnants) become
    // underscores, while a single interior dot (an extension separator) is
    // preserved so names like `ok.svg` keep working. A trailing dot-run is
    // dropped entirely (Windows also rejects trailing dots).
    let mut cleaned = String::with_capacity(out.len());
    let mut pending = 0usize;
    let mut leading = true;
    for c in out.chars() {
        if c == '.' {
            pending += 1;
        } else {
            if pending > 0 {
                if leading || pending >= 2 {
                    cleaned.extend(std::iter::repeat_n('_', pending));
                } else {
                    cleaned.push('.');
                }
                pending = 0;
            }
            leading = false;
            cleaned.push(c);
        }
    }
    if pending > 0 && leading {
        cleaned.extend(std::iter::repeat_n('_', pending));
    }
    let mut out = cleaned;

    if out.is_empty() {
        out.push_str("untitled");
    }

    if windows_reserved().is_match(&out) {
        out.insert(0, '_');
    }

    truncate_name(&out, MAX_FILENAME_BYTES)
}

/// Truncate a file name to `max` bytes without splitting UTF-8 and while
/// preserving the extension.
pub fn truncate_name(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let budget = max.saturating_sub(ext.len());
    let mut end = budget.min(stem.len());
    while end > 0 && !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", stem[..end].trim_end_matches(['.', ' ']), ext)
}

/// Truncate arbitrary text to at most `max` UTF-8 chars, appending `…`.
pub fn truncate_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// True if the string contains bytes that could manipulate a terminal
/// (escape sequences, cursor moves, BEL, etc).
pub fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// Remove ANSI/OSC escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let stripped = ansi_re().replace_all(s, "");
    stripped.into_owned()
}

/// Sanitize untrusted text before it is rendered in the terminal or written
/// into logs. Strips escape sequences and control characters, then clamps
/// length so a malicious response cannot flood the screen.
pub fn sanitize_text(s: &str) -> String {
    let stripped = strip_ansi(s);
    let mut out = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        if c.is_control() {
            // Preserve line breaks in multi-line fields; turn tabs into spaces.
            if c == '\n' {
                out.push('\n');
            } else if c == '\t' {
                out.push(' ');
            }
            // drop everything else (ESC, BEL, CR, NUL, ...)
        } else {
            out.push(c);
        }
    }
    truncate_text(&out, MAX_TEXT_CHARS)
}

/// Flatten wikitext-ish markup that leaks into extmetadata values.
pub fn strip_markup(s: &str) -> String {
    // Resolve every `[[...]]` to its display text: the segment after the
    // final `|` (captions). Bare file/image references without a caption
    // are dropped so raw `[[File:...]]` noise cannot leak through.
    let no_links = wiki_bracket_re().replace_all(s, |caps: &regex::Captures<'_>| {
        let whole = caps.get(0).expect("full match").as_str();
        let body = &whole[2..whole.len() - 2];
        let lowered = body.to_ascii_lowercase();
        let is_file = lowered.starts_with("file:") || lowered.starts_with("image:");
        let display = body.rsplit('|').next().unwrap_or("").trim();
        if is_file && display.is_empty() {
            String::new()
        } else {
            display.to_owned()
        }
    });
    let no_tags = Regex::new(r"(?s)<[^>]*>")
        .map(|re| re.replace_all(&no_links, " ").into_owned())
        .unwrap_or_else(|_| no_links.into_owned());
    no_tags.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Validate that a URL is HTTPS, has a host, and carries no embedded
/// credentials (which can be used to smuggle redirects).
pub fn validate_https_url(s: &str) -> bool {
    match url::Url::parse(s) {
        Ok(u) => {
            u.scheme() == "https"
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        }
        Err(_) => false,
    }
}

/// Join `name` onto `base`, guaranteeing the result stays inside `base`.
///
/// This is the single choke point for turning untrusted names into paths.
pub fn safe_join(base: &Path, name: &str) -> Result<PathBuf> {
    let clean = sanitize_filename(name);
    if clean.is_empty() || clean == "." || clean == ".." {
        return Err(Error::UnsafeFilename(name.to_string()));
    }
    if clean.contains('/') || clean.contains('\\') {
        return Err(Error::UnsafeFilename(name.to_string()));
    }

    let base_abs = if base.is_absolute() {
        base.to_path_buf()
    } else {
        std::env::current_dir()?.join(base)
    };

    let joined = base_abs.join(&clean);
    if !joined.starts_with(&base_abs) {
        return Err(Error::UnsafePath(name.to_string()));
    }
    // Belt and braces: reject anything that still walks out lexically.
    for comp in joined.components() {
        match comp {
            Component::ParentDir => return Err(Error::UnsafePath(name.to_string())),
            Component::Prefix(_) => return Err(Error::UnsafePath(name.to_string())),
            _ => {}
        }
    }
    Ok(joined)
}

/// Extract a file name from a MediaWiki `File:` title.
pub fn file_name_from_title(title: &str) -> String {
    title
        .strip_prefix("File:")
        .or_else(|| title.strip_prefix("file:"))
        .unwrap_or(title)
        .to_string()
}

/// Normalize an already-sanitized base name against existing names on a
/// case-insensitive filesystem by comparing lowercased keys.
pub fn casefold(name: &str) -> String {
    name.to_lowercase()
}

/// Best-effort available disk space in bytes (Unix only; `None` elsewhere).
pub fn available_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c.as_ptr(), &mut stat) == 0 {
                Some(stat.f_bavail as u64 * stat.f_frsize as u64)
            } else {
                None
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_path_traversal() {
        for name in [
            "../../secret.svg",
            "../../../etc/passwd",
            "..\\..\\windows\\system32\\evil.svg",
            "/etc/shadow",
            "sub/dir/file.svg",
        ] {
            let out = sanitize_filename(name);
            assert!(!out.contains('/'), "{name} produced {out}");
            assert!(!out.contains('\\'), "{name} produced {out}");
            assert_ne!(out, "..", "{name} produced {out}");
        }
    }

    #[test]
    fn safe_join_contains_paths() {
        let base = Path::new("/tmp/out");
        let p = safe_join(base, "../../etc/passwd").unwrap();
        assert!(p.starts_with(base), "{p:?}");
        assert!(safe_join(base, "ok.svg").unwrap().ends_with("ok.svg"));
    }

    #[test]
    fn windows_reserved_names_are_prefixed() {
        assert_eq!(sanitize_filename("CON.svg"), "_CON.svg");
        assert_eq!(sanitize_filename("con"), "_con");
        assert_eq!(sanitize_filename("LPT1.txt"), "_LPT1.txt");
        assert_ne!(sanitize_filename("console.svg"), "_console.svg");
    }

    #[test]
    fn strips_terminal_escapes() {
        let evil = "\x1b[2J\x1b[1;31mGitHub\x07.svg";
        let clean = sanitize_text(evil);
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\x07'));
        assert!(clean.contains("GitHub"));
    }

    #[test]
    fn strips_osc_and_hyperlinks() {
        let evil = "\x1b]8;;http://evil.test\x07Click\x1b]8;;\x07";
        let clean = sanitize_text(evil);
        assert_eq!(clean, "Click");
    }

    #[test]
    fn leading_dots_become_visible() {
        assert_eq!(sanitize_filename(".bashrc.svg"), "_bashrc.svg");
    }

    #[test]
    fn trailing_dots_stripped() {
        assert_eq!(sanitize_filename("file.svg..."), "file.svg");
    }

    #[test]
    fn empty_and_dot_names_are_named() {
        assert_eq!(sanitize_filename(""), "untitled");
        assert_eq!(sanitize_filename("."), "_");
        assert_eq!(sanitize_filename(".."), "__");
        assert_eq!(sanitize_filename("/"), "_");
    }

    #[test]
    fn truncates_preserving_extension() {
        let long = format!("{}.svg", "x".repeat(500));
        let out = sanitize_filename(&long);
        assert!(
            out.len() <= MAX_FILENAME_BYTES,
            "{} > {MAX_FILENAME_BYTES}",
            out.len()
        );
        assert!(out.ends_with(".svg"));
    }

    #[test]
    fn unicode_names_survive() {
        assert_eq!(
            sanitize_filename("日本語のファイル.svg"),
            "日本語のファイル.svg"
        );
        let long = "é".repeat(300);
        let out = truncate_name(&long, 50);
        assert!(out.len() <= 50);
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[test]
    fn url_validation() {
        assert!(validate_https_url("https://commons.wikimedia.org/x"));
        assert!(!validate_https_url("http://commons.wikimedia.org/x"));
        assert!(!validate_https_url("file:///etc/passwd"));
        assert!(!validate_https_url("https://user:pass@evil.test/"));
        assert!(!validate_https_url("not a url"));
        assert!(!validate_https_url("javascript:alert(1)"));
    }

    #[test]
    fn markup_stripping() {
        assert_eq!(strip_markup("[[File:Test.svg|thumb|A link]]"), "A link");
        assert_eq!(strip_markup("[[Foo|Bar]]"), "Bar");
        assert_eq!(strip_markup("<b>bold</b> text"), "bold text");
    }

    #[test]
    fn text_truncation_is_utf8_safe() {
        assert_eq!(truncate_text("abc", 2), "a…");
        assert_eq!(truncate_text("日本語", 3), "日本語");
        assert_eq!(truncate_text("日本語です", 4), "日本語…");
    }
}
