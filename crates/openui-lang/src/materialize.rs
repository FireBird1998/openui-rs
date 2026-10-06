//! Lowering expression trees to elements and plain values: resolving
//! references, mapping positional args to named props, applying defaults and
//! reporting invalid props, as the reference parser's `materializeValue` and
//! `materializeExpr`.

use std::collections::HashSet;

use indexmap::{IndexMap, IndexSet};

use crate::MAX_DEPTH;
use crate::ast::{Ast, RefType};
use crate::builtins::{is_builtin, is_lazy_builtin, is_reserved_call};
use crate::js::JsObject;
use crate::library::{Library, Param};
use crate::parser::Statement;
use crate::result::{Element, ValidationError, Value};
use crate::validation::{
    Issue, element_parts, is_ast_node, params_signature, resolve_invalid_value,
    validate_schema_value,
};

pub(crate) struct Ctx<'a> {
    pub statements: &'a IndexMap<String, Statement>,
    pub library: &'a Library,
    pub errors: Vec<ValidationError>,
    pub unresolved: Vec<String>,
    /// Statements being materialized right now, to break reference cycles.
    pub visited: HashSet<String>,
    pub partial: bool,
    /// The statement errors are attributed to.
    pub current_statement_id: Option<String>,
    /// Value statements not reached yet; what's left at the end is orphaned.
    pub unreached: IndexSet<String>,
    /// Open materialize calls, across references too: a chain of references
    /// nests as deeply as one big expression.
    pub depth: usize,
    /// Went past `MAX_DEPTH`; the deeper parts became null.
    pub depth_limited: bool,
}

impl Ctx<'_> {
    pub(crate) fn push_issue(&mut self, component: &str, path: &str, issue: Issue) {
        self.errors.push(ValidationError {
            code: issue.code(),
            component: component.to_owned(),
            path: path.to_owned(),
            message: issue.message(component, path),
            statement_id: self.current_statement_id.clone(),
        });
    }

    fn unknown_component(&mut self, name: &str) {
        let library = self.library;
        let available = library.component_names().map(str::to_owned).collect();
        self.push_issue(name, "", Issue::UnknownComponent { available });
    }
}

/// Iterator variables in scope inside `@Each` templates.
type Scope = HashSet<String>;

/// Lower a node to what ends up in the tree: elements, plain values, and AST
/// for builtins and runtime expressions; `Null` for placeholders and for
/// components that had to be dropped. Past `MAX_DEPTH`, `Null`.
pub(crate) fn materialize_value(node: &Ast, ctx: &mut Ctx) -> Value {
    if ctx.depth >= MAX_DEPTH {
        ctx.depth_limited = true;
        return Value::Null;
    }
    ctx.depth += 1;
    let value = materialize_value_at_depth(node, ctx);
    ctx.depth -= 1;
    value
}

// The functions on the recursive path stay small, and the heavier
// non-recursive work lives in `#[inline(never)]` helpers. In unoptimized
// builds a frame holds every temporary its function could need, so this is
// what keeps MAX_DEPTH levels inside a small thread stack.
fn materialize_value_at_depth(node: &Ast, ctx: &mut Ctx) -> Value {
    match node {
        Ast::Ref(name) => match resolve_ref(name, ctx, Mode::Value) {
            Resolved::Value(v) => v,
            Resolved::Ast(a) => Value::Ast(Box::new(a)),
        },
        Ast::Str(v) => Value::Str(v.clone()),
        Ast::Num(n) => Value::Num(*n),
        Ast::Bool(b) => Value::Bool(*b),
        Ast::Null | Ast::Ph(_) => Value::Null,
        Ast::Arr(els) => materialize_array(els, ctx),
        Ast::Obj(entries) => materialize_object(entries, ctx),
        Ast::Comp {
            name,
            args,
            mapped_props,
        } => materialize_comp(name, args, mapped_props, ctx),
        // Runtime expressions stay AST, with their insides normalized.
        _ => Value::Ast(Box::new(materialize_expr(node, ctx, &Scope::new()))),
    }
}

