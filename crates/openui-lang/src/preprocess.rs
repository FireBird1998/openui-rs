//! Cleaning up model output before parsing, as the reference parser's
//! `stripFences`, `stripComments` and `preprocess`.
//!
//! Byte positions stand in for the reference's UTF-16 positions: every
//! position that's sliced at is a newline, a quote, a comment marker or the
//! end of an ASCII run, so slices are whole UTF-8.

use crate::js;

/// Model output ready to parse: trimmed, the code fences' contents (if any),
/// `//` and `#` comments removed, trimmed again.
pub fn preprocess(input: &str) -> String {
    let stripped = strip_comments(&strip_fences(js::trim(input)));
    js::trim(&stripped).to_owned()
}

/// `/^[ \t]*```[ \t]*([\w.+-]*)[ \t]*(.*?)[ \t]*\r?$/` on a line: a fence
/// line's tag and the rest of the line after it, or `None` for other lines.
/// The rest is `.*?`, which matches no line terminator: a `\r`, U+2028 or
/// U+2029 in it (other than a final `\r`) makes the line not a fence.
pub(crate) fn fence_line(line: &str) -> Option<(&str, &str)> {
    let after = line.trim_start_matches([' ', '\t']).strip_prefix("```")?;
    let after = after.trim_start_matches([' ', '\t']);
    let tag_len = after
        .bytes()
        .take_while(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
        .count();
    let (tag, rest) = after.split_at(tag_len);
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest.strip_suffix('\r').unwrap_or(rest);
    if rest.contains(['\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    Some((tag, rest.trim_end_matches([' ', '\t'])))
}

/// `/^\s*\$?[A-Za-z_]\w*\s*=(?!=)/`: whether the text starts an OpenUI Lang
/// statement (`name =` or `$name =`). `\s` spans line breaks too.
pub(crate) fn statement_start(text: &str) -> bool {
    matches!(starts(text), Starts::Yes | Starts::YesForNow)
}

/// What [`statement_start`] says of text that may still grow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Starts {
    Yes,
    No,
    /// It matches, but a `=` next would make the `=` an `==`.
    YesForNow,
    /// It doesn't match, but more text could make it.
    NoForNow,
}

pub(crate) fn starts(text: &str) -> Starts {
    let rest = text.trim_start_matches(js::is_js_whitespace);
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    let mut bytes = rest.bytes();
    match bytes.next() {
        None => return Starts::NoForNow,
        Some(b) if b.is_ascii_alphabetic() || b == b'_' => {}
        Some(_) => return Starts::No,
    }
    let name_len = 1 + bytes
        .take_while(|&b| b.is_ascii_alphanumeric() || b == b'_')
        .count();
    let rest = rest[name_len..].trim_start_matches(js::is_js_whitespace);
    let mut bytes = rest.bytes();
    match bytes.next() {
        None => Starts::NoForNow,
        Some(b'=') => match bytes.next() {
            None => Starts::YesForNow,
            Some(b'=') => Starts::No,
            Some(_) => Starts::Yes,
        },
        Some(_) => Starts::No,
    }
}

/// Code written on a fence opener's line (```` ```openui root = ... ```` or
/// ```` ```root = ... ````), or `None`.
pub(crate) fn opener_code(tag: &str, rest: &str) -> Option<String> {
    if statement_start(rest) {
        return Some(rest.to_owned());
    }
    if !tag.is_empty() && rest.starts_with('=') && !rest.starts_with("==") {
        return Some(format!("{tag} {rest}"));
    }
    None
}

/// Strings and brackets so far in code lines: whether a `"..."` string is
/// open, and the bracket depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CodeScan {
    pub(crate) open: bool,
    pub(crate) depth: u32,
}

impl CodeScan {
    /// Scan one line of code.
    pub(crate) fn line(&mut self, line: &str) {
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if self.open {
                if c == '\\' {
                    chars.next();
                } else if c == '"' {
                    self.open = false;
                }
            } else if c == '"' {
                self.open = true;
            } else if matches!(c, '(' | '[' | '{') {
                self.depth += 1;
            } else if matches!(c, ')' | ']' | '}') && self.depth > 0 {
                self.depth -= 1;
            }
        }
    }
}

