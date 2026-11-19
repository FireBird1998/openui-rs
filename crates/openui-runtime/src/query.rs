//! The clock-free query manager and its tool provider seam.
mod audit;
mod cascade;
mod delivery;
pub use delivery::{ReaderDiagnostic, ToolDelivery};
mod mutations;
mod readers;
mod refresh;
mod results;
#[cfg(test)]
mod round_tests;
mod runs;
mod scripts;
mod session;
use crate::tree::TreeScope;
use crate::{
    EvaluationFailure, EvaluationFlags, EvaluationScope, EvaluationSession, ExpressionEvaluator,
    ExpressionOutput, ExpressionValue, JsObject, LimitError, Limits, PreparedReferenceValue,
    PreparedReferenceValues, Store, TreeInput, Value,
};
use mutations::{MutationCall, Mutations};
use openui_lang::{Ast, JsonValue, ParseResult, PreparedProgram, QueryStatement};
use readers::{Read, ReaderPlan};
use results::Admitted;
pub use results::ResultLimit;
pub use scripts::CloudScripts;
pub use session::ToolCallOutcome;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

/// An opaque tool call identity minted by one query manager.
#[derive(Clone)]
pub struct ToolCallId(Arc<CallIdentity>);

impl fmt::Debug for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolCallId").finish_non_exhaustive()
    }
}

struct CallIdentity {
    lifecycle: Weak<()>,
    manager: Weak<()>,
    target: CallTarget,
    consumed: AtomicBool,
}

enum CallTarget {
    Query(Weak<PendingCall>),
    Mutation(MutationCall),
}

/// A call the embedder should perform with its tool provider.
#[derive(Debug)]
#[non_exhaustive]
pub struct ToolCall {
    /// Pass this identity back with the outcome.
    pub id: ToolCallId,
    /// The evaluated tool name coerced to a JavaScript string.
    pub tool_name: String,
    /// Evaluated arguments. Queries normalize nullish arguments to an empty object.
    /// A function at the root has no JSON projection.
    pub args: Option<Value>,
}

/// The outcome reported by the tool provider.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ToolOutcome<'a> {
    /// Host JSON. A missing query result becomes null. A missing mutation result omits data.
    Result(Option<&'a JsonValue>),
    /// A host result already represented as an evaluator value (including MCP numbers).
    ResultValue(&'a Value),
    /// The requested tool is missing. Names are in provider order.
    ToolNotFound(Vec<&'a str>),
    /// The tool reported an MCP error with this text.
    McpError(&'a str),
    /// The call failed with this message.
    CallFailed(&'a str),
}

/// One step of the query manager.
#[non_exhaustive]
pub enum QueryCommand<'a> {
    /// Evaluate the current program against the Store. The caller initializes
    /// state and decides when the program is settled.
    Evaluate {
        /// The current parser output.
        program: &'a ParseResult,
        /// The current state.
        store: &'a Store,
    },
    /// Advance the monotonic millisecond clock (initially zero). Leave room
    /// for one full platform interval: at most `u64::MAX - i32::MAX as u64`.
    AdvanceClock {
        /// Absolute time in milliseconds.
        now_ms: u64,
    },
    /// Suspend queries, release calls and mutations, and retain cached data.
    Dispose,
    /// Allow registration again. The next evaluate reinstalls timers.
    Activate,
    /// Complete one wanted call atomically. Readers use live Store writes, or
    /// the last committed state if the Store was dropped. A result over a limit
    /// ends the call as a tool-error. A refused command preserves its receipt.
    Complete {
        /// Identity returned when the call started.
        id: &'a ToolCallId,
        /// The tool provider's outcome.
        outcome: ToolOutcome<'a>,
    },
    /// Run the named queries again, as upstream's `invalidate` does for a
    /// `@Run` of a query and for its renderer's Retry. For each name that is
    /// a registered query, in order: a call starts for its current key with
    /// the tool name and arguments of the query's last evaluate, or, while
    /// that key has a call in flight, one more call is remembered for the
    /// key, with the tool name and arguments of the first query that asked.
    /// The remembered call starts when the call in flight is delivered
    /// to a query still on the key, whichever query started it (ADR 0013),
    /// and is forgotten if none is. However many runs are asked for
    /// meanwhile, one call follows. Unknown names are skipped. An empty list
    /// runs every registered query, as upstream treats an empty list.
    Invalidate {
        /// Statement ids, in the order to run them.
        statement_ids: &'a [&'a str],
    },
}

/// The effect of an accepted query command.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct QueryUpdate {
    /// Earliest absolute millisecond deadline after this command.
    pub next_refresh_due_ms: Option<u64>,
    /// Newly wanted calls, in dispatch order.
    pub calls: Vec<ToolCall>,
    /// Whether the snapshot's serialized bytes changed.
    pub changed: bool,
    /// Whether the diagnostic beside the snapshot changed.
    pub reader_diagnostic_changed: bool,
    /// More reader inputs differed after the bounded propagation rounds.
    pub reader_rounds_limited: bool,
    /// A program-visible mutation result changed under a reserved snapshot list.
    pub(crate) hidden_change: bool,
}

/// A refused query command. The previous snapshot and pending calls survive.
#[derive(Debug)]
#[non_exhaustive]
pub enum QueryCommandError {
    /// The identity belongs to another manager or was already completed.
    InvalidCall,
    /// Invalid refresh floor, backwards clock, or unrepresentable deadline.
    InvalidClock,
    /// Evaluation exceeded a configured limit, or an outcome's failure text
    /// or its snapshot did not fit. A result past a limit is not refused: it
    /// is delivered as the call's `tool-error` ([`ResultLimit`]).
    Limit(LimitError),
    /// Evaluation or tool-name coercion failed.
    Evaluation(EvaluationFailure),
}

impl QueryUpdate {
    /// Whether to redraw the tree, including reads of reserved statement names.
    pub fn needs_redraw(&self) -> bool {
        self.changed || self.hidden_change || self.reader_diagnostic_changed
    }
}

impl fmt::Display for QueryCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidClock => f.write_str("invalid refresh minimum or clock time"),
            Self::InvalidCall => f.write_str("invalid tool call identity"),
            Self::Limit(error) => error.fmt(f),
            Self::Evaluation(EvaluationFailure::MissingContext { recovery }) => write!(
                f,
                "query readers need their registered context; use {recovery}"
            ),
            Self::Evaluation(error) => write!(f, "query evaluation failed: {error:?}"),
        }
    }
}
impl std::error::Error for QueryCommandError {}

