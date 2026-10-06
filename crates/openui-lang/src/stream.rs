//! Streaming parsing, as the reference parser's `createStreamParser`.
//!
//! Statements that end in a newline outside brackets are parsed once and
//! cached. The trailing statement is still arriving, so it's auto-closed and
//! re-parsed on every push and laid over a copy of the cache.

use std::sync::Arc;

use indexmap::IndexMap;

use crate::expr::parse_expression;
use crate::js;
use crate::lexer::tokenize;
use crate::library::Library;
use crate::parser::{Statement, build_result, classify, preprocess};
use crate::result::ParseResult;
use crate::statements::{auto_close, split};

/// Parses a program chunk by chunk as it streams in.
pub struct StreamParser {
    library: Arc<Library>,
    root_name: Option<String>,
    /// Everything pushed so far.
    buf: String,
    /// `buf` preprocessed; statement boundaries are found in this, never in `buf`.
    cleaned: String,
    /// How far into `cleaned` the completed statements reach.
    completed_end: usize,
    completed: IndexMap<String, Statement>,
    /// Completed statements parsed, counting re-definitions.
    completed_count: usize,
    first_id: String,
}

impl StreamParser {
    pub fn new(library: Arc<Library>, root_name: Option<&str>) -> StreamParser {
        StreamParser {
            library,
            root_name: root_name.map(str::to_owned),
            buf: String::new(),
            cleaned: String::new(),
            completed_end: 0,
            completed: IndexMap::new(),
            completed_count: 0,
            first_id: String::new(),
        }
    }

    /// Feed the next chunk and get the latest parse result.
    pub fn push(&mut self, chunk: &str) -> ParseResult {
        self.buf.push_str(chunk);
        self.current_result()
    }

    fn add_statements(&mut self, text: &str) {
        let text = js::trim(text);
        if text.is_empty() {
            return;
        }
        for raw in split(&tokenize(text)) {
            let expr = parse_expression(&raw.tokens);
            if self.first_id.is_empty() {
                self.first_id = raw.id.clone();
            }
            self.completed.insert(raw.id.clone(), classify(&raw, expr));
            self.completed_count += 1;
        }
    }

    /// Re-derive `cleaned`. If the completed prefix changed (say an opening
    /// fence appeared and shifted everything), the cache is stale: start over.
    fn refresh_cleaned(&mut self) {
        let next = preprocess(&self.buf);
        if !next.starts_with(&self.cleaned[..self.completed_end]) {
            self.completed_end = 0;
            self.completed.clear();
            self.completed_count = 0;
            self.first_id.clear();
        }
        self.cleaned = next;
    }

    /// Cache statements completed since the last scan. Returns where the
    /// pending (still incomplete) statement starts.
    fn scan_new_completed(&mut self) -> usize {
        let s = self.cleaned.as_bytes();
        let mut depth: i32 = 0;
        let mut ternary_depth: i32 = 0;
        let mut in_str: Option<u8> = None;
        let mut esc = false;
        let mut stmt_start = self.completed_end;
        let mut completed = Vec::new();

        for i in self.completed_end..s.len() {
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
                    self.completed_end = i + 1;
                }
                _ => {}
            }
        }

        let cleaned = std::mem::take(&mut self.cleaned);
        for range in completed {
            self.add_statements(&cleaned[range]);
        }
        self.cleaned = cleaned;
        stmt_start
    }

    fn current_result(&mut self) -> ParseResult {
        self.refresh_cleaned();
        let pending_start = self.scan_new_completed();
        let pending = js::trim(&self.cleaned[pending_start..]);
        let library = &self.library;
        let root_name = self.root_name.as_deref();

        if pending.is_empty() {
            if self.completed_count == 0 {
                return ParseResult::empty(true);
            }
            return build_result(
                &self.completed,
                &self.first_id,
                false,
                self.completed_count,
                library,
                root_name,
            );
        }

        let (closed, was_incomplete) = auto_close(pending);
        let pending_statements = split(&tokenize(&closed));
        if pending_statements.is_empty() {
            if self.completed_count == 0 {
                return ParseResult::empty(was_incomplete);
            }
            return build_result(
                &self.completed,
                &self.first_id,
                was_incomplete,
                self.completed_count,
                library,
                root_name,
            );
        }

        // A new id renders as it arrives; a re-defined one replaces the cached
        // statement only once it no longer needs closing.
        let mut all = self.completed.clone();
        for raw in &pending_statements {
            if was_incomplete && self.completed.contains_key(&raw.id) {
                continue;
            }
            let expr = parse_expression(&raw.tokens);
            all.insert(raw.id.clone(), classify(raw, expr));
        }
        let first_id = if self.first_id.is_empty() {
            &pending_statements[0].id
        } else {
            &self.first_id
        };
        build_result(
            &all,
            first_id,
            was_incomplete,
            self.completed_count + pending_statements.len(),
            library,
            root_name,
        )
    }
}
