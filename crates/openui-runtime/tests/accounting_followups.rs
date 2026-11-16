//! Accounting edges through public interfaces, independent of host lifetimes.
use openui_lang::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Limits, Value, parse_expression,
    parse_json, tokenize,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
struct Scope(Value);
impl EvaluationScope for Scope {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
    fn reference(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
}
#[test]
fn false_output_needs_five_bytes_and_true_needs_four() {
    for (value, bytes) in [(false, 5), (true, 4)] {
        for room in [bytes - 1, bytes] {
            let s = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_size(room));
            let out = s.evaluate(&Ast::Bool(value));
            assert_eq!(out.size_limited, room < bytes);
            if room == bytes {
                assert_eq!(out.value, Some(Value::Bool(value)));
            }
        }
    }
}
#[test]
fn state_and_query_member_and_index_reads_have_the_same_depth() {
    for n in [1, 4, 10, 20] {
        let mut value = Value::Null;
        for _ in 0..n {
            value = Value::Arr(Arc::new(vec![value]));
        }
        let scope = Scope(value);
        for depth in 1..15 {
            for suffix in ["[0]", ".length", "[0][0]"] {
                let read = |name| {
                    EvaluationSession::with_limits(
                        &scope,
                        100_000,
                        Limits::DEFAULT.with_depth(depth),
                    )
                    .evaluate(&parse_expression(&tokenize(&format!("{name}{suffix}"))))
                };
                let state = read("$s");
                let query = read("q");
                assert_eq!(
                    (state.size_limited, state.depth_limited),
                    (query.size_limited, query.depth_limited),
                    "n={n},depth={depth},suffix={suffix}"
                );
                assert_eq!(state.value, query.value);
            }
        }
    }
}
#[test]
fn scalar_state_payload_reads_earn_credit_once_per_run() {
    let s = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT
            .with_result_floor(0)
            .with_amplification(1)
            .with_work(10_000),
    );
    s.with_evaluator(|e| {
        assert!(e.read_state_value("$x", &Value::Num(123.)));
        assert!(e.read_state_key("$x"));
        let before = test_alloc::allocation_calls();
        for _ in 0..100 {
            assert!(e.read_state_value("$x", &Value::Num(123.)));
            assert!(e.read_state_key("$x"));
        }
        assert_eq!(
            test_alloc::allocation_calls(),
            before,
            "unchanged keys must not allocate"
        );
        assert!(e.reserve_result(7)); // 3-byte number plus quoted key.
        assert!(!e.reserve_result(1));
    });
}

#[test]
fn argument_cache_misses_reuse_unchanged_text_projections_when_the_host_releases_them() {
    use openui_runtime::{LibraryFunctions, Store, TreeInput, evaluate_tree};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    let schema=openui_lang::parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Read":{"params":{"type":"object","properties":{"text":{"type":"string"},"index":{"type":"number"}}},"returnType":{"type":"number"}}}}"#).unwrap();
    let library = openui_lang::Library::from_json_schema(&schema);
    let calls = AtomicUsize::new(0);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Read", |args| {
            calls.fetch_add(1, Relaxed);
            let Value::Obj(ref o) = args else {
                panic!("args")
            };
            assert!(matches!(o.get("text"),Some(Value::Str(s)) if s.len()==100_000));
            Ok(o.get("index").cloned())
        })
        .unwrap();
    let p = openui_lang::parse(
        &format!(
            "root=Box(@Each([{}],i,@Read($text,i)))",
            (0..32).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        ),
        &library,
        None,
    );
    let mut store = Store::new();
    store
        .set("$text", Some(Value::Str("x".repeat(100_000))))
        .unwrap();
    let before = test_alloc::allocated_bytes();
    let r = evaluate_tree(
        TreeInput::new(&p)
            .with_library(&library)
            .with_functions(&functions),
        &store,
    );
    let allocated = test_alloc::allocated_bytes() - before;
    assert!(!r.size_limited && !r.depth_limited);
    assert_eq!(calls.load(Relaxed), 32);
    assert!(
        allocated < 4_000_000,
        "unchanged text copied on misses: {allocated}"
    );
}

