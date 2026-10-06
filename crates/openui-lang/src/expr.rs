//! Tokens to an expression tree, as the reference parser's `parseExpression`:
//! a Pratt parser that never fails. Unknown tokens become `Null`, and missing
//! closers are taken as given.

use crate::ast::Ast;
use crate::builtins::is_builtin;
use crate::js::number_to_string;
use crate::lexer::{EOF, Token};

const PREC_TERNARY: u8 = 1;
const PREC_OR: u8 = 2;
const PREC_AND: u8 = 3;
const PREC_EQ: u8 = 4;
const PREC_CMP: u8 = 5;
const PREC_ADD: u8 = 6;
const PREC_MUL: u8 = 7;
const PREC_UNARY: u8 = 8;
const PREC_MEMBER: u8 = 9;

/// Parse one statement's expression tokens.
pub fn parse_expression(tokens: &[Token]) -> Ast {
    ExprParser { tokens, pos: 0 }.expr(0)
}

/// How tightly `token` binds as an infix or postfix operator; 0 if it isn't one.
fn infix_precedence(token: &Token) -> u8 {
    match token {
        Token::Question => PREC_TERNARY,
        Token::Or => PREC_OR,
        Token::And => PREC_AND,
        Token::EqEq | Token::NotEq => PREC_EQ,
        Token::Greater | Token::Less | Token::GreaterEq | Token::LessEq => PREC_CMP,
        Token::Plus | Token::Minus => PREC_ADD,
        Token::Star | Token::Slash | Token::Percent => PREC_MUL,
        Token::Dot | Token::LBrack => PREC_MEMBER,
        _ => 0,
    }
}

/// Left-associative binary operators: the operator text and its precedence.
fn binary_operator(token: &Token) -> Option<(&'static str, u8)> {
    Some(match token {
        Token::Plus => ("+", PREC_ADD),
        Token::Minus => ("-", PREC_ADD),
        Token::Star => ("*", PREC_MUL),
        Token::Slash => ("/", PREC_MUL),
        Token::Percent => ("%", PREC_MUL),
        Token::EqEq => ("==", PREC_EQ),
        Token::NotEq => ("!=", PREC_EQ),
        Token::Greater => (">", PREC_CMP),
        Token::Less => ("<", PREC_CMP),
        Token::GreaterEq => (">=", PREC_CMP),
        Token::LessEq => ("<=", PREC_CMP),
        Token::And => ("&&", PREC_AND),
        Token::Or => ("||", PREC_OR),
        _ => return None,
    })
}

