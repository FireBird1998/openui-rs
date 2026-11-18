//! The response bundle reader from the upstream renderer (react-lang).

mod text;

use std::{borrow::Cow, collections::HashSet, fmt};

use crate::js::{normalize_surrogate_pairs, write_json_char};
use crate::message::split_message;
use crate::{Limits, Value};

use text::TextFormat;

const CONTENT: &str = "]]>openui:content";
const SCRIPTS: &str = "]]>openui:scripts";
const LINE_TERMINATORS: [char; 4] = ['\n', '\r', '\u{2028}', '\u{2029}'];
const INVALID_SCRIPTS: &str = "Invalid scripts bundle";

/// A stored or streaming response as read by the renderer.
///
/// Text fields use the core's [UTF-16 string convention](crate::utf16_units),
/// shared with [`ParsedMessage`](crate::ParsedMessage).
#[derive(PartialEq, Eq)]
pub struct ResponseBundle {
    /// The winning program with its scripts section removed.
    pub program: String,
    /// The optional name from the winning content marker.
    pub name: Option<String>,
    /// Cloud script names in scripts section order, each appearing once.
    pub script_names: Vec<String>,
    /// The renderer's `isBundle` flag. True when it sees a content marker line
    /// in the original response, recognizes a scripts section, or reads an end
    /// marker line at or after the winning content. This includes messages
    /// without a scripts section. A streaming content marker line counts
    /// once its newline arrives.
    pub is_bundle: bool,
    /// Whether the renderer currently considers the response complete.
    /// While streaming, this can change in either direction. The renderer
    /// blocks when streaming, incomplete, or reporting a scripts error.
    /// A response is settled when all three conditions are false.
    pub complete: bool,
    /// The renderer's error for an invalid section after streaming ends.
    pub error: Option<&'static str>,
    script_revision: Option<String>,
}

impl fmt::Debug for ResponseBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseBundle")
            .field("program", &self.program)
            .field("name", &self.name)
            .field("script_names", &self.script_names)
            .field("is_bundle", &self.is_bundle)
            .field("complete", &self.complete)
            .field("error", &self.error)
            .field(
                "script_revision",
                &self.script_revision.as_ref().map(|_| ".."),
            )
            .finish()
    }
}

impl ResponseBundle {
    /// The opaque revision of a nonempty, valid scripts section. Compare it
    /// for equality. It identifies the set of Cloud script names and code.
    pub fn script_revision(&self) -> Option<&str> {
        self.script_revision.as_deref()
    }

    /// Serialize the renderer's result as JSON. The scripts set is represented
    /// as an array in insertion order. The optional revision is opaque.
    pub fn to_json(&self) -> String {
        self.serialize(TextFormat::Core)
    }

    fn serialize(&self, format: TextFormat) -> String {
        let mut out = String::from("{\"program\":");
        format.write_json(&mut out, &self.program);
        out.push_str(",\"metadata\":{");
        if let Some(name) = &self.name {
            out.push_str("\"name\":");
            format.write_json(&mut out, name);
        }
        out.push_str("},\"scripts\":[");
        for (index, name) in self.script_names.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            format.write_json(&mut out, name);
        }
        out.push_str(if self.complete {
            "],\"complete\":true"
        } else {
            "],\"complete\":false"
        });
        out.push_str(if self.is_bundle {
            ",\"isBundle\":true"
        } else {
            ",\"isBundle\":false"
        });
        if let Some(error) = self.error {
            out.push_str(",\"error\":\"");
            for ch in error.chars() {
                write_json_char::<true>(&mut out, ch);
            }
            out.push('"');
        }
        if let Some(revision) = &self.script_revision {
            out.push_str(",\"scriptRevision\":\"");
            for ch in revision.chars() {
                write_json_char::<true>(&mut out, ch);
            }
            out.push('"');
        }
        out.push('}');
        out
    }
}

/// A response bundle reader limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseBundleError {
    /// The original response exceeds the program-size limit.
    ProgramSize,
    /// The winning message context exceeds the JSON depth limit.
    ContextDepth,
    /// JSON on the scripts marker line or in the scripts section exceeds the depth limit.
    ScriptsDepth,
}

impl fmt::Display for ResponseBundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ProgramSize => "response bundle exceeds the program-size limit",
            Self::ContextDepth => "response bundle context JSON exceeds the depth limit",
            Self::ScriptsDepth => "response bundle scripts JSON exceeds the depth limit",
        })
    }
}

impl std::error::Error for ResponseBundleError {}

