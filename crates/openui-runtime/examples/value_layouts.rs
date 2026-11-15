//! Public value/evaluator layout sizes for independent baseline comparisons.
use openui_lang::{
    Ast, Element, EvaluationFlags, EvaluationResult, EvaluationSession, ExpressionEvaluator,
    ExpressionOutput, ExpressionValue, JsObject, Limits, PreparedInput, PreparedReferenceValue,
    Value,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
fn main() {
    macro_rules! size {
        ($ty:ty) => {
            println!("{}\t{}", stringify!($ty), std::mem::size_of::<$ty>());
        };
    }
    size!(Ast);
    size!(Value);
    size!(Element);
    size!(JsObject<Value>);
    size!(Limits);
    size!(PreparedReferenceValue);
    size!(PreparedInput);
    size!(EvaluationFlags);
    size!(EvaluationResult);
    size!(EvaluationSession<'static>);
    size!(ExpressionEvaluator<'static>);
    size!(ExpressionValue<'static>);
    size!(ExpressionOutput);
}
