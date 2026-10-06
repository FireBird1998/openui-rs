//! JavaScript behaviour the reference parser relies on, reproduced exactly:
//! `String.prototype.trim`, `JSON.parse` of a string literal, and how
//! `JSON.stringify` writes strings and numbers.
//!
//! JavaScript strings are UTF-16 and may hold a lone surrogate, as when a
//! stream stops between the halves of `"\uD83D\uDE00"`. Rust strings can't,
//! so a lone surrogate travels as one of the last 2048 code points of plane 16
//! (U+10F800–U+10FFFF) and is written back out as the `\udXXX` escape
//! `JSON.stringify` uses. Programs that contain those private-use code points
//! themselves can't conform.

use std::fmt::Write as _;

/// Stands in for the lone surrogate U+D800; U+D801 is the next code point, and so on.
const LONE_SURROGATE_BASE: u32 = 0x10_F800;

fn lone_surrogate(unit: u16) -> char {
    char::from_u32(LONE_SURROGATE_BASE + u32::from(unit - 0xD800)).expect("plane 16 code point")
}

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

/// `JSON.parse` of a whole JSON string literal, quotes included; `None` where
/// `JSON.parse` would throw (bad escape, raw control character, no closing quote).
pub(crate) fn json_parse_string(literal: &str) -> Option<String> {
    let s = literal.as_bytes();
    if s.first() != Some(&b'"') {
        return None;
    }
    let mut out = String::with_capacity(literal.len());
    let mut high: Option<u16> = None; // a `\uD8xx` escape waiting for its low half
    let flush = |out: &mut String, high: &mut Option<u16>| {
        if let Some(unit) = high.take() {
            out.push(lone_surrogate(unit));
        }
    };
    let mut i = 1;
    loop {
        match *s.get(i)? {
            b'"' => {
                flush(&mut out, &mut high);
                return (i + 1 == s.len()).then_some(out);
            }
            b'\\' => {
                let escaped = match *s.get(i + 1)? {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{08}',
                    b'f' => '\u{0C}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'u' => {
                        let hex = literal.get(i + 2..i + 6)?;
                        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                            return None;
                        }
                        let unit = u16::from_str_radix(hex, 16).ok()?;
                        i += 6;
                        match unit {
                            0xD800..=0xDBFF => {
                                flush(&mut out, &mut high);
                                high = Some(unit);
                            }
                            0xDC00..=0xDFFF => match high.take() {
                                Some(h) => {
                                    let c = 0x10000
                                        + ((u32::from(h) - 0xD800) << 10)
                                        + (u32::from(unit) - 0xDC00);
                                    out.push(char::from_u32(c).expect("valid surrogate pair"));
                                }
                                None => out.push(lone_surrogate(unit)),
                            },
                            _ => {
                                flush(&mut out, &mut high);
                                out.push(char::from_u32(u32::from(unit)).expect("not a surrogate"));
                            }
                        }
                        continue;
                    }
                    _ => return None,
                };
                flush(&mut out, &mut high);
                out.push(escaped);
                i += 2;
            }
            0x00..=0x1F => return None,
            _ => {
                flush(&mut out, &mut high);
                let c = literal[i..].chars().next()?;
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
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
            c if c as u32 >= LONE_SURROGATE_BASE => {
                let unit = c as u32 - LONE_SURROGATE_BASE + 0xD800;
                let _ = write!(out, "\\u{unit:04x}");
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

/// Append `n` as `JSON.stringify` writes a number: JavaScript's shortest
/// round-trip form (`1e+21`, `1e-7`, `-0` as `0`), and `null` when not finite.
pub(crate) fn write_json_number(out: &mut String, n: f64) {
    if n.is_finite() {
        out.push_str(ryu_js::Buffer::new().format_finite(n));
    } else {
        out.push_str("null");
    }
}

/// `String(n)` in JavaScript: like [`write_json_number`] for finite numbers,
/// and `NaN`, `Infinity` or `-Infinity` otherwise.
pub(crate) fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_owned()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        ryu_js::Buffer::new().format_finite(n).to_owned()
    }
}
