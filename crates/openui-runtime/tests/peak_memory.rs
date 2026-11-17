//! Opt-in portable reservation bytes, independently of process allocator bytes.
use openui_lang::{
    Library, ParseResult, StreamParser, parse, parse_expression, parse_json, parse_with_limits,
    tokenize,
};
use openui_runtime::{
    Ast, EvaluationSession, JsObject, Limits, Store, TreeInput, Value, evaluate_tree_with_limits,
    evaluate_tree_with_memory, resume_action, run_action_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);

fn library() -> Library {
    Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap())
}

#[test]
fn memory_cut_recovery_never_exports_refused_syntax() {
    let library = library();
    let store = Store::new();
    let list = (0..1000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    for prop in [
        "[@Sum(L),@Sum(L),@Sum(L),@Sum(L),@Count(L)]",
        "{first:@Sum(L),second:@Sum(L),last:@Count(L)}",
        "[Box(@Sum(L)),Box({a:@Sum(L),b:@Count(L)})]",
        "@Each([1,2,3],i,Box({a:@Sum(L),b:i}))",
    ] {
        let p = parse_with_limits(
            &format!("L=[{list}]\nroot=Box({prop})"),
            &library,
            None,
            limits(),
        );
        assert!(!p.size_limited && !p.depth_limited);
        let (whole, usage) = evaluate_tree_with_memory(
            TreeInput::new(&p),
            &store,
            limits().with_peak_memory(usize::MAX),
        );
        assert!(!whole.size_limited && whole.errors.is_empty());
        let complete = whole.to_json().unwrap();
        let unset = evaluate_tree_with_limits(TreeInput::new(&p), &store, limits());
        assert_eq!(unset.to_json().unwrap(), complete);
        assert!(!unset.size_limited && !unset.depth_limited && unset.errors.is_empty());
        let peak = usage.unwrap().accounted_peak;
        println!(
            "recovery {prop}: complete={}, account={peak}",
            complete.len()
        );
        // All reservation-equivalent allowance intervals, descending. If a
        // refusal leaves p successful bytes, [p, allowance] has the same
        // successful prefix and refuses the same next reservation. Check both
        // ends, then visit the preceding interval. This covers every allowance
        // without reevaluating thousands of identical integer settings.
        let mut allowance = peak - 1;
        loop {
            let (cut, usage) = evaluate_tree_with_memory(
                TreeInput::new(&p),
                &store,
                limits().with_peak_memory(allowance),
            );
            let json = cut.to_json().unwrap();
            assert!(
                json.len() <= complete.len() + 32,
                "{prop}, allowance={allowance}: {} vs {}",
                json.len(),
                complete.len()
            );
            assert!(cut.size_limited && usage.unwrap().limited);
            let prefix = usage.unwrap().accounted_peak;
            assert!(prefix <= allowance);
            let (edge, edge_usage) = evaluate_tree_with_memory(
                TreeInput::new(&p),
                &store,
                limits().with_peak_memory(prefix),
            );
            assert_eq!(edge.to_json().unwrap(), json);
            assert!(edge.size_limited);
            assert_eq!(edge_usage, usage);
            if prefix == 0 {
                break;
            }
            allowance = prefix - 1;
        }
        for allowance in [500, 1257, 5029, peak, peak + 1, usize::MAX] {
            let (result, usage) = evaluate_tree_with_memory(
                TreeInput::new(&p),
                &store,
                limits().with_peak_memory(allowance),
            );
            assert!(result.to_json().unwrap().len() <= complete.len() + 32);
            assert_eq!(result.size_limited, allowance < peak);
            assert_eq!(usage.unwrap().limited, allowance < peak);
            if allowance >= peak {
                assert_eq!(result.to_json().unwrap(), complete);
            }
        }
    }
}

#[test]
fn each_reserves_its_result_vector_before_building_items() {
    use openui_runtime::{EvaluationScope, ExpressionEvaluator};
    use std::cell::Cell;
    struct Items {
        data: Value,
        first: Cell<Option<usize>>,
    }
    impl EvaluationScope for Items {
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            if matches!(call, Ast::Comp { name, .. } if name == "Items") {
                Some(self.data.clone())
            } else {
                if self.first.get().is_none() {
                    self.first.set(Some(eval.memory().unwrap().accounted_peak));
                }
                Some(Value::Num(1.))
            }
        }
    }
    let scope = Items {
        data: Value::Arr(Arc::new(vec![Value::Num(0.); 8192])),
        first: Cell::new(None),
    };
    let expression = ast("@Each(@Items(),i,@Tick())");
    let session = EvaluationSession::with_limits(&scope, 0, limits().with_peak_memory(usize::MAX));
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let whole = session.evaluate(&expression);
    let full_heap = test_alloc::peak().saturating_sub(start);
    assert!(!whole.size_limited);
    drop(whole);
    drop(session);
    // Derive the edge from a one-item control, so delaying the production
    // check cannot also move the test's edge. The callback return and Each
    // destination each admit one length-dependent container (ADR 0011).
    let small = Items {
        data: Value::Arr(Arc::new(vec![Value::Num(0.)])),
        first: Cell::new(None),
    };
    let control = EvaluationSession::with_limits(&small, 0, limits().with_peak_memory(usize::MAX));
    assert!(!control.evaluate(&expression).size_limited);
    let storage = |n| {
        let s = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
        assert!(s.with_evaluator(|e| e.reserve_container_storage(n)));
        s.memory().unwrap().accounted_peak
    };
    let threshold = small.first.get().unwrap() + 2 * (storage(8192) - storage(1));
    scope.first.set(None);
    let session =
        EvaluationSession::with_limits(&scope, 0, limits().with_peak_memory(threshold - 1));
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let cut = session.evaluate(&expression);
    let cut_heap = test_alloc::peak().saturating_sub(start);
    assert!(cut.size_limited && session.memory().unwrap().limited);
    println!("Each preflight: threshold={threshold}, cut={cut_heap}, full={full_heap}");
    assert!(
        cut_heap < full_heap / 32 + 16 * 1024,
        "cut={cut_heap}, full={full_heap}"
    );
}
fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}
fn limits() -> Limits {
    Limits::DEFAULT
        .with_result_size(16 * 1024 * 1024)
        .with_work(1_000_000_000)
}

