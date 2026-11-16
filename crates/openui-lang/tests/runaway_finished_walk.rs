//! Legitimate finished-tree walks have no absolute debug segment ceiling.
use openui_lang::Ast;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
#[test]
fn a_finished_walk_past_four_million_levels_does_not_panic() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let depth = 4_100_000;
            let mut node = Ast::Null;
            for _ in 0..depth {
                node = Ast::UnaryOp {
                    op: "!",
                    operand: Arc::new(node),
                };
            }
            let mut visited = 0;
            node.walk(&mut |_| visited += 1);
            assert_eq!(visited, depth + 1);
            let json = node.to_json();
            assert!(json.len() > depth * 30);
            drop(json);
            drop(node);
        })
        .unwrap()
        .join()
        .unwrap();
}
