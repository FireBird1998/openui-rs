//! Program text to tokens, as the reference parser's `tokenize`.
//!
//! Works on bytes: every character the grammar cares about is ASCII, and UTF-8
//! continuation bytes never look like ASCII, so byte positions behave like the
//! reference parser's UTF-16 positions wherever it matters.

use crate::js::{json_parse_string, write_json_number, write_json_string};

pub(crate) static EOF: Token = Token::Eof;

/// One token, mirroring the reference parser's `T` kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Newline,
    LParen,
    RParen,
    LBrack,
    RBrack,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Equals,
    True,
    False,
    Null,
    Eof,
    Str(String),
    /// As JavaScript's `+text`: `1e` is NaN, `1e400` is Infinity.
    Num(f64),
    /// Lowercase identifier; becomes a reference.
    Ident(String),
    /// PascalCase identifier; becomes a component name or a reference.
    Type(String),
    /// `$name`, including the `$`.
    StateVar(String),
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Greater,
    Less,
    GreaterEq,
    LessEq,
    And,
    Or,
    Not,
    Question,
    /// `@name`, without the `@`.
    BuiltinCall(String),
}

impl Token {
    /// The reference parser's numeric token kind (`T`).
    pub fn kind(&self) -> u8 {
        match self {
            Token::Newline => 0,
            Token::LParen => 1,
            Token::RParen => 2,
            Token::LBrack => 3,
            Token::RBrack => 4,
            Token::LBrace => 5,
            Token::RBrace => 6,
            Token::Comma => 7,
            Token::Colon => 8,
            Token::Equals => 9,
            Token::True => 10,
            Token::False => 11,
            Token::Null => 12,
            Token::Eof => 13,
            Token::Str(_) => 14,
            Token::Num(_) => 15,
            Token::Ident(_) => 16,
            Token::Type(_) => 17,
            Token::StateVar(_) => 18,
            Token::Dot => 19,
            Token::Plus => 20,
            Token::Minus => 21,
            Token::Star => 22,
            Token::Slash => 23,
            Token::Percent => 24,
            Token::EqEq => 25,
            Token::NotEq => 26,
            Token::Greater => 27,
            Token::Less => 28,
            Token::GreaterEq => 29,
            Token::LessEq => 30,
            Token::And => 31,
            Token::Or => 32,
            Token::Not => 33,
            Token::Question => 34,
            Token::BuiltinCall(_) => 35,
        }
    }
}

/// Tokens as the reference parser's `JSON.stringify(tokenize(...))` writes them.
pub fn tokens_to_json(tokens: &[Token]) -> String {
    let mut out = String::with_capacity(tokens.len() * 12);
    out.push('[');
    for (i, token) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"t\":");
        out.push_str(&token.kind().to_string());
        match token {
            Token::Str(v)
            | Token::Ident(v)
            | Token::Type(v)
            | Token::StateVar(v)
            | Token::BuiltinCall(v) => {
                out.push_str(",\"v\":");
                write_json_string(&mut out, v);
            }
            Token::Num(n) => {
                out.push_str(",\"v\":");
                write_json_number(&mut out, *n);
            }
            _ => {}
        }
        out.push('}');
    }
    out.push(']');
    out
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Tokens after which `-` is subtraction rather than the sign of a number.
fn ends_value(token: Option<&Token>) -> bool {
    matches!(
        token,
        Some(
            Token::Num(_)
                | Token::Str(_)
                | Token::Ident(_)
                | Token::Type(_)
                | Token::RParen
                | Token::RBrack
                | Token::True
                | Token::False
                | Token::Null
                | Token::StateVar(_)
                | Token::BuiltinCall(_)
        )
    )
}

