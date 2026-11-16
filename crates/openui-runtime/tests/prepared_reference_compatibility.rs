//! Prepared and raw references use the same whole-read depth rule.
use openui_lang::{
    Ast, EvaluationScope, EvaluationSession, Limits, PreparedReferenceValue, Value,
    parse_expression, tokenize,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
struct Scope {
    value: Value,
    prepared: Option<PreparedReferenceValue>,
}
impl EvaluationScope for Scope {
    fn prepared_reference(&self, _: &str) -> Option<&PreparedReferenceValue> {
        self.prepared.as_ref()
    }
    fn reference(&self, _: &str, _: &openui_lang::ExpressionEvaluator<'_>) -> Option<Value> {
        Some(self.value.clone())
    }
}
#[test]
fn prepared_and_raw_whole_reads_cut_at_the_same_depth() {
    for levels in [0, 1, 4, 20, 800] {
        let value = (0..levels).fold(Value::Null, |v, _| Value::Arr(Arc::new(vec![v])));
        for depth in [1, 2, 4, 16, 256] {
            for source in ["q == q", "q.length", "@Count(q)", "\"\" + q"] {
                let expression = parse_expression(&tokenize(source));
                let read = |prepared: bool| {
                    let scope = Scope {
                        value: value.clone(),
                        prepared: prepared.then(|| PreparedReferenceValue::new(&value)),
                    };
                    EvaluationSession::with_limits(
                        &scope,
                        100_000,
                        Limits::DEFAULT.with_depth(depth),
                    )
                    .evaluate(&expression)
                };
                let raw = read(false);
                let cached = read(true);
                assert_eq!(
                    (raw.size_limited, raw.depth_limited),
                    (cached.size_limited, cached.depth_limited),
                    "{source}, levels={levels}, depth={depth}"
                );
                assert_eq!(raw.value, cached.value);
            }
        }
    }
}
#[test]
fn reference_sizes_are_core_measured_and_cannot_be_claimed() {
    let value = Value::Str("x".repeat(600));
    let prepared = PreparedReferenceValue::new(&value);
    assert_eq!(prepared.input_bytes(), 602);
    let scope = Scope {
        value,
        prepared: Some(prepared),
    };
    assert!(
        EvaluationSession::new(&scope, 601)
            .evaluate(&Ast::Ref("q".into()))
            .size_limited
    );
    assert!(
        !EvaluationSession::new(&scope, 602)
            .evaluate(&Ast::Ref("q".into()))
            .size_limited
    );
}

#[test]
fn prepared_typed_scalar_nodes_count_their_complete_ast_json() {
    for node in [Ast::Num(1.), Ast::Bool(false), Ast::Bool(true), Ast::Null] {
        let expected = node.to_json().len();
        assert_eq!(
            PreparedReferenceValue::new(&Value::Ast(Arc::new(node))).input_bytes(),
            expected
        );
    }
}
#[test]
fn prepared_and_raw_deferred_nodes_have_the_same_whole_read_depth() {
    let mut node = Ast::Num(1.);
    for _ in 0..20 {
        node = Ast::UnaryOp {
            op: "!",
            operand: Arc::new(node),
        };
    }
    let value = Value::Ast(Arc::new(node));
    for depth in 1..15 {
        for source in ["q == q", "q.length", "@Count(q)", "\"\" + q"] {
            let expression = parse_expression(&tokenize(source));
            let read = |prepared: bool| {
                let scope = Scope {
                    value: value.clone(),
                    prepared: prepared.then(|| PreparedReferenceValue::new(&value)),
                };
                EvaluationSession::with_limits(&scope, 100_000, Limits::DEFAULT.with_depth(depth))
                    .evaluate(&expression)
            };
            let raw = read(false);
            let cached = read(true);
            assert_eq!(
                (raw.size_limited, raw.depth_limited),
                (cached.size_limited, cached.depth_limited),
                "{source},depth={depth}"
            );
            assert_eq!(raw.value, cached.value);
        }
    }
}
