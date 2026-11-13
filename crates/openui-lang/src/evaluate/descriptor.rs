//! Lazy reading of untyped AST descriptors. Operators, control flow, indexing
//! and builtins are the same implementations as the typed AST evaluator.
use super::*;
use expression::{ExpressionOperation as Op, ExpressionType as Type, Supplied};

fn invalid(field: &'static str, operation: Op, value: Option<&Value>) -> Js {
    let actual = match value {
        None => Type::Undefined,
        Some(Value::Null) => Type::Null,
        Some(Value::Num(_)) => Type::Number,
        Some(Value::Bool(b)) => Type::Boolean(*b),
        Some(Value::Obj(_) | Value::Element(_) | Value::Ast(_)) => Type::Object,
        _ => Type::Other,
    };
    expression::fail(EvaluationFailure::InvalidExpression {
        field,
        operation,
        actual,
    });
    Js::Undefined
}
fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Obj(o) => o.get(name),
        _ => None,
    }
}
fn key(value: Option<&Value>) -> String {
    value
        .map(expression::data_value)
        .map(|v| to_string_in::<Supplied<'_>>(&v))
        .unwrap_or_else(|| "undefined".into())
}
fn text(value: Option<&Value>) -> &str {
    match value {
        Some(Value::Str(s)) => s,
        _ => "",
    }
}
fn running() -> bool {
    !expression::failed() && LIMITS.with(|l| l.cut.get().is_none())
}

pub(super) fn evaluate(value: Option<&Value>, scope: &Scope<Supplied<'_>>) -> Js {
    crate::stack::grow_stack!(evaluate(value, scope), budget else Js::Null);
    if !running() {
        return Js::Null;
    }
    let Some(_level) = Level::enter() else {
        return Js::Null;
    };
    if !step(1) || !running() {
        return Js::Null;
    }
    let Some(value) = value else {
        return invalid("node", Op::ReadKind, None);
    };
    if matches!(value, Value::Null) {
        return invalid("node", Op::ReadKind, Some(value));
    }
    if let Value::Ast(ast) = value {
        return super::evaluate(ast, scope);
    }
    let get = |name| field(value, name);
    match text(get("k")) {
        "Str" | "Num" | "Bool" => get("v")
            .map(expression::data_value)
            .unwrap_or(Js::Undefined),
        "Null" | "Ph" => Js::Null,
        "StateRef" => match get("n") {
            Some(Value::Str(name)) => scope.state(name),
            _ => Js::Undefined,
        },
        "Ref" | "RuntimeRef" => match get("n") {
            Some(Value::Str(name)) => scope.reference(name),
            _ => Js::Null,
        },
        "BinOp" => binary_lazy::<Supplied<'_>>(text(get("op")), |right| {
            evaluate(get(if right { "right" } else { "left" }), scope)
        }),
        "UnaryOp" => unary(text(get("op")), || evaluate(get("operand"), scope)),
        "Ternary" => conditional::<Supplied<'_>>(
            || evaluate(get("cond"), scope),
            || evaluate(get("then"), scope),
            || evaluate(get("else"), scope),
        ),
        "Member" => {
            let obj = evaluate(get("obj"), scope);
            if is_nullish(&obj) || !running() {
                return Js::Null;
            }
            member_field(obj, get("field"))
        }
        "Index" => {
            let obj = evaluate(get("obj"), scope);
            if !running() {
                return Js::Null;
            }
            let at = evaluate(get("index"), scope);
            index::<Supplied<'_>>(obj, at)
        }
        "Arr" => {
            let Some(Value::Arr(items)) = get("els") else {
                return invalid("els", Op::Map, get("els"));
            };
            Js::Arr(Arc::new(
                items
                    .iter()
                    .take_while(|_| running())
                    .map(|v| evaluate(Some(v), scope))
                    .collect(),
            ))
        }
        "Obj" => {
            let Some(Value::Arr(entries)) = get("entries") else {
                return invalid("entries", Op::Map, get("entries"));
            };
            let mut mapped = Vec::new();
            for entry in entries.iter() {
                if !running() {
                    break;
                }
                match entry {
                    Value::Arr(pair) => mapped.push((
                        pair.first()
                            .map(expression::data_value)
                            .unwrap_or(Js::Undefined),
                        evaluate(pair.get(1), scope),
                    )),
                    Value::Str(s) => {
                        let mut chars = s.chars();
                        let key = chars
                            .next()
                            .map(|c| Js::Str(c.to_string().into()))
                            .unwrap_or(Js::Undefined);
                        if chars.next().is_none() {
                            return invalid("node", Op::ReadKind, None);
                        }
                        mapped.push((key, Js::Undefined));
                    }
                    _ => return invalid("entries", Op::Entry, Some(entry)),
                }
            }
            let mut out = JsObject::new();
            for (k, v) in mapped {
                if !running() {
                    break;
                }
                out.define(to_string_in::<Supplied<'_>>(&k), v);
            }
            Js::Obj(Arc::new(out))
        }
        "Comp" => {
            let name = key(get("name"));
            if builtin_function_params(&name).is_none() {
                return Js::Null;
            }
            // BUILTINS indexes an object (coerced key); parameter definitions
            // use Map.get and therefore only match an original string name.
            let params = match get("name") {
                Some(Value::Str(name)) => builtin_function_params(name).unwrap_or(&[]),
                _ => &[],
            };
            let mut named = JsObject::new();
            match get("mappedProps") {
                Some(Value::Obj(p)) => {
                    for (k, v) in p.iter() {
                        if !running() {
                            break;
                        }
                        named.define(k.clone(), evaluate(Some(v), scope));
                    }
                }
                Some(Value::Arr(p)) => {
                    for (i, v) in p.iter().enumerate() {
                        if !running() {
                            break;
                        }
                        named.define(i.to_string(), evaluate(Some(v), scope));
                    }
                }
                Some(v) if !matches!(v, Value::Null) => {}
                _ => {
                    let args = get("args");
                    match args {
                        Some(Value::Arr(args)) => {
                            for (name, arg) in params.iter().zip(args.iter()) {
                                if !running() {
                                    break;
                                }
                                named.define(*name, evaluate(Some(arg), scope));
                            }
                        }
                        None | Some(Value::Null) if !params.is_empty() => {
                            return invalid("args", Op::Length, args);
                        }
                        Some(Value::Obj(args)) => {
                            let length = args
                                .get("length")
                                .map(expression::data_value)
                                .unwrap_or(Js::Undefined);
                            let length = if is_object(&length) {
                                to_primitive_in::<Supplied<'_>>(&length)
                            } else {
                                length
                            };
                            if !running() {
                                return Js::Null;
                            }
                            let length = to_number(&length);
                            for (i, name) in params.iter().enumerate() {
                                if (i as f64).partial_cmp(&length) != Some(Ordering::Less)
                                    || !running()
                                {
                                    break;
                                }
                                named.define(*name, evaluate(args.get(&i.to_string()), scope));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if !running() {
                return Js::Null;
            }
            call::<Supplied<'_>>(&name, &named)
        }
        "Assign" => {
            let mut out = JsObject::new();
            out.define("__reactive", Js::Str("assign".into()));
            out.define(
                "target",
                get("target")
                    .map(expression::data_value)
                    .unwrap_or(Js::Undefined),
            );
            out.define(
                "expr",
                get("value")
                    .map(expression::data_value)
                    .unwrap_or(Js::Undefined),
            );
            Js::Obj(Arc::new(out))
        }
        _ => Js::Undefined,
    }
}

// Array pluck's length special case is a strict comparison. Other fields
// become property keys lazily, once per non-null item (optional chaining).
fn member_field(obj: Js, raw: Option<&Value>) -> Js {
    if !matches!(raw, Some(Value::Str(_))) {
        let pluck = |item: Js| {
            if is_nullish(&item) {
                return Js::Null;
            }
            let field = key(raw);
            if !running() {
                return Js::Null;
            }
            let value = property(&item, &field);
            if is_nullish(&value) { Js::Null } else { value }
        };
        match &obj {
            Js::Arr(items) => {
                if !step(items.len()) {
                    return Js::Null;
                }
                return Js::Arr(Arc::new(
                    items
                        .iter()
                        .take_while(|_| running())
                        .cloned()
                        .map(pluck)
                        .collect(),
                ));
            }
            Js::Data(value) => {
                if let Value::Arr(items) = value.as_ref() {
                    if !step(items.len()) {
                        return Js::Null;
                    }
                    return Js::Arr(Arc::new(
                        items
                            .iter()
                            .take_while(|_| running())
                            .map(expression::data_value)
                            .map(pluck)
                            .collect(),
                    ));
                }
            }
            _ => {}
        }
    }
    let field = key(raw);
    if !running() {
        return Js::Null;
    }
    LIMITS.with(|limits| member::<Supplied<'_>>(obj, &field, limits))
}
