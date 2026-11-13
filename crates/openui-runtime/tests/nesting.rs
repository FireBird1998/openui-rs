//! Public nesting behaviour.
use openui_lang::{parse_expression, tokenize};
use openui_runtime::{
    Ast, EvaluationScope, ExpressionEvaluator, Limits, Value, evaluate_expression,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
use openui_runtime::EvaluationSession;
use std::cell::Cell;

struct Chain;
impl EvaluationScope for Chain {
    fn reference(&self, name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        let n: usize = name.parse().ok()?;
        if n == 20_000 {
            return Some(Value::Num(1.0));
        }
        evaluate_expression(&Ast::Ref((n + 1).to_string()), self).value
    }
}

struct SharedChain {
    visits: Cell<usize>,
}

impl EvaluationScope for SharedChain {
    fn reference(&self, name: &str, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        self.visits.set(self.visits.get() + 1);
        let n: usize = name.parse().ok()?;
        if n == 20_000 {
            return Some(Value::Num(1.0));
        }
        evaluator.evaluate(&Ast::Ref((n + 1).to_string()))
    }
}

fn ast(source: &str) -> Ast {
    parse_expression(&tokenize(source))
}

#[test]
fn fresh_nested_references_cut_by_depth_without_panicking() {
    let result = std::panic::catch_unwind(|| evaluate_expression(&Ast::Ref("0".into()), &Chain));
    assert!(result.is_ok(), "fresh nested evaluation panicked");
    let result = result.unwrap();
    assert!(result.depth_limited && !result.size_limited, "{result:?}");
}

#[test]
fn references_share_work_and_depth_instead_of_starting_fresh_allowances() {
    let scope = SharedChain {
        visits: Cell::new(0),
    };
    let depth = evaluate_expression(&Ast::Ref("0".into()), &scope);
    assert!(depth.depth_limited && !depth.size_limited);
    assert!(scope.visits.get() <= Limits::DEFAULT.depth() * 3);
    scope.visits.set(0);
    let session = EvaluationSession::with_limits(&scope, 1, Limits::DEFAULT.with_work(99));
    let work = session.evaluate(&Ast::Ref("0".into()));
    assert!(work.size_limited && !work.depth_limited);
    assert!(scope.visits.get() < 100);
    assert!(session.flags().size_limited);
    assert!(session.evaluate(&Ast::Num(1.0)).size_limited);
}

#[test]
fn caught_callback_panic_restores_allowance_and_stack_policy() {
    struct Panic;
    impl EvaluationScope for Panic {
        fn call(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let result = eval.evaluate_result(&Ast::Arr(Arc::new(vec![Ast::Num(1.0); 100])));
            assert!(result.size_limited);
            panic!("caller panic after a cut");
        }
    }
    let session = EvaluationSession::with_limits(&Panic, 1, Limits::DEFAULT.with_work(30));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.evaluate(&ast("@Panic()"))
    }));
    assert!(result.is_err());
    assert!(session.flags().size_limited);
    assert!(session.evaluate(&Ast::Num(1.0)).size_limited);
    let fresh = evaluate_expression(&Ast::Num(1.0), &());
    assert!(!fresh.size_limited && !fresh.depth_limited);
    assert_eq!(fresh.value, Some(Value::Num(1.0)));
}

#[test]
fn lookup_evaluators_share_the_allowance_while_returning_supplied_data() {
    struct Indirect;
    impl EvaluationScope for Indirect {
        fn state(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            if name == "$state" {
                eval.evaluate(&Ast::Num(3.0));
                Some(Value::Num(3.0))
            } else {
                None
            }
        }
        fn extra(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            if name == "$extra" {
                eval.evaluate(&Ast::Num(7.0));
                Some(Value::Num(7.0))
            } else {
                None
            }
        }
    }
    let session = EvaluationSession::with_limits(&Indirect, 1, Limits::DEFAULT.with_work(100));
    assert_eq!(
        session.evaluate(&ast("$state + $extra")).value,
        Some(Value::Num(10.0))
    );
    let cut = EvaluationSession::with_limits(&Indirect, 1, Limits::DEFAULT.with_work(10))
        .evaluate(&ast("$state + $extra"));
    assert!(cut.size_limited && !cut.depth_limited);
}