/// A member name or object key: identifiers, strings and numbers as text,
/// state variables without their `$`, anything else as `?`.
fn key_text(token: &Token) -> String {
    match token {
        Token::Ident(v) | Token::Type(v) | Token::Str(v) => v.clone(),
        Token::Num(n) => number_to_string(*n),
        Token::StateVar(v) => v.strip_prefix('$').unwrap_or(v).to_owned(),
        _ => "?".to_owned(),
    }
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    fn cur(&self) -> &'a Token {
        self.tokens.get(self.pos).unwrap_or(&EOF)
    }

    fn next_is(&self, token: &Token) -> bool {
        self.tokens.get(self.pos + 1) == Some(token)
    }

    /// Consume the current token and return it.
    fn advance(&mut self) -> &'a Token {
        let token = self.cur();
        self.pos += 1;
        token
    }

    fn eat(&mut self, token: &Token) {
        if self.cur() == token {
            self.pos += 1;
        }
    }

    fn expr(&mut self, min_prec: u8) -> Ast {
        let mut left = self.prefix();
        while infix_precedence(self.cur()) > min_prec {
            left = self.infix(left);
        }
        left
    }

    fn prefix(&mut self) -> Ast {
        match self.cur() {
            Token::Str(v) => {
                self.pos += 1;
                Ast::Str(v.clone())
            }
            Token::Num(n) => {
                self.pos += 1;
                Ast::Num(*n)
            }
            Token::True => {
                self.pos += 1;
                Ast::Bool(true)
            }
            Token::False => {
                self.pos += 1;
                Ast::Bool(false)
            }
            Token::Null => {
                self.pos += 1;
                Ast::Null
            }
            Token::LBrack => self.array(),
            Token::LBrace => self.object(),
            Token::StateVar(name) => {
                self.pos += 1;
                if *self.cur() == Token::Equals {
                    self.pos += 1;
                    let value = Box::new(self.expr(0));
                    return Ast::Assign {
                        target: name.clone(),
                        value,
                    };
                }
                Ast::StateRef(name.clone())
            }
            Token::Type(name) => {
                // Builtins need their `@` prefix to be called; only Action is exempt.
                if self.next_is(&Token::LParen) && (!is_builtin(name) || name == "Action") {
                    return self.comp(name);
                }
                self.pos += 1;
                Ast::Ref(name.clone())
            }
            Token::BuiltinCall(name) => {
                if self.next_is(&Token::LParen) {
                    return self.comp(name);
                }
                self.pos += 1;
                Ast::Ref(name.clone())
            }
            Token::Ident(name) => {
                self.pos += 1;
                Ast::Ref(name.clone())
            }
            Token::Not | Token::Minus => {
                let op = if *self.advance() == Token::Not {
                    "!"
                } else {
                    "-"
                };
                let operand = Box::new(self.expr(PREC_UNARY));
                Ast::UnaryOp { op, operand }
            }
            Token::LParen => {
                self.pos += 1;
                let inner = self.expr(0);
                self.eat(&Token::RParen);
                inner
            }
            _ => {
                // Unknown token: skip it.
                self.pos += 1;
                Ast::Null
            }
        }
    }

    fn infix(&mut self, left: Ast) -> Ast {
        let left = Box::new(left);
        let token = self.advance();
        if let Some((op, prec)) = binary_operator(token) {
            let right = Box::new(self.expr(prec));
            return Ast::BinOp { op, left, right };
        }
        match token {
            // `cond ? then : else`, right-associative.
            Token::Question => {
                let then = Box::new(self.expr(0));
                self.eat(&Token::Colon);
                let otherwise = Box::new(self.expr(0));
                Ast::Ternary {
                    cond: left,
                    then,
                    otherwise,
                }
            }
            Token::Dot => {
                let field = key_text(self.advance());
                Ast::Member { obj: left, field }
            }
            Token::LBrack => {
                let index = Box::new(self.expr(0));
                self.eat(&Token::RBrack);
                Ast::Index { obj: left, index }
            }
            // Unreachable: only tokens with an infix precedence get here.
            _ => *left,
        }
    }

    /// `Name(arg, arg, ...)`, with the cursor on `Name`.
    fn comp(&mut self, name: &str) -> Ast {
        self.pos += 1;
        self.eat(&Token::LParen);
        let args = self.list_until(&Token::RParen);
        Ast::Comp {
            name: name.to_owned(),
            args,
        }
    }

    /// `[el, el, ...]`, with the cursor on `[`.
    fn array(&mut self) -> Ast {
        self.pos += 1;
        Ast::Arr(self.list_until(&Token::RBrack))
    }

    /// Comma-separated expressions up to `close` (consumed if present).
    fn list_until(&mut self, close: &Token) -> Vec<Ast> {
        let mut items = Vec::new();
        while self.cur() != close && *self.cur() != Token::Eof {
            items.push(self.expr(0));
            self.eat(&Token::Comma);
        }
        self.eat(close);
        items
    }

    /// `{ key: value, ... }`, with the cursor on `{`.
    fn object(&mut self) -> Ast {
        self.pos += 1;
        let mut entries = Vec::new();
        while !matches!(self.cur(), Token::RBrace | Token::Eof) {
            let key = key_text(self.advance());
            self.eat(&Token::Colon);
            entries.push((key, self.expr(0)));
            self.eat(&Token::Comma);
        }
        self.eat(&Token::RBrace);
        Ast::Obj(entries)
    }
}
