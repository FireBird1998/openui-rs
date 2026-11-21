//! Streaming parsing, as the reference parser's `createStreamParser`.
//!
//! Statements that end in a newline outside brackets are parsed once and
//! cached. The trailing statement is still arriving, so it's auto-closed and
//! re-parsed on every push and laid over a copy of the cache.
//!
//! The reference re-derives the cleaned text from the whole buffer on every
//! push and rebuilds the whole tree; here a [`Preprocessor`] extends the
//! cleaned text chunk by chunk, a [`Cache`] keeps the materialized
//! statements that nothing changed under, and a [`Reuse`] the evaluated
//! state defaults, with the same result.

use std::sync::Arc;

use crate::evaluate::Reuse;
use crate::expr::parse_expression_with_limit;
use crate::js;
use crate::lexer::{skip_while, tokenize};
use crate::library::Library;
use crate::limits::Limits;
use crate::parser::{Against, PartialRef, build_result, classify};
use crate::preprocess::{Preprocessor, PreprocessorMark};
use crate::progress;
use crate::result::ParseResult;
use crate::statements::{auto_close, split};
use crate::table::{Cache, Slot, Table, Undo};

/// Parses a program chunk by chunk as it streams in.
pub struct StreamParser {
    library: Arc<Library>,
    root_name: Option<String>,
    limits: Limits,
    /// Everything pushed so far.
    buf: String,
    /// `buf` preprocessed; statement boundaries are found in this, never in `buf`.
    pre: Preprocessor,
    completed: Completed,
    /// What evaluating the state defaults found at the last push.
    reuse: Reuse,
    /// The result a stall cut the stream at (in a release build), given
    /// again until [`StreamParser::set`] starts over.
    stalled: Option<ParseResult>,
    /// The text reached the program-size limit: the rest was left out, and
    /// later pushes are too, until [`StreamParser::set`] starts over.
    past_program_size: bool,
    /// The open staged update, if any.
    stage: Option<Box<Stage>>,
    /// Bytes taken in, for tests of what a stream costs.
    absorbed: u64,
    /// Bytes looked at again for each result (the statement still arriving).
    examined: u64,
}

/// The statements that have fully arrived.
#[derive(Default)]
struct Completed {
    /// How far into the cleaned text they reach.
    end: usize,
    table: Table,
    cache: Cache,
    /// Statements parsed, counting re-definitions.
    count: usize,
    /// Some statement nested past the depth limit.
    depth_limited: bool,
    /// While a staged update is open: the definitions it made, to undo.
    undos: Option<Vec<Undo>>,
}

/// Where a staged update began ([`StreamParser::try_update`]): what it needs
/// to put the parser back exactly as it was if the update is refused. The
/// large parts (the text, the cleaned text, the table and the cache) are not
/// copied: they keep a journal of what the update changed instead, so staging
/// costs about what the update itself does.
struct Stage {
    /// The text's length once the update took back its line breaks, and
    /// those line breaks: a rollback truncates to it and puts them back.
    buf: usize,
    taken: String,
    pre: PreprocessorMark,
    /// The completed statements, if the update threw them away (the cleaned
    /// text changed under them); otherwise their journals are open.
    replaced: Option<Completed>,
    end: usize,
    count: usize,
    depth_limited: bool,
    reuse: Reuse,
    past_program_size: bool,
}

impl Completed {
    fn clear(&mut self) {
        *self = Completed::default();
    }

    fn mark(&mut self) {
        self.table.mark();
        self.cache.mark_journal();
        self.undos = Some(Vec::new());
    }

    fn commit(&mut self) {
        self.table.commit();
        self.cache.commit();
        self.undos = None;
    }

    fn rollback(&mut self) {
        self.table.rollback_counts();
        for undo in self.undos.take().into_iter().flatten().rev() {
            self.table.undo(undo);
        }
        self.table.rollback_rest();
        self.cache.rollback();
    }

