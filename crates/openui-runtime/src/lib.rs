//! OpenUI Lang's runtime, built on the core's one expression evaluator.
//!
//! Evaluate expressions against caller-supplied names and keep live state in a
//! [`Store`]. Evaluate parsed trees with [`evaluate_tree`]. Write input bindings with [`write_binding`]. Parsing remains in `openui-lang`, which
//! does not depend on this crate.
//!
//! References follow lang-core 1.1.0's query manager before any tool runs:
//! complete queries read their evaluated defaults (or null), incomplete queries
//! read null, and unrun mutations read `{status:"idle"}`. Declaration order does
//! not matter. Defaults resolve against root scope, never a loop's bindings.
//! An own supplied entry for a declared query/mutation wins, including null,
//! false and empty containers. This supplied overlay is our embedder interface
//! for loaded data; it does not run tools or reproduce query scheduling.
//!
//! Use [`TreeInput::with_reference_values`] for per-call measurement, or prepare
//! [`PreparedReferenceValues`] once and pass it on every stream evaluation with
//! [`TreeInput::with_prepared_reference_values`]. Admitted supplied data, including
//! undeclared/unread entries, contributes input size before the session starts.
//! Undeclared names do not resolve. Refused data earns no allowance and flags a
//! cut only when read. Preparation has its own depth/work bounds; containers
//! stay shared; scalar reference strings are converted once in preparation.
//! Prepared reads use the cached representation, with no new credit or full
//! input walk. Output and operations still pay their normal limits. Defaults and props share
//! the same evaluation session. Reading data earns no additional allowance.
//!
//! Action calls produce deferred `{steps:[...]}` plans. Pass the original
//! library's `actions` JSON with [`TreeInput::with_actions`] for custom calls:
//! compiled parsing definitions do not retain the whole validation schema.
//! AST-shaped calls in evaluated state/query props can invoke declared actions
//! with arguments chosen by that data. Finished custom steps cannot forge marks.
//! Retained Values keep custom-step identity within their Store; serializing
//! and restoring JSON does not grant that identity. Running plans is separate.
//!
//! The reference harness registers and reads lang-core's own query manager with
//! no provider, refresh or tool execution. See `CONFORMANCE.md` for its source
//! locations and the pinned supplied overlay. Both parsers currently mark an
//! auto-closed query `complete: true` while streaming. We honor explicit false
//! metadata, but cannot identify an unfinished query from per-statement parser
//! metadata until the parsers expose it. Global `meta.incomplete` is insufficient.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod store;
mod tree;
pub use store::{InitializationResult, StateRefusal, StateValue, Store, StoreError, StoreSnapshot};
pub use tree::{
    BindingProps, BindingWriteError, PreparedReferenceValues, RuntimeError, TreeInput, TreeResult,
    evaluate_tree, evaluate_tree_with_limits, write_binding, write_binding_with_limits,
};

pub use openui_lang::{
    Ast, EvaluationFailure, EvaluationFlags, EvaluationResult, EvaluationScope, EvaluationSession,
    ExpressionEvaluator, ExpressionOperation, ExpressionOutput, ExpressionType, ExpressionValue,
    JsObject, Limits, PreparedReferenceValue, PreparedValidationSchema, Value, evaluate_expression,
    evaluate_expression_with_limits,
};
