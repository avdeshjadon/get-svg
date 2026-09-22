//! Property-based tests for the security-critical filename/text paths.

use get_svg::security::{
    has_control_chars, safe_join, sanitize_filename, sanitize_text, validate_https_url,
};
use proptest::prelude::*;

proptest! {
    /// Sanitizing must be idempotent: applying it twice never changes output.
    #[test]
    fn sanitize_is_idempotent(name in "\\PC{0,120}") {
        let once = sanitize_filename(&name);
        let twice = sanitize_filename(&once);
        prop_assert_eq!(once, twice);
    }

    /// Sanitized names must never contain path separators or control chars.
    #[test]
    fn sanitize_never_contains_separators_or_controls(name in "\\PC{0,200}") {
        let out = sanitize_filename(&name);
        prop_assert!(!out.contains('/'));
        prop_assert!(!out.contains('\\'));
        prop_assert!(!has_control_chars(&out));
        prop_assert!(out.len() <= get_svg::security::MAX_FILENAME_BYTES);
    }

    /// Joins always stay inside the base directory.
    #[test]
    fn safe_join_never_escapes(name in "\\PC{1,80}") {
        let base = std::path::Path::new("/tmp/out");
        if let Ok(joined) = safe_join(base, &name) {
            prop_assert!(joined.starts_with(base));
        }
    }

    /// Terminal sanitization must remove every escape/control byte (line
    /// breaks are deliberately preserved for multi-line fields, so only
    /// non-newline control characters must never survive).
    #[test]
    fn sanitize_text_removes_controls(bytes in prop::collection::vec(any::<u8>(), 0..80)) {
        let s = String::from_utf8_lossy(&bytes);
        let out = sanitize_text(&s);
        prop_assert!(
            !out.chars().any(|c| c.is_control() && c != '\n'),
            "control chars survived sanitization"
        );
    }

    /// URL validation never accepts non-HTTPS schemes.
    #[test]
    fn urls_must_be_https(scheme in prop::collection::vec(any::<u8>(), 0..16)) {
        let prefix = String::from_utf8_lossy(&scheme).to_string();
        let candidate = format!("{prefix}example.com/x.svg");
        let accepted = validate_https_url(&candidate);
        if accepted {
            prop_assert!(candidate.starts_with("https://"));
        }
    }
}
