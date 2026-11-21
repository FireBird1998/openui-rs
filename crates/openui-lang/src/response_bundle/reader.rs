//! An incremental reader for a raw response that grows (ADR 0015).
//!
//! The one-shot readers ([`parse_response_bundle`](super::parse_response_bundle),
//! [`parse_message`](crate::parse_message) and the WebAssembly transport) read
//! the whole text on every call. A driver that hands over the whole response
//! after each chunk then does work quadratic in its length. [`ResponseReader`]
//! remembers how far it got: lines that have fully arrived go through the
//! message rules once and are folded into a committed state, and each call
//! goes through only the text after them (the last line, usually) with the
//! same rules, from that state. Its program feeds a [`StreamParser`] the new
//! part only. Any edit that is not an append reads the text afresh.
//!
//! The one-shot readers stay as they are and are the reference: a
//! differential test compares this reader with them on every prefix.
//!
//! Why a complete line can be committed: every rule of the message reader
//! that looks ahead (a retry marker inside a line, a bare scripts marker, the
//! content marker line the renderer looks for) looks at most at the rest of
//! its line, the line break included. The rules that depend on where the
//! text ends (holding back a last line that may be a marker, a retry marker
//! still arriving) touch only the last line. The one exception is an end
//! marker, which stops the reader when exactly one empty line follows it; a
//! line with an end marker is committed only once two more complete lines
//! follow it, so that can no longer happen.
//!
//! The line still arriving is itself resumed when it is a long line in a
//! scripts section's array, as a Cloud script's code always is: see
//! [`Resume`] (#122).

use std::borrow::Cow;

use super::{
    BundleParts, INVALID_SCRIPTS, ResponseBundleError, SCRIPTS, Scripts, has_content_line,
    mask_bare_scripts, read_name, read_scripts, script_json, text::TextFormat, write_bundle,
};
use crate::js::{lone_surrogate_unit, normalize_surrogate_pairs};
use crate::message::{MessageError, PREFIX, hold_retry, marker, read_attributes, split_retries};
use crate::result::ParseResult;
use crate::stream::StreamParser;
use crate::{Limits, Value};

/// Why a [`ResponseReader`] refused a response. The reader that refused
/// first names it, in the order the one-shot readers run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseReaderError {
    /// What [`parse_response_bundle`](super::parse_response_bundle) refuses.
    Bundle(ResponseBundleError),
    /// What [`parse_message`](crate::parse_message) refuses.
    Message(MessageError),
    /// An append to a text other than the last accepted one.
    NotAnAppend,
}

impl std::fmt::Display for ResponseReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bundle(error) => error.fmt(f),
            Self::Message(error) => error.fmt(f),
            Self::NotAnAppend => f.write_str("the response reader holds a different text"),
        }
    }
}

impl std::error::Error for ResponseReaderError {}

impl From<ResponseBundleError> for ResponseReaderError {
    fn from(error: ResponseBundleError) -> Self {
        Self::Bundle(error)
    }
}

/// Reads a raw response that grows, and parses its program, remembering how
/// far it got.
///
/// A call stages a read ([`ResponseReader::read`] and friends) and then
/// either keeps it ([`ResponseReader::try_parse`] accepts) or leaves the
/// reader exactly where it was: the next text is compared with the last text
/// it kept. A staged read that is never finished is dropped by the next read.
///
/// What it keeps is bounded by the program-size limit: one copy of the last
/// accepted text, the current section's content and context, and the program
/// (which the parser holds once more, as any stream parser does).
pub struct ResponseReader {
    format: TextFormat,
    limits: Limits,
    accepted: Accepted,
    /// The program of the last accepted read: the text the parser last took.
    program: String,
    /// Where `program` came from, to find what the next one keeps.
    origin: Origin,
    parser: StreamParser,
    /// The parser holds the last accepted program (lossily, when
    /// `escaped`); false once a caller drove the parser itself.
    in_sync: bool,
    /// Where the program first holds an escaped unit (a lone surrogate, or
    /// U+FDD0), in a WebAssembly reader.
    escaped: Option<usize>,
    /// The outcome of the last accepted read, or the staged one.
    last: Outcome,
    staged: Option<Box<Staged>>,
    examined: u64,
}

/// The accepted text and what has been folded from it.
struct Accepted {
    /// The text in the reader's representation: the core's convention as
    /// given, or JavaScript's UTF-16 units losslessly in UTF-8.
    raw: String,
    /// What the text is charged against the program-size limit.
    charged: usize,
    /// UTF-16 units in a WebAssembly text.
    units: usize,
    /// A WebAssembly text ends in a lone high surrogate, which the next
    /// chunk may pair.
    lone_high: Option<u16>,
    committed: Committed,
    /// The scan of the line still arriving, for the next append to resume.
    resume: Resume,
}

impl Accepted {
    fn new() -> Self {
        Self {
            raw: String::new(),
            charged: 0,
            units: 0,
            lone_high: None,
            committed: Committed::new(),
            resume: Resume::None,
        }
    }
}

/// Whether the next append can resume the scans of the line still arriving
/// (the text after the committed lines) instead of reading it again (#122).
///
/// A Cloud script's code is one JSON string, so its line is as long as its
/// code. While that line has no line break and no `]]>` and sits in the
/// content, in a scripts section's array, every rule of the message reader
/// takes it as one line of content (see [`ResponseReader::stage_resumed`]),
/// and only the scripts scan looks into it; that scan stops and continues
/// at any character boundary. So the scan's state at the end of the line so
/// far is all a call needs to go on with the new bytes only.
///
/// Like the committed lines, it changes only when a read is kept: a staged
/// read carries the scan it reached, and a refused one is dropped with it.
enum Resume {
    /// Not built for the line still arriving.
    None,
    /// The line starting at this raw offset cannot be resumed, and appends
    /// cannot change that: it holds a line break or `]]>`, or the committed
    /// lines put it somewhere other than a scripts section's array, or the
    /// scripts marker line's JSON is nested past the depth limit (or a check
    /// that cannot fail did).
    Never {
        line: usize,
    },
    Line(LineScan),
}

/// The scripts scan through the line still arriving, as far as later bytes
/// cannot change it. Bounded: a few numbers, whatever the line's length.
#[derive(Clone, Copy)]
struct LineScan {
    /// Where the line starts in the raw text: the committed lines' end.
    line: usize,
    /// The raw text before this has been scanned. What follows it, up to
    /// the text's end, is at most a held-back `/`, `]` or `]]` and a lone
    /// high surrogate the next chunk may pair.
    raw_at: usize,
    /// The scan's position in the content (the committed content, a line
    /// break and the line), which is `raw_at` in the raw text.
    pos: usize,
    /// The scan's stage there: always [`Stage::Array`].
    stage: Stage,
}

/// A read waiting for the draw that uses it.
struct Staged {
    /// The text is not an append: start from nothing.
    fresh: bool,
    /// How much of the accepted text stays, and what follows it.
    raw_keep: usize,
    appended: String,
    charged: usize,
    units: usize,
    lone_high: Option<u16>,
    outcome: Outcome,
    /// The program keeps this many leading bytes and continues with `rest`.
    kept: usize,
    rest: String,
    origin: Origin,
    /// The line still arriving was resumed, and its scan reached this.
    resume: Option<LineScan>,
}

/// What a resumed read found, for [`ResponseReader::stage`] to stage.
struct Resumed {
    outcome: Outcome,
    kept: usize,
    rest: String,
    origin: Origin,
    scan: LineScan,
}

/// What one read found, for the drivers.
#[derive(Clone, Default)]
struct Outcome {
    name: Option<String>,
    scripts: Vec<String>,
    revision: Option<String>,
    is_bundle: bool,
    complete: bool,
    error: Option<&'static str>,
    /// The message reader's context and attributes, from the text as given
    /// (bare scripts markers not masked).
    context: Option<String>,
    query: String,
    has_marker: bool,
    program_changed: bool,
}

/// Which bytes of a program are the committed content's own, so that the
/// next program can be compared from there on only.
#[derive(Clone, Copy, Default)]
struct Origin {
    epoch: u64,
    /// Leading program bytes that are the committed content's first bytes.
    from_base: usize,
    /// Where the program's two parts come from in the content: the part
    /// before the scripts section ends at `before`; the part after it, if
    /// any, starts at `after`, and its first `after_base` bytes are the
    /// committed content's too.
    before: usize,
    after: Option<usize>,
    after_base: usize,
}

