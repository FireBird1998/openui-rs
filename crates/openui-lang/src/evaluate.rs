//! State defaults evaluated while parsing, as 1.1's parser does with the
//! runtime evaluator: built-in functions, operators, member and index access,
//! ternaries and `@Each`, over data, with JavaScript's semantics. Not the
//! renderer's runtime: references to statements and queries read as null, and
//! a default that calls anything but a built-in function is null.
//!
//! Values are JavaScript's: `undefined` apart from `null`, arrays and objects
//! compared by identity, `==` with its coercions, strings indexed by UTF-16
//! unit. Where the core can't follow the reference (a property of a
//! prototype, string order outside ASCII), CONFORMANCE.md lists it.
//!
//! Evaluating keeps to the core's limits (see [`Limits`]): a default that
//! would build past what's left of `MAX_RESULT_SIZE`, or nest too deep, is
//! null instead.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use crate::MAX_DEPTH;
use crate::ast::Ast;
use crate::builtins::builtin_function_params;
use crate::js::{self, JsObject, OBJECT_PROTOTYPE_KEYS, array_index, number_to_string, parse_json};
use crate::result::{Value, write_value};
use crate::validation::{element_parts, is_ast_node};

/// A JavaScript value, as the evaluator sees one.
#[derive(Clone, Debug)]
pub(crate) enum Js {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    /// A string, with lone surrogates as the core carries them (see `js`).
    Str(Rc<str>),
    Arr(Rc<Vec<Js>>),
    Obj(Rc<JsObject<Js>>),
    /// A built-in method or constructor read off a value (`[].map`).
    Function {
        name: &'static str,
        constructor: bool,
    },
    /// A prototype, read through `__proto__`.
    Proto(Kind),
}

/// Which built-in prototype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Object,
    Array,
    String,
    Number,
    Boolean,
}

/// What a JavaScript object has from `String.prototype` and so on, as Bun has
/// it (`length` apart).
const STRING_METHODS: [&str; 50] = [
    "anchor",
    "at",
    "big",
    "blink",
    "bold",
    "charAt",
    "charCodeAt",
    "codePointAt",
    "concat",
    "endsWith",
    "fixed",
    "fontcolor",
    "fontsize",
    "includes",
    "indexOf",
    "isWellFormed",
    "italics",
    "lastIndexOf",
    "link",
    "localeCompare",
    "match",
    "matchAll",
    "normalize",
    "padEnd",
    "padStart",
    "repeat",
    "replace",
    "replaceAll",
    "search",
    "slice",
    "small",
    "split",
    "startsWith",
    "strike",
    "sub",
    "substr",
    "substring",
    "sup",
    "toLocaleLowerCase",
    "toLocaleUpperCase",
    "toLowerCase",
    "toString",
    "toUpperCase",
    "toWellFormed",
    "trim",
    "trimEnd",
    "trimLeft",
    "trimRight",
    "trimStart",
    "valueOf",
];
const ARRAY_METHODS: [&str; 38] = [
    "at",
    "concat",
    "copyWithin",
    "entries",
    "every",
    "fill",
    "filter",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "map",
    "pop",
    "push",
    "reduce",
    "reduceRight",
    "reverse",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "toLocaleString",
    "toReversed",
    "toSorted",
    "toSpliced",
    "toString",
    "unshift",
    "values",
    "with",
];
const NUMBER_METHODS: [&str; 6] = [
    "toExponential",
    "toFixed",
    "toLocaleString",
    "toPrecision",
    "toString",
    "valueOf",
];
const BOOLEAN_METHODS: [&str; 2] = ["toString", "valueOf"];

// ─── Limits ─────────────────────────────────────────────────────────────────

/// How deep the evaluator's walks may nest: as deep as a parse result's JSON
/// can, three levels per `MAX_DEPTH` level, so an evaluated default reads
/// back and serializes like any result (see `serialize::JSON_DEPTH_LIMIT`).
const MAX_VALUE_DEPTH: usize = 3 * MAX_DEPTH;

/// What evaluating defaults may still build, and whether it went past that.
///
/// A default can build far more than the program holds. One listing an
/// earlier one twice doubles with every line, and joining that into a string,
/// adding a string to itself or `@Each` copying its items doubles it again;
/// defaults reading defaults also nest without bound. The reference builds on
/// until the engine runs out of memory or stack. The core keeps all of a
/// parse's evaluating to what materializing left of `MAX_RESULT_SIZE`, and its
/// walks to `MAX_VALUE_DEPTH`. A default that goes past either is null, as is
/// every expression default after the budget runs out.
///
/// What draws on it: each expression evaluated, each string joined or added,
/// each value `@Each` turns into a literal, and each value turned back into
/// the result's. They reach it through a thread-local, set for one parse's
/// defaults at a time: evaluating is synchronous, and passing it through
/// every coercion would touch nearly every function here.
struct Limits {
    /// About how many bytes, or steps, are left.
    budget: Cell<usize>,
    /// Levels open across the walks.
    depth: Cell<usize>,
    /// The default being evaluated ran out of budget, or of depth.
    size_limited: Cell<bool>,
    depth_limited: Cell<bool>,
}

thread_local! {
    static LIMITS: Limits = const {
        Limits {
            budget: Cell::new(usize::MAX),
            depth: Cell::new(0),
            size_limited: Cell::new(false),
            depth_limited: Cell::new(false),
        }
    };
}

