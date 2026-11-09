//! Splitting tokens into statements, and closing what a stream left open, as
//! the reference parser's `split` and `autoClose`.

use crate::lexer::{EOF, Pos, Token, Tokens};
use crate::progress::Progress;

/// Which kind of token named a statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdKind {
    /// A lowercase name: `header = ...`.
    Ident,
    /// A PascalCase name: `Header = ...`.
    Type,
    /// A state variable: `$count = ...`.
    StateVar,
}

impl IdKind {
    /// The reference parser's numeric token kind (`T`) for the id's token.
    pub fn token_kind(self) -> u8 {
        match self {
            IdKind::Ident => 16,
            IdKind::Type => 17,
            IdKind::StateVar => 18,
        }
    }
}

/// A statement before its expression is parsed: `id = <tokens>`.
#[derive(Clone, Debug, PartialEq)]
pub struct RawStatement {
    /// The statement's name, `$` included for state.
    pub id: String,
    /// Which kind of token named it.
    pub id_kind: IdKind,
    /// The expression's tokens.
    pub tokens: Tokens,
    /// Where the statement starts: at its name.
    pub start: Pos,
    /// Where it ends: at the newline that closes it, or the end of the input.
    pub end: Pos,
}

/// Close an unterminated string and any open brackets so a partial program
/// parses. Returns the closed text and whether anything was open.
pub fn auto_close(input: &str) -> (String, bool) {
    let mut stack: Vec<u8> = Vec::new();
    let mut in_str: Option<u8> = None;
    let mut esc = false;

    for &c in input.as_bytes() {
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
            b'(' | b'[' | b'{' => stack.push(c),
            b')' if stack.last() == Some(&b'(') => {
                stack.pop();
            }
            b']' if stack.last() == Some(&b'[') => {
                stack.pop();
            }
            b'}' if stack.last() == Some(&b'{') => {
                stack.pop();
            }
            _ => {}
        }
    }

    if in_str.is_none() && stack.is_empty() {
        return (input.to_owned(), false);
    }
    let mut out = input.to_owned();
    if let Some(quote) = in_str {
        if esc {
            out.push('\\');
        }
        out.push(quote as char);
    }
    for &open in stack.iter().rev() {
        out.push(match open {
            b'(' => ')',
            b'[' => ']',
            _ => '}',
        });
    }
    (out, true)
}

/// The position after a newline whose following newline run will be skipped.
fn after_newline(pos: usize) -> usize {
    pos + 1
}

/// Split a token stream into `id = expression` statements. Statements end at a
/// newline outside brackets, unless the next line continues a ternary. Lines
/// that aren't statements are skipped.
pub fn split(tokens: &Tokens) -> Vec<RawStatement> {
    let starts = tokens.starts();
    let mut stmts = Vec::new();
    let mut pos = 0;
    let n = tokens.len();
    // Where the run of tokens matching `pred` from `pos` ends. It can't
    // stall: a count, not a loop.
    let skip = |pos: usize, pred: fn(&Token) -> bool| {
        pos + tokens
            .get(pos..)
            .unwrap_or_default()
            .iter()
            .take_while(|t| pred(t))
            .count()
    };
    let skip_line = |pos: &mut usize| {
        *pos = skip(*pos, |t| !matches!(t, Token::Newline | Token::Eof));
    };
    let mut progress = Progress::new("statement splitting");

    while pos < n {
        if progress.stalled(pos) {
            break;
        }
        pos = skip(pos, |t| *t == Token::Newline);
        if pos >= n || tokens[pos] == Token::Eof {
            break;
        }

        let start = starts[pos];
        let (id, id_kind) = match &tokens[pos] {
            Token::Ident(v) => (v.clone(), IdKind::Ident),
            Token::Type(v) => (v.clone(), IdKind::Type),
            Token::StateVar(v) => (v.clone(), IdKind::StateVar),
            _ => {
                skip_line(&mut pos);
                continue;
            }
        };
        pos += 1;

        if pos >= n || tokens[pos] != Token::Equals {
            skip_line(&mut pos);
            continue;
        }
        pos += 1;

        let mut expr = Tokens::default();
        let mut depth: i32 = 0;
        let mut ternary_depth: i32 = 0;
        let mut expr_progress = Progress::new("statement splitting");
        while pos < n && tokens[pos] != Token::Eof {
            if expr_progress.stalled(pos) {
                // Stop at the end: the tokens always end in an `Eof`.
                pos = n - 1;
                break;
            }
            let token = &tokens[pos];
            if *token == Token::Newline && depth <= 0 && ternary_depth <= 0 {
                let peek = skip(after_newline(pos), |t| *t == Token::Newline);
                let next = tokens.get(peek).unwrap_or(&EOF);
                if *next == Token::Question {
                    // The line breaks up to it are no part of the
                    // expression: past them all at once, so a long run
                    // isn't looked through again from each one.
                    pos = peek;
                    continue;
                }
                break;
            }
            if *token == Token::Newline {
                pos += 1;
                continue;
            }
            match token {
                Token::LParen | Token::LBrack | Token::LBrace => depth += 1,
                Token::RParen | Token::RBrack | Token::RBrace if depth > 0 => depth -= 1,
                Token::Question if depth == 0 => ternary_depth += 1,
                Token::Colon if depth == 0 && ternary_depth > 0 => ternary_depth -= 1,
                _ => {}
            }
            expr.push(token.clone(), starts[pos]);
            pos += 1;
        }

        if !expr.is_empty() {
            stmts.push(RawStatement {
                id,
                id_kind,
                tokens: expr,
                start,
                // A newline or the end: the tokens always end in the latter.
                end: starts[pos],
            });
        }
    }

    stmts
}