#[test]
fn a_resumed_outer_cut_is_not_charged_to_the_inner_allowance() {
    struct Inner<'a>(&'a ExpressionEvaluator<'a>);
    impl EvaluationScope for Inner<'_> {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            let cut = self
                .0
                .evaluate_result(&Ast::Arr(Arc::new(vec![Ast::Num(1.0); 100])));
            assert!(cut.size_limited && !cut.depth_limited);
            Some(Value::Num(9.0))
        }
    }
    struct Outer;
    impl EvaluationScope for Outer {
        fn call(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let inner = evaluate_expression(&ast("@Inner()"), &Inner(eval));
            assert!(!inner.size_limited && !inner.depth_limited);
            assert_eq!(inner.value, Some(Value::Num(9.0)));
            Some(Value::Num(3.0))
        }
    }
    let outer = EvaluationSession::with_limits(&Outer, 1, Limits::DEFAULT.with_work(40));
    let cut = outer.evaluate(&ast("@Outer()"));
    assert!(cut.size_limited && outer.flags().size_limited);
    let fresh = evaluate_expression(&Ast::Num(1.0), &());
    assert!(!fresh.size_limited && !fresh.depth_limited);
}

#[test]
fn fresh_nested_depth_and_ceiling_restore_after_caught_panics() {
    struct Panic;
    impl EvaluationScope for Panic {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            panic!("caller panic")
        }
    }
    let small = EvaluationSession::with_limits(&Panic, 1, Limits::DEFAULT.with_depth(1));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || small.evaluate(&ast("@Panic()"))
        ))
        .is_err()
    );
    let mut deep = Ast::Num(1.0);
    for _ in 0..4000 {
        deep = Ast::UnaryOp {
            op: "-",
            operand: Arc::new(deep),
        };
    }
    let raised = openui_runtime::evaluate_expression_with_limits(
        &deep,
        &(),
        Limits::DEFAULT.with_depth(5000),
    );
    assert!(!raised.size_limited && !raised.depth_limited);
    assert_eq!(raised.value, Some(Value::Num(1.0)));
}

#[test]
fn nested_references_work_when_the_raised_depth_allows_the_whole_chain() {
    struct Chain;
    impl EvaluationScope for Chain {
        fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let n: usize = name.parse().ok()?;
            if n == 2000 {
                Some(Value::Num(1.0))
            } else {
                eval.evaluate(&Ast::Ref((n + 1).to_string()))
            }
        }
    }
    std::thread::Builder::new()
        .name("references_depth::worker".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let result = openui_runtime::evaluate_expression_with_limits(
                &Ast::Ref("0".into()),
                &Chain,
                Limits::DEFAULT.with_depth(1000),
            );
            assert!(!result.size_limited && !result.depth_limited);
            assert_eq!(result.value, Some(Value::Num(1.0)));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_fresh_nested_walk_at_the_physical_ceiling_is_a_depth_cut_in_every_build() {
    struct Stack(Cell<usize>);
    impl EvaluationScope for Stack {
        #[inline(never)]
        fn reference(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            // Keep less than the stack red zone live across each nested entry.
            // This spends physical stack faster than the logical depth limit,
            // so the public evaluator's ceiling recovery itself is exercised.
            let held = [0u8; 64 * 1024];
            std::hint::black_box(&held);
            self.0.set(self.0.get() + 1);
            let result = evaluate_expression(&Ast::Ref("r".into()), self).value;
            std::hint::black_box(&held);
            result
        }
    }
    let scope = Stack(Cell::new(0));
    let result = evaluate_expression(&Ast::Ref("r".into()), &scope);
    assert!(result.depth_limited && !result.size_limited);
    assert!(
        scope.0.get() < Limits::DEFAULT.depth() * 3,
        "ceiling was not reached: {}",
        scope.0.get()
    );
}

#[test]
fn a_fresh_lower_depth_limit_includes_the_already_active_reference_chain() {
    struct Lower;
    impl EvaluationScope for Lower {
        fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let n: usize = name.parse().ok()?;
            if n < 10 {
                eval.evaluate(&Ast::Ref((n + 1).to_string()))
            } else {
                openui_runtime::evaluate_expression_with_limits(
                    &Ast::Num(1.0),
                    &(),
                    Limits::DEFAULT.with_depth(1),
                )
                .value
            }
        }
    }
    let result = evaluate_expression(&Ast::Ref("0".into()), &Lower);
    assert!(result.depth_limited && !result.size_limited);
}

