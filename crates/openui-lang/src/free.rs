//! Freeing trees without recursion (ADR 0002), so dropping a result needs no
//! stack however deep it is. Rust drops a tree by recursing into it, a few
//! frames per level, on whoever drops it; at a raised depth limit that would
//! outgrow any fixed thread stack.
//!
//! Instead, [`Value`] and [`Ast`] free themselves with a loop. The loop takes
//! uniquely owned children before dropping their parent. For a collection,
//! an owning iterator reuses its storage and resumes after each subtree, so
//! the pending list grows with branching depth rather than collection width.
//!
//! A weak reference to a child (the evaluator keeps them) makes
//! `Arc::get_mut` decline even for the child's last strong owner. The loop then
//! takes the payload with `Arc::into_inner`, which still succeeds for a last
//! owner, and frees it the same way; see `sole_payload`.

use std::sync::{Arc, OnceLock};

use crate::ast::Ast;
use crate::evaluate::Js;
use crate::js::JsObject;
use crate::materialize::{ErrorPart, UnresolvedPart};
use crate::result::{Element, Value};
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

/// Empty `Arc`s to leave in a parent while its child's payload is taken out
/// of an `Arc` that has weak references. Each kind is made once per process,
/// when first needed, and every taken `Arc` is replaced by a clone of it, so
/// a tree whose every level has a weak reference allocates once, not once per
/// level, and a drop that needs one allocates nothing after the first.
static VALUES: OnceLock<Arc<Vec<Value>>> = OnceLock::new();
static PROPS: OnceLock<Arc<JsObject<Value>>> = OnceLock::new();
static ELEMENT: OnceLock<Arc<Element>> = OnceLock::new();
static AST: OnceLock<Arc<Ast>> = OnceLock::new();
static EXPRESSIONS: OnceLock<Arc<Vec<Ast>>> = OnceLock::new();
static ENTRIES: OnceLock<Arc<Vec<(String, Ast)>>> = OnceLock::new();
static MAPPED_PROPS: OnceLock<Arc<JsObject<Ast>>> = OnceLock::new();
static JS_ITEMS: OnceLock<Arc<Vec<Js>>> = OnceLock::new();
static JS_OBJECT: OnceLock<Arc<JsObject<Js>>> = OnceLock::new();

/// The payload of an `Arc` that `Arc::get_mut` declined because of weak
/// references: its last strong owner takes it out, leaving a clone of `spare`
/// behind. Callers ask only when the payload has a child that may own more
/// (a collection of scalars drops in one frame, and the placeholder costs an
/// allocation). Any weak reference then dangles, as it would after the drop. An
/// `Arc` with another strong owner is not touched; the parent's drop only
/// releases this owner's reference, so a shared subtree stays whole for its
/// other owners.
fn sole_payload<T>(
    slot: &mut Arc<T>,
    spare: &'static OnceLock<Arc<T>>,
    empty: fn() -> T,
) -> Option<T> {
    if Arc::strong_count(slot) != 1 {
        return None;
    }
    let spare = Arc::clone(spare.get_or_init(|| Arc::new(empty())));
    Arc::into_inner(std::mem::replace(slot, spare))
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
            } else if items.iter().any(has_children)
                && let Some(mut items) = sole_payload(items, &VALUES, Vec::new)
            {
                if items.len() <= 1 {
                    rest.extend(items.pop().and_then(value_branch));
                } else {
                    Children::Values(items.into_iter()).advance(rest);
                }
            }
        }
        Value::Obj(object) => {
            if let Some(object) = Arc::get_mut(object) {
                Children::Props(object.take_values()).advance(rest);
            } else if object.iter_unordered().any(|(_, v)| has_children(v))
                && let Some(mut object) = sole_payload(object, &PROPS, JsObject::new)
            {
                Children::Props(object.take_values()).advance(rest);
            }
        }
        Value::Element(element) => {
            if let Some(element) = Arc::get_mut(element) {
                Children::Props(element.props.take_values()).advance(rest);
            } else if element.props.iter_unordered().any(|(_, v)| has_children(v))
                && let Some(mut element) = sole_payload(element, &ELEMENT, empty_element)
            {
                Children::Props(element.props.take_values()).advance(rest);
            }
        }
        Value::Ast(ast) => take_ast(ast, rest),
        Value::Null | Value::Bool(_) | Value::Num(_) | Value::Str(_) => {}
    }
}

fn empty_element() -> Element {
    Element {
        type_name: String::new(),
        props: JsObject::new(),
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    }
}

