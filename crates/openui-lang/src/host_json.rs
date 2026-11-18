//! Stack-constant JSON projection for host data.

use super::{js_property_order, write_json_number, write_json_string};
use crate::{JsObject, Value, json_number_len, json_string_len};

/// Measure a host JSON value under a byte ceiling, optionally appending its
/// JSON to a string. Uses JavaScript's property order and number formatting.
///
/// Returns the byte count, or `None` if the limit is crossed or the value
/// contains OpenUI syntax or elements. A supplied string may receive partial
/// output on refusal. Traversal uses an explicit stack.
pub fn write_host_json(value: &Value, max_bytes: usize, out: Option<&mut String>) -> Option<usize> {
    let mut output = Output {
        out,
        room: max_bytes,
    };
    let mut frames = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(value) = next {
            match value {
                Value::Null => output.token("null")?,
                Value::Bool(value) => output.token(if *value { "true" } else { "false" })?,
                Value::Num(value) => output.number(*value)?,
                Value::Str(text) => output.string(text)?,
                Value::Arr(items) => {
                    output.token("[")?;
                    frames.push(Frame::Array(items.iter().enumerate()));
                }
                Value::Obj(fields) => {
                    output.token("{")?;
                    frames.push(Frame::object(fields, output.out.is_some()));
                }
                Value::Element(_) | Value::Ast(_) => return None,
            }
        }
        let Some(frame) = frames.last_mut() else {
            return Some(max_bytes - output.room);
        };
        next = if let Some(value) = frame.next(&mut output)? {
            Some(value)
        } else {
            output.token(frame.close())?;
            frames.pop();
            None
        };
    }
}

enum FieldOrder {
    Insertion(std::ops::Range<usize>),
    JavaScript(std::vec::IntoIter<usize>),
}

impl Iterator for FieldOrder {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        match self {
            Self::Insertion(indices) => indices.next(),
            Self::JavaScript(indices) => indices.next(),
        }
    }
}

enum Frame<'a> {
    Array(std::iter::Enumerate<std::slice::Iter<'a, Value>>),
    Object {
        fields: &'a JsObject<Value>,
        indices: std::iter::Enumerate<FieldOrder>,
    },
}

impl<'a> Frame<'a> {
    fn object(fields: &'a JsObject<Value>, writing: bool) -> Self {
        let order = if writing {
            FieldOrder::JavaScript(
                js_property_order(fields.entries.keys().map(String::as_str)).into_iter(),
            )
        } else {
            FieldOrder::Insertion(0..fields.len())
        };
        Self::Object {
            fields,
            indices: order.enumerate(),
        }
    }

    fn next(&mut self, output: &mut Output<'_>) -> Option<Option<&'a Value>> {
        match self {
            Self::Array(items) => {
                let Some((index, item)) = items.next() else {
                    return Some(None);
                };
                if index != 0 {
                    output.token(",")?;
                }
                Some(Some(item))
            }
            Self::Object { fields, indices } => {
                let Some((index, position)) = indices.next() else {
                    return Some(None);
                };
                let (key, value) = fields.entries.get_index(position)?;
                if index != 0 {
                    output.token(",")?;
                }
                output.string(key)?;
                output.token(":")?;
                Some(Some(value))
            }
        }
    }

    fn close(&self) -> &'static str {
        match self {
            Self::Array(_) => "]",
            Self::Object { .. } => "}",
        }
    }
}

struct Output<'a> {
    out: Option<&'a mut String>,
    room: usize,
}

impl Output<'_> {
    fn reserve(&mut self, bytes: usize) -> Option<()> {
        self.room = self.room.checked_sub(bytes)?;
        Some(())
    }

    fn token(&mut self, token: &str) -> Option<()> {
        self.reserve(token.len())?;
        if let Some(out) = &mut self.out {
            out.push_str(token);
        }
        Some(())
    }

    fn string(&mut self, text: &str) -> Option<()> {
        self.reserve(json_string_len(text).checked_add(2)?)?;
        if let Some(out) = &mut self.out {
            write_json_string(out, text);
        }
        Some(())
    }

    fn number(&mut self, number: f64) -> Option<()> {
        self.reserve(json_number_len(number))?;
        if let Some(out) = &mut self.out {
            write_json_number(out, number);
        }
        Some(())
    }
}
