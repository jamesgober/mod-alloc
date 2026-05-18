//! Minimal JSON writer for the DHAT output schema.
//!
//! Hand-rolled, no external deps. The schema is fixed (see
//! `super::mod`), so we only need three primitives: number,
//! boolean, and JSON-escaped string. Object / array layout is
//! handled inline by the caller.

use std::fmt::Write as _;

/// Append `s` to `out` as a JSON string literal (with surrounding
/// quotes and RFC 8259-compliant escaping for the small subset of
/// characters we actually emit).
pub(super) fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Append a `"key":` token (with trailing colon, no surrounding
/// whitespace) to `out`. Caller handles the value and any leading
/// comma.
pub(super) fn write_key(out: &mut String, key: &str) {
    write_json_string(out, key);
    out.push(':');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quote_and_backslash() {
        let mut buf = String::new();
        write_json_string(&mut buf, "a\"b\\c");
        assert_eq!(buf, "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn escapes_control_chars() {
        let mut buf = String::new();
        write_json_string(&mut buf, "x\ny\rz\t\x01end");
        assert_eq!(buf, "\"x\\ny\\rz\\t\\u0001end\"");
    }

    #[test]
    fn passes_through_unicode_at_or_above_0x20() {
        let mut buf = String::new();
        write_json_string(&mut buf, "héllo ✓");
        assert_eq!(buf, "\"héllo ✓\"");
    }

    #[test]
    fn empty_string_emits_quoted_empty() {
        let mut buf = String::new();
        write_json_string(&mut buf, "");
        assert_eq!(buf, "\"\"");
    }

    #[test]
    fn write_key_appends_colon() {
        let mut buf = String::new();
        write_key(&mut buf, "tb");
        assert_eq!(buf, "\"tb\":");
    }
}
