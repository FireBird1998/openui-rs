//! Lossless UTF-16 in UTF-8 storage, so the message splitter can work on
//! ASCII syntax without conflating lone surrogates with real Unicode scalars.

use std::{borrow::Cow, cmp::Ordering};

use crate::js::{
    read_bundle_json, read_message_json, write_json_char, write_json_string, write_json_surrogate,
};
use crate::{JsonError, Limits, Value};

const ESCAPE: char = '\u{fdd0}';
const SURROGATE_BASE: u32 = 0xe000;

// Rust readers share the core's string convention. The WebAssembly boundary
// instead carries the exact units of JavaScript strings through to JSON.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TextFormat {
    Core,
    ExactUtf16,
}

impl TextFormat {
    pub(super) fn attribute(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Core => Cow::Owned(crate::message::decode_attribute(text)),
            Self::ExactUtf16 => decode_attribute(text),
        }
    }

    pub(super) fn read_json(self, text: &str, limits: Limits) -> Result<Value, JsonError> {
        match self {
            Self::Core => read_message_json(text, limits),
            Self::ExactUtf16 => read_bundle_json(text, limits, decode_json_string),
        }
    }

    pub(super) fn write_json(self, out: &mut String, text: &str) {
        match self {
            Self::Core => write_json_string(out, text),
            Self::ExactUtf16 => write_exact_json(out, text),
        }
    }

    pub(super) fn compare(self, left: &str, right: &str) -> Ordering {
        match self {
            Self::Core => crate::utf16_units(left).cmp(crate::utf16_units(right)),
            Self::ExactUtf16 => units(left).cmp(units(right)),
        }
    }
}

fn write_exact_json(out: &mut String, text: &str) {
    out.push('"');
    for ch in chars(text) {
        match ch {
            TextChar::Scalar(c) => write_json_char::<true>(out, c),
            TextChar::Surrogate(unit) => write_json_surrogate(out, unit),
        }
    }
    out.push('"');
}

fn encode(text: &str) -> Cow<'_, str> {
    if !text.contains(ESCAPE) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        push_scalar(&mut out, ch);
    }
    Cow::Owned(out)
}

pub(super) fn encode_utf16(input: impl Iterator<Item = u16>) -> String {
    let mut out = String::new();
    for ch in char::decode_utf16(input) {
        match ch {
            Ok(ch) => push_scalar(&mut out, ch),
            Err(err) => {
                out.push(ESCAPE);
                if let Some(ch) = char::from_u32(
                    SURROGATE_BASE + u32::from(err.unpaired_surrogate().saturating_sub(0xd800)),
                ) {
                    out.push(ch);
                }
            }
        }
    }
    out
}

/// Where `text` first holds anything the lossless encoding escapes.
pub(super) fn first_escape(text: &str) -> Option<usize> {
    text.find(ESCAPE)
}

/// The text as a JavaScript string becomes when it crosses into Rust as a
/// `String`: each lone surrogate replaced with U+FFFD.
pub(super) fn lossy(text: &str) -> String {
    chars(text)
        .map(|ch| match ch {
            TextChar::Scalar(c) => c,
            TextChar::Surrogate(_) => char::REPLACEMENT_CHARACTER,
        })
        .collect()
}

fn push_scalar(out: &mut String, ch: char) {
    out.push(ch);
    if ch == ESCAPE {
        out.push(ch);
    }
}

enum TextChar {
    Scalar(char),
    Surrogate(u16),
}

fn chars(text: &str) -> impl Iterator<Item = TextChar> + '_ {
    let mut chars = text.chars();
    std::iter::from_fn(move || {
        let ch = chars.next()?;
        if ch == ESCAPE
            && let Some(next) = chars.next()
            && let Some(offset) = (next as u32).checked_sub(SURROGATE_BASE)
            && offset < 0x800
        {
            return Some(TextChar::Surrogate(0xd800 + offset as u16));
        }
        Some(TextChar::Scalar(ch))
    })
}

fn units(text: &str) -> impl Iterator<Item = u16> + '_ {
    chars(text).flat_map(|ch| {
        let mut units = [0; 2];
        let len = match ch {
            TextChar::Scalar(ch) => ch.encode_utf16(&mut units).len(),
            TextChar::Surrogate(unit) => {
                units = [unit, 0];
                1
            }
        };
        units.into_iter().take(len)
    })
}

// The JSON reader handles the containing grammar and depth. At a string
// token, decode escapes to units before joining pairs that can straddle a
// literal lone unit and a JSON escape.
fn decode_json_string(literal: &str) -> Option<String> {
    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    if !body.contains(['\\', '"']) && !body.bytes().any(|byte| byte < 0x20) {
        return Some(body.to_owned());
    }
    let mut input = units(body);
    let mut decoded = Vec::with_capacity(body.len());
    while let Some(unit) = input.next() {
        decoded.push(match unit {
            0..=0x1f | 0x22 => return None,
            0x5c => match input.next()? {
                unit @ (0x22 | 0x2f | 0x5c) => unit,
                0x62 => 8,
                0x66 => 12,
                0x6e => 10,
                0x72 => 13,
                0x74 => 9,
                0x75 => {
                    let mut value = 0;
                    for _ in 0..4 {
                        let digit = char::from_u32(u32::from(input.next()?))?.to_digit(16)?;
                        value = value * 16 + digit as u16;
                    }
                    value
                }
                _ => return None,
            },
            unit => unit,
        });
    }
    Some(encode_utf16(decoded.into_iter()))
}

// decodeURIComponent decodes percent bytes, leaves literal UTF-16 alone,
// and falls back to the original text on any malformed percent escape.
fn decode_attribute(text: &str) -> Cow<'_, str> {
    if !text.contains(['%', '+']) {
        return Cow::Borrowed(text);
    }
    decode_percent(text).map_or(Cow::Borrowed(text), Cow::Owned)
}

fn decode_percent(mut text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut bytes = Vec::new();
    while let Some(at) = text.find(['%', '+']) {
        let (before, tail) = text.split_at_checked(at)?;
        out.push_str(before);
        if let Some(after) = tail.strip_prefix('+') {
            out.push(' ');
            text = after;
            continue;
        }
        text = tail;
        while let Some(rest) = text.strip_prefix('%') {
            let (hex, after) = rest.split_at_checked(2)?;
            if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            text = after;
        }
        out.push_str(&encode(std::str::from_utf8(&bytes).ok()?));
        bytes.clear();
    }
    out.push_str(text);
    Some(out)
}
