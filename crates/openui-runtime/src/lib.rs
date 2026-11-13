//! OpenUI Lang's runtime, built on the core's one expression evaluator.
//!
//! This first interface evaluates one expression against caller-supplied state,
//! statement references and extra scope. The store, tree evaluation, bindings
//! and action runner will live here. Parsing remains in `openui-lang`, which
//! does not depend on this crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub use openui_lang::{
    Ast, EvaluationFlags, EvaluationResult, EvaluationScope, EvaluationSession,
    ExpressionEvaluator, Limits, Value, evaluate_expression, evaluate_expression_with_limits,
};
