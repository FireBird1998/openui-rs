//! Failures and caller walks use only the public expression interface.
use openui_lang::{
    Ast, EvaluationFailure, EvaluationScope, EvaluationSession, ExpressionEvaluator,
    ExpressionOperation, ExpressionType, JsObject, Limits, Value, evaluate_expression, free_json,
    parse_expression, parse_json, tokenize,
};
use std::{cell::Cell, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}
fn bad() -> Value {
    let mut o = JsObject::new();
    o.define("toString", Value::Null);
    Value::Obj(Arc::new(o))
}
struct Supplied {
    after: Cell<usize>,
}
impl EvaluationScope for Supplied {
    fn state(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        match name {
            "$bad" => Some(bad()),
            "$rows" => Some(Value::Arr(Arc::new(vec![bad(), Value::Num(1.0)]))),
            "$obj" => Some(Value::Obj(Arc::new(JsObject::new()))),
            "$after" => {
                self.after.set(self.after.get() + 1);
                Some(Value::Num(9.0))
            }
            _ => None,
        }
    }
    fn reference(&self, _: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let r = eval.evaluate_result(&ast("\"\"+$bad"));
        assert_eq!(r.failure, Some(EvaluationFailure::CannotConvertToPrimitive));
        r.value
    }
    fn call(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let r = eval.evaluate_result(&ast("\"\"+$bad"));
        assert!(r.failure.is_some());
        r.value
    }
}
#[test]
fn coercion_throw_sites_report_typed_failures_without_limit_cuts() {
    let scope = Supplied {
        after: Cell::new(0),
    };
    for expr in [
        "\"\"+$bad",
        "$bad==1",
        "$bad!=true",
        "@Sort($rows)",
        "@Sort([1,2],$bad)",
        "@Sort([1,2],\"\",$bad)",
        "@Filter([1],$bad)",
        "@Filter([1],\"\",$bad)",
        "@Filter($rows,\"\",\"contains\",\"x\")",
        "@Filter([\"x\"],\"\",\"contains\",$bad)",
        "@Filter($rows,\"\",\"==\",1)",
        "@Filter($rows,\"\",\"!=\",1)",
        "$obj[$bad]",
    ] {
        let r = evaluate_expression(&ast(expr), &scope);
        assert_eq!(
            r.failure,
            Some(EvaluationFailure::CannotConvertToPrimitive),
            "{expr}"
        );
        assert!(
            r.value.is_none() && !r.size_limited && !r.depth_limited,
            "{expr}"
        );
    }
}
#[test]
fn first_failure_stops_later_operands_and_survives_callback_reentry() {
    let scope = Supplied {
        after: Cell::new(0),
    };
    let session = EvaluationSession::new(&scope, 1000);
    for expr in [
        "[\"\"+$bad,$after]",
        "r+$after",
        "r+$after",
        "@Host()+$after",
        "{a:\"\"+$bad,b:$after}",
        "(\"\"+$bad)?$after:$after",
    ] {
        let r = session.evaluate(&ast(expr));
        assert_eq!(
            r.failure,
            Some(EvaluationFailure::CannotConvertToPrimitive),
            "{expr}"
        );
        assert_eq!(scope.after.get(), 0, "{expr}");
    }
    for expr in [
        "false&&(\"\"+$bad)",
        "true||(\"\"+$bad)",
        "false?(\"\"+$bad):7",
        "7",
    ] {
        let r = session.evaluate(&ast(expr));
        assert!(r.failure.is_none(), "{expr}");
        assert!(!r.size_limited && !r.depth_limited);
    }
}
#[test]
fn descriptor_failures_are_lazy_typed_and_reset_between_evaluations() {
    let session = EvaluationSession::new(&(), 1000);
    let j = parse_json(r#"{"k":"Comp","name":"Count"}"#).unwrap();
    let value = Value::from_json(&j);
    free_json(j);
    let r = session.with_evaluator(|eval| eval.evaluate_value(&value));
    assert!(matches!(
        r.failure,
        Some(EvaluationFailure::InvalidExpression {
            field: "args",
            operation: ExpressionOperation::Length,
            actual: ExpressionType::Undefined,
            ..
        })
    ));
    assert!(r.value.is_none());
    assert_eq!(
        session.evaluate(&Ast::Num(7.0)).value,
        Some(Value::Num(7.0))
    );
}
fn walk(eval: &ExpressionEvaluator<'_>, n: usize) -> usize {
    eval.with_level(|| if n == 0 { 0 } else { 1 + walk(eval, n - 1) })
        .unwrap_or(0)
}
#[test]
fn caller_levels_share_depth_and_grow_small_stacks() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_depth(32));
            let entered = session.with_evaluator(|eval| walk(eval, 200_000));
            assert_eq!(entered, 32);
            assert!(session.flags().depth_limited && !session.flags().size_limited);
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn caller_work_and_result_reservations_survive_same_session_reentry() {
    for work in [false, true] {
        let limits = if work {
            Limits::DEFAULT.with_work(20)
        } else {
            Limits::DEFAULT.with_result_size(20)
        };
        let session = EvaluationSession::with_limits(&(), 0, limits);
        session.with_evaluator(|eval| {
            assert!(if work {
                eval.with_level(|| walk(eval, 10)).is_some() && !eval.flags().size_limited
            } else {
                eval.reserve_result(12)
            });
            session.with_evaluator(|inner| {
                assert!(!if work {
                    inner.with_level(|| walk(inner, 8)).is_some() && !inner.flags().size_limited
                } else {
                    inner.reserve_result(9)
                })
            });
            assert!(eval.flags().size_limited);
        });
        assert!(session.flags().size_limited);
    }
    let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_size(20));
    session.with_evaluator(|eval| {
        assert!(eval.reserve_result(12));
        let r = session.evaluate(&Ast::Str("x".repeat(9)));
        assert!(r.size_limited);
    });
    assert!(session.flags().size_limited);
}
#[test]
fn caller_level_unwind_restores_depth_and_context() {
    let session = EvaluationSession::with_limits(&(), 1000, Limits::DEFAULT.with_depth(8));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.with_evaluator(|eval| eval.with_level(|| panic!("caller unwind")))
    }));
    assert!(result.is_err());
    assert_eq!(
        session.evaluate(&Ast::Num(3.0)).value,
        Some(Value::Num(3.0))
    );
    assert!(!session.flags().depth_limited);
}