/// Whether a block's first code line, after blank and comment lines, starts
/// a statement: `statement_start` of the block without its leading
/// `/^(\s*(\/\/|#)[^\n]*\n)+/`.
pub(crate) fn starts_program(block: &str) -> bool {
    statement_start(after_comment_lines(block))
}

/// The text after its leading comment lines (and the blank space between
/// them): what `/^(\s*(\/\/|#)[^\n]*\n)+/` leaves. A last comment line with no
/// line break after it stays.
pub(crate) fn after_comment_lines(text: &str) -> &str {
    let mut rest = text;
    loop {
        let line = rest.trim_start_matches(js::is_js_whitespace);
        if !(line.starts_with("//") || line.starts_with('#')) {
            return rest;
        }
        match line.find('\n') {
            Some(newline) => rest = &line[newline + 1..],
            None => return rest,
        }
    }
}

/// The program in markdown code fences, or the input when no fence holds one.
///
/// A fence line (three backticks at the start of a line, after spaces or
/// tabs) opens a block, and the next one closes it; one left open runs to the
/// end. A pair of fences on one line is inline code. Code may follow the tag on
/// the opening line. An `openui-js` block is a script and is skipped. If some
/// block's first code line starts a statement, the program is every block
/// joined with newlines. Otherwise a statement in the text outside the blocks
/// makes that text the program (a stray closing fence, or `js` helper blocks
/// after an unfenced program). A fence line inside a `"..."` string is part of
/// the string: in a block, strings may open on any line; outside, only on a
/// statement and the lines that continue it.
pub fn strip_fences(input: &str) -> String {
    if !input.contains("```") {
        return input.to_owned();
    }
    let mut blocks: Vec<String> = Vec::new();
    let mut outside: Vec<&str> = Vec::new();
    let mut body: Option<Vec<String>> = None;
    let mut script = false;
    let mut scan = CodeScan::default();
    for line in input.split('\n') {
        let fence = if scan.open { None } else { fence_line(line) };
        let Some((tag, rest)) = fence else {
            if script {
                continue;
            }
            match &mut body {
                Some(lines) => lines.push(line.to_owned()),
                None => outside.push(line),
            }
            if scan.open || scan.depth > 0 || body.is_some() || statement_start(line) {
                scan.line(line);
            }
            continue;
        };
        // A fence line ends any statement.
        scan.depth = 0;
        if let Some(lines) = body.take() {
            if !script {
                blocks.push(lines.join("\n"));
            }
            script = false;
            continue;
        }
        if rest.contains("```") {
            continue;
        }
        script = tag.eq_ignore_ascii_case("openui-js");
        body = Some(opener_code(tag, rest).into_iter().collect());
    }
    if let Some(lines) = body
        && !script
    {
        blocks.push(lines.join("\n"));
    }
    if blocks.iter().any(|b| starts_program(b)) {
        return blocks.join("\n");
    }
    let unfenced = outside.iter().any(|line| statement_start(line));
    if unfenced || blocks.iter().all(|b| js::trim(b).is_empty()) {
        outside.join("\n")
    } else {
        blocks.join("\n")
    }
}

/// Remove `//` and `#` comments outside strings, then trailing whitespace on
/// lines that had one. String state carries over from line to line, so an
/// apostrophe in prose opens a `'...'` string that can hide later comments.
fn strip_comments(input: &str) -> String {
    let mut in_str: Option<u8> = None;
    let lines: Vec<&str> = input
        .split('\n')
        .map(|line| {
            let s = line.as_bytes();
            let mut i = 0;
            while i < s.len() {
                let c = s[i];
                if let Some(quote) = in_str {
                    if c == b'\\' && i + 1 < s.len() {
                        i += 2;
                        continue;
                    }
                    if c == quote {
                        in_str = None;
                    }
                } else if c == b'"' || c == b'\'' {
                    in_str = Some(c);
                } else if (c == b'/' && s.get(i + 1) == Some(&b'/')) || c == b'#' {
                    return js::trim_end(&line[..i]);
                }
                i += 1;
            }
            line
        })
        .collect();
    lines.join("\n")
}