#[test]
fn an_unbounded_outer_limit_still_reports_a_nested_default_depth_cut() {
    let result = openui_runtime::evaluate_expression_with_limits(
        &Ast::Ref("0".into()),
        &Chain,
        Limits::DEFAULT.with_depth(usize::MAX),
    );
    assert!(result.depth_limited && !result.size_limited);
}

#[test]
fn typed_values_at_the_stack_ceiling_preserve_a_depth_cut() {
    struct Typed {
        items: Value,
        probe: Ast,
    }
    impl EvaluationScope for Typed {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(self.items.clone())
        }
        #[inline(never)]
        fn reference(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            let held = [0u8; 64 * 1024];
            std::hint::black_box(&held);
            // Substitution reads the deferred AST as data. Its conversion
            // helpers must also recover when a nested walk hits the ceiling.
            let probe = evaluate_expression(&self.probe, self);
            if probe.depth_limited {
                assert!(!probe.size_limited);
                return probe.value;
            }
            let result = evaluate_expression(&Ast::Ref("r".into()), self).value;
            std::hint::black_box(&held);
            result
        }
    }
    let raw = ast("@Fn({data:[true,false,null,query]}, $value = @Count([1,2]))");
    let scope = Typed {
        items: Value::Arr(Arc::new(vec![Value::Ast(Arc::new(raw))])),
        probe: ast("@Each($items, \"item\", item)"),
    };
    let result = evaluate_expression(&Ast::Ref("r".into()), &scope);
    assert!(result.depth_limited && !result.size_limited);
}

#[test]
fn every_mix_of_session_helper_and_fresh_hops_counts_the_same_depth() {
    struct Mix<'a> {
        session: Cell<Option<&'a EvaluationSession<'a>>>,
        visits: Cell<usize>,
        mode: usize,
    }
    impl EvaluationScope for Mix<'_> {
        fn reference(&self, _: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            let visit = self.visits.get();
            self.visits.set(visit + 1);
            let next = Ast::Ref("next".into());
            match if self.mode == 3 { visit % 3 } else { self.mode } {
                0 => eval.evaluate(&next),
                1 => self.session.get()?.evaluate(&next).value,
                _ => evaluate_expression(&next, self).value,
            }
        }
    }
    let mut counts = Vec::new();
    for mode in 0..4 {
        let scope = Mix {
            session: Cell::new(None),
            visits: Cell::new(0),
            mode,
        };
        let session = EvaluationSession::new(&scope, 100);
        scope.session.set(Some(&session));
        let result = session.evaluate(&Ast::Ref("start".into()));
        assert!(result.depth_limited && !result.size_limited);
        counts.push(scope.visits.get());
    }
    assert!(counts.iter().all(|n| *n == counts[0]), "{counts:?}");
    assert!(counts[0] <= Limits::DEFAULT.depth() * 3);
}

#[test]
fn fresh_work_cuts_need_forwarding_while_chain_depth_cuts_are_shared() {
    struct Inner {
        depth: bool,
        cut: Cell<bool>,
    }
    impl EvaluationScope for Inner {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            let limits = if self.depth {
                Limits::DEFAULT.with_depth(0)
            } else {
                Limits::DEFAULT.with_work(0)
            };
            let inner =
                openui_runtime::evaluate_expression_with_limits(&Ast::Num(1.0), &(), limits);
            self.cut.set(inner.size_limited || inner.depth_limited);
            inner.value
        }
    }
    for depth in [false, true] {
        let scope = Inner {
            depth,
            cut: Cell::new(false),
        };
        let outer = evaluate_expression(&ast("@Inner()"), &scope);
        assert!(scope.cut.get());
        assert!(!outer.size_limited);
        assert_eq!(outer.depth_limited, depth);
    }
}
