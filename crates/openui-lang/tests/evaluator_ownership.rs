//! Captured evaluators keep their own allowance across foreign sessions.
use openui_lang::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, JsObject, Limits, Value,
};
use std::{cell::Cell, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
struct Foreign<'a, 'b> {
    owner: &'a ExpressionEvaluator<'b>,
    heavy: &'a Ast,
    mode: u8,
    cut: Cell<bool>,
}
impl EvaluationScope for Foreign<'_, '_> {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        let cut = match self.mode {
            0 => self.owner.evaluate_result(self.heavy).size_limited,
            1 => {
                let mut node = JsObject::new();
                node.define("k", Value::Str("Str".into()));
                node.define("v", Value::Str("x".repeat(1000)));
                self.owner
                    .evaluate_value(&Value::Obj(Arc::new(node)))
                    .size_limited
            }
            2 => !self.owner.reserve_result(1000),
            3 => !self.owner.reserve_value(&Value::Str("x".repeat(1000))),
            _ => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.owner.with_level(|| panic!("foreign unwind"));
                }));
                assert!(result.is_err());
                self.owner.evaluate_result(self.heavy).size_limited
            }
        };
        self.cut.set(cut);
        assert_eq!(self.owner.flags().size_limited, cut);
        Some(Value::Num(7.0))
    }
}
#[test]
fn captured_root_evaluator_uses_its_owner_across_foreign_accounts_and_unwind() {
    let heavy = Ast::Arr(Arc::new(vec![Ast::Num(1.0); 100]));
    let small = Limits::DEFAULT.with_work(50);
    assert!(
        EvaluationSession::with_limits(&(), 0, small)
            .evaluate(&heavy)
            .size_limited
    );
    for owner_small in [true, false] {
        for mode in 0..5 {
            for inside_level in [false, true] {
                let big = Limits::DEFAULT.with_work(1_000_000_000);
                let owner = EvaluationSession::with_limits(
                    &(),
                    10000,
                    if owner_small { small } else { big },
                );
                owner.with_evaluator(|eval| {
                    let scope = Foreign {
                        owner: eval,
                        heavy: &heavy,
                        mode,
                        cut: Cell::new(false),
                    };
                    let foreign = EvaluationSession::with_limits(
                        &scope,
                        10000,
                        if owner_small { big } else { small },
                    );
                    let run = || foreign.evaluate(&Ast::StateRef("$foreign".into()));
                    let result = if inside_level {
                        eval.with_level(run).unwrap()
                    } else {
                        run()
                    };
                    assert_eq!(
                        scope.cut.get(),
                        owner_small,
                        "mode {mode}, owner_small {owner_small}"
                    );
                    assert!(
                        !result.size_limited && !foreign.flags().size_limited,
                        "foreign charged in mode {mode}"
                    );
                    assert_eq!(result.value, Some(Value::Num(7.0)));
                });
                assert_eq!(owner.flags().size_limited, owner_small);
            }
        }
    }
}
#[test]
fn reserve_value_charges_work_and_room_without_earning_credit() {
    for limits in [
        Limits::DEFAULT.with_work(50),
        Limits::DEFAULT.with_result_size(50),
    ] {
        let owner = EvaluationSession::with_limits(&(), 1000, limits);
        owner.with_evaluator(|eval| {
            assert!(!eval.reserve_value(&Value::Str("x".repeat(1000))));
            assert!(eval.flags().size_limited);
        });
        assert!(owner.flags().size_limited);
    }
    let owner = EvaluationSession::new(&(), 0);
    owner.with_evaluator(|eval| {
        assert!(!eval.reserve_value(&Value::Str(
            "x".repeat(Limits::DEFAULT.result_size_for(0) + 1)
        )));
    });
    assert!(owner.flags().size_limited);
    let owner = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_depth(8));
    let mut deep = Value::Null;
    for _ in 0..30 {
        deep = Value::Arr(Arc::new(vec![deep]));
    }
    owner.with_evaluator(|eval| assert!(!eval.reserve_value(&deep)));
    assert!(owner.flags().depth_limited);
    let owner = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_size(20));
    owner.with_evaluator(|eval| {
        assert!(eval.reserve_value(&Value::Str("12345".into())));
        assert!(!eval.reserve_value(&Value::Str("123456789012".into())));
    });
}