/// Spend `cost` of the budget: `false`, and the default is cut, once it's spent.
fn spend(cost: usize) -> bool {
    LIMITS.with(|l| match l.budget.get().checked_sub(cost) {
        Some(left) => {
            l.budget.set(left);
            true
        }
        None => {
            l.budget.set(0);
            l.size_limited.set(true);
            false
        }
    })
}

/// A level open in one of the walks, closed when dropped.
struct Level;

impl Level {
    /// `None`, and the default is cut, past `MAX_VALUE_DEPTH`.
    fn enter() -> Option<Level> {
        LIMITS.with(|l| {
            let depth = l.depth.get();
            if depth >= MAX_VALUE_DEPTH {
                l.depth_limited.set(true);
                return None;
            }
            l.depth.set(depth + 1);
            Some(Level)
        })
    }
}

impl Drop for Level {
    fn drop(&mut self) {
        LIMITS.with(|l| l.depth.set(l.depth.get() - 1));
    }
}

/// About what a value's own JSON costs, its items' and entries' apart.
fn json_cost(js: &Js) -> usize {
    match js {
        Js::Str(s) => s.len() + 2,
        Js::Arr(items) => items.len() + 2,
        Js::Obj(o) => o.iter().map(|(key, _)| key.len() + 4).sum::<usize>() + 2,
        Js::Num(n) => number_cost(*n),
        _ => 4,
    }
}

/// About how long a number's JSON is: an integer's digits, else the longest.
fn number_cost(n: f64) -> usize {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        let digits = if n == 0.0 {
            1
        } else {
            n.abs().log10() as usize + 1
        };
        digits + usize::from(n < 0.0)
    } else {
        24
    }
}

// ─── State defaults ─────────────────────────────────────────────────────────

/// A parse's state defaults, evaluated.
pub(crate) struct Evaluated {
    pub state: JsObject<Value>,
    /// Those that came out `undefined`: left out of the result, as JSON
    /// leaves them, but declared all the same.
    pub undefined: HashSet<String>,
    /// One went past the budget, or too deep, and is null.
    pub size_limited: bool,
    pub depth_limited: bool,
}

/// 1.1 evaluates expression defaults (`$total = 1 + 2`) once, in statement
/// order, each reading the defaults declared above it, building no more than
/// about `budget` bytes in all (see [`Limits`]).
pub(crate) fn state_defaults(state: JsObject<Value>, budget: usize) -> Evaluated {
    let mut out = Evaluated {
        state: JsObject::new(),
        undefined: HashSet::new(),
        size_limited: false,
        depth_limited: false,
    };
    if !state.values().any(has_runtime) {
        out.state = state;
        return out;
    }
    LIMITS.with(|l| {
        l.budget.set(budget);
        l.depth.set(0);
    });
    let mut declared = JsObject::new();
    for (id, value) in state.iter() {
        if !has_runtime(value) {
            out.state.set(id.clone(), value.clone());
            declared.define(id.clone(), from_value(value));
            continue;
        }
        LIMITS.with(|l| {
            l.size_limited.set(false);
            l.depth_limited.set(false);
        });
        let js = state_default(value, &declared);
        let converted = to_value(&js);
        let (size, depth) = LIMITS.with(|l| (l.size_limited.get(), l.depth_limited.get()));
        if size || depth {
            out.size_limited |= size;
            out.depth_limited |= depth;
            out.state.set(id.clone(), Value::Null);
            declared.define(id.clone(), Js::Null);
            continue;
        }
        match converted {
            Some(v) => out.state.set(id.clone(), v),
            None => {
                out.undefined.insert(id.clone());
            }
        }
        declared.define(id.clone(), js);
    }
    LIMITS.with(|l| l.budget.set(usize::MAX));
    out
}

/// Whether a materialized default holds anything to evaluate: a runtime
/// expression, in it or in its lists and plain objects.
pub(crate) fn has_runtime(value: &Value) -> bool {
    crate::stack::grow_stack!(has_runtime(value));
    match value {
        Value::Ast(_) => true,
        Value::Arr(items) => items.iter().any(has_runtime),
        Value::Obj(o) if element_parts(value).is_none() && !is_ast_node(value) => {
            o.values().any(has_runtime)
        }
        _ => false,
    }
}

/// `stateDefault`: the materialized default with its runtime expressions
/// evaluated, reading `declared`, the defaults declared before it. An object
/// literal shaped like an AST node isn't evaluated (a divergence).
fn state_default(value: &Value, declared: &JsObject<Js>) -> Js {
    crate::stack::grow_stack!(state_default(value, declared));
    match value {
        Value::Ast(ast) => {
            if only_builtin_functions(ast) {
                evaluate(ast, &Scope::Root(declared))
            } else {
                Js::Null
            }
        }
        Value::Arr(items) => Js::Arr(Rc::new(
            items.iter().map(|v| state_default(v, declared)).collect(),
        )),
        Value::Obj(o) if element_parts(value).is_none() && !is_ast_node(value) => {
            let mut out = JsObject::new();
            for (key, v) in o.iter() {
                out.define(key.clone(), state_default(v, declared));
            }
            Js::Obj(Rc::new(out))
        }
        _ => from_value(value),
    }
}

/// Whether every call in the expression is a built-in function: anything else
/// exists only in the client, so the default is null.
fn only_builtin_functions(node: &Ast) -> bool {
    let mut ok = true;
    node.walk(&mut |n| {
        if let Ast::Comp { name, .. } = n
            && builtin_function_params(name).is_none()
        {
            ok = false;
        }
    });
    ok
}

