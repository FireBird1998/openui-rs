//! JavaScript behaviour the reference parser relies on, reproduced exactly:
//! `String.prototype.trim`, `JSON.parse` of a string literal, and how
//! `JSON.stringify` writes strings and numbers.
//!
//! JavaScript strings are UTF-16 and may hold a lone surrogate, as when a
//! stream stops between the halves of `"\uD83D\uDE00"`. Rust strings can't,
//! so a lone surrogate travels as one of the last 2048 code points of plane 16
//! (U+10F800–U+10FFFF) and is written back out as the `\udXXX` escape
//! `JSON.stringify` uses. Text that is, or decodes to, a character in that
//! range collides with this representation, including a program's string escapes.

use std::borrow::Cow;
use std::fmt::Write as _;

use crate::progress::Progress;

#[path = "host_json.rs"]
mod host_json;
pub use host_json::write_host_json;

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

/// Iterate JavaScript UTF-16 units without allocating, including lone
/// surrogates in the core's public string representation. Unlike Rust's
/// `encode_utf16`, the reserved U+10F800–U+10FFFF scalars each produce one unit.
/// Text that is, or decodes to, a character in that range collides with this
/// representation.
pub fn utf16_units(text: &str) -> impl Iterator<Item = u16> + '_ {
    text.chars().flat_map(|c| {
        let (units, len) = if let Some(unit) = lone_surrogate_unit(c) {
            ([unit, 0], 1)
        } else {
            let mut units = [0u16; 2];
            let len = c.encode_utf16(&mut units).len();
            (units, len)
        };
        units.into_iter().take(len)
    })
}

/// Encode one JavaScript UTF-16 unit in the core's Rust string representation.
pub fn utf16_unit_string(unit: u16) -> String {
    char::from_u32(u32::from(unit))
        .unwrap_or_else(|| lone_surrogate(unit))
        .to_string()
}

/// Combine adjacent surrogate halves in a string using the core's UTF-16
/// representation, as JavaScript sees them when independently supplied
/// UTF-16 units meet: each high half directly followed by a low half becomes
/// the character they encode. Every other half keeps its reserved scalar.
/// Text with no such pair comes back as it was given, borrowed or owned,
/// without a copy.
pub fn normalize_surrogate_pairs<'a>(text: impl Into<Cow<'a, str>>) -> Cow<'a, str> {
    let text = text.into();
    let mut high = false;
    let has_pair = text.chars().map(lone_surrogate_unit).any(|unit| {
        let pair = high && matches!(unit, Some(0xDC00..=0xDFFF));
        high = matches!(unit, Some(0xD800..=0xDBFF));
        pair
    });
    if !has_pair {
        return text;
    }
    Cow::Owned(
        char::decode_utf16(utf16_units(&text))
            .map(|unit| unit.unwrap_or_else(|e| lone_surrogate(e.unpaired_surrogate())))
            .collect(),
    )
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
        write_json_char::<false>(out, c);
    }
    out.push('"');
}

#[inline(always)]
pub(crate) fn write_json_char<const EXACT_UTF16: bool>(out: &mut String, c: char) {
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
        c if !EXACT_UTF16 && c as u32 >= LONE_SURROGATE_BASE => {
            let unit = c as u32 - LONE_SURROGATE_BASE + 0xD800;
            let _ = write!(out, "\\u{unit:04x}");
        }
        c => out.push(c),
    }
}

#[inline]
pub(crate) fn write_json_surrogate(out: &mut String, unit: u16) {
    let _ = write!(out, "\\u{unit:04x}");
}

/// UTF-8 bytes in JSON.stringify's escaped string body, without quotes.
/// A control character or lone surrogate can require six bytes.
pub fn json_string_len(s: &str) -> usize {
    saturating_total(s.chars().map(json_char_len))
}

// Saturating, so a raised limit on a 32-bit target cannot wrap past it. Its
// own function so a test can reach the arithmetic no 64-bit string can.
fn saturating_total(lengths: impl Iterator<Item = usize>) -> usize {
    lengths.fold(0usize, usize::saturating_add)
}

