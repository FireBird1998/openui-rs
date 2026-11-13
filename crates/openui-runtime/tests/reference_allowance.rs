//! References consume the program's allowance and never add input credit.
use openui_lang::{JsObject, parse_expression, tokenize};
use openui_runtime::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Value, evaluate_expression,
};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}
struct Wrapped;
impl EvaluationScope for Wrapped {
    fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let n: usize = name.strip_prefix('s')?.parse().ok()?;
        let value = if n == 0 {
            Value::Arr(Arc::new(vec![Value::Num(0.0)]))
        } else {
            eval.evaluate(&ast(&format!("@Each(s{}.v, \"i\", [i,i])", n - 1)))?
        };
        let mut wrapper = JsObject::new();
        wrapper.define("v", value);
        Some(Value::Obj(Arc::new(wrapper)))
    }
}
#[test]
fn wrapping_computed_reference_values_cannot_buy_input_credit() {
    let expression = ast("s16.v");
    let one = evaluate_expression(&expression, &Wrapped);
    assert!(one.size_limited && !one.depth_limited);
    let session = EvaluationSession::new(&Wrapped, 600);
    let many = session.evaluate(&expression);
    assert!(many.size_limited && !many.depth_limited);
}
struct Data(Value);
impl EvaluationScope for Data {
    fn reference(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.0.clone())
    }
}
#[test]
fn supplied_reference_data_must_be_paid_for_in_the_initial_size() {
    let data = Data(Value::Str("x".repeat(100_000)));
    let expression = Ast::Ref("rows".into());
    let paid = EvaluationSession::new(&data, 100_002).evaluate(&expression);
    assert!(!paid.size_limited && !paid.depth_limited);
    assert_eq!(paid.value, Some(data.0.clone()));
    let unpaid = EvaluationSession::new(&data, 0).evaluate(&expression);
    assert!(unpaid.size_limited && !unpaid.depth_limited);
}
