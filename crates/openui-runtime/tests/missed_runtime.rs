//! Public admission and action observations for the second #40 survivor batch.
use openui_lang::{Element, Library, ParseResult, parse, parse_json};
use openui_runtime::*;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
fn value(s: &str) -> Value {
    Value::from_json(&parse_json(s).unwrap())
}
fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut o = JsObject::new();
    for (k, v) in fields {
        o.define(k, v);
    }
    Value::Obj(Arc::new(o))
}
fn root(v: Value) -> ParseResult {
    let mut props = JsObject::new();
    props.define("a", v);
    ParseResult {
        root: Some(Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..Default::default()
    }
}
fn limits(dim: &str, n: usize) -> Limits {
    match dim {
        "work" => Limits::DEFAULT.with_work(n),
        "room" => Limits::DEFAULT.with_result_size(n),
        "depth" => Limits::DEFAULT.with_depth(n),
        _ => panic!("dimension"),
    }
}
fn record(name: &str, rows: Vec<String>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support")
        .join(format!("missed-{name}.txt"));
    let actual = rows.join("\n") + "\n";
    if std::env::var_os("UPDATE_MISSED_RUNTIME").is_some() {
        std::fs::write(path, actual).unwrap();
    } else {
        let expected = std::fs::read_to_string(path).unwrap();
        for (index, (a, b)) in actual.lines().zip(expected.lines()).enumerate() {
            assert_eq!(a, b, "{name} observation {index}");
        }
        assert_eq!(
            actual.lines().count(),
            expected.lines().count(),
            "{name} observation count"
        );
    }
}
fn digest(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
fn marker(expr: Value) -> Value {
    object([
        ("__reactive", Value::Str("assign".into())),
        ("target", Value::Str("$out".into())),
        ("expr", expr),
    ])
}
fn admission_values() -> Vec<Value> {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap(),
    );
    let mut out = vec![
        Value::Str("\u{1}\u{20}é\u{f0000}\n\\\"".into()),
        value("[1,2,3]"),
        value(r#"{"a":1,"b":2}"#),
    ];
    for src in [
        "'long text'",
        "missing",
        "{x}",
        "$state",
        "[1,2,3]",
        "{a:1,b:2}",
        "Box(1,2)",
        "$state+2",
        "$state.x",
        "$state[2]",
        "$state?1:2",
        "$state=2",
        "!$state",
    ] {
        let p = parse(&format!("root=Box({src})"), &library, None);
        if let Some(v) = p.root.unwrap().props.get("a") {
            out.push(v.clone());
        }
    }
    out.push(Value::Ast(Arc::new(Ast::RuntimeRef {
        n: "query-name".into(),
        ref_type: openui_lang::RefType::Query,
    })));
    out.push(Value::Element(Arc::new(root(value("[1,2]")).root.unwrap())));
    let shared = Arc::new(vec![Value::Str("shared".into())]);
    out.push(Value::Arr(Arc::new(vec![
        Value::Arr(shared.clone()),
        Value::Arr(shared),
    ])));
    let leaf = Arc::new(Ast::Str("long text\u{1}".into()));
    let mut mapped = JsObject::new();
    mapped.define("named", Ast::Str("mapped".into()));
    for ast in [
        Ast::Str("text".into()),
        Ast::Ref("ref".into()),
        Ast::Ph("ph".into()),
        Ast::StateRef("$state".into()),
        Ast::Arr(Arc::new(vec![Ast::Null, Ast::Null])),
        Ast::Obj(Arc::new(vec![
            ("key".into(), Ast::Null),
            ("other".into(), Ast::Null),
        ])),
        Ast::Comp {
            name: "Other".into(),
            args: Arc::new(vec![Ast::Null, Ast::Null]),
            function: false,
            mapped_props: Some(Arc::new(mapped)),
        },
        Ast::BinOp {
            op: "+",
            left: leaf.clone(),
            right: leaf.clone(),
        },
        Ast::Member {
            obj: leaf.clone(),
            field: "field".into(),
        },
        Ast::Index {
            obj: leaf.clone(),
            index: leaf.clone(),
        },
        Ast::Ternary {
            cond: leaf.clone(),
            then: leaf.clone(),
            otherwise: leaf.clone(),
        },
        Ast::Assign {
            target: "$target".into(),
            value: leaf,
        },
    ] {
        out.push(Value::Ast(Arc::new(ast)));
    }
    let mut mapped = JsObject::new();
    for key in ["a", "b", "c", "d"] {
        mapped.define(key, Ast::Null);
    }
    out.push(Value::Ast(Arc::new(Ast::Comp {
        name: "Mapped".into(),
        args: Arc::new(vec![]),
        function: false,
        mapped_props: Some(Arc::new(mapped)),
    })));
    out
}
#[test]
fn field_admission_preserves_typed_syntax_work_depth_and_escaped_text_edges() {
    let mut rows = Vec::new();
    for (case, v) in admission_values().into_iter().enumerate() {
        for (dim, max) in [("work", 400), ("room", 300), ("depth", 12)] {
            let mut last = String::new();
            for n in 0..=max {
                let mut store = Store::new();
                let r = set_field_value_with_limits(
                    &mut store,
                    None,
                    Some("Text"),
                    "x",
                    Some(v.clone()),
                    false,
                    limits(dim, n),
                );
                let observation = format!("{:?} {}", r, store.version());
                if observation != last {
                    rows.push(format!("{case} {dim} {n} {observation}"));
                    last = observation;
                }
            }
        }
    }
    record("field-admission", rows);
}
#[test]
fn action_resume_errors_and_opaque_continuations_have_stable_text() {
    for (e, s) in [
        (ActionResumeError::Disposed, "Disposed"),
        (ActionResumeError::ForeignSession, "ForeignSession"),
        (ActionResumeError::Consumed, "Consumed"),
    ] {
        assert_eq!(e.to_string(), s);
    }
    let p = ParseResult::default();
    let mut store = Store::new();
    let action = value(r#"{"steps":[{"type":"run","refType":"mutation","statementId":"m"}]}"#);
    let run = run_action(TreeInput::new(&p), &mut store, Some(&action), "", None);
    assert_eq!(
        format!("{:?}", run.continuation.unwrap()),
        "ActionContinuation { .. }"
    );
}
#[test]
fn action_string_steps_are_ignored_and_string_reset_targets_iterate_characters() {
    let mut p = ParseResult::default();
    p.state_declarations.define("a", Value::Num(1.));
    p.state_declarations.define("é", Value::Num(2.));
    let mut store = Store::new();
    store.set("a", Some(Value::Num(9.))).unwrap();
    store.set("é", Some(Value::Num(9.))).unwrap();
    let r = run_action(
        TreeInput::new(&p),
        &mut store,
        Some(&value(r#"{"steps":"nonempty"}"#)),
        "",
        None,
    );
    assert!(r.failure.is_none() && r.events.is_empty());
    let r = run_action(
        TreeInput::new(&p),
        &mut store,
        Some(&value(r#"{"steps":[{"type":"reset","targets":"aé"}]}"#)),
        "",
        None,
    );
    assert!(r.failure.is_none());
    assert_eq!(store.get("a"), Some(Value::Num(1.)));
    assert_eq!(store.get("é"), Some(Value::Num(2.)));
    for v in [Value::Num(1.), Value::Bool(true), Value::Str("x".into())] {
        let r = run_action(TreeInput::new(&p), &mut store, Some(&v), "", None);
        assert!(
            matches!(r.failure.unwrap().cause,BindingWriteError::Evaluation(EvaluationFailure::Callback(ref s)) if s=="action is not an object")
        );
    }
}
#[test]
fn action_events_keep_form_spread_and_text_budget_edges() {
    let actions = parse_json("{}").unwrap();
    let p = ParseResult::default();
    let mut rows = Vec::new();
    for (case, action) in [
        value(r#"{"type":"go","params":"éx"}"#),
        value(r#"{"type":"go","params":{"x":1,"q":"text"}}"#),
        value(r#"{"steps":[]}"#),
        Value::Null,
    ]
    .into_iter()
    .enumerate()
    {
        for form in [None, Some("f")] {
            for (dim, max) in [("work", 400), ("room", 300), ("depth", 12)] {
                let mut last = String::new();
                for n in 0..=max {
                    let mut store = Store::new();
                    store.set("f", Some(value(r#"{"x":{"value":7}}"#))).unwrap();
                    let r = run_action_with_limits(
                        TreeInput::new(&p).with_actions(&actions),
                        &mut store,
                        Some(&action),
                        "message",
                        form,
                        limits(dim, n),
                    );
                    let obs = format!(
                        "{:?} {} {} {:?}",
                        r.failure,
                        r.size_limited,
                        r.depth_limited,
                        r.events
                            .iter()
                            .map(|e| e.to_json().unwrap())
                            .collect::<Vec<_>>()
                    );
                    if last != obs {
                        rows.push(format!("{case} {form:?} {dim} {n} {obs}"));
                        last = obs;
                    }
                }
            }
        }
    }
    record("action-events", rows);
}
#[test]
fn binding_annotations_balance_nested_types_and_report_ambiguous_quotes() {
    let mut rows = Vec::new();
    for annotation in [
        "$binding<(string)>",
        "$binding<'ok'>",
        "$binding<'a b'>",
        "$binding<'a'bad'>",
        "$binding<'a' | 'b'>",
        "$binding<\"x\"[]>",
        "$binding<`x`>",
        "$binding<string>[]",
        "$binding<'x' >",
    ] {
        for structural in [false, true] {
            let signature = format!("Box(a: {annotation}, b: $binding<number>)");
            let mut spec = object([(
                "components",
                object([("Box", object([("signature", Value::Str(signature))]))]),
            )]);
            if structural && let Value::Obj(o) = &mut spec {
                Arc::make_mut(o).define(
                    "schema",
                    value(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#),
                );
            }
            let b = BindingProps::from_prompt_spec(&spec).unwrap();
            rows.push(format!(
                "{annotation} {structural} {} {} {:?}",
                b.contains("Box", "a"),
                b.contains("Box", "b"),
                b.undecided().collect::<Vec<_>>()
            ));
        }
    }
    record("annotations", rows);
}
#[test]
fn binding_transforms_keep_calls_queries_and_loop_values_under_limits() {
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"a":{}}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    functions.register("Echo", |v| Ok(Some(v))).unwrap();
    let p = parse("q=Query(\"q\",{},[1,2])\nroot=Box()", &library, None);
    let mut refs = JsObject::new();
    refs.define("q", value("[3,4,5]"));
    let prepared = PreparedReferenceValues::new(&refs);
    let mut rows = Vec::new();
    for (case,expr) in [
        r#"{"k":"Comp","name":"Echo","args":[{"k":"StateRef","n":"$value"}]}"#,
        r#"{"k":"Comp","name":"Other","args":[]}"#,
        r#"{"k":"Arr","els":[{"k":"RuntimeRef","n":"q","refType":"query"},{"k":"RuntimeRef","n":"q","refType":"query"}]}"#,
        r#"{"k":"Comp","name":"Each","args":[{"k":"Arr","els":[{"k":"Num","v":1},{"k":"Num","v":2}]},{"k":"Ref","n":"i"},{"k":"Ref","n":"i"}]}"#,
        r#"{"k":"Comp","name":"Box","args":[],"mappedProps":{"a":{"k":"StateRef","n":"$value"}}}"#,
    ].into_iter().enumerate() {
        let m=marker(value(expr));
        for (dim,max) in [("work",600),("room",400),("depth",15)] {
            let mut last=String::new();
            for n in 0..=max {
                let mut store=Store::new();
                let r=write_binding_with_limits(TreeInput::new(&p).with_library(&library).with_functions(&functions).with_prepared_reference_values(&prepared),&mut store,&m,Value::Num(7.),limits(dim,n));
                let obs=format!("{r:?} {:?}",store.get("$out").map(|v|value_to_json(&v).unwrap()));
                if obs!=last {rows.push(format!("{case} {dim} {n} {obs}"));last=obs;}
            }
        }
    }
    record("binding-transforms", rows);
}
#[test]
fn declared_actions_preserve_argument_shapes_failures_and_defaults() {
    let mut rows = Vec::new();
    let schemas = [
        r#"{"params":{"type":"object","properties":{"a":{},"b":{}}}}"#,
        r#"{"params":{"properties":[{},{}]}}"#,
        r#"{"params":{"properties":"ab"}}"#,
        r#"{"params":{"type":"object","properties":{"a":{"default":null},"b":{}},"required":["a"]}}"#,
    ];
    let args = [
        "null",
        "[]",
        r#"[{"k":"Null"}]"#,
        r#"[{"k":"Ref","n":"missing"}]"#,
        r#"[{"k":"Num","v":7}]"#,
        r#""ab""#,
        r#"{"length":2,"0":{"k":"Str","v":"url"},"1":{"k":"Null"}}"#,
    ];
    for (si, schema) in schemas.iter().enumerate() {
        let actions = parse_json(&format!(r#"{{"Go":{schema}}}"#)).unwrap();
        for name in ["Go", "Set", "OpenUrl", "ToAssistant", "Run"] {
            for (ai, args) in args.iter().enumerate() {
                let p = root(value(&format!(
                    r#"{{"k":"Comp","name":"{name}","args":{args}}}"#
                )));
                let tree = evaluate_tree(TreeInput::new(&p).with_actions(&actions), &Store::new());
                rows.push(format!("{si} {name} {ai} {}", tree.to_json().unwrap()));
            }
        }
    }
    record("declared-actions", rows);
}
#[test]
fn declared_argument_failures_and_cuts_stop_before_emitting_plans() {
    let actions =
        parse_json(r#"{"Go":{"params":{"type":"object","properties":{"a":{},"b":{}}}}}"#).unwrap();
    let mut rows = Vec::new();
    let args = [
        r#""args":[{"k":"Num","v":7},{"k":"Str","v":"text"}]"#,
        r#""args":[],"mappedProps":{"a":{"k":"Num","v":7},"b":{"k":"Str","v":"text"}}"#,
        r#""args":[{"k":"Obj","entries":[["a",{"k":"Num","v":7}],["b",{"k":"Str","v":"text"}]]}]"#,
        r#""args":[],"mappedProps":{"a":{"k":"Comp","name":"Fail","args":[]}}"#,
    ];
    for (case, args) in args.iter().enumerate() {
        let mut actions = actions.clone();
        if case == 2 {
            actions["Go"]["params"]["properties"] = parse_json(r#"{"__proto__":{}}"#).unwrap();
        }
        let p = root(value(&format!(r#"{{"k":"Comp","name":"Go",{args}}}"#)));
        for (dim, max) in [("work", 900), ("room", 500), ("depth", 12)] {
            let mut last = String::new();
            for n in 0..=max {
                let tree = evaluate_tree_with_limits(
                    TreeInput::new(&p).with_actions(&actions),
                    &Store::new(),
                    limits(dim, n),
                );
                let obs = format!(
                    "{:016x} {} {}",
                    digest(&tree.to_json().unwrap()),
                    tree.size_limited,
                    tree.depth_limited
                );
                if obs != last {
                    rows.push(format!("{case} {dim} {n} {obs}"));
                    last = obs;
                }
            }
        }
    }
    record("argument-cuts", rows);
}
#[test]
fn mixed_binding_decisions_charge_each_retained_component_and_refund_resolution() {
    let mut rows = Vec::new();
    for width in [1, 80, 160] {
        let name = "C".repeat(width);
        let mut components = JsObject::new();
        components.define(
            &name,
            object([(
                "signature",
                Value::Str(
                    "X(a: $binding<string>, b: $binding<'x'bad>, c: $binding<'y'bad>)".into(),
                ),
            )]),
        );
        let spec = object([("components", Value::Obj(Arc::new(components)))]);
        let mut last = String::new();
        for room in 0..800 {
            let r = BindingProps::from_prompt_spec_with_limits(
                &spec,
                Limits::DEFAULT.with_result_size(room),
            );
            let obs = match r {
                Err(e) => format!("{:?}", e.flags),
                Ok(mut b) => {
                    let decisions = format!(
                        "{} {:?}",
                        b.contains(&name, "a"),
                        b.undecided().collect::<Vec<_>>()
                    );
                    let first = b.insert(&name, "b");
                    let middle = b.insert("New", "extra");
                    let second = b.insert(&name, "c");
                    let extra = b.insert("New", "extra");
                    format!("{decisions} {first:?} {middle:?} {second:?} {extra:?}")
                }
            };
            if last != obs {
                rows.push(format!("{width} {room} {obs}"));
                last = obs;
            }
        }
    }
    record("binding-room", rows);
}
#[test]
fn mapped_null_and_coerced_action_names_keep_argument_errors() {
    let actions = parse_json(
        r#"{"Action":{"params":{"properties":{}}},"Go":{"params":{"properties":{"a":{}}}}}"#,
    )
    .unwrap();
    let mut rows = Vec::new();
    for raw in [
        r#"{"k":"Comp","name":"OpenUrl","args":[{"k":"Str","v":"url"}],"mappedProps":null}"#,
        r#"{"k":"Comp","name":["Action"],"args":null}"#,
        r#"{"k":"Comp","name":["Go"],"args":null}"#,
        r#"{"k":"Comp","name":"Set","args":[{"k":"StateRef","n":"$x"}],"mappedProps":{}}"#,
        r#"{"k":"Comp","name":"Set","args":"ab"}"#,
        r#"{"k":"Comp","name":"OpenUrl","args":"ab"}"#,
    ] {
        let p = root(value(raw));
        let tree = evaluate_tree(TreeInput::new(&p).with_actions(&actions), &Store::new());
        rows.push(tree.to_json().unwrap());
    }
    record("action-names", rows);
}
#[test]
fn function_return_truthiness_preserves_undefined_and_argument_caches_keep_identity() {
    let mut rows = Vec::new();
    for returns in ["0", "1", r#""""#, r#""x""#] {
        let mut defs = parse_json(r#"{"Echo":{"params":{"properties":{}}}}"#).unwrap();
        defs["Echo"]["returns"] = parse_json(returns).unwrap();
        let mut functions = LibraryFunctions::new(&defs);
        functions.register("Echo", |_| Ok(None)).unwrap();
        let p = root(value(r#"{"k":"Comp","name":"Echo","args":[]}"#));
        for (dim, max) in [("work", 300), ("room", 250), ("depth", 10)] {
            let mut last = String::new();
            for n in 0..=max {
                let tree = evaluate_tree_with_limits(
                    TreeInput::new(&p).with_functions(&functions),
                    &Store::new(),
                    limits(dim, n),
                );
                let obs = format!(
                    "{} {} {}",
                    tree.to_json().unwrap(),
                    tree.size_limited,
                    tree.depth_limited
                );
                if obs != last {
                    rows.push(format!("{returns} {dim} {n} {obs}"));
                    last = obs;
                }
            }
        }
    }
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"a":{}}}},"Other":{"params":{"type":"object","properties":{"a":{"type":"string"}}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    for name in ["Echo", "Other"] {
        functions.register(name, |v| Ok(Some(v))).unwrap();
    }
    for source in [
        "root=Box(@Echo($a),@Echo($b))",
        "root=Box(@Echo($a),@Other($a))",
        "root=Box(@Echo($a),Box(@Echo($a)))",
    ] {
        let p = parse(source, &library, None);
        let mut store = Store::new();
        store.set("$a", Some(value(r#"{"x":1}"#))).unwrap();
        store.set("$b", Some(value(r#"{"x":2}"#))).unwrap();
        let tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &store);
        rows.push(tree.to_json().unwrap());
    }
    record("function-caches", rows);
}
#[test]
fn caller_text_at_the_room_ceiling_is_accepted_for_an_empty_step_plan() {
    let p = ParseResult::default();
    let mut store = Store::new();
    let action = value(r#"{"steps":[]}"#);
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&action),
        &"x".repeat(256),
        None,
        Limits::DEFAULT.with_result_size(256),
    );
    assert!(
        r.failure.is_none() && !r.size_limited && !r.depth_limited && r.events.is_empty(),
        "{r:?}"
    );
}
#[test]
fn supplied_field_syntax_preserves_scaled_room_and_first_refusal_kind() {
    let mut rows = Vec::new();
    for (case, v) in admission_values().into_iter().enumerate() {
        for depth in [1, 2, 3, 4, 20] {
            let mut last = String::new();
            for work in 0..80 {
                let mut store = Store::new();
                let r = set_field_value_with_limits(
                    &mut store,
                    None,
                    None,
                    "",
                    Some(v.clone()),
                    false,
                    Limits::DEFAULT.with_depth(depth).with_work(work),
                );
                let obs = format!("{r:?}");
                if last != obs {
                    rows.push(format!("{case} depth={depth} work={work} {obs}"));
                    last = obs;
                }
            }
        }
        let mut last = String::new();
        for floor in 0..350 {
            let mut store = Store::new();
            let r = set_field_value_with_limits(
                &mut store,
                None,
                None,
                "",
                Some(v.clone()),
                false,
                Limits::DEFAULT
                    .with_result_floor(floor)
                    .with_amplification(1),
            );
            let obs = format!("{r:?}");
            if last != obs {
                rows.push(format!("{case} floor={floor} {obs}"));
                last = obs;
            }
        }
    }
    record("field-sizing", rows);
}
#[test]
fn refused_reference_tail_does_not_cut_an_absent_declared_supply() {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let p = parse("q=Query(\"q\",{},7)\nroot=Box(q)", &library, None);
    let mut refs = JsObject::new();
    refs.define("other", value("[1,2,3]"));
    let prepared = PreparedReferenceValues::with_limits(&refs, Limits::DEFAULT.with_work(0));
    let a = evaluate_tree(
        TreeInput::new(&p).with_prepared_reference_values(&prepared),
        &Store::new(),
    );
    let b = evaluate_tree(TreeInput::new(&p), &Store::new());
    assert_eq!(a.to_json().unwrap(), b.to_json().unwrap());
    assert_eq!(
        (a.size_limited, a.depth_limited),
        (b.size_limited, b.depth_limited)
    );
}
#[test]
fn binding_query_registration_and_repeated_reference_reads_keep_their_work() {
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Echo":{"params":{"properties":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Echo", |_| {
            Ok(Some(Value::Num(
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as f64,
            )))
        })
        .unwrap();
    let mut rows = Vec::new();
    for (case, source) in [
        "q=Query(\"q\",{},@Echo())\nr=Query(\"r\",{},[q,q])\nroot=Box()",
        "q=Query(\"q\",{},[1,2,3])\nroot=Box()",
        "q=Mutation(\"q\",{})\nroot=Box()",
    ]
    .into_iter()
    .enumerate()
    {
        let p = parse(source, &library, None);
        let expr = value(
            r#"{"k":"Comp","name":"Each","args":[{"k":"Arr","els":[{"k":"Num","v":1},{"k":"Num","v":2},{"k":"Num","v":3}]},{"k":"Ref","n":"i"},{"k":"RuntimeRef","n":"q","refType":"query"}]}"#,
        );
        let m = marker(expr);
        let mut refs = JsObject::new();
        refs.define("q", Value::Str("x".repeat(200)));
        let prepared = PreparedReferenceValues::new(&refs);
        for supplied in [false, true] {
            let mut last = String::new();
            for work in 0..1600 {
                calls.store(0, std::sync::atomic::Ordering::SeqCst);
                let mut store = Store::new();
                let mut input = TreeInput::new(&p).with_functions(&functions);
                if supplied {
                    input = input.with_prepared_reference_values(&prepared);
                }
                let r = write_binding_with_limits(
                    input,
                    &mut store,
                    &m,
                    Value::Null,
                    Limits::DEFAULT.with_work(work),
                );
                let obs = format!(
                    "{r:?} {} {:?}",
                    calls.load(std::sync::atomic::Ordering::SeqCst),
                    store.get("$out").map(|v| value_to_json(&v).unwrap())
                );
                if obs != last {
                    rows.push(format!("{case} {supplied} {work} {obs}"));
                    last = obs;
                }
            }
        }
    }
    record("binding-queries", rows);
}
#[test]
fn numeric_action_definitions_control_whether_type_coercion_is_attempted() {
    let p = ParseResult::default();
    let action = value(r#"{"type":{"toString":1,"valueOf":1}}"#);
    for metadata in ["0", "1"] {
        let actions = parse_json(metadata).unwrap();
        let mut store = Store::new();
        let r = run_action(
            TreeInput::new(&p).with_actions(&actions),
            &mut store,
            Some(&action),
            "",
            None,
        );
        if metadata == "0" {
            assert!(r.failure.is_none());
            assert_eq!(r.events.len(), 1);
        } else {
            assert!(matches!(
                r.failure.unwrap().cause,
                BindingWriteError::Evaluation(EvaluationFailure::CannotConvertToPrimitive)
            ));
            assert!(r.events.is_empty());
        }
    }
}
#[test]
fn structural_binding_anchors_require_a_colon_followed_by_space() {
    let spec = value(
        r#"{"components":{"Box":{"signature":"Box(a:$binding<string>)"}},"schema":{"$defs":{"Box":{"properties":{"a":{}}}}}}"#,
    );
    let b = BindingProps::from_prompt_spec(&spec).unwrap();
    assert!(!b.contains("Box", "a"));
    assert_eq!(b.undecided().collect::<Vec<_>>(), [("Box", "a")]);
}
#[test]
fn undefined_field_names_admit_exact_text_work_and_room() {
    let mut rows = Vec::new();
    for name in ["", "abc", "\u{1}", "é", "\u{f0000}"] {
        for dim in ["work", "room"] {
            for n in 0..20 {
                let mut store = Store::new();
                let r = set_field_value_with_limits(
                    &mut store,
                    None,
                    None,
                    name,
                    None,
                    false,
                    limits(dim, n),
                );
                rows.push(format!("{name:?} {dim} {n} {r:?}"));
            }
        }
    }
    record("field-text", rows);
}
#[test]
fn binding_marker_metadata_is_admitted_even_when_the_transform_does_not_read_it() {
    let p = ParseResult::default();
    let mut rows = Vec::new();
    for (case, v) in admission_values().into_iter().enumerate() {
        let mut m = marker(Value::Ast(Arc::new(Ast::Num(1.))));
        if let Value::Obj(o) = &mut m {
            Arc::make_mut(o).define("metadata", v);
        }
        for depth in [2, 3, 4, 20] {
            let mut last = String::new();
            for work in 0..550 {
                let mut store = Store::new();
                let r = write_binding_with_limits(
                    TreeInput::new(&p),
                    &mut store,
                    &m,
                    Value::Null,
                    Limits::DEFAULT
                        .with_result_size(4)
                        .with_work(work)
                        .with_depth(depth),
                );
                let obs = format!("{r:?}");
                if last != obs {
                    rows.push(format!("{case} depth={depth} work={work} {obs}"));
                    last = obs;
                }
            }
        }
    }
    record("marker-admission", rows);
}
#[test]
fn repeated_mapped_function_arguments_reuse_the_same_public_object() {
    let defs = parse_json(r#"{"Echo":{"params":{"properties":{"a":{}}}}}"#).unwrap();
    let seen = std::sync::Mutex::new(Vec::<Value>::new());
    let mut f = LibraryFunctions::new(&defs);
    f.register("Echo", |v| {
        seen.lock().unwrap().push(v.clone());
        Ok(Some(v))
    })
    .unwrap();
    let call = value(
        r#"{"k":"Comp","name":"Echo","args":[],"mappedProps":{"a":{"k":"StateRef","n":"$x"}}}"#,
    );
    let p = root(Value::Arr(Arc::new(vec![call.clone(), call.clone(), call])));
    let mut store = Store::new();
    store.set("$x", Some(value(r#"{"v":1}"#))).unwrap();
    let tree = evaluate_tree(TreeInput::new(&p).with_functions(&f), &store);
    assert!(!tree.size_limited && !tree.depth_limited && tree.errors.is_empty());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    for item in seen.iter().skip(1) {
        let (Value::Obj(a), Value::Obj(b)) = (&seen[0], item) else {
            panic!("objects")
        };
        assert!(Arc::ptr_eq(a, b));
    }
}
#[test]
fn failed_argument_evaluation_stops_before_spending_later_argument_work() {
    let bad = r#"{"k":"Comp","name":"ToAssistant","args":null}"#;
    let mut rows = Vec::new();
    for (case, fragment) in [
        format!(r#""args":[{bad}]"#),
        format!(r#""args":[],"mappedProps":{{"a":{bad}}}"#),
        format!(r#""args":[{{"a":{bad}}}]"#),
    ]
    .iter()
    .enumerate()
    {
        let properties = if case == 2 {
            r#"{"__proto__":{}}"#
        } else {
            r#"{"a":{}}"#
        };
        let actions = parse_json(&format!(
            r#"{{"Go":{{"params":{{"properties":{properties}}}}}}}"#
        ))
        .unwrap();
        let p = root(value(&format!(r#"{{"k":"Comp","name":"Go",{fragment}}}"#)));
        let mut last = String::new();
        for n in 0..1200 {
            let t = evaluate_tree_with_limits(
                TreeInput::new(&p).with_actions(&actions),
                &Store::new(),
                Limits::DEFAULT.with_work(n),
            );
            let obs = format!(
                "{} {} {}",
                t.to_json().unwrap(),
                t.size_limited,
                t.depth_limited
            );
            if last != obs {
                rows.push(format!("{case} {n} {obs}"));
                last = obs;
            }
        }
    }
    record("failed-arguments", rows);
}
#[test]
fn escaped_field_names_share_the_exact_work_and_quoted_room_boundaries() {
    let mut rows = Vec::new();
    for name in ["", "abc", "\u{1}", "é"] {
        for work in 0..12 {
            for room in 0..16 {
                let mut store = Store::new();
                let r = set_field_value_with_limits(
                    &mut store,
                    None,
                    None,
                    name,
                    None,
                    false,
                    Limits::DEFAULT.with_work(work).with_result_size(room),
                );
                rows.push(format!("{name:?} {work} {room} {r:?}"));
            }
        }
    }
    record("escaped-field-text", rows);
}
#[test]
fn binding_loop_item_callbacks_and_unknown_calls_keep_their_work_edges() {
    let p = ParseResult::default();
    let mut rows = Vec::new();
    for template in [
        r#"{"k":"Ref","n":"i"}"#,
        r#"{"k":"Comp","name":"Unknown","args":[]}"#,
    ] {
        let expr = value(&format!(
            r#"{{"k":"Comp","name":"Each","args":[{{"k":"StateRef","n":"$value"}},{{"k":"Ref","n":"i"}},{template}]}}"#
        ));
        let m = marker(expr);
        let input = value(&format!("[{}]", vec!["1"; 40].join(",")));
        let mut last = String::new();
        for work in 0..1600 {
            let mut store = Store::new();
            let r = write_binding_with_limits(
                TreeInput::new(&p),
                &mut store,
                &m,
                input.clone(),
                Limits::DEFAULT.with_work(work),
            );
            let obs = format!(
                "{r:?} {:?}",
                store.get("$out").map(|v| value_to_json(&v).unwrap())
            );
            if last != obs {
                rows.push(format!("{template} {work} {obs}"));
                last = obs;
            }
        }
    }
    record("binding-loops", rows);
}
#[test]
fn array_like_declared_arguments_respect_their_reported_length() {
    let actions = parse_json(r#"{"Go":{"params":{"properties":{"a":{}}}}}"#).unwrap();
    let p = root(value(
        r#"{"k":"Comp","name":"Go","args":{"length":0,"0":{"k":"Num","v":7}}}"#,
    ));
    let tree = evaluate_tree(TreeInput::new(&p).with_actions(&actions), &Store::new());
    let Some(Value::Element(root)) = &tree.root else {
        panic!("root")
    };
    assert_eq!(
        value_to_json(root.props.get("a").unwrap()).unwrap(),
        r#"{"steps":[{"type":"custom_action","name":"Go","params":{}}]}"#
    );
}
#[test]
fn borrowed_mapped_arguments_avoid_copying_the_temporary_name_map() {
    for width in [16, 64] {
        let definitions = parse_json(r#"{"Echo":{"params":{"properties":{}}}}"#).unwrap();
        let mut f = LibraryFunctions::new(&definitions);
        f.register("Echo", |v| Ok(Some(v))).unwrap();
        let mut mapped = JsObject::new();
        for i in 0..width {
            mapped.define(format!("key{i}"), value(r#"{"k":"StateRef","n":"$x"}"#));
        }
        let mut store = Store::new();
        store.set("$x", Some(Value::Num(1.))).unwrap();
        let measure = |mapped: JsObject<Value>| {
            let p = root(object([
                ("k", Value::Str("Comp".into())),
                ("name", Value::Str("Echo".into())),
                ("mappedProps", Value::Obj(Arc::new(mapped))),
            ]));
            // Initialize thread/process evaluator helpers before comparing
            // allocation calls; the first use can own shared initialization.
            drop(evaluate_tree(TreeInput::new(&p).with_functions(&f), &store));
            let start = test_alloc::allocation_calls();
            let r = evaluate_tree(TreeInput::new(&p).with_functions(&f), &store);
            let calls = test_alloc::allocation_calls() - start;
            assert!(
                !r.size_limited && !r.depth_limited && r.errors.is_empty(),
                "{r:?}"
            );
            (calls, r.to_json().unwrap())
        };
        let fast = measure(mapped.clone());
        mapped.define("__proto__", Value::Null);
        let slow = measure(mapped);
        assert_eq!(fast.1, slow.1);
        assert!(
            fast.0 + width / 2 < slow.0,
            "width={width}, borrowed={}, copied={}",
            fast.0,
            slow.0
        );
    }
}
#[test]
fn mapped_action_arguments_do_not_coerce_the_unused_positional_length() {
    let p = root(value(
        r#"{"k":"Comp","name":"OpenUrl","args":{"length":{"toString":1,"valueOf":1}},"mappedProps":{"url":{"k":"Str","v":"url"}}}"#,
    ));
    let t = evaluate_tree(TreeInput::new(&p), &Store::new());
    assert!(t.errors.is_empty(), "{t:?}");
    let Some(Value::Element(root)) = &t.root else {
        panic!("root")
    };
    assert_eq!(
        value_to_json(root.props.get("a").unwrap()).unwrap(),
        r#"{"steps":[{"type":"open_url","url":"url"}]}"#
    );
}
#[test]
fn string_action_arguments_pay_for_evaluating_their_indexed_characters() {
    let mut rows = Vec::new();
    let p = root(value(r#"{"k":"Comp","name":"ToAssistant","args":"a"}"#));
    let mut last = String::new();
    for work in 0..600 {
        let t = evaluate_tree_with_limits(
            TreeInput::new(&p),
            &Store::new(),
            Limits::DEFAULT.with_work(work),
        );
        let obs = format!(
            "{} {} {}",
            t.to_json().unwrap(),
            t.size_limited,
            t.depth_limited
        );
        if last != obs {
            rows.push(format!("{work} {obs}"));
            last = obs;
        }
    }
    record("string-arguments", rows);
}
