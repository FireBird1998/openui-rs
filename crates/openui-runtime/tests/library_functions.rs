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