/// The message rules' state after the committed lines.
struct Committed {
    /// Where the first line not committed starts in the raw text.
    at: usize,
    /// The renderer's first content marker line, once one is seen: whether
    /// its line break has arrived.
    content_line: Option<bool>,
    has_marker: bool,
    /// The bundle reader's view, bare scripts markers masked.
    masked: Fold<'static, Body, Body>,
    /// The message reader's view, for the context and attributes.
    plain: Fold<'static, Ignored, Body>,
    /// Bumped whenever the masked content starts over.
    epoch: u64,
    /// The shortest the masked content has been since the last program was
    /// taken: a closing marker can take back a CR.
    low: usize,
    extract: Extract,
}

impl Committed {
    fn new() -> Self {
        Self {
            at: 0,
            content_line: None,
            has_marker: false,
            masked: Fold::new(Body::default(), Body::default()),
            plain: Fold::new(Ignored, Body::default()),
            epoch: 0,
            low: 0,
            extract: Extract::new(),
        }
    }
}

// ─── The message reader's line rules ────────────────────────────────────────

/// A section body as the message reader builds it.
trait Section {
    fn push(&mut self, line: &str);
    /// Drop a CR the body ends with.
    fn close(&mut self);
    fn clear(&mut self);
}

#[derive(Default)]
struct Body {
    text: String,
    has_line: bool,
}

impl Section for Body {
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

    fn clear(&mut self) {
        *self = Body::default();
    }
}

/// A body nobody reads.
struct Ignored;

impl Section for Ignored {
    fn push(&mut self, _: &str) {}
    fn close(&mut self) {}
    fn clear(&mut self) {}
}

/// A committed body as the lines after it change it, without copying it:
/// the first `keep` bytes of `base`, then `extra`.
struct View<'a> {
    base: &'a str,
    keep: usize,
    /// The body started over.
    reset: bool,
    extra: String,
    has_line: bool,
}

impl<'a> View<'a> {
    fn of(body: &'a Body) -> Self {
        Self {
            base: &body.text,
            keep: body.text.len(),
            reset: false,
            extra: String::new(),
            has_line: body.has_line,
        }
    }

    fn base(&self) -> &'a str {
        self.base.get(..self.keep).unwrap_or_default()
    }

    fn text(&self) -> String {
        let mut text = String::with_capacity(self.keep.saturating_add(self.extra.len()));
        text.push_str(self.base());
        text.push_str(&self.extra);
        text
    }
}

impl Section for View<'_> {
    fn push(&mut self, line: &str) {
        if self.has_line {
            self.extra.push('\n');
        }
        self.extra.push_str(line);
        self.has_line = true;
    }

    fn close(&mut self) {
        if !self.extra.is_empty() {
            if self.extra.ends_with('\r') {
                self.extra.pop();
            }
        } else if self.base().ends_with('\r') {
            self.keep -= 1;
        }
    }

    fn clear(&mut self) {
        self.keep = 0;
        self.reset = true;
        self.extra.clear();
        self.has_line = false;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Content,
    Context,
    Other,
}

/// What a line did, beyond its section.
#[derive(PartialEq, Eq)]
enum Event {
    None,
    /// A content marker: everything starts over.
    Reset,
    /// An end marker.
    End,
}

/// The message reader's state between two lines: `split_message`'s loop.
struct Fold<'q, C, X> {
    part: Part,
    content: C,
    context: X,
    has_context: bool,
    query: Cow<'q, str>,
    end: bool,
}

impl<'q, C: Section, X: Section> Fold<'q, C, X> {
    fn new(content: C, context: X) -> Self {
        Self {
            part: Part::Content,
            content,
            context,
            has_context: false,
            query: Cow::Borrowed(""),
            end: false,
        }
    }

    fn body(&mut self) -> Option<&mut dyn Section> {
        match self.part {
            Part::Content => Some(&mut self.content),
            Part::Context => Some(&mut self.context),
            Part::Other => None,
        }
    }

    fn line(&mut self, line: &str) -> Event {
        let Some((kind, attributes)) = marker(line) else {
            if let Some(body) = self.body() {
                body.push(line);
            }
            return Event::None;
        };
        if let Some(body) = self.body() {
            body.close();
        }
        match kind {
            "content" => {
                self.content.clear();
                self.context.clear();
                self.has_context = false;
                self.query = Cow::Owned(attributes.to_owned());
                self.end = false;
                self.part = Part::Content;
                Event::Reset
            }
            "context" => {
                self.context.clear();
                self.has_context = true;
                self.part = Part::Context;
                Event::None
            }
            "end" => {
                self.end = true;
                Event::End
            }
            _ => {
                self.part = Part::Other;
                Event::None
            }
        }
    }

    /// The text after the committed lines, through the rules that depend on
    /// where the text ends: `split_message` from its retry split on.
    fn finish(&mut self, retries: &str, streaming: bool) {
        let text = if streaming {
            hold_retry(retries)
        } else {
            retries
        };
        let (before, last) = text
            .rsplit_once('\n')
            .map_or((None, text), |(before, last)| (Some(before), last));
        let held =
            streaming && !last.is_empty() && (last.starts_with(PREFIX) || PREFIX.starts_with(last));
        let visible = if held { before } else { Some(text) };
        let mut lines = visible.into_iter().flat_map(|text| text.split('\n'));
        while let Some(line) = lines.next() {
            if self.line(line) == Event::End && lines.clone().eq([""]) {
                break;
            }
        }
        if held && let Some(body) = self.body() {
            body.close();
        }
    }
}

/// The lines of a text that ends in a line break.
fn lines_of(text: &str) -> std::str::Split<'_, char> {
    text.strip_suffix('\n').unwrap_or(text).split('\n')
}

// ─── Finding the scripts section in the content ─────────────────────────────

/// `marker_index`'s scan, resumable: where it is in quotes and comments.
#[derive(Clone, Copy, Default)]
enum Scan {
    #[default]
    Code,
    Quoted(char),
    Escaped(char),
    Comment,
}

/// How far a resumable scan got.
enum Scanned {
    /// The first unquoted marker prefix starts here.
    Found(usize),
    /// Read up to here; what follows decides the rest.
    Stopped(usize),
    /// There is none (only when the text is known to end).
    Absent,
}

impl Scan {
    /// Scan `text` from its start, continuing this state. Without `eof`,
    /// stop before a character whose meaning depends on what has not
    /// arrived yet.
    fn advance(&mut self, text: &str, eof: bool) -> Scanned {
        let bytes = text.as_bytes();
        for (at, ch) in text.char_indices() {
            match *self {
                Scan::Escaped(quote) => *self = Scan::Quoted(quote),
                Scan::Quoted(quote) => {
                    if ch == '\\' {
                        *self = Scan::Escaped(quote);
                    } else if ch == quote {
                        *self = Scan::Code;
                    }
                }
                Scan::Comment => {
                    if ch == '\n' {
                        *self = Scan::Code;
                    }
                }
                Scan::Code => match ch {
                    '#' => *self = Scan::Comment,
                    '/' => match bytes.get(at + 1) {
                        Some(b'/') => *self = Scan::Comment,
                        Some(_) => {}
                        None if eof => {}
                        None => return Scanned::Stopped(at),
                    },
                    '"' | '\'' => *self = Scan::Quoted(ch),
                    ']' => {
                        let rest = &bytes[at + 1..];
                        if rest.starts_with(b"]>") {
                            return Scanned::Found(at);
                        }
                        if !eof && rest.len() < 2 && (rest.is_empty() || rest == b"]") {
                            return Scanned::Stopped(at);
                        }
                    }
                    _ => {}
                },
            }
        }
        if eof {
            Scanned::Absent
        } else {
            Scanned::Stopped(text.len())
        }
    }
}

/// The scripts array's shape so far, enough to know when reading it would
/// fail without reading it (see [`Shape::fails`]).
#[derive(Clone, Copy, Default)]
struct Shape {
    depth: usize,
    deepest: usize,
    in_string: bool,
    escaped: bool,
    first: Option<u8>,
    closed: bool,
    trailing: bool,
}