/// Evaluates query statements and tracks their tool calls, results and errors.
/// The embedder owns the Store, performs wanted calls and reports outcomes.
pub struct QueryManager {
    reader_limit: Option<LimitError>,
    readers: Option<ReaderPlan>,
    last_result: u64,
    reader_rounds_limited: bool,
    owner: Arc<()>,
    limits: Limits,
    lifecycle: Arc<()>,
    clock: refresh::Clock,
    disposed: bool,
    mutations: Mutations,
    queries: HashMap<String, Query>,
    order: Vec<String>,
    cache: HashMap<Arc<str>, CacheEntry>,
    // The embedder's extra cache-key values by tool name.
    extra_keys: HashMap<String, ExtraKey>,
    snapshot: JsObject<Value>,
    snapshot_json: String,
    // The cache cap and the bytes the live entries' data charge to it (#90).
    cache_limit: usize,
    cache_bytes: usize,
}

#[derive(Clone)]
struct ExtraKey {
    // Literal expressions in the embedder's order.
    fields: Vec<(String, Ast)>,
    // This tool's `"name":{...}` member of all the values as one JSON object.
    bytes: usize,
}

#[derive(Clone)]
struct Query {
    suspended: bool,
    waiting_for_other: bool,
    waiting_for_self: bool,
    reads: Arc<[Read]>,
    key: Arc<str>,
    previous: Option<Arc<str>>,
    defaults: Option<Value>,
    // Set only by a success delivered while this query wants the key.
    ever_fetched: bool,
    error: Option<Value>,
    // The error is not listed: the snapshot shows one compact diagnostic (#90).
    withheld: bool,
    // What the last evaluation would call; a run starts this call.
    request: Arc<runs::Request>,
}

impl Query {
    fn waiting(&self) -> bool {
        self.waiting_for_self || self.waiting_for_other
    }
}

#[derive(Default, Clone)]
struct CacheEntry {
    // Sequence number of the last success delivered to this entry.
    result_version: u64,
    data: Option<Arc<Value>>,
    pending: Option<Arc<PendingCall>>,
    // The data's JSON bytes, charged to the cache cap.
    bytes: usize,
    // Each current or previous slot holds one reference, including shared keys.
    references: usize,
    failed: bool,
    // A run asked for while a call is in flight: the request of the query
    // that asked first, so the call is the one asked for even if that query
    // has since gone.
    run_again: Option<Arc<runs::Request>>,
}

struct PendingCall {
    key: Arc<str>,
    tool_name: String,
    raw_tool_name: Option<Value>,
}

struct Node {
    id: String,
    reads: Arc<[Read]>,
    interval: Option<refresh::Interval>,
    key: Arc<str>,
    defaults: Option<Value>,
    request: Arc<runs::Request>,
}

impl Default for QueryManager {
    fn default() -> Self {
        Self::with_limits(Limits::DEFAULT)
    }
}

impl QueryManager {
    /// Create an empty manager using the given evaluation limits.
    pub fn with_limits(limits: Limits) -> Self {
        let mut manager = Self {
            reader_limit: None,
            readers: None,
            last_result: 0,
            reader_rounds_limited: false,
            owner: Arc::new(()),
            limits,
            lifecycle: Arc::new(()),
            clock: refresh::Clock::default(),
            disposed: false,
            mutations: Mutations::default(),
            queries: HashMap::new(),
            order: Vec::new(),
            cache: HashMap::new(),
            extra_keys: HashMap::new(),
            snapshot: JsObject::new(),
            snapshot_json: String::new(),
            cache_limit: limits.result_size(),
            cache_bytes: 0,
        };
        manager.rebuild_snapshot();
        manager
    }

