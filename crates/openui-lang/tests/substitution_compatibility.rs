//! Independent main (388ede7) observations for typed, data and mixed @Each templates.
use openui_lang::{
    Ast, EvaluationResult, EvaluationSession, Limits, Value, parse_expression, parse_json, tokenize,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn expression(source: &str) -> Ast {
    parse_expression(&tokenize(source))
}
fn data(ast: &Ast) -> Value {
    Value::from_json(&parse_json(&ast.to_json()).unwrap())
}
fn observe(
    value: &Value,
    work: usize,
    room: usize,
) -> (EvaluationResult, EvaluationSession<'static>) {
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT.with_work(work).with_result_size(room),
    );
    let result = session.with_evaluator(|e| e.evaluate_value(value));
    (result, session)
}
fn fingerprint(result: &EvaluationResult, hash: &mut u64) {
    for byte in format!("{:?}", result).bytes() {
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
}
fn cases() -> Vec<(String, Value)> {
    let mut cases = Vec::new();
    for (i, template) in [
        "i",
        "i.a",
        "i.b[0]",
        "[i.a,i.a]",
        "{a:i.a,b:i.b}",
        "i.a+3",
        "-i.a",
        "i.a>1?i.b:i",
        "@Count(i.b)",
        "@Each([i],j,j.a)",
        "@Each([i.a],i,i+1)",
        "{a:7}.a",
        "false?i:9",
        "[i,unknown]",
        "@Unknown(i)",
        "@Set($x,i.a)",
    ]
    .into_iter()
    .enumerate()
    {
        let ast = expression(&format!(
            "@Each([{{a:2,b:[3]}},{{a:4,b:[5]}}],i,{template})"
        ));
        cases.push((format!("typed-{i}"), Value::Ast(Arc::new(ast.clone()))));
        cases.push((format!("data-{i}"), data(&ast)));
        if let Ast::Comp { args, .. } = &ast {
            let mut mixed = data(&ast);
            if let Value::Obj(o) = &mut mixed
                && let Some(Value::Arr(a)) = Arc::make_mut(o).get_mut("args")
            {
                Arc::make_mut(a)[2] = Value::Ast(Arc::new(args[2].clone()));
            }
            cases.push((format!("mixed-{i}"), mixed));
        }
    }
    for (i, template) in [
        "null", "3", r#"{"k":"BinOp","op":"+","right":{"k":"Ref","n":"i"}}"#,
        r#"{"k":"Index","obj":7,"index":{"k":"Ref","n":"i"}}"#,
        r#"{"k":"Member","obj":{"k":"Obj","entries":[["a",{"k":"Num","v":7}]]},"field":"a"}"#,
        r#"{"k":"Arr","els":null}"#, r#"{"k":"Arr","els":{"map":3}}"#,
        r#"{"k":"Comp","name":"Box","args":null}"#,
        r#"{"k":"Obj","entries":["xy",["a",{"k":"Ref","n":"i"}]]}"#,
        r#"{"k":"Obj","entries":[["a"],null,["b",{"k":"Ref","n":"i"}]]}"#,
        r#"{"k":"Assign","target":"$x","value":{"k":"Ref","n":"i"}}"#,
        r#"{"k":"Comp","name":"Box","args":[{"k":"Ref","n":"i"}],"mappedProps":[{"k":"Ref","n":"i"}]}"#,
    ].into_iter().enumerate() {
        let json = format!(r#"{{"k":"Comp","name":"Each","args":[{{"k":"Arr","els":[{{"k":"Num","v":2}}]}},{{"k":"Ref","n":"i"}},{template}]}}"#);
        cases.push((format!("malformed-{i}"), Value::from_json(&parse_json(&json).unwrap())));
    }
    cases
}
#[test]
fn substitution_preserves_main_values_failures_flags_work_and_room() {
    let mut observations = String::new();
    for (name, value) in cases() {
        let (result, work_session) = observe(&value, 10_000, 10_000);
        let mut remaining_work = 0;
        while work_session.with_evaluator(|e| e.charge_work(1)) {
            remaining_work += 1;
        }
        let (_, room_session) = observe(&value, 1_000_000, 10_000);
        let mut remaining_room = 0;
        while room_session.with_evaluator(|e| e.reserve_result(1)) {
            remaining_room += 1;
        }
        let mut hash = 0xcbf29ce484222325;
        for work in 0..=256 {
            fingerprint(&observe(&value, work, 10_000).0, &mut hash);
        }
        for room in 0..=256 {
            fingerprint(&observe(&value, 10_000, room).0, &mut hash);
        }
        observations.push_str(&format!(
            "{name}\t{}\t{}\t{hash:016x}\t{result:?}\n",
            10_000 - remaining_work,
            10_000 - remaining_room
        ));
    }
    assert_eq!(observations, include_str!("support/substitution-main.tsv"));
}

#[test]
fn unchanged_assignment_templates_preserve_parser_allocation_counts() {
    let library = openui_lang::Library::from_json_schema(&serde_json::json!({
        "$defs": { "Box": { "properties": { "a": {} } } }
    }));
    let source = "$a=@Each([1,2,3,4],i,$x=7)\nroot=Box(0)";
    // Initialize thread-local evaluator state outside the measured parse.
    drop(openui_lang::parse(source, &library, None));
    let before = test_alloc::allocation_calls();
    let result = openui_lang::parse(source, &library, None);
    let calls = test_alloc::allocation_calls() - before;
    // Freeing leaf children of parser temporaries saves five calls (#234).
    assert_eq!(calls, 290);
    assert!(!result.size_limited && !result.depth_limited);
}