/// A parse result's value as a JavaScript value.
fn from_value(value: &Value) -> Js {
    crate::stack::grow_stack!(from_value(value));
    match value {
        Value::Null => Js::Null,
        Value::Bool(b) => Js::Bool(*b),
        Value::Num(n) => Js::Num(*n),
        Value::Str(s) => Js::Str(s.as_str().into()),
        Value::Arr(items) => Js::Arr(Rc::new(items.iter().map(from_value).collect())),
        Value::Obj(o) => {
            let mut out = JsObject::new();
            for (key, v) in o.iter() {
                out.define(key.clone(), from_value(v));
            }
            Js::Obj(Rc::new(out))
        }
        // As the object it is: its JSON, read back.
        Value::Element(_) | Value::Ast(_) => {
            let mut json = String::new();
            write_value(&mut json, value);
            from_json(&parse_json(&json).expect("the core writes JSON"))
        }
    }
}

fn from_json(json: &serde_json::Value) -> Js {
    crate::stack::grow_stack!(from_json(json));
    use serde_json::Value as Json;
    match json {
        Json::Null => Js::Null,
        Json::Bool(b) => Js::Bool(*b),
        Json::Number(n) => Js::Num(js::json_number(n)),
        Json::String(s) => Js::Str(s.as_str().into()),
        Json::Array(items) => Js::Arr(Rc::new(items.iter().map(from_json).collect())),
        Json::Object(map) => {
            let mut out = JsObject::new();
            for (key, v) in js::json_entries(map) {
                out.define(key.clone(), from_json(v));
            }
            Js::Obj(Rc::new(out))
        }
    }
}

/// The value as `JSON.stringify` writes it, or `None` where it leaves it
/// out (`undefined`, a function): a property is dropped, an array item is
/// null. Null where the limits cut it.
fn to_value(js: &Js) -> Option<Value> {
    crate::stack::grow_stack!(to_value(js));
    if matches!(js, Js::Undefined | Js::Function { .. }) {
        return None;
    }
    let Some(_level) = Level::enter() else {
        return Some(Value::Null);
    };
    if !spend(json_cost(js)) {
        return Some(Value::Null);
    }
    Some(match js {
        Js::Undefined | Js::Function { .. } => return None,
        Js::Null => Value::Null,
        Js::Bool(b) => Value::Bool(*b),
        Js::Num(n) => Value::Num(*n),
        Js::Str(s) => Value::Str(s.to_string()),
        Js::Arr(items) => Value::Arr(Arc::new(
            items
                .iter()
                .map(|v| to_value(v).unwrap_or(Value::Null))
                .collect(),
        )),
        Js::Obj(o) => {
            let mut out = JsObject::new();
            for (key, v) in o.iter() {
                if let Some(v) = to_value(v) {
                    out.define(key.clone(), v);
                }
            }
            Value::Obj(Arc::new(out))
        }
        // The boxed values `String.prototype` and the rest are.
        Js::Proto(kind) => match kind {
            Kind::Object => Value::Obj(Arc::new(JsObject::new())),
            Kind::Array => Value::Arr(Arc::new(Vec::new())),
            Kind::String => Value::Str(String::new()),
            Kind::Number => Value::Num(0.0),
            Kind::Boolean => Value::Bool(false),
        },
    })
}

// ─── Evaluating ─────────────────────────────────────────────────────────────

/// What names read: state from the defaults declared so far, and `@Each`'s
/// iterator variables. Other references are null here.
enum Scope<'a> {
    Root(&'a JsObject<Js>),
    Each {
        name: &'a str,
        item: &'a Js,
        parent: &'a Scope<'a>,
    },
}

impl Scope<'_> {
    /// `getState(name)`: `declared[name] ?? null`.
    fn state(&self, name: &str) -> Js {
        match self {
            Scope::Root(declared) => match declared.get(name) {
                Some(Js::Undefined | Js::Null) | None => Js::Null,
                Some(v) => v.clone(),
            },
            Scope::Each { parent, .. } => parent.state(name),
        }
    }

    /// `resolveRef(name)`.
    fn reference(&self, name: &str) -> Js {
        match self {
            Scope::Root(_) => Js::Null,
            Scope::Each {
                name: var, item, ..
            } if *var == name => (*item).clone(),
            Scope::Each { parent, .. } => parent.reference(name),
        }
    }
}

