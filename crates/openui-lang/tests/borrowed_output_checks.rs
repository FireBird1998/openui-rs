//! Sequential raw output checks retain work/depth but release result room.
use openui_lang::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, JsObject, Limits, Value,
};
use std::{cell::Cell, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);

#[test]
fn exact_room_is_released_and_a_refused_check_stays_cut() {
    let value = Value::Str("abcdefgh".into());
    let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_size(10));
    session.with_evaluator(|eval| {
        assert!(eval.check_result_value(&value));
        assert!(eval.check_result_value(&value));
    });
    assert_eq!(
        session.evaluate(&Ast::Str("abcdefgh".into())).value,
        Some(value.clone())
    );
    assert!(!session.flags().size_limited);
    let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_size(9));
    session.with_evaluator(|eval| assert!(!eval.check_result_value(&value)));
    assert!(session.flags().size_limited);
    assert!(session.evaluate(&Ast::Null).size_limited);
}

#[test]
fn scalar_checks_keep_work_and_shared_output_counts_every_position() {
    let scalar = Value::Str("x".repeat(1000));
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT.with_result_size(2000).with_work(5000),
    );
    let accepted = session.with_evaluator(|eval| {
        (0..20)
            .take_while(|_| eval.check_result_value(&scalar))
            .count()
    });
    assert!(accepted > 0 && accepted < 20);
    assert!(session.flags().size_limited);
    let mut object = JsObject::new();
    object.define("x", scalar);
    let object = Value::Obj(Arc::new(object));
    let repeated = Value::Arr(Arc::new(vec![object; 20]));
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT.with_result_size(2000).with_work(100000),
    );
    session.with_evaluator(|eval| assert!(!eval.check_result_value(&repeated)));
    assert!(session.flags().size_limited);
}

#[test]
fn deep_supplied_values_cut_on_a_small_stack() {
    std::thread::Builder::new()
        .name("deep_supplied_values_cut_on_a_small_stack::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut value = Value::Null;
            for _ in 0..10000 {
                value = Value::Arr(Arc::new(vec![value]));
            }
            let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_depth(8));
            session.with_evaluator(|eval| assert!(!eval.check_result_value(&value)));
            assert!(session.flags().depth_limited);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_captured_output_checker_charges_its_owner_across_foreign_sessions() {
    struct Foreign<'a, 'b> {
        owner: &'a ExpressionEvaluator<'b>,
        cut: Cell<bool>,
    }
    impl EvaluationScope for Foreign<'_, '_> {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.cut
                .set(!self.owner.check_result_value(&Value::Str("x".repeat(1000))));
            Some(Value::Num(7.))
        }
    }
    let owner = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_size(64));
    owner.with_evaluator(|eval| {
        let scope = Foreign {
            owner: eval,
            cut: Cell::new(false),
        };
        let foreign = EvaluationSession::new(&scope, 0);
        let result = foreign.evaluate(&Ast::StateRef("$foreign".into()));
        assert!(scope.cut.get());
        assert_eq!(result.value, Some(Value::Num(7.)));
        assert!(!foreign.flags().size_limited);
    });
    assert!(owner.flags().size_limited);
}
