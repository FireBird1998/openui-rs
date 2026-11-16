//! Count substitution costs through the public expression interface.
use openui_lang::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, JsObject, Limits, Value,
    parse_expression, parse_json, tokenize,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

fn expression(source: &str) -> Ast {
    parse_expression(&tokenize(source))
}
fn data(ast: &Ast) -> Value {
    Value::from_json(&parse_json(&ast.to_json()).unwrap())
}
struct Capture(bool);
impl EvaluationScope for Capture {
    fn mapped_call_only(&self, _: &str) -> bool {
        self.0
    }
    fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Null)
    }
    fn call_value(&self, _: &Value, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Null)
    }
}
fn mapped(body: Ast) -> Ast {
    let mut props = JsObject::new();
    props.define("value", body.clone());
    Ast::Comp {
        name: "Box".into(),
        args: Arc::new(vec![body]),
        function: false,
        mapped_props: Some(Arc::new(props)),
    }
}
fn each(template: Ast) -> Ast {
    Ast::Comp {
        name: "Each".into(),
        args: Arc::new(vec![
            Ast::Arr(Arc::new(vec![Ast::Num(7.)])),
            Ast::Ref("i".into()),
            template,
        ]),
        function: true,
        mapped_props: None,
    }
}

#[test]
fn substitution_leaves_object_keys_named_like_the_loop_variable_untouched() {
    let ast = expression("@Each([2,4],i,{i:i,other:i+1})");
    for value in [Value::Ast(Arc::new(ast.clone())), data(&ast)] {
        let result = EvaluationSession::new(&(), 1024).with_evaluator(|e| e.evaluate_value(&value));
        assert!(result.failure.is_none());
        assert!(!result.size_limited && !result.depth_limited);
        let expected =
            Value::from_json(&parse_json(r#"[{"i":2,"other":3},{"i":4,"other":5}]"#).unwrap());
        assert_eq!(result.value, Some(expected));
    }
}

#[test]
fn unchanged_typed_lists_allocate_independently_of_their_width() {
    let measure = |width| {
        let unchanged = Ast::Arr(Arc::new(vec![Ast::Str("unchanged".into()); width]));
        // Nested call argument lists, object entries and arrays all stay unchanged.
        let template = Ast::Comp {
            name: "Capture".into(),
            args: Arc::new(vec![
                Ast::Obj(Arc::new(vec![("key".into(), unchanged.clone())])),
                mapped(unchanged),
                Ast::Ref("i".into()),
            ]),
            function: false,
            mapped_props: None,
        };
        let ast = each(template);
        let session = EvaluationSession::with_limits(
            &Capture(false),
            0,
            Limits::DEFAULT
                .with_work(1_000_000)
                .with_result_size(1_000_000),
        );
        let before = test_alloc::allocated_bytes();
        let result = session.with_evaluator(|e| e.evaluate_result(&ast));
        let bytes = test_alloc::allocated_bytes() - before;
        assert!(result.failure.is_none());
        assert!(!result.size_limited && !result.depth_limited);
        assert_eq!(result.value, Some(Value::Arr(Arc::new(vec![Value::Null]))));
        bytes
    };
    measure(4); // Warm evaluator TLS outside the comparison.
    let narrow = measure(4);
    let wide = measure(2048);
    assert_eq!(
        wide, narrow,
        "unchanged typed lists copied: {narrow} -> {wide}"
    );
}

#[test]
fn assignment_bodies_escape_mapped_sharing_without_changing_allocation_or_peak() {
    let mut body = expression("\"unique leaf\"+i");
    for _ in 0..5 {
        body = mapped(body);
    }
    let ast = each(Ast::Assign {
        target: "$x".into(),
        value: Arc::new(body),
    });
    for value in [Value::Ast(Arc::new(ast.clone())), data(&ast)] {
        let measure = |enabled| {
            let scope = Capture(enabled);
            let session = EvaluationSession::with_limits(
                &scope,
                0,
                Limits::DEFAULT
                    .with_work(1_000_000)
                    .with_result_size(1_000_000),
            );
            test_alloc::reset_peak();
            let baseline = test_alloc::peak();
            let before = test_alloc::allocated_bytes();
            let result = session.with_evaluator(|e| e.evaluate_value(&value));
            let observed = (
                test_alloc::allocated_bytes() - before,
                test_alloc::peak().saturating_sub(baseline),
            );
            assert!(result.failure.is_none());
            assert!(!result.size_limited && !result.depth_limited);
            (observed, result)
        };
        drop(measure(false));
        let (separate, result) = measure(false);
        drop(result);
        let (opted_in, result) = measure(true);
        assert_eq!(opted_in, separate, "assignment retained a sharing policy");
        assert!(result.value.is_some());
    }
}

#[test]
fn positional_arguments_precede_mapped_props_at_every_work_cut() {
    // Keep every public result and flag exact at each work cut. Different
    // argument/prop shapes also expose order through allocation-call plateaus:
    // these adjacent cuts visit the same allocating children only args-first.
    // Compare in-process counts, never target-specific bytes or exact counts.
    let mut template = mapped(expression("[i,i+1,i+2,i+3]"));
    if let Ast::Comp { args, .. } = &mut template {
        *args = Arc::new(vec![expression("{i:i}")]);
    }
    let ast = each(template);
    let mut observations = String::new();
    for (form, value) in [
        ("typed", Value::Ast(Arc::new(ast.clone()))),
        ("data", data(&ast)),
    ] {
        let mut calls_at_cut = Vec::with_capacity(81);
        for work in 0..=80 {
            let scope = Capture(true);
            let session = EvaluationSession::with_limits(
                &scope,
                0,
                Limits::DEFAULT.with_work(work).with_result_size(10_000),
            );
            let before = test_alloc::allocation_calls();
            let result = session.with_evaluator(|e| e.evaluate_value(&value));
            let calls = test_alloc::allocation_calls() - before;
            calls_at_cut.push(calls);
            observations.push_str(&format!("{form}\t{work}\t{result:?}\n"));
        }
        let (a, b) = if form == "typed" { (13, 14) } else { (22, 23) };
        assert_eq!(
            calls_at_cut[a], calls_at_cut[b],
            "{form}: args-first allocation plateau at work {a}/{b}"
        );
    }
    assert_eq!(
        observations,
        include_str!("support/substitution-argument-order.tsv")
    );
}

#[test]
fn unchanged_descriptor_lists_keep_their_eager_scratch_allocation() {
    let measure = |width| {
        let ast = each(Ast::Comp {
            name: "Capture".into(),
            args: Arc::new(vec![
                Ast::Arr(Arc::new(vec![Ast::Num(1.); width])),
                Ast::Ref("i".into()),
            ]),
            function: false,
            mapped_props: None,
        });
        let value = data(&ast);
        let scope = Capture(false);
        let session = EvaluationSession::with_limits(
            &scope,
            0,
            Limits::DEFAULT
                .with_work(1_000_000)
                .with_result_size(1_000_000),
        );
        let before = test_alloc::allocated_bytes();
        let result = session.with_evaluator(|e| e.evaluate_value(&value));
        let bytes = test_alloc::allocated_bytes() - before;
        assert!(result.failure.is_none());
        assert!(!result.size_limited && !result.depth_limited);
        assert_eq!(result.value, Some(Value::Arr(Arc::new(vec![Value::Null]))));
        bytes
    };
    measure(4);
    let narrow = measure(4);
    let wide = measure(2048);
    assert!(
        wide >= narrow + 1024 * std::mem::size_of::<Value>(),
        "descriptor scratch allocation disappeared: {narrow} -> {wide}"
    );
}
