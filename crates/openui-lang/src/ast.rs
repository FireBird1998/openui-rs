//! Expression trees, mirroring the reference parser's `ASTNode`.

use crate::js::{write_json_number, write_json_string};

/// One expression node.
#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    /// A component or builtin call: `Header("Hello")`, `@Count(items)`.
    Comp {
        name: String,
        args: Vec<Ast>,
    },
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
    Arr(Vec<Ast>),
    /// Entries in source order; keys may repeat.
    Obj(Vec<(String, Ast)>),
    /// A reference to another statement: `header`.
    Ref(String),
    /// A placeholder for a reference that couldn't be resolved.
    Ph(String),
    /// A reactive state variable, `$` included: `$count`.
    StateRef(String),
    BinOp {
        op: &'static str,
        left: Box<Ast>,
        right: Box<Ast>,
    },
    UnaryOp {
        op: &'static str,
        operand: Box<Ast>,
    },
    Ternary {
        cond: Box<Ast>,
        then: Box<Ast>,
        otherwise: Box<Ast>,
    },
    Member {
        obj: Box<Ast>,
        field: String,
    },
    Index {
        obj: Box<Ast>,
        index: Box<Ast>,
    },
    /// `$count = expression`.
    Assign {
        target: String,
        value: Box<Ast>,
    },
}

impl Ast {
    /// The node as the reference parser's `JSON.stringify` writes it.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_ast(&mut out, self);
        out
    }
}

fn write_ast(out: &mut String, node: &Ast) {
    let key = |out: &mut String, k: &str| {
        out.push_str(",\"");
        out.push_str(k);
        out.push_str("\":");
    };
    out.push_str("{\"k\":\"");
    match node {
        Ast::Comp { name, args } => {
            out.push_str("Comp\"");
            key(out, "name");
            write_json_string(out, name);
            key(out, "args");
            write_list(out, args);
        }
        Ast::Str(v) => {
            out.push_str("Str\"");
            key(out, "v");
            write_json_string(out, v);
        }
        Ast::Num(v) => {
            out.push_str("Num\"");
            key(out, "v");
            write_json_number(out, *v);
        }
        Ast::Bool(v) => {
            out.push_str("Bool\"");
            key(out, "v");
            out.push_str(if *v { "true" } else { "false" });
        }
        Ast::Null => out.push_str("Null\""),
        Ast::Arr(els) => {
            out.push_str("Arr\"");
            key(out, "els");
            write_list(out, els);
        }
        Ast::Obj(entries) => {
            out.push_str("Obj\"");
            key(out, "entries");
            out.push('[');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('[');
                write_json_string(out, k);
                out.push(',');
                write_ast(out, v);
                out.push(']');
            }
            out.push(']');
        }
        Ast::Ref(n) | Ast::Ph(n) | Ast::StateRef(n) => {
            out.push_str(match node {
                Ast::Ref(_) => "Ref\"",
                Ast::Ph(_) => "Ph\"",
                _ => "StateRef\"",
            });
            key(out, "n");
            write_json_string(out, n);
        }
        Ast::BinOp { op, left, right } => {
            out.push_str("BinOp\"");
            key(out, "op");
            write_json_string(out, op);
            key(out, "left");
            write_ast(out, left);
            key(out, "right");
            write_ast(out, right);
        }
        Ast::UnaryOp { op, operand } => {
            out.push_str("UnaryOp\"");
            key(out, "op");
            write_json_string(out, op);
            key(out, "operand");
            write_ast(out, operand);
        }
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => {
            out.push_str("Ternary\"");
            key(out, "cond");
            write_ast(out, cond);
            key(out, "then");
            write_ast(out, then);
            key(out, "else");
            write_ast(out, otherwise);
        }
        Ast::Member { obj, field } => {
            out.push_str("Member\"");
            key(out, "obj");
            write_ast(out, obj);
            key(out, "field");
            write_json_string(out, field);
        }
        Ast::Index { obj, index } => {
            out.push_str("Index\"");
            key(out, "obj");
            write_ast(out, obj);
            key(out, "index");
            write_ast(out, index);
        }
        Ast::Assign { target, value } => {
            out.push_str("Assign\"");
            key(out, "target");
            write_json_string(out, target);
            key(out, "value");
            write_ast(out, value);
        }
    }
    out.push('}');
}

fn write_list(out: &mut String, nodes: &[Ast]) {
    out.push('[');
    for (i, node) in nodes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_ast(out, node);
    }
    out.push(']');
}