#[inline]
pub(crate) fn json_char_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\u{08}' | '\u{0C}' | '\n' | '\r' | '\t' => 2,
        c if c < '\u{20}' || c as u32 >= LONE_SURROGATE_BASE => 6,
        c => c.len_utf8(),
    }
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

/// The UTF-8 byte length of a JSON number, without allocating. Non-finite
/// numbers count as `null`, and negative zero as `0`.
pub fn json_number_len(n: f64) -> usize {
    if n.is_finite() {
        ryu_js::Buffer::new().format_finite(n).len()
    } else {
        4
    }
}

/// JavaScript’s string representation of a double, including non-finite numbers.
pub fn number_to_string(n: f64) -> String {
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
    let canonical = key == "0" || !key.starts_with('0');
    if !canonical || !key.bytes().all(|b| b.is_ascii_digit()) {
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
#[derive(Clone)]
pub struct JsObject<V> {
    entries: indexmap::IndexMap<String, V>,
    /// `obj.__proto__ = null` was assigned: nothing is inherited, and a later
    /// `__proto__` assignment makes an own property.
    null_prototype: bool,
    /// An object-valued prototype assignment was intentionally discarded.
    /// Parser validation must not substitute Object.prototype for this unknown
    /// prototype. This is provenance, not an own property or a different value.
    unrepresented_prototype: bool,
}

impl<V: std::fmt::Debug> std::fmt::Debug for JsObject<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsObject")
            .field("entries", &self.entries)
            .field("null_prototype", &self.null_prototype)
            .finish()
    }
}

impl<V: PartialEq> PartialEq for JsObject<V> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.null_prototype == other.null_prototype
    }
}

impl<V> Default for JsObject<V> {
    fn default() -> Self {
        JsObject {
            entries: indexmap::IndexMap::new(),
            null_prototype: false,
            unrepresented_prototype: false,
        }
    }
}