impl Shape {
    fn feed(&mut self, text: &str) {
        for &b in text.as_bytes() {
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if b == b'\\' {
                    self.escaped = true;
                } else if b == b'"' {
                    self.in_string = false;
                }
                continue;
            }
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                continue;
            }
            if self.closed {
                self.trailing = true;
                continue;
            }
            self.first.get_or_insert(b);
            match b {
                b'"' => self.in_string = true,
                b'[' | b'{' => {
                    self.depth += 1;
                    self.deepest = self.deepest.max(self.depth);
                }
                b']' | b'}' if self.depth > 0 => {
                    self.depth -= 1;
                    self.closed = self.depth == 0 && self.first == Some(b'[');
                }
                _ => {}
            }
        }
    }

    /// Whether the JSON reader certainly fails on this text, short of its
    /// depth limit, so that the scripts section is invalid. The reader
    /// refuses nesting only at an opening bracket with `limit` containers
    /// open; until its first error the text it read is a valid prefix, where
    /// these brackets count exactly the containers open, so nothing deeper
    /// than `limit` here means no depth error. And a valid array starts with
    /// `[`, ends with its bracket balanced outside strings, and has only
    /// whitespace after it.
    fn fails(&self, limit: usize) -> bool {
        self.deepest <= limit
            && (self.first != Some(b'[') || !self.closed || self.trailing || self.in_string)
    }
}

/// A scripts section as `read_scripts` read it.
type ScriptsRead = Result<Option<Scripts>, ResponseBundleError>;

/// `extract_scripts` resumable over a content that grows: how far it has
/// looked and what it has found.
#[derive(Clone, Default)]
struct Extract {
    /// Everything before this position (in the content) has been looked at.
    pos: usize,
    stage: Stage,
    /// The scripts marker line's JSON as read, once the line is complete
    /// and committed.
    count: Option<Result<Option<Value>, ResponseBundleError>>,
    /// The section as read up to the position given, committed, once the
    /// array had closed there: whitespace after it changes nothing.
    read: Option<(usize, std::sync::Arc<ScriptsRead>)>,
}

#[derive(Clone, Copy, Default)]
enum Stage {
    /// Looking for the first unquoted marker prefix.
    #[default]
    Seek,
    Seeking(Scan),
    /// One at `at`; is it the scripts marker?
    Marker {
        at: usize,
    },
    /// Another marker: the content is the program.
    Other,
    /// The scripts marker at `at`: looking for the end of its line.
    Count {
        at: usize,
    },
    /// Its line ends at `newline`: reading the array up to the next marker.
    Array {
        at: usize,
        newline: usize,
        scan: Scan,
        shape: Shape,
    },
    /// The array ends at the next marker, at `end`.
    Closed {
        at: usize,
        newline: usize,
        end: usize,
        shape: Shape,
    },
}

impl Extract {
    fn new() -> Self {
        Self::default()
    }

    /// Look at `text`, the content from `self.pos` on. Without `eof`, stop
    /// where what has not arrived could change the answer.
    fn advance(&mut self, text: &str, eof: bool, examined: &mut u64) {
        let start = self.pos;
        let end = start + text.len();
        loop {
            let local = self.pos - start;
            let rest = text.get(local..).unwrap_or_default();
            match self.stage {
                Stage::Seek => self.stage = Stage::Seeking(Scan::Code),
                Stage::Seeking(mut scan) => {
                    *examined += rest.len() as u64;
                    match scan.advance(rest, eof) {
                        Scanned::Found(at) => {
                            self.pos += at;
                            self.stage = Stage::Marker { at: self.pos };
                        }
                        Scanned::Stopped(at) => {
                            self.pos += at;
                            self.stage = Stage::Seeking(scan);
                            return;
                        }
                        Scanned::Absent => {
                            self.pos = end;
                            self.stage = Stage::Seeking(scan);
                            return;
                        }
                    }
                }
                Stage::Marker { at } => {
                    if rest.len() >= SCRIPTS.len() {
                        if rest.starts_with(SCRIPTS) {
                            self.pos += SCRIPTS.len();
                            self.stage = Stage::Count { at };
                            continue;
                        }
                        self.stage = Stage::Other;
                    } else if eof || !SCRIPTS.starts_with(rest) {
                        self.stage = Stage::Other;
                    }
                    return;
                }
                Stage::Other => {
                    self.pos = end;
                    return;
                }
                Stage::Count { at } => {
                    *examined += rest.len() as u64;
                    match rest.find('\n') {
                        Some(newline) => {
                            self.pos += newline + 1;
                            self.stage = Stage::Array {
                                at,
                                newline: start + local + newline,
                                scan: Scan::Code,
                                shape: Shape::default(),
                            };
                        }
                        None => {
                            self.pos = end;
                            return;
                        }
                    }
                }
                Stage::Array {
                    at,
                    newline,
                    mut scan,
                    mut shape,
                } => {
                    *examined += rest.len() as u64;
                    match scan.advance(rest, eof) {
                        Scanned::Found(found) => {
                            shape.feed(rest.get(..found).unwrap_or_default());
                            self.stage = Stage::Closed {
                                at,
                                newline,
                                end: self.pos + found,
                                shape,
                            };
                            self.pos = end;
                        }
                        Scanned::Stopped(stop) => {
                            shape.feed(rest.get(..stop).unwrap_or_default());
                            self.pos += stop;
                            self.stage = Stage::Array {
                                at,
                                newline,
                                scan,
                                shape,
                            };
                        }
                        Scanned::Absent => {
                            shape.feed(rest);
                            self.pos = end;
                            self.stage = Stage::Array {
                                at,
                                newline,
                                scan,
                                shape,
                            };
                        }
                    }
                    return;
                }
                Stage::Closed { .. } => {
                    self.pos = end;
                    return;
                }
            }
        }
    }
}

/// The content of a read: the committed content's leading bytes, then the
/// lines after it.
struct Content<'a> {
    base: &'a str,
    extra: &'a str,
}

impl Content<'_> {
    /// The bytes from `from` to `to`, without copying them.
    fn bytes(&self, from: usize, to: usize) -> impl Iterator<Item = u8> + '_ {
        let split = self.base.len();
        let base = self
            .base
            .as_bytes()
            .get(from.min(split)..to.min(split))
            .unwrap_or_default();
        let extra = self
            .extra
            .as_bytes()
            .get(from.max(split) - split..to.max(split) - split)
            .unwrap_or_default();
        base.iter().chain(extra).copied()
    }

    fn len(&self) -> usize {
        self.base.len() + self.extra.len()
    }

    fn get(&self, from: usize, to: usize) -> Cow<'_, str> {
        let split = self.base.len();
        if to <= split {
            Cow::Borrowed(self.base.get(from..to).unwrap_or_default())
        } else if from >= split {
            Cow::Borrowed(self.extra.get(from - split..to - split).unwrap_or_default())
        } else {
            let mut text = String::with_capacity(to - from);
            text.push_str(self.base.get(from..).unwrap_or_default());
            text.push_str(self.extra.get(..to - split).unwrap_or_default());
            Cow::Owned(text)
        }
    }
}

/// The program `extract_scripts` makes of the content: a leading part of
/// it, maybe a line break and the content from `after` on.
struct Program {
    before: usize,
    after: Option<usize>,
    section: bool,
    scripts: Option<Scripts>,
}

/// Where `content[..at]` ends without one trailing LF and then one CR
/// before it, as `extract_scripts` strips them; only its last bytes are read.
fn strip_line_break(content: &Content<'_>, at: usize) -> usize {
    let tail: Vec<u8> = content.bytes(at.saturating_sub(2), at).collect();
    match tail.as_slice() {
        [b'\r', b'\n'] => at - 2,
        [.., b'\n'] => at - 1,
        _ => at,
    }
}

impl ResponseReader {
    /// A reader for responses in the core's [UTF-16 string
    /// convention](crate::utf16_units), as [`parse_response_bundle`](super::parse_response_bundle)
    /// and [`parse_message`](crate::parse_message) take them, parsing
    /// programs with `parser` (made with the same limits).
    pub fn new(parser: StreamParser, limits: Limits) -> Self {
        Self::with_format(parser, limits, TextFormat::Core)
    }