fn materialize_array(els: &[Ast], ctx: &mut Ctx) -> Value {
    let mut items = Vec::new();
    for el in els {
        if matches!(el, Ast::Ph(_)) {
            continue; // unresolved placeholders drop out of arrays
        }
        let value = materialize_value(el, ctx);
        // So do components and refs that came out null (dropped, unresolved, unknown).
        if matches!(value, Value::Null) && matches!(el, Ast::Comp { .. } | Ast::Ref(_)) {
            continue;
        }
        items.push(value);
    }
    Value::Arr(items)
}

fn materialize_object(entries: &[(String, Ast)], ctx: &mut Ctx) -> Value {
    let mut obj = JsObject::new();
    for (key, value) in entries {
        let value = materialize_value(value, ctx);
        obj.set(key.clone(), value);
    }
    Value::Obj(obj)
}

#[derive(Clone, Copy)]
enum Mode {
    Value,
    Expr,
}

enum Resolved {
    Value(Value),
    Ast(Ast),
}

/// Inline the referenced statement. Unknown or cyclic refs are recorded as
/// unresolved; refs to Query/Mutation statements become `RuntimeRef`s.
fn resolve_ref(name: &str, ctx: &mut Ctx, mode: Mode) -> Resolved {
    let statements = ctx.statements;
    let target = match statements.get(name) {
        Some(stmt) if !ctx.visited.contains(name) => stmt.expr(),
        _ => {
            ctx.unresolved.push(name.to_owned());
            return match mode {
                Mode::Expr => Resolved::Ast(Ast::Ph(name.to_owned())),
                Mode::Value => Resolved::Value(Value::Null),
            };
        }
    };
    ctx.unreached.shift_remove(name);
    if let Ast::Comp { name: callee, .. } = target
        && is_reserved_call(callee)
    {
        let ref_type = if callee == "Mutation" {
            RefType::Mutation
        } else {
            RefType::Query
        };
        let node = Ast::RuntimeRef {
            n: name.to_owned(),
            ref_type,
        };
        return match mode {
            Mode::Expr => Resolved::Ast(node),
            Mode::Value => Resolved::Value(Value::Ast(Box::new(node))),
        };
    }
    ctx.visited.insert(name.to_owned());
    let previous = ctx.current_statement_id.replace(name.to_owned());
    let result = match mode {
        Mode::Value => {
            let mut value = materialize_value(target, ctx);
            tag_statement(&mut value, name);
            Resolved::Value(value)
        }
        Mode::Expr => Resolved::Ast(materialize_expr(target, ctx, &Scope::new())),
    };
    ctx.current_statement_id = previous;
    ctx.visited.remove(name);
    result
}

/// `result.statementId = name` on an element (or anything shaped like one).
pub(crate) fn tag_statement(value: &mut Value, name: &str) {
    if element_parts(value).is_none() {
        return;
    }
    match value {
        Value::Element(e) => e.statement_id = Some(name.to_owned()),
        Value::Obj(o) => o.set("statementId", Value::Str(name.to_owned())),
        _ => {}
    }
}

fn materialize_comp(
    name: &str,
    args: &[Ast],
    mapped_props: &Option<JsObject<Ast>>,
    ctx: &mut Ctx,
) -> Value {
    if is_builtin(name) {
        return Value::Ast(Box::new(builtin_call(name, args, mapped_props, ctx)));
    }
    if is_reserved_call(name) {
        return report(ctx, name, Issue::InlineReserved);
    }
    let library = ctx.library;
    let Some(def) = library.component(name) else {
        return unknown_component(ctx, name);
    };
    let mut props = JsObject::new();
    // Set when a required prop is invalid with no default to fall back on.
    let mut drop_component = false;
    for (param, arg) in def.params.iter().zip(args) {
        let value = materialize_value(arg, ctx);
        drop_component |= add_prop(&mut props, param, value, name, ctx);
    }
    finish_component(name, &def.params, args.len(), props, drop_component, ctx)
}

/// Builtins stay AST for the runtime.
fn builtin_call(
    name: &str,
    args: &[Ast],
    mapped_props: &Option<JsObject<Ast>>,
    ctx: &mut Ctx,
) -> Ast {
    if let Some(lazy) = materialize_lazy_builtin(name, args, mapped_props, ctx, &Scope::new()) {
        return lazy;
    }
    Ast::Comp {
        name: name.to_owned(),
        args: expr_list(args, ctx, &Scope::new()),
        mapped_props: mapped_props.clone(),
    }
}

