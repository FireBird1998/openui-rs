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
    let _context = (text(get("k")) != "Comp").then(|| expression::CallContext::set(false));
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
            if name == "Each" {
                return if matches!(get("name"), Some(Value::Str(n)) if n=="Each") {
                    each_descriptor(value, scope)
                } else {
                    Js::Null
                };
            }
            let inherited = OBJECT_PROTOTYPE_KEYS.contains(&name.as_str());
            if !inherited && builtin_function_params(&name).is_none() {
                if !get("mappedProps")
                    .map(expression::data_value)
                    .is_some_and(|v| truthy(&v))
                {
                    return Js::Null;
                }
                return scope.call_value(value);
            }
            let _context = expression::CallContext::set(false);
            // BUILTINS indexes an object (coerced key); parameter definitions
            // use Map.get and therefore only match an original string name.
            let params = match get("name") {
                Some(Value::Str(name)) => builtin_function_params(name).unwrap_or(&[]),
                _ => &[],
            };
            // An inherited entry cannot consume the argument record. Its
            // children still run in order, but their unused results need not
            // accumulate in a potentially huge temporary map.
            let mut named = JsObject::new();
            match get("mappedProps") {
                Some(Value::Obj(p)) => {
                    for (k, v) in p.iter() {
                        if !running() {
                            break;
                        }
                        let value = evaluate(Some(v), scope);
                        if !inherited {
                            named.define(k.clone(), value);
                        }
                    }
                }
                Some(Value::Arr(p)) => {
                    for (i, v) in p.iter().enumerate() {
                        if !running() {
                            break;
                        }
                        let value = evaluate(Some(v), scope);
                        if !inherited {
                            named.define(i.to_string(), value);
                        }
                    }
                }
                Some(v) if !matches!(v, Value::Null) => {}
                _ => {
                    let args = get("args");
                    match args {
                        _ if params.is_empty() => {}
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
            if inherited {
                expression::fail(EvaluationFailure::BuiltinNotCallable);
                Js::Undefined
            } else {
                call::<Supplied<'_>>(&name, &named)
            }
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

// The same iteration, literal construction and per-item hook as syntax. This
// adapter only supplies the descriptor's structural child fields.
fn each_descriptor(node: &Value, scope: &Scope<Supplied<'_>>) -> Js {
    let args = field(node, "args");
    if matches!(args, None | Some(Value::Null)) {
        return invalid("args", Op::Length, args);
    }
    let length = args
        .map(expression::data_value)
        .map(|v| property(&v, "length"))
        .unwrap_or(Js::Undefined);
    let length = if is_object(&length) {
        to_primitive_in::<Supplied<'_>>(&length)
    } else {
        length
    };
    // This is JavaScript's internal comparison, not OpenUI's toNumber,
    // which turns undefined and malformed strings into zero.
    let short = match &length {
        Js::Undefined => false,
        Js::Str(s) => js_number(s) < 3.0,
        _ => to_number(&length) < 3.0,
    };
    if short {
        return Js::Arr(Arc::new(Vec::new()));
    }
    // Indexed characters are non-null scalar nodes, so evaluation produces
    // undefined and the non-list rule returns before reading a variable.
    if matches!(args, Some(Value::Str(_))) {
        return Js::Arr(Arc::new(Vec::new()));
    }
    let arg = |index: usize| match args {
        Some(Value::Arr(a)) => a.get(index),
        Some(Value::Obj(o)) => o.get(&index.to_string()),
        _ => None,
    };
    let array = {
        let _context = expression::CallContext::set(false);
        evaluate(arg(0), scope)
    };
    if !matches!(&array, Js::Arr(_))
        && !matches!(&array, Js::Data(v) if matches!(v.as_ref(),Value::Arr(_)))
    {
        return Js::Arr(Arc::new(Vec::new()));
    }
    let name_node = arg(1);
    if matches!(name_node, None | Some(Value::Null)) {
        return invalid("eachVar", Op::ReadKind, name_node);
    }
    let var = name_node.and_then(|v| {
        if let Value::Ast(a) = v {
            return match a.as_ref() {
                Ast::Ref(n) | Ast::Str(n) => Some(std::borrow::Cow::Owned(Value::Str(n.clone()))),
                _ => None,
            };
        }
        match text(field(v, "k")) {
            "Ref" => field(v, "n"),
            "Str" => field(v, "v"),
            _ => None,
        }
        .map(std::borrow::Cow::Borrowed)
    });
    let Some(var) = var.filter(|v| truthy(&expression::data_value(v))) else {
        return Js::Arr(Arc::new(Vec::new()));
    };
    let name = if let Value::Str(n) = var.as_ref() {
        Some(n.as_str())
    } else {
        None
    };
    each_array(array, name, scope, |item, child| {
        let Some(template) = arg(2) else {
            return invalid("node", Op::ReadKind, None);
        };
        let copy = substitute_descriptor(template, &var, item);
        evaluate(Some(copy.as_ref().unwrap_or(template)), child)
    })
}

// Representation adapter for substituteRef. Typed children use the existing
// typed walker and both forms use `literal` for captured loop values.
fn substitute_descriptor(node: &Value, var: &Value, item: &Js) -> Option<Value> {
    crate::stack::grow_stack!(substitute_descriptor(node,var,item), budget else None);
    if !step(1) || !running() {
        return None;
    }
    if let Value::Ast(a) = node {
        return if let Value::Str(var) = var {
            substitute(a, var, item).map(|a| Value::Ast(Arc::new(a)))
        } else {
            None
        };
    }
    let Value::Obj(o) = node else {
        if matches!(node, Value::Null) {
            invalid("node", Op::ReadKind, Some(node));
        }
        return None;
    };
    let kind = text(o.get("k"));
    if kind == "Ref"
        && o.get("n")
            .is_some_and(|n| crate::materialize::js_strict_eq(n, var))
    {
        return Some(Value::Ast(Arc::new(literal(item))));
    }
    let fields: &[&str] = match kind {
        "Member" => &["obj"],
        "Index" => &["obj", "index"],
        "BinOp" => &["left", "right"],
        "UnaryOp" => &["operand"],
        "Ternary" => &["cond", "then", "else"],
        "Assign" => &["value"],
        _ => &[],
    };
    let mut out = None;
    for key in fields {
        if let Some(child) = o.get(key) {
            if matches!(kind, "Member" | "Index") && !is_descriptor(child) {
                continue;
            }
            if let Some(new) = substitute_descriptor(child, var, item) {
                out.get_or_insert_with(|| o.as_ref().clone())
                    .define(*key, new);
            }
        } else if !matches!(kind, "Member" | "Index") {
            invalid("node", Op::ReadKind, None);
        }
    }
    if kind == "Member" {
        let obj = out
            .as_ref()
            .and_then(|o: &JsObject<Value>| o.get("obj"))
            .or_else(|| o.get("obj"));
        if let (Some(obj), Some(Value::Str(name))) = (obj, o.get("field")) {
            match obj {
                Value::Ast(a) => {
                    if let Ast::Obj(entries) = a.as_ref()
                        && let Some((_, v)) = entries.iter().find(|(k, _)| k == name)
                    {
                        return Some(Value::Ast(Arc::new(v.clone())));
                    }
                }
                Value::Obj(obj) if text(obj.get("k")) == "Obj" => {
                    if let Some(Value::Arr(entries)) = obj.get("entries") {
                        for entry in entries.iter() {
                            if let Value::Arr(pair) = entry
                                && matches!(pair.first(),Some(Value::Str(k)) if k==name)
                            {
                                return pair.get(1).cloned();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let sequence = if kind == "Arr" {
        Some("els")
    } else if kind == "Comp" {
        Some("args")
    } else {
        None
    };
    if let Some(key) = sequence {
        if let Some(Value::Arr(children)) = o.get(key) {
            let mut changed = false;
            let mut items = Vec::new();
            for child in children.iter() {
                if !running() {
                    break;
                }
                let new = substitute_descriptor(child, var, item);
                changed |= new.is_some();
                items.push(new.unwrap_or_else(|| child.clone()));
            }
            if changed {
                out.get_or_insert_with(|| o.as_ref().clone())
                    .define(key, Value::Arr(Arc::new(items)));
            }
        } else {
            invalid(key, Op::SubstituteMap, o.get(key));
        }
    }
    if kind == "Obj" {
        if let Some(Value::Arr(entries)) = o.get("entries") {
            let mut changed = false;
            let mut items = Vec::new();
            for entry in entries.iter() {
                if !running() {
                    break;
                }
                if let Value::Arr(pair) = entry {
                    if let Some(child) = pair.get(1) {
                        let new = substitute_descriptor(child, var, item);
                        changed |= new.is_some();
                        if let Some(new) = new {
                            let mut pair = pair.as_ref().clone();
                            if let Some(v) = pair.get_mut(1) {
                                *v = new;
                            }
                            items.push(Value::Arr(Arc::new(pair)));
                        } else {
                            items.push(entry.clone());
                        }
                    } else {
                        invalid("node", Op::ReadKind, None);
                    }
                } else if let Value::Str(text) = entry {
                    let mut chars = text.chars();
                    match (chars.next(), chars.next()) {
                        (Some(k), Some(v)) => {
                            // Destructuring a string entry yields its first two
                            // code points; the second is a non-AST scalar.
                            changed = true;
                            items.push(Value::Arr(Arc::new(vec![
                                Value::Str(k.to_string()),
                                Value::Str(v.to_string()),
                            ])));
                        }
                        _ => {
                            invalid("node", Op::ReadKind, None);
                        }
                    }
                } else {
                    invalid("entries", Op::Entry, Some(entry));
                }
            }
            if changed {
                out.get_or_insert_with(|| o.as_ref().clone())
                    .define("entries", Value::Arr(Arc::new(items)));
            }
        } else {
            invalid("entries", Op::SubstituteMap, o.get("entries"));
        }
    }
    if kind == "Comp" {
        let mapped = match o.get("mappedProps") {
            Some(Value::Obj(props)) => {
                substitute_props(props.iter().map(|(k, v)| (k.clone(), v)), var, item)
            }
            Some(Value::Arr(props)) => substitute_props(
                props.iter().enumerate().map(|(i, v)| (i.to_string(), v)),
                var,
                item,
            ),
            _ => None,
        };
        if let Some(mapped) = mapped {
            out.get_or_insert_with(|| o.as_ref().clone())
                .define("mappedProps", Value::Obj(Arc::new(mapped)));
        }
    }
    out.map(|o| Value::Obj(Arc::new(o)))
}
fn substitute_props<'a>(
    props: impl Iterator<Item = (String, &'a Value)>,
    var: &Value,
    item: &Js,
) -> Option<JsObject<Value>> {
    let mut changed = false;
    let mut mapped = JsObject::new();
    for (k, v) in props {
        if !running() {
            break;
        }
        let new = substitute_descriptor(v, var, item);
        changed |= new.is_some();
        mapped.set(k, new.unwrap_or_else(|| v.clone()));
    }
    changed.then_some(mapped)
}

fn is_descriptor(v: &Value) -> bool {
    matches!(v, Value::Ast(_))
        || matches!(v,Value::Obj(o) if matches!(o.get("k"),Some(Value::Str(k)) if matches!(k.as_str(),"Comp"|"Ref"|"RuntimeRef"|"StateRef"|"Ph"|"Str"|"Num"|"Bool"|"Null"|"Arr"|"Obj"|"Member"|"Index"|"BinOp"|"UnaryOp"|"Ternary"|"Assign")))
}