#[test]
fn a_failure_at_the_work_edge_preempts_later_work_and_cut_flags() {
    for source in [
        "\"\"+$bad",
        "$obj[$bad]",
        "@Sort([1,2],$bad)",
        "@Filter([1],\"\",$bad)",
    ] {
        let scope = Supplied {
            after: Cell::new(0),
        };
        let expression = ast(source);
        let work = (1..2000)
            .find(|&work| {
                let s =
                    EvaluationSession::with_limits(&scope, 1000, Limits::DEFAULT.with_work(work));
                let r = s.evaluate(&expression);
                r.failure.is_some() && !r.size_limited && !r.depth_limited
            })
            .unwrap();
        let s = EvaluationSession::with_limits(&scope, 1000, Limits::DEFAULT.with_work(work + 1));
        let r = s.evaluate(&Ast::Arr(Arc::new(vec![
            expression,
            Ast::StateRef("$after".into()),
        ])));
        assert_eq!(
            r.failure,
            Some(EvaluationFailure::CannotConvertToPrimitive),
            "{source}"
        );
        assert!(!r.size_limited && !r.depth_limited, "{source}");
        assert_eq!(scope.after.get(), 0);
    }
}

#[test]
fn descriptor_callback_evaluation_keeps_loop_bindings() {
    struct Calls;
    impl EvaluationScope for Calls {
        fn call(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let mut descriptor = JsObject::new();
            descriptor.define("k", Value::Str("Ref".into()));
            descriptor.define("n", Value::Str("i".into()));
            eval.with_level(|| eval.evaluate_value(&Value::Obj(Arc::new(descriptor))).value)
                .flatten()
        }
    }
    let r = evaluate_expression(&ast("@Each([1,2],\"i\",@Host())"), &Calls);
    assert!(r.failure.is_none() && !r.size_limited && !r.depth_limited);
    assert_eq!(
        r.value,
        Some(Value::Arr(Arc::new(vec![Value::Num(1.0), Value::Num(2.0)])))
    );
}

#[test]
fn fresh_caller_walks_carry_depth_cuts_to_the_enclosing_expression() {
    struct Nested;
    impl EvaluationScope for Nested {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            let child = EvaluationSession::new(&(), 0);
            child.with_evaluator(|eval| {
                walk(eval, 200_000);
            });
            assert!(child.flags().depth_limited);
            Some(Value::Num(1.0))
        }
    }
    let parent = EvaluationSession::with_limits(&Nested, 1000, Limits::DEFAULT.with_depth(16));
    let r = parent.with_evaluator(|eval| {
        eval.with_level(|| parent.evaluate(&ast("@Host()")))
            .unwrap()
    });
    assert!(r.depth_limited && parent.flags().depth_limited);
}

#[test]
fn failed_extra_lookup_does_not_fall_back_to_state() {
    struct Extra {
        fallback: Cell<usize>,
    }
    impl EvaluationScope for Extra {
        fn extra(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            if name == "$outer" {
                eval.evaluate_result(&ast("\"\"+$bad")).value
            } else {
                None
            }
        }
        fn state(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            if name == "$bad" {
                Some(bad())
            } else {
                self.fallback.set(self.fallback.get() + 1);
                Some(Value::Num(7.0))
            }
        }
    }
    let scope = Extra {
        fallback: Cell::new(0),
    };
    let session = EvaluationSession::new(&scope, 1000);
    for _ in 0..2 {
        let r = session.evaluate(&Ast::StateRef("$outer".into()));
        assert_eq!(r.failure, Some(EvaluationFailure::CannotConvertToPrimitive));
        assert_eq!(scope.fallback.get(), 0);
    }
}
