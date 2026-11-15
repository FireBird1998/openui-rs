//! Opaque default preparation preserves shallow reads of supplied data.
use openui_lang::{
    Ast, EvaluationScope, Limits, PreparedReferenceValue, Value, evaluate_expression,
    parse_expression, tokenize,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
struct Scope(PreparedReferenceValue);
impl EvaluationScope for Scope {
    fn prepared_reference(&self, _: &str) -> Option<&PreparedReferenceValue> {
        Some(&self.0)
    }
}
fn shallow(scope: &Scope, length: usize) {
    let out = evaluate_expression(&parse_expression(&tokenize("q != null")), scope);
    assert_eq!(out.value, Some(Value::Bool(true)));
    assert!(!out.size_limited && !out.depth_limited);
    let out = evaluate_expression(&parse_expression(&tokenize("@Count(q)")), scope);
    assert_eq!(out.value, Some(Value::Num(length as f64)));
    assert!(!out.size_limited && !out.depth_limited);
}
#[test]
fn default_opaque_tokens_keep_shallow_reads_beyond_the_preparation_depth() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let value = (0..40_000).fold(Value::Null, |v, _| Value::Arr(Arc::new(vec![v])));
            shallow(&Scope(PreparedReferenceValue::new(&value, 0)), 1);
            let explicit = Scope(PreparedReferenceValue::with_limits(
                &value,
                0,
                Limits::DEFAULT.with_depth(32_768),
            ));
            assert!(evaluate_expression(&Ast::Ref("q".into()), &explicit).depth_limited);
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn opaque_preparation_is_not_an_eight_mebibyte_value_admission() {
    let string = Value::Str("x".repeat(9 * 1024 * 1024));
    for token in [
        PreparedReferenceValue::new(&string, 0),
        PreparedReferenceValue::with_limits(&string, 0, Limits::DEFAULT),
    ] {
        shallow(&Scope(token), 0);
    }
    let array = Value::Arr(Arc::new(vec![Value::Null; 5 * 1024 * 1024]));
    for token in [
        PreparedReferenceValue::new(&array, 0),
        PreparedReferenceValue::with_limits(&array, 0, Limits::DEFAULT),
    ] {
        shallow(&Scope(token), 5 * 1024 * 1024);
    }
}
#[test]
fn default_reference_combination_preserves_large_prepaid_contributions() {
    use openui_lang::{EvaluationSession, PreparedInput};
    let token = PreparedReferenceValue::new(&Value::Null, 100_000_000);
    let visited = std::cell::Cell::new(0);
    let input = PreparedInput::from_prepared_references(
        std::iter::repeat_n(("q", &token), 3).inspect(|_| visited.set(visited.get() + 1)),
    );
    assert_eq!(visited.get(), 3);
    let session = EvaluationSession::with_limits(
        &(),
        0,
        Limits::DEFAULT
            .with_result_floor(0)
            .with_amplification(1)
            .with_result_ceiling(usize::MAX)
            .with_work(usize::MAX),
    );
    assert!(
        session.with_evaluator(|e| {
            e.register_input_floor(&input) && e.reserve_result(300_000_017)
        })
    );
}
