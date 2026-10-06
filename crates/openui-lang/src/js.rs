//! JavaScript string behaviour the reference parser relies on, reproduced
//! exactly: `String.prototype.trim` and `JSON.stringify` string escaping.

use std::fmt::Write as _;

/// The characters `String.prototype.trim` removes (WhiteSpace + LineTerminator).
/// Differs from `char::is_whitespace`: includes U+FEFF, excludes U+0085.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `s.trim()` in JavaScript.
pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// Append `s` as a JSON string literal, escaped exactly as `JSON.stringify` does.
pub(crate) fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\u{20}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.stringify(s)` for a string.
pub(crate) fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_json_string(&mut out, s);
    out
}
