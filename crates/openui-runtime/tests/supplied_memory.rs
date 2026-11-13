//! Peak heap for supplied lists, including input and live results.
//! Each shape has its own bound, documented in TESTING.md.
use openui_runtime::{
    Ast, EvaluationScope, ExpressionEvaluator, Limits, Value, evaluate_expression_with_limits,
};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);

struct Supplied(Value);
impl EvaluationScope for Supplied {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
}
fn supplied() -> Supplied {
    Supplied(Value::Arr(Arc::new(
        (0..900_000)
            .map(|_| Value::Arr(Arc::new(Vec::new())))
            .collect(),
    )))
}
#[test]
fn nine_hundred_thousand_arrays_pass_through_below_twice_the_input_heap() {
    let input = supplied();
    test_alloc::reset_peak();
    let input_bytes = test_alloc::peak();
    let result =
        evaluate_expression_with_limits(&Ast::StateRef("$x".into()), &input, Limits::DEFAULT);
    let peak = test_alloc::peak();
    eprintln!("pass: input={input_bytes}, peak={peak}");
    assert!(!result.size_limited && !result.depth_limited);
    let (Some(Value::Arr(output)), Value::Arr(original)) = (&result.value, &input.0) else {
        panic!("expected array");
    };
    assert_eq!(output.len(), 900_000);
    assert!(Arc::ptr_eq(output, original));
    assert!(peak < input_bytes * 2, "input={input_bytes}, peak={peak}");
}
#[test]
fn large_values_cut_by_room_or_work_stay_below_twice_the_input_heap() {
    let input = supplied();
    for limits in [
        Limits::DEFAULT.with_result_size(128),
        Limits::DEFAULT.with_work(128),
    ] {
        test_alloc::reset_peak();
        let input_bytes = test_alloc::peak();
        let result = evaluate_expression_with_limits(&Ast::StateRef("$x".into()), &input, limits);
        let peak = test_alloc::peak();
        eprintln!("cut: input={input_bytes}, peak={peak}");
        assert!(result.size_limited && !result.depth_limited);
        assert!(peak < input_bytes * 2, "input={input_bytes}, peak={peak}");
    }
}

#[test]
fn count_and_first_read_a_large_list_without_materializing_it() {
    let input = supplied();
    for (name, expected) in [
        ("Count", Value::Num(900_000.0)),
        ("First", Value::Arr(Arc::new(Vec::new()))),
    ] {
        test_alloc::reset_peak();
        let input_bytes = test_alloc::peak();
        let call = Ast::Comp {
            name: name.into(),
            function: false,
            mapped_props: None,
            args: Arc::new(vec![Ast::StateRef("$x".into())]),
        };
        let result = evaluate_expression_with_limits(&call, &input, Limits::DEFAULT);
        let peak = test_alloc::peak();
        assert!(!result.size_limited && !result.depth_limited);
        assert_eq!(result.value, Some(expected));
        assert!(peak < input_bytes * 2, "input={input_bytes}, peak={peak}");
    }
}

#[test]
fn object_coercion_does_not_build_a_discarded_field_copy() {
    let mut object = openui_lang::JsObject::new();
    for i in 0..100_000 {
        object.define(format!("k{i}"), Value::Arr(Arc::new(Vec::new())));
    }
    let input = Supplied(Value::Obj(Arc::new(object)));
    test_alloc::reset_peak();
    let input_bytes = test_alloc::peak();
    let expression = openui_lang::parse_expression(&openui_lang::tokenize("\"\" + $x"));
    let result = evaluate_expression_with_limits(&expression, &input, Limits::DEFAULT);
    let peak = test_alloc::peak();
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(result.value, Some(Value::Str("[object Object]".into())));
    assert!(peak < input_bytes * 2, "input={input_bytes}, peak={peak}");
}

#[test]
fn element_props_borrow_the_large_list_for_pass_through_and_cut() {
    let rows = supplied();
    let mut props = openui_lang::JsObject::new();
    props.define("rows", rows.0);
    let input = Supplied(Value::Element(Arc::new(openui_lang::Element {
        type_name: "Table".into(),
        props,
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    })));
    let expression = openui_lang::parse_expression(&openui_lang::tokenize("$x.props.rows"));
    for limits in [Limits::DEFAULT, Limits::DEFAULT.with_result_size(128)] {
        test_alloc::reset_peak();
        let input_bytes = test_alloc::peak();
        let result = evaluate_expression_with_limits(&expression, &input, limits);
        let peak = test_alloc::peak();
        assert!(!result.depth_limited);
        if result.size_limited {
            assert_eq!(limits, Limits::DEFAULT.with_result_size(128));
        } else {
            let Value::Element(el) = &input.0 else {
                panic!("element");
            };
            let (Some(Value::Arr(output)), Some(Value::Arr(original))) =
                (&result.value, el.props.get("rows"))
            else {
                panic!("arrays");
            };
            assert!(Arc::ptr_eq(output, original));
            assert_eq!(output.len(), 900_000);
        }
        assert!(peak < input_bytes * 2, "input={input_bytes}, peak={peak}");
    }
}

#[test]
fn each_identity_stays_below_four_times_the_input_heap() {
    let input = supplied();
    let expression = openui_lang::parse_expression(&openui_lang::tokenize("@Each($x,\"i\",i)"));
    test_alloc::reset_peak();
    let bytes = test_alloc::peak();
    let result = evaluate_expression_with_limits(&expression, &input, Limits::DEFAULT);
    let peak = test_alloc::peak();
    eprintln!("each: input={bytes}, peak={peak}");
    assert!(!result.size_limited && !result.depth_limited);
    let Some(Value::Arr(items)) = &result.value else {
        panic!("array");
    };
    assert_eq!(items.len(), 900_000);
    assert!(peak < bytes * 4, "input={bytes}, peak={peak}");
}

#[test]
fn numeric_sum_round_and_field_pluck_have_separate_heap_bounds() {
    let input = Supplied(Value::Arr(Arc::new(
        (0..900_000).map(|_| Value::Num(1.2)).collect(),
    )));
    for (source, tenths) in [("@Sum($x)", 11), ("@Round($x)", 25), ("$x.a", 25)] {
        let expression = openui_lang::parse_expression(&openui_lang::tokenize(source));
        test_alloc::reset_peak();
        let bytes = test_alloc::peak();
        let result = evaluate_expression_with_limits(&expression, &input, Limits::DEFAULT);
        let peak = test_alloc::peak();
        eprintln!("{source}: input={bytes}, peak={peak}");
        assert!(
            !result.size_limited && !result.depth_limited,
            "{source}: {result:?}"
        );
        if source == "@Sum($x)" {
            let Some(Value::Num(sum)) = result.value else {
                panic!("number");
            };
            assert!((sum - 1_080_000.0).abs() < 0.001);
        } else {
            let Some(Value::Arr(items)) = &result.value else {
                panic!("array");
            };
            assert_eq!(items.len(), 900_000);
            let expected = if source == "@Round($x)" {
                Value::Num(1.0)
            } else {
                Value::Null
            };
            assert!(items.iter().all(|item| *item == expected));
        }
        assert!(
            peak * 10 < bytes * tenths,
            "{source}: input={bytes}, peak={peak}"
        );
    }
}