/// `preprocess`, chunk by chunk. After every push, [`Preprocessor::cleaned`]
/// equals `preprocess` of everything pushed so far, and a push costs the chunk
/// plus the last line, not the whole text.
///
/// Every line before the last one with text in it is final: it goes through
/// the fence rules once, into one of two comment stages, the text inside
/// blocks and the text outside them. The last line, trimmed as the end of
/// the text is, goes through them anew on every push, as do the choices that
/// depend on it: which of the two the program is. [`Preprocessor::push`]
/// reports how much of the previous cleaned text survived, so a caller that
/// cached work on it knows what to throw away.
pub(crate) struct Preprocessor {
    /// Past the raw text's leading whitespace.
    started: bool,
    /// The raw text from the start of the last line with text in it.
    pending: String,
    /// Where the last line of `pending` starts.
    line_start: usize,
    /// Where the last line with text in it starts: `pending` before it is
    /// complete lines that the fence rules can take for good.
    tail_start: usize,
    fences: Fences,
    blocks: Comments,
    outside: Comments,
    /// Which of the two `cleaned` comes from.
    chosen: Stream,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Blocks,
    Outside,
}

/// The fence rules' state after the final lines.
#[derive(Default)]
struct Fences {
    scan: CodeScan,
    /// The open block, if one is.
    body: Option<Block>,
    /// The open block is `openui-js`: a script, skipped.
    script: bool,
    /// A closed block starts a program.
    starts: bool,
    /// A closed block has text in it.
    nonblank: bool,
    /// A line outside blocks starts a statement.
    unfenced: bool,
}

/// An open block.
#[derive(Default)]
struct Block {
    lines: usize,
    nonblank: bool,
    /// Whether it starts a program, once more lines can't change that.
    starts: Option<bool>,
    /// Its text from where that's still being decided, while it is.
    head: String,
}

impl Block {
    /// The block's text with `line` after it.
    fn with(&self, line: &str) -> String {
        if self.lines == 0 {
            line.to_owned()
        } else {
            format!("{}\n{line}", self.head)
        }
    }

    fn add(&mut self, line: &str) {
        self.nonblank |= !js::trim(line).is_empty();
        if self.starts.is_none() {
            let text = self.with(line);
            match block_starts(&text) {
                Starts::Yes => self.starts = Some(true),
                Starts::No => self.starts = Some(false),
                _ => self.head = after_comment_lines(&text).to_owned(),
            }
        }
        self.lines += 1;
    }

    /// Whether it starts a program if the text ends after `last` (a line
    /// still to come, or none).
    fn starts_if_ending(&self, last: Option<&str>) -> bool {
        match (self.starts, last) {
            (Some(starts), _) => starts,
            (None, Some(line)) => matches!(
                block_starts(&self.with(line)),
                Starts::Yes | Starts::YesForNow
            ),
            (None, None) => matches!(block_starts(&self.head), Starts::Yes | Starts::YesForNow),
        }
    }
}

/// [`starts_program`] of a block that may still grow. A comment line it ends
/// with may still be stripped, once a line follows it.
fn block_starts(text: &str) -> Starts {
    let rest = after_comment_lines(text);
    let line = rest.trim_start_matches(js::is_js_whitespace);
    if line.starts_with("//") || line.starts_with('#') {
        return Starts::NoForNow;
    }
    starts(rest)
}

impl Preprocessor {
    pub(crate) fn new() -> Preprocessor {
        Preprocessor {
            started: false,
            pending: String::new(),
            line_start: 0,
            tail_start: 0,
            fences: Fences::default(),
            blocks: Comments::default(),
            outside: Comments::default(),
            chosen: Stream::Outside,
        }
    }

    /// `preprocess` of everything pushed so far.
    pub(crate) fn cleaned(&self) -> &str {
        &self.stream(self.chosen).cleaned
    }

    fn stream(&self, which: Stream) -> &Comments {
        match which {
            Stream::Blocks => &self.blocks,
            Stream::Outside => &self.outside,
        }
    }