/// Read a response or an absent response with the renderer's streaming rules.
/// The whole original response counts against the program-size limit. The
/// winning context and scripts section are read under the existing JSON depth
/// limit. Cloud script code is used only to validate and form the revision.
/// Input and returned strings use the core's [UTF-16 convention](crate::utf16_units)
/// shared with [`parse_message`](crate::parse_message).
pub fn parse_response_bundle(
    response: Option<&str>,
    streaming: bool,
    limits: Limits,
) -> Result<ResponseBundle, ResponseBundleError> {
    let raw = response.unwrap_or_default();
    if raw.len() > limits.program_size() {
        return Err(ResponseBundleError::ProgramSize);
    }
    read_bundle(
        &normalize_surrogate_pairs(raw),
        streaming,
        limits,
        TextFormat::Core,
    )
}

/// Serialize a JavaScript response directly from its exact UTF-16 units.
/// Size is charged as UTF-8 bytes with three bytes per lone surrogate. An
/// empty slice has the same meaning as an absent response.
pub fn response_bundle_json_utf16(
    response: &[u16],
    streaming: bool,
    limits: Limits,
) -> Result<String, ResponseBundleError> {
    let mut room = limits.program_size();
    if response.len() > room {
        return Err(ResponseBundleError::ProgramSize);
    }
    for scalar in char::decode_utf16(response.iter().copied()) {
        let bytes = scalar.map_or(3, char::len_utf8);
        room = room
            .checked_sub(bytes)
            .ok_or(ResponseBundleError::ProgramSize)?;
    }
    read_bundle(
        &text::encode_utf16(response.iter().copied()),
        streaming,
        limits,
        TextFormat::ExactUtf16,
    )
    .map(|bundle| bundle.serialize(TextFormat::ExactUtf16))
}

fn read_bundle(
    raw: &str,
    streaming: bool,
    limits: Limits,
    format: TextFormat,
) -> Result<ResponseBundle, ResponseBundleError> {
    let content_line = has_content_line(raw, streaming);
    let masked = mask_bare_scripts(raw);
    split_message(&masked, streaming, |program, context, query, end| {
        if let Some(context) = context
            && let Err(error) = format.read_json(&context, limits)
            && error.depth_limited()
        {
            return Err(ResponseBundleError::ContextDepth);
        }
        let (program, section, scripts) = extract_scripts(program, limits, format)?;
        let valid = !section || scripts.is_some();
        let (script_names, script_revision) = scripts.unwrap_or_default();
        Ok(ResponseBundle {
            program,
            name: if content_line {
                read_name(query, format)
            } else {
                None
            },
            script_names,
            script_revision,
            is_bundle: content_line || section || end,
            complete: valid && (end || !streaming || !content_line),
            error: (!streaming && !valid).then_some(INVALID_SCRIPTS),
        })
    })
}

// The renderer looks for the first content marker line before the message
// reader inserts newlines for retries. JavaScript multiline anchors also recognize CR,
// U+2028 and U+2029, while the message reader splits only on LF.
fn has_content_line(raw: &str, streaming: bool) -> bool {
    for (at, _) in raw.match_indices(CONTENT) {
        if !line_start(raw, at) {
            continue;
        }
        let Some(tail) = raw.get(at + CONTENT.len()..) else {
            continue;
        };
        let end = tail.find(LINE_TERMINATORS).unwrap_or(tail.len());
        if end == 0 || tail.starts_with('?') {
            return !streaming
                || tail
                    .get(end..)
                    .is_some_and(|rest| rest.starts_with(['\n']) || rest.starts_with("\r\n"));
        }
    }
    false
}

fn line_start(raw: &str, at: usize) -> bool {
    at == 0
        || raw
            .get(..at)
            .is_some_and(|prefix| prefix.ends_with(LINE_TERMINATORS))
}

fn mask_bare_scripts(raw: &str) -> Cow<'_, str> {
    let mut masked = String::new();
    let mut start = 0;
    for (at, _) in raw.match_indices(SCRIPTS) {
        let after = at + SCRIPTS.len();
        if line_start(raw, at)
            && raw.get(after..).is_some_and(|tail| {
                tail.is_empty() || tail.starts_with('?') || tail.starts_with(LINE_TERMINATORS)
            })
            && let Some(part) = raw.get(start..after)
        {
            masked.push_str(part);
            masked.push(' ');
            start = after;
        }
    }
    if start == 0 {
        return Cow::Borrowed(raw);
    }
    if let Some(tail) = raw.get(start..) {
        masked.push_str(tail);
    }
    Cow::Owned(masked)
}

