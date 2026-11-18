//! The lang-core message format.

use std::{borrow::Cow, fmt};

use crate::js::{
    json_char_len, lone_surrogate_unit, normalize_surrogate_pairs, read_message_json,
    write_json_number, write_json_string,
};
use crate::{JsObject, Limits, Value};

const PREFIX: &str = "]]>openui:";
const CONTENT: &str = "]]>openui:content";
const CONTEXT: &str = "\n]]>openui:context\n";
const END: &str = "\n]]>openui:end";

/// A message read from stored or streaming text.
#[derive(Debug, PartialEq)]
pub struct ParsedMessage {
    /// Body of the winning content section, or the text before the first marker
    /// when there is no content marker.
    pub content: String,
    /// Parsed JSON, verbatim text if it is invalid JSON, or null if absent.
    /// Strings use the core's [UTF-16 representation](crate::utf16_units).
    pub context: Value,
    /// Own attributes of the winning content marker in JavaScript property order.
    pub attributes: JsObject<String>,
    /// Whether an end marker occurs at or after the winning content section.
    pub end: bool,
}

impl ParsedMessage {
    /// The reference's JSON.stringify representation of the parsed message.
    /// JavaScript infinity serializes as null and negative zero as zero.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"content\":");
        write_json_string(&mut out, &self.content);
        out.push_str(",\"context\":");
        crate::result::write_value(&mut out, &self.context);
        out.push_str(",\"attributes\":{");
        for (i, (key, value)) in self.attributes.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            write_json_string(&mut out, key);
            out.push(':');
            write_json_string(&mut out, value);
        }
        out.push_str(if self.end {
            "},\"end\":true}"
        } else {
            "},\"end\":false}"
        });
        out
    }
}

/// Parts to write into a stored message. Uses the core's
/// [UTF-16 representation](crate::utf16_units) for strings.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildMessageInput<'a> {
    /// Text written after the content marker.
    pub content: &'a str,
    /// A string written verbatim or a value serialized as JavaScript JSON.
    /// Omitted for null or no value.
    pub context: Option<&'a Value>,
    /// Own string attributes, written in JavaScript property order.
    pub attributes: Option<&'a JsObject<String>>,
    /// Whether to append an end marker.
    pub end: bool,
}

/// A message limit or input error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageError {
    /// The input text or written message exceeds the program-size limit.
    ProgramSize,
    /// JSON exceeds the nesting limit described by [Limits::depth].
    ContextDepth,
    /// An attribute name or value contains a lone surrogate or its reserved code point.
    InvalidAttribute,
    /// The parts are malformed, or their context contains a non-JSON core value.
    InvalidInput,
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ProgramSize => "message exceeds the program-size limit",
            Self::ContextDepth => "message JSON exceeds the depth limit",
            Self::InvalidAttribute => {
                "cannot URI-encode message attribute containing a lone surrogate or reserved scalar"
            }
            Self::InvalidInput => "invalid message parts",
        })
    }
}

impl std::error::Error for MessageError {}

/// Read a stored message, holding back a potential last marker line when
/// streaming is true. Refuses oversized text before scanning and checks the
/// JSON depth of the winning context. Uses the core's
/// [UTF-16 representation](crate::utf16_units) for strings.
pub fn parse_message(
    raw: &str,
    streaming: bool,
    limits: Limits,
) -> Result<ParsedMessage, MessageError> {
    admit_text(raw, limits)?;
    let normalized = normalize_surrogate_pairs(raw);
    split_message(&normalized, streaming, |content, context, query, end| {
        finish(content, context, query, end, limits)
    })
}