#[inline(never)]
fn report(ctx: &mut Ctx, name: &str, issue: Issue) -> Value {
    ctx.push_issue(name, "", issue);
    Value::Null
}

#[inline(never)]
fn unknown_component(ctx: &mut Ctx, name: &str) -> Value {
    ctx.unknown_component(name);
    Value::Null
}

/// Validate one prop and store it. True if it's required, invalid, and has no
/// default, which drops the component.
#[inline(never)]
fn add_prop(
    props: &mut JsObject<Value>,
    param: &Param,
    mut value: Value,
    component: &str,
    ctx: &mut Ctx,
) -> bool {
    let path = format!("/{}", param.name);
    let invalid = validate_schema_value(&mut value, &param.schema, component, &path, ctx);
    props.set(param.name.clone(), value);
    invalid && resolve_invalid_value(props, &param.name, param.required, param.default.as_ref())
}

/// Excess args, missing required props (defaults first), and the element.
#[inline(never)]
fn finish_component(
    name: &str,
    params: &[Param],
    arg_count: usize,
    mut props: JsObject<Value>,
    drop_component: bool,
    ctx: &mut Ctx,
) -> Value {
    if arg_count > params.len() {
        let issue = Issue::ExcessArgs {
            declared: params.len(),
            got: arg_count,
        };
        ctx.push_issue(name, "", issue);
    }

    // Missing or null required props take their default; without one, the
    // component is dropped.
    let missing: Vec<&Param> = params
        .iter()
        .filter(|p| {
            p.required && (!props.has(&p.name) || matches!(props.get(&p.name), Some(Value::Null)))
        })
        .collect();
    let mut still_missing = Vec::new();
    for p in missing {
        match &p.default {
            Some(default) => props.set(p.name.clone(), Value::from_json(default)),
            None => still_missing.push(p),
        }
    }
    if !still_missing.is_empty() {
        let signature = params_signature(name, params);
        for p in still_missing {
            let signature = Some(signature.clone());
            let issue = if props.has(&p.name) {
                Issue::NullRequired { signature }
            } else {
                Issue::MissingRequired { signature }
            };
            ctx.push_issue(name, &format!("/{}", p.name), issue);
        }
        return Value::Null;
    }
    if drop_component {
        return Value::Null;
    }

    let has_dynamic_props = props.values().any(contains_dynamic_value);
    Value::Element(Box::new(Element {
        type_name: name.to_owned(),
        props,
        partial: ctx.partial,
        has_dynamic_props,
        statement_id: None,
    }))
}

/// `@Each(items, var, template)`: the iterator variable (a ref or a string)
/// stays in scope while the other args are normalized; `var` itself is kept as
/// written. `None` if this isn't a lazy builtin with a usable variable.
fn materialize_lazy_builtin(
    name: &str,
    args: &[Ast],
    mapped_props: &Option<JsObject<Ast>>,
    ctx: &mut Ctx,
    scope: &Scope,
) -> Option<Ast> {
    if !is_lazy_builtin(name) || args.len() < 3 {
        return None;
    }
    let var = match &args[1] {
        Ast::Ref(n) => n,
        Ast::Str(v) => v,
        _ => return None,
    };
    if var.is_empty() {
        return None;
    }
    let mut inner = scope.clone();
    inner.insert(var.clone());
    let args = args
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if i == 1 {
                a.clone()
            } else {
                materialize_expr(a, ctx, &inner)
            }
        })
        .collect();
    Some(Ast::Comp {
        name: name.to_owned(),
        args,
        mapped_props: mapped_props.clone(),
    })
}

/// Normalize a node for runtime evaluation: resolve refs, map library
/// components' args to `mapped_props`. The structure stays AST. Past
/// `MAX_DEPTH`, `Null`.
pub(crate) fn materialize_expr(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    if ctx.depth >= MAX_DEPTH {
        ctx.depth_limited = true;
        return Ast::Null;
    }
    ctx.depth += 1;
    let ast = materialize_expr_at_depth(node, ctx, scope);
    ctx.depth -= 1;
    ast
}

