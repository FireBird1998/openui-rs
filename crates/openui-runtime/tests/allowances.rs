//! Public allowances behaviour.
use openui_lang::{parse_expression, tokenize};
use openui_runtime::{
    Ast, EvaluationScope, ExpressionEvaluator, Limits, Value, evaluate_expression,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
use openui_lang::{JsObject, RefType};
use openui_runtime::EvaluationSession;
use std::cell::Cell;

struct Supplied(Value);
impl EvaluationScope for Supplied {
    fn state(&self, _: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
}

fn ast(source: &str) -> Ast {
    parse_expression(&tokenize(source))
}

struct Emit {
    bytes: usize,
}

impl EvaluationScope for Emit {
    fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Str("x".repeat(self.bytes - 2)))
    }
}

fn scaled_room() -> Limits {
    Limits::DEFAULT
        .with_result_floor(0)
        .with_amplification(1)
        .with_result_ceiling(usize::MAX)
}

fn repeat_numbers(input: usize, limits: Limits, count: usize) -> bool {
    let session = EvaluationSession::with_limits(&(), input, limits);
    for _ in 0..count {
        if session.evaluate(&Ast::Num(1.0)).size_limited {
            return false;
        }
    }
    true
}

#[test]
fn a_diamond_is_resolved_once_per_name_for_the_whole_session() {
    struct Diamond(Cell<usize>);
    impl EvaluationScope for Diamond {
        fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.0.set(self.0.get() + 1);
            match name {
                "top" => eval.evaluate(&ast("left + right")),
                "left" | "right" => eval.evaluate(&Ast::Ref("bottom".into())),
                "bottom" => Some(Value::Num(3.0)),
                _ => None,
            }
        }
    }
    let scope = Diamond(Cell::new(0));
    let session = EvaluationSession::new(&scope, 100);
    for _ in 0..5 {
        assert_eq!(
            session.evaluate(&Ast::Ref("top".into())).value,
            Some(Value::Num(6.0))
        );
    }
    assert_eq!(scope.0.get(), 4);
    assert!(!session.flags().size_limited && !session.flags().depth_limited);
}

#[test]
fn many_props_share_result_room_and_report_aggregate_and_local_cuts() {
    let session = EvaluationSession::with_limits(&(), 1, Limits::DEFAULT.with_result_size(3));
    for _ in 0..3 {
        let one = session.evaluate(&Ast::Num(1.0));
        assert!(!one.size_limited);
    }
    assert!(!session.flags().size_limited);
    let cut = session.evaluate(&Ast::Num(1.0));
    assert!(cut.size_limited && session.flags().size_limited);
    assert!(session.evaluate(&Ast::Null).size_limited);
}

#[test]
fn scaled_expression_room_agrees_exactly_with_ast_json_bytes() {
    let mut mapped = JsObject::new();
    mapped.define("quoted\nkey", ast("$s = $value + \"é\\n\""));
    let nodes = vec![
        Ast::Str("é\n\"\\".into()),
        Ast::Num(f64::INFINITY),
        Ast::Num(-0.0),
        Ast::Bool(false),
        Ast::Null,
        Ast::Ph("missing".into()),
        Ast::RuntimeRef {
            n: "query".into(),
            ref_type: RefType::Query,
        },
        ast("[1, {a: true, b: false}]"),
        ast("$a ? -$b : row[2].field"),
        ast("x && y || z"),
        Ast::Comp {
            name: "Inner".into(),
            args: Arc::new(vec![]),
            function: false,
            mapped_props: None,
        },
    ];
    for node in nodes {
        let expression = Ast::Comp {
            name: "Emit".into(),
            args: Arc::new(vec![node]),
            function: true,
            mapped_props: Some(Arc::new(mapped.clone())),
        };
        let bytes = expression.to_json().len();
        let result = openui_runtime::evaluate_expression_with_limits(
            &expression,
            &Emit { bytes },
            scaled_room(),
        );
        assert!(
            !result.size_limited && !result.depth_limited,
            "{}: {bytes}",
            expression.to_json()
        );
        let result = openui_runtime::evaluate_expression_with_limits(
            &expression,
            &Emit { bytes: bytes + 1 },
            scaled_room(),
        );
        assert!(result.size_limited && !result.depth_limited);
    }
}

