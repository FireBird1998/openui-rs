//! Action plans and private, session-owned custom-step identities.
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
    fn mark(&self, step: &Arc<JsObject<Value>>) {
        self.with(|m| {
            if m.steps.len() >= m.sweep_at {
                m.steps.retain(|_, v| v.strong_count() > 0);
                m.sweep_at = m.steps.len().saturating_mul(2).max(256);
            }
            m.steps
                .insert(Arc::as_ptr(step) as usize, Arc::downgrade(step));
        });
    }
    fn owns(&self, step: &Value) -> bool {
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
    pub(super) fn action(
        &self,
        node: &JsObject<Value>,
        name: &str,
        eval: &ExpressionEvaluator<'_>,
    ) -> Option<Value> {
        // Builtin step detection is Map.get: an array coercing to "Set" is not Set.
        let original = node.get("name");
        let builtin = matches!(original,Some(Value::Str(n)) if matches!(n.as_str(),"Action"|"Run"|"Set"|"Reset"|"ToAssistant"|"OpenUrl"));
        let custom = self
            .actions
            .and_then(|a| a.as_object())
            .and_then(|a| a.get(name));
        if !builtin && custom.is_none() {
            return None;
        }
        if !builtin && matches!(custom, Some(openui_lang::JsonValue::Null)) {
            eval.fail("null is not an object (evaluating 'def.params')");
            return Some(plan(vec![]));
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
            return Some(plan(vec![]));
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
            return Some(plan(vec![]));
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
                        return Some(plan(vec![]));
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
                        return Some(plan(vec![]));
                    }
                    let Some(a) =
                        at(0).filter(|a| kind(a, "StateRef") && (len >= 2.0 || len.is_nan()))
                    else {
                        return Some(plan(vec![]));
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
                        return Some(plan(vec![]));
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
                        return Some(plan(vec![]));
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
                return Some(plan(vec![]));
            };
            let key = schema as *const _ as usize;
            let cached = self.checked_schemas.borrow().get(&key).cloned();
            let prepared = if let Some(cached) = cached {
                cached
            } else {
                let Some(validation) = eval.prepare_validation_schema(schema) else {
                    return Some(plan(vec![]));
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
        if let Some(mapped) = node
            .get("mappedProps")
            .filter(|v| !matches!(v, Value::Null))
        {
            for (k, v) in own_fields(mapped).iter() {
                if cut(eval) {
                    break;
                }
                if let Some(v) = eval
                    .with_call_context(false, || eval.evaluate_value(v).value)
                    .flatten()
                {
                    let Some(key) = eval.copy_text(k) else {
                        break;
                    };
                    named.set(key, v);
                }
            }
        } else {
            let builtin_iter = builtin
                .then_some(builtin_names)
                .into_iter()
                .flatten()
                .copied();
            let declared_iter = prepared.iter().flat_map(|p| p.property_names());
            for (i, name) in builtin_iter.chain(declared_iter).enumerate() {
                if cut(eval) {
                    break;
                }
                if (i as f64) >= len || len.is_nan() {
                    break;
                }
                if let Some(v) = at(i).and_then(|v| {
                    eval.with_call_context(false, || eval.evaluate_value(&v).value)
                        .flatten()
                }) {
                    let Some(name) = eval.copy_text(name) else {
                        break;
                    };
                    named.set(name, v);
                }
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
            return Some(plan(vec![]));
        };
        let required = schema.get("required").filter(|r| !r.is_null());
        let mut nulls = Vec::new();
        for (key, value) in named.iter() {
            if !matches!(value, Value::Null) {
                continue;
            }
            let required_name = match required {
                Some(openui_lang::JsonValue::Array(_)) => {
                    prepared.as_ref().is_some_and(|p| p.requires(key))
                }
                Some(raw @ openui_lang::JsonValue::String(_)) => {
                    let Some(required) = eval.coerce_json_string(raw) else {
                        return Some(plan(vec![]));
                    };
                    required.contains(key)
                }
                None => false,
                _ => {
                    eval.fail(format!("params.required?.includes is not a function. (In 'params.required?.includes(key)', 'params.required?.includes' is {})",method_value(required.and_then(|r|r.get("includes")))));
                    return Some(plan(vec![]));
                }
            };
            if !required_name {
                let Some(key) = eval.copy_text(key) else {
                    return Some(plan(vec![]));
                };
                nulls.push(key);
            }
        }
        for key in nulls {
            named.remove(&key);
        }
        if let Some(required) = required {
            let Some(required) = required.as_array() else {
                eval.fail(format!("params.required?.some is not a function. (In 'params.required?.some((key) => args[key] == null && getSchemaDefaultValue(params.properties?.[key]) === void 0)', 'params.required?.some' is {})",method_value(required.get("some"))));
                return Some(plan(vec![]));
            };
            for raw_key in required {
                let Some(key) = eval.coerce_json_string(raw_key) else {
                    return Some(plan(vec![]));
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
                    return Some(plan(vec![]));
                }
            }
        }
        let Some(prepared) = prepared else {
            return Some(plan(vec![]));
        };
        let (params, messages) = eval.check_value(&Value::Obj(Arc::new(named)), &prepared);
        for message in messages {
            if let Some(bytes) = self.report_bytes.get() {
                let message = format!("@{name}: {message}");
                if eval.reserve_result(bytes.saturating_add(text_bytes(&message))) {
                    self.errors.borrow_mut().push(RuntimeError {
                        code: "runtime-error",
                        component: None,
                        statement_id: None,
                        message,
                        hint: None,
                    });
                }
            }
        }
        let Some(params) = params else {
            return Some(plan(vec![]));
        };
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