fn evaluate(node: &Ast, scope: &Scope) -> Js {
    crate::stack::grow_stack!(evaluate(node, scope));
    let Some(_level) = Level::enter() else {
        return Js::Null;
    };
    if !spend(1) {
        return Js::Null;
    }
    match node {
        Ast::Str(s) => Js::Str(s.as_str().into()),
        Ast::Num(n) => Js::Num(*n),
        Ast::Bool(b) => Js::Bool(*b),
        Ast::Null | Ast::Ph(_) => Js::Null,
        Ast::StateRef(name) => scope.state(name),
        Ast::Ref(name) | Ast::RuntimeRef { n: name, .. } => scope.reference(name),
        Ast::Arr(els) => Js::Arr(Rc::new(els.iter().map(|e| evaluate(e, scope)).collect())),
        Ast::Obj(entries) => {
            let mut out = JsObject::new();
            for (key, v) in entries.iter() {
                out.define(key.clone(), evaluate(v, scope));
            }
            Js::Obj(Rc::new(out))
        }
        Ast::Comp {
            name,
            args,
            mapped_props,
            ..
        } => {
            if name == "Each" {
                return each(args, scope);
            }
            let Some(params) = builtin_function_params(name) else {
                return Js::Null;
            };
            // `node.mappedProps ?? nameArgs(node.args, params)`, evaluated.
            let mut named = JsObject::new();
            match mapped_props {
                Some(props) => {
                    for (key, v) in props.iter() {
                        named.define(key.clone(), evaluate(v, scope));
                    }
                }
                None => {
                    for (param, arg) in params.iter().zip(args.iter()) {
                        named.define(*param, evaluate(arg, scope));
                    }
                }
            }
            call(name, &named)
        }
        Ast::BinOp { op, left, right } => binary(op, left, right, scope),
        Ast::UnaryOp { op, operand } => match *op {
            "!" => Js::Bool(!truthy(&evaluate(operand, scope))),
            "-" => Js::Num(-to_number(&evaluate(operand, scope))),
            _ => Js::Null,
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => {
            if truthy(&evaluate(cond, scope)) {
                evaluate(then, scope)
            } else {
                evaluate(otherwise, scope)
            }
        }
        Ast::Member { obj, field } => {
            let obj = evaluate(obj, scope);
            match &obj {
                Js::Undefined | Js::Null => Js::Null,
                // A field plucked from every item.
                Js::Arr(items) if field == "length" => Js::Num(items.len() as f64),
                Js::Arr(items) => Js::Arr(Rc::new(
                    items
                        .iter()
                        .map(|item| match property(item, field) {
                            Js::Undefined | Js::Null => Js::Null,
                            v => v,
                        })
                        .collect(),
                )),
                _ => property(&obj, field),
            }
        }
        Ast::Index { obj, index } => {
            let obj = evaluate(obj, scope);
            let index = evaluate(index, scope);
            if is_nullish(&obj) || is_nullish(&index) {
                return Js::Null;
            }
            match &obj {
                Js::Arr(items) => {
                    let n = to_number(&index);
                    if n.fract() == 0.0 && n >= 0.0 && n < items.len() as f64 {
                        items[n as usize].clone()
                    } else {
                        Js::Undefined
                    }
                }
                _ => property(&obj, &to_string(&index)),
            }
        }
        // A reactive binding, as data: `{__reactive: "assign", target, expr}`.
        Ast::Assign { target, value } => {
            let mut out = JsObject::new();
            out.define("__reactive", Js::Str("assign".into()));
            out.define("target", Js::Str(target.as_str().into()));
            out.define("expr", from_value(&Value::Ast(value.clone())));
            Js::Obj(Rc::new(out))
        }
    }
}

fn binary(op: &str, left: &Ast, right: &Ast, scope: &Scope) -> Js {
    let left = evaluate(left, scope);
    match op {
        "&&" => {
            return if truthy(&left) {
                evaluate(right, scope)
            } else {
                left
            };
        }
        "||" => {
            return if truthy(&left) {
                left
            } else {
                evaluate(right, scope)
            };
        }
        _ => {}
    }
    let right = evaluate(right, scope);
    match op {
        "+" if matches!(left, Js::Str(_)) || matches!(right, Js::Str(_)) => {
            let text = |v: &Js| {
                if is_nullish(v) {
                    String::new()
                } else {
                    to_string(v)
                }
            };
            let (left, right) = (text(&left), text(&right));
            if !spend(left.len() + right.len()) {
                return Js::Str("".into());
            }
            Js::Str(format!("{left}{right}").into())
        }
        "+" => Js::Num(to_number(&left) + to_number(&right)),
        "-" => Js::Num(to_number(&left) - to_number(&right)),
        "*" => Js::Num(to_number(&left) * to_number(&right)),
        // Division by zero is 0 by design.
        "/" | "%" => {
            let (a, b) = (to_number(&left), to_number(&right));
            Js::Num(match (b == 0.0, op) {
                (true, _) => 0.0,
                (false, "/") => a / b,
                (false, _) => a % b,
            })
        }
        "==" => Js::Bool(loose_eq(&left, &right)),
        "!=" => Js::Bool(!loose_eq(&left, &right)),
        ">" => Js::Bool(to_number(&left) > to_number(&right)),
        "<" => Js::Bool(to_number(&left) < to_number(&right)),
        ">=" => Js::Bool(to_number(&left) >= to_number(&right)),
        "<=" => Js::Bool(to_number(&left) <= to_number(&right)),
        _ => Js::Null,
    }
}

/// `@Each(array, varName, template)`: the template once per item, its
/// iterator variable replaced by the item's literal first.
fn each(args: &[Ast], scope: &Scope) -> Js {
    if args.len() < 3 {
        return Js::Arr(Rc::new(Vec::new()));
    }
    let Js::Arr(items) = evaluate(&args[0], scope) else {
        return Js::Arr(Rc::new(Vec::new()));
    };
    let var = match &args[1] {
        Ast::Ref(name) | Ast::Str(name) if !name.is_empty() => name.as_str(),
        _ => return Js::Arr(Rc::new(Vec::new())),
    };
    Js::Arr(Rc::new(
        items
            .iter()
            .map(|item| {
                let template = substitute(&args[2], var, item);
                let child = Scope::Each {
                    name: var,
                    item,
                    parent: scope,
                };
                evaluate(&template, &child)
            })
            .collect(),
    ))
}

/// `substituteRef`: the iterator variable replaced by the item's literal.
fn substitute(node: &Ast, var: &str, value: &Js) -> Ast {
    crate::stack::grow_stack!(substitute(node, var, value));
    let sub = |n: &Ast| substitute(n, var, value);
    let boxed = |n: &Ast| Arc::new(substitute(n, var, value));
    match node {
        Ast::Ref(name) if name == var => literal(value),
        Ast::Member { obj, field } => {
            let obj = sub(obj);
            if let Ast::Obj(entries) = &obj
                && let Some((_, v)) = entries.iter().find(|(k, _)| k == field)
            {
                return v.clone();
            }
            Ast::Member {
                obj: Arc::new(obj),
                field: field.clone(),
            }
        }
        Ast::Index { obj, index } => Ast::Index {
            obj: boxed(obj),
            index: boxed(index),
        },
        Ast::BinOp { op, left, right } => Ast::BinOp {
            op,
            left: boxed(left),
            right: boxed(right),
        },
        Ast::UnaryOp { op, operand } => Ast::UnaryOp {
            op,
            operand: boxed(operand),
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => Ast::Ternary {
            cond: boxed(cond),
            then: boxed(then),
            otherwise: boxed(otherwise),
        },
        Ast::Arr(els) => Ast::Arr(Arc::new(els.iter().map(sub).collect())),
        Ast::Obj(entries) => Ast::Obj(Arc::new(
            entries.iter().map(|(k, v)| (k.clone(), sub(v))).collect(),
        )),
        Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } => Ast::Comp {
            name: name.clone(),
            args: Arc::new(args.iter().map(sub).collect()),
            function: *function,
            mapped_props: mapped_props.as_ref().map(|props| {
                let mut out = JsObject::new();
                for (key, v) in props.iter() {
                    out.define(key.clone(), sub(v));
                }
                Arc::new(out)
            }),
        },
        Ast::Assign { target, value: v } => Ast::Assign {
            target: target.clone(),
            value: boxed(v),
        },
        _ => node.clone(),
    }
}

/// `toLiteralAST`; null where the limits cut it.
fn literal(value: &Js) -> Ast {
    crate::stack::grow_stack!(literal(value));
    let Some(_level) = Level::enter() else {
        return Ast::Null;
    };
    if !spend(json_cost(value)) {
        return Ast::Null;
    }
    match value {
        Js::Undefined | Js::Null | Js::Function { .. } => Ast::Null,
        Js::Str(s) => Ast::Str(s.to_string()),
        Js::Num(n) => Ast::Num(*n),
        Js::Bool(b) => Ast::Bool(*b),
        Js::Arr(items) => Ast::Arr(Arc::new(items.iter().map(literal).collect())),
        Js::Obj(o) => Ast::Obj(Arc::new(
            o.iter().map(|(k, v)| (k.clone(), literal(v))).collect(),
        )),
        // `Array.prototype` is an array; the others have no own entries.
        Js::Proto(Kind::Array) => Ast::Arr(Arc::new(Vec::new())),
        Js::Proto(_) => Ast::Obj(Arc::new(Vec::new())),
    }
}

// ─── The built-in functions ─────────────────────────────────────────────────

fn call(name: &str, args: &JsObject<Js>) -> Js {
    let arg = |key: &str| args.get(key).cloned().unwrap_or(Js::Undefined);
    let array = arg("array");
    let numbers = arg("numbers");
    match name {
        "Count" => Js::Num(match &array {
            Js::Arr(items) => items.len() as f64,
            _ => 0.0,
        }),
        "First" | "Last" => match &array {
            Js::Arr(items) => {
                let item = if name == "First" {
                    items.first()
                } else {
                    items.last()
                };
                match item {
                    None | Some(Js::Undefined | Js::Null) => Js::Null,
                    Some(v) => v.clone(),
                }
            }
            _ => Js::Null,
        },
        "Sum" => Js::Num(match &numbers {
            Js::Arr(items) => sum(items),
            _ => 0.0,
        }),
        "Avg" => Js::Num(match &numbers {
            Js::Arr(items) if !items.is_empty() => sum(items) / items.len() as f64,
            _ => 0.0,
        }),
        "Min" | "Max" => Js::Num(match &numbers {
            Js::Arr(items) if !items.is_empty() => {
                let pick = if name == "Min" { js_min } else { js_max };
                items
                    .iter()
                    .fold(to_number(&items[0]), |acc, v| pick(acc, to_number(v)))
            }
            _ => 0.0,
        }),
        "Sort" => sort(&array, &arg("field"), &arg("direction")),
        "Filter" => filter(&array, &arg("field"), &arg("operator"), &arg("value")),
        "Round" => {
            let decimals = arg("decimals");
            let d = if is_nullish(&decimals) {
                0.0
            } else {
                to_number(&decimals)
            };
            let factor = 10f64.powf(d);
            elementwise(&arg("number"), &|n| js_round(n * factor) / factor)
        }
        "Abs" => elementwise(&arg("number"), &f64::abs),
        "Floor" => elementwise(&arg("number"), &f64::floor),
        "Ceil" => elementwise(&arg("number"), &f64::ceil),
        _ => Js::Null,
    }
}

fn sum(items: &[Js]) -> f64 {
    items.iter().fold(0.0, |acc, v| acc + to_number(v))
}

/// `Math.min` and `Math.max`: NaN wins, and -0 is below +0.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_negative() { a } else { b }
    } else {
        a.min(b)
    }
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else {
        a.max(b)
    }
}

