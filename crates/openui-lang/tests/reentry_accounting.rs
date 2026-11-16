//! Same-session re-entry charges nested evaluation to the suspended owner.
use openui_lang::{Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Limits, Value};
use std::{cell::RefCell, rc::Rc, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
thread_local! {
    static SESSION: RefCell<Option<Rc<EvaluationSession<'static>>>> = const { RefCell::new(None) };
}
struct Outer;
struct Inner;
impl EvaluationScope for Inner {
    fn state(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(Value::Num(if name == "$x" { 100. } else { 20. }))
    }
}
impl EvaluationScope for Outer {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        let session = SESSION.with(|s| s.borrow().as_ref().unwrap().clone());
        session.with_scope(&Inner, |s| {
            s.evaluate(&Ast::Arr(Arc::new(vec![
                Ast::StateRef("$x".into()),
                Ast::StateRef("$y".into()),
            ])))
            .value
        })
    }
}
#[test]
fn reentry_charges_nested_reads_and_projection_to_the_owning_session() {
    for work in 0..=40 {
        let session = Rc::new(EvaluationSession::with_limits(
            &Outer,
            0,
            Limits::DEFAULT.with_work(work).with_result_size(1000),
        ));
        SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
        let out = session.evaluate(&Ast::StateRef("$z".into()));
        SESSION.with(|s| s.borrow_mut().take());
        if work >= 29 {
            assert_eq!(
                out.value,
                Some(Value::Arr(Arc::new(vec![
                    Value::Num(100.),
                    Value::Num(20.)
                ])))
            );
            assert!(!out.size_limited && !out.depth_limited && out.failure.is_none());
            assert!(session.with_evaluator(|e| e.charge_work(work - 29)));
            assert!(!session.with_evaluator(|e| e.charge_work(1)));
        } else {
            assert!(
                out.size_limited && session.flags().size_limited,
                "work={work}"
            );
            assert!(!out.depth_limited && out.failure.is_none());
        }
    }
}

#[test]
fn reentry_releases_inner_projection_room_without_refunding_its_work() {
    for room in 0..=20 {
        let session = Rc::new(EvaluationSession::with_limits(
            &Outer,
            0,
            Limits::DEFAULT.with_work(1000).with_result_size(room),
        ));
        SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
        let out = session.evaluate(&Ast::StateRef("$z".into()));
        SESSION.with(|s| s.borrow_mut().take());
        if room < 9 {
            assert!(
                out.size_limited && session.flags().size_limited,
                "room={room}"
            );
        } else {
            assert_eq!(
                out.value,
                Some(Value::Arr(Arc::new(vec![
                    Value::Num(100.),
                    Value::Num(20.)
                ])))
            );
            assert!(!out.size_limited && !out.depth_limited && out.failure.is_none());
            // Inner projection costs nine room units and is released by
            // with_scope. The final projection retains eight room units.
            assert!(session.with_evaluator(|e| e.reserve_result(room - 8)));
            assert!(!session.with_evaluator(|e| e.reserve_result(1)));
        }
    }
}