#[test]
fn memory_is_opt_in_and_has_an_exact_portable_edge() {
    assert_eq!(Limits::DEFAULT.peak_memory(), None);
    assert_eq!(Limits::DEFAULT.with_peak_memory(12).peak_memory(), Some(12));
    assert_eq!(
        Limits::DEFAULT.with_peak_memory(12).without_peak_memory(),
        Limits::DEFAULT
    );
    let overflow = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
    assert!(!overflow.with_evaluator(|e| e.reserve_container_storage(usize::MAX)));
    assert!(overflow.flags().size_limited && overflow.memory().unwrap().limited);
    let overflow = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
    assert!(overflow.with_evaluator(|e| e.reserve_container_storage(usize::MAX / 128 - 1)));
    assert!(!overflow.with_evaluator(|e| e.reserve_container_storage(1)));
    assert!(overflow.flags().size_limited && overflow.memory().unwrap().limited);
    for text in [
        "1",
        "\"hello\"",
        "[[],[],[]]",
        "[{a:[]},{a:[]}]",
        "@Each([1,2,3],i,[i,i])",
        "@Sort([5,2,3,1])",
        "@Filter([1,2,3],\"\",\">\",1)",
    ] {
        let expression = ast(text);
        let session = EvaluationSession::with_limits(&(), 0, limits());
        let baseline = session.evaluate(&expression);
        assert_eq!(session.memory(), None);
        let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
        assert_eq!(session.evaluate(&expression), baseline, "{text}");
        let peak = session.memory().unwrap().accounted_peak;
        assert!(peak > 0);
        assert!(!session.memory().unwrap().limited);
        for memory in [peak, peak + 1] {
            let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(memory));
            assert_eq!(session.evaluate(&expression), baseline, "{text}, {memory}");
            assert_eq!(session.memory().unwrap().accounted_peak, peak);
        }
        let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(peak - 1));
        assert!(session.evaluate(&expression).size_limited, "{text}");
        assert!(session.memory().unwrap().limited);
        assert!(session.evaluate(&Ast::Num(7.)).size_limited);
        // A full sweep rejects a cut appearing after the first uncut result.
        let mut completed = false;
        for memory in (0..=peak + 256).step_by(37) {
            let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(memory));
            let result = session.evaluate(&expression);
            assert!(!completed || !result.size_limited, "{text}, {memory}");
            completed |= !result.size_limited;
        }
    }
}