    /// Conformance-test hook reporting the number of live entries. Outside the supported interface.
    #[doc(hidden)]
    pub fn cache_entry_count(&self) -> usize {
        self.cache.len()
    }

    /// Checks a current entry against the error and outstanding calls observed by
    /// a conformance-test driver. This hook is outside the supported interface.
    #[doc(hidden)]
    pub fn current_entry_is_served<'a>(
        &self,
        id: &str,
        visible_error: bool,
        mut outstanding: impl Iterator<Item = &'a ToolCallId>,
    ) -> bool {
        if let Some(q) = self.queries.get(id) {
            if q.suspended {
                return self.explains_suspension(id, q);
            }
            if q.waiting() {
                return self.audit_cache(outstanding, true).is_ok();
            }
        }
        let withheld = self.queries.get(id).is_some_and(|q| q.withheld);
        self.queries
            .get(id)
            .and_then(|q| self.cache.get(&q.key))
            .is_some_and(|entry| {
                entry.data.is_some()
                    || (entry.failed && (visible_error || withheld))
                    || entry.pending.as_ref().is_some_and(|pending| {
                        let target = Arc::downgrade(pending);
                        outstanding.any(|id| matches!(&id.0.target, CallTarget::Query(call) if call.ptr_eq(&target)))
                    })
            })
    }

    /// Set the values the embedder adds to the cache key of every query whose
    /// tool name, coerced to a JavaScript string, is `tool_name`. This is
    /// upstream's `extraDeps`: from the next evaluate the values join the
    /// state variables the query lists as dependencies, replacing one of the
    /// same name, so a change refetches. A later value for a repeated name
    /// wins. A `__proto__` name is skipped, as `Object.assign` skips it. No
    /// values remove the tool name's entry; a key with no dependencies left
    /// has none, as upstream omits an empty dependency record.
    ///
    /// Each value must pass the depth limit as a tool result does. All tool
    /// names' values, written as one JSON object of objects keyed by tool
    /// name, must fit the result-size limit. Both are checked before anything
    /// is copied, and a refused call keeps the previous values.
    pub fn set_extra_key_values<'a>(
        &mut self,
        tool_name: &str,
        values: impl IntoIterator<Item = (&'a str, &'a JsonValue)>,
    ) -> Result<(), QueryCommandError> {
        let limit = self.limits.result_size();
        // The other members, their separating commas and the outer braces.
        let (mut used, others) = self
            .extra_keys
            .iter()
            .filter(|(name, _)| name.as_str() != tool_name)
            .fold((2usize, 0usize), |(bytes, count), (_, extra)| {
                (bytes.saturating_add(extra.bytes), count + 1)
            });
        used = used.saturating_add(others);
        // This member: `"tool":{` and `}`.
        let mut bytes = openui_lang::json_string_len(tool_name).saturating_add(5);
        let mut admitted = Vec::new();
        for (name, value) in values {
            if name == "__proto__" {
                continue;
            }
            // A separating comma, then `"name":`.
            let field = openui_lang::json_string_len(name)
                .saturating_add(3)
                .saturating_add(usize::from(!admitted.is_empty()));
            bytes = bytes.saturating_add(field);
            let room = limit.saturating_sub(used.saturating_add(bytes));
            if used.saturating_add(bytes) > limit {
                return Err(QueryCommandError::Limit(crate::admission::size_error()));
            }
            bytes = bytes.saturating_add(host_json_bytes(value, self.limits, room)?);
            admitted.push((name, value));
        }
        if admitted.is_empty() {
            self.extra_keys.remove(tool_name);
            return Ok(());
        }
        // A repeated name is evaluated as a repeated object literal key.
        let fields = admitted
            .into_iter()
            .map(|(name, value)| (name.to_owned(), json_ast(value)))
            .collect();
        self.extra_keys
            .insert(tool_name.to_owned(), ExtraKey { fields, bytes });
        Ok(())
    }

    /// The most recent observable snapshot, including the three status lists.
    pub fn snapshot(&self) -> &JsObject<Value> {
        &self.snapshot
    }

    /// The snapshot in the reference's JSON property order and number format.
    pub fn snapshot_json(&self) -> &str {
        &self.snapshot_json
    }

    /// Apply a command. An outcome for a released cache entry is discarded.
    /// Completing an admitted result consumes its receipt even when evaluating
    /// dependent readers hits a limit. Inspect [`Self::reader_diagnostic`]
    /// for that cut, as with [`Self::complete_tool_call`].
    pub fn apply(&mut self, command: QueryCommand<'_>) -> Result<QueryUpdate, QueryCommandError> {
        let calls = match command {
            QueryCommand::Evaluate { program, store } => {
                return self
                    .evaluate_transaction(&TreeInput::new(program), store, |_| Ok(()))
                    .map(|(_, update)| update);
            }
            QueryCommand::Complete { id, outcome } => {
                return self.stage_borrowed(id, outcome)?.finish(None);
            }
            QueryCommand::Dispose => {
                let previous = self.mutations.hidden_results();
                self.stop();
                return Ok(QueryUpdate {
                    changed: self.rebuild_snapshot(),
                    hidden_change: self.mutations.hidden_changed(&previous),
                    ..self.idle_update()
                });
            }
            QueryCommand::Activate => {
                self.disposed = false;
                return Ok(self.idle_update());
            }
            QueryCommand::AdvanceClock { now_ms } => self.advance(now_ms)?,
            QueryCommand::Invalidate { .. } if self.disposed => return Ok(self.idle_update()),
            QueryCommand::Invalidate { statement_ids } => self.invalidate(statement_ids),
        };
        if calls.is_empty() {
            return Ok(self.idle_update());
        }
        Ok(QueryUpdate {
            calls,
            changed: self.rebuild_snapshot(),
            ..self.idle_update()
        })
    }

    fn evaluate_nodes(
        &self,
        input: &TreeInput<'_>,
        store: &Store,
        budget: &EvaluationSession<'_>,
        measured: &PreparedProgram,
    ) -> Result<Vec<Arc<Node>>, QueryCommandError> {
        self.with_statement_evaluator(input, store, budget, measured, |scope, eval| {
            let mut nodes = Vec::new();
            for statement in input.result.query_statements.iter().filter(|s| s.complete) {
                eval.charge_work(statement.statement_id.len().saturating_add(1));
                check_cuts(eval.flags())?;
                nodes.push(Arc::new(scope.node(statement, eval)?));
            }
            Ok(nodes)
        })
    }

    fn with_statement_evaluator<T>(
        &self,
        input: &TreeInput<'_>,
        store: &Store,
        budget: &EvaluationSession<'_>,
        measured: &PreparedProgram,
        evaluate: impl FnOnce(
            &QueryScope<'_, '_>,
            &ExpressionEvaluator<'_>,
        ) -> Result<T, QueryCommandError>,
    ) -> Result<T, QueryCommandError> {
        let prepared = self.statement_results()?;
        let calls = TreeScope::new(input, store, None, self.limits);
        let scope = QueryScope {
            manager: self,
            cloud_scripts: input.cloud_scripts,
            prepared: &prepared,
            calls: &calls,
            cascade: None,
            inspection: Cell::new(Inspection::None),
            inspected: Cell::new(None),
            seconds: Cell::new(0.0),
        };
        budget.with_scope(&scope, |session| {
            session.with_evaluator(|eval| {
                eval.supply_input(openui_lang::SuppliedInput {
                    program: openui_lang::SuppliedProgram::Prepared(measured),
                    start: openui_lang::SuppliedStart::Raw {
                        action: None,
                        message: "",
                        form: None,
                    },
                    references: Some(&prepared.input),
                })
                .map_err(|_| QueryCommandError::Limit(crate::admission::size_error()))?;
                evaluate(&scope, eval)
            })
        })
    }

    fn statement_results(&self) -> Result<PreparedReferenceValues<'static>, QueryCommandError> {
        self.admit_snapshot_values()?;
        let mut results = JsObject::new();
        for id in &self.order {
            if let Some(data) = self.queries.get(id).and_then(|query| self.data(query)) {
                results.define(id, data.clone());
            }
        }
        self.mutations.write_results(&mut results);
        #[cfg(test)]
        round_tests::reference_pass();
        Ok(PreparedReferenceValues::owned_with_limits(
            results,
            Limits::DEFAULT.with_depth(32_768),
        ))
    }

    fn apply_nodes(&mut self, program: &ParseResult, nodes: &[Arc<Node>]) -> Vec<ToolCall> {
        let active = program
            .query_statements
            .iter()
            .map(|statement| statement.statement_id.as_str())
            .collect::<HashSet<_>>();
        self.order.retain(|id| active.contains(id.as_str()));
        self.queries.retain(|id, query| {
            if active.contains(id.as_str()) {
                return true;
            }
            drop_reference(&mut self.cache, &query.key);
            if let Some(key) = &query.previous {
                drop_reference(&mut self.cache, key);
            }
            false
        });
        let mut calls = Vec::new();
        let mut intervals = Vec::new();
        for node in nodes {
            self.apply_node(node, &mut calls);
            if !self.waits(&node.id) {
                intervals.push((node.id.as_str(), node.interval.clone()));
            }
        }
        self.clock.reconcile(&active, intervals);
        calls
    }

    fn apply_node(&mut self, node: &Node, calls: &mut Vec<ToolCall>) {
        #[cfg(test)]
        round_tests::applied();
        if self.queries.get(&node.id).is_some_and(|q| q.suspended) {
            return;
        }
        let missing_input = node
            .reads
            .iter()
            .any(|read| read.id != node.id && self.version(&read.id).is_none());
        let mut acquire = false;
        let query = self.queries.entry(node.id.clone()).or_insert_with(|| {
            acquire = true;
            self.order.push(node.id.clone());
            Query {
                suspended: false,
                waiting_for_other: false,
                waiting_for_self: false,
                reads: node.reads.clone(),
                key: node.key.clone(),
                previous: None,
                defaults: None,
                ever_fetched: false,
                error: None,
                withheld: false,
                request: node.request.clone(),
            }
        });
        if query.key != node.key {
            acquire = true;
            let previous = std::mem::replace(&mut query.key, node.key.clone());
            if let Some(displaced) = query.previous.replace(previous) {
                drop_reference(&mut self.cache, &displaced);
            }
        }
        // Upstream tests self-reads against the newly installed key.
        query.waiting_for_other = missing_input;
        query.waiting_for_self = node.reads.iter().any(|read| read.id == node.id)
            && std::iter::once(&query.key)
                .chain(query.previous.as_ref())
                .all(|key| self.cache.get(key).is_none_or(|e| e.data.is_none()));
        query.reads = node.reads.clone();
        query.defaults = node.defaults.clone();
        query.request = node.request.clone();
        let entry = self.cache.entry(node.key.clone());
        let key = entry.key().clone();
        let entry = entry.or_default();
        if acquire {
            entry.references = entry.references.saturating_add(1);
        }
        // A failed entry suppresses another call while this query can see an
        // error. A new reader otherwise has neither data nor an explanation.
        if !query.waiting()
            && entry.data.is_none()
            && (!entry.failed || query.error.is_none())
            && entry.pending.is_none()
        {
            calls.push(runs::start_call(
                &self.owner,
                &self.lifecycle,
                entry,
                key,
                &query.request,
            ));
        }
    }

    // A success is admitted under the limits before it is copied. A refused
    // one is delivered as the call's failure and reported (#90).
    fn complete(
        &mut self,
        id: &ToolCallId,
        outcome: ToolOutcome<'_>,
    ) -> Result<(bool, Option<ResultLimit>), QueryCommandError> {
        self.check_call(id)?;
        if self.disposed {
            return Ok((false, None));
        }
        let target = match &id.0.target {
            CallTarget::Mutation(call) => {
                let room = self.mutation_room();
                return self.mutations.complete(call, outcome, self.limits, room);
            }
            CallTarget::Query(target) => target,
        };
        // Only the originating entry owns this target. A host holding an old
        // identity cannot retain the key, its data, or a replacement's call.
        let Some(pending) = target
            .upgrade()
            .filter(|pending| self.cache.contains_key(&pending.key))
        else {
            return Ok((false, None));
        };
        // A previous key keeps existing data alive. Only current readers can
        // accept new data or a failure. Leave an unsettled entry reusable.
        if !self
            .queries
            .values()
            .any(|query| !query.suspended && query.key == pending.key)
        {
            if let Some(entry) = self.cache.get_mut(&pending.key) {
                entry.pending = None;
                // A remembered run needs a current reader to deliver to.
                entry.run_again = None;
            }
            return Ok((false, None));
        }
        let admission = self.admit_query_result(&pending.key, &outcome)?;
        let refused = match &admission {
            Some(Admitted::Refused(limit)) => Some(*limit),
            _ => None,
        };
        let message = refused.map(|limit| limit.to_string());
        let outcome = match &message {
            Some(message) => ToolOutcome::CallFailed(message),
            None => outcome,
        };
        let success = matches!(admission, Some(Admitted::Data(..)));
        let Some(entry) = self.cache.get_mut(&pending.key) else {
            return Ok((false, None));
        };
        if let Some(Admitted::Data(value, bytes)) = admission {
            self.cache_bytes = self
                .cache_bytes
                .saturating_sub(entry.bytes)
                .saturating_add(bytes);
            entry.data = Some(Arc::new(value));
            entry.bytes = bytes;
            entry.failed = false;
        } else {
            entry.failed = true;
        }
        if success {
            self.last_result = self
                .last_result
                .checked_add(1)
                .ok_or_else(|| QueryCommandError::Limit(crate::admission::size_error()))?;
            entry.result_version = self.last_result;
        }
        entry.pending = None;
        let readers = self
            .queries
            .iter_mut()
            .filter(|(_, query)| !query.suspended && query.key == pending.key);
        for (statement_id, query) in readers {
            query.withheld = false;
            if success {
                query.error = None;
                query.ever_fetched = true;
                query.waiting_for_self = false;
                if let Some(previous) = query.previous.take() {
                    drop_reference(&mut self.cache, &previous);
                }
            } else {
                query.error = Some(pending.error(statement_id, &outcome));
            }
        }
        Ok((true, refused))
    }

    fn data(&self, query: &Query) -> Option<&Value> {
        self.data_entry(query)?.data.as_deref()
    }

    fn rebuild_snapshot(&mut self) -> bool {
        let pending = self.pending_queries();
        let mut loading = Vec::new();
        let mut refetching = Vec::new();
        let mut errors = Vec::new();
        for id in &self.order {
            let Some(query) = self.queries.get(id) else {
                continue;
            };
            if pending.contains(id.as_str()) {
                loading.push(Value::Str(id.clone()));
                if query.ever_fetched {
                    refetching.push(Value::Str(id.clone()));
                }
            }
            if let Some(error) = query.error.as_ref().filter(|_| !query.withheld) {
                errors.push(error.clone());
            }
        }
        errors.extend(self.withheld_diagnostic());
        self.mutations.append_errors(&mut errors);
        let mut snapshot = JsObject::new();
        snapshot.define("__openui_loading", Value::Arr(Arc::new(loading)));
        snapshot.define("__openui_refetching", Value::Arr(Arc::new(refetching)));
        snapshot.define("__openui_errors", Value::Arr(Arc::new(errors)));
        for id in &self.order {
            if snapshot.get(id).is_none()
                && let Some(query) = self.queries.get(id)
                && let Some(result) = self.data(query).or(query.defaults.as_ref())
            {
                snapshot.set(id, result.clone());
            }
        }
        self.mutations.write_snapshot(&mut snapshot);
        let snapshot = Arc::new(snapshot);
        let json = crate::tree::value_json(&Value::Obj(snapshot.clone()));
        if json == self.snapshot_json {
            return false;
        }
        self.snapshot = Arc::unwrap_or_clone(snapshot);
        self.snapshot_json = json;
        true
    }
}

