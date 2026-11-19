//! Where a tool outcome's JSON ends and its result begins (#90).
//!
//! A result too large or too deep for the session's limits is the tool's,
//! and is delivered as the call's `tool-error`. Outcome JSON that is
//! malformed or has the wrong shape is the embedder's, and is refused with
//! nothing consumed. Text longer than the outcome bound is never parsed: one
//! pass checks its syntax and finds its shape without copying anything, with
//! memory bounded by the depth limit rather than by the text.
use super::*;
use openui_runtime::{ResultLimit, ToolCallId};

/// `{"value":}`, the envelope around a result.
const ENVELOPE: usize = 10;

/// What the outcome JSON holds, as far as the scan read it.
#[derive(Debug, PartialEq, Eq)]
enum Scan {
    /// Not JSON.
    Malformed,
    /// Valid JSON up to an array or object nested past the bound. `in_value`
    /// when that is inside a member of the outermost object and every member
    /// so far is named `value`.
    TooDeep { in_value: bool },
    /// Valid JSON throughout.
    Complete(Shape),
}

#[derive(Debug, PartialEq, Eq)]
enum Shape {
    /// An object whose members are all named `value`: a result.
    Value,
    /// An object whose members are all named `mcpResponseJson`, the last a
    /// string: an MCP response.
    McpResponse,
    Other,
}

impl SessionCore {
    /// The bytes the outcome JSON may have, and the size it names when a
    /// result's JSON is longer: the smallest of the host input bound, the
    /// result-size limit and the work limit, plus the envelope, so that a
    /// result at the limit fits when written compactly.
    fn outcome_bound(&self) -> (usize, usize) {
        let limits = self.core_limits;
        let size = usize::try_from(self.limits.max_json_bytes)
            .unwrap_or(usize::MAX)
            .min(limits.result_size())
            .min(limits.work_for(usize::MAX));
        (size, size.saturating_add(ENVELOPE))
    }

    /// Deliver a result refused under a limit as the call's `tool-error`.
    fn deliver_refused(
        &mut self,
        call: &ToolCallId,
        limit: ResultLimit,
    ) -> Result<(), OpenUiError> {
        self.complete_tool_call(call, ToolCallOutcome::CallFailed(limit.to_string()))
    }

    /// For outcome JSON longer than the bound: deliver a result envelope as a
    /// size (or depth) refusal, or refuse the outcome as before.
    pub(super) fn complete_long_outcome(
        &mut self,
        call: &ToolCallId,
        json: &str,
    ) -> Result<(), OpenUiError> {
        let (size, _) = self.outcome_bound();
        let depth = self.core_limits.depth();
        match scan(json, depth.saturating_add(1)) {
            Scan::Complete(Shape::Value | Shape::McpResponse) => {
                self.deliver_refused(call, ResultLimit::Size(size))
            }
            Scan::TooDeep { in_value: true } => {
                self.deliver_refused(call, ResultLimit::Depth(depth))
            }
            _ => Err(limit("tool outcome byte limit")),
        }
    }

    /// For outcome JSON the reader refused as too deep: deliver a too-deep
    /// result, or refuse the outcome as before.
    pub(super) fn complete_deep_outcome(
        &mut self,
        call: &ToolCallId,
        json: &str,
    ) -> Result<(), OpenUiError> {
        let depth = self.core_limits.depth();
        match scan(json, depth.saturating_add(1)) {
            Scan::TooDeep { in_value: true } => {
                self.deliver_refused(call, ResultLimit::Depth(depth))
            }
            _ => Err(limit("tool outcome depth limit")),
        }
    }

    /// An MCP response refused under the limits is the tool's result too.
    pub(super) fn complete_refused_mcp(
        &mut self,
        call: &ToolCallId,
        error: &openui_runtime::LimitError,
    ) -> Result<(), OpenUiError> {
        let limits = self.core_limits;
        let refused = if error.flags.depth_limited {
            ResultLimit::Depth(limits.depth())
        } else {
            ResultLimit::Size(limits.result_size().min(limits.work_for(usize::MAX)))
        };
        self.deliver_refused(call, refused)
    }

    /// The outcome JSON's byte bound.
    pub(super) fn outcome_byte_bound(&self) -> usize {
        self.outcome_bound().1
    }
}