#[test]
fn tree_rebuilds_share_the_account_and_keep_completed_bytes() {
    let library = library();
    let store = Store::new();
    for text in [
        "root=Box(@Each([1,2,3],i,Box([i,i])))",
        "root=Box(@Each([1,2,3],i,[[],{a:[]}]))",
    ] {
        let p = parse(text, &library, None);
        let main = evaluate_tree_with_limits(TreeInput::new(&p), &store, limits());
        let bytes = main.to_json().unwrap();
        let (result, memory) = evaluate_tree_with_memory(
            TreeInput::new(&p),
            &store,
            limits().with_peak_memory(usize::MAX),
        );
        assert_eq!(result.to_json().unwrap(), bytes);
        assert!(!result.size_limited && !result.depth_limited && result.errors.is_empty());
        let peak = memory.unwrap().accounted_peak;
        for allowance in [peak, peak + 1] {
            let (result, memory) = evaluate_tree_with_memory(
                TreeInput::new(&p),
                &store,
                limits().with_peak_memory(allowance),
            );
            assert_eq!(result.to_json().unwrap(), bytes);
            assert!(!result.size_limited);
            assert_eq!(memory.unwrap().accounted_peak, peak);
        }
        let (result, memory) = evaluate_tree_with_memory(
            TreeInput::new(&p),
            &store,
            limits().with_peak_memory(peak - 1),
        );
        assert!(result.size_limited && !result.depth_limited);
        assert!(memory.unwrap().limited);
        let mut complete = false;
        for allowance in (0..=peak + 1024).step_by(101) {
            let (result, _) = evaluate_tree_with_memory(
                TreeInput::new(&p),
                &store,
                limits().with_peak_memory(allowance),
            );
            assert!(!complete || !result.size_limited);
            complete |= !result.size_limited;
        }
    }
}

#[test]
fn repeated_references_cut_before_the_quadratic_result_including_unread_defaults() {
    let library = library();
    let list = (0..2652)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let store = Store::new();
    for unread in [false, true] {
        let source = if unread {
            format!("L=[{list}]\nq=Query(\"t\",{{}},@Each(L,i,L))\nroot=Box(1)")
        } else {
            format!("L=[{list}]\nroot=Box(@Each(L,i,L))")
        };
        let p = parse(&source, &library, None);
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let baseline = evaluate_tree_with_limits(TreeInput::new(&p), &store, Limits::DEFAULT);
        let big_peak = test_alloc::peak().saturating_sub(before);
        assert_eq!(baseline.size_limited, !unread);
        drop(baseline);
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let (result, memory) = evaluate_tree_with_memory(
            TreeInput::new(&p),
            &store,
            Limits::DEFAULT.with_peak_memory(2 * 1024 * 1024),
        );
        let small_peak = test_alloc::peak().saturating_sub(before);
        assert!(result.size_limited && !result.depth_limited && result.errors.is_empty());
        assert!(memory.unwrap().limited);
        // Same process/control, ample layout tolerance, no macOS byte pin.
        assert!(
            small_peak + 32 * 1024 < big_peak / 3 + 32 * 1024,
            "unread={unread}: cut {small_peak}, unset {big_peak}"
        );
        println!(
            "repeated-reference unread={unread}: unset={big_peak}, set={small_peak}, account={memory:?}"
        );
    }
}

