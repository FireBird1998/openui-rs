//! Fresh scopes share work/input credit while releasing sequential output room.
use openui_lang::{Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Limits, Value};
use std::cell::Cell;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
struct State(Cell<f64>);
impl EvaluationScope for State {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Num(self.0.get()))
    }
}
#[test]
fn fresh_scope_caches_observe_writes_without_replenishing_work() {
    let state = State(Cell::new(1.));
    let budget = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(100));
    for n in 1..5 {
        state.0.set(n as f64);
        let v = budget.with_scope(&state, |s| s.evaluate(&Ast::StateRef("$n".into())));
        assert_eq!(v.value, Some(Value::Num(n as f64)));
    }
    assert!(!budget.with_evaluator(|e| e.charge_work(100)));
    assert!(budget.flags().size_limited);
    assert!(
        budget
            .with_scope(&state, |s| s.evaluate(&Ast::Num(1.)))
            .size_limited
    );
}
#[test]
fn work_units_are_exact_and_do_not_reserve_result_room() {
    let b =
        EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(5).with_result_size(0));
    assert!(b.with_scope(&(), |s| s.with_evaluator(|e| e.charge_work(5))));
    assert!(!b.with_scope(&(), |s| s.with_evaluator(|e| e.charge_work(1))));
    assert!(b.flags().size_limited);
}
#[test]
fn sequential_operations_release_all_output_room_and_keep_work() {
    let b =
        EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(100).with_result_size(4));
    for _ in 0..20 {
        assert!(b.with_scope(&(), |s| s.with_evaluator(|e| e.reserve_result(4))));
    }
    assert!(!b.with_scope(&(), |s| s.with_evaluator(|e| e.reserve_result(5))));
    assert!(b.flags().size_limited);
}
#[test]
fn supplied_input_earns_credit_but_cannot_raise_the_hard_ceiling() {
    let data = Value::Str("s".repeat(70_000));
    let b = EvaluationSession::new(&(), 0);
    assert!(b.with_scope(&(), |s| {
        s.with_evaluator(|e| e.register_input(&data) && e.check_result_value(&data))
    }));
    let b = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_result_ceiling(69_999));
    assert!(!b.with_scope(&(), |s| {
        s.with_evaluator(|e| e.register_input(&data) && e.check_result_value(&data))
    }));
    assert!(b.flags().size_limited);
}

#[test]
fn borrowed_text_is_priced_like_a_supplied_string_without_an_initial_copy() {
    let text = "x".repeat(70_000);
    let b = EvaluationSession::new(&(), 0);
    assert!(b.with_scope(&(), |s| s.with_evaluator(
        |e| e.register_input_text(&text) && e.reserve_result(text.len() + 2)
    )));
    let b = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(5));
    assert!(!b.with_scope(&(), |s| s.with_evaluator(|e| e.register_input_text(&text))));
    assert!(!b.with_scope(&(), |s| s.with_evaluator(|e| e.register_input_text(""))));
}