#[test]
fn supplied_input_raises_scaled_room_at_the_exact_edge() {
    struct InputEmit {
        output: usize,
    }
    impl EvaluationScope for InputEmit {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(Value::Str("v".repeat(2000)))
        }
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            assert_eq!(eval.evaluate(&args[0]), Some(Value::Str("v".repeat(2000))));
            Some(Value::Str("x".repeat(self.output - 2)))
        }
    }
    let expression = ast("@Emit($input)");
    let bytes = expression.to_json().len();
    for (output, cut) in [(bytes, false), (bytes + 1, true)] {
        let result = openui_runtime::evaluate_expression_with_limits(
            &expression,
            &InputEmit { output },
            scaled_room(),
        );
        assert_eq!(
            result.size_limited, cut,
            "output={output} expression={bytes}"
        );
        assert!(!result.depth_limited);
    }
}

#[test]
fn default_work_scales_with_program_input_at_a_one_byte_edge() {
    let mut one = 1;
    while !repeat_numbers(
        1,
        Limits::DEFAULT.with_work(one).with_result_size(usize::MAX),
        1,
    ) {
        one += 1;
    }
    // Choose a multiple so the 1,000-steps-per-byte rule lands exactly on it.
    let count = 1_100_000 / one + 1000;
    let count = count.div_ceil(1000) * 1000;
    let bytes = count * one / 1000;
    assert!(Limits::DEFAULT.work_for(bytes) > Limits::DEFAULT.work_for(0));
    let limits = Limits::DEFAULT.with_result_size(usize::MAX);
    assert!(repeat_numbers(bytes, limits, count));
    assert!(!repeat_numbers(bytes - 1, limits, count));
}

#[test]
fn supplied_values_raise_default_work_and_are_not_credited_again_per_prop() {
    let scope = Supplied(Value::Str("x".repeat(2000)));
    let session =
        EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_result_size(usize::MAX));
    for _ in 0..600 {
        assert!(!session.evaluate(&Ast::StateRef("$x".into())).size_limited);
    }
    // Each read costs about 2,000 bytes of work. Crediting the same state
    // again on every prop would make this loop unbounded instead of cutting.
    let mut cut = false;
    for _ in 0..1000 {
        if session.evaluate(&Ast::StateRef("$x".into())).size_limited {
            cut = true;
            break;
        }
    }
    assert!(cut && session.flags().size_limited);
}

#[test]
fn supplied_aliases_do_not_buy_result_room_twice() {
    let shared = Value::Str("x".repeat(2000));
    struct Aliases(Value);
    impl EvaluationScope for Aliases {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(self.0.clone())
        }
    }
    let scope = Aliases(Value::Arr(Arc::new(vec![shared])));
    let session = EvaluationSession::with_limits(&scope, 1, scaled_room());
    assert!(!session.evaluate(&Ast::StateRef("$a".into())).size_limited);
    assert!(session.evaluate(&Ast::StateRef("$b".into())).size_limited);
}

#[test]
fn amplified_supplied_data_is_cut_under_default_limits() {
    let scope = Supplied(Value::Str("x".repeat(10_000)));
    let source = format!("[{}]", vec!["$x"; 200].join(","));
    let result = evaluate_expression(&ast(&source), &scope);
    assert!(result.size_limited && !result.depth_limited);
}

#[test]
fn expression_input_raises_default_work_at_a_one_byte_edge() {
    struct Repeat;
    impl EvaluationScope for Repeat {
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            for _ in 0..700_000 {
                let result = eval.evaluate_result(&args[1]);
                if result.size_limited || result.depth_limited {
                    return result.value;
                }
            }
            Some(Value::Num(1.0))
        }
    }
    let whole = |padding| {
        let expression = Ast::Comp {
            name: "Repeat".into(),
            function: true,
            mapped_props: None,
            args: Arc::new(vec![Ast::Str("x".repeat(padding)), Ast::Num(1.0)]),
        };
        let result = openui_runtime::evaluate_expression_with_limits(
            &expression,
            &Repeat,
            Limits::DEFAULT.with_result_size(usize::MAX),
        );
        assert!(!result.depth_limited);
        !result.size_limited
    };
    assert!(!whole(0));
    assert!(whole(4096));
    let (mut low, mut high) = (1, 4096);
    while low < high {
        let mid = low + (high - low) / 2;
        if whole(mid) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    assert!(whole(low));
    assert!(!whole(low - 1));
}