    /// Parse and define `text`'s statements, noting each slot defined in
    /// `changed`: the caller drops what's cached over them, once for them all
    /// ([`Cache::invalidate`]).
    fn add(&mut self, text: &str, limits: Limits, changed: &mut Vec<Slot>) {
        let text = js::trim(text);
        if text.is_empty() {
            return;
        }
        for raw in split(&tokenize(text)) {
            if progress::stalled_so_far() {
                break;
            }
            let (expr, limited) = parse_expression_with_limit(&raw.tokens, limits.depth());
            self.depth_limited |= limited;
            let undo = self.table.define(&raw.id, classify(&raw, expr));
            changed.push(undo.slot);
            if let Some(undos) = &mut self.undos {
                undos.push(undo);
            }
            self.count += 1;
        }
    }
}

impl StreamParser {
    /// A parser for one stream. `root_name` is the library's root component:
    /// with no `root` statement, the first statement that calls it is the
    /// entry.
    pub fn new(library: Arc<Library>, root_name: Option<&str>) -> StreamParser {
        StreamParser::with_limits(library, root_name, Limits::DEFAULT)
    }

    /// [`StreamParser::new`] within `limits` instead of the defaults, for
    /// every push until the stream is dropped.
    pub fn with_limits(
        library: Arc<Library>,
        root_name: Option<&str>,
        limits: Limits,
    ) -> StreamParser {
        StreamParser {
            library,
            root_name: root_name.map(str::to_owned),
            limits,
            buf: String::new(),
            pre: Preprocessor::new(),
            completed: Completed::default(),
            reuse: Reuse::for_limits(&limits),
            stalled: None,
            past_program_size: false,
            stage: None,
            absorbed: 0,
            examined: 0,
        }
    }

    /// Bytes this parser has taken in since it was made, counting text
    /// parsed afresh again: an operation count for cost tests.
    #[doc(hidden)]
    pub fn bytes_absorbed(&self) -> u64 {
        self.absorbed
    }

    /// [`StreamParser::bytes_absorbed`] plus the bytes of the statement
    /// still arriving, which every result closes, tokenizes and splits
    /// again: the parser's own operation count for cost tests.
    #[doc(hidden)]
    pub fn bytes_examined(&self) -> u64 {
        self.absorbed + self.examined
    }

    /// Feed the next chunk and get the latest parse result.
    ///
    /// A stream cut at the program-size limit (see [`Limits::program_size`])
    /// or by a stall (see [`ParseResult::size_limited`]) ignores further
    /// pushes and gives the cut result again, until [`StreamParser::set`].
    pub fn push(&mut self, chunk: &str) -> ParseResult {
        self.watched(|parser| {
            parser.absorb(chunk);
            parser.current_result(false)
        })
    }

    /// Give the full text so far, as after a reconnect. Text that continues
    /// what was pushed only parses the new part; anything else starts over,
    /// as does a stream that was cut.
    pub fn set(&mut self, full_text: &str) -> ParseResult {
        if self.past_program_size
            || self.stalled.is_some()
            || !full_text.starts_with(self.buf.as_str())
        {
            self.reset();
        }
        self.watched(|parser| {
            parser.absorb(&full_text[parser.buf.len()..]);
            parser.current_result(false)
        })
    }

    /// Parse a replacement and publish its caches only if the consumer accepts
    /// the result. In particular a refused edit cannot change the identity of
    /// a completed container default on the next push.
    pub fn try_set<T, E>(
        &mut self,
        text: &str,
        complete: bool,
        accept: impl FnOnce(ParseResult) -> Result<T, E>,
    ) -> Result<T, E> {
        // Either way `kept` is a character boundary within the text.
        let kept = if text.starts_with(self.buf.as_str()) {
            self.buf.len()
        } else {
            0
        };
        self.update(kept, &text[kept..], complete, accept)
    }

    /// [`StreamParser::try_set`] for a text the caller already knows starts
    /// with the first `kept` bytes of the text this parser last took: the new
    /// text is those bytes and then `rest`. Nothing is compared, so a stream
    /// of small appends costs what pushing them does.
    ///
    /// The update continues from where the parser is when `kept` is all of
    /// the old text, or when all it drops is line breaks (CR or LF) at the
    /// end, which a response reader takes back when a marker line starts to
    /// arrive. Anything else parses the new text afresh. Either way the
    /// parser is left exactly as it was unless `accept` takes the result.
    pub(crate) fn try_update<T, E, F>(
        &mut self,
        kept: usize,
        rest: &str,
        complete: bool,
        accept: F,
    ) -> Result<Result<T, E>, F>
    where
        F: FnOnce(ParseResult) -> Result<T, E>,
    {
        // The caller's account of the text cannot hold: hand `accept` back
        // rather than guess at what the new text is.
        if kept > self.buf.len() || !self.buf.is_char_boundary(kept) {
            return Err(accept);
        }
        Ok(self.update(kept, rest, complete, accept))
    }

