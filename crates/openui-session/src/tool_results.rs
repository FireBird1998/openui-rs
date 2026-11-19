//! Where a tool outcome's JSON ends and its result begins (#90).
//!
//! An outcome is decided in one order on every path: first whether it is
//! valid (malformed or wrongly shaped JSON is the embedder's, refused with
//! nothing consumed), then the depth limit, then the result-size limit (the
//! tool's, delivered as the call's `tool-error`). Validity is decided before
//! any limit wherever memory bounded by the depth limit can decide it, which
//! is always for JSON text; only text nested past the depth limit before its
//! validity is known counts as too deep without the rest being read. Text
//! longer than the outcome bound is never parsed: one pass checks it, an MCP
//! response inside it included, without copying anything.
use super::*;
use openui_runtime::{ResultLimit, ToolCallId};

/// `{"value":}`, the envelope around a result.
const ENVELOPE: usize = 10;

/// What JSON text holds, as far as the scan read it.
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
    /// string starting at this offset: an MCP response.
    McpResponse(usize),
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

    /// Outcome JSON longer than the bound: malformed text is refused as
    /// such; a valid result or MCP response is too large (or too deep); any
    /// other valid outcome is refused as before.
    pub(super) fn complete_long_outcome(
        &mut self,
        call: &ToolCallId,
        json: &str,
    ) -> Result<(), OpenUiError> {
        let (size, _) = self.outcome_bound();
        let depth = self.core_limits.depth();
        let bytes = json.as_bytes();
        match scan(Slice::new(bytes, 0), depth.saturating_add(1)) {
            Scan::Malformed => Err(OpenUiError::InvalidJson {
                reason: "malformed tool outcome JSON".into(),
            }),
            Scan::Complete(Shape::Value) => self.deliver_refused(call, ResultLimit::Size(size)),
            Scan::TooDeep { in_value: true } => {
                self.deliver_refused(call, ResultLimit::Depth(depth))
            }
            // The response is read where it lies, its escapes decoded as it goes.
            Scan::Complete(Shape::McpResponse(at)) => {
                match scan(Unescape::new(bytes, at.saturating_add(1)), depth) {
                    Scan::Malformed => Err(OpenUiError::InvalidJson {
                        reason: "malformed MCP response JSON".into(),
                    }),
                    Scan::TooDeep { .. } => self.deliver_refused(call, ResultLimit::Depth(depth)),
                    Scan::Complete(_) => self.deliver_refused(call, ResultLimit::Size(size)),
                }
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
        match scan(Slice::new(json.as_bytes(), 0), depth.saturating_add(1)) {
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

/// Bytes read one at a time with one byte of lookahead.
trait Source {
    fn peek(&mut self) -> Option<u8>;
    fn bump(&mut self);
    fn offset(&self) -> usize;
}

struct Slice<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Slice<'a> {
    fn new(s: &'a [u8], i: usize) -> Self {
        Self { s, i }
    }
}

impl Source for Slice<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn bump(&mut self) {
        self.i = self.i.saturating_add(1);
    }
    fn offset(&self) -> usize {
        self.i
    }
}

/// The decoded UTF-8 of a JSON string literal whose text starts at `i`,
/// ending at its closing quote. The literal is already known to be valid.
struct Unescape<'a> {
    s: &'a [u8],
    i: usize,
    buffer: [u8; 4],
    len: usize,
    at: usize,
}

impl<'a> Unescape<'a> {
    fn new(s: &'a [u8], i: usize) -> Self {
        Self {
            s,
            i,
            buffer: [0; 4],
            len: 0,
            at: 0,
        }
    }

    fn hex(&self, i: usize) -> Option<u32> {
        let digits = self.s.get(i..i.checked_add(4)?)?;
        let text = std::str::from_utf8(digits).ok()?;
        u32::from_str_radix(text, 16).ok()
    }

    fn fill(&mut self) {
        if self.at < self.len {
            return;
        }
        (self.len, self.at) = (0, 0);
        let Some(&byte) = self.s.get(self.i) else {
            return;
        };
        let decoded = match byte {
            b'"' => return,
            b'\\' => {
                let escaped = self.s.get(self.i + 1).copied();
                self.i += 2;
                match escaped {
                    Some(b'b') => '\u{8}',
                    Some(b'f') => '\u{c}',
                    Some(b'n') => '\n',
                    Some(b'r') => '\r',
                    Some(b't') => '\t',
                    Some(b'u') => {
                        let unit = self.hex(self.i).unwrap_or(0xfffd);
                        self.i += 4;
                        let pair = (0xd800..0xdc00).contains(&unit)
                            && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                            && self
                                .hex(self.i + 2)
                                .is_some_and(|low| (0xdc00..0xe000).contains(&low));
                        let code = if pair {
                            let low = self.hex(self.i + 2).unwrap_or(0xdc00);
                            self.i += 6;
                            0x10000 + ((unit - 0xd800) << 10) + (low - 0xdc00)
                        } else {
                            unit
                        };
                        char::from_u32(code).unwrap_or('\u{fffd}')
                    }
                    Some(other) => char::from(other),
                    None => return,
                }
            }
            _ => {
                self.i += 1;
                self.buffer[0] = byte;
                self.len = 1;
                return;
            }
        };
        self.len = decoded.encode_utf8(&mut self.buffer).len();
    }
}

impl Source for Unescape<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.fill();
        (self.at < self.len).then(|| self.buffer[self.at])
    }
    fn bump(&mut self) {
        self.fill();
        self.at += 1;
    }
    fn offset(&self) -> usize {
        self.i
    }
}

