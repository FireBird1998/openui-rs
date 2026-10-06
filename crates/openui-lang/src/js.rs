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
pub(crate) fn is_js_whitespace(c: char) -> bool {
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

/// `s.trimEnd()` in JavaScript.
pub(crate) fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_whitespace)
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

/// Names every object inherits from `Object.prototype` (as Bun/JavaScriptCore
/// has them). `name in obj` is true for these even when `obj` doesn't have them.
pub(crate) const OBJECT_PROTOTYPE_KEYS: [&str; 12] = [
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
    "__proto__",
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
];

/// The key as an array index ("0", "17", up to 2³² − 2), if it is one.
/// JavaScript lists such keys first, in numeric order, on every object.
fn array_index(key: &str) -> Option<u32> {
    let canonical = key == "0" || (!key.starts_with('0') && !key.is_empty());
    if !canonical || key.len() > 10 || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u64>()
        .ok()
        .filter(|&n| n < u64::from(u32::MAX))
        .map(|n| n as u32)
}

/// Positions of `keys` in JavaScript property order: array indices ascending,
/// then the other keys in insertion order.
fn js_property_order<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let mut order: Vec<(Option<u32>, usize)> =
        keys.enumerate().map(|(i, k)| (array_index(k), i)).collect();
    if order.iter().any(|(index, _)| index.is_some()) {
        order.sort_by_key(|&(index, i)| match index {
            Some(n) => (0, u64::from(n)),
            None => (1, i as u64),
        });
    }
    order.into_iter().map(|(_, i)| i).collect()
}

/// A string-keyed object with JavaScript's semantics: re-setting a key keeps
/// its place, and iteration lists array-index keys first (ascending), then the
/// rest in insertion order.
#[derive(Clone, Debug, PartialEq)]
pub struct JsObject<V> {
    entries: indexmap::IndexMap<String, V>,
}

impl<V> Default for JsObject<V> {
    fn default() -> Self {
        JsObject {
            entries: indexmap::IndexMap::new(),
        }
    }
}

impl<V> JsObject<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `obj[key] = value`. Assigning `__proto__` changes the prototype instead
    /// of adding a property, so it's dropped here.
    pub fn set(&mut self, key: impl Into<String>, value: V) {
        let key = key.into();
        if key != "__proto__" {
            self.entries.insert(key, value);
        }
    }

    /// Add an own property even if it's `__proto__`, as `JSON.parse` does.
    pub fn define(&mut self, key: impl Into<String>, value: V) {
        self.entries.insert(key.into(), value);
    }

    /// The own property `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.entries.get_mut(key)
    }

    pub fn has_own(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// `key in obj`: an own property, or one inherited from `Object.prototype`.
    pub fn has(&self, key: &str) -> bool {
        self.has_own(key) || OBJECT_PROTOTYPE_KEYS.contains(&key)
    }

    /// `delete obj[key]`.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        self.entries.shift_remove(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in JavaScript property order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        js_property_order(self.entries.keys().map(String::as_str))
            .into_iter()
            .map(|i| self.entries.get_index(i).expect("index in range"))
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
}

/// A JSON object's entries in the order `JSON.parse` would give them.
pub(crate) fn json_entries(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Vec<(&String, &serde_json::Value)> {
    let entries: Vec<_> = map.iter().collect();
    js_property_order(entries.iter().map(|(k, _)| k.as_str()))
        .into_iter()
        .map(|i| entries[i])
        .collect()
}

/// A JSON value's number as `JSON.parse` reads it.
pub(crate) fn json_number(n: &serde_json::Number) -> f64 {
    n.as_f64().unwrap_or(f64::NAN)
}

/// Append a JSON value as `JSON.stringify(JSON.parse(...))` would write it.
pub(crate) fn write_json_value(out: &mut String, value: &serde_json::Value) {
    use serde_json::Value as Json;
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => write_json_number(out, json_number(n)),
        Json::String(s) => write_json_string(out, s),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_value(out, item);
            }
            out.push(']');
        }
        Json::Object(map) => {
            out.push('{');
            for (i, (k, v)) in json_entries(map).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(out, k);
                out.push(':');
                write_json_value(out, v);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(value)` for a JSON value.
pub(crate) fn json_value_string(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_json_value(&mut out, value);
    out
}

/// `String(value)` in JavaScript, for a JSON value: template interpolation
/// and property keys see this.
pub(crate) fn js_string(value: &serde_json::Value) -> String {
    use serde_json::Value as Json;
    match value {
        Json::String(s) => s.clone(),
        Json::Number(n) => number_to_string(json_number(n)),
        Json::Bool(b) => b.to_string(),
        Json::Null => "null".to_owned(),
        Json::Array(items) => items
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Json::Object(_) => "[object Object]".to_owned(),
    }
}

/// JavaScript truthiness of a JSON value; an absent value is falsy.
pub(crate) fn truthy(value: Option<&serde_json::Value>) -> bool {
    use serde_json::Value as Json;
    match value {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(Json::Number(n)) => {
            let n = json_number(n);
            n != 0.0 && !n.is_nan()
        }
        Some(Json::String(s)) => !s.is_empty(),
        Some(Json::Array(_) | Json::Object(_)) => true,
    }
}
