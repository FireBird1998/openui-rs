//! Internal transform buffers respect output and live-memory limits.
use openui_lang::{
    Ast, EvaluationScope, ExpressionEvaluator, ExpressionOutput, Limits, Value,
    evaluate_expression_with_limits,
};
use std::{cell::Cell, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(2 * 1024 * 1024 * 1024);
struct Build {
    count: usize,
    visited: Cell<usize>,
}
impl EvaluationScope for Build {
    fn call_output(&self, _: &Ast, e: &ExpressionEvaluator<'_>) -> ExpressionOutput {
        e.transform_value(&Value::Ast(Arc::new(Ast::Num(123456789.))), |v| {
            v.array((0..self.count).map(|_| {
                self.visited.set(self.visited.get() + 1);
                v.property("nothing")
            }))
        })
    }
}
#[test]
fn transform_arrays_bound_result_and_buffer_bytes() {
    for room in [65_536, 1_000_000_000] {
        let s = Build {
            count: 100_000_000,
            visited: Cell::new(0),
        };
        let ast = openui_lang::parse_expression(&openui_lang::tokenize("@Build()"));
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let r = evaluate_expression_with_limits(
            &ast,
            &s,
            Limits::DEFAULT.with_work(usize::MAX).with_result_size(room),
        );
        let peak = test_alloc::peak().saturating_sub(before);
        println!(
            "room={room} visited={} peak={peak} cut={} value={:?}",
            s.visited.get(),
            r.size_limited,
            r.value.as_ref().map(|v| match v {
                Value::Arr(a) => a.len(),
                _ => 0,
            })
        );
        assert!(r.size_limited);
        assert!(
            r.value.is_none(),
            "an interrupted transform must not expose its buffer"
        );
        assert!(
            peak <= room + 16 * 1024,
            "buffer retained {peak} bytes with {room} room"
        );
        assert!(
            s.visited.get() < room / 16,
            "entries must be charged as memory, not one byte each"
        );
    }
}

#[test]
fn projection_cuts_discard_the_complete_transformed_value() {
    struct Long;
    impl EvaluationScope for Long {
        fn call_output(&self, _: &Ast, e: &ExpressionEvaluator<'_>) -> ExpressionOutput {
            let node = Value::Ast(Arc::new(Ast::Obj(Arc::new(vec![(
                "payload".into(),
                Ast::Str("x".repeat(1000)),
            )]))));
            e.transform_value(&node, |v| {
                v.object([
                    ("fitted", v.property("payload")),
                    (
                        "remaining",
                        v.array((0..1000).map(|_| v.property("payload"))),
                    ),
                ])
            })
        }
    }
    let ast = openui_lang::parse_expression(&openui_lang::tokenize("@Long()"));
    let r = evaluate_expression_with_limits(
        &ast,
        &Long,
        Limits::DEFAULT
            .with_work(1_000_000)
            .with_result_size(65_536),
    );
    assert!(r.size_limited);
    assert!(!r.depth_limited);
    assert!(
        r.value.is_none(),
        "a projection cut must not expose an oversized/partial transformed value"
    );
}

#[test]
fn nested_buffers_share_room_and_discarded_buffers_release_it() {
    struct Nested(u8);
    impl EvaluationScope for Nested {
        fn call_output(&self, _: &Ast, e: &ExpressionEvaluator<'_>) -> ExpressionOutput {
            e.transform_value(&Value::Ast(Arc::new(Ast::Null)), |v| {
                if self.0 != 0 {
                    let (outer, inner) = if self.0 == 1 { (1000, 1000) } else { (20, 100) };
                    v.array((0..outer).map(|_| v.array((0..inner).map(|_| v.property("none")))))
                } else {
                    for _ in 0..1000 {
                        drop(v.array((0..1000).map(|_| v.property("none"))));
                    }
                    v.array([v.property("none")])
                }
            })
        }
    }
    let ast = openui_lang::parse_expression(&openui_lang::tokenize("@Nested()"));
    for nested in [1u8, 2, 0] {
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let r = evaluate_expression_with_limits(
            &ast,
            &Nested(nested),
            Limits::DEFAULT
                .with_work(10_000_000)
                .with_result_size(65_536),
        );
        let peak = test_alloc::peak().saturating_sub(before);
        assert!(
            peak <= 65_536 + 16 * 1024,
            "nested/discarded buffers retained {peak} bytes"
        );
        assert_eq!(r.size_limited, nested != 0);
        if nested != 0 {
            assert!(r.value.is_none());
        } else {
            assert!(matches!(r.value.as_ref(),Some(Value::Arr(a)) if a.len()==1));
        }
    }
}

#[test]
fn earlier_transforms_do_not_change_later_ordinary_array_work() {
    use openui_lang::EvaluationSession;
    fn minimum(prelude: bool, body: bool) -> usize {
        let mut lo = 0;
        let mut hi = 100_000;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let scope = Build {
                count: 0,
                visited: Cell::new(0),
            };
            let session = EvaluationSession::with_limits(
                &scope,
                0,
                Limits::DEFAULT.with_work(mid).with_result_size(1_000_000),
            );
            if prelude {
                session.evaluate(&openui_lang::parse_expression(&openui_lang::tokenize(
                    "@Build()",
                )));
            }
            if body {
                session.evaluate(&Ast::Arr(Arc::new(vec![Ast::Num(1.); 1000])));
            }
            if session.flags().size_limited {
                lo = mid + 1
            } else {
                hi = mid
            }
        }
        lo
    }
    assert_eq!(
        minimum(true, true) - minimum(true, false),
        minimum(false, true)
    );
}