fn drop_reference(cache: &mut HashMap<Arc<str>, CacheEntry>, key: &Arc<str>) {
    if let Some(entry) = cache.get_mut(key) {
        entry.references = entry.references.saturating_sub(1);
    }
}

impl ToolOutcome<'_> {
    // Bound formatted error strings before duplicating them at shared ids.
    fn admit_failure(
        &self,
        tool_bytes: usize,
        readers: usize,
        limits: Limits,
    ) -> Result<(), QueryCommandError> {
        let limit = limits.result_size().min(limits.work_for(usize::MAX));
        let text = match self {
            ToolOutcome::ToolNotFound(names) => {
                let mut bytes = 0usize;
                for name in names {
                    bytes = bytes.saturating_add(name.len()).saturating_add(2);
                    if bytes > limit {
                        return Err(QueryCommandError::Limit(crate::admission::size_error()));
                    }
                }
                bytes
            }
            ToolOutcome::McpError(text) | ToolOutcome::CallFailed(text) => text.len(),
            _ => return Ok(()),
        };
        if text != 0 && text.saturating_add(tool_bytes).saturating_mul(readers) > limit {
            return Err(QueryCommandError::Limit(crate::admission::size_error()));
        }
        Ok(())
    }
}

impl PendingCall {
    fn error(&self, id: &str, outcome: &ToolOutcome<'_>) -> Value {
        let tool = &self.tool_name;
        let (code, message, hint) = match outcome {
            ToolOutcome::ToolNotFound(available) => (
                "tool-not-found",
                format!("Query tool \"{tool}\" not found"),
                (!available.is_empty())
                    .then(|| format!("Available tools: {}", available.join(", "))),
            ),
            ToolOutcome::McpError(text) => (
                "mcp-error",
                format!("Query \"{tool}\" returned an error: {text}"),
                None,
            ),
            ToolOutcome::CallFailed(text) => (
                "tool-error",
                format!("Query \"{tool}\" failed: {text}"),
                None,
            ),
            ToolOutcome::Result(_) | ToolOutcome::ResultValue(_) => return Value::Null,
        };
        let mut error = JsObject::new();
        error.define("source", Value::Str("query".into()));
        error.define("statementId", Value::Str(id.into()));
        error.define("component", Value::Str("Query".into()));
        if let Some(name) = &self.raw_tool_name {
            error.define("toolName", name.clone());
        }
        error.define("code", Value::Str(code.into()));
        error.define("message", Value::Str(message));
        if let Some(hint) = hint {
            error.define("hint", Value::Str(hint));
        }
        Value::Obj(Arc::new(error))
    }
}