    /// A reader for JavaScript strings as exact UTF-16 units, as
    /// [`response_draw_json_utf16`](super::response_draw_json_utf16) takes them.
    pub fn new_utf16(parser: StreamParser, limits: Limits) -> Self {
        Self::with_format(parser, limits, TextFormat::ExactUtf16)
    }

    fn with_format(parser: StreamParser, limits: Limits, format: TextFormat) -> Self {
        Self {
            format,
            limits,
            accepted: Accepted::new(),
            program: String::new(),
            origin: Origin::default(),
            parser,
            escaped: None,
            in_sync: true,
            last: Outcome {
                complete: true,
                ..Outcome::default()
            },
            staged: None,
            examined: 0,
        }
    }

    /// Stage a read of the whole response so far, `text` in the core's
    /// convention (a reader made with [`ResponseReader::new`]).
    ///
    /// Whether the text continues the last accepted one is decided by
    /// comparing it with that text: a length or a hash could take another
    /// text for an append. The caller hands over the whole text on every
    /// call, so it has already copied it once; the comparison is one more
    /// pass of the same length, at memory speed, and the rules and the
    /// parser then see only what is new.
    pub fn read(&mut self, text: &str, streaming: bool) -> Result<(), ResponseReaderError> {
        self.staged = None;
        if text.len() > self.limits.program_size() {
            return Err(ResponseBundleError::ProgramSize.into());
        }
        let old = &self.accepted.raw;
        let append = text.len() >= old.len()
            && text.as_bytes().get(..old.len()) == Some(old.as_bytes())
            && text.is_char_boundary(old.len());
        let (fresh, keep) = if append {
            (false, old.len())
        } else {
            (true, 0)
        };
        let appended = text.get(keep..).unwrap_or_default().to_owned();
        let charged = text.len();
        self.stage(fresh, keep, appended, charged, 0, None, streaming)
    }

    /// Stage a read of a whole JavaScript response, as exact UTF-16 units (a
    /// reader made with [`ResponseReader::new_utf16`]). An empty text means
    /// an absent response.
    pub fn read_utf16(
        &mut self,
        units: &[u16],
        streaming: bool,
    ) -> Result<(), ResponseReaderError> {
        self.staged = None;
        let (appended, charged, lone_high) = self.admit_units(None, units, 0)?;
        self.stage(
            true,
            0,
            appended,
            charged,
            units.len(),
            lone_high,
            streaming,
        )
    }

    /// Stage a read of a JavaScript response that is the last accepted one
    /// (`base` UTF-16 units long, as the caller checked) followed by
    /// `units`. A `base` other than the accepted length is refused as
    /// invalid: the caller must then send the whole text.
    pub fn read_utf16_append(
        &mut self,
        base: usize,
        units: &[u16],
        streaming: bool,
    ) -> Result<(), ResponseReaderError> {
        self.staged = None;
        if base != self.accepted.units {
            return Err(ResponseReaderError::NotAnAppend);
        }
        let lone = self.accepted.lone_high;
        // A chunk that starts with the low half of a pair the last text
        // ended with replaces that lone unit with the pair.
        let pairs = lone.is_some() && units.first().is_some_and(|u| (0xdc00..0xe000).contains(u));
        let (keep, charged) = if pairs {
            let escape = super::text::encode_utf16(lone.into_iter()).len();
            (
                self.accepted.raw.len() - escape,
                self.accepted.charged.saturating_sub(3),
            )
        } else {
            (self.accepted.raw.len(), self.accepted.charged)
        };
        let lead = if pairs { lone } else { None };
        let total = base.saturating_add(units.len());
        let (appended, charged, lone_high) = self.admit_units(lead, units, charged)?;
        if total > self.limits.program_size() {
            return Err(ResponseBundleError::ProgramSize.into());
        }
        let lone_high = if units.is_empty() { lone } else { lone_high };
        self.stage(false, keep, appended, charged, total, lone_high, streaming)
    }

    /// Charge units as `admitted_utf16` does: three bytes for a lone
    /// surrogate, UTF-8 bytes otherwise.
    fn admit_units(
        &mut self,
        lead: Option<u16>,
        units: &[u16],
        charged: usize,
    ) -> Result<(String, usize, Option<u16>), ResponseReaderError> {
        let room = self.limits.program_size();
        if units.len() > room {
            return Err(ResponseBundleError::ProgramSize.into());
        }
        let mut charged = charged;
        let mut last_lone = None;
        let all = lead.into_iter().chain(units.iter().copied());
        for scalar in char::decode_utf16(all.clone()) {
            let bytes = match scalar {
                Ok(ch) => {
                    last_lone = None;
                    ch.len_utf8()
                }
                Err(error) => {
                    let unit = error.unpaired_surrogate();
                    last_lone = (0xd800..0xdc00).contains(&unit).then_some(unit);
                    3
                }
            };
            charged = charged.saturating_add(bytes);
            if charged > room {
                return Err(ResponseBundleError::ProgramSize.into());
            }
        }
        self.examined += units.len() as u64;
        Ok((super::text::encode_utf16(all), charged, last_lone))
    }

    #[allow(clippy::too_many_arguments)]
    fn stage(
        &mut self,
        fresh: bool,
        raw_keep: usize,
        appended: String,
        charged: usize,
        units: usize,
        lone_high: Option<u16>,
        streaming: bool,
    ) -> Result<(), ResponseReaderError> {
        if !fresh {
            let mut examined = 0;
            let resumed =
                self.stage_resumed(raw_keep, &appended, lone_high, streaming, &mut examined);
            self.examined += examined;
            if let Some(resumed) = resumed {
                let Resumed {
                    outcome,
                    kept,
                    rest,
                    origin,
                    scan,
                } = resumed;
                self.staged = Some(Box::new(Staged {
                    fresh,
                    raw_keep,
                    appended,
                    charged,
                    units,
                    lone_high,
                    outcome,
                    kept,
                    rest,
                    origin,
                    resume: Some(scan),
                }));
                return Ok(());
            }
        }
        let empty;
        let accepted = if fresh {
            empty = Accepted::new();
            &empty
        } else {
            &self.accepted
        };
        let committed = &accepted.committed;
        let mut tail = String::new();
        tail.push_str(
            accepted
                .raw
                .get(committed.at..raw_keep.max(committed.at))
                .unwrap_or_default(),
        );
        tail.push_str(&appended);
        let format = self.format;
        let limits = self.limits;
        let tail = match format {
            TextFormat::Core => normalize_surrogate_pairs(&tail),
            TextFormat::ExactUtf16 => Cow::Borrowed(tail.as_str()),
        };
        // Copied, normalized (the core's strings) and searched once.
        let mut examined = tail.len() as u64 * if format == TextFormat::Core { 3 } else { 2 };
        // Without `]]` no marker, retry or bare scripts marker can start, and
        // only `]` alone can be a last line held back: every line is text
        // for the section it is in, as `split_message` would find.
        let quiet = !tail.contains("]]") && tail.rsplit('\n').next() != Some("]");

        // The bundle reader: bare scripts markers masked.
        let mut masked = Fold {
            part: committed.masked.part,
            content: View::of(&committed.masked.content),
            context: View::of(&committed.masked.context),
            has_context: committed.masked.has_context,
            query: Cow::Borrowed(committed.masked.query.as_ref()),
            end: committed.masked.end,
        };
        examined += tail.len() as u64;
        if quiet {
            for line in tail.split('\n') {
                masked.line(line);
            }
        } else {
            examined += tail.len() as u64 * 4;
            masked.finish(&split_retries(&mask_bare_scripts(&tail)), streaming);
        }
        let content_line = match committed.content_line {
            Some(arrived) => !streaming || arrived,
            None if quiet => false,
            None => has_content_line(&tail, streaming),
        };
        if masked.has_context {
            let context = masked.context.text();
            examined += context.len() as u64;
            if let Err(error) = format.read_json(&context, limits)
                && error.depth_limited()
            {
                return Err(ResponseBundleError::ContextDepth.into());
            }
        }

        // The scripts section, continuing the committed scan when the
        // content does.
        let content = Content {
            base: masked.content.base(),
            extra: &masked.content.extra,
        };
        let mut extract = if masked.content.reset {
            Extract::new()
        } else {
            accepted.committed.extract.clone()
        };
        let from = extract.pos.min(content.len());
        extract.advance(&content.get(from, content.len()), true, &mut examined);
        let mut program = finish_extract(&extract, &content, format, limits, &mut examined)?;
        let valid = !program.section || program.scripts.is_some();
        let (scripts, revision) = program.scripts.take().unwrap_or_default();
        let is_bundle = content_line || program.section || masked.end;

        // The message reader's context and attributes, bare markers as they are.
        let mut plain = Fold {
            part: committed.plain.part,
            content: Ignored,
            context: View::of(&committed.plain.context),
            has_context: committed.plain.has_context,
            query: Cow::Borrowed(committed.plain.query.as_ref()),
            end: committed.plain.end,
        };
        if !quiet {
            examined += tail.len() as u64 * 2;
            plain.finish(&split_retries(&tail), streaming);
        } else if plain.part == Part::Context {
            examined += tail.len() as u64;
            for line in tail.split('\n') {
                plain.line(line);
            }
        }
        let context = plain.has_context.then(|| plain.context.text());
        // The WebAssembly transport checks this context as the bundle reader
        // does; the core's message reader checks it in `message_json`.
        if let (TextFormat::ExactUtf16, Some(context)) = (format, &context) {
            examined += context.len() as u64;
            if let Err(error) = format.read_json(context, limits)
                && error.depth_limited()
            {
                return Err(ResponseBundleError::ContextDepth.into());
            }
        }

        let (kept, rest, origin, program_changed) = self.compare_program(
            fresh,
            committed,
            &content,
            &program,
            masked.content.reset,
            &mut examined,
        );

        let has_marker = committed.has_marker || tail.contains("]]>");
        let outcome = Outcome {
            name: if content_line {
                read_name(&masked.query, format)
            } else {
                None
            },
            scripts,
            revision,
            is_bundle,
            complete: valid && (masked.end || !streaming || !content_line),
            error: (!streaming && !valid).then_some(INVALID_SCRIPTS),
            context,
            query: plain.query.into_owned(),
            has_marker,
            program_changed,
        };
        self.examined += examined;
        self.staged = Some(Box::new(Staged {
            fresh,
            raw_keep,
            appended,
            charged,
            units,
            lone_high,
            outcome,
            kept,
            rest,
            origin,
            resume: None,
        }));
        Ok(())
    }