/// Tokenize a program. Always ends with [`Token::Eof`].
pub fn tokenize(src: &str) -> Vec<Token> {
    let s = src.as_bytes();
    let n = s.len();
    let at = |i: usize| s.get(i).copied();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < n {
        while i < n && matches!(s[i], b' ' | b'\t' | b'\r') {
            i += 1;
        }
        if i >= n {
            break;
        }
        let c = s[i];
        let next_is_digit = at(i + 1).is_some_and(|b| b.is_ascii_digit());

        // One- and two-character punctuation and operators.
        let followed_by_eq = at(i + 1) == Some(b'=');
        let operator = match c {
            b'\n' => Some((Token::Newline, 1)),
            b'(' => Some((Token::LParen, 1)),
            b')' => Some((Token::RParen, 1)),
            b'[' => Some((Token::LBrack, 1)),
            b']' => Some((Token::RBrack, 1)),
            b'{' => Some((Token::LBrace, 1)),
            b'}' => Some((Token::RBrace, 1)),
            b',' => Some((Token::Comma, 1)),
            b':' => Some((Token::Colon, 1)),
            b'=' if followed_by_eq => Some((Token::EqEq, 2)),
            b'=' => Some((Token::Equals, 1)),
            b'!' if followed_by_eq => Some((Token::NotEq, 2)),
            b'!' => Some((Token::Not, 1)),
            b'>' if followed_by_eq => Some((Token::GreaterEq, 2)),
            b'>' => Some((Token::Greater, 1)),
            b'<' if followed_by_eq => Some((Token::LessEq, 2)),
            b'<' => Some((Token::Less, 1)),
            // A single `&` or `|` counts as the doubled operator.
            b'&' => Some((Token::And, if at(i + 1) == Some(b'&') { 2 } else { 1 })),
            b'|' => Some((Token::Or, if at(i + 1) == Some(b'|') { 2 } else { 1 })),
            b'.' => Some((Token::Dot, 1)),
            b'?' => Some((Token::Question, 1)),
            b'+' => Some((Token::Plus, 1)),
            b'*' => Some((Token::Star, 1)),
            b'/' => Some((Token::Slash, 1)),
            b'%' => Some((Token::Percent, 1)),
            // A minus right before a digit signs a number, unless it follows a value.
            b'-' if !next_is_digit || ends_value(tokens.last()) => Some((Token::Minus, 1)),
            _ => None,
        };
        if let Some((token, width)) = operator {
            tokens.push(token);
            i += width;
            continue;
        }

        if c == b'"' {
            let (token, next) = double_quoted(src, i);
            tokens.push(token);
            i = next;
            continue;
        }
        if c == b'\'' {
            let (token, next) = single_quoted(src, i);
            tokens.push(token);
            i = next;
            continue;
        }

        if c.is_ascii_digit() || c == b'-' {
            let start = i;
            if c == b'-' {
                i += 1;
            }
            while i < n && s[i].is_ascii_digit() {
                i += 1;
            }
            if at(i) == Some(b'.') && at(i + 1).is_some_and(|b| b.is_ascii_digit()) {
                i += 1;
                while i < n && s[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if matches!(at(i), Some(b'e' | b'E')) {
                i += 1;
                if matches!(at(i), Some(b'+' | b'-')) {
                    i += 1;
                }
                while i < n && s[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // JavaScript's `+text`; a dangling exponent such as `1e` is NaN.
            tokens.push(Token::Num(src[start..i].parse().unwrap_or(f64::NAN)));
            continue;
        }

        if c == b'$' && at(i + 1).is_some_and(is_ident_start) {
            let start = i;
            i += 1;
            while i < n && is_ident_char(s[i]) {
                i += 1;
            }
            tokens.push(Token::StateVar(src[start..i].to_owned()));
            continue;
        }

        if is_ident_start(c) {
            let start = i;
            while i < n && is_ident_char(s[i]) {
                i += 1;
            }
            let word = &src[start..i];
            tokens.push(match word {
                "true" => Token::True,
                "false" => Token::False,
                "null" => Token::Null,
                _ if c.is_ascii_uppercase() => Token::Type(word.to_owned()),
                _ => Token::Ident(word.to_owned()),
            });
            continue;
        }

        if c == b'@' && at(i + 1).is_some_and(is_ident_start) {
            i += 1;
            let start = i;
            while i < n && is_ident_char(s[i]) {
                i += 1;
            }
            tokens.push(Token::BuiltinCall(src[start..i].to_owned()));
            continue;
        }

        i += 1; // any other character is skipped
    }

    tokens.push(Token::Eof);
    tokens
}

/// A `"..."` string starting at `start`. Unescaping is `JSON.parse`'s; an
/// unterminated string (mid-stream) gets a closing quote first, and text that
/// still won't parse falls back to the raw text without its quotes.
fn double_quoted(src: &str, start: usize) -> (Token, usize) {
    let s = src.as_bytes();
    let n = s.len();
    let mut i = start + 1;
    let mut closed = false;
    while i < n {
        match s[i] {
            b'\\' => i += 2,
            b'"' => {
                i += 1;
                closed = true;
                break;
            }
            _ => i += 1,
        }
    }
    let end = i.min(n);
    let raw = &src[start..end];
    let parsed = if closed {
        json_parse_string(raw)
    } else {
        json_parse_string(&format!("{raw}\""))
    };
    let value = parsed.unwrap_or_else(|| {
        let inner = raw.strip_prefix('"').unwrap_or(raw);
        inner.strip_suffix('"').unwrap_or(inner).to_owned()
    });
    (Token::Str(value), end)
}

/// A `'...'` string starting at `start`: `\'`, `\\`, `\n` and `\t` are escapes,
/// any other escaped character stands for itself. May be unterminated.
fn single_quoted(src: &str, start: usize) -> (Token, usize) {
    let s = src.as_bytes();
    let n = s.len();
    let mut value = String::new();
    let mut i = start + 1;
    let mut run = i; // start of the current run of plain characters
    while i < n {
        match s[i] {
            b'\\' => {
                value.push_str(&src[run..i]);
                i += 1;
                if let Some(escaped) = src[i..].chars().next() {
                    value.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    i += escaped.len_utf8();
                }
                run = i;
            }
            b'\'' => {
                value.push_str(&src[run..i]);
                return (Token::Str(value), i + 1);
            }
            _ => i += 1,
        }
    }
    value.push_str(&src[run..n]);
    (Token::Str(value), n)
}