/// `Math.round`: to the nearest integer, halves up.
fn js_round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let floor = x.floor();
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    if rounded == 0.0 && x < 0.0 {
        -0.0
    } else {
        rounded
    }
}

/// A number function on a number, or on each item of a list, nested lists too.
fn elementwise(value: &Js, f: &dyn Fn(f64) -> f64) -> Js {
    crate::stack::grow_stack!(elementwise(value, f));
    match value {
        Js::Arr(items) => Js::Arr(Rc::new(items.iter().map(|v| elementwise(v, f)).collect())),
        v => Js::Num(f(to_number(v))),
    }
}

/// `resolveField`: a property, or a dot path of them.
fn resolve_field(obj: &Js, path: &str) -> Js {
    if path.is_empty() || is_nullish(obj) {
        return Js::Undefined;
    }
    if !path.contains('.') {
        return property(obj, path);
    }
    let mut current = obj.clone();
    for part in path.split('.') {
        if is_nullish(&current) {
            return Js::Undefined;
        }
        current = property(&current, part);
    }
    current
}

fn field_of(item: &Js, field: &str) -> Js {
    if field.is_empty() {
        item.clone()
    } else {
        resolve_field(item, field)
    }
}

fn text_or(value: &Js, otherwise: &str) -> String {
    if is_nullish(value) {
        otherwise.to_owned()
    } else {
        to_string(value)
    }
}