enum Inspection {
    None,
    Tool,
    Key,
    Interval,
}

struct QueryScope<'a, 'tree> {
    cloud_scripts: Option<&'a CloudScripts>,
    manager: &'a QueryManager,
    prepared: &'a PreparedReferenceValues<'a>,
    calls: &'a TreeScope<'tree>,
    cascade: Option<&'a cascade::RoundResults>,
    inspection: Cell<Inspection>,
    inspected: Cell<Option<String>>,
    seconds: Cell<f64>,
}

impl EvaluationScope for QueryScope<'_, '_> {
    fn state(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        self.calls.state(name, eval)
    }

    fn prepared_reference(&self, name: &str) -> Option<&PreparedReferenceValue> {
        self.read_reference(name).ok().flatten()
    }

    fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        if let Err(flags) = self.read_reference(name) {
            eval.mark_cut(flags);
            return None;
        }
        if let Some(query) = self.manager.queries.get(name) {
            return query.defaults.clone();
        }
        Some(Value::Null)
    }

    fn mapped_call_only(&self, name: &str) -> bool {
        self.calls.mapped_call_only(name)
    }

    fn wants_unmapped_call(&self, call: &Value, name: &str) -> bool {
        self.calls.wants_unmapped_call(call, name)
    }

    fn call_output(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> ExpressionOutput {
        self.calls.call_output(call, eval)
    }

    fn call_value_output(
        &self,
        call: &Value,
        name: &str,
        eval: &ExpressionEvaluator<'_>,
    ) -> ExpressionOutput {
        self.calls.call_value_output(call, name, eval)
    }

    // Query evaluation has no renderer prop context. Inspect the original
    // result for its tool name or key, preserving assign markers and @Each items.
    fn project_result<'v>(
        &self,
        value: ExpressionValue<'v>,
        _: &dyn Fn(&str) -> ExpressionValue<'v>,
    ) -> Option<ExpressionValue<'v>> {
        self.inspected
            .set(match self.inspection.replace(Inspection::None) {
                Inspection::None => None,
                Inspection::Interval => {
                    self.seconds.set(value.coerce_number());
                    None
                }
                Inspection::Tool => value.coerce_string(),
                Inspection::Key => value.stable_json(),
            });
        None
    }
}

