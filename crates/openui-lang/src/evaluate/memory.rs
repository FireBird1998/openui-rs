//! Portable, conservative evaluation reservations (ADR 0011).
use super::*;

/// Memory accounting for one evaluation allowance, in portable reservation bytes.
/// Reservations live until the allowance ends, even if a value dies earlier.
/// This is not a measurement of the process heap. See [`crate::Limits::peak_memory`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvaluationMemory {
    /// Sum of successful reservations, also the allowance's accounted peak.
    pub accounted_peak: usize,
    /// A memory reservation was refused. The public size cut flag is also set.
    pub limited: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Account {
    limit: usize,
    pub(super) usage: EvaluationMemory,
}
impl Account {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            usage: EvaluationMemory {
                accounted_peak: 0,
                limited: false,
            },
        }
    }
}

// Fixed model weights; never Rust layout or allocator-request sizes.
const HEADER: usize = 64;
const SLOT: usize = 128;

#[inline]
pub(super) fn reserve_in(l: &EvalLimits, bytes: usize) -> bool {
    let Some(mut account) = l.memory.get() else {
        return true;
    };
    if account.usage.limited {
        l.cut_by(Cut::Memory);
        return false;
    }
    if l.cut.get().is_some() {
        return false;
    }
    match account.usage.accounted_peak.checked_add(bytes) {
        Some(total) if total <= account.limit => account.usage.accounted_peak = total,
        _ => {
            account.usage.limited = true;
            l.cut_by(Cut::Memory);
        }
    }
    l.memory.set(Some(account));
    !account.usage.limited
}

#[inline]
pub(super) fn reserve(bytes: usize) -> bool {
    LIMITS.with(|l| reserve_in(l, bytes))
}

pub(super) fn container(slots: usize) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match slots.checked_mul(SLOT).and_then(|n| n.checked_add(HEADER)) {
        Some(bytes) => reserve(bytes),
        None => overflow(),
    }
}

pub(super) fn text(bytes: usize) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match bytes.checked_add(HEADER) {
        Some(bytes) => reserve(bytes),
        None => overflow(),
    }
}

/// Reserve a shallow public value's potential copy, before allocating it.
/// Descendants are admitted by their own visits, not by a recursive prewalk.
pub(super) fn value(value: &Value) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match value {
        Value::Str(s) => text(s.len()),
        Value::Arr(v) => container(v.len()),
        Value::Obj(v) => {
            object(v)
                && v.values().all(|value| match value {
                    Value::Str(s) => text(s.len()),
                    _ => true,
                })
        }
        Value::Element(e) => element(e),
        Value::Ast(a) => ast(a),
        _ => reserve(SLOT),
    }
}

pub(super) fn js(value: &Js) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match value {
        Js::Str(s) => text(s.len()),
        Js::Arr(v) => container(v.len()),
        Js::Obj(v) => object(v),
        _ => reserve(SLOT),
    }
}

pub(super) fn object<T>(object: &JsObject<T>) -> bool {
    // No key walk whatsoever on the unset path.
    LIMITS.with(|l| {
        if l.memory.get().is_none() {
            return true;
        }
        container(object.len()) && object.iter().all(|(key, _)| reserve_in(l, key.len()))
    })
}

pub(super) fn ast(ast: &Ast) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match ast {
        Ast::Str(s) | Ast::Ref(s) | Ast::Ph(s) | Ast::StateRef(s) => text(s.len()),
        Ast::RuntimeRef { n, .. } => text(n.len()),
        Ast::Member { field, .. } => text(field.len()) && reserve(SLOT),
        Ast::Assign { target, .. } => text(target.len()) && reserve(SLOT),
        Ast::Arr(v) => container(v.len()),
        Ast::Obj(v) => LIMITS.with(|l| {
            if l.memory.get().is_none() {
                return true;
            }
            container(v.len()) && v.iter().all(|(k, _)| reserve_in(l, k.len()))
        }),
        Ast::Comp {
            name,
            args,
            mapped_props,
            ..
        } => {
            text(name.len())
                && container(args.len())
                && mapped_props.as_ref().is_none_or(|p| object(p))
        }
        _ => reserve(SLOT),
    }
}

pub(super) fn element(e: &Element) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    container(6)
        && text(e.type_name.len())
        && object(&e.props)
        // Element::clone copies directly owned scalar prop strings. Nested
        // Arc bodies remain caller-owned; their rebuilds admit separately.
        && e.props.values().all(|value| match value {
            Value::Str(s) => text(s.len()),
            _ => true,
        })
        && e.statement_id.as_ref().is_none_or(|id| text(id.len()))
}

pub(super) fn field(field: &Field<'_>) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match field {
        Field::Text(s) => text(s.len()),
        Field::Nodes(v) => container(v.len()) && v.iter().all(ast),
        Field::Entries(v) => {
            container(v.len().saturating_mul(3))
                && v.iter().all(|(key, node)| text(key.len()) && ast(node))
        }
        Field::Props(v) => object(v) && v.values().all(ast),
        _ => reserve(SLOT),
    }
}

fn overflow() -> bool {
    LIMITS.with(|l| {
        if let Some(mut account) = l.memory.get() {
            account.usage.limited = true;
            l.memory.set(Some(account));
            l.cut_by(Cut::Memory);
            false
        } else {
            true
        }
    })
}

// The temporary plain-data representation of syntax is built eagerly by
// json_shaped_ast. Admit its known shape first, without changing evaluation work.
pub(super) fn syntax(node: &Ast) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    crate::stack::grow_stack!(syntax(node), budget else false);
    let fields = fields(node);
    if !container(fields.len()) {
        return false;
    }
    fields.into_iter().all(|(key, field)| {
        text(key.len())
            && self::field(&field)
            && match field {
                Field::Node(node) => syntax(node),
                Field::Nodes(nodes) => nodes.iter().all(syntax),
                Field::Entries(entries) => entries.iter().all(|(_, node)| syntax(node)),
                Field::Props(props) => props.values().all(syntax),
                _ => true,
            }
    })
}

// Runtime own-fields helpers borrow objects but eagerly convert array/element/
// syntax fields. Admit their immediate owned copies, without traversing lazy
// descendants or changing their expression-depth semantics.
pub(super) fn object_fields(value: &Value) -> bool {
    if LIMITS.with(|l| l.memory.get().is_none()) {
        return true;
    }
    match value {
        Value::Arr(v) => {
            container(v.len())
                && v.iter().all(|value| match value {
                    Value::Str(s) => text(s.len()),
                    _ => true,
                })
        }
        Value::Ast(node) => {
            let fields = fields(node);
            container(fields.len()) && fields.iter().all(|(key, f)| text(key.len()) && field(f))
        }
        _ => self::value(value),
    }
}
