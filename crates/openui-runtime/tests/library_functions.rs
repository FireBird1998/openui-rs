//! Library implementations, returned data, limits, and unwind recovery.
use openui_lang::{Element, JsonValue, Library, ParseResult, free_json, parse, parse_json};
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
            r.to_json(),
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
    funcs.register("NotDeclared", |_| {
        panic!("must not call an undeclared implementation")
    });
    let p = parse("root=Box(@Identity(2),@NotDeclared(2))", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert!(r.errors.is_empty());
    assert!(r.to_json().contains("\"value\":null"), "{}", r.to_json());
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs.register("Identity", |_| Ok(Some(Value::Num(1.0))));
    funcs.register("Identity", |_| Ok(Some(Value::Num(2.0))));
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert!(r.to_json().contains("\"value\":2"));
    drop(funcs);
}
#[test]
fn host_panics_unwind_contexts_and_leave_the_store_reusable() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let calls = AtomicUsize::new(0);
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs.register("Identity", |args| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("embedder panic")
        };
        Ok(Some(args))
    });
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
        r.to_json().contains("[{\"value\":1},{\"value\":2}]"),
        "{}",
        r.to_json()
    );
    drop(funcs);
}
#[test]
fn deferred_set_expressions_never_call_implementations() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut funcs = LibraryFunctions::new(&j["functions"]);
    funcs.register("Throw", |_| panic!("set values must stay deferred"));
    let p = parse("root=Box(@Each([1,2],r,Box(@Set($n,@Throw(r)))))", &l, None);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_functions(&funcs),
        &Store::new(),
    );
    assert!(r.errors.is_empty());
    assert!(
        r.to_json()
            .contains("\"valueAST\":{\"k\":\"Comp\",\"name\":\"Throw\""),
        "{}",
        r.to_json()
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
        let s = r.to_json();
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
    funcs.register("Identity", |_| panic!("arguments failed before invocation"));
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
            funcs.register("Deep", |_| {
                let mut v = Value::Null;
                for _ in 0..10_000 {
                    v = Value::Arr(Arc::new(vec![v]));
                }
                Ok(Some(v))
            });
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
