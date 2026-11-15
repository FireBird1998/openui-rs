//! Corpus host implementations; operation recipes are shared with TypeScript.
use openui_lang::{JsonValue, parse_json};
use openui_runtime::{FunctionResult, LibraryFunctions, Value};
use std::sync::Arc;
fn get(v: &Value, k: &str) -> Option<Value> {
    if let Value::Obj(o) = v {
        o.get(k).cloned()
    } else {
        None
    }
}
fn number(v: Option<Value>) -> f64 {
    match v {
        Some(Value::Num(n)) => n,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(b),
        Some(Value::Str(ref s)) => s.parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}
fn string(v: Option<Value>) -> String {
    match v {
        Some(Value::Str(ref s)) => s.clone(),
        Some(Value::Num(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Null) | None => String::new(),
        _ => "[object Object]".into(),
    }
}
pub fn registry(definitions: &JsonValue) -> LibraryFunctions<'_> {
    counted_registry(definitions, None)
}
pub fn counted_registry(
    definitions: &JsonValue,
    counts: Option<Arc<std::sync::Mutex<std::collections::BTreeMap<String, usize>>>>,
) -> LibraryFunctions<'_> {
    let recipes = parse_json(include_str!(
        "../../../../conformance/function-implementations.json"
    ))
    .unwrap();
    let mut registry = LibraryFunctions::new(definitions);
    if let Some(defs) = definitions.as_object() {
        for name in defs.keys() {
            let recipe = recipes
                .get(name)
                .unwrap_or_else(|| panic!("No corpus implementation for {name}"));
            let op = recipe["op"].as_str().unwrap().to_owned();
            let key = recipe
                .get("key")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_owned();
            let value = recipe.get("value").map(Value::from_json);
            let counts = counts.clone();
            let name = name.clone();
            registry
                .register(name.clone(), move |args| -> FunctionResult {
                    if let Some(c) = &counts {
                        *c.lock().unwrap().entry(name.clone()).or_default() += 1;
                    }
                    Ok(match op.as_str() {
                        "args" => Some(args),
                        "get" => get(&args, &key),
                        "constant" => value.clone(),
                        "undefined" => None,
                        "throw" => return Err(string(value.clone())),
                        "repeat" => Some(Value::Str("x".repeat(number(value.clone()) as usize))),
                        "deep" => {
                            let mut v = Value::Null;
                            for _ in 0..number(value.clone()) as usize {
                                v = Value::Arr(Arc::new(vec![v]));
                            }
                            Some(v)
                        }
                        "twice" => Some(Value::Num(number(get(&args, &key)) * 2.0)),
                        "percent" => Some(Value::Str(format!(
                            "{:.1}%",
                            number(get(&args, "part")) / number(get(&args, "total")) * 100.0
                        ))),
                        "upper" => Some(Value::Str(
                            string(get(&args, "s").or_else(|| get(&args, "text"))).to_uppercase(),
                        )),
                        "format" => Some(Value::Str(
                            string(get(&args, "label")) + &string(get(&args, "value")),
                        )),
                        "sum" => {
                            let sum = match get(&args, "numbers") {
                                Some(Value::Arr(ref a)) => {
                                    a.iter().fold(0.0, |sum, v| sum + number(Some(v.clone())))
                                }
                                None | Some(Value::Null) => 0.0,
                                _ => return Err("numbers must be an array".into()),
                            };
                            Some(Value::Str(string(get(&args, "label")) + &sum.to_string()))
                        }
                        _ => panic!("Unknown corpus operation {op}"),
                    })
                })
                .unwrap();
        }
    }
    registry
}