fn sort(array: &Js, field: &Js, direction: &Js) -> Js {
    let Js::Arr(items) = array else {
        return array.clone();
    };
    let field = text_or(field, "");
    let desc = text_or(direction, "asc") == "desc";
    let numeric = |v: &Js| match v {
        Js::Num(_) => true,
        Js::Str(s) => !s.is_empty() && !js_number(s).is_nan(),
        _ => false,
    };
    let compare = |a: &Js, b: &Js| -> Ordering {
        let (av, bv) = (field_of(a, &field), field_of(b, &field));
        let ordering = if numeric(&av) && numeric(&bv) {
            // A NaN difference counts as equal.
            (to_number(&av) - to_number(&bv))
                .partial_cmp(&0.0)
                .unwrap_or(Ordering::Equal)
        } else {
            locale_compare(&text_or(&av, ""), &text_or(&bv, ""))
        };
        if desc { ordering.reverse() } else { ordering }
    };
    // `undefined` items go last without being compared.
    let (defined, undefined): (Vec<Js>, Vec<Js>) = items
        .iter()
        .cloned()
        .partition(|v| !matches!(v, Js::Undefined));
    let mut sorted = js_sort(defined, &|a, b| compare(a, b) == Ordering::Less);
    sorted.extend(undefined);
    Js::Arr(Rc::new(sorted))
}

/// `Array.prototype.sort` as JavaScriptCore (Bun) does it, so that even a
/// comparison that isn't a total order (numbers and text mixed so that it
/// loops) gives the reference's order. Traced in Bun: binary insertion under 8
/// items; from 8 to 65, the first run (reversed if it descends strictly) and
/// binary insertion after it; past 65, runs merged. That last is a stable
/// merge sort here, which agrees whenever the comparison is consistent (a
/// divergence otherwise). It never panics, as `slice::sort_by` may.
fn js_sort(mut items: Vec<Js>, less: &dyn Fn(&Js, &Js) -> bool) -> Vec<Js> {
    let n = items.len();
    if n > 65 {
        return merge_sort(items, less);
    }
    if n < 2 {
        return items;
    }
    let start = if n < 8 {
        1
    } else {
        let mut run = 2;
        if less(&items[1], &items[0]) {
            while run < n && less(&items[run], &items[run - 1]) {
                run += 1;
            }
            items[..run].reverse();
        } else {
            while run < n && !less(&items[run], &items[run - 1]) {
                run += 1;
            }
        }
        run
    };
    for i in start..n {
        let (mut lo, mut hi) = (0, i);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if less(&items[i], &items[mid]) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let item = items.remove(i);
        items.insert(lo, item);
    }
    items
}

/// A bottom-up merge sort that takes the right item only when it's strictly
/// less, so it's stable. It never panics, whatever the comparison does.
fn merge_sort(items: Vec<Js>, less: &dyn Fn(&Js, &Js) -> bool) -> Vec<Js> {
    let n = items.len();
    let mut src = items;
    let mut width = 1;
    while width < n {
        let mut dst = Vec::with_capacity(n);
        let mut start = 0;
        while start < n {
            let (mut left, left_end) = (start, (start + width).min(n));
            let (mut right, right_end) = (left_end, (left_end + width).min(n));
            while left < left_end || right < right_end {
                if right < right_end && (left >= left_end || less(&src[right], &src[left])) {
                    dst.push(src[right].clone());
                    right += 1;
                } else {
                    dst.push(src[left].clone());
                    left += 1;
                }
            }
            start += 2 * width;
        }
        src = dst;
        width *= 2;
    }
    src
}