fn skip_ws(src: &mut impl Source) {
    while matches!(src.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        src.bump();
    }
}

/// One pass over JSON text with `JSON.parse`'s grammar, as the session's
/// reader applies it, refusing an array or object opened at `max_depth` open
/// containers. Allocates only the open containers' kinds.
fn scan(mut src: impl Source, max_depth: usize) -> Scan {
    // The kinds of the open containers: true for an object.
    let mut open: Vec<bool> = Vec::new();
    let mut members = 0usize;
    let (mut all_value, mut all_mcp) = (true, true);
    let (mut top_object, mut last_string_at) = (false, None);
    loop {
        skip_ws(&mut src);
        if open.len() == 1 && top_object {
            last_string_at = (src.peek() == Some(b'"')).then(|| src.offset());
        }
        match src.peek() {
            Some(b'[' | b'{') if open.len() >= max_depth => {
                return Scan::TooDeep {
                    in_value: top_object && all_value,
                };
            }
            Some(b'[') => {
                src.bump();
                skip_ws(&mut src);
                if src.peek() == Some(b']') {
                    src.bump();
                } else {
                    open.push(false);
                    continue;
                }
            }
            Some(b'{') => {
                if open.is_empty() {
                    top_object = true;
                }
                src.bump();
                skip_ws(&mut src);
                if src.peek() == Some(b'}') {
                    src.bump();
                } else {
                    open.push(true);
                    let Some((value, mcp)) = key(&mut src) else {
                        return Scan::Malformed;
                    };
                    if open.len() == 1 {
                        members += 1;
                        all_value &= value;
                        all_mcp &= mcp;
                    }
                    continue;
                }
            }
            Some(b'"') => {
                if string(&mut src).is_none() {
                    return Scan::Malformed;
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                if number(&mut src).is_none() {
                    return Scan::Malformed;
                }
            }
            Some(_) => {
                if !literal(&mut src) {
                    return Scan::Malformed;
                }
            }
            None => return Scan::Malformed,
        }
        // A value is complete: continue or close what holds it.
        loop {
            skip_ws(&mut src);
            let Some(&object) = open.last() else {
                if src.peek().is_some() {
                    return Scan::Malformed;
                }
                let shape = match last_string_at {
                    _ if !top_object || members == 0 => Shape::Other,
                    _ if all_value => Shape::Value,
                    Some(at) if all_mcp => Shape::McpResponse(at),
                    _ => Shape::Other,
                };
                return Scan::Complete(shape);
            };
            match src.peek() {
                Some(b',') if object => {
                    src.bump();
                    skip_ws(&mut src);
                    let Some((value, mcp)) = key(&mut src) else {
                        return Scan::Malformed;
                    };
                    if open.len() == 1 {
                        members += 1;
                        all_value &= value;
                        all_mcp &= mcp;
                    }
                    break;
                }
                Some(b',') => {
                    src.bump();
                    break;
                }
                Some(b'}') if object => {
                    src.bump();
                    open.pop();
                }
                Some(b']') if !object => {
                    src.bump();
                    open.pop();
                }
                _ => return Scan::Malformed,
            }
        }
    }
}

fn literal(src: &mut impl Source) -> bool {
    let word: &[u8] = match src.peek() {
        Some(b't') => b"true",
        Some(b'f') => b"false",
        Some(b'n') => b"null",
        _ => return false,
    };
    for expected in word {
        if src.peek() != Some(*expected) {
            return false;
        }
        src.bump();
    }
    true
}

/// `"key" :`; whether the key is `value` and whether `mcpResponseJson`.
fn key(src: &mut impl Source) -> Option<(bool, bool)> {
    if src.peek() != Some(b'"') {
        return None;
    }
    let names = string(src)?;
    skip_ws(src);
    if src.peek() != Some(b':') {
        return None;
    }
    src.bump();
    Some(names)
}

/// A string: whether its decoded text is `value` and whether `mcpResponseJson`.
fn string(src: &mut impl Source) -> Option<(bool, bool)> {
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
    src.bump();
    loop {
        let byte = src.peek()?;
        src.bump();
        match byte {
            b'"' => break,
            b'\\' => {
                let escaped = src.peek()?;
                src.bump();
                let unit = match escaped {
                    b'"' | b'\\' | b'/' => escaped,
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let digit = char::from(src.peek()?).to_digit(16)?;
                            src.bump();
                            code = code * 16 + digit;
                        }
                        push(u8::try_from(code).ok().filter(u8::is_ascii));
                        continue;
                    }
                    _ => return None,
                };
                push(Some(unit));
            }
            0x00..=0x1f => return None,
            byte => push(byte.is_ascii().then_some(byte)),
        }
    }
    let done = |state: Option<usize>, target: &[u8]| state == Some(target.len());
    Some((done(matched[0], TARGETS[0]), done(matched[1], TARGETS[1])))
}