fn read_name(query: &str, format: TextFormat) -> Option<String> {
    query.rsplit('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (format.attribute(key) == "name").then(|| format.attribute(value).into_owned())
    })
}

// The renderer stops at the first unquoted marker prefix, including an
// unknown prefix or an inline one. Comments run only to LF.
fn marker_index(text: &str) -> Option<usize> {
    if !text.contains("]]>") {
        return None;
    }
    let mut chars = text.char_indices();
    let mut quote = None;
    while let Some((at, ch)) = chars.next() {
        if let Some(delimiter) = quote {
            if ch == '\\' {
                chars.next();
            } else if ch == delimiter {
                quote = None;
            }
        } else if ch == '#' || (ch == '/' && chars.as_str().starts_with('/')) {
            chars.find(|&(_, ch)| ch == '\n')?;
        } else if matches!(ch, '"' | '\'') {
            quote = Some(ch);
        } else if ch == ']' && chars.as_str().starts_with("]>") {
            return Some(at);
        }
    }
    None
}

type Scripts = (Vec<String>, Option<String>);

fn extract_scripts(
    mut program: String,
    limits: Limits,
    format: TextFormat,
) -> Result<(String, bool, Option<Scripts>), ResponseBundleError> {
    let Some(at) = marker_index(&program) else {
        return Ok((program, false, None));
    };
    let Some((before, section)) = program.split_at_checked(at) else {
        return Ok((program, false, None));
    };
    let Some(section) = section.strip_prefix(SCRIPTS) else {
        return Ok((program, false, None));
    };
    let (scripts, after) = if let Some((count_json, tail)) = section.split_once('\n') {
        let end = marker_index(tail).unwrap_or(tail.len());
        if let Some((scripts_json, after)) = tail.split_at_checked(end) {
            (
                read_scripts(count_json, scripts_json, limits, format)?,
                after,
            )
        } else {
            (None, "")
        }
    } else {
        (None, "")
    };

    let before = before
        .strip_suffix('\n')
        .map_or(before, |line| line.strip_suffix('\r').unwrap_or(line));
    if after.is_empty() {
        program.truncate(before.len());
    } else {
        program = [before, "\n", after].concat();
    }
    Ok((program, true, scripts))
}

fn script_json(
    text: &str,
    limits: Limits,
    format: TextFormat,
) -> Result<Option<Value>, ResponseBundleError> {
    match format.read_json(text, limits) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.depth_limited() => Err(ResponseBundleError::ScriptsDepth),
        Err(_) => Ok(None),
    }
}

fn read_scripts(
    count_json: &str,
    scripts_json: &str,
    limits: Limits,
    format: TextFormat,
) -> Result<Option<Scripts>, ResponseBundleError> {
    let Some(count) = script_json(count_json, limits, format)? else {
        return Ok(None);
    };
    if matches!(count, Value::Null) {
        return Ok(None);
    }
    let Some(Value::Arr(ref values)) = script_json(scripts_json, limits, format)? else {
        return Ok(None);
    };
    let Value::Obj(fields) = &count else {
        return Ok(None);
    };
    let Some(Value::Num(count)) = fields.get("count") else {
        return Ok(None);
    };
    if *count != values.len() as f64 || *count > 9_007_199_254_740_991.0 {
        return Ok(None);
    }
    let mut names = HashSet::with_capacity(values.len());
    let mut scripts = Vec::with_capacity(values.len());
    for script in values.iter() {
        let Value::Obj(script) = script else {
            return Ok(None);
        };
        let (Some(Value::Str(name)), Some(Value::Str(code))) =
            (script.get("name"), script.get("code"))
        else {
            return Ok(None);
        };
        if name.is_empty() || !names.insert(name.as_str()) {
            return Ok(None);
        }
        scripts.push((name.as_str(), code.as_str()));
    }
    let names = scripts.iter().map(|(name, _)| (*name).to_owned()).collect();
    let revision = (!scripts.is_empty()).then(|| script_revision(&mut scripts, format));
    Ok(Some((names, revision)))
}

fn script_revision(scripts: &mut [(&str, &str)], format: TextFormat) -> String {
    scripts.sort_unstable_by(|(a, _), (b, _)| format.compare(a, b));
    let mut revision = String::from("[");
    for (index, (name, code)) in scripts.iter().enumerate() {
        if index != 0 {
            revision.push(',');
        }
        revision.push_str("{\"name\":");
        format.write_json(&mut revision, name);
        revision.push_str(",\"code\":");
        format.write_json(&mut revision, code);
        revision.push('}');
    }
    revision.push(']');
    revision
}
