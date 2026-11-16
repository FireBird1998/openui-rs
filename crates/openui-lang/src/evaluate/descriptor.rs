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

fn receiver(value: Option<&Value>, scope: &Scope<Supplied<'_>>) -> Js {
    let direct = matches!(value,Some(Value::Ast(a)) if matches!(a.as_ref(),Ast::StateRef(_)|Ast::Ref(_)|Ast::RuntimeRef{..}))
        || matches!(value,Some(Value::Obj(o)) if matches!(o.get("k"),Some(Value::Str(k)) if matches!(k.as_str(),"StateRef"|"Ref"|"RuntimeRef")) && matches!(o.get("n"),Some(Value::Str(_))));
    let _receiver = direct.then(expression::ReceiverRead::enter);
    evaluate(value, scope)
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
            _ => expression::extra_state(get("n"), scope),
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
            let obj = receiver(get("obj"), scope);
            if is_nullish(&obj) || !running() {
                return Js::Null;
            }
            member_field(obj, get("field"))
        }
        "Index" => {
            let obj = receiver(get("obj"), scope);
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
                    && !scope.wants_unmapped_call(value, &name)
                {
                    return Js::Null;
                }
                return scope.call_value(value, &name);
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
        let direct = matches!(template,Value::Obj(o) if matches!(o.get("k"),Some(Value::Str(k)) if k=="Ref") && o.get("n")==Some(&var));
        if direct && let Some(value) = child.direct_each_reference(item) {
            return value;
        }
        let room = expression::SubstitutionRoom::enter();
        let policy = |name: &str| child.mapped_call_only(name);
        let copy = substitute_descriptor(template, &var, item, Some(&policy));
        room.finish();
        evaluate(Some(copy.as_ref().unwrap_or(template)), child)
    })
}

// All recursion and child order live in substitution::walk. This adapter keeps
// descriptor assignment, malformed entries and typed-child bridge semantics.
fn substitute_descriptor(
    node: &Value,
    var: &Value,
    item: &Js,
    policy: Option<&dyn Fn(&str) -> bool>,
) -> Option<Value> {
    substitution::walk(node, var, item, policy)
}

