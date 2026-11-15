//! Compare scalar JSON length with the expression's accepted result room.
use openui_runtime::{Ast, Limits, evaluate_expression_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
fn main() {
    for value in [false, true] {
        for room in 3..=6 {
            let r = evaluate_expression_with_limits(
                &Ast::Bool(value),
                &(),
                Limits::DEFAULT.with_result_size(room),
            );
            println!(
                "value={value} room={room} result={:?} size={} depth={}",
                r.value, r.size_limited, r.depth_limited
            );
        }
    }
}