    /// Stage a read that goes on with the line still arriving from where
    /// its scan stopped (#122), or `None` when it cannot: the caller then
    /// reads the line again, as every call did before.
    ///
    /// Why the result is the full read's: the text after the committed
    /// lines is then one line `L` (no line break) with no `]]>`, longer than
    /// `]]` (see [`Resume`]). Without `]]>` neither a bare scripts marker
    /// nor a retry can be masked or split out of it, no retry is held back,
    /// it is no marker line and no content marker line, and it can only be
    /// held back as a marker that may still arrive if it is `]` or `]]`.
    /// So each fold takes it as one line of the section it is in, which the
    /// committed state says is the content, with no context in either view
    /// (so the message reader's view takes nothing from it, and there is no
    /// context to read). The content is then the committed content, a line
    /// break and `L`, and its scripts scan is the resumed one. When that
    /// scan ends in the array and the array certainly fails to read (it has
    /// not closed, or has text after it), nothing else looks at `L`, and the
    /// program is the committed content before the section. Every other case
    /// returns `None`.
    fn stage_resumed(
        &self,
        raw_keep: usize,
        appended: &str,
        lone_high: Option<u16>,
        streaming: bool,
        examined: &mut u64,
    ) -> Option<Resumed> {
        let accepted = &self.accepted;
        let Resume::Line(scan) = &accepted.resume else {
            return None;
        };
        let committed = &accepted.committed;
        let raw = &accepted.raw;
        let format = self.format;
        // Checks that cannot fail, kept to fall back rather than trust:
        // only a commit changes the committed lines, and it rebuilds or
        // replaces the scan, so `scan.line` is `committed.at`; a scan never
        // passes the accepted text's stable end, which an append keeps
        // (`raw_keep` is the accepted length, or that less the escape of a
        // trailing lone half the scan stopped before); and neither view has
        // a context in the content (see `build_resume`).
        if scan.line != committed.at
            || scan.raw_at < scan.line
            || scan.raw_at > raw_keep
            || raw_keep > raw.len()
            || committed.masked.has_context
            || committed.plain.has_context
        {
            return None;
        }
        // Needed: a line outside the content must not be taken as content.
        // `build_resume` makes no scan for such a line either; either check
        // alone suffices, and removing both makes the differential tests fail.
        if committed.masked.part != Part::Content {
            return None;
        }
        // A line of `]` or `]]` may be a marker still arriving.
        if (raw_keep - scan.line).saturating_add(appended.len()) <= 2 {
            return None;
        }
        // The new bytes, with the two before them so that a `]]>` across
        // the old end is seen. The line so far has neither a line break nor
        // `]]>` (the scan was built or carried only so).
        let mut look = scan.raw_at.saturating_sub(2).max(scan.line);
        while !raw.is_char_boundary(look) {
            // Stops by `scan.line`, a character boundary (just after a line
            // break, or the text's start).
            look -= 1;
        }
        let mut window = String::with_capacity((raw_keep - look).saturating_add(appended.len()));
        window.push_str(raw.get(look..raw_keep)?);
        window.push_str(appended);
        // Copied and searched.
        *examined += window.len() as u64 * 2;
        if window.contains('\n') || window.contains("]]>") {
            return None;
        }
        let new = window.get(scan.raw_at - look..)?;
        let text = match format {
            TextFormat::Core => {
                *examined += new.len() as u64;
                // Pairs are joined as in the whole line (the scanned part
                // never ends in a lone high surrogate), so that `pos` stays
                // a true content offset. Unobservable in any result while
                // the line is resumed: a pair is one non-ASCII character
                // either way to the scan, and nothing reads the array's
                // bytes. The same holds for leaving a trailing lone high
                // half to the next call (below), which also keeps `raw_at`
                // within a WebAssembly append that pairs it.
                normalize_surrogate_pairs(new)
            }
            TextFormat::ExactUtf16 => Cow::Borrowed(new),
        };
        let trailing = trailing_lone_high(&text, format, lone_high)?;
        let stable = text.get(..text.len().checked_sub(trailing)?)?;
        let mut extract = Extract {
            pos: scan.pos,
            stage: scan.stage,
            count: None,
            read: None,
        };
        extract.advance(stable, false, examined);
        let scanned = extract.pos.checked_sub(scan.pos)?;
        // What the scan held back is ASCII (`/`, `]`, `]]`), so it ends the
        // raw text as it ends the content, before any lone high surrogate.
        let held = stable.get(scanned..)?;
        if !held.is_ascii() || !matches!(extract.stage, Stage::Array { .. }) {
            return None;
        }
        let raw_at = raw_keep
            .checked_add(appended.len())?
            .checked_sub(trailing)?
            .checked_sub(held.len())?;
        if raw_at < scan.raw_at {
            return None;
        }
        let next = LineScan {
            line: scan.line,
            raw_at,
            pos: extract.pos,
            stage: extract.stage,
        };
        // This read's answer: the scan as if the text ended here.
        extract.advance(text.get(scanned..)?, true, examined);
        let Stage::Array { at, shape, .. } = extract.stage else {
            return None;
        };
        let body = committed.masked.content.text.as_str();
        if !shape.fails(self.limits.json_depth()) || at >= body.len() {
            return None;
        }

        // Nothing the full read could refuse: neither view has a context,
        // and the scripts marker line's JSON was read without error when
        // the scan was built.
        let content = Content {
            base: body,
            extra: "",
        };
        // The program is the content before the section, all committed.
        let program = Program {
            before: strip_line_break(&content, at),
            after: None,
            section: true,
            scripts: None,
        };
        let (kept, rest, origin, program_changed) =
            self.compare_program(false, committed, &content, &program, false, examined);
        let content_line = committed
            .content_line
            .is_some_and(|arrived| !streaming || arrived);
        let outcome = Outcome {
            name: if content_line {
                read_name(&committed.masked.query, format)
            } else {
                None
            },
            scripts: Vec::new(),
            revision: None,
            is_bundle: true,
            complete: false,
            error: (!streaming).then_some(INVALID_SCRIPTS),
            context: None,
            query: committed.plain.query.as_ref().to_owned(),
            has_marker: committed.has_marker,
            program_changed,
        };
        Some(Resumed {
            outcome,
            kept,
            rest,
            origin,
            scan: next,
        })
    }

