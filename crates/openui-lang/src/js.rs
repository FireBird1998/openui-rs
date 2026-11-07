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

use crate::progress::Progress;

/// Stands in for the lone surrogate U+D800; U+D801 is the next code point, and so on.
const LONE_SURROGATE_BASE: u32 = 0x10_F800;

pub(crate) fn lone_surrogate(unit: u16) -> char {
    char::from_u32(LONE_SURROGATE_BASE + u32::from(unit - 0xD800)).expect("plane 16 code point")
}

/// The lone surrogate a char stands in for, if it stands in for one.
pub(crate) fn lone_surrogate_unit(c: char) -> Option<u16> {
    let code = c as u32;
    (code >= LONE_SURROGATE_BASE).then(|| (code - LONE_SURROGATE_BASE + 0xD800) as u16)
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
pub(crate) fn array_index(key: &str) -> Option<u32> {
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
    /// `obj.__proto__ = null` was assigned: nothing is inherited, and a later
    /// `__proto__` assignment makes an own property.
    null_prototype: bool,
}

impl<V> Default for JsObject<V> {
    fn default() -> Self {
        JsObject {
            entries: indexmap::IndexMap::new(),
            null_prototype: false,
        }
    }
}

impl<V> JsObject<V> {
    /// An empty object.
    pub fn new() -> Self {
        Self::default()
    }

    /// `obj[key] = value`. Assigning `__proto__` changes the prototype instead
    /// of adding a property, so it's dropped here, unless the prototype is
    /// already null.
    pub fn set(&mut self, key: impl Into<String>, value: V) {
        let key = key.into();
        if key != "__proto__" || self.null_prototype {
            self.entries.insert(key, value);
        }
    }

    /// `obj.__proto__ = null`, on an object that still has its prototype.
    pub(crate) fn drop_prototype(&mut self) {
        self.null_prototype = true;
    }

    /// Add an own property even if it's `__proto__`, as `JSON.parse` does.
    pub fn define(&mut self, key: impl Into<String>, value: V) {
        self.entries.insert(key.into(), value);
    }

    /// The own property `key`.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key)
    }

    /// The own property `key`, to change.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.entries.get_mut(key)
    }

    /// `Object.hasOwn(obj, key)`.
    pub fn has_own(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// `key in obj`: an own property, or one inherited from `Object.prototype`.
    pub fn has(&self, key: &str) -> bool {
        self.has_own(key) || (!self.null_prototype && OBJECT_PROTOTYPE_KEYS.contains(&key))
    }

    /// `delete obj[key]`.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        self.entries.shift_remove(key)
    }

    /// How many own properties.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in JavaScript property order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        js_property_order(self.entries.keys().map(String::as_str))
            .into_iter()
            .map(|i| self.entries.get_index(i).expect("index in range"))
    }

    /// Values in JavaScript property order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// Take every value out, in no particular order, leaving it empty.
    pub(crate) fn drain_values(&mut self) -> impl Iterator<Item = V> + '_ {
        self.entries.drain(..).map(|(_, v)| v)
    }
}

/// `Object.entries(value)` for a parsed JSON value, values cloned: an
/// object's entries in property order, an array's by index, a string's UTF-16
/// units by index, and nothing for anything else.
pub(crate) fn object_entries(value: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
    use serde_json::Value as Json;
    match value {
        Json::Object(map) => json_entries(map)
            .into_iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        Json::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.clone()))
            .collect(),
        // A string's entries are its UTF-16 units.
        Json::String(s) => s
            .encode_utf16()
            .enumerate()
            .map(|(i, unit)| {
                let c = match unit {
                    0xD800..=0xDFFF => lone_surrogate(unit),
                    _ => char::from_u32(u32::from(unit)).expect("not a surrogate"),
                };
                (i.to_string(), Json::String(c.to_string()))
            })
            .collect(),
        _ => Vec::new(),
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
    crate::stack::grow_stack!(write_json_value(out, value));
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
    crate::stack::grow_stack!(js_string(value));
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

/// Write JSON as `JSON.stringify` does: the counterpart of [`parse_json`].
/// Keys come in JavaScript's property order, numbers in JavaScript's format,
/// and the code points that stand in for lone surrogates go back out as
/// `\udXXX` escapes.
pub fn stringify_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_json_value(&mut out, value);
    out
}

/// Why [`parse_json`] couldn't read its input.
#[derive(Debug)]
pub struct JsonError(String);

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JsonError {}

