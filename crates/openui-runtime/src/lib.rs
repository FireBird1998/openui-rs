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
//! and restoring JSON does not grant that identity. [`run_action`] consumes a
//! retained plan by reference and returns ordered [`HostEvent`] values; it does
//! not invoke a handler or redraw. Mutations return an opaque continuation;
//! [`resume_action`] consumes the host outcome against current state/program
//! inputs with the original work allowance. Embedders suppress clicks while
//! their response bundle is blocked. [`get_field_value`] and [`set_field_value`]
//! keep renderer form fields in the Store, and [`initialize_field_default`]
//! applies settled defaults without overwriting user values.
//!
//! Runtime admission and entry points:
//!
//! | Entry points | Configuration and bounds | Cut/refusal |
//! |---|---|---|
//! | `Store::new`, `Default`, `with_limits` | Default or retained core `Limits` | Empty state |
//! | `Store::set`, `set_pristine`, `initialize` | Store work/depth and result ceiling; exact incremental total-state cap | `StoreError`, or per-name `InitializationResult::refused`; refused names unchanged |
//! | `Store::get`, `snapshot`, `is_pristine`, `version`, `dispose` | Work/data bounded by admitted state; snapshots share bodies; disposal revokes continuations | No evaluation cut |
//! | `evaluate_tree`, `evaluate_tree_with_limits` | Default/explicit shared evaluation work, depth, room and grown stack | Tree flags; evaluated props and errors retained, remaining props raw |
//! | `write_binding`, `set_field_value`, `initialize_field_default`, and their `_with_limits` variants | Default/explicit operation limits, then Store admission | `BindingWriteError`; no refused write effect |
//! | `get_field_value` | One admitted Store value, shallow field projection | Undefined as `None` |
//! | `run_action`, `run_action_with_limits`, `resume_action` | Default/explicit one run-wide work allowance retained on resume; per-operation room, then Store admission | Flags and typed failure; earlier effects/events retained; cut ends the run |
//! | `PreparedReferenceValues::new`, `with_limits` | Default/explicit bounded supply measurement; no evaluation work spent | Refused entries flag only when read |
//! | `TreeInput` constructors and builders | Borrowed configuration and retained Arc, constant work | No evaluation |
//! | `BindingProps` construction, insertion, queries; `LibraryFunctions::new`, `register` | Caller-owned host configuration; registration and spec extraction have no configurable admission bound | No refusal channel |
//! | `TreeResult::to_json`, `HostEvent::to_json`, `HostEvent::value`, `ActionRun::is_suspended` | Finished data; core stack-safe serialization, borrowed access or constant work | Serialization has no configurable output cap |
//! | Re-exported expression/session/scope/value interfaces | Core defaults or explicit/inherited `Limits`; one evaluator | Core flags and failures |
//!
//! The Store cap is `Limits::result_size()` (8 MiB by default), independent
//! of the program's scaled evaluation allowance. It counts the exact JSON
//! object bytes of retained names, including pristine-only names, with
//! undefined projected to null. Empty state costs zero. Aliases count at each
//! name. Persisted initialization loads entries that fit and reports others;
//! duplicate names use their last input value. Reset batches preflight their
//! combined final size atomically. A smaller replacement fits the total cap
//! provided it passes its own work/depth/value admission. See ADR 0005.
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
    ActionContinuation, ActionFailure, ActionResumeError, ActionRun, BindingProps,
    BindingWriteError, FieldWrite, FunctionResult, HostEvent, LibraryFunctions,
    PreparedReferenceValues, RuntimeError, TreeInput, TreeResult, evaluate_tree,
    evaluate_tree_with_limits, get_field_value, initialize_field_default,
    initialize_field_default_with_limits, resume_action, run_action, run_action_with_limits,
    set_field_value, set_field_value_with_limits, write_binding, write_binding_with_limits,
};

pub use openui_lang::{
    Ast, EvaluationFailure, EvaluationFlags, EvaluationResult, EvaluationScope, EvaluationSession,
    ExpressionEvaluator, ExpressionOperation, ExpressionOutput, ExpressionType, ExpressionValue,
    JsObject, Limits, PreparedReferenceValue, PreparedValidationSchema, Value, evaluate_expression,
    evaluate_expression_with_limits,
};
