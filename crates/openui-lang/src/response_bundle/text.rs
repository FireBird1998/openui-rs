//! Lossless UTF-16 in UTF-8 storage, so the message splitter can work on
//! ASCII syntax without conflating lone surrogates with real Unicode scalars.

use std::borrow::Cow;

use crate::js::{write_json_char, write_json_surrogate};

const ESCAPE: char = '\u{fdd0}';
const SURROGATE_BASE: u32 = 0xe000;

/// Text returned by the response bundle reader. Streaming prefixes, bundle
/// names and Cloud script names can carry JavaScript's lone UTF-16 surrogates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseText(pub(super) String);

impl ResponseText {
    /// Read the exact text as UTF-16 units.
    pub fn utf16_units(&self) -> impl Iterator<Item = u16> + '_ {
        units(&self.0)
    }

    /// Read as Rust text, replacing each lone surrogate with U+FFFD.
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        if !self.0.contains(ESCAPE) {
            return Cow::Borrowed(&self.0);
        }
        Cow::Owned(
            char::decode_utf16(self.utf16_units())
                .map(|ch| ch.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect(),
        )
    }

    pub(super) fn write_json(&self, out: &mut String) {
        write_json(out, &self.0);
    }
}

pub(super) fn write_json(out: &mut String, text: &str) {
    out.push('"');
    for ch in chars(text) {
        match ch {
            TextChar::Scalar(c) => write_json_char::<true>(out, c),
            TextChar::Surrogate(unit) => write_json_surrogate(out, unit),
        }
    }
    out.push('"');
}

pub(super) fn encode(text: &str) -> Cow<'_, str> {
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

pub(super) fn units(text: &str) -> impl Iterator<Item = u16> + '_ {
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
pub(super) fn decode_json_string(literal: &str) -> Option<String> {
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
pub(super) fn decode_attribute(text: &str) -> Cow<'_, str> {
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