/// One pass over JSON text with `JSON.parse`'s grammar, as the session's
/// reader applies it, refusing an array or object opened at `max_depth` open
/// containers. Allocates only the open containers' kinds.
fn scan(text: &str, max_depth: usize) -> Scan {
    let s = text.as_bytes();
    let mut i = 0usize;
    // The kinds of the open containers: true for an object.
    let mut open: Vec<bool> = Vec::new();
    let mut members = 0usize;
    let (mut all_value, mut all_mcp, mut last_is_string) = (true, true, false);
    let mut top_object = false;
    loop {
        i = skip_ws(s, i);
        if open.len() == 1 && top_object {
            last_is_string = s.get(i) == Some(&b'"');
        }
        match s.get(i) {
            Some(b'[' | b'{') if open.len() >= max_depth => {
                return Scan::TooDeep {
                    in_value: top_object && all_value,
                };
            }
            Some(b'[') => {
                i = skip_ws(s, i + 1);
                if s.get(i) == Some(&b']') {
                    i += 1;
                } else {
                    open.push(false);
                    continue;
                }
            }
            Some(b'{') => {
                if open.is_empty() {
                    top_object = true;
                }
                i = skip_ws(s, i + 1);
                if s.get(i) == Some(&b'}') {
                    i += 1;
                } else {
                    open.push(true);
                    let Some((next, value, mcp)) = key(s, i) else {
                        return Scan::Malformed;
                    };
                    i = next;
                    if open.len() == 1 {
                        members += 1;
                        all_value &= value;
                        all_mcp &= mcp;
                    }
                    continue;
                }
            }
            Some(b'"') => match string(s, i) {
                Some((next, _, _)) => i = next,
                None => return Scan::Malformed,
            },
            Some(b'-' | b'0'..=b'9') => match number(s, i) {
                Some(next) => i = next,
                None => return Scan::Malformed,
            },
            Some(_) => match [&b"true"[..], b"false", b"null"]
                .iter()
                .find(|word| s.get(i..).is_some_and(|rest| rest.starts_with(word)))
            {
                Some(word) => i += word.len(),
                None => return Scan::Malformed,
            },
            None => return Scan::Malformed,
        }
        // A value is complete: continue or close what holds it.
        loop {
            i = skip_ws(s, i);
            match open.last() {
                None => {
                    if i != s.len() {
                        return Scan::Malformed;
                    }
                    let shape = if !top_object || members == 0 {
                        Shape::Other
                    } else if all_value {
                        Shape::Value
                    } else if all_mcp && last_is_string {
                        Shape::McpResponse
                    } else {
                        Shape::Other
                    };
                    return Scan::Complete(shape);
                }
                Some(object) => {
                    let (object, close) = (*object, if *object { b'}' } else { b']' });
                    match s.get(i) {
                        Some(b',') if object => {
                            let Some((next, value, mcp)) = key(s, skip_ws(s, i + 1)) else {
                                return Scan::Malformed;
                            };
                            i = next;
                            if open.len() == 1 {
                                members += 1;
                                all_value &= value;
                                all_mcp &= mcp;
                            }
                            break;
                        }
                        Some(b',') => {
                            i += 1;
                            break;
                        }
                        Some(byte) if *byte == close => {
                            i += 1;
                            open.pop();
                        }
                        _ => return Scan::Malformed,
                    }
                }
            }
        }
    }
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while matches!(s.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        i += 1;
    }
    i
}

/// `"key" :` at `i`; whether the key is `value` and whether `mcpResponseJson`.
fn key(s: &[u8], i: usize) -> Option<(usize, bool, bool)> {
    if s.get(i) != Some(&b'"') {
        return None;
    }
    let (next, value, mcp) = string(s, i)?;
    let next = skip_ws(s, next);
    (s.get(next) == Some(&b':')).then_some((next + 1, value, mcp))
}

/// A string at `i`: the index after it, and whether its decoded text is
/// `value` and whether `mcpResponseJson`.
fn string(s: &[u8], mut i: usize) -> Option<(usize, bool, bool)> {
    const TARGETS: [&[u8]; 2] = [b"value", b"mcpResponseJson"];
    let mut matched = [Some(0usize); 2];
    let mut push = |unit: Option<u8>| {
        for (state, target) in matched.iter_mut().zip(TARGETS) {
            *state = match (*state, unit) {
                (Some(at), Some(unit)) if target.get(at) == Some(&unit) => Some(at + 1),
                _ => None,
            };
        }
    };
    i += 1;
    loop {
        match *s.get(i)? {
            b'"' => break,
            b'\\' => {
                let unit = match *s.get(i + 1)? {
                    b'"' => b'"',
                    b'\\' => b'\\',
                    b'/' => b'/',
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => {
                        let hex = s.get(i + 2..i + 6)?;
                        if !hex.iter().all(u8::is_ascii_hexdigit) {
                            return None;
                        }
                        let code = hex
                            .iter()
                            .fold(0u32, |code, digit| code * 16 + hex_value(*digit));
                        i += 6;
                        push(u8::try_from(code).ok().filter(u8::is_ascii));
                        continue;
                    }
                    _ => return None,
                };
                push(Some(unit));
                i += 2;
            }
            0x00..=0x1f => return None,
            byte => {
                push(byte.is_ascii().then_some(byte));
                i += 1;
            }
        }
    }
    let done = |state: Option<usize>, target: &[u8]| state == Some(target.len());
    Some((
        i + 1,
        done(matched[0], TARGETS[0]),
        done(matched[1], TARGETS[1]),
    ))
}