    /// The new program, compared with the last from where they could
    /// differ: how much of the last it keeps, what follows, where its parts
    /// come from, and whether it changed.
    fn compare_program(
        &self,
        fresh: bool,
        committed: &Committed,
        content: &Content<'_>,
        program: &Program,
        reset: bool,
        examined: &mut u64,
    ) -> (usize, String, Origin, bool) {
        let base_len = if reset { 0 } else { content.base.len() };
        let origin = Origin {
            epoch: committed.epoch,
            from_base: program.before.min(base_len),
            before: program.before,
            after: program.after,
            after_base: program
                .after
                .filter(|_| program.before <= base_len)
                .map_or(0, |after| base_len.saturating_sub(after)),
        };
        let same_base = !fresh && self.origin.epoch == origin.epoch;
        let low = committed.low;
        let mut safe = if same_base {
            self.origin.from_base.min(origin.from_base).min(low)
        } else {
            0
        };
        // Past the scripts section too, when both programs' parts after the
        // section begin inside the committed content.
        if same_base
            && let Some(after) = origin.after
            && safe == origin.before
            && origin.after_base > 0
            && self.origin.after_base > 0
        {
            // Invariants, not branches: no test can reach them failing. Both
            // committed contents agree on every byte below the smaller of
            // their views' lengths; committed content only gets shorter by a
            // closing marker dropping one trailing CR (already outside that
            // view) or by a content reset, which changes the epoch; and a
            // `]]>` cannot straddle the committed content's last line break.
            // So the scripts scan saw the same bytes through `after` on both
            // sides: the same split points, and nothing below `after` plus
            // the shared part has changed. Kept as checks that fall back to
            // comparing from the section on.
            let same_split =
                self.origin.after == Some(after) && self.origin.before == origin.before;
            let shared = self.origin.after_base.min(origin.after_base);
            let unchanged = low >= after + shared;
            debug_assert!(
                same_split && unchanged,
                "the response reader's program origins disagree"
            );
            if same_split && unchanged {
                safe = origin.before + 1 + shared;
            }
        }
        let safe = safe.min(self.program.len());
        // Compare without copying, then copy only what is new.
        let before = program.before;
        let after_start = program.after.unwrap_or(content.len());
        // The new program's bytes from `safe` on: in the part before the
        // scripts section, or already in the part after it.
        let (head_from, separator, tail_from) = if safe <= before {
            (safe, program.after.is_some(), after_start)
        } else {
            (before, false, after_start + (safe - before - 1))
        };
        let new_bytes = content
            .bytes(head_from, before)
            .chain(separator.then_some(b'\n'))
            .chain(content.bytes(tail_from, content.len()));
        let old = self.program.as_bytes().get(safe..).unwrap_or_default();
        let mut common = old
            .iter()
            .zip(new_bytes)
            .take_while(|(a, b)| *a == b)
            .count();
        let program_len = before + program.after.map_or(0, |after| 1 + content.len() - after);
        // Back to a character boundary of both: their bytes agree up to it.
        let boundary = |at: usize| match self.program.as_bytes().get(at) {
            Some(&byte) => !(0x80..0xc0).contains(&byte),
            None => true,
        };
        while common > 0 && !boundary(safe + common) {
            common -= 1;
        }
        let kept = safe + common;
        let mut rest = String::new();
        if kept <= before {
            rest.push_str(&content.get(kept, before));
            if let Some(after) = program.after {
                rest.push('\n');
                rest.push_str(&content.get(after, content.len()));
            }
        } else if let Some(after) = program.after {
            rest.push_str(&content.get(after + (kept - before - 1), content.len()));
        }
        debug_assert_eq!(kept + rest.len(), program_len);
        *examined += (common + rest.len()) as u64;
        let program_changed = kept != self.program.len() || !rest.is_empty();
        (kept, rest, origin, program_changed)
    }

    /// Whether the staged read's program differs from the last accepted one.
    pub fn program_changed(&self) -> bool {
        self.outcome().program_changed
    }

    fn outcome(&self) -> &Outcome {
        self.staged
            .as_ref()
            .map_or(&self.last, |staged| &staged.outcome)
    }

    /// Finish the staged read: parse its program (when `parse`, or when it
    /// changed) and hand the result to `accept`. If `accept` succeeds the
    /// read and the parse are kept; otherwise the reader and its parser are
    /// left exactly as they were before the read. `complete` is as for
    /// [`StreamParser::try_set`].
    pub fn try_parse<T, E>(
        &mut self,
        parse: bool,
        complete: bool,
        accept: impl FnOnce(Option<ParseResult>) -> Result<T, E>,
    ) -> Result<T, E> {
        let Some(staged) = self.staged.take() else {
            return accept(None);
        };
        let output = if parse || staged.outcome.program_changed || !self.in_sync {
            if self.format == TextFormat::ExactUtf16
                && (self.escaped.is_some_and(|at| at < staged.kept)
                    || super::text::first_escape(&staged.rest).is_some())
            {
                // The parser took the program as the JavaScript driver gave
                // it before (a string, lone surrogates replaced), which only
                // a program with an escape in it tells apart: compare the
                // whole of it then.
                let mut program = self
                    .program
                    .get(..staged.kept)
                    .unwrap_or_default()
                    .to_owned();
                program.push_str(&staged.rest);
                let lossy = super::text::lossy(&program);
                self.examined += (program.len() + lossy.len()) as u64;
                self.parser
                    .try_set(&lossy, complete, |result| accept(Some(result)))
            } else if self.in_sync && self.parser.text_len() == self.program.len() {
                // The parser holds the last accepted program: give it only
                // what changed. The lengths agree, and the parser refuses an
                // account it cannot hold; both are checks on an invariant
                // this reader keeps, never on the text itself.
                let accept = |result| accept(Some(result));
                match self
                    .parser
                    .try_update(staged.kept, &staged.rest, complete, accept)
                {
                    Ok(output) => output,
                    Err(accept) => {
                        debug_assert!(false, "the response reader lost track of its parser");
                        self.reparse(&staged, complete, accept)
                    }
                }
            } else {
                // Only a program the parser took lossily (one with an escape
                // in it), a parser a caller drove itself, or a parser whose
                // program-size limit cut the program leaves it holding
                // another length.
                debug_assert!(
                    self.escaped.is_some() || !self.in_sync || self.parser.was_cut(),
                    "the response reader lost track of its parser"
                );
                self.reparse(&staged, complete, |result| accept(Some(result)))
            }
        } else {
            accept(None)
        };
        match output {
            Ok(output) => {
                self.commit(*staged);
                self.in_sync = true;
                Ok(output)
            }
            Err(error) => Err(error),
        }
    }

    /// Parse the staged program whole, compared byte for byte with what the
    /// parser holds: after a program the parser took lossily (an escape in
    /// it), and the way back, never silent, should the reader's account of
    /// its parser ever not hold.
    fn reparse<T, E>(
        &mut self,
        staged: &Staged,
        complete: bool,
        accept: impl FnOnce(ParseResult) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut program = self
            .program
            .get(..staged.kept)
            .unwrap_or_default()
            .to_owned();
        program.push_str(&staged.rest);
        self.examined += program.len() as u64;
        self.parser.try_set(&program, complete, accept)
    }

