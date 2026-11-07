//! Expression trees, mirroring the reference parser's `ASTNode`.
//!
//! Children are reference-counted, so a node clones in constant time and a
//! subtree inlined in many places (a statement referenced twice, a
//! component's args repeated in `mapped_props`) is stored once.

use std::sync::Arc;

use crate::js::{JsObject, write_json_number, write_json_string};

/// One expression node.
#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    /// A component or builtin call: `Header("Hello")`, `@Count(items)`.
    /// Materializing adds `mapped_props`, its args by param name, to a library
    /// component inside a runtime expression and to a call of a function or
    /// action step.
    Comp {
        /// The callee, without `@`.
        name: String,
        /// Positional arguments.
        args: Arc<Vec<Ast>>,
        /// Written `@Name(...)` with a name that isn't a builtin: a library
        /// function or action, or an unknown one. The JSON's `fn`.
        function: bool,
        /// The same args by param name; only after materializing.
        mapped_props: Option<Arc<JsObject<Ast>>>,
    },
    /// A string literal, unescaped.
    Str(String),
    /// A number literal.
    Num(f64),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// An array literal.
    Arr(Arc<Vec<Ast>>),
    /// Entries in source order; keys may repeat.
    Obj(Arc<Vec<(String, Ast)>>),
    /// A reference to another statement: `header`.
    Ref(String),
    /// A placeholder for a reference that couldn't be resolved.
    Ph(String),
    /// A reactive state variable, `$` included: `$count`.
    StateRef(String),
    /// A reference to a Query or Mutation statement, resolved at runtime.
    RuntimeRef {
        /// The statement's name.
        n: String,
        /// Whether it's a query or a mutation.
        ref_type: RefType,
    },
    /// A binary operation: `a + b`, `x == 1`, `p && q`.
    BinOp {
        /// `+ - * / % == != > < >= <= && ||`.
        op: &'static str,
        /// The left operand.
        left: Arc<Ast>,
        /// The right operand.
        right: Arc<Ast>,
    },
    /// `!x` or `-x`.
    UnaryOp {
        /// `!` or `-`.
        op: &'static str,
        /// The operand.
        operand: Arc<Ast>,
    },
    /// `cond ? then : otherwise`.
    Ternary {
        /// The condition.
        cond: Arc<Ast>,
        /// The value when it's truthy.
        then: Arc<Ast>,
        /// The value otherwise (the reference's `else`).
        otherwise: Arc<Ast>,
    },
    /// `obj.field`; on an array, the field of every element.
    Member {
        /// What's read from.
        obj: Arc<Ast>,
        /// The field name.
        field: String,
    },
    /// `obj[index]`.
    Index {
        /// What's indexed.
        obj: Arc<Ast>,
        /// The index or key.
        index: Arc<Ast>,
    },
    /// `$count = expression`.
    Assign {
        /// The state variable, `$` included.
        target: String,
        /// The value assigned.
        value: Arc<Ast>,
    },
}

/// What a [`Ast::RuntimeRef`] points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefType {
    /// A `Query(...)` statement.
    Query,
    /// A `Mutation(...)` statement.
    Mutation,
}

impl RefType {
    /// `"query"` or `"mutation"`, as in the JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            RefType::Query => "query",
            RefType::Mutation => "mutation",
        }
    }
}

impl Ast {
    /// The node as the reference parser's `JSON.stringify` writes it.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_ast(&mut out, self);
        out
    }

    /// Every node in the tree, parents before children, in the reference's
    /// `walkAST` order.
    pub fn walk(&self, visit: &mut impl FnMut(&Ast)) {
        crate::stack::grow_stack!(self.walk(visit));
        visit(self);
        match self {
            Ast::Comp {
                args, mapped_props, ..
            } => {
                args.iter().for_each(|a| a.walk(visit));
                if let Some(props) = mapped_props {
                    props.values().for_each(|a| a.walk(visit));
                }
            }
            Ast::Arr(els) => els.iter().for_each(|e| e.walk(visit)),
            Ast::Obj(entries) => entries.iter().for_each(|(_, v)| v.walk(visit)),
            Ast::BinOp { left, right, .. } => {
                left.walk(visit);
                right.walk(visit);
            }
            Ast::UnaryOp { operand, .. } => operand.walk(visit),
            Ast::Ternary {
                cond,
                then,
                otherwise,
            } => {
                cond.walk(visit);
                then.walk(visit);
                otherwise.walk(visit);
            }
            Ast::Member { obj, .. } => obj.walk(visit),
            Ast::Index { obj, index } => {
                obj.walk(visit);
                index.walk(visit);
            }
            Ast::Assign { value, .. } => value.walk(visit),
            _ => {}
        }
    }
}

pub(crate) fn write_ast(out: &mut String, node: &Ast) {
    crate::stack::grow_stack!(write_ast(out, node));
    let key = |out: &mut String, k: &str| {
        out.push_str(",\"");
        out.push_str(k);
        out.push_str("\":");
    };
    out.push_str("{\"k\":\"");
    match node {
        Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } => {
            out.push_str("Comp\"");
            key(out, "name");
            write_json_string(out, name);
            key(out, "args");
            write_list(out, args);
            if *function {
                key(out, "fn");
                out.push_str("true");
            }
            if let Some(props) = mapped_props {
                key(out, "mappedProps");
                out.push('{');
                for (i, (k, v)) in props.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(out, k);
                    out.push(':');
                    write_ast(out, v);
                }
                out.push('}');
            }
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
        Ast::RuntimeRef { n, ref_type } => {
            out.push_str("RuntimeRef\"");
            key(out, "n");
            write_json_string(out, n);
            key(out, "refType");
            write_json_string(out, ref_type.as_str());
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