pub(crate) fn split_message<T>(
    raw: &str,
    streaming: bool,
    finish: impl FnOnce(String, Option<String>, &str, bool) -> T,
) -> T {
    if !raw.contains("]]>") {
        let (before, last) = raw.rsplit_once('\n').unwrap_or(("", raw));
        let content = if streaming && !last.is_empty() && PREFIX.starts_with(last) {
            strip_cr(before)
        } else {
            raw
        };
        return finish(content.to_owned(), None, "", false);
    }

    let retries = split_retries(raw);
    let text = if streaming {
        hold_retry(&retries)
    } else {
        &retries
    };
    let (before, last) = text
        .rsplit_once('\n')
        .map_or((None, text), |(before, last)| (Some(before), last));
    let held =
        streaming && !last.is_empty() && (last.starts_with(PREFIX) || PREFIX.starts_with(last));
    let visible = if held { before } else { Some(text) };
    let mut lines = visible.into_iter().flat_map(|text| text.split('\n'));
    let mut part = Part::Content;
    let mut content = Body::default();
    let mut context = Body::default();
    let mut has_context = false;
    let mut query = "";
    let mut end = false;
    while let Some(line) = lines.next() {
        let body = match part {
            Part::Content => Some(&mut content),
            Part::Context => Some(&mut context),
            Part::Other => None,
        };
        let Some((kind, attributes)) = marker(line) else {
            if let Some(body) = body {
                body.push(line);
            }
            continue;
        };
        if let Some(body) = body {
            body.close();
        }
        part = match kind {
            "content" => {
                content = Body::default();
                context = Body::default();
                has_context = false;
                query = attributes;
                end = false;
                Part::Content
            }
            "context" => {
                context = Body::default();
                has_context = true;
                Part::Context
            }
            "end" => {
                end = true;
                if lines.clone().eq([""]) {
                    break;
                }
                part
            }
            _ => Part::Other,
        };
    }
    if held {
        match part {
            Part::Content => content.close(),
            Part::Context => context.close(),
            Part::Other => {}
        }
    }
    finish(
        content.text,
        has_context.then_some(context.text),
        query,
        end,
    )
}

fn admit_text(text: &str, limits: Limits) -> Result<(), MessageError> {
    if text.len() > limits.program_size() {
        return Err(MessageError::ProgramSize);
    }
    Ok(())
}

fn finish(
    content: String,
    context: Option<String>,
    query: &str,
    end: bool,
    limits: Limits,
) -> Result<ParsedMessage, MessageError> {
    let context = match context {
        None => Value::Null,
        Some(text) => match read_message_json(&text, limits) {
            Ok(value) => value,
            Err(err) if err.depth_limited() => return Err(MessageError::ContextDepth),
            Err(_) => Value::Str(text),
        },
    };
    Ok(ParsedMessage {
        content,
        context,
        attributes: read_attributes(query),
        end,
    })
}

enum Part {
    Content,
    Context,
    Other,
}

#[derive(Default)]
struct Body {
    text: String,
    has_line: bool,
}

impl Body {
    fn push(&mut self, line: &str) {
        if self.has_line {
            self.text.push('\n');
        }
        self.text.push_str(line);
        self.has_line = true;
    }

    fn close(&mut self) {
        if self.text.ends_with('\r') {
            self.text.pop();
        }
    }
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

// A retry can start mid-line. The lookahead accepts '?' even if the rest of
// that marker line later fails to match.
fn split_retries(raw: &str) -> Cow<'_, str> {
    let mut text = String::new();
    let mut copied = 0;
    for (at, _) in raw.match_indices(CONTENT) {
        if at == 0 || raw.as_bytes().get(at.saturating_sub(1)) == Some(&b'\n') {
            continue;
        }
        let after = at.saturating_add(CONTENT.len());
        let Some(tail) = raw.get(after..) else {
            continue;
        };
        if (tail.is_empty() || tail.starts_with(['?', '\n']) || tail.starts_with("\r\n"))
            && let Some(prefix) = raw.get(copied..at)
        {
            text.push_str(prefix);
            text.push('\n');
            copied = at;
        }
    }
    if copied != 0
        && let Some(tail) = raw.get(copied..)
    {
        text.push_str(tail);
        return Cow::Owned(text);
    }
    Cow::Borrowed(raw)
}