fn hex_value(digit: u8) -> u32 {
    match digit {
        b'0'..=b'9' => u32::from(digit - b'0'),
        b'a'..=b'f' => u32::from(digit - b'a') + 10,
        _ => u32::from(digit.to_ascii_lowercase().wrapping_sub(b'a')) + 10,
    }
}

/// A number at `i` in JSON's grammar: the index after it.
fn number(s: &[u8], mut i: usize) -> Option<usize> {
    let digits = |i: usize| {
        let mut end = i;
        while s.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        end
    };
    if s.get(i) == Some(&b'-') {
        i += 1;
    }
    match s.get(i)? {
        b'0' => i += 1,
        b'1'..=b'9' => i = digits(i),
        _ => return None,
    }
    if s.get(i) == Some(&b'.') {
        let end = digits(i + 1);
        if end == i + 1 {
            return None;
        }
        i = end;
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(s.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let end = digits(i);
        if end == i {
            return None;
        }
        i = end;
    }
    Some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_reads_the_outcome_shapes_the_reader_accepts() {
        for (text, expected) in [
            (r#"{"value":1}"#, Scan::Complete(Shape::Value)),
            (
                r#" { "value" : [1, {"a": null}] } "#,
                Scan::Complete(Shape::Value),
            ),
            (r#"{"value":1,"value":"x"}"#, Scan::Complete(Shape::Value)),
            (
                r#"{"mcpResponseJson":"{}"}"#,
                Scan::Complete(Shape::McpResponse),
            ),
            (r#"{"mcpResponseJson":{}}"#, Scan::Complete(Shape::Other)),
            (r#"{"failed":"x"}"#, Scan::Complete(Shape::Other)),
            (r#"{"value":1,"failed":"x"}"#, Scan::Complete(Shape::Other)),
            (r#"{"values":1}"#, Scan::Complete(Shape::Other)),
            (r#"{}"#, Scan::Complete(Shape::Other)),
            (r#"[{"value":1}]"#, Scan::Complete(Shape::Other)),
            (r#"{"value":-0.5e+3}"#, Scan::Complete(Shape::Value)),
            (r#"{"value":"\ud800"}"#, Scan::Complete(Shape::Value)),
            (r#"{"value":1}x"#, Scan::Malformed),
            (r#"{"value":01}"#, Scan::Malformed),
            (r#"{"value":1.}"#, Scan::Malformed),
            (r#"{"value":"a"#, Scan::Malformed),
            (r#"{"value":"\x"}"#, Scan::Malformed),
            ("{\"value\":\"\u{1}\"}", Scan::Malformed),
            (r#"{"value":[1,]}"#, Scan::Malformed),
            (r#"{"value":[}"#, Scan::Malformed),
            (r#"{"value":tru}"#, Scan::Malformed),
            (r#"{"value":[[[1]]]}"#, Scan::TooDeep { in_value: true }),
            (r#"{"failed":[[[1]]]}"#, Scan::TooDeep { in_value: false }),
            (
                r#"{"x":1,"value":[[[1]]]}"#,
                Scan::TooDeep { in_value: false },
            ),
            (r#"[[[[1]]]]"#, Scan::TooDeep { in_value: false }),
            (r#"{"value":[[]]}"#, Scan::Complete(Shape::Value)),
        ] {
            assert_eq!(scan(text, 3), expected, "{text}");
            // The reader agrees on what is JSON and what is too deep.
            let read = openui_lang::parse_json_value(text, 3);
            match expected {
                Scan::Malformed => assert!(read.is_err_and(|e| !e.depth_limited()), "{text}"),
                Scan::TooDeep { .. } => assert!(read.is_err_and(|e| e.depth_limited()), "{text}"),
                Scan::Complete(_) => assert!(read.is_ok(), "{text}"),
            }
        }
    }
}
