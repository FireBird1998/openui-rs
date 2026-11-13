//! OpenUI Lang's runtime, built on the core's one expression evaluator.
//!
//! Evaluate expressions against caller-supplied names and keep live state in a
//! [`Store`]. Evaluate parsed trees with [`evaluate_tree`]. Bindings and actions will follow. Parsing remains in `openui-lang`, which
//! does not depend on this crate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod store;
mod tree;
pub use store::{InitializationResult, StateRefusal, StateValue, Store, StoreError, StoreSnapshot};
pub use tree::{RuntimeError, TreeInput, TreeResult, evaluate_tree, evaluate_tree_with_limits};

pub use openui_lang::{
    Ast, EvaluationFailure, EvaluationFlags, EvaluationResult, EvaluationScope, EvaluationSession,
    ExpressionEvaluator, ExpressionOperation, ExpressionType, JsObject, Limits, Value,
    evaluate_expression, evaluate_expression_with_limits,
};
