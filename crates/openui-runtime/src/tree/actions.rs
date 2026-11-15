//! Plans, private custom-step identities, and shared checked call arguments.
use super::*;
use std::{
    collections::HashMap,
    sync::{Mutex, Weak},
};

#[derive(Default, Debug)]
pub(crate) struct Provenance(Mutex<Marks>);
#[derive(Default, Debug)]
struct Marks {
    steps: HashMap<usize, Weak<JsObject<Value>>>,
    sweep_at: usize,
}
impl Provenance {
    fn with<T>(&self, f: impl FnOnce(&mut Marks) -> T) -> T {
        let mut marks = match self.0.lock() {
            Ok(v) => v,
            Err(e) => e.into_inner(),
        };
        f(&mut marks)
    }
    pub(crate) fn clear(&self) {
        self.with(|m| {
            *m = Marks::default();
        });
    }
    pub(super) fn mark(&self, step: &Arc<JsObject<Value>>) {
        self.with(|m| {
            if m.steps.len() >= m.sweep_at {
                m.steps.retain(|_, v| v.strong_count() > 0);
                m.sweep_at = m.steps.len().saturating_mul(2).max(256);
            }
            m.steps
                .insert(Arc::as_ptr(step) as usize, Arc::downgrade(step));
        });
    }
    pub(crate) fn owns(&self, step: &Value) -> bool {
        let Value::Obj(step) = step else {
            return false;
        };
        self.with(|m| {
            m.steps
                .get(&(Arc::as_ptr(step) as usize))
                .and_then(Weak::upgrade)
                .is_some_and(|s| Arc::ptr_eq(&s, step))
        })
    }
}
fn method_value(value: Option<&openui_lang::JsonValue>) -> String {
    match value {
        None => "undefined".into(),
        Some(openui_lang::JsonValue::Array(_)) => "an instance of Array".into(),
        Some(openui_lang::JsonValue::Object(_)) => "an instance of Object".into(),
        Some(v) => super::value_json(&Value::from_json(v)),
    }
}
fn obj(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut out = JsObject::new();
    for (k, v) in fields {
        out.define(k, v);
    }
    Value::Obj(Arc::new(out))
}
fn text(s: &str) -> Value {
    Value::Str(s.into())
}
fn plan(steps: Vec<Value>) -> Value {
    obj([("steps", Value::Arr(Arc::new(steps)))])
}
fn field(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Obj(o) => o.get(key).cloned(),
        Value::Ast(a) => match (a.as_ref(), key) {
            (Ast::StateRef(n) | Ast::RuntimeRef { n, .. }, "n") => Some(text(n)),
            (Ast::RuntimeRef { ref_type, .. }, "refType") => Some(text(ref_type.as_str())),
            _ => None,
        },
        Value::Element(_) if key == "type" => Some(text("element")),
        _ => None,
    }
}
fn kind(v: &Value, k: &str) -> bool {
    match v {
        Value::Ast(a) => matches!(
            (a.as_ref(), k),
            (Ast::StateRef(_), "StateRef") | (Ast::RuntimeRef { .. }, "RuntimeRef")
        ),
        Value::Obj(o) => matches!(o.get("k"),Some(Value::Str(s)) if s==k),
        _ => false,
    }
}
pub(super) fn is_plan(v: &Value) -> bool {
    matches!(v,Value::Obj(o) if matches!(o.get("steps"),Some(Value::Arr(_))))
}
impl<'a> TreeScope<'a> {
    pub(super) fn own_plan(&self, v: Value) -> Value {
        let Value::Obj(o) = &v else {
            return v;
        };
        let Some(Value::Arr(steps)) = o.get("steps") else {
            return v;
        };
        let own = |s: &&Value| {
            field(s, "type").as_ref() != Some(&text("custom_action")) || self.store.actions.owns(s)
        };
        if steps.iter().all(|s| own(&s)) {
            return v;
        }
        let steps = steps.iter().filter(own).cloned().collect();
        let mut out = (**o).clone();
        out.define("steps", Value::Arr(Arc::new(steps)));
        Value::Obj(Arc::new(out))
    }
    pub(super) fn aggregate(
        &self,
        node: &JsObject<Value>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<openui_lang::ExpressionOutput> {
        if !matches!(node.get("name"),Some(Value::Str(n)) if n=="Action") {
            return None;
        }
        let args = node.get("args");
        let Some(args) = args.filter(|v| !matches!(v, Value::Null)) else {
            eval.fail(format!(
                "{} is not an object (evaluating 'args.length')",
                if args.is_none() { "undefined" } else { "null" }
            ));
            return Some(None.into());
        };
        let length = match args {
            Value::Arr(a) => a.len() as f64,
            Value::Str(s) => s.encode_utf16().count() as f64,
            Value::Obj(o) => o
                .get("length")
                .and_then(|v| eval.coerce_number(v))
                .unwrap_or(f64::NAN),
            _ => f64::NAN,
        };
        let first = if length > 0.0 {
            match args {
                Value::Arr(a) => a.first().cloned(),
                Value::Obj(o) => o.get("0").cloned(),
                Value::Str(s) => s.chars().next().map(|c| Value::Str(c.to_string())),
                _ => None,
            }
        } else {
            Some(Value::Ast(Arc::new(Ast::Arr(Arc::new(Vec::new())))))
        };
        let Some(first) = first else {
            eval.fail("undefined is not an object (evaluating 'node.k')");
            return Some(None.into());
        };
        Some(
            eval.with_call_context(false, || {
                eval.transform_value(&first, |value| {
                    let steps = value
                        .items()
                        .filter(|item| item.is_object())
                        .flat_map(|item| {
                            let nested = item.property("steps");
                            let stray =
                                (!nested.is_array() && item.has_property("type")).then_some(item);
                            nested.items().chain(stray)
                        })
                        .filter(|s| {
                            !s.is_object()
                                || s.property("type").as_str() != Some("custom_action")
                                || s.data().is_some_and(|v| self.store.actions.owns(v))
                        });
                    value.object([("steps", value.array(steps))])
                })
            })
            .unwrap_or_else(|| None.into()),
        )
    }
    pub(super) fn declared_call(
        &self,
        node: &JsObject<Value>,
        name: &str,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        // Builtin step detection is Map.get: an array coercing to "Set" is not Set.
        let original = node.get("name");
        let builtin = matches!(original,Some(Value::Str(n)) if matches!(n.as_str(),"Action"|"Run"|"Set"|"Reset"|"ToAssistant"|"OpenUrl"));
        let function = (!builtin)
            .then(|| self.functions.and_then(|f| f.get(name)))
            .flatten();
        let empty = || {
            if function.is_some() {
                Value::Null
            } else {
                plan(vec![])
            }
        };
        let custom = function.map(|(definition, _)| definition).or_else(|| {
            self.actions
                .and_then(|a| a.as_object())
                .and_then(|a| a.get(name))
        });
        if !builtin && custom.is_none() {
            return None;
        }
        if !builtin && matches!(custom, Some(openui_lang::JsonValue::Null)) {
            eval.fail("null is not an object (evaluating 'def.params')");
            return Some(empty());
        }
        let schema = custom.and_then(|a| a.get("params"));
        if !builtin && schema.is_none_or(|s| s.is_null()) {
            eval.fail(format!(
                "{} is not an object (evaluating 'params.properties')",
                if schema.is_none() {
                    "undefined"
                } else {
                    "null"
                }
            ));
            return Some(empty());
        }
        let properties = schema.and_then(|s| s.get("properties"));
        let args = node.get("args");
        let nullish_args = args.is_none_or(|a| matches!(a, Value::Null));
        let nullish_message = |receiver: &str| {
            format!(
                "{} is not an object (evaluating '{receiver}')",
                if args.is_none() { "undefined" } else { "null" }
            )
        };
        let lazy = builtin && matches!(name, "Run" | "Set" | "Reset");
        let mapped = node
            .get("mappedProps")
            .filter(|v| !matches!(v, Value::Null));
        if nullish_args
            && (lazy
                || name == "Action"
                || (mapped.is_none()
                    && (builtin
                        || properties.is_some_and(|p| match p {
                            openui_lang::JsonValue::Object(o) => !o.is_empty(),
                            openui_lang::JsonValue::Array(a) => !a.is_empty(),
                            openui_lang::JsonValue::String(s) => !s.is_empty(),
                            _ => false,
                        }))))
        {
            let receiver = match name {
                "Run" => "args[0]",
                "Reset" => "args.filter",
                _ => "args.length",
            };
            eval.fail(nullish_message(receiver));
            return Some(empty());
        }
        // Eager naming reads increasing indices; lazy steps may reread zero.
        // Keep a cursor instead of rescanning a supplied string per parameter.
        let mut string_chars = match args {
            Some(Value::Str(s)) => Some(s.chars().enumerate()),
            _ => None,
        };
        let mut last_char = None;
        let mut at = |i: usize| match args {
            Some(Value::Arr(a)) => a.get(i).cloned(),
            Some(Value::Obj(a)) => a.get(&i.to_string()).cloned(),
            Some(Value::Str(_)) => {
                if let Some((index, c)) = last_char
                    && index == i
                {
                    return Some(Value::Str(char::to_string(&c)));
                }
                for (index, c) in string_chars.as_mut()? {
                    last_char = Some((index, c));
                    if index == i {
                        return Some(Value::Str(c.to_string()));
                    }
                }
                None
            }
            _ => None,
        };
        let len =
            if (lazy && name != "Set") || (mapped.is_some() && name != "Action" && name != "Set") {
                f64::NAN
            } else {
                match args {
                    Some(Value::Arr(a)) => a.len() as f64,
                    Some(Value::Str(s)) => s.encode_utf16().count() as f64,
                    Some(Value::Obj(a)) => a
                        .get("length")
                        .and_then(|v| eval.coerce_number(v))
                        .unwrap_or(f64::NAN),
                    _ => f64::NAN,
                }
            };
        if builtin && matches!(name, "Run" | "Set" | "Reset") {
            let mut step = JsObject::new();
            match name {
                "Run" => {
                    let Some(a) = at(0).filter(|a| kind(a, "RuntimeRef")) else {
                        return Some(empty());
                    };
                    step.define("type", text("run"));
                    if let Some(n) = field(&a, "n") {
                        step.define("statementId", n);
                    }
                    if let Some(t) = field(&a, "refType") {
                        step.define("refType", t);
                    }
                }
                "Set" => {
                    if (len >= 2.0 || len.is_nan())
                        && at(0).is_none_or(|v| matches!(v, Value::Null))
                    {
                        eval.fail(format!(
                            "{} is not an object (evaluating 'args[0].k')",
                            if at(0).is_none() { "undefined" } else { "null" }
                        ));
                        return Some(empty());
                    }
                    let Some(a) =
                        at(0).filter(|a| kind(a, "StateRef") && (len >= 2.0 || len.is_nan()))
                    else {
                        return Some(empty());
                    };
                    step.define("type", text("set"));
                    if let Some(n) = field(&a, "n") {
                        step.define("target", n);
                    }
                    if let Some(v) = at(1) {
                        step.define("valueAST", v);
                    }
                }
                _ => {
                    let mut targets = Vec::new();
                    if !matches!(args, Some(Value::Arr(_))) {
                        eval.fail("args.filter is not a function. (In 'args.filter((a) => a.k === \"StateRef\")', 'args.filter' is undefined)");
                        return Some(empty());
                    }
                    if let Some(Value::Arr(args)) = args {
                        for a in args.iter() {
                            if cut(eval) {
                                break;
                            }
                            if matches!(a, Value::Null) {
                                eval.fail("null is not an object (evaluating 'a.k')");
                                break;
                            }
                            if kind(a, "StateRef") {
                                targets.push(field(a, "n").unwrap_or(Value::Null));
                            }
                        }
                    }
                    if targets.is_empty() {
                        return Some(empty());
                    }
                    step.define("type", text("reset"));
                    step.define("targets", Value::Arr(Arc::new(targets)));
                }
            }
            return Some(plan(vec![Value::Obj(Arc::new(step))]));
        }
        let builtin_names: &[&str] = if name == "ToAssistant" {
            &["message", "context"]
        } else {
            &["url"]
        };
        let prepared = if builtin {
            None
        } else {
            let Some(schema) = schema else {
                return Some(empty());
            };
            let key = schema as *const _ as usize;
            let cached = self.checked_schemas.borrow().get(&key).cloned();
            let prepared = if let Some(cached) = cached {
                cached
            } else {
                let Some(validation) = eval.prepare_validation_schema(schema) else {
                    return Some(empty());
                };
                let prepared = std::rc::Rc::new(validation);
                self.checked_schemas
                    .borrow_mut()
                    .insert(key, prepared.clone());
                prepared
            };
            Some(prepared)
        };
        let mut named = JsObject::new();
        let mut undefined = std::collections::BTreeSet::new();
        if let Some(mapped) = node
            .get("mappedProps")
            .filter(|v| !matches!(v, Value::Null))
        {
            for (k, v) in own_fields(mapped).iter() {
                if cut(eval) {
                    break;
                }
                if matches!(v, Value::Null) {
                    eval.fail("null is not an object (evaluating 'node.k')");
                    return Some(empty());
                }
                let Some(result) = eval.with_call_context(false, || eval.evaluate_value(v)) else {
                    return Some(empty());
                };
                if result.failure.is_some() || result.size_limited || result.depth_limited {
                    return Some(empty());
                }
                let v = result.value;
                let Some(key) = eval.copy_text(k) else {
                    break;
                };
                if v.is_none() {
                    undefined.insert(key.clone());
                }
                named.set(key, v.unwrap_or(Value::Null));
            }
        } else {
            let builtin_iter = builtin
                .then_some(builtin_names)
                .into_iter()
                .flatten()
                .copied();
            let declared_iter = prepared.iter().flat_map(|p| p.property_names());
            let mut prototype = JsObject::new();
            for (i, name) in builtin_iter.chain(declared_iter).enumerate() {
                if cut(eval) {
                    break;
                }
                if (i as f64) >= len || len.is_nan() {
                    break;
                }
                if let Some(arg) = at(i) {
                    // nameArgs assigns descriptors onto a plain object before
                    // evaluateArgs walks it. __proto__ sets that intermediate
                    // object's prototype instead of naming an argument.
                    if name == "__proto__" {
                        prototype = own_fields(&arg).into_owned();
                        continue;
                    }
                    if matches!(arg, Value::Null) {
                        eval.fail("null is not an object (evaluating 'node.k')");
                        return Some(empty());
                    }
                    let Some(result) = eval.with_call_context(false, || eval.evaluate_value(&arg))
                    else {
                        return Some(empty());
                    };
                    if result.failure.is_some() || result.size_limited || result.depth_limited {
                        return Some(empty());
                    }
                    let v = result.value;
                    let Some(name) = eval.copy_text(name) else {
                        break;
                    };
                    if v.is_none() {
                        undefined.insert(name.clone());
                    }
                    named.set(name, v.unwrap_or(Value::Null));
                }
            }
            for (key, raw) in prototype.iter() {
                if named.has_own(key) {
                    continue;
                }
                if matches!(raw, Value::Null) {
                    eval.fail("null is not an object (evaluating 'node.k')");
                    return Some(empty());
                }
                let Some(result) = eval.with_call_context(false, || eval.evaluate_value(raw))
                else {
                    return Some(empty());
                };
                if result.failure.is_some() || result.size_limited || result.depth_limited {
                    return Some(empty());
                }
                let Some(key) = eval.copy_text(key) else {
                    return Some(empty());
                };
                if result.value.is_none() {
                    undefined.insert(key.clone());
                }
                named.set(key, result.value.unwrap_or(Value::Null));
            }
        }
        if builtin {
            let key = if name == "ToAssistant" {
                "message"
            } else {
                "url"
            };
            let value = named.get(key).filter(|v| !matches!(v, Value::Null));
            let value = value
                .and_then(|v| eval.coerce_string(v))
                .unwrap_or_default();
            let mut step = JsObject::new();
            step.define(
                "type",
                text(if name == "ToAssistant" {
                    "continue_conversation"
                } else {
                    "open_url"
                }),
            );
            step.define(key, Value::Str(value));
            if name == "ToAssistant"
                && let Some(context) = named.get("context").filter(|v| !matches!(v, Value::Null))
            {
                step.define("context", context.clone());
            }
            return Some(plan(vec![Value::Obj(Arc::new(step))]));
        }
        let Some(schema) = schema.filter(|s| !s.is_null()) else {
            eval.fail(format!(
                "{} is not an object (evaluating 'params.properties')",
                if schema.is_none() {
                    "undefined"
                } else {
                    "null"
                }
            ));
            return Some(empty());
        };
        let required = schema.get("required").filter(|r| !r.is_null());
        let mut nulls = Vec::new();
        for (key, value) in named.iter() {
            if !matches!(value, Value::Null) || undefined.contains(key) {
                continue;
            }
            let required_name = match required {
                Some(openui_lang::JsonValue::Array(_)) => {
                    prepared.as_ref().is_some_and(|p| p.requires(key))
                }
                Some(raw @ openui_lang::JsonValue::String(_)) => {
                    let Some(required) = eval.coerce_json_string(raw) else {
                        return Some(empty());
                    };
                    required.contains(key)
                }
                None => false,
                _ => {
                    eval.fail(format!("params.required?.includes is not a function. (In 'params.required?.includes(key)', 'params.required?.includes' is {})",method_value(required.and_then(|r|r.get("includes")))));
                    return Some(empty());
                }
            };
            if !required_name {
                let Some(key) = eval.copy_text(key) else {
                    return Some(empty());
                };
                nulls.push(key);
            }
        }
        for key in nulls {
            named.remove(&key);
        }
        let mut defaulted = std::collections::BTreeSet::new();
        if let Some(required) = required {
            let Some(required) = required.as_array() else {
                eval.fail(format!("params.required?.some is not a function. (In 'params.required?.some((key) => args[key] == null && getSchemaDefaultValue(params.properties?.[key]) === void 0)', 'params.required?.some' is {})",method_value(required.get("some"))));
                return Some(empty());
            };
            for raw_key in required {
                let Some(key) = eval.coerce_json_string(raw_key) else {
                    return Some(empty());
                };
                let nullish = !named.has(&key) || matches!(named.get(&key), Some(Value::Null));
                let sub = match properties {
                    Some(openui_lang::JsonValue::Object(p)) => p.get(&key),
                    Some(openui_lang::JsonValue::Array(p)) => key
                        .parse::<usize>()
                        .ok()
                        .filter(|i| i.to_string() == key)
                        .and_then(|i| p.get(i)),
                    _ => None,
                };
                let default = sub
                    .and_then(|s| s.as_object())
                    .and_then(|s| s.get("default"));
                if nullish && default.is_none() {
                    return Some(empty());
                }
                if nullish
                    && schema.get("type").and_then(openui_lang::JsonValue::as_str) == Some("object")
                {
                    defaulted.insert(key);
                }
            }
        }
        let Some(prepared) = prepared else {
            return Some(empty());
        };
        let raw = Value::Obj(Arc::new(named));
        let (params, messages) = if function.is_some() {
            eval.validate_value(&raw, &prepared, "")
        } else {
            eval.check_value(&raw, &prepared)
        };
        for message in messages {
            self.report_call(format!("@{name}: {message}"), eval);
        }
        let Some(mut params) = params else {
            return Some(empty());
        };
        // Value's public projection omits undefined own fields. Keep a nullish
        // placeholder through validation so they shadow inherited members;
        // remove it afterwards unless a required default supplied real null.
        if let Value::Obj(object) = &params {
            undefined.retain(|key| {
                !defaulted.contains(key) && matches!(object.get(key), Some(Value::Null))
            });
            if !undefined.is_empty() {
                // Rebuild once: repeated shift-removal would be quadratic for
                // a wide object of undefined arguments. Price actual copies.
                let mut projected = JsObject::new();
                for (key, value) in object.iter() {
                    if undefined.contains(key) {
                        continue;
                    }
                    let Some(key) = eval.copy_text(key) else {
                        return Some(empty());
                    };
                    let value = if let Value::Str(s) = value {
                        let Some(s) = eval.copy_text(s) else {
                            return Some(empty());
                        };
                        Value::Str(s)
                    } else {
                        value.clone()
                    };
                    projected.define(key, value);
                }
                params = Value::Obj(Arc::new(projected));
            }
        }
        if cut(eval) {
            return Some(empty());
        }
        if let Some((definition, implementation)) = function {
            let result = match implementation(params) {
                Ok(result) => result,
                Err(message) => {
                    if let Some(message) = eval.copy_text(&message) {
                        self.report_call(format!("@{name} threw: {message}"), eval);
                    }
                    return Some(Value::Null);
                }
            };
            let returns = definition.get("returns").filter(|s| match s {
                openui_lang::JsonValue::Null => false,
                openui_lang::JsonValue::Bool(b) => *b,
                openui_lang::JsonValue::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
                openui_lang::JsonValue::String(s) => !s.is_empty(),
                _ => true,
            });
            let Some(returns) = returns else {
                return result;
            };
            let key = returns as *const _ as usize;
            let cached = self.checked_schemas.borrow().get(&key).cloned();
            let schema = if let Some(schema) = cached {
                schema
            } else {
                let schema = std::rc::Rc::new(eval.prepare_validation_schema(returns)?);
                self.checked_schemas
                    .borrow_mut()
                    .insert(key, schema.clone());
                schema
            };
            let (checked, messages) =
                eval.validate_value(result.as_ref().unwrap_or(&Value::Null), &schema, "/returns");
            for message in messages {
                self.report_call(format!("@{name}: {message}"), eval);
            }
            return Some(checked.unwrap_or(Value::Null));
        }
        let step = obj([
            ("type", text("custom_action")),
            ("name", original.cloned().unwrap_or_else(|| text(name))),
            ("params", params),
        ]);
        if let Value::Obj(step) = &step {
            self.store.actions.mark(step);
        }
        Some(plan(vec![step]))
    }
}
