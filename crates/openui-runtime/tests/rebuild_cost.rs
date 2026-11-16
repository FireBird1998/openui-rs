//! Allocation traffic stays linear for retained rows and wide argument misses.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{Store, TreeInput, Value, evaluate_tree};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn cost(n: usize) -> f64 {
    let l = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap(),
    );
    let p = parse("root=Box(@Each($rows,x,Box(x)))", &l, None);
    let mut store = Store::new();
    store
        .set("$rows", Some(Value::Arr(Arc::new(vec![Value::Num(1.); n]))))
        .unwrap();
    (0..3)
        .map(|_| {
            let start = test_alloc::allocation_calls() as f64;
            let r = evaluate_tree(TreeInput::new(&p), &store);
            let used = test_alloc::allocation_calls() as f64 - start;
            assert!(!r.size_limited && !r.depth_limited);
            assert!(r.errors.is_empty());
            used
        })
        .fold(f64::INFINITY, f64::min)
}
#[test]
fn retained_row_results_keep_allocation_traffic_linear() {
    let small = cost(4000);
    let large = cost(16000);
    assert!(
        large < small * 8.,
        "four times the rows: {large} / {small} allocation requests"
    );
}

struct TextScope;
impl openui_lang::EvaluationScope for TextScope {
    fn state(&self, _: &str, _: &openui_lang::ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Str("unchanged".into()))
    }
}
fn argument_cost(n: usize) -> f64 {
    use openui_lang::{Ast, EvaluationSession};
    let node = Value::Ast(Arc::new(Ast::StateRef("$text".into())));
    let mut fields = openui_lang::JsObject::new();
    for i in 0..n {
        fields.define(format!("field{i}"), node.clone());
    }
    fields.define("index", Value::Ast(Arc::new(Ast::Num(0.))));
    (0..3)
        .map(|_| {
            let session = EvaluationSession::with_limits(
                &TextScope,
                0,
                openui_lang::Limits::DEFAULT
                    .with_work(100_000_000)
                    .with_result_size(16_000_000),
            );
            session.with_evaluator(|e| {
                let first = e.evaluate_fields(&fields);
                assert!(!first.size_limited && !first.depth_limited);
                drop(first);
                let start = test_alloc::allocation_calls() as f64;
                for round in 1..=4 {
                    fields.define("index", Value::Ast(Arc::new(Ast::Num(round as f64))));
                    let out = e.evaluate_fields(&fields);
                    assert!(!out.size_limited && !out.depth_limited);
                    let Some(Value::Obj(o)) = &out.value else {
                        panic!("argument object")
                    };
                    assert_eq!(o.len(), n + 1);
                    assert_eq!(o.get("field0"), Some(&Value::Str("unchanged".into())));
                    assert_eq!(o.get("index"), Some(&Value::Num(round as f64)));
                }
                test_alloc::allocation_calls() as f64 - start
            })
        })
        .fold(f64::INFINITY, f64::min)
}
#[test]
fn argument_string_reuse_on_misses_scales_with_field_count() {
    let small = argument_cost(1000);
    let large = argument_cost(6000);
    assert!(
        large < small * 12.,
        "six times the fields: {large} / {small} allocation requests"
    );
}
