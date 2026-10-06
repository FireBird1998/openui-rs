//! Cleaning up model output before parsing, as the reference parser's
//! `stripFences`, `stripComments` and `preprocess`.
//!
//! Byte positions stand in for the reference's UTF-16 positions: every
//! position that's sliced at is a backtick, a newline, a quote or a comment
//! marker, all ASCII.

use crate::js;

/// Model output ready to parse: trimmed, the code fences' contents (if any),
/// `//` and `#` comments removed, trimmed again.
pub fn preprocess(input: &str) -> String {
    let stripped = strip_comments(&strip_fences(js::trim(input)));
    js::trim(&stripped).to_owned()
}

/// Where a `"..."` string starting at `start` ends (just past its closing
/// quote, or at or past the end if unclosed); `start` itself if no string starts there.
fn skip_string(s: &[u8], start: usize) -> usize {
    if s.get(start) != Some(&b'"') {
        return start;
    }
    let mut i = start + 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// The first ``` at or after `from` that isn't inside a `"..."` string.
fn find_fence(s: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < s.len() {
        let next = skip_string(s, i);
        if next > i {
            i = next;
            continue;
        }
        if s[i..].starts_with(b"```") {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The contents of every markdown code fence, joined by newlines; the input
/// as-is when there are none. An unclosed fence (mid-stream) runs to the end.
/// ``` inside a `"..."` string doesn't count.
pub fn strip_fences(input: &str) -> String {
    let s = input.as_bytes();
    let mut blocks: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let Some(open) = find_fence(s, i) else { break };
        // The rest of the opening line is the language tag.
        let Some(newline) = s[open + 3..].iter().position(|&b| b == b'\n') else {
            // No newline yet after the fence: nothing of the block has arrived.
            blocks.push("");
            break;
        };
        let body = open + 3 + newline + 1;
        match find_fence(s, body) {
            Some(close) => {
                blocks.push(&input[body..close]);
                i = close + 3;
            }
            None => {
                blocks.push(&input[body..]);
                break;
            }
        }
    }
    if !blocks.is_empty() {
        return blocks.join("\n");
    }

    // The reference keeps this fallback for input starting with ``` that wasn't
    // matched above, though the scan always matches it first.
    if let Some(rest) = input.strip_prefix("```") {
        let body = match rest.find('\n') {
            Some(newline) => &rest[newline + 1..],
            None => rest,
        };
        return match body.rfind("```") {
            Some(fence) => body[..fence].to_owned(),
            None => body.to_owned(),
        };
    }
    input.to_owned()
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
/// equals `preprocess` of everything pushed so far, and all but a bounded
/// tail of it is final: the current line, and any whitespace-only lines
/// before it, are the only parts that can still change. So a push costs the
/// chunk plus that tail, not the whole text.
///
/// The one exception is the first code fence: until it opens, the text is
/// passed through as it is (a program may come with no fence at all), and when
/// it opens everything before it is dropped. [`Preprocessor::push`] reports how
/// much of the previous cleaned text survived, so a caller that cached work
/// on it knows what to throw away.
pub(crate) struct Preprocessor {
    /// Past the raw text's leading whitespace.
    started: bool,
    fence: Fence,
    /// A fence has opened: only fence bodies count from here on.
    fenced: bool,
    /// Fences opened so far; the bodies are joined with `\n`.
    blocks: usize,
    /// Backticks at the end of the current line, outside a string, that would
    /// close the fence if a third one came. They're in `line` for now.
    held: usize,
    /// Comment-stripped output of the complete lines, each ending in `\n`.
    stable: String,
    /// The current line as the fence stage produced it.
    line: Vec<u8>,
    /// Comment stripping of `line`, as far as it has got.
    scan: Scan,
    /// Offset in `stable` of its first non-whitespace char, once there is one.
    lead: Option<usize>,
    /// Offset in `stable` just past its last non-whitespace char.
    last_nonws_end: usize,
    /// `preprocess` of everything pushed so far.
    cleaned: String,
    /// `cleaned[..final_len]` is `stable[lead..synced]`, and never changes.
    final_len: usize,
    synced: usize,
}

enum Fence {
    /// Outside fences, looking for ``` (not inside a `"..."` string).
    Prose {
        in_str: bool,
        esc: bool,
        ticks: usize,
    },
    /// Just after ```: the rest of the line is the language tag.
    Opened,
    /// Inside a fence body, looking for the closing ```.
    Body { in_str: bool, esc: bool },
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

impl Preprocessor {
    pub(crate) fn new() -> Preprocessor {
        Preprocessor {
            started: false,
            fence: Fence::Prose {
                in_str: false,
                esc: false,
                ticks: 0,
            },
            fenced: false,
            blocks: 0,
            held: 0,
            stable: String::new(),
            line: Vec::new(),
            scan: Scan::default(),
            lead: None,
            last_nonws_end: 0,
            cleaned: String::new(),
            final_len: 0,
            synced: 0,
        }
    }

    /// `preprocess` of everything pushed so far.
    pub(crate) fn cleaned(&self) -> &str {
        &self.cleaned
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
        for &b in rest.as_bytes() {
            self.feed(b);
        }
        self.scan_line(false);
        self.rebuild()
    }

    /// One byte of the trimmed raw text through the fence stage.
    fn feed(&mut self, c: u8) {
        match &mut self.fence {
            Fence::Prose { in_str, esc, ticks } => {
                if *in_str {
                    if *esc {
                        *esc = false;
                    } else if c == b'\\' {
                        *esc = true;
                    } else if c == b'"' {
                        *in_str = false;
                    }
                    *ticks = 0;
                } else if c == b'"' {
                    *in_str = true;
                    *ticks = 0;
                } else if c == b'`' {
                    *ticks += 1;
                } else {
                    *ticks = 0;
                }
                if *ticks == 3 {
                    // A fence opens. The first one drops the prose passed
                    // through so far; later ones join their body with `\n`.
                    if !self.fenced {
                        self.fenced = true;
                        self.drop_lines();
                    } else {
                        self.emit(b'\n');
                    }
                    self.blocks += 1;
                    self.fence = Fence::Opened;
                } else if !self.fenced {
                    self.emit(c);
                }
            }
            Fence::Opened => {
                if c == b'\n' {
                    self.fence = Fence::Body {
                        in_str: false,
                        esc: false,
                    };
                }
            }
            Fence::Body { in_str, esc } => {
                if *in_str {
                    if *esc {
                        *esc = false;
                    } else if c == b'\\' {
                        *esc = true;
                    } else if c == b'"' {
                        *in_str = false;
                    }
                    self.emit(c);
                } else if c == b'`' {
                    self.held += 1;
                    self.line.push(c);
                    if self.held == 3 {
                        // The closing fence: the three backticks were never body.
                        self.line.truncate(self.line.len() - 3);
                        self.held = 0;
                        self.fence = Fence::Prose {
                            in_str: false,
                            esc: false,
                            ticks: 0,
                        };
                    }
                } else {
                    self.held = 0;
                    if c == b'"' {
                        *in_str = true;
                    }
                    self.emit(c);
                }
            }
        }
    }

    /// One byte of fence-stripped text into the comment stage.
    fn emit(&mut self, c: u8) {
        if c == b'\n' {
            self.finish_line();
        } else {
            self.line.push(c);
        }
    }

    /// Comment-strip the current line as far as possible. Mid-line, a trailing
    /// backslash in a string or a trailing `/` can't be decided yet and wait;
    /// at the end of the line (`whole`) they're plain characters.
    fn scan_line(&mut self, whole: bool) {
        let limit = self.line.len() - if whole { 0 } else { self.held };
        let s = &self.line[..limit];
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

    /// The current line is complete: its output is final.
    fn finish_line(&mut self) {
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
        let in_str = self.scan.in_str;
        self.stable.push_str(&output);
        self.stable.push('\n');
        self.line.clear();
        self.held = 0;
        self.scan = Scan {
            in_str,
            ..Scan::default()
        };
    }

    /// The first fence opened: nothing before it was program text.
    fn drop_lines(&mut self) {
        self.stable.clear();
        self.line.clear();
        self.held = 0;
        self.scan = Scan::default();
        self.lead = None;
        self.last_nonws_end = 0;
        self.final_len = 0;
        self.synced = 0;
    }

    /// Bring `cleaned` up to date and return how much of it is unchanged.
    fn rebuild(&mut self) -> usize {
        let old_final = self.final_len;
        let old_tail: String = self.cleaned[old_final..].to_owned();
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
                self.cleaned
                    .push_str(line_output(&self.line, self.scan.comment_at));
                let keep = self.final_len + js::trim_end(&self.cleaned[self.final_len..]).len();
                self.cleaned.truncate(keep);
            }
            None => {
                // Every complete line is whitespace: the text starts in this one.
                self.cleaned = js::trim(line_output(&self.line, self.scan.comment_at)).to_owned();
                self.final_len = 0;
            }
        }
        let new_tail = &self.cleaned[old_final.min(self.cleaned.len())..];
        let same = old_tail
            .bytes()
            .zip(new_tail.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        old_final.min(self.cleaned.len()) + same
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
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