#[test]
fn snapshots_omit_undefined_entries_while_retaining_their_store_names() {
    let mut store = openui_runtime::Store::new();
    store.set("$n", Some(Value::Num(1.))).unwrap();
    store.set("$n", None).unwrap();
    assert!(store.snapshot().has_own("$n"));
    assert_eq!(store.snapshot_json().unwrap(), "{}");
    assert_eq!(
        store
            .snapshot_json_with_limits(Limits::DEFAULT.with_result_size(2))
            .unwrap(),
        "{}"
    );
}

#[test]
fn a_suspended_run_keeps_only_a_weak_program_identity() {
    use openui_runtime::{Store, TreeInput, resume_action, run_action};
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = openui_lang::Library::from_json_schema(&schema);
    let mut p = Arc::new(openui_lang::parse(
        "m=Mutation(\"tool\",{version:1})\nroot=Box(null)",
        &l,
        None,
    ));
    let plan=Value::from_json(&parse_json(r#"{"steps":[{"type":"run","statementId":"m","refType":"mutation"},{"type":"run","statementId":"m","refType":"mutation"}]}"#).unwrap());
    let mut store = Store::new();
    let r = run_action(
        TreeInput::from_shared_result(&p),
        &mut store,
        Some(&plan),
        "",
        None,
    );
    let continuation = r.continuation.unwrap();
    assert_eq!(Arc::strong_count(&p), 1);
    *Arc::make_mut(&mut p) =
        openui_lang::parse("m=Mutation(\"tool\",{version:2})\nroot=Box(null)", &l, None);
    let out = resume_action(
        TreeInput::from_shared_result(&p),
        &mut store,
        &continuation,
        true,
    )
    .unwrap();
    assert!(out.failure.is_none());
    assert!(out.events[0].to_json().unwrap().contains("\"version\":2"));
}

#[test]
fn typed_and_data_plans_share_input_credit_but_keep_their_expression_room_prices() {
    use openui_runtime::{Store, TreeInput, run_action_with_limits, value_to_json};
    let mut expression = Ast::Str("x".into());
    for _ in 0..300 {
        expression = Ast::BinOp {
            op: "+",
            left: Arc::new(expression),
            right: Arc::new(Ast::Str("x".into())),
        };
    }
    let mut step = openui_lang::JsObject::new();
    step.define("type", Value::Str("set".into()));
    step.define("target", Value::Str("$s".into()));
    step.define("valueAST", Value::Ast(Arc::new(expression)));
    let mut plan = openui_lang::JsObject::new();
    plan.define(
        "steps",
        Value::Arr(Arc::new(vec![Value::Obj(Arc::new(step))])),
    );
    let typed = Value::Obj(Arc::new(plan));
    let data = Value::from_json(&parse_json(&value_to_json(&typed).unwrap()).unwrap());
    assert_eq!(supplied_room(&typed), supplied_room(&data));
    assert_eq!(
        supplied_room(&typed),
        value_to_json(&typed).unwrap().len() + 4
    );
    let p = openui_lang::ParseResult::default();
    for (index, plan) in [&typed, &data].into_iter().enumerate() {
        let mut store = Store::new();
        let limits = Limits::DEFAULT
            .with_result_floor(0)
            .with_amplification(1)
            .with_work(100_000_000);
        let r =
            run_action_with_limits(TreeInput::new(&p), &mut store, Some(plan), "", None, limits);
        assert_eq!(r.size_limited, index == 1);
        if index == 0 {
            assert!(matches!(store.get("$s"),Some(Value::Str(ref s)) if s.len()==301));
        }
        let (mut lo, mut hi) = (0, 100_000);
        while hi - lo > 1 {
            let room = (lo + hi) / 2;
            let r = run_action_with_limits(
                TreeInput::new(&p),
                &mut Store::new(),
                Some(plan),
                "",
                None,
                limits.with_result_size(room),
            );
            if r.size_limited { lo = room } else { hi = room }
        }
        assert_eq!(hi, if index == 0 { 604 } else { 45_753 });
        assert!(
            !run_action_with_limits(
                TreeInput::new(&p),
                &mut Store::new(),
                Some(plan),
                "",
                None,
                limits.with_result_size(hi)
            )
            .size_limited
        );
        assert!(
            run_action_with_limits(
                TreeInput::new(&p),
                &mut Store::new(),
                Some(plan),
                "",
                None,
                limits.with_result_size(hi - 1)
            )
            .size_limited
        );
    }
}

#[path = "support/reuse.rs"]
#[allow(dead_code)]
mod reuse;
#[test]
fn retaining_an_earlier_result_does_not_change_a_programs_work() {
    use openui_runtime::{TreeInput, evaluate_tree, evaluate_tree_with_limits};
    // A host may keep another owner of completed callback/result data. The
    // repeated public admission must spend the same work in either case.
    let repeated_admission = |retain| {
        let value = EvaluationSession::new(&(), 0)
            .evaluate(&Ast::Arr(Arc::new(vec![Ast::Num(1.)])))
            .value
            .unwrap();
        let held = if retain { Some(value.clone()) } else { None };
        let session = EvaluationSession::with_limits(
            &(),
            0,
            Limits::DEFAULT.with_work(1000).with_result_size(1000),
        );
        let remaining = session.with_evaluator(|e| {
            assert!(e.reserve_value(&value));
            assert!(e.reserve_value(&value));
            let mut work = 0;
            while e.charge_work(1) {
                work += 1;
            }
            work
        });
        std::hint::black_box(held);
        remaining
    };
    assert_eq!(repeated_admission(false), repeated_admission(true));
    let j = reuse::schema();
    let l = openui_lang::Library::from_json_schema(&j);
    let f = reuse::functions(&j);
    let (s, refs) = reuse::supplied(1000);
    for seed in 0..64 {
        let p = openui_lang::parse(&reuse::source(seed), &l, None);
        for data in [false, true] {
            let p = if data {
                reuse::shaped(p.clone())
            } else {
                p.clone()
            };
            let input = TreeInput::new(&p)
                .with_library(&l)
                .with_functions(&f)
                .with_reference_values(&refs);
            let edge = |retain| {
                let previous = evaluate_tree(input.clone(), &s);
                let previous = if retain { Some(previous) } else { None };
                let (mut lo, mut hi) = (0, 1_000_000);
                while hi - lo > 1 {
                    let m = (lo + hi) / 2;
                    let r = evaluate_tree_with_limits(
                        input.clone(),
                        &s,
                        Limits::DEFAULT.with_work(m).with_result_size(128_000_000),
                    );
                    if r.size_limited { lo = m } else { hi = m }
                }
                std::hint::black_box(previous);
                hi
            };
            assert_eq!(edge(false), edge(true), "seed={seed},data={data}");
        }
    }
}

#[test]
fn an_oversized_unused_message_does_not_refuse_a_step_plan() {
    let p = openui_lang::ParseResult::default();
    let plan = Value::from_json(
        &parse_json(r#"{"steps":[{"type":"set","target":"$n","valueAST":{"k":"Num","v":7}}]}"#)
            .unwrap(),
    );
    let mut store = openui_runtime::Store::new();
    let message = "m".repeat(9 * 1024 * 1024);
    let before = test_alloc::allocated_bytes();
    let out = openui_runtime::run_action(
        openui_runtime::TreeInput::new(&p),
        &mut store,
        Some(&plan),
        &message,
        None,
    );
    assert!(out.failure.is_none());
    assert_eq!(store.get("$n"), Some(Value::Num(7.)));
    assert!(test_alloc::allocated_bytes() - before < 100_000);
    let refused = openui_runtime::run_action(
        openui_runtime::TreeInput::new(&p),
        &mut store,
        None,
        &message,
        None,
    );
    assert!(refused.size_limited);
}

#[test]
fn a_two_hundred_thousand_node_deferred_ast_and_its_plain_json_pass_with_default_limits() {
    let node = Ast::Arr(Arc::new(vec![Ast::Num(1.); 200_000]));
    let plain = Value::from_json(&openui_lang::parse_json(&node.to_json()).unwrap());
    for value in [Value::Ast(Arc::new(node)), plain] {
        let scope = Scope(value);
        let result = EvaluationSession::new(&scope, 0).evaluate(&Ast::StateRef("$ast".into()));
        assert!(!result.size_limited && !result.depth_limited);
        assert_eq!(result.value, Some(scope.0.clone()));
    }
}

fn supplied_room(action: &Value) -> usize {
    let program = openui_lang::ParseResult::default();
    let limits = Limits::DEFAULT
        .with_result_floor(0)
        .with_amplification(1)
        .with_result_ceiling(usize::MAX)
        .with_work(usize::MAX);
    let (mut lo, mut hi) = (0, 16 * 1024 * 1024);
    while hi - lo > 1 {
        let m = (hi + lo) / 2;
        let s = EvaluationSession::with_limits(&(), 0, limits);
        let fit = s.with_evaluator(|e| {
            e.supply_input(openui_lang::SuppliedInput {
                program: openui_lang::SuppliedProgram::Raw(&program),
                start: openui_lang::SuppliedStart::Raw {
                    action: Some(action),
                    message: "",
                    form: None,
                },
                references: None,
            })
            .unwrap();
            e.reserve_result(m)
        });
        if fit { lo = m } else { hi = m }
    }
    lo
}
#[test]
fn a_deep_part_does_not_erase_independent_supplied_credit_in_either_order() {
    let mut deep = Value::Null;
    for _ in 0..1000 {
        deep = Value::Arr(Arc::new(vec![deep]));
    }
    for size in [2000, 9 * 1024 * 1024] {
        let text = Value::Str("x".repeat(size));
        let first = Value::Arr(Arc::new(vec![deep.clone(), text.clone()]));
        let last = Value::Arr(Arc::new(vec![text, deep.clone()]));
        let a = supplied_room(&first);
        let b = supplied_room(&last);
        assert_eq!(a, b);
        assert!(a > 2000);
    }
}

#[test]
fn nested_element_rebuilds_keep_the_existing_room_boundary() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}}}"#).unwrap();
    let library = openui_lang::Library::from_json_schema(&schema);
    let program = openui_lang::parse(
        "root=Box(@Each([1,2,3],i,Box(Box(Box($s)))))",
        &library,
        None,
    );
    let mut store = openui_runtime::Store::new();
    store.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
    for room in [10734, 10735] {
        let out = openui_runtime::evaluate_tree_with_limits(
            openui_runtime::TreeInput::new(&program).with_library(&library),
            &store,
            Limits::DEFAULT
                .with_work(100_000_000)
                .with_result_size(room),
        );
        assert_eq!(out.size_limited, room == 10734);
        assert!(!out.depth_limited);
        assert!(out.errors.is_empty());
        if room == 10735 {
            assert_eq!(
                openui_runtime::value_to_json(out.root.as_ref().unwrap())
                    .unwrap()
                    .len(),
                3921
            );
        }
    }
}

