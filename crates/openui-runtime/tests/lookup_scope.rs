//! Lookups evaluate at the root; calls retain their enclosing loop bindings.
use openui_lang::{parse_expression, tokenize};
use openui_runtime::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Value, evaluate_expression,
};
use std::cell::Cell;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}
struct Scope<'a>(Cell<Option<&'a EvaluationSession<'a>>>);
impl EvaluationScope for Scope<'_> {
    fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        if name == "i" {
            return Some(Value::Num(7.0));
        }
        let node = ast("i * 10");
        if let Some(session) = self.0.get() {
            session.evaluate(&node).value
        } else {
            eval.evaluate(&node)
        }
    }
    fn state(&self, _: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let node = Ast::Ref("i".into());
        let value = if let Some(session) = self.0.get() {
            session.evaluate(&node).value
        } else {
            eval.evaluate(&node)
        };
        assert_eq!(value, Some(Value::Num(7.0)));
        Some(Value::Num(20.0))
    }
    fn extra(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let node = Ast::Ref("i".into());
        let value = if let Some(session) = self.0.get() {
            session.evaluate(&node).value
        } else {
            eval.evaluate(&node)
        };
        assert_eq!(value, Some(Value::Num(7.0)));
        (name == "$extra").then_some(Value::Num(30.0))
    }
}
#[test]
fn lookup_callbacks_and_session_reentry_do_not_capture_loop_bindings() {
    for reenter in [false, true] {
        let scope = Scope(Cell::new(None));
        let session = EvaluationSession::new(&scope, 600);
        if reenter {
            scope.0.set(Some(&session));
        }
        for source in ["@Each([1,2,3],\"i\",r)", "@Each([5,6,7],\"i\",r)"] {
            let result = session.evaluate(&ast(source));
            assert!(!result.size_limited && !result.depth_limited);
            assert_eq!(
                result.value,
                Some(Value::Arr(Arc::new(vec![Value::Num(70.0); 3])))
            );
        }
        let result = session.evaluate(&ast("@Each([1,2],\"i\",[$state,$extra])"));
        assert_eq!(
            result.value,
            Some(Value::Arr(Arc::new(vec![
                Value::Arr(Arc::new(vec![
                    Value::Num(20.0),
                    Value::Num(30.0)
                ]));
                2
            ])))
        );
    }
    // A free evaluation must use the same root lookup semantics.
    assert_eq!(
        evaluate_expression(&ast("@Each([1],\"i\",r)"), &Scope(Cell::new(None))).value,
        Some(Value::Arr(Arc::new(vec![Value::Num(70.0)])))
    );
}
