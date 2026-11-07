//! Streaming parsing, as the reference parser's `createStreamParser`.
//!
//! Statements that end in a newline outside brackets are parsed once and
//! cached. The trailing statement is still arriving, so it's auto-closed and
//! re-parsed on every push and laid over a copy of the cache.
//!
//! The reference re-derives the cleaned text from the whole buffer on every
//! push and rebuilds the whole tree; here a [`Preprocessor`] extends the
//! cleaned text chunk by chunk, and a [`Cache`] keeps the materialized
//! statements that nothing changed under, with the same result.

use std::sync::Arc;

use crate::expr::parse_expression_with_limit;
use crate::js;
use crate::lexer::{skip_while, tokenize};
use crate::library::Library;
use crate::parser::{PartialRef, build_result, classify};
use crate::preprocess::Preprocessor;
use crate::progress;
use crate::result::ParseResult;
use crate::statements::{auto_close, split};
use crate::table::{Cache, Slot, Table};

/// Parses a program chunk by chunk as it streams in.
pub struct StreamParser {
    library: Arc<Library>,
    root_name: Option<String>,
    /// Everything pushed so far.
    buf: String,
    /// `buf` preprocessed; statement boundaries are found in this, never in `buf`.
    pre: Preprocessor,
    completed: Completed,
    /// The result a stall cut the stream at (in a release build), given
    /// again until [`StreamParser::set`] starts over.
    stalled: Option<ParseResult>,
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
    /// Some statement nested past `MAX_DEPTH`.
    depth_limited: bool,
}

impl Completed {
    fn clear(&mut self) {
        *self = Completed::default();
    }

    fn add(&mut self, text: &str) {
        let text = js::trim(text);
        if text.is_empty() {
            return;
        }
        for raw in split(&tokenize(text)) {
            if progress::stalled_so_far() {
                break;
            }
            let (expr, limited) = parse_expression_with_limit(&raw.tokens);
            self.depth_limited |= limited;
            let undo = self.table.define(&raw.id, classify(&raw, expr));
            self.cache.invalidate(undo.slot, &self.table);
            self.count += 1;
        }
    }
}

impl StreamParser {
    /// A parser for one stream. `root_name` is the library's root component:
    /// with no `root` statement, the first statement that calls it is the
    /// entry.
    pub fn new(library: Arc<Library>, root_name: Option<&str>) -> StreamParser {
        StreamParser {
            library,
            root_name: root_name.map(str::to_owned),
            buf: String::new(),
            pre: Preprocessor::new(),
            completed: Completed::default(),
            stalled: None,
        }
    }

    /// Feed the next chunk and get the latest parse result.
    ///
    /// A stream a stall cut (see [`ParseResult::size_limited`]) ignores
    /// further pushes and gives the cut result again, until
    /// [`StreamParser::set`].
    pub fn push(&mut self, chunk: &str) -> ParseResult {
        self.watched(|parser| {
            parser.absorb(chunk);
            parser.current_result(false)
        })
    }

    /// Give the full text so far, as after a reconnect. Text that continues
    /// what was pushed only parses the new part; anything else starts over,
    /// as does a stream a stall cut.
    pub fn set(&mut self, full_text: &str) -> ParseResult {
        if self.stalled.is_some() || !full_text.starts_with(self.buf.as_str()) {
            self.reset();
        }
        self.watched(|parser| {
            parser.absorb(&full_text[parser.buf.len()..]);
            parser.current_result(false)
        })
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
    /// it was cut at. A stall in `step` cuts it there.
    fn watched(&mut self, step: impl FnOnce(&mut Self) -> ParseResult) -> ParseResult {
        if let Some(cut) = &self.stalled {
            return cut.clone();
        }
        let (mut result, stalled) = progress::watch(|| step(self));
        if stalled {
            result.size_limited = true;
            self.stalled = Some(result.clone());
        }
        result
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.pre.reset();
        self.completed.clear();
        self.stalled = None;
    }

    /// Take in new text. If the cleaned text changed under the completed
    /// statements (an opening fence appeared and shifted everything, say),
    /// they're stale: start over, as the reference does.
    fn absorb(&mut self, chunk: &str) {
        self.buf.push_str(chunk);
        let unchanged = self.pre.push(chunk);
        if self.completed.end > unchanged {
            self.completed.clear();
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
                    // A next line starting with `?` (or `:` mid-ternary) continues it.
                    let peek = skip_while(s, i + 1, |b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
                    if let Some(&next) = s.get(peek)
                        && (next == b'?' || (next == b':' && ternary_depth > 0))
                    {
                        continue;
                    }
                    completed.push(stmt_start..i);
                    stmt_start = i + 1;
                    self.completed.end = i + 1;
                }
                _ => {}
            }
        }

        for range in completed {
            self.completed.add(&cleaned[range]);
        }
        stmt_start
    }

    fn current_result(&mut self, complete: bool) -> ParseResult {
        let (mut result, pending_limited) = self.build_current(complete);
        result.depth_limited |= self.completed.depth_limited || pending_limited;
        result
    }

    /// The result, and whether a pending statement nested past `MAX_DEPTH`.
    fn build_current(&mut self, complete: bool) -> (ParseResult, bool) {
        let pending_start = self.scan_new_completed();
        let pending = js::trim(&self.pre.cleaned()[pending_start..]);
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
            let (expr, limited) = parse_expression_with_limit(&raw.tokens);
            pending_limited |= limited;
            let undo = completed.table.define(&raw.id, classify(raw, expr));
            completed.cache.invalidate(undo.slot, &completed.table);
            undos.push(undo);
        }
        let count = completed.count + pending_statements.len();
        let growing = if complete {
            None
        } else {
            growing_state_ref(&self.buf, pending).map(str::to_owned)
        };
        let pending_slots: Vec<Slot> = undos.iter().map(|undo| undo.slot).collect();
        let partial = growing.as_deref().map(|name| PartialRef {
            name,
            slots: &pending_slots,
        });
        let result = self.build_from_table(was_incomplete, count, partial);
        for undo in undos.into_iter().rev() {
            let slot = undo.slot;
            self.completed.table.undo(undo);
            self.completed.cache.invalidate(slot, &self.completed.table);
        }
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
            &self.library,
            self.root_name.as_deref(),
            partial,
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