fn hold_retry(text: &str) -> &str {
    if let Some(at) = text.rfind("]]>")
        && at > 0
        && text.as_bytes().get(at.saturating_sub(1)) != Some(&b'\n')
        && let Some((before, suffix)) = text.split_at_checked(at)
        && CONTENT.starts_with(suffix)
    {
        return before;
    }
    text
}

fn marker(line: &str) -> Option<(&str, &str)> {
    let body = strip_cr(line).strip_prefix(PREFIX)?;
    let (kind, query) = body.split_once('?').unwrap_or((body, ""));
    (!kind.is_empty()
        && kind.bytes().all(|b| b.is_ascii_lowercase())
        && !query.contains(['\r', '\u{2028}', '\u{2029}']))
    .then_some((kind, query))
}

fn read_attributes(query: &str) -> JsObject<String> {
    let mut attributes = JsObject::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_attribute(key);
        if !key.is_empty() {
            attributes.set(key, decode_attribute(value));
        }
    }
    attributes
}

pub(crate) fn decode_attribute(text: &str) -> String {
    if !text.contains(['%', '+']) {
        return text.to_owned();
    }
    decode_escaped(text).unwrap_or_else(|| text.to_owned())
}

fn decode_escaped(text: &str) -> Option<String> {
    let mut bytes = text.bytes();
    let mut out = Vec::with_capacity(text.len());
    while let Some(byte) = bytes.next() {
        out.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = char::from(bytes.next()?).to_digit(16)?;
                let low = char::from(bytes.next()?).to_digit(16)?;
                (high * 16 + low) as u8
            }
            other => other,
        });
    }
    String::from_utf8(out).ok()
}

/// Write a message with lang-core's marker lines and attribute encoding.
/// Refuses an output above the program-size limit, or a context above the
/// existing JSON depth limit.
pub fn build_message(input: BuildMessageInput<'_>, limits: Limits) -> Result<String, MessageError> {
    admit_text(input.content, limits)?;
    let mut out = Output {
        text: String::new(),
        limit: limits.program_size(),
    };
    out.push(CONTENT)?;
    if let Some(attributes) = input.attributes {
        out.check(attributes.len())?;
        for (i, (key, value)) in attributes.iter().enumerate() {
            out.push(if i == 0 { "?" } else { "&" })?;
            out.attribute(key)?;
            out.push("=")?;
            out.attribute(value)?;
        }
    }
    out.push("\n")?;
    out.push(&normalize_surrogate_pairs(input.content))?;
    match input.context {
        None | Some(Value::Null) => {}
        Some(context) => {
            out.push(CONTEXT)?;
            if let Value::Str(text) = context {
                out.check(text.len())?;
                out.push(&normalize_surrogate_pairs(text))?;
            } else {
                out.json(context, limits.json_depth())?;
            }
        }
    }
    if input.end {
        out.push(END)?;
    }
    Ok(out.text)
}

/// Write from JSON text with the same fields as [BuildMessageInput].
/// The input JSON itself must fit the existing program-size and JSON depth
/// limits. Use this entry point when message parts cross a JSON interface.
pub fn build_message_json(input: &str, limits: Limits) -> Result<String, MessageError> {
    admit_text(input, limits)?;
    let value = read_message_json(input, limits).map_err(|err| {
        if err.depth_limited() {
            MessageError::ContextDepth
        } else {
            MessageError::InvalidInput
        }
    })?;
    let Value::Obj(fields) = &value else {
        return Err(MessageError::InvalidInput);
    };
    let Some(Value::Str(content)) = fields.get("content") else {
        return Err(MessageError::InvalidInput);
    };
    let attributes = match fields.get("attributes") {
        None | Some(Value::Null) => None,
        Some(Value::Obj(fields)) => {
            let mut attributes = JsObject::with_capacity(fields.len());
            for (key, value) in fields.iter() {
                let Value::Str(value) = value else {
                    return Err(MessageError::InvalidInput);
                };
                attributes.define(key.clone(), value.clone());
            }
            Some(attributes)
        }
        _ => return Err(MessageError::InvalidInput),
    };
    let end = match fields.get("end") {
        None => false,
        Some(Value::Bool(end)) => *end,
        _ => return Err(MessageError::InvalidInput),
    };
    build_message(
        BuildMessageInput {
            content,
            context: fields.get("context"),
            attributes: attributes.as_ref(),
            end,
        },
        limits,
    )
}