fn materialize_expr_at_depth(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    match node {
        Ast::Ref(name) if scope.contains(name) => node.clone(),
        Ast::Ref(name) => match resolve_ref(name, ctx, Mode::Expr) {
            Resolved::Ast(a) => a,
            Resolved::Value(_) => unreachable!("expr mode resolves to AST"),
        },
        Ast::Comp {
            name,
            args,
            mapped_props,
        } => expr_comp(name, args, mapped_props, ctx, scope),
        Ast::Arr(els) => Ast::Arr(expr_list(els, ctx, scope)),
        Ast::Obj(entries) => expr_object(entries, ctx, scope),
        Ast::BinOp { op, left, right } => {
            let left = expr_boxed(left, ctx, scope);
            Ast::BinOp {
                op,
                left,
                right: expr_boxed(right, ctx, scope),
            }
        }
        Ast::UnaryOp { op, operand } => Ast::UnaryOp {
            op,
            operand: expr_boxed(operand, ctx, scope),
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => expr_ternary(cond, then, otherwise, ctx, scope),
        Ast::Member { obj, field } => Ast::Member {
            obj: expr_boxed(obj, ctx, scope),
            field: field.clone(),
        },
        Ast::Index { obj, index } => {
            let obj = expr_boxed(obj, ctx, scope);
            Ast::Index {
                obj,
                index: expr_boxed(index, ctx, scope),
            }
        }
        Ast::Assign { target, value } => Ast::Assign {
            target: target.clone(),
            value: expr_boxed(value, ctx, scope),
        },
        // Literals, placeholders, state and runtime refs pass through.
        _ => node.clone(),
    }
}

fn expr_boxed(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Box<Ast> {
    Box::new(materialize_expr(node, ctx, scope))
}

fn expr_list(nodes: &[Ast], ctx: &mut Ctx, scope: &Scope) -> Vec<Ast> {
    nodes
        .iter()
        .map(|n| materialize_expr(n, ctx, scope))
        .collect()
}

fn expr_object(entries: &[(String, Ast)], ctx: &mut Ctx, scope: &Scope) -> Ast {
    Ast::Obj(
        entries
            .iter()
            .map(|(k, v)| (k.clone(), materialize_expr(v, ctx, scope)))
            .collect(),
    )
}

fn expr_ternary(cond: &Ast, then: &Ast, otherwise: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    let cond = expr_boxed(cond, ctx, scope);
    let then = expr_boxed(then, ctx, scope);
    Ast::Ternary {
        cond,
        then,
        otherwise: expr_boxed(otherwise, ctx, scope),
    }
}

fn expr_comp(
    name: &str,
    args: &[Ast],
    mapped_props: &Option<JsObject<Ast>>,
    ctx: &mut Ctx,
    scope: &Scope,
) -> Ast {
    if let Some(lazy) = materialize_lazy_builtin(name, args, mapped_props, ctx, scope) {
        return lazy;
    }
    let args = expr_list(args, ctx, scope);
    library_comp(name, args, mapped_props, ctx)
}

/// A call in a runtime expression: library components get `mapped_props`
/// (args by param name); unknown ones are reported; builtins pass through.
#[inline(never)]
fn library_comp(
    name: &str,
    args: Vec<Ast>,
    mapped_props: &Option<JsObject<Ast>>,
    ctx: &mut Ctx,
) -> Ast {
    let keep = |args| Ast::Comp {
        name: name.to_owned(),
        args,
        mapped_props: mapped_props.clone(),
    };
    if is_builtin(name) || is_reserved_call(name) {
        return keep(args);
    }
    let library = ctx.library;
    if let Some(def) = library.component(name) {
        let mut by_name = JsObject::new();
        for (param, arg) in def.params.iter().zip(&args) {
            by_name.set(param.name.clone(), arg.clone());
        }
        return Ast::Comp {
            name: name.to_owned(),
            args,
            mapped_props: Some(by_name),
        };
    }
    ctx.unknown_component(name);
    keep(args)
}

/// Whether a prop value holds anything that needs runtime evaluation.
fn contains_dynamic_value(value: &Value) -> bool {
    if is_ast_node(value) {
        return true;
    }
    match value {
        Value::Arr(items) => items.iter().any(contains_dynamic_value),
        _ => match element_parts(value) {
            Some((_, props)) => props.is_some_and(|p| p.values().any(contains_dynamic_value)),
            None => match value {
                Value::Obj(o) => o.values().any(contains_dynamic_value),
                _ => false,
            },
        },
    }
}
