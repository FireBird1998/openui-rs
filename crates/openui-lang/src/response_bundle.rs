//! The response bundle reader from the upstream renderer (react-lang).

mod text;

use std::{borrow::Cow, collections::HashSet, fmt};

use crate::js::{read_bundle_json, write_json_char};
use crate::message::split_message;
use crate::{Limits, Value};

pub use text::ResponseText;

const CONTENT: &str = "]]>openui:content";
const SCRIPTS: &str = "]]>openui:scripts";
const LINE_TERMINATORS: [char; 4] = ['\n', '\r', '\u{2028}', '\u{2029}'];
const INVALID_SCRIPTS: &str = "Invalid scripts bundle";

/// A stored or streaming response as read by the renderer.
#[derive(Debug, PartialEq, Eq)]
pub struct ResponseBundle {
    /// The winning program with its scripts section removed.
    pub program: ResponseText,
    /// The optional name from the winning content marker.
    pub name: Option<ResponseText>,
    /// Cloud script names in payload order, each appearing once.
    pub script_names: Vec<ResponseText>,
    /// The opaque revision of a nonempty, valid scripts section.
    pub script_revision: Option<ScriptRevision>,
    /// Whether the renderer sees a content marker line, scripts section or end.
    pub is_bundle: bool,
    /// Whether the renderer considers the response complete.
    pub complete: bool,
    /// The renderer's error for an invalid section after streaming ends.
    pub error: Option<&'static str>,
}

/// An opaque scripts revision. Compare revisions for equality.
#[derive(Clone, PartialEq, Eq)]
pub struct ScriptRevision(String);

impl fmt::Debug for ScriptRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ScriptRevision(..)")
    }
}

impl ResponseBundle {
    /// Serialize the renderer's result as JSON. The scripts set is represented
    /// as an array in insertion order. The optional revision is opaque.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"program\":");
        self.program.write_json(&mut out);
        out.push_str(",\"metadata\":{");
        if let Some(name) = &self.name {
            out.push_str("\"name\":");
            name.write_json(&mut out);
        }
        out.push_str("},\"scripts\":[");
        for (index, name) in self.script_names.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            name.write_json(&mut out);
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
            for ch in revision.0.chars() {
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
    /// The scripts header or payload exceeds the JSON depth limit.
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
pub fn parse_response_bundle(
    response: Option<&str>,
    streaming: bool,
    limits: Limits,
) -> Result<ResponseBundle, ResponseBundleError> {
    let raw = response.unwrap_or_default();
    if raw.len() > limits.program_size() {
        return Err(ResponseBundleError::ProgramSize);
    }
    read_bundle(&text::encode(raw), streaming, limits)
}

/// Read a response from JavaScript UTF-16 units, including any lone surrogates.
/// Size is charged as UTF-8 bytes with three bytes per lone surrogate. An
/// empty slice has the same meaning as an absent response.
pub fn parse_response_bundle_utf16(
    response: &[u16],
    streaming: bool,
    limits: Limits,
) -> Result<ResponseBundle, ResponseBundleError> {
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
    )
}

fn read_bundle(
    raw: &str,
    streaming: bool,
    limits: Limits,
) -> Result<ResponseBundle, ResponseBundleError> {
    let framed = framed(raw, streaming);
    let masked = mask_bare_scripts(raw);
    split_message(&masked, streaming, |program, context, query, end| {
        if let Some(context) = context
            && let Err(error) = read_bundle_json(&context, limits, text::decode_json_string)
            && error.depth_limited()
        {
            return Err(ResponseBundleError::ContextDepth);
        }
        let (program, section, scripts) = extract_scripts(program, limits)?;
        let valid = !section || scripts.is_some();
        let (script_names, script_revision) = scripts.unwrap_or_default();
        Ok(ResponseBundle {
            program: ResponseText(program),
            name: if framed { read_name(query) } else { None },
            script_names,
            script_revision,
            is_bundle: framed || section || end,
            complete: valid && (end || !streaming || !framed),
            error: (!streaming && !valid).then_some(INVALID_SCRIPTS),
        })
    })
}

// The renderer looks for the first match before the message reader inserts
// newlines for retries. JavaScript multiline anchors also recognize CR,
// U+2028 and U+2029, while the message reader splits only on LF.
fn framed(raw: &str, streaming: bool) -> bool {
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

fn read_name(query: &str) -> Option<ResponseText> {
    query.rsplit('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (text::decode_attribute(key) == "name")
            .then(|| ResponseText(text::decode_attribute(value).into_owned()))
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

type Scripts = (Vec<ResponseText>, Option<ScriptRevision>);

fn extract_scripts(
    mut program: String,
    limits: Limits,
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
    let (scripts, after) = if let Some((header, tail)) = section.split_once('\n') {
        let end = marker_index(tail).unwrap_or(tail.len());
        if let Some((payload, after)) = tail.split_at_checked(end) {
            (read_scripts(header, payload, limits)?, after)
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

fn script_json(text: &str, limits: Limits) -> Result<Option<Value>, ResponseBundleError> {
    match read_bundle_json(text, limits, text::decode_json_string) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.depth_limited() => Err(ResponseBundleError::ScriptsDepth),
        Err(_) => Ok(None),
    }
}

fn read_scripts(
    header: &str,
    payload: &str,
    limits: Limits,
) -> Result<Option<Scripts>, ResponseBundleError> {
    let Some(header) = script_json(header, limits)? else {
        return Ok(None);
    };
    if matches!(header, Value::Null) {
        return Ok(None);
    }
    let Some(Value::Arr(ref values)) = script_json(payload, limits)? else {
        return Ok(None);
    };
    let Value::Obj(fields) = &header else {
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
    let names = scripts
        .iter()
        .map(|(name, _)| ResponseText((*name).to_owned()))
        .collect();
    let revision = (!scripts.is_empty()).then(|| script_revision(&mut scripts));
    Ok(Some((names, revision)))
}

fn script_revision(scripts: &mut [(&str, &str)]) -> ScriptRevision {
    scripts.sort_unstable_by(|(a, _), (b, _)| text::units(a).cmp(text::units(b)));
    let mut revision = String::from("[");
    for (index, (name, code)) in scripts.iter().enumerate() {
        if index != 0 {
            revision.push(',');
        }
        revision.push_str("{\"name\":");
        text::write_json(&mut revision, name);
        revision.push_str(",\"code\":");
        text::write_json(&mut revision, code);
        revision.push('}');
    }
    revision.push(']');
    ScriptRevision(revision)
}