/// Read JSON as `JSON.parse` does, into a `serde_json::Value`, including what
/// serde_json alone refuses: a lone surrogate in a `\u` escape is kept (as the
/// plane-16 code point this crate carries it as, written back out as the same
/// escape), and nesting goes as deep as any parse result does. Use it to read
/// back a [`crate::ParseResult::to_json`] exactly, or reference output.
///
/// The reader never recurses, so its stack use doesn't grow with nesting.
/// Nesting deeper than the default limits allow, 800 levels, is refused; no
/// parse result under them comes close (see [`parse_json_with_limits`]). A
/// number too large for a double (`1e400`) is refused too: `JSON.parse` makes
/// it `Infinity`, which a `serde_json::Value` can't hold.
pub fn parse_json(text: &str) -> Result<serde_json::Value, JsonError> {
    parse_json_with_limits(text, crate::Limits::DEFAULT)
}

/// [`parse_json`] under `limits`: nesting deeper than three JSON levels per
/// level of the depth limit, and 32 more, is refused (800 at the default
/// 256), so the JSON of any parse result under the same limits reads back.
/// Its time and memory grow with the text's length, nothing more. A value
/// read under a raised depth limit can nest too deep for its own recursive
/// drop on a small stack: free it with [`crate::free_json`].
pub fn parse_json_with_limits(
    text: &str,
    limits: crate::Limits,
) -> Result<serde_json::Value, JsonError> {
    read_json(text, limits.json_depth())
}

fn read_json(text: &str, limit: usize) -> Result<serde_json::Value, JsonError> {
    JsonReader {
        s: text.as_bytes(),
        text,
        pos: 0,
    }
    .read(limit)
}

struct JsonReader<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
}

enum Open {
    Array(Vec<serde_json::Value>),
    Object(serde_json::Map<String, serde_json::Value>, String),
}