impl<V> JsObject<V> {
    /// An empty object.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty object with space for the given number of own properties.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: indexmap::IndexMap::with_capacity(capacity),
            null_prototype: false,
            unrepresented_prototype: false,
        }
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

    /// A copy made by object spread has ordinary inheritance, even when
    /// its source has none. Own properties retain their order and values.
    pub(crate) fn spread(&self) -> Self
    where
        V: Clone,
    {
        let mut copy = self.clone();
        copy.null_prototype = false;
        copy.unrepresented_prototype = false;
        copy
    }

    /// `obj.__proto__ = null`, on an object that still has its prototype.
    pub(crate) fn drop_prototype(&mut self) {
        self.null_prototype = true;
        self.unrepresented_prototype = false;
    }

    pub(crate) fn note_unrepresented_prototype(&mut self) {
        self.unrepresented_prototype = true;
    }

    pub(crate) fn same_prototype(&self, other: &Self) -> bool {
        self.null_prototype == other.null_prototype
            && self.unrepresented_prototype == other.unrepresented_prototype
    }

    pub(crate) fn has_unrepresented_prototype(&self) -> bool {
        self.unrepresented_prototype
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

    /// Entries in insertion order. Unlike [`JsObject::iter`], this computes
    /// no property order, so it allocates nothing: for measuring, where the
    /// order does not matter.
    pub fn iter_unordered(&self) -> impl Iterator<Item = (&String, &V)> {
        self.entries.iter()
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
pub struct JsonError {
    message: String,
    depth_limited: bool,
}

impl JsonError {
    /// Whether reading JSON stopped at the configured nesting limit.
    pub fn depth_limited(&self) -> bool {
        self.depth_limited
    }
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
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
/// Nesting deeper than [`crate::Limits::depth`] allows by default is refused; no
/// parse result under them comes close (see [`parse_json_with_limits`]). A
/// number too large for a double (`1e400`) is refused too: `JSON.parse` makes
/// it `Infinity`, which a `serde_json::Value` can't hold.
pub fn parse_json(text: &str) -> Result<serde_json::Value, JsonError> {
    parse_json_with_limits(text, crate::Limits::DEFAULT)
}

/// [`parse_json`] under `limits`: nesting past the JSON limit described by
/// [`crate::Limits::depth`] is refused, so the JSON of any parse result
/// under the same limits reads back.
/// Its time and memory grow with the text's length, nothing more. A value
/// read under a raised depth limit can nest too deep for its own recursive
/// drop on a small stack: free it with [`crate::free_json`].
pub fn parse_json_with_limits(
    text: &str,
    limits: crate::Limits,
) -> Result<serde_json::Value, JsonError> {
    let _ceiling = crate::stack::limit(limits.depth());
    JsonReader::new(text).read::<_, false>(limits.json_depth(), json_parse_string)
}

/// Read host JSON directly as an OpenUI [Value](crate::Value).
///
/// Each open array or object counts toward the depth limit. The reader and the
/// returned value's destruction use bounded stack space, including on a
/// syntax error or replacement of a duplicate key. Numbers retain JavaScript's
/// double values, including negative zero and overflow to infinity.
pub fn parse_json_value(text: &str, max_depth: usize) -> Result<crate::Value, JsonError> {
    JsonReader::new(text).read::<_, false>(max_depth, json_parse_string)
}

/// Message context uses the core's JavaScript numbers and stack-safe values.
pub(crate) fn read_message_json(
    text: &str,
    limits: crate::Limits,
) -> Result<crate::Value, JsonError> {
    parse_json_value(text, limits.json_depth())
}

// The bundle decoder has already canonicalized surrogate pairs in its own
// lossless string representation.
pub(crate) fn read_bundle_json(
    text: &str,
    limits: crate::Limits,
    decode_string: impl Copy + Fn(&str) -> Option<String>,
) -> Result<crate::Value, JsonError> {
    JsonReader::new(text).read::<_, true>(limits.json_depth(), decode_string)
}

struct JsonReader<'a> {
    s: &'a [u8],
    text: &'a str,
    pos: usize,
}

enum Open<V: JsonTarget> {
    Array(Vec<V>),
    Object(V::Fields, String),
}

/// What the reader has open. A refused text leaves values read so far, as
/// deep as the limit; they are freed with [`JsonTarget::free`], so refusing
/// needs no stack however deep they nest (#203).
struct Opened<V: JsonTarget>(Vec<Open<V>>);

impl<V: JsonTarget> Drop for Opened<V> {
    fn drop(&mut self) {
        while let Some(open) = self.0.pop() {
            V::free(match open {
                Open::Array(items) => V::array(items),
                Open::Object(fields, _) => V::object(fields),
            });
        }
    }
}

// The legacy serde_json entry point and the host-value entry point use the
// same grammar. Only the latter can store all of JavaScript's double values.
trait JsonTarget: Sized {
    type Fields: Default;

    fn null() -> Self;
    fn boolean(value: bool) -> Self;
    fn string<const EXACT_UTF16: bool>(text: String) -> Self;
    fn number(text: &str, integer: bool) -> Option<Self>;
    fn array(items: Vec<Self>) -> Self;
    fn object(fields: Self::Fields) -> Self;
    fn define<const EXACT_UTF16: bool>(fields: &mut Self::Fields, key: String, value: Self);
    /// Let go of a value the reader won't return.
    fn free(self) {
        drop(self);
    }
}

impl JsonTarget for serde_json::Value {
    type Fields = serde_json::Map<String, Self>;

    fn null() -> Self {
        Self::Null
    }
    fn boolean(value: bool) -> Self {
        Self::Bool(value)
    }
    fn string<const EXACT_UTF16: bool>(text: String) -> Self {
        Self::String(text)
    }
    fn number(text: &str, integer: bool) -> Option<Self> {
        if integer {
            if let Ok(n) = text.parse::<u64>() {
                return Some(n.into());
            }
            if let Ok(n) = text.parse::<i64>() {
                return Some(n.into());
            }
        }
        serde_json::Number::from_f64(text.parse().ok()?).map(Self::Number)
    }
    fn array(items: Vec<Self>) -> Self {
        Self::Array(items)
    }
    fn object(fields: Self::Fields) -> Self {
        Self::Object(fields)
    }
    fn define<const EXACT_UTF16: bool>(fields: &mut Self::Fields, key: String, value: Self) {
        // A repeated key's earlier value may nest to the limit.
        if let Some(earlier) = fields.insert(key, value) {
            crate::free_json(earlier);
        }
    }
    /// serde_json drops a value by recursing: free it with a loop instead.
    fn free(self) {
        crate::free_json(self);
    }
}

impl JsonTarget for crate::Value {
    type Fields = JsObject<Self>;

    fn null() -> Self {
        Self::Null
    }
    fn boolean(value: bool) -> Self {
        Self::Bool(value)
    }
    fn string<const EXACT_UTF16: bool>(text: String) -> Self {
        Self::Str(if EXACT_UTF16 {
            text
        } else {
            normalize_surrogate_pairs(text).into_owned()
        })
    }
    fn number(text: &str, _: bool) -> Option<Self> {
        text.parse().ok().map(Self::Num)
    }
    fn array(items: Vec<Self>) -> Self {
        Self::Arr(std::sync::Arc::new(items))
    }
    fn object(fields: Self::Fields) -> Self {
        Self::Obj(std::sync::Arc::new(fields))
    }
    fn define<const EXACT_UTF16: bool>(fields: &mut Self::Fields, key: String, value: Self) {
        fields.define(
            if EXACT_UTF16 {
                key
            } else {
                normalize_surrogate_pairs(key).into_owned()
            },
            value,
        );
    }
}

impl<'a> JsonReader<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            s: text.as_bytes(),
            text,
            pos: 0,
        }
    }

    fn error(&self, what: &str) -> JsonError {
        JsonError {
            message: format!("{what} at byte {}", self.pos),
            depth_limited: false,
        }
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

    fn read<V: JsonTarget, const EXACT_UTF16: bool>(
        mut self,
        limit: usize,
        decode_string: impl Copy + Fn(&str) -> Option<String>,
    ) -> Result<V, JsonError> {
        let mut open: Opened<V> = Opened(Vec::new());
        let mut progress = Progress::new("the JSON reader");
        loop {
            if progress.stalled(self.pos) {
                return Err(self.error("the JSON reader stalled"));
            }
            // A value starts here.
            self.skip_ws();
            let mut value = match self.s.get(self.pos) {
                Some(b'[') | Some(b'{') if open.0.len() >= limit => {
                    return Err(JsonError {
                        depth_limited: true,
                        ..self.error(&format!("JSON nests deeper than {limit} levels"))
                    });
                }
                Some(b'[') => {
                    self.pos += 1;
                    if self.eat(b']') {
                        V::array(Vec::new())
                    } else {
                        open.0.push(Open::Array(Vec::new()));
                        continue;
                    }
                }
                Some(b'{') => {
                    self.pos += 1;
                    if self.eat(b'}') {
                        V::object(V::Fields::default())
                    } else {
                        let key = self.key(decode_string)?;
                        open.0.push(Open::Object(V::Fields::default(), key));
                        continue;
                    }
                }
                Some(b'"') => V::string::<EXACT_UTF16>(self.string(decode_string)?),
                Some(b'-' | b'0'..=b'9') => self.number::<V>()?,
                Some(_) if self.literal("true") => V::boolean(true),
                Some(_) if self.literal("false") => V::boolean(false),
                Some(_) if self.literal("null") => V::null(),
                Some(_) => return Err(self.error("unexpected character")),
                None => return Err(self.error("unexpected end of JSON")),
            };
            // The value is complete: add it to what's open, closing what ends.
            loop {
                match open.0.last_mut() {
                    None => {
                        self.skip_ws();
                        if self.pos != self.s.len() {
                            value.free();
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
                        let Some(Open::Array(items)) = open.0.pop() else {
                            return Err(self.error("expected an open array"));
                        };
                        value = V::array(items);
                    }
                    Some(Open::Object(map, key)) => {
                        // A repeated key keeps its first position and its last value.
                        V::define::<EXACT_UTF16>(map, std::mem::take(key), value);
                        if self.eat(b',') {
                            *key = self.key(decode_string)?;
                            break;
                        }
                        if !self.eat(b'}') {
                            return Err(self.error("expected , or }"));
                        }
                        let Some(Open::Object(map, _)) = open.0.pop() else {
                            return Err(self.error("expected an open object"));
                        };
                        value = V::object(map);
                    }
                }
            }
        }
    }

    /// `"key" :`
    fn key(
        &mut self,
        decode_string: impl Copy + Fn(&str) -> Option<String>,
    ) -> Result<String, JsonError> {
        self.skip_ws();
        if self.s.get(self.pos) != Some(&b'"') {
            return Err(self.error("expected a string key"));
        }
        let key = self.string(decode_string)?;
        if !self.eat(b':') {
            return Err(self.error("expected :"));
        }
        Ok(key)
    }

    fn string(
        &mut self,
        decode_string: impl Copy + Fn(&str) -> Option<String>,
    ) -> Result<String, JsonError> {
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
        self.text
            .get(start..self.pos)
            .and_then(decode_string)
            .ok_or_else(|| JsonError {
                message: format!("bad string at byte {start}"),
                depth_limited: false,
            })
    }

    fn literal(&mut self, word: &str) -> bool {
        let found = self
            .s
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(word.as_bytes()));
        if found {
            self.pos += word.len();
        }
        found
    }

    // Scan the JSON number before converting it to the target representation.
    fn number<V: JsonTarget>(&mut self) -> Result<V, JsonError> {
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
        self.text
            .get(start..self.pos)
            .and_then(|text| V::number(text, integer))
            .ok_or_else(|| self.error("number out of range"))
    }
}

