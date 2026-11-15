//! Substituting mapped component props does not retain duplicate argument trees.
use openui_lang::{Ast, JsObject, Library, parse, parse_expression, parse_json, tokenize};
use openui_runtime::{Limits, Store, TreeInput, Value, evaluate_tree_with_limits};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn mapped(ast: &mut Ast) {
    if let Ast::Comp {
        name,
        args,
        mapped_props,
        ..
    } = ast
    {
        for arg in Arc::make_mut(args) {
            mapped(arg);
        }
        if name == "Box" {
            let mut props = JsObject::new();
            if let Some(child) = args.first() {
                props.define("a", child.clone());
            }
            *mapped_props = Some(Arc::new(props));
        }
    }
}
fn node(kind: &str, fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut out = JsObject::new();
    out.define("k", Value::Str(kind.into()));
    for (key, value) in fields {
        out.define(key, value);
    }
    Value::Obj(Arc::new(out))
}
fn descriptor(depth: usize) -> Value {
    let mut body = node("Str", [("v", Value::Str("u".repeat(1000)))]);
    for i in 0..depth {
        body = node(
            "BinOp",
            [
                ("op", Value::Str("+".into())),
                ("left", body),
                ("right", node("Ref", [("n", Value::Str(format!("i{i}")))])),
            ],
        );
    }
    for i in 0..depth {
        let mut props = JsObject::new();
        props.define("a", body.clone());
        let component = node(
            "Comp",
            [
                ("name", Value::Str("Box".into())),
                ("args", Value::Arr(Arc::new(vec![body]))),
                ("mappedProps", Value::Obj(Arc::new(props))),
            ],
        );
        body = node(
            "Comp",
            [
                ("name", Value::Str("Each".into())),
                (
                    "args",
                    Value::Arr(Arc::new(vec![
                        node("StateRef", [("n", Value::Str("$rows".into()))]),
                        node("Ref", [("n", Value::Str(format!("i{i}")))]),
                        component,
                    ])),
                ),
            ],
        );
    }
    body
}
fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
fn minimum(program: &openui_lang::ParseResult, store: &Store, work: bool) -> usize {
    let (mut lo, mut hi) = (0, 1_000_000);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let result = evaluate_tree_with_limits(
            TreeInput::new(program),
            store,
            Limits::DEFAULT
                .with_result_size(if work { 1_000_000 } else { mid })
                .with_work(if work { mid } else { 1_000_000 }),
        );
        if result.size_limited || result.depth_limited {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    lo
}
#[test]
fn one_item_loops_keep_unique_leaf_memory_instead_of_doubling_it() {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    for data in [false, true] {
        for rows in [1, 3] {
            for depth in 1..=8 {
                let mut body = (0..depth).fold(format!("\"{}\"", "u".repeat(1000)), |s, i| {
                    format!("{s}+i{i}")
                });
                for i in 0..depth {
                    body = format!("@Each($rows,i{i},Box({body}))");
                }
                let mut program = parse("root=Box(0)", &library, None);
                let mut ast = parse_expression(&tokenize(&body));
                mapped(&mut ast);
                let value = if data {
                    descriptor(depth)
                } else {
                    Value::Ast(Arc::new(ast))
                };
                program.root.as_mut().unwrap().has_dynamic_props = true;
                program.root.as_mut().unwrap().props.define("a", value);
                let mut store = Store::new();
                store
                    .set(
                        "$rows",
                        Some(Value::Arr(Arc::new(
                            (0..rows).map(|n| Value::Num(n as f64)).collect(),
                        ))),
                    )
                    .unwrap();
                test_alloc::reset_peak();
                let baseline = test_alloc::peak();
                let result = evaluate_tree_with_limits(
                    TreeInput::new(&program),
                    &store,
                    Limits::DEFAULT
                        .with_result_size(4_100_000)
                        .with_work(1_000_000_000),
                );
                let peak = test_alloc::peak().saturating_sub(baseline);
                println!(
                    "data={data} rows={rows} depth={depth} peak={peak} json={} cut={}",
                    result.to_json().unwrap().len(),
                    result.size_limited
                );
                let json = result.to_json().unwrap();
                let oracle = format!(
                    "{data} {rows} {depth} {} {}",
                    hash(&json),
                    result.size_limited
                );
                assert!(
                    include_str!("support/nested-loop-oracle.tsv")
                        .lines()
                        .any(|line| line == oracle),
                    "{oracle}"
                );
                if rows == 1 {
                    let prices = format!(
                        "{data} {depth} {} {}",
                        minimum(&program, &store, true),
                        minimum(&program, &store, false)
                    );
                    assert!(
                        include_str!("support/nested-loop-prices.tsv")
                            .lines()
                            .any(|line| line == prices),
                        "{prices}"
                    );
                }
                assert!(!result.depth_limited);
                assert!(result.errors.is_empty());
                if rows == 1 {
                    assert!(!result.size_limited);
                    // Input syntax and live traversal frames grow with depth;
                    // the 1 KB leaf must no longer be copied 2^depth times.
                    assert!(peak < 128_000, "data={data} depth={depth} peak={peak}");
                }
            }
        }
    }
}

#[test]
fn an_omitted_prototype_prop_is_not_restored_by_sharing_substituted_bodies() {
    use openui_runtime::LibraryFunctions;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Trap":{"params":{"type":"object","properties":{"x":{}}},"returns":{"type":"number"}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let calls = AtomicUsize::new(0);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Trap", |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Err("an omitted mapped prop was evaluated".into())
        })
        .unwrap();
    let trap = node(
        "Comp",
        [
            ("name", Value::Str("Trap".into())),
            ("fn", Value::Bool(true)),
            (
                "args",
                Value::Arr(Arc::new(vec![node("Ref", [("n", Value::Str("i".into()))])])),
            ),
        ],
    );
    let mut props = JsObject::new();
    props.define("__proto__", trap.clone());
    let template = node(
        "Comp",
        [
            ("name", Value::Str("Box".into())),
            ("args", Value::Arr(Arc::new(vec![trap]))),
            ("mappedProps", Value::Obj(Arc::new(props))),
        ],
    );
    let each = node(
        "Comp",
        [
            ("name", Value::Str("Each".into())),
            (
                "args",
                Value::Arr(Arc::new(vec![
                    node(
                        "Arr",
                        [(
                            "els",
                            Value::Arr(Arc::new(vec![node("Num", [("v", Value::Num(0.))])])),
                        )],
                    ),
                    node("Ref", [("n", Value::Str("i".into()))]),
                    template,
                ])),
            ),
        ],
    );
    let mut program = parse("root=Box(0)", &library, None);
    let root = program.root.as_mut().unwrap();
    root.has_dynamic_props = true;
    root.props.define("a", each);
    let result = evaluate_tree_with_limits(
        TreeInput::new(&program)
            .with_library(&library)
            .with_functions(&functions),
        &Store::new(),
        Limits::DEFAULT,
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(result.errors.is_empty());
    assert!(!result.size_limited && !result.depth_limited);
}