#[test]
fn substituted_arguments_do_not_buy_input_allowance_again() {
    struct Labels;
    impl EvaluationScope for Labels {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(Value::Arr(Arc::new(
                (0..1000).map(|n| Value::Num(n as f64)).collect(),
            )))
        }
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            eval.evaluate(&args[0]);
            Some(Value::Str("x".repeat(1000)))
        }
    }
    let result = evaluate_expression(&ast("@Each($x, \"row\", @Label(row))"), &Labels);
    assert!(result.size_limited && !result.depth_limited);
}

#[test]
fn function_results_are_read_under_the_shared_work_and_depth_limits() {
    struct Rows;
    impl EvaluationScope for Rows {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(Value::Arr(Arc::new(
                (0..1000).map(|n| Value::Num(n as f64)).collect(),
            )))
        }
        fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            eval.evaluate(&args[0]);
            let mut row = JsObject::new();
            row.define("label", Value::Str("x".repeat(10_000)));
            Some(Value::Obj(Arc::new(row)))
        }
    }
    let result = evaluate_expression(&ast("@Count(@Each($x, \"row\", @Build(row)))"), &Rows);
    assert!(result.size_limited && !result.depth_limited);

    struct Deep;
    impl EvaluationScope for Deep {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some((0..1000).fold(Value::Null, |v, _| Value::Arr(Arc::new(vec![v]))))
        }
    }
    let deep = evaluate_expression(&ast("@Count(@Deep())"), &Deep);
    assert!(deep.depth_limited && !deep.size_limited);
}

#[test]
fn deferred_ast_sizing_cannot_borrow_work_from_the_ceiling() {
    let scope = Supplied(Value::Ast(Arc::new(Ast::Str("x".repeat(10_000)))));
    let session =
        EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_result_size(usize::MAX));
    for _ in 0..499_950 {
        assert!(!session.evaluate(&Ast::Num(1.0)).size_limited);
    }
    let result = session.evaluate(&Ast::StateRef("$ast".into()));
    assert!(result.size_limited && !result.depth_limited);
    assert!(session.flags().size_limited);
    let fresh = EvaluationSession::new(&scope, 0).evaluate(&Ast::StateRef("$ast".into()));
    assert!(!fresh.size_limited && !fresh.depth_limited);
}

#[test]
fn a_shared_deferred_ast_dag_is_sized_only_with_remaining_work() {
    let mut node = Arc::new(Ast::Num(1.0));
    for _ in 0..40 {
        node = Arc::new(Ast::BinOp {
            op: "+",
            left: node.clone(),
            right: node,
        });
    }
    let scope = Supplied(Value::Ast(node));
    let session =
        EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_result_size(usize::MAX));
    for _ in 0..499_950 {
        assert!(!session.evaluate(&Ast::Num(1.0)).size_limited);
    }
    let cut = session.evaluate(&Ast::StateRef("$ast".into()));
    assert!(cut.size_limited && !cut.depth_limited);
}

#[test]
fn a_large_deferred_ast_needs_work_available_before_it_earns_credit() {
    let scope = Supplied(Value::Ast(Arc::new(Ast::Str("x".repeat(1_100_000)))));
    let cut = EvaluationSession::new(&scope, 0).evaluate(&Ast::StateRef("$ast".into()));
    assert!(cut.size_limited && !cut.depth_limited);
    let whole = EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_work(4_000_000))
        .evaluate(&Ast::StateRef("$ast".into()));
    assert!(!whole.size_limited && !whole.depth_limited);
    assert_eq!(whole.value, Some(scope.0.clone()));
}