    /// Keep a staged read: the accepted text, the committed lines and the
    /// program move on.
    fn commit(&mut self, staged: Staged) {
        let Staged {
            fresh,
            raw_keep,
            appended,
            charged,
            units,
            lone_high,
            outcome,
            kept,
            rest,
            origin,
            resume,
        } = staged;
        if fresh {
            self.accepted = Accepted::new();
        }
        let accepted = &mut self.accepted;
        accepted.raw.truncate(raw_keep);
        accepted.raw.push_str(&appended);
        accepted.charged = charged;
        accepted.units = units;
        accepted.lone_high = lone_high;
        self.program.truncate(kept);
        self.program.push_str(&rest);
        if self.format == TextFormat::ExactUtf16 && !self.escaped.is_some_and(|at| at < kept) {
            self.escaped = super::text::first_escape(&rest).map(|at| kept + at);
        }
        self.origin = origin;
        self.last = outcome;
        accepted.committed.low = accepted.committed.masked.content.text.len();
        match resume {
            // A resumed read leaves the committed lines as they were: the
            // text after them is still one line, so `advance` would find no
            // line to commit and its scan nothing new to look at.
            Some(scan) => accepted.resume = Resume::Line(scan),
            None => {
                self.examined += advance(
                    &mut accepted.committed,
                    &accepted.raw,
                    self.format,
                    self.limits,
                );
                let line = accepted.committed.at;
                if !matches!(accepted.resume, Resume::Never { line: never } if never == line) {
                    let mut examined = 0;
                    accepted.resume =
                        build_resume(accepted, self.format, self.limits, &mut examined);
                    self.examined += examined;
                }
            }
        }
    }

    /// Drop a staged read without keeping it: the reader stays at the last
    /// accepted text.
    pub fn discard(&mut self) {
        self.staged = None;
    }

    /// The last accepted read's program.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The parser, for a caller that also pushes or sets text itself (a
    /// stream session that reads responses too). The next read then
    /// compares its program whole with whatever the parser holds.
    pub fn parser_mut(&mut self) -> &mut StreamParser {
        self.staged = None;
        self.in_sync = false;
        &mut self.parser
    }

    /// The parser's latest result, without new text ([`StreamParser::result`]).
    pub fn result(&mut self) -> ParseResult {
        self.parser.result()
    }

    /// The parser's result once the stream has ended, after the last read
    /// ([`StreamParser::finish`]).
    pub fn finish(&mut self) -> ParseResult {
        self.parser.finish()
    }

    /// The last read's response bundle as JSON, exactly as
    /// [`ResponseBundle::to_json`](super::ResponseBundle::to_json) writes it
    /// for the same text, program included.
    pub fn bundle_json(&self) -> String {
        let outcome = self.outcome();
        let program = match &self.staged {
            Some(staged) => {
                let mut program = self
                    .program
                    .get(..staged.kept)
                    .unwrap_or_default()
                    .to_owned();
                program.push_str(&staged.rest);
                Cow::Owned(program)
            }
            None => Cow::Borrowed(self.program.as_str()),
        };
        self.write(Some(&program), outcome)
    }

    fn write(&self, program: Option<&str>, outcome: &Outcome) -> String {
        write_bundle(
            &BundleParts {
                program,
                name: outcome.name.as_deref(),
                scripts: &outcome.scripts,
                complete: outcome.complete,
                is_bundle: outcome.is_bundle,
                error: outcome.error,
                revision: outcome.revision.as_deref(),
            },
            self.format,
        )
    }

    /// The last read's message context and attributes as JSON, as
    /// [`ParsedMessage::to_json`](crate::ParsedMessage::to_json) writes them
    /// (`{"context":…,"attributes":{…}}`, no content), or `None` when the
    /// text has no marker prefix (`]]>`) at all. Refuses a context nested
    /// past the JSON depth limit as [`parse_message`](crate::parse_message) does.
    pub fn message_json(&self) -> Result<Option<String>, ResponseReaderError> {
        let outcome = self.outcome();
        if !outcome.has_marker {
            return Ok(None);
        }
        let context = match &outcome.context {
            None => Value::Null,
            Some(text) => match crate::js::read_message_json(text, self.limits) {
                Ok(value) => value,
                Err(error) if error.depth_limited() => {
                    return Err(ResponseReaderError::Message(MessageError::ContextDepth));
                }
                Err(_) => Value::Str(text.clone()),
            },
        };
        let mut out = String::from("{\"context\":");
        crate::result::write_value(&mut out, &context);
        out.push_str(",\"attributes\":{");
        for (i, (key, value)) in read_attributes(&outcome.query).iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            crate::js::write_json_string(&mut out, key);
            out.push(':');
            crate::js::write_json_string(&mut out, value);
        }
        out.push_str("}}");
        Ok(Some(out))
    }

    /// The last read for the WebAssembly driver, as
    /// [`response_draw_json_utf16`](super::response_draw_json_utf16) writes
    /// it but without the program, which stays here for the parser:
    /// `{"bundle":…,"context":…,"attributes":{…},"programChanged":…}`.
    pub fn draw_json(&self) -> String {
        let outcome = self.outcome();
        let format = self.format;
        let mut out = String::from("{\"bundle\":");
        out.push_str(&self.write(None, outcome));
        out.push_str(",\"context\":");
        match &outcome.context {
            Some(context) => format.write_json(&mut out, context),
            None => out.push_str("null"),
        }
        out.push_str(",\"attributes\":{");
        let mut first = true;
        for pair in outcome.query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = format.attribute(key);
            if key.is_empty() || key == "__proto__" {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            format.write_json(&mut out, &key);
            out.push(':');
            format.write_json(&mut out, &format.attribute(value));
        }
        out.push_str(if outcome.program_changed {
            "},\"programChanged\":true}"
        } else {
            "},\"programChanged\":false}"
        });
        out
    }

    /// Bytes the reader has looked at since it was made: an operation count
    /// for tests of its cost, which must grow about linearly with a stream.
    #[doc(hidden)]
    pub fn bytes_examined(&self) -> u64 {
        self.examined
    }

    /// Its parser's operation count ([`StreamParser::bytes_examined`]).
    #[doc(hidden)]
    pub fn parser_bytes_examined(&self) -> u64 {
        self.parser.bytes_examined()
    }
}

/// The program, scripts and validity `extract_scripts` gives the content,
/// from what the scan found.
fn finish_extract(
    extract: &Extract,
    content: &Content<'_>,
    format: TextFormat,
    limits: Limits,
    examined: &mut u64,
) -> Result<Program, ResponseBundleError> {
    let whole = content.len();
    let plain = Program {
        before: whole,
        after: None,
        section: false,
        scripts: None,
    };
    let before = |at: usize| strip_line_break(content, at);
    let (at, newline, end, shape) = match extract.stage {
        Stage::Seek | Stage::Seeking(_) | Stage::Marker { .. } | Stage::Other => return Ok(plain),
        Stage::Count { at } => {
            return Ok(Program {
                before: before(at),
                after: None,
                section: true,
                scripts: None,
            });
        }
        Stage::Array {
            at, newline, shape, ..
        } => (at, newline, whole, shape),
        Stage::Closed {
            at,
            newline,
            end,
            shape,
        } => (at, newline, end, shape),
    };
    let count_json = content.get(at + SCRIPTS.len(), newline);
    let scripts = if shape.fails(limits.json_depth()) {
        // The array cannot be read: only the marker line's depth matters.
        let count = match &extract.count {
            Some(count) => count.clone(),
            None => {
                *examined += count_json.len() as u64;
                script_json(&count_json, limits, format)
            }
        };
        count.map(|_| None)?
    } else if let Some((to, read)) = extract.read.as_ref().filter(|(to, _)| *to <= end) {
        let _ = to;
        Result::clone(read)?
    } else {
        let scripts_json = content.get(newline + 1, end);
        *examined += (count_json.len() + scripts_json.len()) as u64;
        read_scripts(&count_json, &scripts_json, limits, format)?
    };
    Ok(Program {
        before: before(at),
        after: (end < whole).then_some(end),
        section: true,
        scripts,
    })
}

/// How many bytes at the end of `text` (the line still arriving, as far as
/// it is new) the resumable scan must leave for the next call: a lone high
/// surrogate the next chunk may pair, which then changes in both forms (the
/// core joins the pair, the WebAssembly reader replaces its escape). `None`
/// when a WebAssembly text does not end in the escape it says it ends in.
fn trailing_lone_high(text: &str, format: TextFormat, lone_high: Option<u16>) -> Option<usize> {
    match format {
        TextFormat::Core => Some(match text.chars().next_back() {
            Some(ch) if lone_surrogate_unit(ch).is_some_and(|unit| unit < 0xdc00) => ch.len_utf8(),
            _ => 0,
        }),
        TextFormat::ExactUtf16 => match lone_high {
            None => Some(0),
            Some(unit) => {
                let escape = super::text::encode_utf16(std::iter::once(unit));
                text.ends_with(&escape).then_some(escape.len())
            }
        },
    }
}