    /// Forget everything, as if freshly made.
    pub(crate) fn reset(&mut self) {
        *self = Preprocessor::new();
    }

    /// Add the next chunk of raw text. Returns how many leading bytes of the
    /// previous `cleaned()` are still the leading bytes of the new one.
    pub(crate) fn push(&mut self, chunk: &str) -> usize {
        let mut rest = chunk;
        if !self.started {
            match chunk
                .char_indices()
                .find(|&(_, c)| !js::is_js_whitespace(c))
            {
                Some((i, _)) => {
                    self.started = true;
                    rest = &chunk[i..];
                }
                None => rest = "",
            }
        }
        let at = self.pending.len();
        self.pending.push_str(rest);
        for (i, c) in rest.char_indices() {
            if c == '\n' {
                self.line_start = at + i + 1;
            } else if !js::is_js_whitespace(c) {
                self.tail_start = self.line_start;
            }
        }
        self.commit_lines();
        self.tentative()
    }

    /// Put the complete lines before the last one with text through for good.
    fn commit_lines(&mut self) {
        if self.tail_start == 0 {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        let mut lines = pending[..self.tail_start].split('\n');
        // The lines before the tail all end in a line break, so the last
        // piece is empty.
        lines.next_back();
        for line in lines {
            self.commit(line);
        }
        self.pending = pending[self.tail_start..].to_owned();
        self.line_start -= self.tail_start;
        self.tail_start = 0;
    }

    /// One final line through the fence rules, into a comment stage.
    fn commit(&mut self, line: &str) {
        let f = &mut self.fences;
        let fence = if f.scan.open { None } else { fence_line(line) };
        let Some((tag, rest)) = fence else {
            if f.script {
                return;
            }
            match &mut f.body {
                Some(block) => {
                    block.add(line);
                    self.blocks.finish(line);
                }
                None => {
                    f.unfenced |= statement_start(line);
                    self.outside.finish(line);
                }
            }
            if f.scan.open || f.scan.depth > 0 || f.body.is_some() || statement_start(line) {
                f.scan.line(line);
            }
            return;
        };
        // A fence line ends any statement.
        f.scan.depth = 0;
        if let Some(block) = f.body.take() {
            if !f.script {
                if block.lines == 0 {
                    // An empty block joins the others as an empty line.
                    self.blocks.finish("");
                }
                f.starts |= block.starts_if_ending(None);
                f.nonblank |= block.nonblank;
            }
            f.script = false;
            return;
        }
        if rest.contains("```") {
            return;
        }
        f.script = tag.eq_ignore_ascii_case("openui-js");
        let mut block = Block::default();
        if let Some(code) = opener_code(tag, rest) {
            block.add(&code);
            if !f.script {
                self.blocks.finish(&code);
            }
        }
        f.body = Some(block);
    }

    /// The last line with text in it, as the end of the text: where it goes,
    /// which of the two texts is the program, and that text brought up to
    /// date. Returns how much of the previous `cleaned()` is unchanged.
    fn tentative(&mut self) -> usize {
        let last = js::trim_end(&self.pending);
        let f = &self.fences;
        let mut starts = f.starts;
        let mut nonblank = f.nonblank;
        let mut unfenced = f.unfenced;
        let mut blocks_tail = "";
        let mut outside_tail = "";
        let mut new_block = String::new();
        let fence = if f.scan.open || last.is_empty() {
            None
        } else {
            fence_line(last)
        };
        match (fence, &f.body) {
            // Nothing yet.
            (None, _) if last.is_empty() => {
                if let (Some(block), false) = (&f.body, f.script) {
                    starts |= block.starts_if_ending(None);
                    nonblank |= block.nonblank;
                }
            }
            (None, Some(block)) => {
                if !f.script {
                    blocks_tail = last;
                    starts |= block.starts_if_ending(Some(last));
                    nonblank |= block.nonblank || !js::trim(last).is_empty();
                }
            }
            (None, None) => {
                outside_tail = last;
                unfenced |= statement_start(last);
            }
            // It closes the open block.
            (Some(_), Some(block)) => {
                if !f.script {
                    starts |= block.starts_if_ending(None);
                    nonblank |= block.nonblank;
                }
            }
            (Some((_, rest)), None) if rest.contains("```") => {}
            // It opens a block.
            (Some((tag, rest)), None) => {
                if !tag.eq_ignore_ascii_case("openui-js") {
                    new_block = opener_code(tag, rest).unwrap_or_default();
                    starts |= starts_program(&new_block);
                    nonblank |= !js::trim(&new_block).is_empty();
                }
            }
        }
        // An open block with no lines yet joins the others as an empty line.
        let empty_open = match (&f.body, fence) {
            (Some(block), _) => !f.script && block.lines == 0 && blocks_tail.is_empty(),
            (None, Some((tag, rest))) => {
                !rest.contains("```") && !tag.eq_ignore_ascii_case("openui-js")
            }
            (None, None) => false,
        };
        let blocks_tail = if new_block.is_empty() {
            blocks_tail
        } else {
            new_block.as_str()
        };
        let has_blocks_tail = !blocks_tail.is_empty() || empty_open;
        let chosen = if starts {
            Stream::Blocks
        } else if unfenced || !nonblank {
            Stream::Outside
        } else {
            Stream::Blocks
        };
        self.blocks.set_tail(blocks_tail, has_blocks_tail);
        self.outside
            .set_tail(outside_tail, !outside_tail.is_empty());
        let before = self.chosen;
        self.chosen = chosen;
        let unchanged = match chosen {
            Stream::Blocks => self.blocks.rebuild(),
            Stream::Outside => self.outside.rebuild(),
        };
        if before == chosen {
            return unchanged;
        }
        // The program switched texts: compare the two.
        common_prefix(&self.stream(before).cleaned, &self.stream(chosen).cleaned)
    }
}

/// How many leading bytes two strings share.
fn common_prefix(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// Comment stripping and trimming of a text that grows line by line, its last
/// line still changing.
#[derive(Default)]
struct Comments {
    /// Comment-stripped output of the finished lines, each ending in `\n`.
    stable: String,
    /// The quote of the string open at the start of the next line.
    in_str: Option<u8>,
    /// The last line, not finished yet.
    line: Vec<u8>,
    /// Whether there is one: an empty last line still ends the text after a
    /// line break.
    has_line: bool,
    /// Comment stripping of `line`, as far as it has got.
    scan: Scan,
    /// Offset in `stable` of its first non-whitespace char, once there is one.
    lead: Option<usize>,
    /// Offset in `stable` just past its last non-whitespace char.
    last_nonws_end: usize,
    /// The text so far, comment-stripped and trimmed.
    cleaned: String,
    /// `cleaned[..final_len]` is `stable[lead..synced]`, and never changes.
    final_len: usize,
    synced: usize,
}

/// Comment stripping of one line, resumable: `pos` is how far it has scanned.
#[derive(Default)]
struct Scan {
    pos: usize,
    /// The quote of the string that is open, carried over from the line before.
    in_str: Option<u8>,
    /// Where a comment starts, once one is found; the line ends there.
    comment_at: Option<usize>,
}

impl Comments {
    /// Make `text` the last line, scanned as far as it can be. Scanning
    /// resumes where it was when the old last line is the start of it.
    fn set_tail(&mut self, text: &str, present: bool) {
        if !text.as_bytes().starts_with(&self.line) {
            self.line.clear();
            self.scan = Scan {
                in_str: self.in_str,
                ..Scan::default()
            };
        }
        self.line
            .extend_from_slice(&text.as_bytes()[self.line.len()..]);
        self.has_line = present;
        self.scan_line(false);
    }

    /// The last line is `text`, and finished.
    fn finish(&mut self, text: &str) {
        self.set_tail(text, true);
        self.scan_line(true);
        let output = line_output(&self.line, self.scan.comment_at).to_owned();
        let kept = js::trim_end(&output);
        if !kept.is_empty() {
            let start = self.stable.len();
            if self.lead.is_none() {
                let leading = output.len() - output.trim_start_matches(js::is_js_whitespace).len();
                self.lead = Some(start + leading);
            }
            self.last_nonws_end = start + kept.len();
        }
        self.in_str = self.scan.in_str;
        self.stable.push_str(&output);
        self.stable.push('\n');
        self.line.clear();
        self.has_line = false;
        self.scan = Scan {
            in_str: self.in_str,
            ..Scan::default()
        };
    }

    /// Comment-strip the last line as far as possible. Mid-line, a trailing
    /// backslash in a string or a trailing `/` can't be decided yet and wait;
    /// at the end of the line (`whole`) they're plain characters.
    fn scan_line(&mut self, whole: bool) {
        let s = &self.line[..];
        let scan = &mut self.scan;
        while scan.pos < s.len() && scan.comment_at.is_none() {
            let c = s[scan.pos];
            if let Some(quote) = scan.in_str {
                if c == b'\\' {
                    if scan.pos + 1 < s.len() {
                        scan.pos += 2;
                        continue;
                    }
                    if !whole {
                        break;
                    }
                } else if c == quote {
                    scan.in_str = None;
                }
            } else if c == b'"' || c == b'\'' {
                scan.in_str = Some(c);
            } else if c == b'#' {
                scan.comment_at = Some(scan.pos);
                break;
            } else if c == b'/' {
                if scan.pos + 1 < s.len() {
                    if s[scan.pos + 1] == b'/' {
                        scan.comment_at = Some(scan.pos);
                        break;
                    }
                } else if !whole {
                    break;
                }
            }
            scan.pos += 1;
        }
    }

    /// Bring `cleaned` up to date and return how much of it is unchanged.
    fn rebuild(&mut self) -> usize {
        let old_final = self.final_len.min(self.cleaned.len());
        let old_tail: String = self.cleaned[old_final..].to_owned();
        let line = if self.has_line {
            line_output(&self.line, self.scan.comment_at)
        } else {
            ""
        };
        match self.lead {
            Some(lead) => {
                if self.synced < lead {
                    self.synced = lead;
                }
                self.cleaned.truncate(self.final_len);
                self.cleaned
                    .push_str(&self.stable[self.synced..self.last_nonws_end]);
                self.final_len = self.cleaned.len();
                self.synced = self.last_nonws_end;
                self.cleaned.push_str(&self.stable[self.last_nonws_end..]);
                self.cleaned.push_str(line);
                let keep = self.final_len + js::trim_end(&self.cleaned[self.final_len..]).len();
                self.cleaned.truncate(keep);
            }
            None => {
                // Every finished line is whitespace: the text starts in the last one.
                self.cleaned = js::trim(line).to_owned();
                self.final_len = 0;
            }
        }
        let new_tail = &self.cleaned[old_final.min(self.cleaned.len())..];
        old_final.min(self.cleaned.len()) + common_prefix(&old_tail, new_tail)
    }
}

/// A line as comment stripping leaves it: cut at the comment and trimmed.
fn line_output(line: &[u8], comment_at: Option<usize>) -> &str {
    // Cuts are at ASCII bytes, so the slice is whole UTF-8.
    match comment_at {
        Some(at) => js::trim_end(std::str::from_utf8(&line[..at]).expect("line is UTF-8")),
        None => std::str::from_utf8(line).expect("line is UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use proptest::prelude::*;
    use proptest::test_runner::{FileFailurePersistence, RngSeed};

    use super::{Preprocessor, preprocess};

    /// Push `text` in pieces of `size` code points, checking after every push
    /// that the incremental result is `preprocess` of the text so far and that
    /// the reported unchanged length is honest.
    fn check(text: &str, size: usize) {
        let mut pre = Preprocessor::new();
        let mut previous = String::new();
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        for group in chars.chunks(size) {
            let start = group[0].0;
            let end = group.last().map_or(start, |&(i, c)| i + c.len_utf8());
            let common = pre.push(&text[start..end]);
            let expected = preprocess(&text[..end]);
            assert_eq!(pre.cleaned(), expected, "after {end} bytes of {text:?}");
            assert!(common <= previous.len() && common <= expected.len());
            assert_eq!(previous.as_bytes()[..common], expected.as_bytes()[..common]);
            previous = expected;
        }
    }

    fn programs(dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                programs(&path, out);
            } else if path.extension().is_some_and(|e| e == "oui") {
                out.push(std::fs::read_to_string(&path).unwrap());
            }
        }
    }

    #[test]
    fn every_corpus_program_cleans_the_same_incrementally() {
        let root = crate::conformance_dir();
        let mut texts = Vec::new();
        programs(&root.join("cases"), &mut texts);
        programs(&root.join("imported/cases"), &mut texts);
        assert!(texts.len() > 200);
        for text in &texts {
            for size in [1, 3, 7] {
                check(text, size);
            }
        }
    }

    #[test]
    fn fences_comments_and_whitespace_at_every_boundary() {
        let texts = [
            "  \n\t a = 1 \n\n  ",
            "\u{FEFF}\u{A0}a = 1\u{A0}\n\u{2028}",
            "Here you go:\n```openui\nroot = Text(\"hi\")\n```\nEnjoy!",
            "```\nroot = Text(\"a```b\")\n```",
            "a = 1\n```openui\nb = 2\n```\nc = 3\n```openui\nd = 4\n```",
            "```openui\na = 1\n``\n`x\n```",
            "a = \"he said \\\"```\\\" ok\"\nb = `\n",
            "a = 1 // one\nb = '# not' # two\nc = \"x\\\n// still string?\"\nd = /\n",
            "x = 'it''s // c\ny = 2\n",
            "a = 1\n   \n \n",
            "// only\n# comments\n",
            "```openui",
            "```",
            "`",
            "``",
            "a\n```openui\n",
            "text with \"an unclosed string ```openui\nroot = 1\n",
            "````\nroot = Text(1)\n````",
            "a = 1 /",
            "a = \"x\\",
            "   \n```openui\n\n  \n b = 1 \n\n```\n\n",
            // 1.1: a script block, an inline pair, code on the opener, the tag as
            // the name, a stray closing fence, helpers after an unfenced program.
            "```openui-js\nx = 1\n```\nroot = Text(\"a\")",
            "Use ```openui``` inline.\nroot = Text(\"a\")",
            "```openui root = Text(\"a\")\nb = 1\n```",
            "```root = Text(\"a\")\n```",
            "root = Text(\"a\")\n```",
            "root = Text(\"a\")\n```js\nconsole.log(1)\n```",
            "Prose first.\n```js\nhelper()\n```\nMore prose.",
            // A fence line inside a multi-line string, unfenced and in a block.
            "a = \"line one\n```\nline three\"\nb = 1",
            "```openui\na = \"x\n```\ny\"\n```\nafter",
            // Comment lines before the first statement, and `\s` across lines.
            "```\n// c\n\n# d\nroot = 1\n```",
            "```\nfoo\n= 1\n```",
            "```\n# only a comment\n```\nx = 1",
            "``` \u{2028}\nx = 1\n```",
            "```openui\n```\n```openui\nroot = 1\n```",
        ];
        for text in texts {
            for size in 1..=5 {
                check(text, size);
            }
        }
    }

    fn noisy_text() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(vec![
            "\"",
            "'",
            "\\",
            "`",
            "``",
            "```",
            "```openui\n",
            "```openui-js\n",
            "```js\n",
            "```openui root = ",
            "```root ",
            "x = ",
            "$s = ",
            "==",
            "[",
            "]",
            "{",
            "}",
            "\u{2028}",
            "\n",
            "\r",
            " ",
            "\t",
            "\u{FEFF}",
            "\u{A0}",
            "/",
            "//",
            "#",
            "a",
            "=",
            "1",
            "(",
            ")",
            "é",
            "😀",
        ]);
        prop::collection::vec(pieces, 0..40).prop_map(|p| p.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 2000,
            rng_seed: RngSeed::Fixed(0x1006_2026),
            failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
            ..ProptestConfig::default()
        })]

        #[test]
        fn noisy_text_cleans_the_same_incrementally(text in noisy_text(), size in 1usize..6) {
            check(&text, size);
        }

        #[test]
        fn any_text_cleans_the_same_incrementally(text in any::<String>(), size in 1usize..6) {
            check(&text, size);
        }
    }
}