#[test]
fn parse_time_defaults_cut_and_streams_reaccount_each_prefix() {
    let library = Arc::new(library());
    let list = (0..100)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("$L=[{list}]\n$s=@Each($L,i,$L)\nroot=Box(1)");
    let main = parse_with_limits(&source, &library, None, limits());
    assert!(!main.size_limited);
    let high = parse_with_limits(
        &source,
        &library,
        None,
        limits().with_peak_memory(usize::MAX),
    );
    assert_eq!(high.to_json(), main.to_json());
    let constrained = limits().with_peak_memory(32 * 1024);
    let cut = parse_with_limits(&source, &library, None, constrained);
    assert!(cut.size_limited && !cut.depth_limited);
    let mut stream = StreamParser::with_limits(library.clone(), None, constrained);
    let mut prefix = String::new();
    for chunk in source.as_bytes().chunks(11) {
        stream.push(std::str::from_utf8(chunk).unwrap());
        prefix.push_str(std::str::from_utf8(chunk).unwrap());
        let fresh = parse_with_limits(&prefix, &library, None, constrained);
        assert_eq!(stream.result().to_json(), fresh.to_json());
    }
    assert_eq!(stream.finish().to_json(), cut.to_json());
}

fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut out = JsObject::new();
    for (k, v) in fields {
        out.define(k, v);
    }
    Value::Obj(Arc::new(out))
}
fn set_step(text: &str) -> Value {
    object([
        ("type", Value::Str("set".into())),
        ("target", Value::Str("$n".into())),
        ("valueAST", Value::Ast(Arc::new(ast(text)))),
    ])
}
fn plan(steps: Vec<Value>) -> Value {
    object([("steps", Value::Arr(Arc::new(steps)))])
}
fn run_step() -> Value {
    object([
        ("type", Value::Str("run".into())),
        ("refType", Value::Str("mutation".into())),
        ("statementId", Value::Str("m".into())),
    ])
}

#[test]
fn action_resumes_keep_one_memory_allowance_and_cut_writes_change_no_state() {
    let p = parse("m=Mutation(\"t\",{})", &library(), None);
    let action = plan(vec![
        set_step("[1,2,3,4]"),
        run_step(),
        set_step("[5,6,7,8]"),
    ]);
    let run = |limit: usize| {
        let mut store = Store::new();
        let first = run_action_with_limits(
            TreeInput::new(&p),
            &mut store,
            Some(&action),
            "",
            None,
            limits().with_peak_memory(limit),
        );
        let first_usage = first.memory();
        let after_first = store.snapshot();
        let version = store.version();
        if let Some(continuation) = first.continuation {
            let last = resume_action(TreeInput::new(&p), &mut store, &continuation, true).unwrap();
            if last.size_limited {
                assert_eq!(store.snapshot(), after_first);
                assert_eq!(store.version(), version);
            }
            (last, first_usage, store)
        } else {
            (first, first_usage, store)
        }
    };
    let (whole, first, expected) = run(usize::MAX);
    assert!(whole.failure.is_none());
    let peak = whole.memory().unwrap().accounted_peak;
    let first = first.unwrap().accounted_peak;
    assert!(peak > first);
    let (edge, _, store) = run(peak);
    assert!(edge.failure.is_none());
    assert_eq!(store.snapshot(), expected.snapshot());
    let (cut, _, _) = run(peak - 1);
    assert!(cut.size_limited && cut.failure.is_some());
    assert!(cut.memory().unwrap().limited);
    let mut completed = false;
    for allowance in (first..=peak + 100).step_by(17) {
        let (result, _, _) = run(allowance);
        assert!(!completed || !result.size_limited);
        completed |= !result.size_limited;
    }
    let p = ParseResult::default();
    let action = plan(vec![set_step("[1,2,3]")]);
    let mut store = Store::new();
    store.set_pristine("$n", Some(Value::Num(99.))).unwrap();
    let before = store.snapshot();
    let version = store.version();
    let cut = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&action),
        "",
        None,
        limits().with_peak_memory(0),
    );
    assert!(cut.size_limited && cut.failure.is_some());
    assert_eq!(store.snapshot(), before);
    assert_eq!(store.version(), version);
    assert!(store.is_pristine("$n"));
}

