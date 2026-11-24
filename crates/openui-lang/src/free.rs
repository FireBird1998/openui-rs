//! Freeing trees without recursion (ADR 0002), so dropping a result needs no
//! stack however deep it is. Rust drops a tree by recursing into it, a few
//! frames per level, on whoever drops it; at a raised depth limit that would
//! outgrow any fixed thread stack.
//!
//! Instead, [`Value`] and [`Ast`] free themselves with a loop. The loop takes
//! uniquely owned children before dropping their parent. For a collection,
//! an owning iterator reuses its storage and resumes after each subtree, so
//! the pending list grows with branching depth rather than collection width.

use std::sync::Arc;

use crate::ast::Ast;
use crate::evaluate::Js;
use crate::materialize::{ErrorPart, UnresolvedPart};
use crate::result::Value;
use crate::table::Reach;

/// A node waiting to be freed.
enum Node {
    Value(Value),
    Ast(Ast),
    Children(Children),
}

/// The unvisited part of a collection, still in its original allocation.
enum Children {
    Values(std::vec::IntoIter<Value>),
    Props(indexmap::map::IntoValues<String, Value>),
    Expressions(std::vec::IntoIter<Ast>),
    Entries(std::vec::IntoIter<(String, Ast)>),
    MappedProps(indexmap::map::IntoValues<String, Ast>),
}

impl Children {
    fn advance(self, rest: &mut Vec<Node>) {
        match self {
            Self::Values(items) => take_branch(items, rest, Self::Values, value_branch),
            Self::Props(props) => take_branch(props, rest, Self::Props, value_branch),
            Self::Expressions(items) => take_branch(items, rest, Self::Expressions, ast_branch),
            Self::Entries(entries) => {
                take_branch(entries, rest, Self::Entries, |(_, ast)| ast_branch(ast));
            }
            Self::MappedProps(props) => take_branch(props, rest, Self::MappedProps, ast_branch),
        }
    }
}

/// Drop leaves in place. Save the iterator below the next branch so its
/// subtree is freed before taking another child.
fn take_branch<I>(
    mut children: I,
    rest: &mut Vec<Node>,
    resume: impl FnOnce(I) -> Children,
    branch: impl FnMut(I::Item) -> Option<Node>,
) where
    I: DoubleEndedIterator + ExactSizeIterator,
{
    if let Some(node) = children.by_ref().rev().find_map(branch) {
        if children.len() > 0 {
            rest.push(Node::Children(resume(children)));
        }
        rest.push(node);
    }
}

fn value_branch(value: Value) -> Option<Node> {
    has_children(&value).then_some(Node::Value(value))
}

fn ast_branch(ast: Ast) -> Option<Node> {
    (!ast_is_leaf(&ast)).then_some(Node::Ast(ast))
}

/// Free everything below a node, one node at a time.
fn free(take: impl FnOnce(&mut Vec<Node>)) {
    let mut rest = Vec::new();
    take(&mut rest);
    while let Some(node) = rest.pop() {
        match node {
            Node::Value(mut value) => value_children(&mut value, &mut rest),
            Node::Ast(mut ast) => ast_children(&mut ast, &mut rest),
            Node::Children(children) => children.advance(&mut rest),
        }
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        if has_children(self) {
            free(|rest| value_children(self, rest));
        }
    }
}

impl Drop for Ast {
    fn drop(&mut self) {
        if !ast_is_leaf(self) {
            free(|rest| ast_children(self, rest));
        }
    }
}

/// Whether a value may own children: anything that isn't a plain scalar.
fn has_children(value: &Value) -> bool {
    !matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Num(_) | Value::Str(_)
    )
}

fn ast_is_leaf(ast: &Ast) -> bool {
    matches!(
        ast,
        Ast::Str(_)
            | Ast::Num(_)
            | Ast::Bool(_)
            | Ast::Null
            | Ast::Ref(_)
            | Ast::Ph(_)
            | Ast::StateRef(_)
            | Ast::RuntimeRef { .. }
    )
}

/// Move a value's uniquely owned children onto `rest`.
fn value_children(value: &mut Value, rest: &mut Vec<Node>) {
    match value {
        Value::Arr(items) => {
            if let Some(items) = Arc::get_mut(items) {
                // A single child needs no iterator to resume.
                if items.len() <= 1 {
                    rest.extend(items.pop().and_then(value_branch));
                } else {
                    Children::Values(std::mem::take(items).into_iter()).advance(rest);
                }
            }
        }
        Value::Obj(object) => {
            if let Some(object) = Arc::get_mut(object) {
                Children::Props(object.take_values()).advance(rest);
            }
        }
        Value::Element(element) => {
            if let Some(element) = Arc::get_mut(element) {
                Children::Props(element.props.take_values()).advance(rest);
            }
        }
        Value::Ast(ast) => take_ast(ast, rest),
        Value::Null | Value::Bool(_) | Value::Num(_) | Value::Str(_) => {}
    }
}