#[cfg(test)]
mod json_reader_tests {
    use std::path::Path;

    use super::{json_string_len, parse_json, write_json_string, write_json_value};

    /// What the result-size limit charges for a string is what's written.
    #[test]
    fn a_strings_charge_is_its_escaped_length() {
        let lone = char::from_u32(super::LONE_SURROGATE_BASE + 0x10).unwrap();
        for s in [
            "",
            "plain",
            "é😀",
            "\"\\\u{08}\u{0C}\n\r\t",
            "\u{0}\u{1}\u{1f}",
            &format!("a{lone}b"),
        ] {
            let mut out = String::new();
            write_json_string(&mut out, s);
            assert_eq!(json_string_len(s) + 2, out.len(), "{s:?}");
        }
    }

    /// A string's charge stops at `usize::MAX` rather than wrapping. No
    /// string on a 64-bit machine is long enough to show it, so this checks
    /// the sum `json_string_len` folds its characters' lengths with.
    #[test]
    fn a_strings_charge_saturates_instead_of_wrapping() {
        let six = super::json_char_len('\u{0}');
        assert_eq!(six, 6);
        let near = usize::MAX - 5;
        assert_eq!(super::saturating_total([near, six].into_iter()), usize::MAX);
        assert_eq!(
            super::saturating_total([near, six, six].into_iter()),
            usize::MAX
        );
        assert_eq!(super::saturating_total([near, 5].into_iter()), usize::MAX);
        assert_eq!(
            super::saturating_total([near, 4].into_iter()),
            usize::MAX - 1
        );
        // Below the ceiling it is the plain sum.
        assert_eq!(
            super::saturating_total(std::iter::repeat_n(six, 1 << 20)),
            six << 20
        );
    }

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

#[cfg(test)]
mod issue16_tests {
    use super::*;