impl JsonReader<'_> {
    fn error(&self, what: &str) -> JsonError {
        JsonError(format!("{what} at byte {}", self.pos))
    }

    fn skip_ws(&mut self) {
        self.pos = crate::lexer::skip_while(self.s, self.pos, |b| {
            matches!(b, b' ' | b'\t' | b'\n' | b'\r')
        });
    }

    fn eat(&mut self, c: u8) -> bool {
        self.skip_ws();
        if self.s.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn read(mut self, limit: usize) -> Result<serde_json::Value, JsonError> {
        use serde_json::Value as Json;
        let mut open: Vec<Open> = Vec::new();
        let mut progress = Progress::new("the JSON reader");
        loop {
            if progress.stalled(self.pos) {
                return Err(self.error("the JSON reader stalled"));
            }
            // A value starts here.
            self.skip_ws();
            let mut value = match self.s.get(self.pos) {
                Some(b'[') | Some(b'{') if open.len() + 1 > limit => {
                    return Err(self.error(&format!("JSON nests deeper than {limit} levels")));
                }
                Some(b'[') => {
                    self.pos += 1;
                    if self.eat(b']') {
                        Json::Array(Vec::new())
                    } else {
                        open.push(Open::Array(Vec::new()));
                        continue;
                    }
                }
                Some(b'{') => {
                    self.pos += 1;
                    if self.eat(b'}') {
                        Json::Object(serde_json::Map::new())
                    } else {
                        let key = self.key()?;
                        open.push(Open::Object(serde_json::Map::new(), key));
                        continue;
                    }
                }
                Some(b'"') => Json::String(self.string()?),
                Some(b'-' | b'0'..=b'9') => self.number()?,
                Some(_) if self.literal("true") => Json::Bool(true),
                Some(_) if self.literal("false") => Json::Bool(false),
                Some(_) if self.literal("null") => Json::Null,
                Some(_) => return Err(self.error("unexpected character")),
                None => return Err(self.error("unexpected end of JSON")),
            };
            // The value is complete: add it to what's open, closing what ends.
            loop {
                match open.last_mut() {
                    None => {
                        self.skip_ws();
                        if self.pos != self.s.len() {
                            return Err(self.error("unexpected text after JSON"));
                        }
                        return Ok(value);
                    }
                    Some(Open::Array(items)) => {
                        items.push(value);
                        if self.eat(b',') {
                            break;
                        }
                        if !self.eat(b']') {
                            return Err(self.error("expected , or ]"));
                        }
                        let Some(Open::Array(items)) = open.pop() else {
                            unreachable!()
                        };
                        value = Json::Array(items);
                    }
                    Some(Open::Object(map, key)) => {
                        // A repeated key keeps its first position and its last value.
                        map.insert(std::mem::take(key), value);
                        if self.eat(b',') {
                            *key = self.key()?;
                            break;
                        }
                        if !self.eat(b'}') {
                            return Err(self.error("expected , or }"));
                        }
                        let Some(Open::Object(map, _)) = open.pop() else {
                            unreachable!()
                        };
                        value = Json::Object(map);
                    }
                }
            }
        }
    }

    /// `"key" :`
    fn key(&mut self) -> Result<String, JsonError> {
        self.skip_ws();
        if self.s.get(self.pos) != Some(&b'"') {
            return Err(self.error("expected a string key"));
        }
        let key = self.string()?;
        if !self.eat(b':') {
            return Err(self.error("expected :"));
        }
        Ok(key)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        let start = self.pos;
        let mut i = start + 1;
        let mut progress = Progress::new("the JSON reader");
        loop {
            if progress.stalled(i) {
                return Err(self.error("the JSON reader stalled"));
            }
            match self.s.get(i) {
                None => return Err(self.error("unterminated string")),
                Some(b'\\') => i += 2,
                Some(b'"') => break,
                Some(_) => i += 1,
            }
        }
        self.pos = i + 1;
        json_parse_string(&self.text[start..self.pos])
            .ok_or_else(|| JsonError(format!("bad string at byte {start}")))
    }

    fn literal(&mut self, word: &str) -> bool {
        let found = self.s[self.pos..].starts_with(word.as_bytes());
        if found {
            self.pos += word.len();
        }
        found
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`, as serde_json stores it:
    /// an integer that fits as an integer, anything else as a double.
    fn number(&mut self) -> Result<serde_json::Value, JsonError> {
        let start = self.pos;
        let digits = |r: &mut Self| {
            let from = r.pos;
            r.pos = crate::lexer::skip_while(r.s, r.pos, |b| b.is_ascii_digit());
            r.pos > from
        };
        self.literal("-");
        match self.s.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(self.error("bad number")),
        }
        let mut integer = true;
        if self.literal(".") {
            integer = false;
            if !digits(self) {
                return Err(self.error("bad number"));
            }
        }
        if matches!(self.s.get(self.pos), Some(b'e' | b'E')) {
            integer = false;
            self.pos += 1;
            if matches!(self.s.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !digits(self) {
                return Err(self.error("bad number"));
            }
        }
        let text = &self.text[start..self.pos];
        if integer {
            if let Ok(n) = text.parse::<u64>() {
                return Ok(n.into());
            }
            if let Ok(n) = text.parse::<i64>() {
                return Ok(n.into());
            }
        }
        let n: f64 = text.parse().map_err(|_| self.error("bad number"))?;
        serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| self.error("number out of range"))
    }
}

#[cfg(test)]
mod json_reader_tests {
    use std::path::Path;

    use super::{parse_json, write_json_value};

    fn goldens(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                goldens(&path, out);
            } else if path.to_str().unwrap().ends_with(".oneshot.json") {
                out.push(path);
            }
        }
    }

    /// Everything the reference wrote with `JSON.stringify` reads back and
    /// writes out byte for byte: lone surrogates, numbers and key order included.
    #[test]
    fn every_reference_result_round_trips() {
        let root = crate::conformance_dir().join("golden");
        let mut files = Vec::new();
        goldens(&root, &mut files);
        assert!(files.len() > 250);
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let value = parse_json(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            let mut out = String::new();
            write_json_value(&mut out, &value);
            assert_eq!(out, text.trim_end(), "{}", file.display());
        }
    }

    #[test]
    fn reads_what_json_parse_reads() {
        for (text, json) in [
            (
                " \t\r\n[1, -0, 1.5e3, -2E-2, 18446744073709551616, \"\\ud800\\udc00\"] ",
                "[1,0,1500,-0.02,18446744073709552000,\"\u{10000}\"]",
            ),
            ("{\"a\":1,\"b\":2,\"a\":3}", "{\"a\":3,\"b\":2}"),
            ("{\"b\":1,\"2\":2,\"1\":3}", "{\"1\":3,\"2\":2,\"b\":1}"),
            ("\"\\ud800\"", "\"\\ud800\""),
            ("{\"__proto__\":[]}", "{\"__proto__\":[]}"),
            ("[[], {}, [{}], \"\u{2028}\"]", "[[],{},[{}],\"\u{2028}\"]"),
        ] {
            let value = parse_json(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            let mut out = String::new();
            write_json_value(&mut out, &value);
            assert_eq!(out, json, "{text}");
        }
    }

    #[test]
    fn refuses_what_json_parse_refuses() {
        for text in [
            "",
            " ",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "-",
            "1.",
            ".5",
            "+1",
            "1e",
            "1e+",
            "NaN",
            "Infinity",
            "'a'",
            "[1 2]",
            "{\"a\" 1}",
            "{a: 1}",
            "\"\\x\"",
            "\"\u{1}\"",
            "\"unterminated",
            "[1] x",
            "// c\n1",
            "tru",
            "nul",
            "[",
            "{\"a\":",
            "1e400",
            "\u{a0}1",
        ] {
            assert!(parse_json(text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn nesting_is_bounded_without_recursing() {
        let at_limit = format!("{}{}", "[".repeat(800), "]".repeat(800));
        assert!(parse_json(&at_limit).is_ok());
        let past = format!("{}{}", "[".repeat(801), "]".repeat(801));
        assert!(parse_json(&past).is_err());
        let objects = format!("{}1{}", "{\"a\":".repeat(801), "}".repeat(801));
        assert!(parse_json(&objects).is_err());
    }
}
