//! Cached admission cannot replace depth checks at the position of use.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, Value, evaluate_tree_with_limits};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn library() -> Library {
    Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap(),
    )
}
fn shaped(mut p: ParseResult) -> ParseResult {
    let json = parse_json(&p.to_json()).unwrap();
    let value = Value::from_json(&json["root"]["props"]);
    if let (Some(root), Value::Obj(props)) = (&mut p.root, &value) {
        root.props = props.as_ref().clone();
    }
    p
}
fn shaped_value(value: Value) -> Value {
    let mut props = openui_runtime::JsObject::new();
    props.define("a", value);
    let p = shaped(ParseResult {
        root: Some(openui_lang::Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..ParseResult::default()
    });
    p.root.as_ref().unwrap().props.get("a").unwrap().clone()
}
fn deep(n: usize) -> Value {
    let mut v = Value::Null;
    for _ in 0..n {
        v = Value::Arr(Arc::new(vec![v]));
    }
    v
}
fn nested(mut value: String, n: usize) -> String {
    for level in 0..n {
        value = format!("@Each([1],i{level},{value})");
    }
    value
}
#[test]
fn earlier_state_and_query_reads_do_not_prime_a_deeper_use() {
    for query in [false, true] {
        for data in [false, true] {
            for first in [false, true] {
                for prime in [false, true] {
                    let name = if query { "q" } else { "$d" };
                    let declarations = if query { "q=Query(\"t\",{},$d)\n" } else { "" };
                    let shallow = if prime {
                        format!("[{name}]")
                    } else {
                        "null".into()
                    };
                    let deeper = nested(format!("@Count({name})"), 12);
                    let body = if first {
                        format!("[{shallow},{deeper}]")
                    } else {
                        format!("[{deeper},{shallow}]")
                    };
                    let p = parse(&format!("{declarations}root=Box({body})"), &library(), None);
                    let p = if data { shaped(p) } else { p };
                    let mut store = Store::new();
                    store.set("$d", Some(deep(748))).unwrap();
                    let r = evaluate_tree_with_limits(TreeInput::new(&p), &store, Limits::DEFAULT);
                    assert!(
                        r.depth_limited,
                        "query={query} data={data} shallow_first={first} prime={prime}"
                    );
                    assert!(!r.size_limited);
                }
            }
        }
    }
}
#[test]
fn deep_function_arguments_binding_values_and_action_params_recheck_depth() {
    use openui_runtime::{LibraryFunctions, evaluate_tree, run_action, write_binding};
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}},"functions":{"Len":{"params":{"properties":{"value":{}}}}},"actions":{"Probe":{"params":{"type":"object","properties":{"value":{}}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Len", |_| Ok(Some(Value::Num(1.))))
        .unwrap();
    for data in [false, true] {
        for first in [false, true] {
            for call in ["@Len($d)", "@Probe($d)"] {
                let deeper = nested(call.into(), 24);
                let body = if first {
                    format!("[[$d],{deeper}]")
                } else {
                    format!("[{deeper},[$d]]")
                };
                let p = parse(&format!("root=Box({body})"), &l, None);
                let p = if data { shaped(p) } else { p };
                let mut s = Store::new();
                s.set("$d", Some(deep(748))).unwrap();
                let r = evaluate_tree(
                    TreeInput::new(&p)
                        .with_library(&l)
                        .with_functions(&functions)
                        .with_actions(&schema["actions"]),
                    &s,
                );
                assert!(
                    r.depth_limited,
                    "call={call} data={data} shallow_first={first}"
                );
            }
            let deeper = nested("@Count($value)".into(), 24);
            let expr = if first {
                format!("[$value,{deeper}]")
            } else {
                format!("[{deeper},$value]")
            };
            let mut marker = openui_runtime::JsObject::new();
            marker.define("__reactive", Value::Str("assign".into()));
            marker.define("target", Value::Str("$out".into()));
            let expression = openui_lang::parse_expression(&openui_lang::tokenize(&expr));
            let expression = Value::Ast(Arc::new(expression));
            marker.define(
                "expr",
                if data {
                    shaped_value(expression)
                } else {
                    expression
                },
            );
            let mut s = Store::new();
            let empty = ParseResult::default();
            let result = write_binding(
                TreeInput::new(&empty),
                &mut s,
                &Value::Obj(Arc::new(marker)),
                deep(748),
            );
            assert!(
                matches!(result,Err(openui_runtime::BindingWriteError::Cut(flags)) if flags.depth_limited),
                "binding data={data} shallow_first={first}"
            );
            let p = parse(
                &format!(
                    "root=Box(Action([@Set($out,{})]))",
                    expr.replace("$value", "$d")
                ),
                &l,
                None,
            );
            let p = if data { shaped(p) } else { p };
            let draw = evaluate_tree(TreeInput::new(&p), &Store::new());
            let Some(Value::Element(root)) = &draw.root else {
                panic!("root")
            };
            s.set("$d", Some(deep(748))).unwrap();
            let action = run_action(TreeInput::new(&p), &mut s, root.props.get("a"), "", None);
            assert!(
                action.depth_limited,
                "action data={data} shallow_first={first}"
            );
        }
    }
}

#[test]
fn aliased_input_names_and_prepared_query_uses_keep_their_position_depth() {
    let l = library();
    let body = deep(748);
    let mut store = Store::new();
    store.set("$d", Some(body.clone())).unwrap();
    store.set("$alias", Some(body.clone())).unwrap();
    let mut values = openui_runtime::JsObject::new();
    values.define("q", body);
    let prepared = openui_runtime::PreparedReferenceValues::new(&values);
    for data in [false, true] {
        for first in [false, true] {
            for supplied in [false, true] {
                let name = if supplied { "q" } else { "$alias" };
                let primed = if supplied { "[q]" } else { "[$d]" };
                let deeper = if supplied {
                    nested("q".into(), 24)
                } else {
                    nested("@Count($alias)".into(), 24)
                };
                let source = format!(
                    "q=Query(\"t\",{{}})\nroot=Box([{}])",
                    if first {
                        format!("{primed},{deeper}")
                    } else {
                        format!("{deeper},{primed}")
                    }
                );
                let p = parse(&source, &l, None);
                let p = if data { shaped(p) } else { p };
                let input = if supplied {
                    TreeInput::new(&p).with_prepared_reference_values(&prepared)
                } else {
                    TreeInput::new(&p)
                };
                let r = evaluate_tree_with_limits(input, &store, Limits::DEFAULT);
                assert!(r.depth_limited, "{name}: data={data} shallow_first={first}");
            }
        }
    }
}
