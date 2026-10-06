//! Tokens to an expression tree, as the reference parser's `parseExpression`.

use crate::ast::Ast;
use crate::builtins::is_builtin;
use crate::lexer::{EOF, Token};

/// Parse one statement's expression tokens.
pub fn parse_expression(tokens: &[Token]) -> Ast {
    ExprParser { tokens, pos: 0 }.expr()
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    fn cur(&self) -> &'a Token {
        self.tokens.get(self.pos).unwrap_or(&EOF)
    }

    fn eat(&mut self, token: &Token) {
        if self.cur() == token {
            self.pos += 1;
        }
    }

    fn expr(&mut self) -> Ast {
        self.prefix()
    }

    fn prefix(&mut self) -> Ast {
        match self.cur() {
            Token::Str(v) => {
                self.pos += 1;
                Ast::Str(v.clone())
            }
            Token::Null => {
                self.pos += 1;
                Ast::Null
            }
            Token::Type(name) => {
                // Builtins need their `@` prefix to be called; only Action is exempt.
                let called = self.tokens.get(self.pos + 1) == Some(&Token::LParen);
                if called && (!is_builtin(name) || name == "Action") {
                    return self.comp(name);
                }
                self.pos += 1;
                Ast::Ref(name.clone())
            }
            Token::Ident(name) => {
                self.pos += 1;
                Ast::Ref(name.clone())
            }
            _ => {
                // Unknown token: skip it.
                self.pos += 1;
                Ast::Null
            }
        }
    }

    /// `Name(arg, arg, ...)`, with the cursor on `Name`.
    fn comp(&mut self, name: &str) -> Ast {
        self.pos += 1;
        self.eat(&Token::LParen);
        let mut args = Vec::new();
        while !matches!(self.cur(), Token::RParen | Token::Eof) {
            args.push(self.expr());
            if *self.cur() == Token::Comma {
                self.pos += 1;
            }
        }
        self.eat(&Token::RParen);
        Ast::Comp {
            name: name.to_owned(),
            args,
        }
    }
}