struct Output {
    text: String,
    limit: usize,
}

type ObjectEntries<'a> = dyn Iterator<Item = (&'a String, &'a Value)> + 'a;

enum Frame<'a> {
    Array(std::iter::Enumerate<std::slice::Iter<'a, Value>>),
    Object(std::iter::Enumerate<Box<ObjectEntries<'a>>>),
}

impl Output {
    fn check(&self, bytes: usize) -> Result<(), MessageError> {
        if bytes > self.limit.saturating_sub(self.text.len()) {
            return Err(MessageError::ProgramSize);
        }
        Ok(())
    }

    fn push(&mut self, text: &str) -> Result<(), MessageError> {
        self.check(text.len())?;
        self.text.push_str(text);
        Ok(())
    }

    fn attribute(&mut self, value: &str) -> Result<(), MessageError> {
        self.check(value.len())?;
        let value = normalize_surrogate_pairs(value);
        if value.chars().any(|ch| lone_surrogate_unit(ch).is_some()) {
            return Err(MessageError::InvalidAttribute);
        }
        for byte in value.bytes() {
            match byte {
                b' ' => self.push("+")?,
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    self.check(1)?;
                    self.text.push(char::from(byte));
                }
                _ => {
                    self.check(3)?;
                    use fmt::Write as _;
                    let _ = write!(self.text, "%{byte:02X}");
                }
            }
        }
        Ok(())
    }

    fn json_string(&mut self, text: &str) -> Result<(), MessageError> {
        self.check(text.len())?;
        let text = normalize_surrogate_pairs(text);
        let bytes = text
            .chars()
            .try_fold(2usize, |bytes, ch| bytes.checked_add(json_char_len(ch)));
        self.check(bytes.ok_or(MessageError::ProgramSize)?)?;
        write_json_string(&mut self.text, &text);
        Ok(())
    }

    // Resume each container after its current child. A caller can reuse
    // the same array at many positions, so charge output as it is written.
    fn json(&mut self, root: &Value, depth: usize) -> Result<(), MessageError> {
        let mut stack = Vec::new();
        let mut next = Some(root);
        loop {
            if let Some(value) = next.take() {
                match value {
                    Value::Null => self.push("null")?,
                    Value::Bool(v) => self.push(if *v { "true" } else { "false" })?,
                    Value::Num(v) => {
                        self.check(crate::json_number_len(*v))?;
                        write_json_number(&mut self.text, *v);
                    }
                    Value::Str(v) => self.json_string(v)?,
                    Value::Arr(_) | Value::Obj(_) if stack.len() >= depth => {
                        return Err(MessageError::ContextDepth);
                    }
                    Value::Arr(items) => {
                        self.push("[")?;
                        stack.push(Frame::Array(items.iter().enumerate()));
                    }
                    Value::Obj(fields) => {
                        self.push("{")?;
                        self.check(fields.len())?;
                        let entries: Box<ObjectEntries<'_>> = Box::new(fields.iter());
                        stack.push(Frame::Object(entries.enumerate()));
                    }
                    Value::Element(_) | Value::Ast(_) => return Err(MessageError::InvalidInput),
                }
            }
            let close = match stack.last_mut() {
                None => return Ok(()),
                Some(Frame::Array(items)) => {
                    if let Some((i, item)) = items.next() {
                        if i != 0 {
                            self.push(",")?;
                        }
                        next = Some(item);
                        continue;
                    }
                    "]"
                }
                Some(Frame::Object(fields)) => {
                    if let Some((i, (key, value))) = fields.next() {
                        if i != 0 {
                            self.push(",")?;
                        }
                        self.json_string(key)?;
                        self.push(":")?;
                        next = Some(value);
                        continue;
                    }
                    "}"
                }
            };
            self.push(close)?;
            stack.pop();
        }
    }
}