#[test]
fn representative_shapes_account_for_measured_heap() {
    let library = library();
    let store = Store::new();
    for leaf in [
        "[]".to_owned(),
        "[[],[],[],[],[],[],[],[]]".to_owned(),
        "{a:[]}".to_owned(),
        format!("\"{}\"", "x".repeat(1024)),
    ] {
        let p = parse(
            &format!("root=Box(@Each([1,2,3,4,5,6,7,8],i,{leaf}))"),
            &library,
            None,
        );
        test_alloc::reset_peak();
        let start = test_alloc::peak();
        let (result, usage) = evaluate_tree_with_memory(
            TreeInput::new(&p),
            &store,
            limits().with_peak_memory(usize::MAX),
        );
        let heap = test_alloc::peak().saturating_sub(start);
        let account = usage.unwrap().accounted_peak;
        assert!(!result.size_limited && !result.depth_limited);
        assert!(heap <= account, "heap={heap}, account={account}");
        println!(
            "leaf={} account={account} heap={heap} heap/account={:.3}",
            leaf.len(),
            heap as f64 / account as f64
        );
    }
}

#[test]
fn repeated_reference_profile_covers_measured_heap() {
    let list = (0..1000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(
        &format!("L=[{list}]\nroot=Box(@Each(L,i,L))"),
        &library(),
        None,
    );
    let store = Store::new();
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let (result, memory) = evaluate_tree_with_memory(
        TreeInput::new(&p),
        &store,
        limits().with_peak_memory(usize::MAX),
    );
    let heap = test_alloc::peak().saturating_sub(start);
    let account = memory.unwrap().accounted_peak;
    assert!(!result.size_limited && !result.depth_limited);
    assert!(heap <= account, "heap={heap}, account={account}");
    println!(
        "1000-row Each: heap={heap}, account={account}, account/heap={:.2}",
        account as f64 / heap as f64
    );
}

#[test]
fn a_one_byte_short_allowance_refuses_the_large_public_copy() {
    let expression = Ast::Str("x".repeat(512 * 1024));
    let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let whole = session.evaluate(&expression);
    let whole_heap = test_alloc::peak().saturating_sub(start);
    let peak = session.memory().unwrap().accounted_peak;
    assert!(!whole.size_limited);
    drop(whole);
    let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(peak - 1));
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let cut = session.evaluate(&expression);
    let cut_heap = test_alloc::peak().saturating_sub(start);
    assert!(cut.size_limited && session.memory().unwrap().limited);
    // The internal string can exist, but the large public copy cannot.
    assert!(
        cut_heap < whole_heap * 3 / 4 + 16 * 1024,
        "{cut_heap} vs {whole_heap}"
    );
}

#[test]
fn descriptor_substitution_and_reentrant_scopes_share_memory_cuts() {
    use openui_runtime::{EvaluationScope, ExpressionEvaluator};
    let expression = ast("@Each([1,2,3],i,[i,{a:i}])");
    let descriptor = Value::from_json(&parse_json(&expression.to_json()).unwrap());
    let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
    let baseline = session.with_evaluator(|e| e.evaluate_value(&descriptor));
    assert!(!baseline.size_limited);
    let peak = session.memory().unwrap().accounted_peak;
    for limit in [peak - 1, peak, peak + 1] {
        let session = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(limit));
        let result = session.with_evaluator(|e| e.evaluate_value(&descriptor));
        assert_eq!(result.size_limited, limit < peak);
        if limit >= peak {
            assert_eq!(result, baseline);
        }
    }
    struct Reenter;
    impl EvaluationScope for Reenter {
        fn call(&self, _: &Ast, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
            evaluator.evaluate(&ast("[1,2,3,4]"))
        }
    }
    let session =
        EvaluationSession::with_limits(&Reenter, 0, limits().with_peak_memory(usize::MAX));
    let baseline = session.evaluate(&ast("@Call()"));
    assert!(!baseline.size_limited);
    let peak = session.memory().unwrap().accounted_peak;
    let session = EvaluationSession::with_limits(&Reenter, 0, limits().with_peak_memory(peak - 1));
    assert!(session.evaluate(&ast("@Call()")).size_limited);
    assert!(session.memory().unwrap().limited);
}