    #[test]
    fn object_order_distinguishes_indices_from_other_numeric_keys() {
        let text = r#"{"ordinary":0,"4294967295":1,"4294967294":2,"1000000000":3,"01":4,"":5,"0":6,"-1":7,"12345678901":8,"1a":9}"#;
        let value = parse_json(text).unwrap();
        let mut out = String::new();
        write_json_value(&mut out, &value);
        assert_eq!(
            out,
            r#"{"0":6,"1000000000":3,"4294967294":2,"ordinary":0,"4294967295":1,"01":4,"":5,"-1":7,"12345678901":8,"1a":9}"#
        );
    }

    #[test]
    fn escaped_and_plain_strings_and_number_scans_advance() {
        for text in [r#""a\nb""#, r#""abc""#, " \t\n12345 ", "-12.34e+56"] {
            assert!(parse_json(text).is_ok(), "{text}");
        }
    }
}

#[cfg(test)]
mod surrogate_normalizer_tests {
    use super::*;

    /// The two normalizers this one replaced (#120) both rewrote with this
    /// decode: `normalize_js_string` whenever the text held a reserved
    /// stand-in, `normalize_surrogate_pairs` whenever it held a pair of them.
    /// A text with no stand-in decodes to itself, so the old
    /// `normalize_js_string` equals this decode on every input, and a
    /// differential test over the cases below showed the old pair normalizer
    /// did too before they were merged.
    fn rewritten(text: &str) -> String {
        char::decode_utf16(utf16_units(text))
            .map(|unit| unit.unwrap_or_else(|e| lone_surrogate(e.unpaired_surrogate())))
            .collect()
    }