#[path = "support/functions.rs"]
mod corpus_functions;
#[test]
fn false_projection_moves_only_the_documented_corpus_room_edges() {
    use openui_lang::JsonValue;
    use openui_runtime::{BindingProps, JsObject, Store, TreeInput, evaluate_tree_with_limits};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let load = |path: &str| parse_json(&std::fs::read_to_string(dir.join(path)).unwrap()).unwrap();
    let mut schemas = std::collections::BTreeMap::new();
    let mut cases = Vec::new();
    for manifest in ["manifest", "imported", "runtime"] {
        let m = load(&format!("{manifest}.json"));
        schemas.extend(m["schemas"].as_object().unwrap().clone());
        cases.extend(m["cases"].as_array().unwrap().clone());
    }
    let binding_sources = load("bindings.json");
    for (name, room, work, persisted) in [
        ("hand/20-operator-precedence", 515, None, false),
        ("hand/66-lexer-operators", 536, None, false),
        ("spec/evaluation/005-op-coerce-values", 547, None, false),
        ("runtime/descriptor-fields-types", 7461, None, false),
        ("runtime/each-state-kinds", 1010, None, false),
        ("runtime/query-supplied-false", 115, None, false),
        ("runtime/binding-non-string-target", 2200, None, false),
        ("runtime/action-steps", 669, None, false),
        ("hand/20-operator-precedence", 516, Some(565), true),
        (
            "hand/107-operands-that-need-parentheses",
            699,
            Some(739),
            true,
        ),
        ("runtime/binding-function-value", 245, Some(281), true),
    ] {
        let case = cases
            .iter()
            .find(|c| c["name"].as_str() == Some(name))
            .unwrap();
        let key = case["schema"].as_str().unwrap();
        let schema = load(schemas[key].as_str().unwrap());
        let library = openui_lang::Library::from_json_schema(&schema);
        let program = openui_lang::parse(
            &std::fs::read_to_string(dir.join(case["file"].as_str().unwrap())).unwrap(),
            &library,
            case.get("root").and_then(JsonValue::as_str),
        );
        let mut store = Store::new();
        let initialized = store.initialize(
            program
                .state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            None,
        );
        assert!(initialized.refused.is_empty());
        if persisted {
            let golden = load(&format!("golden/runtime/{name}.oneshot.json"));
            if let Some(saved) = golden["saved"].as_object() {
                for (key, value) in saved {
                    store.set(key, Some(Value::from_json(value))).unwrap();
                }
            }
        }
        if let Some(values) = case.get("stateValues").and_then(JsonValue::as_object) {
            for (k, v) in values {
                store.set(k, Some(Value::from_json(v))).unwrap();
            }
        }
        let mut references = JsObject::new();
        if let Some(values) = case.get("referenceValues").and_then(JsonValue::as_object) {
            for (k, v) in values {
                references.define(k, Value::from_json(v));
            }
        }
        let bindings = binding_sources
            .get(key)
            .and_then(JsonValue::as_str)
            .map_or_else(BindingProps::new, |path| {
                BindingProps::from_prompt_spec(&Value::from_json(&load(path)["library"])).unwrap()
            });
        let functions =
            corpus_functions::registry(schema.get("functions").unwrap_or(&JsonValue::Null));
        let input = TreeInput::new(&program)
            .with_library(&library)
            .with_functions(&functions)
            .with_actions(schema.get("actions").unwrap_or(&JsonValue::Null))
            .with_binding_props(&bindings)
            .with_reference_values(&references);
        for limit in [room - 1, room] {
            let out = evaluate_tree_with_limits(
                input.clone(),
                &store,
                Limits::DEFAULT
                    .with_work(1_000_000_000)
                    .with_result_size(limit),
            );
            assert_eq!(out.size_limited, limit < room, "{name}: {limit}");
            assert!(!out.depth_limited);
        }
        if let Some(work) = work {
            for limit in [work - 1, work] {
                let out = evaluate_tree_with_limits(
                    input.clone(),
                    &store,
                    Limits::DEFAULT.with_work(limit).with_result_size(64 << 20),
                );
                assert_eq!(out.size_limited, limit < work, "{name}: work {limit}");
                assert!(!out.depth_limited);
            }
        }
    }
}

#[test]
fn drawing_keeps_the_existing_saturated_setup_bound_before_unread_deep_declarations() {
    use openui_lang::Library;
    use openui_runtime::{Store, evaluate_tree};
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    let baseline = openui_lang::parse("root=Box(1)", &library, None);
    let expected = evaluate_tree((&baseline).into(), &Store::new())
        .to_json()
        .unwrap();
    for reversed in [false, true] {
        let mut program = baseline.clone();
        let big = Value::Str("\u{100000}".repeat(11_000_000));
        let mut deep = Value::Null;
        for _ in 0..2000 {
            deep = Value::Arr(Arc::new(vec![deep]));
        }
        if reversed {
            program.state_declarations.define("$big", big);
            program.state_declarations.define("$deep", deep);
        } else {
            program.state_declarations.define("$deep", deep);
            program.state_declarations.define("$big", big);
        }
        let out = evaluate_tree((&program).into(), &Store::new());
        assert!(!out.size_limited);
        assert_eq!(out.depth_limited, reversed);
        assert_eq!(out.to_json().unwrap(), expected);
    }
}
