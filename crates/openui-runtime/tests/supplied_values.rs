//! Public supplied values behaviour.
use openui_lang::{parse_expression, tokenize};
use openui_runtime::{
    Ast, EvaluationScope, ExpressionEvaluator, Limits, Value, evaluate_expression,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
use openui_lang::{JsObject, RefType};

struct Supplied(Value);
impl EvaluationScope for Supplied {
    fn state(&self, _: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
}

fn ast(source: &str) -> Ast {
    parse_expression(&tokenize(source))
}

#[test]
fn realistic_supplied_rows_pass_under_default_limits() {
    let rows = (0..1400)
        .map(|_| {
            let mut row = JsObject::new();
            row.define("label", Value::Str("x".repeat(40)));
            Value::Obj(Arc::new(row))
        })
        .collect();
    let scope = Supplied(Value::Arr(Arc::new(rows)));
    let result = evaluate_expression(&Ast::StateRef("$rows".into()), &scope);
    assert!(
        !result.size_limited && !result.depth_limited,
        "supplied rows were cut"
    );
    assert_eq!(result.value, Some(scope.0.clone()));
}

#[test]
fn pass_through_supplied_containers_do_not_need_a_second_tree() {
    let scope = Supplied(Value::Arr(Arc::new(
        (0..100_000)
            .map(|_| Value::Arr(Arc::new(Vec::new())))
            .collect(),
    )));
    test_alloc::reset_peak();
    let input = test_alloc::peak();
    let result = evaluate_expression(&Ast::StateRef("$x".into()), &scope);
    let peak = test_alloc::peak();
    eprintln!("input={input} peak={peak}");
    assert!(peak < input * 2, "input={input} peak={peak}");
    assert!(!result.size_limited);
}

#[test]
fn undefined_references_are_omitted_from_objects() {
    let expression = Ast::Obj(Arc::new(vec![("k".into(), Ast::Ref("missing".into()))]));
    let result = evaluate_expression(&expression, &());
    assert_eq!(result.value, Some(Value::Obj(Arc::new(JsObject::new()))));
}

#[test]
fn cut_supplied_containers_also_stay_below_twice_the_input_peak() {
    let scope = Supplied(Value::Arr(Arc::new(
        (0..100_000)
            .map(|_| Value::Arr(Arc::new(Vec::new())))
            .collect(),
    )));
    test_alloc::reset_peak();
    let input = test_alloc::peak();
    let result = openui_runtime::evaluate_expression_with_limits(
        &Ast::StateRef("$x".into()),
        &scope,
        Limits::DEFAULT.with_result_size(128),
    );
    let peak = test_alloc::peak();
    eprintln!("cut input={input} peak={peak}");
    assert!(result.size_limited && !result.depth_limited);
    assert!(peak < input * 2, "input={input} peak={peak}");
}

#[test]
fn a_large_deferred_ast_with_enough_work_passes_without_a_tree_copy() {
    let node = Arc::new(Ast::Str("x".repeat(1_100_000)));
    let scope = Supplied(Value::Ast(node.clone()));
    let result = openui_runtime::evaluate_expression_with_limits(
        &Ast::StateRef("$x".into()),
        &scope,
        Limits::DEFAULT.with_work(4_000_000),
    );
    assert!(!result.size_limited && !result.depth_limited);
    let Some(Value::Ast(output)) = &result.value else {
        panic!("expected deferred expression");
    };
    assert!(Arc::ptr_eq(output, &node));
}

#[test]
fn function_round_trips_preserve_computed_loop_item_identity() {
    struct Echo;
    impl EvaluationScope for Echo {
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            eval.evaluate(&args[0])
        }
    }
    let item = || Ast::RuntimeRef {
        n: "row".into(),
        ref_type: RefType::Query,
    };
    let echo = Ast::Comp {
        name: "Echo".into(),
        function: true,
        mapped_props: None,
        args: Arc::new(vec![item()]),
    };
    let compare = Ast::BinOp {
        op: "==",
        left: Arc::new(echo),
        right: Arc::new(item()),
    };
    let expression = Ast::Comp {
        name: "Each".into(),
        function: false,
        mapped_props: None,
        args: Arc::new(vec![ast("[{a:1}]"), Ast::Str("row".into()), compare]),
    };
    let result = evaluate_expression(&expression, &Echo);
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(
        result.value,
        Some(Value::Arr(Arc::new(vec![Value::Bool(true)])))
    );
}

#[test]
fn element_props_keep_the_identity_of_supplied_children() {
    let rows = Value::Arr(Arc::new(vec![Value::Num(3.0)]));
    let mut props = JsObject::new();
    props.define("rows", rows.clone());
    let element = Value::Element(Arc::new(openui_lang::Element {
        type_name: "Table".into(),
        props,
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    }));
    struct Elements {
        element: Value,
        rows: Value,
    }
    impl EvaluationScope for Elements {
        fn state(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            if name == "$el" {
                Some(self.element.clone())
            } else {
                Some(self.rows.clone())
            }
        }
    }
    let result = evaluate_expression(&ast("$el.props.rows == $rows"), &Elements { element, rows });
    assert_eq!(result.value, Some(Value::Bool(true)));
}

#[test]
fn supplied_list_operations_match_literal_lists() {
    for (input, expressions) in [
        (
            "[-1.2,2.7,0]",
            vec![
                "@Sum($x)",
                "@Avg($x)",
                "@Min($x)",
                "@Max($x)",
                "@Round($x)",
                "@Floor($x)",
                "@Ceil($x)",
                "$x.a",
            ],
        ),
        (
            "[1,[-2,3],null]",
            vec!["@Abs($x)", "@Round($x, 2)", "@Sum($x)", "$x.a"],
        ),
        (
            "[{a:\"text\"},{a:3},null,true,1]",
            vec!["$x.a", "$x.constructor", "$x.__proto__", "@Round($x)"],
        ),
        (
            "[]",
            vec![
                "@Sum($x)",
                "@Avg($x)",
                "@Min($x)",
                "@Max($x)",
                "@Round($x)",
                "$x.a",
            ],
        ),
    ] {
        let literal = ast(input);
        let value = evaluate_expression(&literal, &()).value.unwrap();
        let scope = Supplied(value);
        for expression in expressions {
            let expected = evaluate_expression(&ast(&expression.replace("$x", input)), &());
            let actual = evaluate_expression(&ast(expression), &scope);
            assert_eq!(actual, expected, "{expression} with {input}");
        }
    }
}