    fn cases() -> Vec<String> {
        let high = |unit: u16| lone_surrogate(unit).to_string();
        let low = high;
        let mut cases: Vec<String> = vec![
            String::new(),
            "plain ascii".into(),
            "é漢😀".into(),
            high(0xD800),
            high(0xDBFF),
            low(0xDC00),
            low(0xDFFF),
            high(0xD83D) + &low(0xDE00),
            high(0xDBFF) + &low(0xDFFF),
            high(0xD800) + &low(0xDC00),
            low(0xDE00) + &high(0xD83D),
            high(0xD83D) + "x" + &low(0xDE00),
            high(0xD83D) + "😀" + &low(0xDE00),
            "😀".to_owned() + &low(0xDE00),
            high(0xD83D) + "😀",
            high(0xD83D) + &high(0xD83D) + &low(0xDE00),
            high(0xD83D) + &low(0xDE00) + &low(0xDE00),
            high(0xD83D) + &low(0xDE00) + &high(0xD83D) + &low(0xDE00),
            "\u{fdcf}\u{fdd0}\u{fdd1}\u{fdef}\u{fdf0}".into(),
            high(0xD83D) + "\u{fdd0}" + &low(0xDE00),
            "\u{fdd0}".to_owned() + &high(0xD83D) + &low(0xDE00) + "\u{fdd0}",
            "\u{10f7ff}\u{10f800}\u{10ffff}".into(),
            "\u{10f7ff}".to_owned() + &low(0xDC00),
            "\u{fffd}\u{ffff}\u{10000}".into(),
            (high(0xD83D) + &low(0xDE00)).repeat(10_000),
            "a".repeat(100_000) + &high(0xD83D) + &low(0xDE00),
            high(0xD83D).repeat(5_000) + &low(0xDE00).repeat(5_000),
        ];
        // Every reserved stand-in, alone and before and after every other kind.
        for code in 0x10_F800..=0x10_FFFF_u32 {
            let c = char::from_u32(code).map(String::from).unwrap_or_default();
            cases.push(c.clone());
            cases.push(high(0xD83D) + &c);
            cases.push(c.clone() + &low(0xDE00));
        }
        // Random strings over a small, collision-heavy alphabet, fixed seed.
        let alphabet: Vec<String> = vec![
            "a".into(),
            "😀".into(),
            "\u{fdd0}".into(),
            "\u{fdcf}".into(),
            "\u{10f7ff}".into(),
            high(0xD800),
            high(0xD83D),
            high(0xDBFF),
            low(0xDC00),
            low(0xDE00),
            low(0xDFFF),
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = (next() % 12) as usize;
            let text: String = (0..len)
                .map(|_| alphabet[(next() % alphabet.len() as u64) as usize].as_str())
                .collect();
            cases.push(text);
        }
        cases
    }

    #[test]
    fn the_normalizer_matches_both_former_ones_on_every_input() {
        for text in cases() {
            let expected = rewritten(&text);
            let borrowed = normalize_surrogate_pairs(text.as_str());
            assert_eq!(
                *borrowed,
                *expected,
                "{:?}",
                text.escape_unicode().to_string()
            );
            // Unchanged text is never copied, borrowed or owned.
            assert_eq!(matches!(borrowed, Cow::Borrowed(_)), *borrowed == *text);
            let owned = normalize_surrogate_pairs(text.clone()).into_owned();
            assert_eq!(owned, expected);
            let unchanged = text.clone();
            let unchanged_address = unchanged.as_ptr();
            if let Cow::Owned(same) = normalize_surrogate_pairs(unchanged)
                && same == text
            {
                assert_eq!(same.as_ptr(), unchanged_address, "copied unchanged text");
            }
        }
    }
}
