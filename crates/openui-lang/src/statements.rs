//! Splitting tokens into statements, and closing what a stream left open, as
//! the reference parser's `split` and `autoClose`.

use crate::lexer::Token;

/// Which kind of token named a statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdKind {
    Ident,
    Type,
    StateVar,
}

/// A statement before its expression is parsed: `id = <tokens>`.
#[derive(Clone, Debug, PartialEq)]
pub struct RawStatement {
    pub id: String,
    pub id_kind: IdKind,
    pub tokens: Vec<Token>,
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

/// Split a token stream into `id = expression` statements. Statements end at a
/// newline outside brackets, unless the next line continues a ternary. Lines
/// that aren't statements are skipped.
pub fn split(tokens: &[Token]) -> Vec<RawStatement> {
    let mut stmts = Vec::new();
    let mut pos = 0;
    let n = tokens.len();
    let skip_line = |pos: &mut usize| {
        while *pos < n && !matches!(tokens[*pos], Token::Newline | Token::Eof) {
            *pos += 1;
        }
    };

    while pos < n {
        while pos < n && tokens[pos] == Token::Newline {
            pos += 1;
        }
        if pos >= n || tokens[pos] == Token::Eof {
            break;
        }

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

        let mut expr = Vec::new();
        let mut depth: i32 = 0;
        let mut ternary_depth: i32 = 0;
        while pos < n && tokens[pos] != Token::Eof {
            let token = &tokens[pos];
            if *token == Token::Newline && depth <= 0 && ternary_depth <= 0 {
                let mut peek = pos + 1;
                while peek < n && tokens[peek] == Token::Newline {
                    peek += 1;
                }
                let next = tokens.get(peek).unwrap_or(&Token::Eof);
                if *next == Token::Question || (*next == Token::Colon && ternary_depth > 0) {
                    pos += 1;
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
            expr.push(token.clone());
            pos += 1;
        }

        if !expr.is_empty() {
            stmts.push(RawStatement {
                id,
                id_kind,
                tokens: expr,
            });
        }
    }

    stmts
}
