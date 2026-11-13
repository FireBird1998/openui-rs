//! OpenUI Lang's runtime, built on the core's one expression evaluator.
//!
//! Evaluate expressions against caller-supplied names and keep live state in a
//! [`Store`]. Tree evaluation, bindings and actions will follow. Parsing remains in `openui-lang`, which
//! does not depend on this crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod store;
pub use store::{InitializationResult, StateRefusal, StateValue, Store, StoreError, StoreSnapshot};

pub use openui_lang::{
    Ast, EvaluationFlags, EvaluationResult, EvaluationScope, EvaluationSession,
    ExpressionEvaluator, JsObject, Limits, Value, evaluate_expression,
    evaluate_expression_with_limits,
};
