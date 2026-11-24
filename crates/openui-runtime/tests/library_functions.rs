//! Library implementations, returned data, limits, and unwind recovery.
use openui_lang::{
    Element, JsonValue, Library, LibraryDefinition, ParseResult, create_library, free_json, parse,
    parse_json, stringify_json,
};
use openui_runtime::{
    LibraryFunctions, Limits, Store, TreeInput, Value, evaluate_tree, evaluate_tree_with_limits,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[path = "support/functions.rs"]
mod functions;
fn schema() -> JsonValue {
    parse_json(include_str!(
        "../../../conformance/schemas/runtime-functions.json"
    ))
    .unwrap()
}
fn parsed_root(value: &JsonValue) -> ParseResult {
    let root = Value::from_json(value);
    let Value::Obj(o) = &root else {
        panic!("element")
    };
    let Some(Value::Obj(props)) = o.get("props") else {
        panic!("props")
    };
    ParseResult {
        root: Some(Element {
            type_name: "Box".into(),
            props: props.as_ref().clone(),
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..Default::default()
    }
}
#[test]
fn returned_values_match_reference_and_are_bounded_on_entry() {
    let j = schema();
    let funcs = functions::registry(&j["functions"]);
    let cases = parse_json(include_str!(
        "../../../conformance/golden/function-results.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 8);
    for c in cases.as_array().unwrap() {
        let p = parsed_root(&c["root"]);
        let store = Store::new();
        let input = TreeInput::new(&p).with_functions(&funcs);
        let limits = Limits::DEFAULT.with_depth(1024).with_result_size(1_000_000);
        let r = evaluate_tree_with_limits(input.clone(), &store, limits);
        assert!(!r.size_limited && !r.depth_limited, "{}", c["name"]);
        assert_eq!(
            r.to_json().unwrap(),
            openui_lang::stringify_json(&c["expected"]),
            "{}",
            c["name"]
        );
        let r = evaluate_tree_with_limits(
            input,
            &store,
            if c["name"] == "Deep" {
                Limits::DEFAULT.with_depth(32)
            } else {
                Limits::DEFAULT
            },
        );
        match c["name"].as_str().unwrap() {
            "Huge" => assert!(r.size_limited),
            "Deep" => assert!(r.depth_limited),
            _ => assert!(!r.size_limited && !r.depth_limited),
        }
    }
    free_json(cases);
    drop(funcs);
}
fn check_return_field(field_schema: &str, returned: &str, expected: &str, diagnostics: &[&str]) {
    let mut j = schema();
    let mut record = parse_json(r#"{"type":"object","properties":{},"required":["n"]}"#).unwrap();
    record["properties"]["n"] = parse_json(field_schema).unwrap();
    j["functions"]["Identity"]["returns"] = record;
    let library = Library::from_json_schema(&j);
    let mut functions = LibraryFunctions::new(&j["functions"]);
    let returned = Value::from_json(&parse_json(returned).unwrap());
    functions
        .register("Identity", move |_| Ok(Some(returned.clone())))
        .unwrap();
    let program = parse("root=Box(@Identity())", &library, None);
    let result = evaluate_tree(
        TreeInput::new(&program)
            .with_library(&library)
            .with_functions(&functions),
        &Store::new(),
    );
    let json = parse_json(&result.to_json().unwrap()).unwrap();
    assert_eq!(
        json["root"]["props"].get("value"),
        Some(&parse_json(expected).unwrap())
    );
    let messages: Vec<_> = result.errors.iter().map(|e| e.message.as_str()).collect();
    let expected_messages: Vec<_> = diagnostics
        .iter()
        .map(|d| format!(r#"Evaluating prop "value" on Box failed: @Identity: {d}"#))
        .collect();
    assert_eq!(messages, expected_messages);
}

fn refuses_null_field(schema: &str) {
    check_return_field(
        schema,
        r#"{"n":null}"#,
        "null",
        &[r#"required field "/returns/n" cannot be null"#],
    );
}

#[test]
fn returned_required_string_refuses_null() {
    refuses_null_field(r#"{"type":"string"}"#);
}
#[test]
fn returned_required_boolean_refuses_null() {
    refuses_null_field(r#"{"type":"boolean"}"#);
}
#[test]
fn returned_required_object_refuses_null() {
    refuses_null_field(r#"{"type":"object"}"#);
}
#[test]
fn returned_required_array_refuses_null() {
    refuses_null_field(r#"{"type":"array"}"#);
}

#[test]
fn returned_number_distinguishes_explicit_null_missing_and_wrong_type() {
    let defaulted = r#"{"type":"number","default":7}"#;
    check_return_field(defaulted, r#"{"n":null}"#, r#"{"n":null}"#, &[]);
    check_return_field(defaulted, "{}", r#"{"n":7}"#, &[]);
    let number = r#"{"type":"number"}"#;
    check_return_field(
        number,
        "{}",
        "null",
        &[r#"missing required field "/returns/n""#],
    );
    check_return_field(
        number,
        r#"{"n":"Infinity"}"#,
        "null",
        &[r#"field "/returns/n" expects number but got string"#],
    );
}

#[test]
fn numeric_return_projection_is_limited_to_direct_fields_of_records() {
    for schema in [
        r#"{"type":"integer"}"#,
        r#"{"type":"number","const":1}"#,
        r#"{"type":"number","enum":[1,2]}"#,
    ] {
        refuses_null_field(schema);
    }
    check_return_field(
        r#"{"type":"object","properties":{"inner":{"type":"number"}},"required":["inner"]}"#,
        r#"{"n":{"inner":null}}"#,
        "null",
        &[r#"required field "/returns/n/inner" cannot be null"#],
    );
}

#[test]
fn a_component_prop_still_rejects_a_null_required_number() {
    let mut j = schema();
    j["$defs"]["Box"]["properties"]["value"] =
        parse_json(r#"{"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}"#)
            .unwrap();
    let library = Library::from_json_schema(&j);
    let program = parse("root=Box({n:null})", &library, None);
    assert_eq!(program.meta.errors.len(), 1);
    assert_eq!(
        program.meta.errors[0].message,
        r#"required field "/value/n" cannot be null"#
    );
    assert!(program.root.unwrap().props.get("value").is_none());
}

#[test]
fn a_function_argument_still_rejects_a_null_required_number_from_state() {
    let mut j = schema();
    let record =
        parse_json(r#"{"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}"#)
            .unwrap();
    j["functions"]["Identity"]["params"]["properties"]["value"] = record.clone();
    j["functions"]["Identity"]["params"]["required"] = parse_json(r#"["value"]"#).unwrap();
    j["functions"]["Identity"]["returns"] = record;
    let library = Library::from_json_schema(&j);
    let mut functions = LibraryFunctions::new(&j["functions"]);
    functions
        .register("Identity", |_| {
            panic!("invalid argument reached the callback")
        })
        .unwrap();
    let store = &mut Store::new();
    store
        .set(
            "$row",
            Some(Value::from_json(&parse_json(r#"{"n":null}"#).unwrap())),
        )
        .unwrap();
    let program = parse("root=Box(@Identity($row))", &library, None);
    let result = evaluate_tree(
        TreeInput::new(&program)
            .with_library(&library)
            .with_functions(&functions),
        store,
    );
    assert_eq!(result.errors.len(), 1);
    assert!(
        result.errors[0]
            .message
            .ends_with(r#"@Identity: required field "/value/n" cannot be null"#)
    );
}

#[test]
fn unimplemented_functions_are_null_by_platform_policy() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs
        .register("NotDeclared", |_| {
            panic!("must not call an undeclared implementation")
        })
        .unwrap();
    let p = parse("root=Box(@Identity(2),@NotDeclared(2))", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert!(r.errors.is_empty());
    assert!(
        r.to_json().unwrap().contains("\"value\":null"),
        "{}",
        r.to_json().unwrap()
    );
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs
        .register("Identity", |_| Ok(Some(Value::Num(1.0))))
        .unwrap();
    funcs
        .register("Identity", |_| Ok(Some(Value::Num(2.0))))
        .unwrap();
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert!(r.to_json().unwrap().contains("\"value\":2"));
    drop(funcs);
}
#[test]
fn host_panics_unwind_contexts_and_leave_the_store_reusable() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let calls = AtomicUsize::new(0);
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs
        .register("Identity", |args| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("embedder panic")
            };
            Ok(Some(args))
        })
        .unwrap();
    let p = parse("root=Box(@Each([1,2],r,@Identity(r)))", &l, None);
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(7.0))).unwrap();
    let before = store.snapshot();
    let version = store.version();
    let run = || {
        evaluate_tree(
            TreeInput::new(&p).with_library(&l).with_functions(&funcs),
            &store,
        )
    };
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err());
    assert!(Arc::ptr_eq(&before, &store.snapshot()));
    assert_eq!(version, store.version());
    let r = run();
    assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
    assert!(
        r.to_json()
            .unwrap()
            .contains("[{\"value\":1},{\"value\":2}]"),
        "{}",
        r.to_json().unwrap()
    );
    drop(funcs);
}
#[test]
fn deferred_set_expressions_never_call_implementations() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs
        .register("Throw", |_| panic!("set values must stay deferred"))
        .unwrap();
    let p = parse("root=Box(@Each([1,2],r,Box(@Set($n,@Throw(r)))))", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert!(r.errors.is_empty());
    assert!(
        r.to_json()
            .unwrap()
            .contains("\"valueAST\":{\"k\":\"Comp\",\"name\":\"Throw\""),
        "{}",
        r.to_json().unwrap()
    );
    drop(funcs);
}
#[test]
fn cuts_keep_earlier_errors_and_props_and_call_results_earn_no_allowance() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let funcs = functions::registry(&j["functions"]);
    for source in [
        "root=Box(@Throw(),@Huge(),@Wrong())",
        "root=Box(@Throw(),@Identity(@Huge()),@Wrong())",
        "root=Box(@Throw(),@Each([1,2],r,@Huge()),@Wrong())",
    ] {
        let p = parse(source, &l, None);
        let r = evaluate_tree(
            TreeInput::new(&p).with_library(&l).with_functions(&funcs),
            &Store::new(),
        );
        assert!(r.size_limited);
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].message.ends_with("@Throw threw: host failure"));
        let s = r.to_json().unwrap();
        assert!(s.contains("\"value\":null"));
        assert!(s.contains("\"name\":\"Wrong\""), "{s}");
    }
    drop(funcs);
}

#[test]
fn failed_argument_evaluation_never_calls_the_host() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs
        .register("Identity", |_| panic!("arguments failed before invocation"))
        .unwrap();
    let p = parse(
        r#"root=Box({k:"Comp",name:"Identity",args:[null]})"#,
        &l,
        None,
    );
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert_eq!(r.errors.len(), 1);
    assert!(
        r.errors[0]
            .message
            .ends_with("null is not an object (evaluating 'node.k')")
    );
}

#[test]
fn non_finite_arguments_refuse_the_call_before_pruning_or_a_required_null() {
    let j = parse_json(
        r#"{"$defs":{"Box":{"properties":{"value":{},"b":{}}}},
        "functions":{"Inspect":{"params":{"type":"object","properties":{
            "item":{}, "label":{"type":"string"}},"required":["item"]}}}}"#,
    )
    .unwrap();
    let library = Library::from_json_schema(&j);
    let mut functions = LibraryFunctions::new(&j["functions"]);
    functions
        .register("Inspect", |args| {
            panic!("refused call reached the host: {args:?}")
        })
        .unwrap();
    for (number, expression) in [
        (f64::INFINITY, "1e308*10"),
        (f64::NEG_INFINITY, "-1e308*10"),
        (f64::NAN, "1e308*10 - 1e308*10"),
    ] {
        let mut store = Store::new();
        store.set("$n", Some(Value::Num(number))).unwrap();
        store.set("$empty", Some(Value::Null)).unwrap();
        for n in ["$n", expression] {
            for (args, key) in [
                (format!("1, {n}"), "label"),
                (format!("$empty, {n}"), "label"),
                (n.to_owned(), "item"),
            ] {
                // Repeat the same supplied arguments within one evaluation, where
                // validated arguments would normally be eligible for reuse.
                let call = format!("@Inspect({args})");
                let program = parse(&format!("root=Box({call},{call})"), &library, None);
                let result = evaluate_tree(
                    TreeInput::new(&program)
                        .with_library(&library)
                        .with_functions(&functions),
                    &store,
                );
                let json = parse_json(&result.to_json().unwrap()).unwrap();
                assert_eq!(
                    json["root"]["props"],
                    parse_json(r#"{"value":null,"b":null}"#).unwrap(),
                    "{call}"
                );
                assert_eq!(result.errors.len(), 2, "{call}: {:?}", result.errors);
                for error in &result.errors {
                    assert!(
                        error
                            .message
                            .ends_with(&format!("@Inspect: {key} is not a finite number")),
                        "{call}: {error:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn optional_null_is_omitted_and_nested_numbers_keep_their_json_projection() {
    let j = schema();
    let library = Library::from_json_schema(&j);
    let mut functions = LibraryFunctions::new(&j["functions"]);
    functions
        .register("Identity", |args| Ok(Some(args)))
        .unwrap();
    let program = parse(
        "root=Box(@Identity(null),@Identity([$n,{x:$n}]),@Identity(0),@Identity())",
        &library,
        None,
    );
    for number in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut store = Store::new();
        store.set("$n", Some(Value::Num(number))).unwrap();
        let result = evaluate_tree(
            TreeInput::new(&program)
                .with_library(&library)
                .with_functions(&functions),
            &store,
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let json = parse_json(&result.to_json().unwrap()).unwrap();
        assert_eq!(
            json["root"]["props"],
            parse_json(r#"{"value":{},"b":{"value":[null,{"x":null}]},"c":{"value":0},"d":{}}"#)
                .unwrap()
        );
    }
}

#[test]
fn deep_host_results_cut_and_drop_on_a_small_stack() {
    std::thread::Builder::new()
        .name("deep_host_results::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let j = schema();
            let l = Library::from_json_schema(&j);
            let mut funcs = LibraryFunctions::new(&j["functions"]);
            funcs
                .register("Deep", |_| {
                    let mut v = Value::Null;
                    for _ in 0..10_000 {
                        v = Value::Arr(Arc::new(vec![v]));
                    }
                    Ok(Some(v))
                })
                .unwrap();
            let p = parse("root=Box(@Deep())", &l, None);
            let r = evaluate_tree(
                TreeInput::new(&p).with_library(&l).with_functions(&funcs),
                &Store::new(),
            );
            assert!(r.depth_limited);
            drop(r);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn callbacks_can_read_and_evaluate_the_same_store_and_recover_from_nested_panic() {
    let j = schema();
    let library = Library::from_json_schema(&j);
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(7.))).unwrap();
    let inner = parse("root=Box($n)", &library, None);
    let mut functions = LibraryFunctions::new(&j["functions"]);
    let calls = AtomicUsize::new(0);
    functions
        .register("Identity", |args| {
            assert_eq!(store.get("$n"), Some(Value::Num(7.)));
            let snapshot = store.snapshot();
            assert_eq!(snapshot.get("$n"), Some(&Some(Value::Num(7.))));
            let nested = evaluate_tree(TreeInput::new(&inner), &store);
            assert!(!nested.size_limited && !nested.depth_limited);
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("host caught after nested evaluation");
            }
            Ok(Some(args))
        })
        .unwrap();
    let program = parse("root=Box(@Identity($n))", &library, None);
    let before = store.snapshot();
    let version = store.version();
    let run = || {
        evaluate_tree(
            TreeInput::new(&program)
                .with_library(&library)
                .with_functions(&functions),
            &store,
        )
    };
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err());
    let result = run();
    assert!(!result.size_limited && !result.depth_limited && result.errors.is_empty());
    assert_eq!(store.version(), version);
    assert!(Arc::ptr_eq(&before, &store.snapshot()));
    drop(functions);
    assert!(store.set("$after", Some(Value::Num(1.))).unwrap());
}

/// A library's functions declared in a library definition reach the runtime
/// as when the same library is supplied as upstream's JSON: through the
/// definition's JSON Schema `functions` and the host-function path. A call
/// with good arguments returns its implementation's result, one with bad
/// arguments or a bad result is reported and null, an unimplemented one is
/// null, and an unknown one is reported, identically for both.
#[test]
fn a_defined_librarys_functions_evaluate_as_its_json_does() {
    let golden = parse_json(include_str!(
        "../../../conformance/golden/library-definitions.json"
    ))
    .unwrap();
    let case = golden
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "functions-in-a-program")
        .unwrap();
    let defined =
        create_library(LibraryDefinition::from_json(&case["definition"]).unwrap()).unwrap();
    let defined_schema = defined.to_json_schema();
    let upstream_schema = &case["schema"];
    assert_eq!(
        stringify_json(&defined_schema),
        stringify_json(upstream_schema)
    );
    let register = |definitions| {
        let mut funcs = LibraryFunctions::new(definitions);
        funcs
            .register("Percent", |args| {
                let Value::Obj(o) = &args else {
                    return Ok(None);
                };
                let (Some(Value::Num(part)), Some(Value::Num(total))) =
                    (o.get("part"), o.get("total"))
                else {
                    return Ok(None);
                };
                Ok(Some(Value::Num(part / total * 100.0)))
            })
            .unwrap();
        // A wrong result for a string, a right one for "ok".
        funcs
            .register("Label", |args| {
                let Value::Obj(o) = &args else {
                    return Ok(None);
                };
                match o.get("text") {
                    Some(Value::Str(t)) if t.as_str() == "ok" => {
                        Ok(Some(Value::Str("label".into())))
                    }
                    _ => Ok(Some(Value::Num(5.0))),
                }
            })
            .unwrap();
        funcs
    };
    let defined_library = defined.library();
    let upstream_library = Library::from_json_schema(upstream_schema);
    assert_eq!(defined_library, upstream_library);
    let defined_funcs = register(&defined_schema["functions"]);
    let upstream_funcs = register(&upstream_schema["functions"]);
    let mut seen = Vec::new();
    for source in [
        "root = Box(@Percent(1, 4), @Label(\"ok\"), @Unimplemented(1))",
        "root = Box(@Percent(\"a\", 4), @Percent(1), @Percent(1, 2, 3))",
        "root = Box(@Label(\"x\"), @Label(2), @Label(null, \"s\"))",
        "root = Box(@Nope(1), @Percnt(1, 2))",
    ] {
        let mut outputs = Vec::new();
        for (library, funcs) in [
            (&defined_library, &defined_funcs),
            (&upstream_library, &upstream_funcs),
        ] {
            let p = parse(source, library, None);
            let r = evaluate_tree(
                TreeInput::new(&p)
                    .with_library(library)
                    .with_functions(funcs),
                &Store::new(),
            );
            let errors: Vec<String> = r.errors.iter().map(|e| e.message.clone()).collect();
            outputs.push((p.to_json(), r.to_json().unwrap(), errors));
        }
        assert_eq!(outputs[0], outputs[1], "{source}");
        seen.push(outputs.remove(0));
    }
    // What the calls did: a result, null for an unimplemented call, and
    // reports for bad arguments, a bad result and unknown names.
    assert!(
        seen[0].1.contains(r#""a":25,"b":"label","c":null"#),
        "{:?}",
        seen[0]
    );
    assert!(seen[0].2.is_empty(), "{:?}", seen[0]);
    // A wrong type or a missing argument is dropped by the parser's
    // validation, and an extra one is ignored.
    assert!(
        seen[1].1.contains(r#""a":null,"b":null,"c":50"#),
        "{:?}",
        seen[1]
    );
    assert!(seen[1].0.contains("\"errors\":[{"), "{:?}", seen[1]);
    assert_eq!(
        seen[2].2,
        [
            "Evaluating prop \"a\" on Box failed: @Label: field \"/returns\" expects string but got number"
        ]
    );
    assert!(seen[3].0.contains("unknown-function"), "{:?}", seen[3]);
    drop((defined_funcs, upstream_funcs));
    free_json(defined_schema);
    free_json(golden);
}