fn filter(array: &Js, field: &Js, operator: &Js, value: &Js) -> Js {
    crate::stack::grow_stack!(filter(array, field, operator, value));
    let Js::Arr(items) = array else {
        return Js::Arr(Rc::new(Vec::new()));
    };
    let field = text_or(field, "");
    let operator = text_or(operator, "==");
    let keep = |item: &Js| {
        let v = field_of(item, &field);
        match operator.as_str() {
            "==" => loose_eq(&v, value),
            "!=" => !loose_eq(&v, value),
            ">" => to_number(&v) > to_number(value),
            "<" => to_number(&v) < to_number(value),
            ">=" => to_number(&v) >= to_number(value),
            "<=" => to_number(&v) <= to_number(value),
            "contains" => text_or(&v, "").contains(&text_or(value, "")),
            _ => false,
        }
    };
    Js::Arr(Rc::new(items.iter().filter(|v| keep(v)).cloned().collect()))
}

// ─── JavaScript's semantics ─────────────────────────────────────────────────

fn is_nullish(v: &Js) -> bool {
    matches!(v, Js::Undefined | Js::Null)
}

fn truthy(v: &Js) -> bool {
    match v {
        Js::Undefined | Js::Null => false,
        Js::Bool(b) => *b,
        Js::Num(n) => *n != 0.0 && !n.is_nan(),
        Js::Str(s) => !s.is_empty(),
        _ => true,
    }
}

