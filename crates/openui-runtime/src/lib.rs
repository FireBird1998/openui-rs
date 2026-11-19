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
//! The standalone [`QueryManager`] evaluates a settled program against a Store
//! and returns the tool calls an embedder should perform. Report outcomes with
//! [`QueryCommand::Complete`]. The manager owns its snapshot and cache and
//! requires no I/O or clock. Hosts can read the snapshot as values or JSON.
//! [`QueryManager::set_extra_key_values`] adds an embedder's values to the
//! cache keys of one tool's queries. [`QueryCommand::Invalidate`] runs
//! queries again; with [`TreeInput::with_query_runs`], an action's `@Run` of
//! a query lands in [`ActionRun::query_runs`] for it instead of a host event.
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
//! inputs with the original work allowance. The session suppresses new clicks
//! while its response bundle is blocked. [`get_field_value`] and [`set_field_value`]
//! keep renderer form fields in the Store, and [`initialize_field_default`]
//! applies settled defaults without overwriting user values.
//!
//! Explicit [`BindingProps`] limits charge JSON-escaped names; removing a pair
//! refunds its prop and, on the last prop, its component name in full. Repeated
//! insert/remove cycles retain no charge for removed names. Refused insertions
//! preserve the configuration. Default binding configuration has no name quota.
//!
//! Runtime admission and entry points:
//!
//! | Entry points | Configuration and bounds | Cut/refusal |
//! |---|---|---|
//! | `Store::new`, `Default`, `with_limits` | Default or retained core `Limits` | Empty state |
//! | `Store::set`, `set_pristine`, `initialize` | Store work/depth and result ceiling; exact incremental total-state cap | `StoreError`, or per-name `InitializationResult::refused`; refused names unchanged |
//! | `Store::get`, `snapshot`, `is_pristine`, `version`, `dispose` | Reads bounded by admitted state; snapshots share bodies; disposal also visits private authority records | No evaluation cut |
//! | `evaluate_tree`, `evaluate_tree_with_limits` | Default/explicit shared evaluation work, depth, room and grown stack | Tree flags; evaluated props and errors retained, remaining props raw |
//! | `write_binding`, `set_field_value`, `initialize_field_default`, and their `_with_limits` variants | Default/explicit operation limits, then Store admission | `BindingWriteError`; no refused write effect |
//! | `get_field_value` | One admitted Store value, shallow field projection | Undefined as `None` |
//! | `run_action`, `run_action_with_limits`, `resume_action` | Default/explicit one run-wide work allowance retained on resume; per-operation room, then Store admission | Flags and typed failure; earlier effects/events retained; cut ends the run |
//! | `PreparedReferenceValues::new`, `with_limits`, `owned_with_limits` | Default/explicit bounded supply measurement; no evaluation work spent | Refused entries flag only when read |
//! | `PreparedReferenceValue::new`, `with_limits` | Core-measured bytes and whole-read height; default or explicit preparation bounds | Refusal flags only when read |
//! | `ExpressionEvaluator::supply_input` | Core-measured program/start/reference receipts; one largest combined floor; setup spends no evaluation work | Typed refusal for different receipt limits or an already-cut allowance |
//! | `TreeInput` constructors and builders | Borrowed configuration and retained Arc, constant work | No evaluation |
//! | `BindingProps::{new,with_limits,insert,remove,from_prompt_spec,from_prompt_spec_with_limits}` | No implicit quota; explicit spec admission and incremental retained-name cap | Typed `LimitError`; refused insertion unchanged |
//! | `LibraryFunctions::{new,with_limits,register}` | Borrow schemas without walking; explicit opt-in name cap; used schemas checked during evaluation | Typed `LimitError`; implementation unchanged on refusal |
//! | `TreeResult::to_json`, `HostEvent::to_json`, their `_with_limits` variants, `value_to_json`, `value_to_json_with_limits` | Default preserves admitted results; explicit variants admit work/depth/output before serialization | `Result<String, LimitError>` |
//! | [`extract_mcp_tool_result`] | Explicit host JSON byte, work and depth bounds | Tool call outcome or typed `McpResponseError` |
//! | `Store::snapshot_json`, `snapshot_json_with_limits` | Default exports admitted state; explicit serialization limits | `Result<String, LimitError>` |
//! | `BindingProps::{contains,undecided}`, `HostEvent::value`, `ActionRun::is_suspended` | Bounded configuration, borrowed access or constant work | No evaluation |
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
//! Completed streaming defaults retain parser identities; their reduced Store
//! notification count is a pinned divergence (CONFORMANCE.md). Values agree.
//! Snapshot JSON omits undefined entries; the raw snapshot and admission count
//! retain their names. Persisted re-initialization therefore restores a declared
//! default for an omitted undefined entry, rather than the null exported by
//! earlier Rust versions. Undeclared undefined names disappear on that round trip. Query defaults are lazy and frozen per action run.
//! State, raw and prepared query lookups check depth only where consumed;
//! shallow receivers do not traverse deep siblings. Whole-value projection and
//! coercion retain their depth checks. Supplied
//! deferred ASTs earn their own measurement work; shared nodes earn credit once.
//! Program and starting receipts contain no supplied data. A suspended program
//! cache holds a Weak identity so a unique Arc can move on make_mut.
//!
//! Serialized result room is not a universal live-memory allowance. Eager
//! temporaries can retain many individually admitted callback results before
//! an opaque later callback selects one. Host computation and allocation have
//! no runtime quota. The opt-in `Limits::with_peak_memory` bounds portable
//! evaluation reservations (ADR 0011), with explicit exclusions; it is disabled
//! by default and is not a process-heap limit. Profile identical inputs with
//! `Limits::with_peak_memory(usize::MAX)` and read `EvaluationMemory::accounted_peak`
//! through `evaluate_tree_with_memory`, `EvaluationSession::memory` or
//! `ActionRun::memory`; use at least that peak, including all resumes. The
//! cumulative account is not a heap multiplier. Finite allocator shape bounds, and the unresolved general
//! amplification bound, are documented in ADR 0001/0008 and `TESTING.md`.
//! Disposal is linear in retained state and private identity records for custom
//! plans; keeping plans alive can grow those records independently of Store JSON.
//! Re-exported raw core `Ast::to_json`/`walk` and `Value::from_json` are stack-safe
//! finished-input walks without a configurable quota. Use the explicit-limit runtime
//! JSON helpers for bounded exports of caller-created or edited data.
//! Substitution shares matching argument/mapped-prop bodies without changing
//! their walks or charges; deferred syntax and arbitrary scopes keep their
//! independent representations.
//!
//! The reference harness registers and reads lang-core's own query manager with
//! no provider, refresh or tool execution. See `CONFORMANCE.md` for its source
//! locations and the pinned supplied overlay. Both parsers currently mark an
//! auto-closed query `complete: true` while streaming. We honor explicit false
//! metadata, but cannot identify an unfinished query from per-statement parser
//! metadata until the parsers expose it. Global `meta.incomplete` is insufficient.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod admission;
pub use admission::{LimitError, value_to_json, value_to_json_with_limits};
mod mcp;
pub use mcp::{McpResponseError, McpToolOutcome, extract_mcp_tool_result};
mod query;
pub use query::{
    CloudScripts, QueryCommand, QueryCommandError, QueryManager, QueryUpdate, ToolCall, ToolCallId,
    ToolCallOutcome, ToolOutcome,
};
mod store;
mod tree;
pub use store::{InitializationResult, StateRefusal, StateValue, Store, StoreError, StoreSnapshot};
pub use tree::{
    ActionContinuation, ActionFailure, ActionResumeError, ActionRun, BindingProps,
    BindingWriteError, FieldWrite, FunctionResult, HostEvent, LibraryFunctions, MutationRequest,
    PreparedReferenceValues, RuntimeError, TreeInput, TreeResult, evaluate_tree,
    evaluate_tree_with_limits, evaluate_tree_with_memory, get_field_value,
    initialize_field_default, initialize_field_default_with_limits, resume_action, run_action,
    run_action_with_limits, set_field_value, set_field_value_with_limits, write_binding,
    write_binding_with_limits,
};

pub use openui_lang::{
    Ast, EvaluationFailure, EvaluationFlags, EvaluationMemory, EvaluationResult, EvaluationScope,
    EvaluationSession, ExpressionEvaluator, ExpressionOperation, ExpressionOutput, ExpressionType,
    ExpressionValue, JsObject, Limits, PreparedReferenceValue, PreparedValidationSchema, Value,
    evaluate_expression, evaluate_expression_with_limits,
};

#[cfg(test)]
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