/// A number in JSON's grammar.
fn number(src: &mut impl Source) -> Option<()> {
    fn digits(src: &mut impl Source) -> usize {
        let mut count = 0usize;
        while src.peek().is_some_and(|b| b.is_ascii_digit()) {
            src.bump();
            count = count.saturating_add(1);
        }
        count
    }
    if src.peek() == Some(b'-') {
        src.bump();
    }
    match src.peek()? {
        b'0' => src.bump(),
        b'1'..=b'9' => {
            digits(src);
        }
        _ => return None,
    }
    if src.peek() == Some(b'.') {
        src.bump();
        if digits(src) == 0 {
            return None;
        }
    }
    if matches!(src.peek(), Some(b'e' | b'E')) {
        src.bump();
        if matches!(src.peek(), Some(b'+' | b'-')) {
            src.bump();
        }
        if digits(src) == 0 {
            return None;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str, depth: usize) -> Scan {
        scan(Slice::new(text.as_bytes(), 0), depth)
    }

    #[test]
    fn the_scan_reads_the_outcome_shapes_the_reader_accepts() {
        let mcp = r#"{"mcpResponseJson":"{}"}"#;
        for (text_, expected) in [
            (r#"{"value":1}"#, Scan::Complete(Shape::Value)),
            (
                r#" { "\u0076alue" : [1, {"a": null}] } "#,
                Scan::Complete(Shape::Value),
            ),
            (r#"{"value":1,"value":"x"}"#, Scan::Complete(Shape::Value)),
            (mcp, Scan::Complete(Shape::McpResponse(19))),
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
            assert_eq!(text(text_, 3), expected, "{text_}");
            // The reader agrees on what is JSON and what is too deep.
            let read = openui_lang::parse_json_value(text_, 3);
            match expected {
                Scan::Malformed => assert!(read.is_err_and(|e| !e.depth_limited()), "{text_}"),
                Scan::TooDeep { .. } => {
                    assert!(read.is_err_and(|e| e.depth_limited()), "{text_}")
                }
                Scan::Complete(_) => assert!(read.is_ok(), "{text_}"),
            }
        }
    }

    #[test]
    fn an_escaped_response_is_read_as_the_reader_reads_its_decoded_text() {
        for inner in [
            r#"{"structuredContent":{"a":[1,"\u00e9",true]}}"#,
            r#"  {"content":[{"type":"text","text":"\ud83d\ude00 \ud800"}]}"#,
            "{",
            r#"{"a":[1,]}"#,
            r#"[[[[1]]]]"#,
            "\"\\u0001\"",
        ] {
            let outer = serde_json::json!({ "mcpResponseJson": inner }).to_string();
            let Scan::Complete(Shape::McpResponse(at)) = text(&outer, 4) else {
                panic!("{outer}");
            };
            let decoded = scan(Unescape::new(outer.as_bytes(), at + 1), 3);
            let read = openui_lang::parse_json_value(inner, 3);
            match decoded {
                Scan::Malformed => assert!(read.is_err_and(|e| !e.depth_limited()), "{inner}"),
                Scan::TooDeep { .. } => {
                    assert!(read.is_err_and(|e| e.depth_limited()), "{inner}")
                }
                Scan::Complete(_) => assert!(read.is_ok(), "{inner}"),
            }
        }
    }
}
