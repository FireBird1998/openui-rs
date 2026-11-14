//! Repeated schema validation shares preparation but charges each value.
use openui_lang::{JsonValue, Library, Value, free_json, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, evaluate_tree_with_limits};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn schema(description: usize, fields: usize, required: usize) -> JsonValue {
    let mut props = serde_props(fields, description);
    let req = (0..required)
        .map(|i| JsonValue::String(format!("field{i}")))
        .collect();
    let mut j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"actions":{"Submit":{"params":{"type":"object","properties":{"body":{"type":"object","properties":{},"required":[]}}}}}}"#).unwrap();
    j["actions"]["Submit"]["params"]["properties"]["body"]["properties"] =
        std::mem::take(&mut props);
    j["actions"]["Submit"]["params"]["properties"]["body"]["required"] = JsonValue::Array(req);
    j
}
fn serde_props(fields: usize, description: usize) -> JsonValue {
    let mut p = parse_json("{}").unwrap();
    for i in 0..fields {
        let mut v = parse_json(r#"{"type":"string"}"#).unwrap();
        if description > 0 {
            v["description"] = JsonValue::String("d".repeat(description));
        }
        p[format!("field{i}")] = v;
    }
    p
}
fn work(j: &JsonValue, rows: usize, fields: usize) -> (usize, bool) {
    let l = Library::from_json_schema(j);
    let fields = (0..fields)
        .map(|i| {
            if i == 0 {
                format!("field{i}:\"row \"+r")
            } else {
                format!("field{i}:\"x\"")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(
        &format!("root=Box(@Each($rows,r,Box(Action([@Submit({{{fields}}})]))))"),
        &l,
        None,
    );
    let mut store = Store::new();
    store
        .set(
            "$rows",
            Some(Value::Arr(Arc::new(
                (0..rows).map(|i| Value::Num(i as f64)).collect(),
            ))),
        )
        .unwrap();
    let run = |limits| {
        evaluate_tree_with_limits(
            TreeInput::new(&p)
                .with_library(&l)
                .with_actions(&j["actions"]),
            &store,
            limits,
        )
    };
    let r = run(Limits::DEFAULT);
    let uncut = !r.size_limited && !r.depth_limited && r.errors.is_empty();
    let mut lo = 0;
    let mut hi = 30_000_000;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if run(Limits::DEFAULT.with_work(mid).with_result_size(16_000_000)).size_limited {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    (lo, uncut)
}
#[test]
fn described_actions_per_row_share_schema_work() {
    let mut marginal = Vec::new();
    for description in [0, 60, 1000] {
        let j = schema(description, 100, 0);
        let one = work(&j, 1, 2);
        let two = work(&j, 2, 2);
        let hundred = work(&j, 100, 2);
        println!(
            "description={description}: one={} marginal={} hundred={} uncut={}",
            one.0,
            two.0 - one.0,
            hundred.0,
            hundred.1
        );
        marginal.push(two.0 - one.0);
        assert!(hundred.1, "100 row actions must fit default limits");
        free_json(j);
    }
    assert!(
        marginal.windows(2).all(|w| w.first() == w.get(1)),
        "description text is session setup, never per-call work: {marginal:?}"
    );
}
#[test]
fn per_call_work_follows_present_fields_and_required_names() {
    let j = schema(1000, 100, 0);
    let mut fields = Vec::new();
    for count in [8, 16, 32, 64] {
        let per = work(&j, 2, count).0 - work(&j, 1, count).0;
        println!("fields={count}: per-call={per}");
        fields.push(per);
    }
    assert!(fields[2] - fields[1] <= 2 * (fields[1] - fields[0]) + 256);
    assert!(fields[3] - fields[2] <= 2 * (fields[2] - fields[1]) + 256);
    let baseline = work(&j, 2, 100).0 - work(&j, 1, 100).0;
    free_json(j);
    for count in [25, 50, 100] {
        let j = schema(1000, 100, count);
        let per = work(&j, 2, 100).0 - work(&j, 1, 100).0;
        println!(
            "required={count}: per-call={per}, overhead={}",
            per - baseline
        );
        assert!(
            per - baseline <= count * 24,
            "required names must not scan the property list"
        );
        free_json(j);
    }
}

#[test]
fn prepared_schema_admission_belongs_to_each_session() {
    use openui_runtime::EvaluationSession;
    let j = schema(1000, 100, 0);
    let raw = &j["actions"]["Submit"]["params"];
    let admitted = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(2_000_000));
    let prepared = admitted.with_evaluator(|e| e.prepare_validation_schema(raw).unwrap());
    let value = Value::from_json(&parse_json(r#"{"body":{"field0":"ok"}}"#).unwrap());
    for _ in 0..2 {
        let (v, errors) = admitted.with_evaluator(|e| e.check_value(&value, &prepared));
        assert!(v.is_some() && errors.is_empty());
    }
    let small = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(1000));
    let (v, _) = small.with_evaluator(|e| e.check_value(&value, &prepared));
    assert!(v.is_none());
    assert!(small.flags().size_limited);
    let shallow = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_depth(1));
    let (v, _) = shallow.with_evaluator(|e| e.check_value(&Value::Null, &prepared));
    assert!(v.is_none());
    assert!(shallow.flags().depth_limited);
    drop(prepared);
    free_json(j);
}

#[test]
fn preparing_nested_required_names_bounds_recursive_join_copies() {
    use openui_runtime::EvaluationSession;
    let mut key = JsonValue::String("x".repeat(1000));
    for _ in 0..5000 {
        key = JsonValue::Array(vec![key]);
    }
    let mut j = parse_json(r#"{"type":"object","properties":{},"required":[]}"#).unwrap();
    j["required"] = JsonValue::Array(vec![key]);
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT.with_depth(6000).with_work(1_000_000),
    );
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let refused = session.with_evaluator(|e| e.prepare_validation_schema(&j).is_none());
    let peak = test_alloc::peak().saturating_sub(before);
    free_json(j);
    assert!(refused);
    assert!(session.flags().size_limited);
    assert!(!session.flags().depth_limited);
    assert!(
        peak < 2 * 1024 * 1024,
        "required-name preparation copied {peak} bytes before cutting"
    );
}

#[test]
fn ignored_schema_keywords_are_not_revisited_per_call() {
    let plain = schema(0, 100, 0);
    let mut annotated = schema(0, 100, 0);
    annotated["actions"]["Submit"]["params"]["properties"]["body"]["enum"] = JsonValue::Array(
        (0..1000)
            .map(|i| JsonValue::String(format!("unused {i}")))
            .collect(),
    );
    let base = work(&plain, 2, 2).0 - work(&plain, 1, 2).0;
    let extra = work(&annotated, 2, 2).0 - work(&annotated, 1, 2).0;
    assert_eq!(
        base, extra,
        "object validation does not apply an enum keyword"
    );
    free_json(plain);
    free_json(annotated);
}

#[test]
fn prepared_validation_does_not_walk_absent_schema_properties() {
    use openui_runtime::EvaluationSession;
    let value = Value::from_json(&parse_json(r#"{"body":{"field0":"ok","field1":"x"}}"#).unwrap());
    let mut allocations = Vec::new();
    for width in [100, 1000] {
        let j = schema(0, width, 0);
        let session =
            EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(100_000_000));
        let prepared = session.with_evaluator(|e| {
            e.prepare_validation_schema(&j["actions"]["Submit"]["params"])
                .unwrap()
        });
        session.with_evaluator(|e| {
            e.check_value(&value, &prepared);
        });
        let before = test_alloc::allocation_calls();
        for _ in 0..100 {
            let (v, errors) = session.with_evaluator(|e| e.check_value(&value, &prepared));
            assert!(v.is_some() && errors.is_empty());
        }
        allocations.push(test_alloc::allocation_calls().saturating_sub(before));
        drop(prepared);
        free_json(j);
    }
    assert_eq!(
        allocations.first(),
        allocations.get(1),
        "per-call paths must visit present fields, not absent schema properties: {allocations:?}"
    );
}

#[test]
fn positional_parameter_metadata_is_prepared_once() {
    fn allocations(width: usize, rows: usize) -> usize {
        let mut j = schema(0, width, 0);
        let properties =
            j["actions"]["Submit"]["params"]["properties"]["body"]["properties"].take();
        j["actions"]["Submit"]["params"]["properties"] = properties;
        let l = Library::from_json_schema(&j);
        let p = parse(
            "root=Box(@Each($rows,r,@Submit(\"row \"+r,\"x\")))",
            &l,
            None,
        );
        let mut store = Store::new();
        store
            .set(
                "$rows",
                Some(Value::Arr(Arc::new(
                    (0..rows).map(|i| Value::Num(i as f64)).collect(),
                ))),
            )
            .unwrap();
        let run = || {
            evaluate_tree_with_limits(
                TreeInput::new(&p)
                    .with_library(&l)
                    .with_actions(&j["actions"]),
                &store,
                Limits::DEFAULT.with_work(100_000_000),
            )
        };
        drop(run());
        let before = test_alloc::allocation_calls();
        let r = run();
        let count = test_alloc::allocation_calls().saturating_sub(before);
        assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
        free_json(j);
        count
    }
    let narrow = allocations(100, 2) - allocations(100, 1);
    let wide = allocations(1000, 2) - allocations(1000, 1);
    assert_eq!(
        narrow, wide,
        "positional naming must not rebuild every declared property per call"
    );
}

#[test]
fn computed_record_keys_are_admitted_before_copying() {
    let text = "x".repeat(100_000);
    let session =
        openui_runtime::EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(1000));
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    assert!(session.with_evaluator(|e| e.copy_text(&text)).is_none());
    assert!(session.flags().size_limited);
    assert!(test_alloc::peak().saturating_sub(before) < 16 * 1024);
}

#[test]
fn malformed_string_properties_are_bounded_before_index_materialization() {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        r#"root=Box({k:"Comp",name:"Probe",args:[{k:"Str",v:"ok"}]})"#,
        &l,
        None,
    );
    let mut actions = parse_json(r#"{"Probe":{"params":{"properties":""}}}"#).unwrap();
    actions["Probe"]["params"]["properties"] = JsonValue::String("x".repeat(900_000));
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_library(&l).with_actions(&actions),
        &Store::new(),
        Limits::DEFAULT.with_work(1_000_000),
    );
    let peak = test_alloc::peak().saturating_sub(before);
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    assert!(
        peak < 512 * 1024,
        "index preparation allocated {peak} bytes before admission"
    );
    free_json(actions);
    free_json(j);
}

#[test]
fn untyped_required_names_keep_explicit_null_arguments() {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse(
        r#"root=Box({k:"Comp",name:"Probe",args:[{k:"Null"}]})"#,
        &l,
        None,
    );
    let actions=parse_json(r#"{"Probe":{"params":{"properties":{"x":{"type":"string","default":"fallback"}},"required":["x"]}}}"#).unwrap();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_library(&l).with_actions(&actions),
        &Store::new(),
        Limits::DEFAULT,
    );
    assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
    assert!(
        r.to_json().contains(r#""params":{"x":null}"#),
        "{}",
        r.to_json()
    );
    free_json(actions);
    free_json(j);
}