use substitution::{Field, Kind, Template};
impl<'a> Template for &'a Value {
    type Variable = Value;
    type Output = Value;
    type Child = Value;
    type Sequence = Arc<Vec<Value>>;
    type Props = JsObject<Value>;
    fn enter<P: SubstitutionPolicy>(
        self,
        var: &Value,
        item: &Js,
        policy: P,
    ) -> std::ops::ControlFlow<Option<Value>> {
        use std::ops::ControlFlow::{Break, Continue};
        if !step(1) || !running() {
            return Break(None);
        }
        if let Value::Ast(a) = self {
            return Break(if let Value::Str(var) = var {
                substitution::walk(a.as_ref(), var.as_str(), item, policy)
                    .map(|a| Value::Ast(Arc::new(a)))
            } else {
                None
            });
        }
        let Value::Obj(o) = self else {
            if matches!(self, Value::Null) {
                invalid("node", Op::ReadKind, Some(self));
            }
            return Break(None);
        };
        if text(o.get("k")) == "Ref"
            && o.get("n")
                .is_some_and(|n| crate::materialize::js_strict_eq(n, var))
        {
            return Break(Some(Value::Ast(Arc::new(literal(item)))));
        }
        Continue(())
    }
    fn kind(self) -> Kind {
        match text(field(self, "k")) {
            "Member" => Kind::Member,
            "Index" => Kind::Index,
            "BinOp" => Kind::Binary,
            "UnaryOp" => Kind::Unary,
            "Ternary" => Kind::Ternary,
            "Assign" => Kind::Assign,
            "Arr" => Kind::Array,
            "Obj" => Kind::Object,
            "Comp" => Kind::Call,
            _ => Kind::Leaf,
        }
    }
    fn child(self, key: Field) -> Option<Self> {
        let child = field(self, key.key());
        if matches!(self.kind(), Kind::Member | Kind::Index) {
            child.filter(|v| is_descriptor(v))
        } else {
            if child.is_none() {
                invalid("node", Op::ReadKind, None);
            }
            child
        }
    }
    fn call_policy<P: SubstitutionPolicy>(self, policy: P) -> (bool, P) {
        let name = text(field(self, "name"));
        let mapped = name != "Each"
            && matches!(
                field(self, "mappedProps"),
                Some(Value::Obj(_) | Value::Arr(_))
            )
            && policy.mapped_only(name);
        (mapped, policy.at_call(name, mapped))
    }
    fn fields(self, fields: &mut [(Field, Option<Value>)]) -> Option<Value> {
        let Value::Obj(o) = self else {
            return None;
        };
        let mut out = None;
        for (key, new) in fields {
            if let Some(new) = new.take() {
                out.get_or_insert_with(|| o.as_ref().clone())
                    .define(key.key(), new);
            }
        }
        out.map(|o| Value::Obj(Arc::new(o)))
    }
    fn assignment(self, visit: impl FnOnce(Self) -> Option<Value>) -> Option<Value> {
        let value = self.child(Field::Value).and_then(visit);
        self.fields(&mut [(Field::Value, value)])
    }
    fn shared(child: Value) -> Value {
        child
    }
    fn member(self, child: Option<Value>) -> Option<Value> {
        let out = self.fields(&mut [(Field::Object, child)]);
        let obj = out
            .as_ref()
            .and_then(|o| field(o, "obj"))
            .or_else(|| field(self, "obj"));
        if let (Some(obj), Some(Value::Str(name))) = (obj, field(self, "field")) {
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
        out
    }
    fn sequence<P: SubstitutionPolicy>(
        self,
        policy: P,
        mut visit: impl FnMut(Self, P) -> Option<Value>,
    ) -> Option<Arc<Vec<Value>>> {
        let key = if self.kind() == Kind::Array {
            "els"
        } else {
            "args"
        };
        let Some(Value::Arr(children)) = field(self, key) else {
            invalid(key, Op::SubstituteMap, field(self, key));
            return None;
        };
        substitution::rewrite_list::<_, false>(children, running, |n| visit(n, policy))
            .map(Arc::new)
    }

    fn array(self, items: Arc<Vec<Value>>) -> Value {
        let Value::Obj(o) = self else {
            return self.clone();
        };
        let mut out = o.as_ref().clone();
        out.define("els", Value::Arr(items));
        Value::Obj(Arc::new(out))
    }
    fn entries<P: SubstitutionPolicy>(
        self,
        policy: P,
        mut visit: impl FnMut(Self, P) -> Option<Value>,
    ) -> Option<Value> {
        if let Some(Value::Arr(entries)) = field(self, "entries") {
            let mut changed = false;
            let mut items = Vec::new();
            for entry in entries.iter() {
                if !running() {
                    break;
                }
                if let Value::Arr(pair) = entry {
                    if let Some(child) = pair.get(1) {
                        let new = visit(child, policy);
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
                let Value::Obj(o) = self else {
                    return None;
                };
                let mut out = o.as_ref().clone();
                out.define("entries", Value::Arr(Arc::new(items)));
                return Some(Value::Obj(Arc::new(out)));
            }
        } else {
            invalid("entries", Op::SubstituteMap, field(self, "entries"));
        }
        None
    }
    fn props<P: SubstitutionPolicy>(
        self,
        policy: P,
        mut visit: impl FnMut(Self, P) -> Option<Value>,
    ) -> Option<JsObject<Value>> {
        let mut changed = false;
        let mut out = JsObject::new();
        let mut put = |key: String, v: &'a Value| {
            if !running() {
                return false;
            }
            let new = visit(v, policy);
            changed |= new.is_some();
            out.set(key, new.unwrap_or_else(|| v.clone()));
            true
        };
        match field(self, "mappedProps") {
            Some(Value::Obj(props)) => {
                for (k, v) in props.iter() {
                    if !put(k.clone(), v) {
                        break;
                    }
                }
            }
            Some(Value::Arr(props)) => {
                for (i, v) in props.iter().enumerate() {
                    if !put(i.to_string(), v) {
                        break;
                    }
                }
            }
            _ => return None,
        }
        changed.then_some(out)
    }
    fn call(
        self,
        args: Option<Arc<Vec<Value>>>,
        props: Option<JsObject<Value>>,
        mapped_only: bool,
    ) -> Option<Value> {
        if args.is_none() && props.is_none() {
            return None;
        }
        let Value::Obj(o) = self else {
            return None;
        };
        let mut out = o.as_ref().clone();
        if let Some(args) = args {
            out.define("args", Value::Arr(args));
        }
        if let Some(props) = props {
            out.define("mappedProps", Value::Obj(Arc::new(props)));
        }
        if mapped_only
            && let (Some(Value::Arr(args)), Some(Value::Obj(props))) = (o.get("args"), o.get("mappedProps"))
            && args.len() == 1 && props.len() == 1
            && let (Some(Value::Obj(arg)), Some((key, Value::Obj(prop)))) = (args.first(), props.iter().next())
            && Arc::ptr_eq(arg, prop)
            && let (Some(Value::Arr(args)), Some(Value::Obj(props))) = (out.get("args"), out.get("mappedProps"))
            // The mapped writer may have discarded __proto__.
            && props.has_own(key)
            && let Some(replacement) = args.first()
        {
            let mut props = props.as_ref().clone();
            props.define(key, replacement.clone());
            out.define("mappedProps", Value::Obj(Arc::new(props)));
        }
        Some(Value::Obj(Arc::new(out)))
    }
}

fn is_descriptor(v: &Value) -> bool {
    matches!(v, Value::Ast(_))
        || matches!(v,Value::Obj(o) if matches!(o.get("k"),Some(Value::Str(k)) if matches!(k.as_str(),"Comp"|"Ref"|"RuntimeRef"|"StateRef"|"Ph"|"Str"|"Num"|"Bool"|"Null"|"Arr"|"Obj"|"Member"|"Index"|"BinOp"|"UnaryOp"|"Ternary"|"Assign")))
}