impl QueryScope<'_, '_> {
    fn evaluate(
        &self,
        ast: &Ast,
        inspection: Inspection,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<Value>, QueryCommandError> {
        self.inspection.set(inspection);
        self.inspected.set(None);
        // evaluateStatements runs without the renderer's schema context.
        let result = eval.with_call_context(false, || {
            eval.with_projection(|eval| eval.evaluate_result(ast)).0
        });
        check_cuts(eval.flags())?;
        let Some(result) = result else {
            return Err(QueryCommandError::Evaluation(EvaluationFailure::Callback(
                "query evaluation unavailable".into(),
            )));
        };
        match result.failure {
            Some(error) => Err(QueryCommandError::Evaluation(error)),
            None => Ok(result.value),
        }
    }

    fn node(
        &self,
        statement: &QueryStatement,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Node, QueryCommandError> {
        let (tool_name, raw_tool_name) = match &statement.tool {
            Some(ast) => {
                let raw = self.evaluate(ast, Inspection::Tool, eval)?;
                (self.inspected.take().unwrap_or_default(), raw)
            }
            None => (String::new(), Some(Value::Str(String::new()))),
        };
        let revision = match self.cloud_scripts {
            Some(scripts) => scripts.dependency(raw_tool_name.as_ref(), eval)?,
            None => None,
        };
        let deps = self.dependencies(
            statement,
            self.manager.extra_keys.get(&tool_name),
            revision,
            eval,
        )?;
        let (args, args_key) = match &statement.args {
            Some(ast) => {
                let args = self.evaluate(ast, Inspection::Key, eval)?;
                (args, self.inspected.take())
            }
            None => (Some(Value::Null), Some("null".into())),
        };
        let args = match (args, &args_key) {
            (Some(Value::Null), _) | (None, Some(_)) => Some(Value::Obj(Arc::new(JsObject::new()))),
            (args, _) => args,
        };
        let defaults = match &statement.defaults {
            Some(ast) => self.evaluate(ast, Inspection::None, eval)?,
            None => Some(Value::Null),
        };
        let args_key = args_key.as_deref().unwrap_or("undefined");
        let key_len = tool_name
            .len()
            .saturating_add(2)
            .saturating_add(args_key.len());
        let key_len = deps.as_ref().map_or(key_len, |deps| {
            key_len.saturating_add(2).saturating_add(deps.len())
        });
        eval.charge_work(key_len);
        check_cuts(eval.flags())?;
        let mut key = String::with_capacity(key_len);
        key.push_str(&tool_name);
        key.push_str("::");
        key.push_str(args_key);
        if let Some(deps) = deps {
            key.push_str("::");
            key.push_str(&deps);
        }
        let interval = if let Some(ast) = &statement.refresh {
            let raw = self.evaluate(ast, Inspection::Interval, eval)?;
            refresh::Interval::new(raw, self.seconds.take(), self.manager.clock.minimum_ms)
        } else {
            None
        };
        Ok(Node {
            interval,
            id: statement.statement_id.clone(),
            reads: self.manager.read_results(statement, eval)?,
            key: key.into(),
            defaults,
            request: Arc::new(runs::Request {
                tool_name,
                raw_tool_name,
                args,
            }),
        })
    }

    fn dependencies(
        &self,
        statement: &QueryStatement,
        extra: Option<&ExtraKey>,
        revision: Option<(String, Ast)>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<String>, QueryCommandError> {
        let deps = statement.deps.as_deref().unwrap_or_default();
        let extra = extra.map_or(&[][..], |extra| &extra.fields);
        // Upstream omits a dependency record with no keys from the cache key.
        if deps.is_empty() && extra.is_empty() && revision.is_none() {
            return Ok(None);
        }
        // Read the dependency record through the same scope and allowance as
        // the arguments, preserving an absent state's undefined cache marker.
        let mut fields = Vec::new();
        for name in deps {
            if let (Some(key), Some(name)) = (eval.copy_text(name), eval.copy_text(name)) {
                fields.push((key, Ast::StateRef(name)));
            }
            check_cuts(eval.flags())?;
        }
        // Later keys win, as in Object.assign over the listed variables.
        fields.extend(extra.iter().cloned());
        fields.extend(revision);
        self.evaluate(&Ast::Obj(Arc::new(fields)), Inspection::Key, eval)?;
        Ok(self.inspected.take())
    }
}

fn check_cuts(flags: EvaluationFlags) -> Result<(), QueryCommandError> {
    if flags.size_limited || flags.depth_limited {
        Err(QueryCommandError::Limit(LimitError { flags }))
    } else {
        Ok(())
    }
}

/// The length of `JSON.stringify` of host JSON, refused past the depth limit
/// or past `budget` bytes. Walks without copying or recursion, and stops at
/// the first byte over.
fn host_json_bytes(
    value: &JsonValue,
    limits: Limits,
    budget: usize,
) -> Result<usize, QueryCommandError> {
    type Level<'a> = Box<dyn Iterator<Item = (Option<&'a String>, &'a JsonValue)> + 'a>;
    let mut bytes = 0usize;
    let mut path: Vec<Level<'_>> = vec![Box::new(std::iter::once((None, value)))];
    while let Some(level) = path.last_mut() {
        let Some((name, value)) = level.next() else {
            path.pop();
            continue;
        };
        let room = budget.saturating_sub(bytes);
        if let Some(name) = name {
            let written = results::string_bytes(name, room)
                .ok_or(QueryCommandError::Limit(crate::admission::size_error()))?;
            bytes = bytes.saturating_add(written.saturating_add(1));
        }
        let room = budget.saturating_sub(bytes);
        let (own, children): (usize, Option<Level<'_>>) = match value {
            JsonValue::Null | JsonValue::Bool(true) => (4, None),
            JsonValue::Bool(false) => (5, None),
            JsonValue::Number(number) => (
                openui_lang::json_number_len(number.as_f64().unwrap_or(f64::NAN)),
                None,
            ),
            JsonValue::String(text) => (
                results::string_bytes(text, room)
                    .ok_or(QueryCommandError::Limit(crate::admission::size_error()))?,
                None,
            ),
            JsonValue::Array(items) => (
                items.len().saturating_add(1).max(2),
                Some(Box::new(items.iter().map(|item| (None, item)))),
            ),
            JsonValue::Object(fields) => (
                fields.len().saturating_add(1).max(2),
                Some(Box::new(
                    fields.iter().map(|(name, item)| (Some(name), item)),
                )),
            ),
        };
        bytes = bytes.saturating_add(own);
        if bytes > budget {
            return Err(QueryCommandError::Limit(crate::admission::size_error()));
        }
        if let Some(children) = children {
            if path.len() > limits.depth() {
                return Err(QueryCommandError::Limit(crate::admission::depth_error()));
            }
            path.push(children);
        }
    }
    Ok(bytes)
}

// The embedder's JSON as literal expressions, so the cache key serializes it
// exactly as it serializes state. Iterative, as admitted JSON can be deep.
fn json_ast(root: &JsonValue) -> Ast {
    enum Level<'a> {
        Array(std::slice::Iter<'a, JsonValue>, Vec<Ast>),
        Object(
            Box<dyn Iterator<Item = (&'a String, &'a JsonValue)> + 'a>,
            Vec<(String, Ast)>,
            String,
        ),
    }
    let mut levels: Vec<Level<'_>> = Vec::new();
    let mut next = Some(root);
    loop {
        let mut done = match next.take() {
            Some(JsonValue::Array(items)) => {
                levels.push(Level::Array(items.iter(), Vec::with_capacity(items.len())));
                None
            }
            Some(JsonValue::Object(fields)) => {
                levels.push(Level::Object(
                    Box::new(fields.iter()),
                    Vec::new(),
                    String::new(),
                ));
                None
            }
            Some(JsonValue::Null) | None => Some(Ast::Null),
            Some(JsonValue::Bool(value)) => Some(Ast::Bool(*value)),
            Some(JsonValue::Number(number)) => Some(Ast::Num(number.as_f64().unwrap_or(f64::NAN))),
            Some(JsonValue::String(text)) => Some(Ast::Str(text.clone())),
        };
        loop {
            let Some(level) = levels.last_mut() else {
                return done.unwrap_or(Ast::Null);
            };
            match level {
                Level::Array(items, out) => {
                    out.extend(done.take());
                    if let Some(item) = items.next() {
                        next = Some(item);
                        break;
                    }
                }
                Level::Object(fields, out, name) => {
                    if let Some(ast) = done.take() {
                        out.push((std::mem::take(name), ast));
                    }
                    if let Some((key, value)) = fields.next() {
                        *name = key.clone();
                        next = Some(value);
                        break;
                    }
                }
            }
            done = match levels.pop() {
                Some(Level::Array(_, out)) => Some(Ast::Arr(Arc::new(out))),
                Some(Level::Object(_, out, _)) => Some(Ast::Obj(Arc::new(out))),
                None => None,
            };
        }
    }
}