/// Move an expression's uniquely owned children onto `rest`.
fn ast_children(ast: &mut Ast, rest: &mut Vec<Node>) {
    match ast {
        Ast::Comp {
            args, mapped_props, ..
        } => {
            take_asts(args, rest);
            if let Some(props) = mapped_props {
                if let Some(props) = Arc::get_mut(props) {
                    Children::MappedProps(props.take_values()).advance(rest);
                } else if props.iter_unordered().any(|(_, ast)| !ast_is_leaf(ast))
                    && let Some(mut props) = sole_payload(props, &MAPPED_PROPS, JsObject::new)
                {
                    Children::MappedProps(props.take_values()).advance(rest);
                }
            }
        }
        Ast::Arr(items) => take_asts(items, rest),
        Ast::Obj(entries) => {
            if let Some(entries) = Arc::get_mut(entries) {
                Children::Entries(std::mem::take(entries).into_iter()).advance(rest);
            } else if entries.iter().any(|(_, ast)| !ast_is_leaf(ast))
                && let Some(entries) = sole_payload(entries, &ENTRIES, Vec::new)
            {
                Children::Entries(entries.into_iter()).advance(rest);
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

fn is_js_container(js: &Js) -> bool {
    matches!(js, Js::Arr(_) | Js::Obj(_))
}

fn js_children(js: &mut Js, rest: &mut Vec<Js>) {
    match js {
        Js::Arr(items) => {
            if let Some(items) = Arc::get_mut(items) {
                rest.append(items);
            } else if items.iter().any(is_js_container)
                && let Some(mut items) = sole_payload(items, &JS_ITEMS, Vec::new)
            {
                rest.append(&mut items);
            }
        }
        Js::Obj(object) => {
            if let Some(object) = Arc::get_mut(object) {
                rest.extend(object.drain_values());
            } else if object.iter_unordered().any(|(_, js)| is_js_container(js))
                && let Some(mut object) = sole_payload(object, &JS_OBJECT, JsObject::new)
            {
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
    } else if items.iter().any(|ast| !ast_is_leaf(ast))
        && let Some(items) = sole_payload(items, &EXPRESSIONS, Vec::new)
    {
        Children::Expressions(items.into_iter()).advance(rest);
    }
}

/// A uniquely owned child, swapped for a leaf so its `Arc` frees nothing deep.
fn take_ast(ast: &mut Arc<Ast>, rest: &mut Vec<Node>) {
    if let Some(ast) = Arc::get_mut(ast) {
        if !ast_is_leaf(ast) {
            rest.push(Node::Ast(std::mem::replace(ast, Ast::Null)));
        }
    } else if !ast_is_leaf(ast)
        && let Some(ast) = sole_payload(ast, &AST, || Ast::Null)
    {
        rest.push(Node::Ast(ast));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    use super::*;

    /// The evaluator keeps weak references to the arrays and objects it builds
    /// (`BufferOwner`), so a deep value of them must free without recursion
    /// with those references alive (#242).
    #[test]
    fn evaluator_values_with_weak_references_free_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let depth = 100_000;
                let mut arrays: Vec<Weak<Vec<Js>>> = Vec::new();
                let mut objects: Vec<Weak<JsObject<Js>>> = Vec::new();
                let mut array = Js::Num(0.0);
                let mut object = Js::Num(0.0);
                for _ in 0..depth {
                    let items = Arc::new(vec![array]);
                    arrays.push(Arc::downgrade(&items));
                    array = Js::Arr(items);
                    let mut fields = JsObject::new();
                    fields.set("k", object);
                    let fields = Arc::new(fields);
                    objects.push(Arc::downgrade(&fields));
                    object = Js::Obj(fields);
                }
                assert!(arrays.iter().all(|weak| weak.upgrade().is_some()));
                drop(array);
                drop(object);
                assert!(arrays.iter().all(|weak| weak.upgrade().is_none()));
                assert!(objects.iter().all(|weak| weak.upgrade().is_none()));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// A level another owner holds is not taken apart.
    #[test]
    fn a_shared_evaluator_level_survives_the_drop_of_its_other_owner() {
        let mut watch = Vec::new();
        let mut value = Js::Num(0.0);
        for _ in 0..1_000 {
            let items = Arc::new(vec![value]);
            watch.push(Arc::downgrade(&items));
            value = Js::Arr(items);
        }
        let kept = value.clone();
        drop(value);
        assert!(watch.iter().all(|weak| weak.upgrade().is_some()));
        drop(kept);
        assert!(watch.iter().all(|weak| weak.upgrade().is_none()));
    }
}
