//! Limits are observed through the public evaluator, including callback work,
//! hostile supplied values and freeing results on a small caller stack.

use openui_lang::{JsObject, Library, parse, parse_expression, tokenize};
use openui_runtime::{
    Ast, EvaluationScope, ExpressionEvaluator, Limits, Value, evaluate_expression,
    evaluate_expression_with_limits,
};
use std::cell::Cell;
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}

fn first_whole(expression: &Ast, work: bool) -> usize {
    let mut low = 0;
    let mut high = 100_000;
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = if work {
            Limits::DEFAULT.with_work(mid)
        } else {
            Limits::DEFAULT.with_result_size(mid)
        };
        let result = evaluate_expression_with_limits(expression, &(), limits);
        if result.size_limited {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

#[test]
fn work_and_room_have_flagged_edges_and_keep_the_part_that_fits() {
    let expression = ast("[1, 2, 3, 4, 5]");
    for work in [false, true] {
        let first = first_whole(&expression, work);
        assert!(first > 0 && first < 100_000);
        let limits = |n| {
            if work {
                Limits::DEFAULT.with_work(n)
            } else {
                Limits::DEFAULT.with_result_size(n)
            }
        };
        let whole = evaluate_expression_with_limits(&expression, &(), limits(first));
        assert!(!whole.size_limited && !whole.depth_limited);
        let cut = evaluate_expression_with_limits(&expression, &(), limits(first - 1));
        assert!(cut.size_limited && !cut.depth_limited);
        if !work {
            let Some(Value::Arr(items)) = &cut.value else {
                panic!("expected fitted items");
            };
            assert_eq!(
                &items[..4],
                &[
                    Value::Num(1.0),
                    Value::Num(2.0),
                    Value::Num(3.0),
                    Value::Num(4.0)
                ]
            );
        }
    }
    assert_eq!(
        evaluate_expression(&expression, &()),
        evaluate_expression_with_limits(&expression, &(), Limits::DEFAULT)
    );
}

struct Supplied(Value);
impl EvaluationScope for Supplied {
    fn state(&self, _: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
}

#[test]
fn deep_expressions_and_supplied_values_cut_and_drop_on_a_small_stack() {
    std::thread::Builder::new()
        .name("evaluation_depth::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut expression = Ast::Num(1.0);
            let mut value = Value::Num(1.0);
            for _ in 0..10_000 {
                expression = Ast::UnaryOp {
                    op: "-",
                    operand: Arc::new(expression),
                };
                value = Value::Arr(Arc::new(vec![value]));
            }
            let cut = evaluate_expression(&expression, &());
            assert!(cut.depth_limited);
            let scope = Supplied(value);
            let cut = evaluate_expression(&Ast::StateRef("$deep".into()), &scope);
            assert!(cut.depth_limited);
            let raised = evaluate_expression_with_limits(
                &expression,
                &(),
                Limits::DEFAULT.with_depth(12_000),
            );
            assert!(!raised.size_limited && !raised.depth_limited);
            assert_eq!(raised.value, Some(Value::Num(1.0)));
            let raised = evaluate_expression_with_limits(
                &Ast::StateRef("$deep".into()),
                &scope,
                Limits::DEFAULT
                    .with_depth(12_000)
                    .with_result_size(128 * 1024),
            );
            assert!(!raised.size_limited && !raised.depth_limited);
            drop((expression, raised, scope));
            assert!(test_alloc::peak() < 32 * 1024 * 1024);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn large_supplied_values_cut_by_work_or_result_size() {
    let scope = Supplied(Value::Str("x".repeat(100_000)));
    for limits in [
        Limits::DEFAULT.with_work(100),
        Limits::DEFAULT.with_result_size(128),
    ] {
        let cut = evaluate_expression_with_limits(&ast("$value"), &scope, limits);
        assert!(cut.size_limited && !cut.depth_limited);
    }
}

struct Reentrant;
impl EvaluationScope for Reentrant {
    fn state(&self, _: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        let parsed = parse(
            "$x = @Sum([1, 2])",
            &Library::from_json_schema(&openui_lang::parse_json("{}").unwrap()),
            None,
        );
        assert!(!parsed.size_limited);
        Some(Value::Num(2.0))
    }
    fn call(&self, call: &Ast, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        let Ast::Comp { args, .. } = call else {
            return None;
        };
        let first = evaluator.evaluate(&args[0]);
        let nested =
            evaluate_expression_with_limits(&ast("[1,2]"), &(), Limits::DEFAULT.with_work(0));
        assert!(nested.size_limited);
        self.state("$any", evaluator);
        assert_eq!(first, evaluator.evaluate(&args[0]));
        first
    }
}

#[test]
fn callback_parsing_and_nested_evaluation_restore_the_enclosing_session() {
    let result = evaluate_expression(&ast("@Identity($n + 3) + $n"), &Reentrant);
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(result.value, Some(Value::Num(7.0)));
}

struct Repeated {
    completed: Cell<usize>,
}
impl EvaluationScope for Repeated {
    fn call(&self, call: &Ast, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        let Ast::Comp { args, .. } = call else {
            return None;
        };
        let mut result = None;
        for _ in 0..100 {
            let next = evaluator.evaluate_result(&args[0]);
            result = next.value;
            if next.size_limited || next.depth_limited {
                break;
            }
            self.completed.set(self.completed.get() + 1);
        }
        result
    }
}

#[test]
fn callback_argument_evaluations_share_one_work_limit() {
    let expression = ast("@Repeat(@Sum([1,2,3]))");
    let scope = Repeated {
        completed: Cell::new(0),
    };
    let cut = evaluate_expression_with_limits(&expression, &scope, Limits::DEFAULT.with_work(100));
    assert!(cut.size_limited && !cut.depth_limited);
    let completed = scope.completed.get();
    assert!(completed > 0 && completed < 100);
    let whole = evaluate_expression(&expression, &scope);
    assert!(!whole.size_limited && !whole.depth_limited);
    assert_eq!(whole.value, Some(Value::Num(6.0)));
    assert_eq!(scope.completed.get(), completed + 100);
}

#[test]
fn runtime_values_keep_identity_across_reentrant_calls() {
    struct Identity(Value);
    impl EvaluationScope for Identity {
        fn reference(&self, _: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(self.0.clone())
        }
        fn call(&self, call: &Ast, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            evaluator.evaluate(&args[0])
        }
    }
    let scope = Identity(Value::Obj(Arc::new(JsObject::new())));
    let result = evaluate_expression(&ast("left == right && @Identity(left) == left"), &scope);
    assert_eq!(result.value, Some(Value::Bool(true)));
    assert!(!result.size_limited && !result.depth_limited);
}
