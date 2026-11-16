//! An address alone never admits a new allocation as previously supplied text.
use openui_runtime::{EvaluationSession, Limits, Value};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
#[test]
fn freed_text_reallocated_at_the_same_address_pays_for_its_new_bytes() {
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT.with_work(20_000).with_result_size(100_000),
    );
    session.with_evaluator(|eval| {
        let _recycle = test_alloc::recycle_bytes(4096);
        let text = "x".repeat(4096);
        let address = text.as_ptr();
        let original = Value::Str(text);
        eval.read_state_value("$test", &original);
        drop(original);
        let fresh = "y".repeat(4096);
        assert_eq!(fresh.as_ptr(), address);
        // Scalar registration cannot retain its allocation. This new allocation
        // must pay 4,097 work units even when its predecessor had the same ptr.
        assert!(eval.reserve_value(&Value::Str(fresh)));
        assert!(!eval.charge_work(9000));
    });
    assert!(session.flags().size_limited);
}

#[test]
fn registered_container_owners_outlive_every_operation_in_their_session() {
    use openui_runtime::{Ast, JsObject};
    use std::sync::Arc;
    let session = EvaluationSession::new(&(), 0);
    let mut fields = JsObject::new();
    fields.define("text", Value::Str("x".repeat(4096)));
    let owner = Arc::new(fields);
    let weak = Arc::downgrade(&owner);
    let value = Value::Obj(owner);
    session.with_evaluator(|eval| {
        assert!(eval.read_state_value("$test", &value));
    });
    drop(value);
    assert!(weak.upgrade().is_some());
    assert!(!session.evaluate(&Ast::Num(0.)).size_limited);
    assert!(weak.upgrade().is_some());
    drop(session);
    assert!(weak.upgrade().is_none());
}