/// The scan of the line still arriving after the committed lines, for the
/// next append to resume (see [`Resume`]), read once in full. Charged to
/// `examined`.
fn build_resume(
    accepted: &Accepted,
    format: TextFormat,
    limits: Limits,
    examined: &mut u64,
) -> Resume {
    let committed = &accepted.committed;
    let line = committed.at;
    let never = Resume::Never { line };
    // Where the committed lines leave the line: in the content, at a
    // scripts section that has not closed, with no context in either view.
    // The content condition is needed (it is checked again in
    // `stage_resumed`; either check alone suffices, and removing both makes
    // the differential tests fail). The context conditions cannot fail: a
    // context marker takes both views out of the content and sets their
    // contexts, and only a content marker, which clears both, brings them
    // back; so in the content neither has one. They are kept as checks that
    // fall back. None of this changes until a line is committed.
    if !matches!(
        committed.extract.stage,
        Stage::Count { .. } | Stage::Array { .. }
    ) || committed.masked.part != Part::Content
        || committed.masked.has_context
        || committed.plain.has_context
    {
        return never;
    }
    let Some(partial) = accepted.raw.get(line..) else {
        return never;
    };
    *examined += partial.len() as u64 * 2;
    // Appends keep a line break or `]]>` (a WebAssembly pair replaces only
    // the escape of a lone half, neither of them).
    if partial.contains('\n') || partial.contains("]]>") {
        return never;
    }
    let text = match format {
        TextFormat::Core => {
            *examined += partial.len() as u64;
            normalize_surrogate_pairs(partial)
        }
        TextFormat::ExactUtf16 => Cow::Borrowed(partial),
    };
    let body = committed.masked.content.text.as_str();
    let from = committed.extract.pos.min(body.len());
    let (Some(trailing), Some(unscanned)) = (
        trailing_lone_high(&text, format, accepted.lone_high),
        body.get(from..),
    ) else {
        return never;
    };
    let Some(stable) = text
        .len()
        .checked_sub(trailing)
        .and_then(|end| text.get(..end))
    else {
        return never;
    };
    // The content from the committed scan on: what it left of the committed
    // content (held-back characters, trailing CRs), the line break the line
    // follows, and the line.
    let mut fed = String::with_capacity(unscanned.len() + 1 + stable.len());
    fed.push_str(unscanned);
    if committed.masked.content.has_line {
        fed.push('\n');
    }
    fed.push_str(stable);
    let mut extract = Extract {
        pos: from,
        stage: committed.extract.stage,
        count: None,
        read: None,
    };
    extract.advance(&fed, false, examined);
    let Stage::Array { at, newline, .. } = extract.stage else {
        return never;
    };
    // The scan holds back at most `/`, `]` or `]]`, ASCII at the line's end.
    let held = from
        .checked_add(fed.len())
        .and_then(|end| end.checked_sub(extract.pos));
    let raw_at = held.and_then(|held| {
        let tail = stable.get(stable.len().checked_sub(held)?..)?;
        let raw_at = accepted
            .raw
            .len()
            .checked_sub(trailing)?
            .checked_sub(held)?;
        (tail.is_ascii() && raw_at >= line).then_some(raw_at)
    });
    let Some(raw_at) = raw_at else {
        return never;
    };
    // The scripts marker line's JSON, which the full read reads whenever
    // its scan is in the array. Needed: the accepted text's own read may not
    // have read it, when its last line was a held-back `]` or `]]` (no line
    // break after the marker line yet, so its scan stopped at the marker).
    // The next append would then make the full read refuse the JSON as too
    // deep, so no scan is built and that read takes the full path
    // (`a_marker_line_too_deep_is_refused_after_a_held_back_bracket`).
    let count = match &committed.extract.count {
        Some(count) => count.as_ref().map(|_| ()).map_err(|error| *error),
        None => body
            .get(at + SCRIPTS.len()..newline)
            .ok_or(ResponseBundleError::ScriptsDepth)
            .and_then(|json| {
                *examined += json.len() as u64;
                script_json(json, limits, format).map(|_| ())
            }),
    };
    if count.is_err() {
        return never;
    }
    Resume::Line(LineScan {
        line,
        raw_at,
        pos: extract.pos,
        stage: extract.stage,
    })
}

/// Commit the complete lines after `committed.at` in `raw` that no later
/// text can change. Returns the bytes looked at.
fn advance(committed: &mut Committed, raw: &str, format: TextFormat, limits: Limits) -> u64 {
    let mut examined = 0u64;
    let first = committed.at;
    let region = raw.get(first..).unwrap_or_default();
    let breaks: Vec<usize> = region.match_indices('\n').map(|(at, _)| at).collect();
    let mut start = 0;
    for (index, &newline) in breaks.iter().enumerate() {
        let Some(line) = region.get(start..=newline) else {
            break;
        };
        let line = match format {
            TextFormat::Core => normalize_surrogate_pairs(line),
            TextFormat::ExactUtf16 => Cow::Borrowed(line),
        };
        examined += line.len() as u64 * if format == TextFormat::Core { 2 } else { 1 };
        if !line.contains("]]") {
            // No marker can start in it: one line of text for each reader.
            examined += line.len() as u64 * 2;
            let text = line.strip_suffix('\n').unwrap_or(&line);
            committed.masked.line(text);
            committed.low = committed.low.min(committed.masked.content.text.len());
            committed.plain.line(text);
            start = newline + 1;
            committed.at = first + start;
            continue;
        }
        examined += line.len() as u64 * 6;
        let retries = split_retries(&line);
        let ends = lines_of(&retries).any(|line| matches!(marker(line), Some(("end", _))));
        if ends && breaks.len() - index - 1 < 2 {
            break;
        }
        if committed.content_line.is_none() && has_content_line(&line, false) {
            committed.content_line = Some(has_content_line(&line, true));
        }
        committed.has_marker |= line.contains("]]>");
        let masked = split_retries(&mask_bare_scripts(&line)).into_owned();
        for piece in lines_of(&masked) {
            if committed.masked.line(piece) == Event::Reset {
                committed.epoch += 1;
                committed.extract = Extract::new();
            }
            committed.low = committed.low.min(committed.masked.content.text.len());
        }
        for piece in lines_of(&retries) {
            committed.plain.line(piece);
        }
        start = newline + 1;
        committed.at = first + start;
    }
    // Scan the committed content as far as no later line can change it: a
    // closing marker can take back the CRs it ends with.
    let content = &committed.masked.content.text;
    let limit = content.trim_end_matches('\r').len();
    let extract = &mut committed.extract;
    let from = extract.pos.min(limit);
    extract.advance(
        content.get(from..limit).unwrap_or_default(),
        false,
        &mut examined,
    );
    remember(extract, content, limit, format, limits, &mut examined);
    examined
}

/// Read once what the committed content has made final: the scripts
/// marker line's JSON, and the array once it closed.
fn remember(
    extract: &mut Extract,
    content: &str,
    limit: usize,
    format: TextFormat,
    limits: Limits,
    examined: &mut u64,
) {
    let (at, newline, to, shape) = match extract.stage {
        Stage::Array {
            at, newline, shape, ..
        } => (at, newline, extract.pos, shape),
        Stage::Closed {
            at,
            newline,
            end,
            shape,
        } => (at, newline, end, shape),
        _ => return,
    };
    let count_json = content.get(at + SCRIPTS.len()..newline).unwrap_or_default();
    if extract.count.is_none() {
        *examined += count_json.len() as u64;
        extract.count = Some(script_json(count_json, limits, format));
    }
    if extract.read.is_none()
        && !shape.fails(limits.json_depth())
        && shape.closed
        && to <= limit
        && let Some(scripts_json) = content.get(newline + 1..to)
    {
        *examined += scripts_json.len() as u64;
        extract.read = Some((
            to,
            std::sync::Arc::new(read_scripts(count_json, scripts_json, limits, format)),
        ));
    }
}
