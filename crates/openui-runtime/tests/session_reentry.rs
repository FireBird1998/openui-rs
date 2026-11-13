//! Re-entering a session uses its suspended allowance and scope.
use openui_runtime::{Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Limits, Value};
use std::cell::Cell;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);

struct Chain<'a> {
    session: Cell<Option<&'a EvaluationSession<'a>>>,
    links: usize,
}
impl EvaluationScope for Chain<'_> {
    fn reference(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        let n: usize = name.parse().ok()?;
        if n == self.links {
            return Some(Value::Num(1.0));
        }
        self.session
            .get()?
            .evaluate(&Ast::Ref((n + 1).to_string()))
            .value
    }
}
#[test]
fn own_session_reference_chain_shares_depth_and_recovers_for_later_props() {
    let scope = Chain {
        session: Cell::new(None),
        links: 4000,
    };
    let session = EvaluationSession::new(&scope, 200_000);
    scope.session.set(Some(&session));
    let result = session.evaluate(&Ast::Ref("0".into()));
    assert!(result.depth_limited && !result.size_limited);
    assert!(session.flags().depth_limited);
    assert!(session.evaluate(&Ast::Num(2.0)).depth_limited);
}
#[test]
fn raised_depth_allows_four_thousand_own_session_references() {
    let scope = Chain {
        session: Cell::new(None),
        links: 4000,
    };
    let session = EvaluationSession::with_limits(&scope, 200_000, Limits::DEFAULT.with_depth(5000));
    scope.session.set(Some(&session));
    let result = session.evaluate(&Ast::Ref("0".into()));
    assert!(!result.depth_limited && !result.size_limited);
    assert_eq!(result.value, Some(Value::Num(1.0)));
    assert_eq!(
        session.evaluate(&Ast::Num(2.0)).value,
        Some(Value::Num(2.0))
    );
}

#[test]
fn reentry_matches_callback_work_and_cached_lookups() {
    struct Calls<'a> {
        session: Cell<Option<&'a EvaluationSession<'a>>>,
        completed: Cell<usize>,
        reads: Cell<usize>,
    }
    impl EvaluationScope for Calls<'_> {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.reads.set(self.reads.get() + 1);
            Some(Value::Num(3.0))
        }
        fn call(&self, _: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            for _ in 0..100 {
                let node = Ast::StateRef("$x".into());
                let result = if let Some(session) = self.session.get() {
                    session.evaluate(&node)
                } else {
                    eval.evaluate_result(&node)
                };
                if result.size_limited || result.depth_limited {
                    return result.value;
                }
                self.completed.set(self.completed.get() + 1);
            }
            Some(Value::Num(3.0))
        }
    }
    let run = |reenter| {
        let scope = Calls {
            session: Cell::new(None),
            completed: Cell::new(0),
            reads: Cell::new(0),
        };
        let session = EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_work(40));
        if reenter {
            scope.session.set(Some(&session));
        }
        let call = openui_lang::parse_expression(&openui_lang::tokenize("@Repeat()"));
        let result = session.evaluate(&call);
        assert!(result.size_limited && !result.depth_limited);
        assert!(session.flags().size_limited);
        assert_eq!(scope.reads.get(), 1);
        assert!(scope.completed.get() > 0 && scope.completed.get() < 100);
        (result, scope.completed.get())
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn reentry_from_calls_keeps_bindings_and_lookups_use_the_root() {
    struct Scope<'a>(Cell<Option<&'a EvaluationSession<'a>>>);
    impl EvaluationScope for Scope<'_> {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.0.get()?.evaluate(&Ast::Num(2.0));
            Some(Value::Num(2.0))
        }
        fn extra(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            if name == "$extra" {
                self.0.get()?.evaluate(&Ast::Num(3.0));
                Some(Value::Num(3.0))
            } else {
                None
            }
        }
        fn call(&self, call: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            let Ast::Comp { args, .. } = call else {
                return None;
            };
            self.0.get()?.evaluate(&args[0]).value
        }
    }
    let scope = Scope(Cell::new(None));
    let session = EvaluationSession::new(&scope, 600);
    scope.0.set(Some(&session));
    let expression = Ast::Comp {
        name: "Each".into(),
        function: false,
        mapped_props: None,
        args: std::sync::Arc::new(vec![
            Ast::Arr(std::sync::Arc::new(vec![Ast::Num(4.0), Ast::Num(8.0)])),
            Ast::Str("i".into()),
            Ast::Comp {
                name: "Echo".into(),
                function: true,
                mapped_props: None,
                args: std::sync::Arc::new(vec![Ast::RuntimeRef {
                    n: "i".into(),
                    ref_type: openui_lang::RefType::Query,
                }]),
            },
        ]),
    };
    let result = session.evaluate(&expression);
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(
        result.value,
        Some(Value::Arr(std::sync::Arc::new(vec![
            Value::Num(4.0),
            Value::Num(8.0)
        ])))
    );
    let sum = openui_lang::parse_expression(&openui_lang::tokenize("$state + $extra"));
    assert_eq!(session.evaluate(&sum).value, Some(Value::Num(5.0)));
}

#[test]
fn capturing_long_loop_bindings_costs_work_in_bytes() {
    struct Calls(Cell<usize>);
    impl EvaluationScope for Calls {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.0.set(self.0.get() + 1);
            Some(Value::Null)
        }
    }
    let scope = Calls(Cell::new(0));
    let expression = Ast::Comp {
        name: "Each".into(),
        function: false,
        mapped_props: None,
        args: std::sync::Arc::new(vec![
            Ast::Arr(std::sync::Arc::new(vec![Ast::Num(1.0); 1000])),
            Ast::Str("i".repeat(100_000)),
            Ast::Comp {
                name: "Call".into(),
                function: true,
                mapped_props: None,
                args: std::sync::Arc::new(vec![]),
            },
        ]),
    };
    let result = openui_runtime::evaluate_expression_with_limits(
        &expression,
        &scope,
        Limits::DEFAULT.with_work(1_000_000),
    );
    assert!(result.size_limited && !result.depth_limited);
    assert!(scope.0.get() > 0 && scope.0.get() < 10);
}
