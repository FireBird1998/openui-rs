//! Lowering expression trees to elements and plain values: resolving
//! references, mapping positional args to named props, applying defaults and
//! reporting invalid props, as the reference parser's `materializeValue` and
//! `materializeExpr`.

use std::collections::HashSet;

use indexmap::{IndexMap, IndexSet};

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
/// components that had to be dropped.
pub(crate) fn materialize_value(node: &Ast, ctx: &mut Ctx) -> Value {
    match node {
        Ast::Ref(name) => match resolve_ref(name, ctx, Mode::Value) {
            Resolved::Value(v) => v,
            Resolved::Ast(a) => Value::Ast(Box::new(a)),
        },
        Ast::Str(v) => Value::Str(v.clone()),
        Ast::Num(n) => Value::Num(*n),
        Ast::Bool(b) => Value::Bool(*b),
        Ast::Null | Ast::Ph(_) => Value::Null,
        Ast::Arr(els) => {
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
        Ast::Obj(entries) => {
            let mut obj = JsObject::new();
            for (key, value) in entries {
                let value = materialize_value(value, ctx);
                obj.set(key.clone(), value);
            }
            Value::Obj(obj)
        }
        Ast::Comp {
            name,
            args,
            mapped_props,
        } => materialize_comp(name, args, mapped_props, ctx),
        // Runtime expressions stay AST, with their insides normalized.
        _ => Value::Ast(Box::new(materialize_expr(node, ctx, &Scope::new()))),
    }
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
    // Builtins stay AST for the runtime.
    if is_builtin(name) {
        let node = materialize_lazy_builtin(name, args, mapped_props, ctx, &Scope::new())
            .unwrap_or_else(|| Ast::Comp {
                name: name.to_owned(),
                args: args
                    .iter()
                    .map(|a| materialize_expr(a, ctx, &Scope::new()))
                    .collect(),
                mapped_props: mapped_props.clone(),
            });
        return Value::Ast(Box::new(node));
    }
    if is_reserved_call(name) {
        ctx.push_issue(name, "", Issue::InlineReserved);
        return Value::Null;
    }
    let library = ctx.library;
    let Some(def) = library.component(name) else {
        ctx.unknown_component(name);
        return Value::Null;
    };

    let mut props = JsObject::new();
    // Set when a required prop is invalid with no default to fall back on.
    let mut drop_component = false;
    for (param, arg) in def.params.iter().zip(args) {
        let mut value = materialize_value(arg, ctx);
        let path = format!("/{}", param.name);
        let invalid = validate_schema_value(&mut value, &param.schema, name, &path, ctx);
        props.set(param.name.clone(), value);
        if invalid
            && resolve_invalid_value(
                &mut props,
                &param.name,
                param.required,
                param.default.as_ref(),
            )
        {
            drop_component = true;
        }
    }

    if args.len() > def.params.len() {
        let issue = Issue::ExcessArgs {
            declared: def.params.len(),
            got: args.len(),
        };
        ctx.push_issue(name, "", issue);
    }

    // Missing or null required props take their default; without one, the
    // component is dropped.
    let missing: Vec<&Param> = def
        .params
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
        let signature = params_signature(name, &def.params);
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
/// components' args to `mapped_props`. The structure stays AST.
pub(crate) fn materialize_expr(node: &Ast, ctx: &mut Ctx, scope: &Scope) -> Ast {
    let boxed = |ctx: &mut Ctx, n: &Ast| Box::new(materialize_expr(n, ctx, scope));
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
        } => {
            if let Some(lazy) = materialize_lazy_builtin(name, args, mapped_props, ctx, scope) {
                return lazy;
            }
            let args: Vec<Ast> = args
                .iter()
                .map(|a| materialize_expr(a, ctx, scope))
                .collect();
            if is_builtin(name) || is_reserved_call(name) {
                return Ast::Comp {
                    name: name.clone(),
                    args,
                    mapped_props: mapped_props.clone(),
                };
            }
            let library = ctx.library;
            if let Some(def) = library.component(name) {
                let mut by_name = JsObject::new();
                for (param, arg) in def.params.iter().zip(&args) {
                    by_name.set(param.name.clone(), arg.clone());
                }
                return Ast::Comp {
                    name: name.clone(),
                    args,
                    mapped_props: Some(by_name),
                };
            }
            ctx.unknown_component(name);
            Ast::Comp {
                name: name.clone(),
                args,
                mapped_props: mapped_props.clone(),
            }
        }
        Ast::Arr(els) => Ast::Arr(
            els.iter()
                .map(|e| materialize_expr(e, ctx, scope))
                .collect(),
        ),
        Ast::Obj(entries) => Ast::Obj(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), materialize_expr(v, ctx, scope)))
                .collect(),
        ),
        Ast::BinOp { op, left, right } => {
            let left = boxed(ctx, left);
            Ast::BinOp {
                op,
                left,
                right: boxed(ctx, right),
            }
        }
        Ast::UnaryOp { op, operand } => Ast::UnaryOp {
            op,
            operand: boxed(ctx, operand),
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => {
            let cond = boxed(ctx, cond);
            let then = boxed(ctx, then);
            Ast::Ternary {
                cond,
                then,
                otherwise: boxed(ctx, otherwise),
            }
        }
        Ast::Member { obj, field } => Ast::Member {
            obj: boxed(ctx, obj),
            field: field.clone(),
        },
        Ast::Index { obj, index } => {
            let obj = boxed(ctx, obj);
            Ast::Index {
                obj,
                index: boxed(ctx, index),
            }
        }
        Ast::Assign { target, value } => Ast::Assign {
            target: target.clone(),
            value: boxed(ctx, value),
        },
        // Literals, placeholders, state and runtime refs pass through.
        _ => node.clone(),
    }
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
