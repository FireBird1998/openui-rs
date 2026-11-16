//! Configuration and serialization refuse before building oversized output.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    BindingProps, LibraryFunctions, Limits, Store, TreeInput, Value, evaluate_tree,
    value_to_json_with_limits,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
#[test]
fn serializers_accept_exact_bytes_and_refuse_one_byte_less() {
    for value in [Value::Bool(false), Value::Str("x\n\"".into()), Value::Null] {
        let json = value_to_json_with_limits(&value, Limits::DEFAULT).unwrap();
        assert_eq!(
            value_to_json_with_limits(&value, Limits::DEFAULT.with_result_size(json.len()))
                .unwrap(),
            json
        );
        assert!(
            value_to_json_with_limits(&value, Limits::DEFAULT.with_result_size(json.len() - 1))
                .unwrap_err()
                .flags
                .size_limited
        );
    }
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let p = parse("root=Box(1)", &library, None);
    let result = evaluate_tree(TreeInput::new(&p), &Store::new());
    let json = result.to_json().unwrap();
    assert_eq!(
        result
            .to_json_with_limits(Limits::DEFAULT.with_result_size(json.len()))
            .unwrap(),
        json
    );
    assert!(
        result
            .to_json_with_limits(Limits::DEFAULT.with_result_size(json.len() - 1))
            .unwrap_err()
            .flags
            .size_limited
    );
}
#[test]
fn caller_modified_results_are_measured_before_copying_them() {
    let p = Default::default();
    let mut result = evaluate_tree(TreeInput::new(&p), &Store::new());
    result.root = Some(Value::Str("x".repeat(1_000_000)));
    test_alloc::reset_peak();
    let base = test_alloc::peak();
    let refusal = result
        .to_json_with_limits(Limits::DEFAULT.with_result_size(128))
        .unwrap_err();
    assert!(refusal.flags.size_limited);
    assert!(test_alloc::peak().saturating_sub(base) < 16_384);
    assert!(matches!(&result.root,Some(Value::Str(s)) if s.len()==1_000_000));
}
#[test]
fn configuration_name_caps_are_incremental_and_refusals_are_atomic() {
    for (cap, fits) in [(8, true), (7, false)] {
        let mut bindings = BindingProps::with_limits(Limits::DEFAULT.with_result_size(cap));
        assert_eq!(bindings.insert("C", "p").is_ok(), fits);
        assert_eq!(bindings.contains("C", "p"), fits);
        if fits {
            bindings.insert("C", "p").unwrap();
            assert!(bindings.insert("C", "q").is_err());
            assert!(!bindings.contains("C", "q"));
        }
    }
    let schema = parse_json(r#"{"x":{}}"#).unwrap();
    for (cap, fits) in [(4, true), (3, false)] {
        let mut functions =
            LibraryFunctions::with_limits(&schema, Limits::DEFAULT.with_result_size(cap));
        assert_eq!(functions.register("x", |_| Ok(None)).is_ok(), fits);
        if fits {
            functions.register("x", |_| Ok(None)).unwrap();
            assert!(functions.register("y", |_| Ok(None)).is_err());
        }
    }
    let mut bindings = BindingProps::with_limits(Limits::DEFAULT.with_result_size(8));
    let name = "x".repeat(1_000_000);
    let before = test_alloc::allocation_calls();
    assert!(bindings.insert(&name, "p").unwrap_err().flags.size_limited);
    assert_eq!(test_alloc::allocation_calls(), before);
}
#[test]
fn prompt_spec_extraction_has_a_default_and_explicit_limits() {
    let spec = Value::from_json(
        &parse_json(r#"{"components":{"C":{"signature":"C(p: $binding<string>)"}}}"#).unwrap(),
    );
    let b = BindingProps::from_prompt_spec(&spec).unwrap();
    assert!(b.contains("C", "p"));
    assert!(
        BindingProps::from_prompt_spec_with_limits(&spec, Limits::DEFAULT.with_result_size(1))
            .unwrap_err()
            .flags
            .size_limited
    );
    assert!(
        BindingProps::from_prompt_spec_with_limits(&spec, Limits::DEFAULT.with_depth(0))
            .unwrap_err()
            .flags
            .depth_limited
    );
}

#[test]
fn individual_prepared_inputs_refuse_before_copying() {
    use openui_runtime::{Ast, EvaluationScope, PreparedReferenceValue, evaluate_expression};
    struct Scope(PreparedReferenceValue);
    impl EvaluationScope for Scope {
        fn prepared_reference(&self, _: &str) -> Option<&PreparedReferenceValue> {
            Some(&self.0)
        }
    }
    let source = Value::Str("x".repeat(1_000_000));
    test_alloc::reset_peak();
    let baseline = test_alloc::peak();
    let token = PreparedReferenceValue::with_limits(
        &source,
        Limits::DEFAULT.with_work(16).with_result_size(16),
    );
    assert!(test_alloc::peak().saturating_sub(baseline) < 16_384);
    assert!(evaluate_expression(&Ast::Ref("q".into()), &Scope(token)).size_limited);
}

#[test]
fn preparing_a_refused_tail_bounds_metadata_by_input_and_flags_only_a_read() {
    use openui_runtime::{JsObject, PreparedReferenceValues, evaluate_tree_with_limits};
    let mut values = JsObject::new();
    for i in 0..10_000 {
        values.define(format!("r{i}"), Value::Null);
    }
    values.define("q", Value::Num(7.));
    test_alloc::reset_peak();
    let baseline = test_alloc::peak();
    let prepared = PreparedReferenceValues::with_limits(&values, Limits::DEFAULT.with_work(8));
    assert!(test_alloc::peak().saturating_sub(baseline) < 2 * 10_000 * 24 + 16_384);
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    for (body, cut) in [("0", false), ("q", true)] {
        let p = parse(
            &format!("q=Query(\"q\",{{}},0)\nroot=Box({body})"),
            &library,
            None,
        );
        let result = evaluate_tree_with_limits(
            TreeInput::new(&p).with_prepared_reference_values(&prepared),
            &Store::new(),
            Limits::DEFAULT,
        );
        assert_eq!(result.size_limited, cut);
        assert!(!result.depth_limited);
    }
}

#[test]
fn serializer_work_and_depth_have_adjacent_edges_for_values_trees_snapshots_and_events() {
    use openui_runtime::{LimitError, run_action};
    use std::sync::Arc;
    let value = (0..8).fold(Value::Null, |v, _| Value::Arr(Arc::new(vec![v])));
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let p = parse("root=Box(1)", &library, None);
    let mut store = Store::new();
    store.set("data", Some(value.clone())).unwrap();
    let tree = evaluate_tree(TreeInput::new(&p), &store);
    let events = run_action(TreeInput::new(&p), &mut store, None, "", None);
    assert!(events.failure.is_none());
    let event = &events.events[0];
    type Writer<'a> = Box<dyn Fn(Limits) -> Result<String, LimitError> + 'a>;
    let writers: [Writer<'_>; 4] = [
        Box::new(|limits| value_to_json_with_limits(&value, limits)),
        Box::new(|limits| tree.to_json_with_limits(limits)),
        Box::new(|limits| store.snapshot_json_with_limits(limits)),
        Box::new(|limits| event.to_json_with_limits(limits)),
    ];
    for writer in writers {
        let json = writer(Limits::DEFAULT).unwrap();
        assert_eq!(
            writer(Limits::DEFAULT.with_result_size(json.len())).unwrap(),
            json
        );
        assert!(
            writer(Limits::DEFAULT.with_result_size(json.len() - 1))
                .unwrap_err()
                .flags
                .size_limited
        );
        for depth in [false, true] {
            let configure = |n| {
                if depth {
                    Limits::DEFAULT.with_depth(n)
                } else {
                    Limits::DEFAULT.with_work(n)
                }
            };
            let (mut lo, mut hi) = (0, 4096);
            assert!(writer(configure(hi)).is_ok());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if writer(configure(mid)).is_ok() {
                    hi = mid
                } else {
                    lo = mid + 1
                }
            }
            assert!(lo > 0);
            assert!(writer(configure(lo)).is_ok());
            let flags = writer(configure(lo - 1)).unwrap_err().flags;
            assert!(if depth {
                flags.depth_limited
            } else {
                flags.size_limited
            });
        }
    }
}

#[test]
fn tree_export_depth_includes_its_json_wrapper() {
    use std::sync::Arc;
    let value = Value::Arr(Arc::new(vec![Value::Arr(Arc::new(vec![Value::Null]))]));
    assert!(value_to_json_with_limits(&value, Limits::DEFAULT.with_depth(1)).is_ok());
    let mut tree = evaluate_tree(TreeInput::new(&Default::default()), &Store::new());
    tree.root = Some(value);
    assert!(
        tree.to_json_with_limits(Limits::DEFAULT.with_depth(1))
            .unwrap_err()
            .flags
            .depth_limited
    );
    assert!(
        tree.to_json_with_limits(Limits::DEFAULT.with_depth(2))
            .is_ok()
    );
}

#[test]
fn prepared_tokens_preserve_an_already_paid_contribution_without_new_read_credit() {
    use openui_runtime::{Ast, EvaluationScope, EvaluationSession, PreparedReferenceValue};
    struct Scope(PreparedReferenceValue);
    impl EvaluationScope for Scope {
        fn prepared_reference(&self, _: &str) -> Option<&PreparedReferenceValue> {
            Some(&self.0)
        }
    }
    let scope = Scope(PreparedReferenceValue::with_limits(
        &Value::Num(1.),
        Limits::DEFAULT,
    ));
    let supplied = EvaluationSession::new(&scope, 1000);
    let result = supplied.evaluate(&Ast::Ref("q".into()));
    assert_eq!(result.value, Some(Value::Num(1.)));
    assert!(!result.size_limited && !result.depth_limited);
    let unpaid = EvaluationSession::new(&scope, 0);
    assert!(unpaid.evaluate(&Ast::Ref("q".into())).size_limited);
}

#[test]
fn prepared_shallow_reads_ignore_unused_depth_while_whole_reads_enforce_it() {
    use openui_runtime::{Ast, EvaluationScope, EvaluationSession, PreparedReferenceValue};
    use std::sync::Arc;
    struct Scope(PreparedReferenceValue);
    impl EvaluationScope for Scope {
        fn prepared_reference(&self, _: &str) -> Option<&PreparedReferenceValue> {
            Some(&self.0)
        }
    }
    let value = (0..10_000).fold(Value::Null, |v, _| Value::Arr(Arc::new(vec![v])));
    let scope = Scope(PreparedReferenceValue::new(&value));
    let session = EvaluationSession::with_limits(
        &scope,
        scope.0.input_bytes(),
        Limits::DEFAULT.with_depth(1),
    );
    let node = Ast::Member {
        obj: Arc::new(Ast::Ref("q".into())),
        field: "length".into(),
    };
    let result = session.evaluate(&node);
    assert!(!result.depth_limited && !result.size_limited);
    assert_eq!(result.value, Some(Value::Num(1.)));
    let whole = session.evaluate(&Ast::Ref("q".into()));
    assert!(whole.depth_limited && !whole.size_limited);
}

#[test]
fn compact_reference_preparation_bounds_the_refused_tail() {
    use openui_runtime::{JsObject, PreparedReferenceValue};
    let mut values = JsObject::new();
    for i in 0..20 {
        values.define(format!("r{i}"), Value::Null);
    }
    let limits = Limits::DEFAULT.with_work(8);
    let (prefix, receipt, tail) = PreparedReferenceValue::measure_all_with_tail(&values, limits);
    assert!(prefix.len() < values.len());
    assert!(tail.size_limited);
    assert!(receipt.input_bytes() > 0);
}
