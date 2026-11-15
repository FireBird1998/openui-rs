//! A refused final literal is bounded before copying; fitting literals keep their price.
use openui_lang::{Ast, EvaluationSession, JsObject, Limits, Value};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[test]
fn refused_final_string_literals_do_not_allocate_their_bodies() {
    let ast = Ast::Str("x".repeat(1_000_000));
    let mut object = JsObject::new();
    object.define("k", Value::Str("Str".into()));
    object.define("v", Value::Str("x".repeat(1_000_000)));
    let data = Value::Obj(Arc::new(object));
    for room in [1024, 2_000_000] {
        for typed in [false, true] {
            let session = EvaluationSession::with_limits(
                &(),
                2_000_000,
                Limits::DEFAULT.with_result_size(room).with_work(100),
            );
            test_alloc::reset_peak();
            let before = test_alloc::peak();
            let result = session.with_evaluator(|e| {
                if typed {
                    e.evaluate_result(&ast)
                } else {
                    e.evaluate_value(&data)
                }
            });
            assert!(result.size_limited);
            assert!(test_alloc::peak().saturating_sub(before) < 128 * 1024);
        }
    }
}
#[test]
fn fitting_final_literals_keep_their_exact_result_room() {
    let ast = Ast::Str("x".repeat(1000));
    for room in [1001, 1002] {
        let session =
            EvaluationSession::with_limits(&(), 2_000_000, Limits::DEFAULT.with_result_size(room));
        let result = session.evaluate(&ast);
        assert_eq!(result.size_limited, room == 1001);
        if room == 1002 {
            assert_eq!(result.value, Some(Value::Str("x".repeat(1000))));
        }
    }
}