    /// [`StreamParser::try_update`] once `kept` is known to be a character
    /// boundary within the text.
    fn update<T, E>(
        &mut self,
        kept: usize,
        rest: &str,
        complete: bool,
        accept: impl FnOnce(ParseResult) -> Result<T, E>,
    ) -> Result<T, E> {
        let dropped = &self.buf.as_bytes()[kept..];
        let continues = !self.buf.is_empty()
            && !self.past_program_size
            && self.stalled.is_none()
            && dropped.iter().all(|b| matches!(b, b'\r' | b'\n'));
        if !continues {
            let mut text = String::with_capacity(kept.saturating_add(rest.len()));
            text.push_str(&self.buf[..kept]);
            text.push_str(rest);
            return self.try_afresh(&text, complete, accept);
        }
        self.begin_stage();
        let dropped = self.buf.len() - kept;
        if dropped > 0
            && let Some(stage) = &mut self.stage
        {
            stage.taken = self.buf[kept..].to_owned();
            stage.buf = kept;
        }
        let result = self.watched(|parser| {
            if dropped > 0 {
                parser.buf.truncate(kept);
                parser.pre.unpush_line_breaks(dropped);
            }
            parser.absorb(rest);
            parser.current_result(complete)
        });
        match accept(result) {
            Ok(output) => {
                self.commit_stage();
                Ok(output)
            }
            Err(error) => {
                self.rollback_stage();
                Err(error)
            }
        }
    }

    /// Parse `text` with a new parser, which replaces this one only if
    /// `accept` takes the result.
    fn try_afresh<T, E>(
        &mut self,
        text: &str,
        complete: bool,
        accept: impl FnOnce(ParseResult) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut next =
            Self::with_limits(self.library.clone(), self.root_name.as_deref(), self.limits);
        next.absorbed = self.absorbed;
        next.examined = self.examined;
        let result = next.watched(|parser| {
            parser.absorb(text);
            parser.current_result(complete)
        });
        let output = accept(result)?;
        *self = next;
        Ok(output)
    }

    /// The length of the text this parser last took.
    pub(crate) fn text_len(&self) -> usize {
        self.buf.len()
    }

    /// The text reached the program-size limit and the rest was left out.
    pub(crate) fn was_cut(&self) -> bool {
        self.past_program_size
    }

    fn begin_stage(&mut self) {
        self.completed.mark();
        self.stage = Some(Box::new(Stage {
            buf: self.buf.len(),
            taken: String::new(),
            pre: self.pre.mark(),
            replaced: None,
            end: self.completed.end,
            count: self.completed.count,
            depth_limited: self.completed.depth_limited,
            reuse: self.reuse.clone(),
            past_program_size: self.past_program_size,
        }));
    }

    fn commit_stage(&mut self) {
        if let Some(stage) = self.stage.take()
            && let Some(mut replaced) = stage.replaced
        {
            // The update started the table over. The old one's journals
            // hold nothing, but its staged counts are dropped with it.
            replaced.commit();
        }
        self.completed.commit();
    }

    fn rollback_stage(&mut self) {
        let Some(stage) = self.stage.take() else {
            return;
        };
        let Stage {
            buf,
            taken,
            pre,
            replaced,
            end,
            count,
            depth_limited,
            reuse,
            past_program_size,
        } = *stage;
        if let Some(replaced) = replaced {
            self.completed = replaced;
        }
        self.completed.rollback();
        self.completed.end = end;
        self.completed.count = count;
        self.completed.depth_limited = depth_limited;
        self.buf.truncate(buf);
        self.buf.push_str(&taken);
        self.pre.restore(pre);
        self.reuse = reuse;
        self.stalled = None;
        self.past_program_size = past_program_size;
    }

    /// The latest parse result, without new text.
    pub fn result(&mut self) -> ParseResult {
        self.watched(|parser| parser.current_result(false))
    }

