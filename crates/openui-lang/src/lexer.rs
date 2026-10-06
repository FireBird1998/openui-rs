//! Program text to tokens, as the reference parser's `tokenize`.
//!
//! Works on bytes: every character the grammar cares about is ASCII, and UTF-8
//! continuation bytes never look like ASCII, so byte positions behave like the
//! reference parser's UTF-16 positions wherever it matters.

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

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Tokenize a program. Always ends with [`Token::Eof`].
pub fn tokenize(src: &str) -> Vec<Token> {
    let s = src.as_bytes();
    let n = s.len();
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

        let punctuation = match c {
            b'\n' => Some(Token::Newline),
            b'(' => Some(Token::LParen),
            b')' => Some(Token::RParen),
            _ => None,
        };
        if let Some(token) = punctuation {
            tokens.push(token);
            i += 1;
            continue;
        }

        if c == b'=' {
            if s.get(i + 1) == Some(&b'=') {
                tokens.push(Token::EqEq);
                i += 2;
            } else {
                tokens.push(Token::Equals);
                i += 1;
            }
            continue;
        }

        if c == b'"' {
            let (token, next) = double_quoted(src, i);
            tokens.push(token);
            i = next;
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
        serde_json::from_str::<String>(raw)
    } else {
        serde_json::from_str::<String>(&format!("{raw}\""))
    };
    let value = parsed.unwrap_or_else(|_| {
        let inner = raw.strip_prefix('"').unwrap_or(raw);
        inner.strip_suffix('"').unwrap_or(inner).to_owned()
    });
    (Token::Str(value), end)
}