#[test]
fn static_element_payloads_are_admitted_before_the_root_clone() {
    let library = library();
    let source = format!("root=Box(\"{}\")", "x".repeat(256 * 1024));
    let p = parse_with_limits(&source, &library, None, limits());
    assert!(!p.size_limited);
    let store = Store::new();
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let whole = evaluate_tree_with_limits(TreeInput::new(&p), &store, limits());
    let full_peak = test_alloc::peak().saturating_sub(start);
    assert!(!whole.size_limited);
    drop(whole);
    test_alloc::reset_peak();
    let start = test_alloc::peak();
    let cut = evaluate_tree_with_limits(
        TreeInput::new(&p),
        &store,
        limits().with_peak_memory(64 * 1024),
    );
    let cut_peak = test_alloc::peak().saturating_sub(start);
    assert!(cut.size_limited && cut.root == Some(Value::Null));
    assert!(
        cut_peak < full_peak / 2 + 16 * 1024,
        "{cut_peak} vs {full_peak}"
    );
}

#[test]
fn shaped_element_props_are_admitted_before_the_fields_helper_copies_them() {
    let library = library();
    let p = parse("root=Box(0)", &library, None);
    let mut props = JsObject::new();
    props.define("a", Value::Str("x".repeat(256 * 1024)));
    // Object props borrow their map. Element props need an owned fields copy.
    let copied_props = Value::Element(Arc::new(openui_lang::Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    }));
    let variants = [
        copied_props,
        Value::Arr(Arc::new(vec![Value::Str("x".repeat(256 * 1024))])),
        Value::Ast(Arc::new(Ast::Arr(Arc::new(vec![Ast::Str(
            "x".repeat(256 * 1024),
        )])))),
    ];
    for copied_props in variants {
        let mut p = p.clone();
        let shaped = object([
            ("type", Value::Str("element".into())),
            ("typeName", Value::Str("Box".into())),
            ("props", copied_props),
            ("hasDynamicProps", Value::Bool(true)),
            ("partial", Value::Bool(false)),
        ]);
        let root = p.root.as_mut().unwrap();
        root.has_dynamic_props = true;
        root.props.define("a", shaped);
        let store = Store::new();
        test_alloc::reset_peak();
        let start = test_alloc::peak();
        let whole = evaluate_tree_with_limits(TreeInput::new(&p), &store, limits());
        let whole_peak = test_alloc::peak().saturating_sub(start);
        assert!(!whole.size_limited);
        drop(whole);
        test_alloc::reset_peak();
        let start = test_alloc::peak();
        let cut = evaluate_tree_with_limits(
            TreeInput::new(&p),
            &store,
            limits().with_peak_memory(64 * 1024),
        );
        let cut_peak = test_alloc::peak().saturating_sub(start);
        assert!(cut.size_limited && !cut.depth_limited);
        assert!(
            cut_peak < whole_peak / 2 + 16 * 1024,
            "{cut_peak} vs {whole_peak}"
        );
        // A relative-to-payload guard rejects even one helper-side string copy;
        // the full control can legitimately hold several simultaneous copies.
        assert!(
            cut_peak < 256 * 1024 / 4 + 16 * 1024,
            "copied payload: {cut_peak}"
        );
    }
}

#[test]
fn memory_settings_do_not_invalidate_setup_only_receipts() {
    use openui_lang::{PreparedProgram, SuppliedInput, SuppliedProgram, SuppliedStart};
    let program = parse("root=Box(1)", &library(), None);
    let prepared = PreparedProgram::new(&program, limits());
    let profile = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(usize::MAX));
    let receipts = profile
        .with_evaluator(|e| {
            e.supply_input(SuppliedInput {
                program: SuppliedProgram::Prepared(&prepared),
                start: SuppliedStart::Raw {
                    action: None,
                    message: "",
                    form: None,
                },
                references: None,
            })
        })
        .unwrap();
    let expression = ast("[1,2,3]");
    let expected = profile.evaluate(&expression);
    let peak = profile.memory().unwrap().accounted_peak;
    for memory in [0, peak, peak + 1] {
        let run = EvaluationSession::with_limits(&(), 0, limits().with_peak_memory(memory));
        assert!(
            run.with_evaluator(|e| e.supply_input(SuppliedInput {
                program: SuppliedProgram::Prepared(&receipts.program),
                start: SuppliedStart::Prepared(&receipts.start),
                references: None,
            }))
            .is_ok()
        );
        assert_eq!(run.memory().unwrap().accounted_peak, 0);
        let result = run.evaluate(&expression);
        assert_eq!(result.size_limited, memory < peak);
        if memory >= peak {
            assert_eq!(result, expected);
        }
    }
}