    /// The result once the stream has ended, after its last chunk: the
    /// reference's `getResult(true)`. Until then, a `$name` at the very end of
    /// the text may still be growing (`$d` of `$dishes`), so the pending
    /// statements don't declare it as state; once the stream has ended, it's
    /// the whole name.
    pub fn finish(&mut self) -> ParseResult {
        self.watched(|parser| parser.current_result(true))
    }

    /// `step`'s result, unless a stall has cut the stream: then the result
    /// it was cut at. A stall in `step` cuts it there. A stream cut at the
    /// program-size limit says so on every result.
    fn watched(&mut self, step: impl FnOnce(&mut Self) -> ParseResult) -> ParseResult {
        let _ceiling = crate::stack::limit(self.limits.depth());
        if let Some(cut) = &self.stalled {
            return cut.clone();
        }
        let (mut result, stalled) = progress::watch(|| step(self));
        result.size_limited |= self.past_program_size;
        if stalled {
            result = ParseResult::empty(true);
            result.size_limited = true;
            self.stalled = Some(result.clone());
        }
        result
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.pre.reset();
        self.completed.clear();
        self.reuse = Reuse::for_limits(&self.limits);
        self.stalled = None;
        self.past_program_size = false;
    }

    /// Take in new text, up to the program-size limit. If the cleaned text
    /// changed under the completed statements (an opening fence appeared and
    /// shifted everything, say), they're stale: start over, as the reference
    /// does.
    fn absorb(&mut self, chunk: &str) {
        if self.past_program_size {
            return;
        }
        let room = self.limits.program_size().saturating_sub(self.buf.len());
        let (chunk, cut) = Limits::fit(chunk, room);
        self.absorbed += chunk.len() as u64;
        self.past_program_size = cut;
        self.buf.push_str(chunk);
        let unchanged = self.pre.push(chunk);
        // The preprocessor takes its last line through both comment stages again.
        self.examined += self.pre.last_line_len() as u64 * 2;
        if self.completed.end > unchanged {
            match &mut self.stage {
                Some(stage) if stage.replaced.is_none() => {
                    stage.replaced = Some(std::mem::take(&mut self.completed));
                }
                _ => self.completed.clear(),
            }
        }
    }