/// The built-ins' `toNumber`: numbers as they are, strings by `Number()` with
/// NaN as 0, booleans as 1 and 0, anything else 0.
fn to_number(v: &Js) -> f64 {
    match v {
        Js::Num(n) => *n,
        Js::Str(s) => {
            let n = js_number(s);
            if n.is_nan() { 0.0 } else { n }
        }
        Js::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

/// `Number(text)`.
pub(crate) fn js_number(text: &str) -> f64 {
    let t = js::trim(text);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let bytes = t.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            return radix_number(&t[2..], radix);
        }
    }
    // StrDecimalLiteral: a sign, digits with a point, an exponent.
    let rest = t.strip_prefix(['+', '-']).unwrap_or(t);
    let mut parts = rest.splitn(2, ['e', 'E']);
    let mantissa = parts.next().unwrap_or_default();
    let exponent_ok = parts.next().is_none_or(|e| {
        let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    });
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mantissa_ok = (!whole.is_empty() || !fraction.is_empty())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.bytes().all(|b| b.is_ascii_digit());
    if mantissa_ok && exponent_ok {
        t.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// Digits in a radix, as `Number("0x…")` reads them: NaN if any isn't one.
fn radix_number(digits: &str, radix: u32) -> f64 {
    let mut exact: u128 = 0;
    let mut approx = 0.0f64;
    let mut overflowed = false;
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else {
            return f64::NAN;
        };
        approx = approx * f64::from(radix) + f64::from(d);
        match exact
            .checked_mul(u128::from(radix))
            .and_then(|v| v.checked_add(u128::from(d)))
        {
            Some(v) => exact = v,
            None => overflowed = true,
        }
    }
    if overflowed { approx } else { exact as f64 }
}

/// `String(value)`.
fn to_string(v: &Js) -> String {
    crate::stack::grow_stack!(to_string(v));
    match v {
        Js::Undefined => "undefined".to_owned(),
        Js::Null => "null".to_owned(),
        Js::Bool(b) => b.to_string(),
        Js::Num(n) => number_to_string(*n),
        Js::Str(s) => s.to_string(),
        Js::Arr(items) => {
            let mut out = String::new();
            join(items, &mut out);
            out
        }
        Js::Obj(_) | Js::Proto(Kind::Object) => "[object Object]".to_owned(),
        Js::Function {
            name,
            constructor: true,
        } => format!("function {name}() {{ [native code] }}"),
        Js::Function { name, .. } => format!("function {name}() {{\n    [native code]\n}}"),
        Js::Proto(Kind::Array | Kind::String) => String::new(),
        Js::Proto(Kind::Number) => "0".to_owned(),
        Js::Proto(Kind::Boolean) => "false".to_owned(),
    }
}

/// `Array.prototype.join(",")` into `out`, nested arrays and all: a string
/// built from an array costs its length, and stops where the limits cut it.
fn join(items: &[Js], out: &mut String) {
    crate::stack::grow_stack!(join(items, out));
    let Some(_level) = Level::enter() else {
        return;
    };
    if !spend(1) {
        return;
    }
    for (i, item) in items.iter().enumerate() {
        if i > 0 && !push(out, ",") {
            return;
        }
        match item {
            Js::Undefined | Js::Null => {}
            Js::Arr(inner) => join(inner, out),
            _ => {
                if !push(out, &to_string(item)) {
                    return;
                }
            }
        }
    }
}

fn push(out: &mut String, text: &str) -> bool {
    if !spend(text.len()) {
        return false;
    }
    out.push_str(text);
    true
}

/// `ToPrimitive` of an object, as `==` takes it.
fn to_primitive(v: &Js) -> Js {
    match v {
        Js::Proto(Kind::Number) => Js::Num(0.0),
        Js::Proto(Kind::Boolean) => Js::Bool(false),
        Js::Arr(_) | Js::Obj(_) | Js::Function { .. } | Js::Proto(_) => {
            Js::Str(to_string(v).into())
        }
        _ => v.clone(),
    }
}

fn is_object(v: &Js) -> bool {
    matches!(
        v,
        Js::Arr(_) | Js::Obj(_) | Js::Function { .. } | Js::Proto(_)
    )
}

/// `a == b`.
fn loose_eq(a: &Js, b: &Js) -> bool {
    crate::stack::grow_stack!(loose_eq(a, b));
    match (a, b) {
        (Js::Undefined | Js::Null, Js::Undefined | Js::Null) => true,
        (Js::Undefined | Js::Null, _) | (_, Js::Undefined | Js::Null) => false,
        (Js::Num(x), Js::Num(y)) => x == y,
        (Js::Str(x), Js::Str(y)) => x == y,
        (Js::Bool(x), Js::Bool(y)) => x == y,
        (Js::Num(x), Js::Str(s)) | (Js::Str(s), Js::Num(x)) => *x == js_number(s),
        (Js::Bool(x), other) | (other, Js::Bool(x)) => {
            loose_eq(&Js::Num(f64::from(u8::from(*x))), other)
        }
        (Js::Arr(x), Js::Arr(y)) => Rc::ptr_eq(x, y),
        (Js::Obj(x), Js::Obj(y)) => Rc::ptr_eq(x, y),
        (
            Js::Function {
                name: x,
                constructor: cx,
            },
            Js::Function {
                name: y,
                constructor: cy,
            },
        ) => x == y && cx == cy,
        (Js::Proto(x), Js::Proto(y)) => x == y,
        (x, y) if is_object(x) && is_object(y) => false,
        (x, y) if is_object(x) => loose_eq(&to_primitive(x), y),
        (x, y) => loose_eq(x, &to_primitive(y)),
    }
}

/// The string's UTF-16 units, lone surrogates included.
fn utf16(s: &str) -> Vec<u16> {
    let mut units = Vec::with_capacity(s.len());
    for c in s.chars() {
        match js::lone_surrogate_unit(c) {
            Some(unit) => units.push(unit),
            None => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
        }
    }
    units
}

/// One UTF-16 unit as a string: a lone surrogate if it's half of a pair.
fn unit_string(unit: u16) -> String {
    match char::from_u32(u32::from(unit)) {
        Some(c) => c.to_string(),
        None => js::lone_surrogate(unit).to_string(),
    }
}

/// `value[key]`, the key a string.
fn property(value: &Js, key: &str) -> Js {
    let inherited = |kind: Kind, methods: &[&'static str]| -> Js {
        if key == "__proto__" {
            return Js::Proto(kind);
        }
        if key == "constructor" {
            let name = match kind {
                Kind::Object => "Object",
                Kind::Array => "Array",
                Kind::String => "String",
                Kind::Number => "Number",
                Kind::Boolean => "Boolean",
            };
            return Js::Function {
                name,
                constructor: true,
            };
        }
        match methods
            .iter()
            .chain(OBJECT_PROTOTYPE_KEYS.iter())
            .find(|&&m| m == key)
        {
            Some(&name) => Js::Function {
                name,
                constructor: false,
            },
            None => Js::Undefined,
        }
    };
    match value {
        Js::Obj(o) => match o.get(key) {
            Some(v) => v.clone(),
            None => inherited(Kind::Object, &[]),
        },
        Js::Arr(items) => {
            if key == "length" {
                return Js::Num(items.len() as f64);
            }
            match array_index(key) {
                Some(i) => items.get(i as usize).cloned().unwrap_or(Js::Undefined),
                None => inherited(Kind::Array, &ARRAY_METHODS),
            }
        }
        Js::Str(s) => {
            let units = utf16(s);
            if key == "length" {
                return Js::Num(units.len() as f64);
            }
            match array_index(key) {
                Some(i) => match units.get(i as usize) {
                    Some(&unit) => Js::Str(unit_string(unit).into()),
                    None => Js::Undefined,
                },
                None => inherited(Kind::String, &STRING_METHODS),
            }
        }
        Js::Num(_) => inherited(Kind::Number, &NUMBER_METHODS),
        Js::Bool(_) => inherited(Kind::Boolean, &BOOLEAN_METHODS),
        // Properties of functions and prototypes: a divergence.
        Js::Undefined | Js::Null | Js::Function { .. } | Js::Proto(_) => Js::Undefined,
    }
}

/// Weights for ASCII in ICU's root collation, which `localeCompare` uses:
/// the controls ignorable (0), then whitespace, punctuation and symbols in
/// ICU's order, digits, and letters with either case alike. Case is the
/// tie-breaker, lowercase first.
fn ascii_weight(c: u8) -> u16 {
    const ORDER: &[u8] =
        b"\t\n\x0b\x0c\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";
    let folded = c.to_ascii_lowercase();
    match ORDER.iter().position(|&o| o == folded) {
        Some(i) => i as u16 + 1,
        None => 0,
    }
}

/// `a.localeCompare(b)` for ASCII text, exactly; text with other characters
/// compares by code point after the ASCII weights, which ICU doesn't (a
/// divergence).
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    let key = |s: &str| -> (Vec<u32>, Vec<bool>) {
        let mut primary = Vec::new();
        let mut upper = Vec::new();
        for c in s.chars() {
            if c.is_ascii() {
                let w = ascii_weight(c as u8);
                if w == 0 {
                    continue;
                }
                primary.push(u32::from(w));
                upper.push(c.is_ascii_uppercase());
            } else {
                primary.push(0x1_0000 + c as u32);
                upper.push(false);
            }
        }
        (primary, upper)
    };
    let (pa, ua) = key(a);
    let (pb, ub) = key(b);
    pa.cmp(&pb).then_with(|| ua.cmp(&ub))
}
