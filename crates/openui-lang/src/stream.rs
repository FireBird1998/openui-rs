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
use crate::lexer::tokenize;
use crate::library::Library;
use crate::parser::{build_result, classify};
use crate::preprocess::Preprocessor;
use crate::result::ParseResult;
use crate::statements::{auto_close, split};
use crate::table::{Cache, Table};

/// Parses a program chunk by chunk as it streams in.
pub struct StreamParser {
    library: Arc<Library>,
    root_name: Option<String>,
    /// Everything pushed so far.
    buf: String,
    /// `buf` preprocessed; statement boundaries are found in this, never in `buf`.
    pre: Preprocessor,
    completed: Completed,
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
    first_id: String,
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
            let (expr, limited) = parse_expression_with_limit(&raw.tokens);
            self.depth_limited |= limited;
            if self.first_id.is_empty() {
                self.first_id = raw.id.clone();
            }
            let undo = self.table.define(&raw.id, classify(&raw, expr));
            self.cache.invalidate(undo.slot, &self.table);
            self.count += 1;
        }
    }
}

impl StreamParser {
    /// A parser for one stream. `root_name` picks the entry statement when
    /// there's no `root`: a statement by that name, else the first statement
    /// calling that component.
    pub fn new(library: Arc<Library>, root_name: Option<&str>) -> StreamParser {
        StreamParser {
            library,
            root_name: root_name.map(str::to_owned),
            buf: String::new(),
            pre: Preprocessor::new(),
            completed: Completed::default(),
        }
    }

    /// Feed the next chunk and get the latest parse result.
    pub fn push(&mut self, chunk: &str) -> ParseResult {
        self.absorb(chunk);
        self.current_result()
    }

    /// Give the full text so far, as after a reconnect. Text that continues
    /// what was pushed only parses the new part; anything else starts over.
    pub fn set(&mut self, full_text: &str) -> ParseResult {
        if !full_text.starts_with(self.buf.as_str()) {
            self.reset();
        }
        self.absorb(&full_text[self.buf.len()..]);
        self.current_result()
    }

    /// The latest parse result, without new text.
    pub fn result(&mut self) -> ParseResult {
        self.current_result()
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.pre.reset();
        self.completed.clear();
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
                    let mut peek = i + 1;
                    while peek < s.len() && matches!(s[peek], b' ' | b'\t' | b'\r' | b'\n') {
                        peek += 1;
                    }
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

    fn current_result(&mut self) -> ParseResult {
        let (mut result, pending_limited) = self.build_current();
        result.depth_limited |= self.completed.depth_limited || pending_limited;
        result
    }

    /// The result, and whether a pending statement nested past `MAX_DEPTH`.
    fn build_current(&mut self) -> (ParseResult, bool) {
        let pending_start = self.scan_new_completed();
        let pending = js::trim(&self.pre.cleaned()[pending_start..]);
        let completed = &mut self.completed;

        if pending.is_empty() {
            if completed.count == 0 {
                return (ParseResult::empty(true), false);
            }
            let count = completed.count;
            return (self.build_from_table(None, false, count), false);
        }

        let (closed, was_incomplete) = auto_close(pending);
        let pending_statements = split(&tokenize(&closed));
        if pending_statements.is_empty() {
            if completed.count == 0 {
                return (ParseResult::empty(was_incomplete), false);
            }
            let count = completed.count;
            return (self.build_from_table(None, was_incomplete, count), false);
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
            let (expr, limited) = parse_expression_with_limit(&raw.tokens);
            pending_limited |= limited;
            let undo = completed.table.define(&raw.id, classify(raw, expr));
            completed.cache.invalidate(undo.slot, &completed.table);
            undos.push(undo);
        }
        let count = completed.count + pending_statements.len();
        let first_id = (completed.first_id.is_empty()).then(|| pending_statements[0].id.as_str());
        let result = self.build_from_table(first_id, was_incomplete, count);
        for undo in undos.into_iter().rev() {
            let slot = undo.slot;
            self.completed.table.undo(undo);
            self.completed.cache.invalidate(slot, &self.completed.table);
        }
        (result, pending_limited)
    }

    /// Build the result from the table as it stands; `first_id` overrides the
    /// completed statements' first id while there is none.
    fn build_from_table(
        &mut self,
        first_id: Option<&str>,
        was_incomplete: bool,
        count: usize,
    ) -> ParseResult {
        let completed = &mut self.completed;
        build_result(
            &mut completed.table,
            &mut completed.cache,
            first_id.unwrap_or(&completed.first_id),
            was_incomplete,
            count,
            &self.library,
            self.root_name.as_deref(),
        )
    }
}