    /// Cache statements completed since the last scan. Returns where the
    /// pending (still incomplete) statement starts.
    fn scan_new_completed(&mut self) -> usize {
        let cleaned = self.pre.cleaned();
        let s = cleaned.as_bytes();
        let mut depth: i32 = 0;
        let mut ternary_depth: i32 = 0;
        let mut in_str: Option<u8> = None;
        let mut esc = false;
        let mut stmt_start = self.completed.end;
        let mut completed = Vec::new();
        // Where the blank space the last line break looked past ends: a
        // line break inside it ends at the same place, without looking again.
        let mut blank_end = 0;

        for i in self.completed.end..s.len() {
            let c = s[i];
            if esc {
                esc = false;
                continue;
            }
            if c == b'\\' && in_str.is_some() {
                esc = true;
                continue;
            }
            if let Some(quote) = in_str {
                if c == quote {
                    in_str = None;
                }
                continue;
            }
            match c {
                b'"' | b'\'' => in_str = Some(c),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = (depth - 1).max(0),
                b'?' if depth == 0 => ternary_depth += 1,
                b':' if depth == 0 && ternary_depth > 0 => ternary_depth -= 1,
                b'\n' if depth <= 0 && ternary_depth <= 0 => {
                    // A next line starting with `?` continues it.
                    if blank_end <= i {
                        blank_end = skip_while(s, i, |b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
                    }
                    if let Some(&next) = s.get(blank_end)
                        && next == b'?'
                    {
                        continue;
                    }
                    completed.push(stmt_start..i);
                    self.completed.end = i + 1;
                    stmt_start = self.completed.end;
                }
                _ => {}
            }
        }

        let mut changed = Vec::new();
        for range in completed {
            self.completed
                .add(&cleaned[range], self.limits, &mut changed);
        }
        self.completed
            .cache
            .invalidate(&changed, &self.completed.table);
        stmt_start
    }

    fn current_result(&mut self, complete: bool) -> ParseResult {
        crate::stack::grow_stack!(self.current_result(complete), else ParseResult::default());
        let (mut result, pending_limited) = self.build_current(complete);
        result.depth_limited |= self.completed.depth_limited || pending_limited;
        result
    }

    /// The result, and whether a pending statement nested past the depth limit.
    fn build_current(&mut self, complete: bool) -> (ParseResult, bool) {
        let pending_start = self.scan_new_completed();
        let pending = js::trim(&self.pre.cleaned()[pending_start..]);
        // Closed, tokenized and split again on every result.
        self.examined += pending.len() as u64 * 3;
        let completed = &mut self.completed;

        if pending.is_empty() {
            if completed.count == 0 {
                return (ParseResult::empty(true), false);
            }
            let count = completed.count;
            return (self.build_from_table(false, count, None), false);
        }

        let (closed, was_incomplete) = auto_close(pending);
        let pending_statements = split(&tokenize(&closed));
        if pending_statements.is_empty() {
            if completed.count == 0 {
                return (ParseResult::empty(was_incomplete), false);
            }
            let count = completed.count;
            return (self.build_from_table(was_incomplete, count, None), false);
        }

        // A new id renders as it arrives; a re-defined one replaces the cached
        // statement only once it no longer needs closing. The pending
        // statements are laid over the table for this result only, so which
        // ids are completed is decided before any of them goes on.
        let keep: Vec<bool> = pending_statements
            .iter()
            .map(|raw| !(was_incomplete && completed.table.by_name(&raw.id).is_some()))
            .collect();
        let mut pending_limited = false;
        let mut undos = Vec::new();
        for (raw, &keep) in pending_statements.iter().zip(&keep) {
            if !keep {
                continue;
            }
            if progress::stalled_so_far() {
                break;
            }
            let (expr, limited) = parse_expression_with_limit(&raw.tokens, self.limits.depth());
            pending_limited |= limited;
            undos.push(completed.table.define(&raw.id, classify(raw, expr)));
        }
        let pending_slots: Vec<Slot> = undos.iter().map(|undo| undo.slot).collect();
        completed.cache.invalidate(&pending_slots, &completed.table);
        let count = completed.count + pending_statements.len();
        let growing = if complete {
            None
        } else {
            growing_state_ref(&self.buf, pending).map(str::to_owned)
        };
        let partial = growing.as_deref().map(|name| PartialRef {
            name,
            slots: &pending_slots,
        });
        let result = self.build_from_table(was_incomplete, count, partial);
        for undo in undos.into_iter().rev() {
            self.completed.table.undo(undo);
        }
        self.completed
            .cache
            .invalidate(&pending_slots, &self.completed.table);
        (result, pending_limited)
    }

    /// Build the result from the table as it stands.
    fn build_from_table(
        &mut self,
        was_incomplete: bool,
        count: usize,
        partial: Option<PartialRef>,
    ) -> ParseResult {
        let completed = &mut self.completed;
        build_result(
            &mut completed.table,
            &mut completed.cache,
            was_incomplete,
            count,
            Against {
                library: &self.library,
                root_name: self.root_name.as_deref(),
                // The result-size and work limits of the text so far, as a
                // one-shot parse of it would have.
                limits: self.limits.for_program(self.buf.len()),
            },
            partial,
            Some(&mut self.reuse),
        )
    }
}

/// A `$name` ending the raw text, which may still be growing (`$d` of
/// `$dishes`): `/\$\w+$/` on the text so far, when the pending statements'
/// text ends with it too and doesn't already have that name in full before
/// it.
fn growing_state_ref<'a>(buf: &'a str, pending: &str) -> Option<&'a str> {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let start = buf.len() - buf.bytes().rev().take_while(|&b| word(b)).count();
    if start == buf.len() || !buf[..start].ends_with('$') {
        return None;
    }
    let tail = &buf[start - 1..];
    let before = pending.strip_suffix(tail)?;
    // `\$name(?!\w)`: the same name, not the start of a longer one.
    let in_full = before.match_indices(tail).any(|(i, _)| {
        !before
            .as_bytes()
            .get(i + tail.len())
            .is_some_and(|&b| word(b))
    });
    (!in_full).then_some(tail)
}
