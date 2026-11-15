//! A real raised-depth callback chain can start an uncapped finished-value walk.
use openui_runtime::{
    Ast, EvaluationScope, ExpressionEvaluator, Limits, Value, evaluate_expression_with_limits,
};
use std::{cell::Cell, sync::Arc};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
struct Chain {
    depth: usize,
    reached: Cell<bool>,
}
impl EvaluationScope for Chain {
    fn cache_references(&self) -> bool {
        false
    }
    #[inline(never)]
    fn reference(&self, name: &str, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        // Reserve real stack space so this also crosses 1,024 segments in
        // optimized test builds. Uninitialized bytes avoid touching 2 GiB of
        // pages merely to make the frame size independent of compiler choices.
        let mut stack_space = [std::mem::MaybeUninit::<u8>::uninit(); 64 * 1024];
        std::hint::black_box(&mut stack_space);
        let level: usize = name.parse().unwrap();
        let result = if level < self.depth {
            evaluator.evaluate(&Ast::Ref((level + 1).to_string()))
        } else {
            let mut walk = Ast::Null;
            for _ in 0..100_000 {
                walk = Ast::UnaryOp {
                    op: "!",
                    operand: Arc::new(walk),
                };
            }
            let mut visited = 0;
            walk.walk(&mut |_| visited += 1);
            assert_eq!(visited, 100_001);
            assert!(!walk.to_json().is_empty());
            drop(walk);
            self.reached.set(true);
            Some(Value::Null)
        };
        std::hint::black_box(&mut stack_space);
        result
    }
}
#[test]
fn an_uncapped_walk_at_the_bottom_of_a_long_callback_chain_does_not_run_away() {
    let depth = std::env::var("CALLBACK_CHAIN_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(34_000);
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            let scope = Chain {
                depth,
                reached: Cell::new(false),
            };
            let result = evaluate_expression_with_limits(
                &Ast::Ref("0".into()),
                &scope,
                Limits::DEFAULT
                    .with_depth(depth * 4 + 10_000)
                    .with_work(500_000_000)
                    .with_result_size(8_000_000),
            );
            assert!(
                scope.reached.get(),
                "size={} depth={}",
                result.size_limited,
                result.depth_limited
            );
            assert!(!result.size_limited && !result.depth_limited);
            assert_eq!(result.value, Some(Value::Null));
        })
        .unwrap()
        .join()
        .unwrap();
}