/// Move an expression's uniquely owned children onto `rest`.
fn ast_children(ast: &mut Ast, rest: &mut Vec<Node>) {
    match ast {
        Ast::Comp {
            args, mapped_props, ..
        } => {
            take_asts(args, rest);
            if let Some(props) = mapped_props
                && let Some(props) = Arc::get_mut(props)
            {
                Children::MappedProps(props.take_values()).advance(rest);
            }
        }
        Ast::Arr(items) => take_asts(items, rest),
        Ast::Obj(entries) => {
            if let Some(entries) = Arc::get_mut(entries) {
                Children::Entries(std::mem::take(entries).into_iter()).advance(rest);
            }
        }
        Ast::BinOp { left, right, .. } => {
            take_ast(left, rest);
            take_ast(right, rest);
        }
        Ast::UnaryOp { operand, .. } => take_ast(operand, rest),
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => {
            take_ast(cond, rest);
            take_ast(then, rest);
            take_ast(otherwise, rest);
        }
        Ast::Member { obj, .. } => take_ast(obj, rest),
        Ast::Index { obj, index } => {
            take_ast(obj, rest);
            take_ast(index, rest);
        }
        Ast::Assign { value, .. } => take_ast(value, rest),
        Ast::Str(_)
        | Ast::Num(_)
        | Ast::Bool(_)
        | Ast::Null
        | Ast::Ref(_)
        | Ast::Ph(_)
        | Ast::StateRef(_)
        | Ast::RuntimeRef { .. } => {}
    }
}

/// Free a `serde_json::Value` with a loop, so it needs no stack however deep
/// it nests.
///
/// serde_json's own drop recurses, a few frames per level, on whoever drops
/// the value. That's fine at the default depth limit (a value from
/// [`crate::parse_json`] nests at most 800 levels, and drops in under 64 KB
/// of stack optimized), but a value read under a raised depth limit
/// ([`crate::parse_json_with_limits`]) can outgrow a small thread's stack.
/// Free such a value through here instead of dropping it. (The core itself
/// never reads JSON back while parsing.)
pub fn free_json(json: serde_json::Value) {
    use serde_json::Value as Json;
    let mut rest = vec![json];
    while let Some(mut json) = rest.pop() {
        match &mut json {
            Json::Array(items) => rest.append(items),
            Json::Object(fields) => {
                rest.extend(std::mem::take(fields).into_iter().map(|(_, v)| v));
            }
            _ => {}
        }
    }
}

/// The evaluator's values nest as deep as an evaluated default does, three
/// levels per level of the depth limit, and are dropped inside the parse:
/// the same loop frees them.
impl Drop for Js {
    fn drop(&mut self) {
        if !matches!(self, Js::Arr(_) | Js::Obj(_)) {
            return;
        }
        let mut rest = Vec::new();
        js_children(self, &mut rest);
        while let Some(mut js) = rest.pop() {
            js_children(&mut js, &mut rest);
        }
    }
}

fn js_children(js: &mut Js, rest: &mut Vec<Js>) {
    match js {
        Js::Arr(items) => {
            if let Some(items) = Arc::get_mut(items) {
                rest.append(items);
            }
        }
        Js::Obj(object) => {
            if let Some(object) = Arc::get_mut(object) {
                rest.extend(object.drain_values());
            }
        }
        _ => {}
    }
}

/// A statement's errors nest one level per statement reached through it, and
/// a reference chain can be as long as the depth limit.
impl Drop for ErrorPart {
    fn drop(&mut self) {
        let mut rest = Vec::new();
        error_children(self, &mut rest);
        while let Some(mut part) = rest.pop() {
            error_children(&mut part, &mut rest);
        }
    }
}

fn error_children(part: &mut ErrorPart, rest: &mut Vec<ErrorPart>) {
    if let ErrorPart::Reached { parts, .. } = part
        && let Some(parts) = Arc::get_mut(parts)
    {
        rest.append(parts);
    }
}

/// So do its unresolved names.
impl Drop for UnresolvedPart {
    fn drop(&mut self) {
        let mut rest = Vec::new();
        unresolved_children(self, &mut rest);
        while let Some(mut part) = rest.pop() {
            unresolved_children(&mut part, &mut rest);
        }
    }
}

fn unresolved_children(part: &mut UnresolvedPart, rest: &mut Vec<UnresolvedPart>) {
    if let UnresolvedPart::Reached(parts) = part
        && let Some(parts) = Arc::get_mut(parts)
    {
        rest.append(parts);
    }
}

/// The statements reached nest the same way.
impl Drop for Reach {
    fn drop(&mut self) {
        let mut rest = std::mem::take(&mut self.children);
        while let Some(mut reach) = rest.pop() {
            if let Some(reach) = Arc::get_mut(&mut reach) {
                rest.append(&mut reach.children);
            }
        }
    }
}

fn take_asts(items: &mut Arc<Vec<Ast>>, rest: &mut Vec<Node>) {
    if let Some(items) = Arc::get_mut(items) {
        Children::Expressions(std::mem::take(items).into_iter()).advance(rest);
    }
}

/// A uniquely owned child, swapped for a leaf so its `Arc` frees nothing deep.
fn take_ast(ast: &mut Arc<Ast>, rest: &mut Vec<Node>) {
    if let Some(ast) = Arc::get_mut(ast)
        && !ast_is_leaf(ast)
    {
        rest.push(Node::Ast(std::mem::replace(ast, Ast::Null)));
    }
}
