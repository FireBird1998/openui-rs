//! Action validation and aggregation costs through the public runtime.
use openui_lang::{JsonValue, Library, ParseResult, free_json, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn schema() -> JsonValue {
    parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#).unwrap()
}
fn threshold(p: &ParseResult, l: &Library, j: &JsonValue, work: bool) -> usize {
    threshold_for(p, l, j, &Store::new(), work)
}
fn threshold_for(p: &ParseResult, l: &Library, j: &JsonValue, store: &Store, work: bool) -> usize {
    let mut low = 0;
    let mut high = 20_000_000;
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = if work {
            Limits::DEFAULT.with_work(mid).with_result_size(20_000_000)
        } else {
            Limits::DEFAULT.with_work(100_000_000).with_result_size(mid)
        };
        let mut input = TreeInput::new(p).with_library(l);
        if let Some(a) = j.get("actions") {
            input = input.with_actions(a);
        }
        let r = evaluate_tree_with_limits(input, store, limits);
        if r.size_limited {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}
#[test]
fn aggregate_plans_and_unmapped_descriptors_have_bounded_costs() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let steps = (0..100)
        .map(|i| format!("@Set($n,{i})"))
        .collect::<Vec<_>>()
        .join(",");
    let mut body = format!("Action([{steps}])");
    for level in 1..=5 {
        let p = parse(&format!("root=Box({body})"), &l, None);
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l),
            &Store::new(),
            Limits::DEFAULT.with_work(100_000_000),
        );
        assert_eq!(threshold(&p, &l, &j, false), 6013);
        let below = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l),
            &Store::new(),
            Limits::DEFAULT.with_result_size(6012),
        );
        assert!(below.size_limited);
        println!(
            "action {level}: room={} work={} json={}",
            threshold(&p, &l, &j, false),
            threshold(&p, &l, &j, true),
            r.to_json().len()
        );
        body = format!("Action([{body}])");
    }
    for (body, room, work) in [
        ("@Set($n,1)".to_string(), 179, 269),
        (
            format!(
                "@Each([{}],i,@Set($n,i))",
                (0..100)
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            7392,
            16896,
        ),
        (
            format!(
                "@Each([{}],i,Box(i))",
                (0..100)
                    .map(|i| format!("{{k:\"Comp\",name:\"Foo{i}\",args:[]}}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            30182,
            50566,
        ),
    ] {
        let p = parse(&format!("root=Box({body})"), &l, None);
        assert_eq!(threshold(&p, &l, &j, false), room);
        assert_eq!(threshold(&p, &l, &j, true), work);
        assert!(
            evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l),
                &Store::new(),
                Limits::DEFAULT.with_result_size(room - 1)
            )
            .size_limited
        );
        assert!(
            evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l),
                &Store::new(),
                Limits::DEFAULT.with_work(work - 1)
            )
            .size_limited
        );

        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l),
            &Store::new(),
            Limits::DEFAULT.with_work(100_000_000),
        );
        println!(
            "{}: room={} work={} json={}",
            &body[..body.len().min(25)],
            threshold(&p, &l, &j, false),
            threshold(&p, &l, &j, true),
            r.to_json().len()
        );
    }
    free_json(j);
}
#[test]
fn described_submit_arguments_fit_default_limits() {
    let fields=(0..30).map(|i|format!("\"field{i}\":{{\"type\":\"string\",\"description\":\"A realistic description for field {i} of the submitted form\"}}")).collect::<Vec<_>>().join(",");
    let mut j = schema();
    j["actions"]=parse_json(&format!("{{\"Submit\":{{\"params\":{{\"type\":\"object\",\"properties\":{{\"body\":{{\"type\":\"object\",\"properties\":{{{fields}}}}}}}}}}}}}" )).unwrap();
    let l = Library::from_json_schema(&j);
    let body = (0..30)
        .map(|i| format!("field{i}:\"value number {i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let p = parse(&format!("root=Box([@Submit({{{body}}})])"), &l, None);
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &Store::new(),
        Limits::DEFAULT,
    );
    println!(
        "submit: cut={} min_work={}",
        r.size_limited,
        threshold(&p, &l, &j, true)
    );
    assert!(!r.size_limited, "realistic described action fits defaults");
    free_json(j);
}

#[test]
fn nested_objects_arrays_and_enums_scale_linearly_in_the_value() {
    use openui_runtime::Value;
    use std::sync::Arc;
    let mut j = schema();
    let fields=(0..30).map(|i|format!("\"field{i}\":{{\"type\":\"string\",\"description\":\"Description for an ordinary submitted field\"}}")).collect::<Vec<_>>().join(",");
    j["actions"]=parse_json(&format!(r#"{{"Submit":{{"params":{{"type":"object","properties":{{"body":{{"type":"array","items":{{"type":"object","properties":{{"payload":{{"type":"object","properties":{{{fields}}}}},"mode":{{"enum":["open","closed"]}},"tags":{{"type":"array","items":{{"enum":["a","b"]}}}}}},"required":["payload","mode","tags"]}}}}}},"required":["body"]}}}}}}"#)).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Submit($body))", &l, None);
    let fields = (0..30)
        .map(|i| format!("\"field{i}\":\"value number {i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let row = parse_json(&format!(
        r#"{{"payload":{{{fields}}},"mode":"open","tags":["a","b"]}}"#
    ))
    .unwrap();

    let mut work = Vec::new();
    for n in [100, 200, 400] {
        let mut store = Store::new();
        store
            .set(
                "$body",
                Some(Value::Arr(Arc::new(
                    (0..n).map(|_| Value::from_json(&row)).collect(),
                ))),
            )
            .unwrap();
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p)
                .with_library(&l)
                .with_actions(&j["actions"]),
            &store,
            Limits::DEFAULT,
        );
        assert!(!r.size_limited && !r.depth_limited);
        assert!(r.errors.is_empty());
        assert!(r.to_json().contains("custom_action"));
        let w = threshold_for(&p, &l, &j, &store, true);
        println!("nested Submit rows={n}: work={w}");
        work.push(w);
    }
    assert!(
        work[2] - work[1] <= 2 * (work[1] - work[0]) + 5000,
        "fixed plain schema must scale linearly: {work:?}"
    );
    free_json(row);
    free_json(j);
}

#[test]
fn wide_enums_and_required_lists_still_cut_with_shared_flags() {
    use openui_runtime::Value;
    use std::sync::Arc;
    let mut j = schema();
    let values = (0..10_000)
        .map(|i| format!("\"enum value {i}\""))
        .collect::<Vec<_>>()
        .join(",");
    j["actions"]=parse_json(&format!(r#"{{"Submit":{{"params":{{"type":"object","properties":{{"body":{{"type":"array","items":{{"enum":[{values}]}}}}}}}}}}}}"#)).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Submit($body))", &l, None);
    let mut store = Store::new();
    store
        .set(
            "$body",
            Some(Value::Arr(Arc::new(vec![
                Value::Str(
                    "enum value 9999".into()
                );
                100
            ]))),
        )
        .unwrap();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &store,
        Limits::DEFAULT.with_work(1_000_000),
    );
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    let required = std::iter::repeat_n("\"body\"", 10_000)
        .collect::<Vec<_>>()
        .join(",");
    j["actions"]=parse_json(&format!(r#"{{"Submit":{{"params":{{"type":"object","properties":{{"body":{{}}}},"required":[{required}]}}}}}}"#)).unwrap();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &store,
        Limits::DEFAULT.with_work(100_000),
    );
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    let mut key = JsonValue::String("body".into());
    for _ in 0..20_000 {
        key = JsonValue::Array(vec![key]);
    }
    j["actions"]["Submit"]["params"]["required"] = JsonValue::Array(vec![key]);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &store,
        Limits::DEFAULT.with_depth(24),
    );
    let extra = test_alloc::peak().saturating_sub(before);
    free_json(j);
    assert!(r.depth_limited);
    assert!(!r.size_limited);
    assert!(
        extra < 512 * 1024,
        "metadata must cut before copying a deep key"
    );
}

#[test]
fn repeated_default_copies_spend_shared_work() {
    let mut j = schema();
    j["actions"] =
        parse_json(r#"{"Submit":{"params":{"type":"object","properties":{"body":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Submit())", &l, None);
    j["actions"]["Submit"]["params"]["required"] =
        JsonValue::Array(vec![JsonValue::String("body".into()); 100]);
    j["actions"]["Submit"]["params"]["properties"]["body"]["default"] =
        JsonValue::String("x".repeat(10_000));
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &Store::new(),
        Limits::DEFAULT.with_work(200_000),
    );
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    free_json(j);
}

#[test]
fn symbolic_reference_cycles_stay_opaque_and_expanded_schemas_are_bounded() {
    let mut j = schema();
    j["actions"]=parse_json(r##"{"Submit":{"params":{"type":"object","properties":{"body":{"$ref":"#/$defs/Loop"}},"$defs":{"Loop":{"$ref":"#/$defs/Loop"}}}}}"##).unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Submit(1))", &l, None);
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &Store::new(),
        Limits::DEFAULT,
    );
    assert!(!r.size_limited && !r.depth_limited);
    assert!(r.errors.is_empty());
    assert!(r.to_json().contains("custom_action"));
    let mut branch = parse_json(r#"{"type":"string"}"#).unwrap();
    for _ in 0..12 {
        let mut next = parse_json(r#"{"anyOf":[]}"#).unwrap();
        next["anyOf"] = JsonValue::Array(vec![branch.clone(), branch]);
        branch = next;
    }
    j["actions"]["Submit"]["params"]["properties"]["body"] = branch;
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p)
            .with_library(&l)
            .with_actions(&j["actions"]),
        &Store::new(),
        Limits::DEFAULT.with_work(2000),
    );
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    free_json(j);
}

#[test]
fn flattening_shared_data_stops_buffering_at_the_result_allowance() {
    use openui_lang::Value;
    use std::sync::Arc;
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(Action($plans))", &l, None);
    let step = Value::from_json(
        &parse_json(r#"{"type":"open_url","url":"https://example.test"}"#).unwrap(),
    );
    let mut store = Store::new();
    store
        .set("$plans", Some(Value::Arr(Arc::new(vec![step; 100_000]))))
        .unwrap();
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_library(&l),
        &store,
        Limits::DEFAULT
            .with_work(100_000_000)
            .with_result_size(4096),
    );
    let extra = test_alloc::peak().saturating_sub(before);
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    assert!(
        extra < 1024 * 1024,
        "flattening buffered {extra} bytes beyond retained input"
    );
}

#[test]
fn internal_object_builders_bound_keys_before_copying_them() {
    use openui_runtime::{
        Ast, EvaluationScope, ExpressionEvaluator, ExpressionOutput, Value,
        evaluate_expression_with_limits,
    };
    use std::sync::Arc;
    struct Build(&'static str);
    impl EvaluationScope for Build {
        fn call_output(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> ExpressionOutput {
            eval.transform_value(&Value::Ast(Arc::new(Ast::Null)), |v| {
                v.object([(self.0, v.property("absent"))])
            })
        }
    }
    let key = Box::leak("k".repeat(100_000).into_boxed_str());
    let ast = openui_lang::parse_expression(&openui_lang::tokenize("@Build()"));
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r =
        evaluate_expression_with_limits(&ast, &Build(key), Limits::DEFAULT.with_result_size(64));
    let extra = test_alloc::peak().saturating_sub(before);
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    assert!(
        extra < 64 * 1024,
        "object builder copied {extra} bytes before its cut"
    );
}
