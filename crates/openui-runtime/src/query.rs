//! The clock-free query manager and its tool provider seam.
use crate::tree::TreeScope;
use crate::{
    EvaluationFailure, EvaluationFlags, EvaluationScope, EvaluationSession, ExpressionEvaluator,
    ExpressionOutput, ExpressionValue, JsObject, LimitError, Limits, PreparedReferenceValue,
    PreparedReferenceValues, Store, TreeInput, Value,
};
use openui_lang::{Ast, JsonValue, ParseResult, PreparedProgram, QueryStatement};
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
    manager: Weak<()>,
    target: Weak<PendingCall>,
    consumed: AtomicBool,
}

/// A call the embedder should perform with its tool provider.
#[derive(Debug)]
#[non_exhaustive]
pub struct ToolCall {
    /// Pass this identity back with the outcome.
    pub id: ToolCallId,
    /// The evaluated tool name coerced to a JavaScript string.
    pub tool_name: String,
    /// Evaluated arguments. Nullish arguments become an empty object.
    /// A function at the root has no JSON projection.
    pub args: Option<Value>,
}

/// The outcome reported by the tool provider.
#[derive(Debug)]
#[non_exhaustive]
pub enum ToolOutcome<'a> {
    /// Host JSON, with a missing result treated as JSON null.
    Result(Option<&'a JsonValue>),
    /// The requested tool is missing. Names are in provider order.
    ToolNotFound(Vec<&'a str>),
    /// The tool reported an MCP error with this text.
    McpError(&'a str),
    /// The call failed with this message.
    ToolError(&'a str),
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
    /// Complete one wanted call.
    Complete {
        /// Identity returned when the call started.
        id: &'a ToolCallId,
        /// The tool provider's outcome.
        outcome: ToolOutcome<'a>,
    },
}

/// The effect of an accepted query command.
#[derive(Debug)]
#[non_exhaustive]
pub struct QueryUpdate {
    /// Newly wanted calls, in dispatch order.
    pub calls: Vec<ToolCall>,
    /// Whether the snapshot's serialized bytes changed.
    pub changed: bool,
}

/// A refused query command. The previous snapshot and pending calls survive.
#[derive(Debug)]
#[non_exhaustive]
pub enum QueryCommandError {
    /// The identity belongs to another manager or was already completed.
    InvalidCall,
    /// Evaluation or result depth exceeded a configured limit.
    Limit(LimitError),
    /// Evaluation or tool-name coercion failed.
    Evaluation(EvaluationFailure),
}

impl fmt::Display for QueryCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCall => f.write_str("invalid tool call identity"),
            Self::Limit(error) => error.fmt(f),
            Self::Evaluation(error) => write!(f, "query evaluation failed: {error:?}"),
        }
    }
}
impl std::error::Error for QueryCommandError {}

/// Evaluates query statements and tracks their tool calls, results and errors.
/// The embedder owns the Store, performs wanted calls and reports outcomes.
pub struct QueryManager {
    owner: Arc<()>,
    limits: Limits,
    queries: HashMap<String, Query>,
    order: Vec<String>,
    cache: HashMap<Arc<str>, CacheEntry>,
    // The embedder's extra cache-key values by tool name.
    extra_keys: HashMap<String, ExtraKey>,
    snapshot: JsObject<Value>,
    snapshot_json: String,
}

struct ExtraKey {
    // Literal expressions in the embedder's order.
    fields: Vec<(String, Ast)>,
    // This tool's `"name":{...}` member of all the values as one JSON object.
    bytes: usize,
}

struct Query {
    key: Arc<str>,
    previous: Option<Arc<str>>,
    defaults: Option<Value>,
    // Set only by a success delivered while this query wants the key.
    ever_fetched: bool,
    error: Option<Value>,
}

#[derive(Default)]
struct CacheEntry {
    data: Option<Value>,
    pending: Option<Arc<PendingCall>>,
    // Each current or previous slot holds one reference, including shared keys.
    references: usize,
    failed: bool,
}

struct PendingCall {
    key: Arc<str>,
    tool_name: String,
    raw_tool_name: Option<Value>,
}

struct Node<'a> {
    id: &'a str,
    key: Arc<str>,
    defaults: Option<Value>,
    tool_name: String,
    raw_tool_name: Option<Value>,
    args: Option<Value>,
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
            owner: Arc::new(()),
            limits,
            queries: HashMap::new(),
            order: Vec::new(),
            cache: HashMap::new(),
            extra_keys: HashMap::new(),
            snapshot: JsObject::new(),
            snapshot_json: String::new(),
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
        self.queries
            .get(id)
            .and_then(|q| self.cache.get(&q.key))
            .is_some_and(|entry| {
                entry.data.is_some()
                    || (entry.failed && visible_error)
                    || entry.pending.as_ref().is_some_and(|pending| {
                        let target = Arc::downgrade(pending);
                        outstanding.any(|id| id.0.target.ptr_eq(&target))
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
    pub fn apply(&mut self, command: QueryCommand<'_>) -> Result<QueryUpdate, QueryCommandError> {
        let calls = match command {
            QueryCommand::Evaluate { program, store } => {
                let nodes = self.evaluate_nodes(program, store)?;
                self.apply_nodes(program, nodes)
            }
            QueryCommand::Complete { id, outcome } => {
                if !self.complete(id, outcome)? {
                    return Ok(QueryUpdate {
                        calls: Vec::new(),
                        changed: false,
                    });
                }
                Vec::new()
            }
        };
        // Ownership changes atomically with the command. Another registration
        // can take a temporarily unreferenced entry later in the same evaluate.
        self.cache.retain(|_, entry| entry.references != 0);
        Ok(QueryUpdate {
            calls,
            changed: self.rebuild_snapshot(),
        })
    }

    fn evaluate_nodes<'a>(
        &self,
        program: &'a ParseResult,
        store: &Store,
    ) -> Result<Vec<Node<'a>>, QueryCommandError> {
        let program_input = PreparedProgram::new(program, self.limits);
        check_cuts(program_input.flags())?;
        let mut results = JsObject::new();
        for id in &self.order {
            if let Some(data) = self.queries.get(id).and_then(|query| self.data(query)) {
                results.define(id, data.clone());
            }
        }
        let prepared = PreparedReferenceValues::new(&results);
        let scope = QueryScope {
            manager: self,
            prepared: &prepared,
            calls: TreeScope::new(&TreeInput::new(program), store, None, self.limits),
            inspection: Cell::new(Inspection::None),
            inspected: Cell::new(None),
        };
        let bytes = program_input
            .input_bytes()
            .saturating_add(prepared.input_bytes());
        let session = EvaluationSession::with_limits(&scope, bytes, self.limits);
        session.with_evaluator(|eval| {
            let mut nodes = Vec::new();
            for statement in &program.query_statements {
                eval.charge_work(statement.statement_id.len().saturating_add(1));
                check_cuts(eval.flags())?;
                if statement.complete {
                    nodes.push(scope.node(statement, eval)?);
                }
            }
            Ok(nodes)
        })
    }

    fn apply_nodes(&mut self, program: &ParseResult, nodes: Vec<Node<'_>>) -> Vec<ToolCall> {
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
        for node in nodes {
            let mut acquire = false;
            let query = self.queries.entry(node.id.into()).or_insert_with(|| {
                acquire = true;
                self.order.push(node.id.into());
                Query {
                    key: node.key.clone(),
                    previous: None,
                    defaults: None,
                    ever_fetched: false,
                    error: None,
                }
            });
            if query.key != node.key {
                acquire = true;
                let previous = std::mem::replace(&mut query.key, node.key.clone());
                if let Some(displaced) = query.previous.replace(previous) {
                    drop_reference(&mut self.cache, &displaced);
                }
            }
            query.defaults = node.defaults;
            let entry = self.cache.entry(node.key);
            let key = entry.key().clone();
            let entry = entry.or_default();
            if acquire {
                entry.references = entry.references.saturating_add(1);
            }
            // A failed entry suppresses another call while this query can see an
            // error. A new reader otherwise has neither data nor an explanation.
            if entry.data.is_none()
                && (!entry.failed || query.error.is_none())
                && entry.pending.is_none()
            {
                let pending = Arc::new(PendingCall {
                    key,
                    tool_name: node.tool_name.clone(),
                    raw_tool_name: node.raw_tool_name,
                });
                let id = ToolCallId(Arc::new(CallIdentity {
                    manager: Arc::downgrade(&self.owner),
                    target: Arc::downgrade(&pending),
                    consumed: AtomicBool::new(false),
                }));
                entry.pending = Some(pending);
                calls.push(ToolCall {
                    id,
                    tool_name: node.tool_name,
                    args: node.args,
                });
            }
        }
        calls
    }

    fn complete(
        &mut self,
        id: &ToolCallId,
        outcome: ToolOutcome<'_>,
    ) -> Result<bool, QueryCommandError> {
        if !id.0.manager.ptr_eq(&Arc::downgrade(&self.owner))
            || id.0.consumed.load(Ordering::Relaxed)
        {
            return Err(QueryCommandError::InvalidCall);
        }
        // Only the originating entry owns this target. A host holding an old
        // identity cannot retain the key, its data, or a replacement's call.
        let active = id.0.target.upgrade().and_then(|pending| {
            self.cache
                .get_mut(&pending.key)
                .map(|entry| (pending, entry))
        });
        let Some((pending, entry)) = active else {
            id.0.consumed.store(true, Ordering::Relaxed);
            return Ok(false);
        };
        // A previous key keeps existing data alive. Only current readers can
        // accept new data or a failure. Leave an unsettled entry reusable.
        let mut readers = self
            .queries
            .iter_mut()
            .filter(|(_, query)| query.key == pending.key)
            .peekable();
        if readers.peek().is_none() {
            entry.pending = None;
            id.0.consumed.store(true, Ordering::Relaxed);
            return Ok(false);
        }
        if let ToolOutcome::Result(value) = &outcome {
            entry.data = Some(match value {
                Some(value) => admit_result(value, self.limits)?,
                None => Value::Null,
            });
            entry.failed = false;
        } else {
            entry.failed = true;
        }
        id.0.consumed.store(true, Ordering::Relaxed);
        entry.pending = None;
        for (statement_id, query) in readers {
            if matches!(outcome, ToolOutcome::Result(_)) {
                query.error = None;
                query.ever_fetched = true;
                if let Some(previous) = query.previous.take() {
                    drop_reference(&mut self.cache, &previous);
                }
            } else {
                query.error = Some(pending.error(statement_id, &outcome));
            }
        }
        Ok(true)
    }

    fn data(&self, query: &Query) -> Option<&Value> {
        self.cache
            .get(&query.key)
            .and_then(|entry| entry.data.as_ref())
            .or_else(|| self.cache.get(query.previous.as_ref()?)?.data.as_ref())
    }

    fn rebuild_snapshot(&mut self) -> bool {
        let mut loading = Vec::new();
        let mut refetching = Vec::new();
        let mut errors = Vec::new();
        for id in &self.order {
            let Some(query) = self.queries.get(id) else {
                continue;
            };
            if self
                .cache
                .get(&query.key)
                .is_some_and(|entry| entry.pending.is_some())
            {
                loading.push(Value::Str(id.clone()));
                if query.ever_fetched {
                    refetching.push(Value::Str(id.clone()));
                }
            }
            if let Some(error) = &query.error {
                errors.push(error.clone());
            }
        }
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
            ToolOutcome::ToolError(text) => (
                "tool-error",
                format!("Query \"{tool}\" failed: {text}"),
                None,
            ),
            ToolOutcome::Result(_) => return Value::Null,
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
}

struct QueryScope<'a> {
    manager: &'a QueryManager,
    prepared: &'a PreparedReferenceValues<'a>,
    calls: TreeScope<'a>,
    inspection: Cell<Inspection>,
    inspected: Cell<Option<String>>,
}

impl EvaluationScope for QueryScope<'_> {
    fn state(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        self.calls.state(name, eval)
    }

    fn prepared_reference(&self, name: &str) -> Option<&PreparedReferenceValue> {
        self.prepared
            .get(name)
            .ok()
            .flatten()
            .map(|(_, prepared)| prepared)
    }

    fn reference(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        if let Err(flags) = self.prepared.get(name) {
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
                Inspection::Tool => value.coerce_string(),
                Inspection::Key => value.stable_json(),
            });
        None
    }
}

impl QueryScope<'_> {
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

    fn node<'a>(
        &self,
        statement: &'a QueryStatement,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Node<'a>, QueryCommandError> {
        let (tool_name, raw_tool_name) = match &statement.tool {
            Some(ast) => {
                let raw = self.evaluate(ast, Inspection::Tool, eval)?;
                (self.inspected.take().unwrap_or_default(), raw)
            }
            None => (String::new(), Some(Value::Str(String::new()))),
        };
        let deps = self.dependencies(statement, self.manager.extra_keys.get(&tool_name), eval)?;
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
        Ok(Node {
            id: &statement.statement_id,
            key: key.into(),
            defaults,
            tool_name,
            raw_tool_name,
            args,
        })
    }

    fn dependencies(
        &self,
        statement: &QueryStatement,
        extra: Option<&ExtraKey>,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<String>, QueryCommandError> {
        let deps = statement.deps.as_deref().unwrap_or_default();
        let extra = extra.map_or(&[][..], |extra| &extra.fields);
        // Upstream omits a dependency record with no keys from the cache key.
        if deps.is_empty() && extra.is_empty() {
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

fn admit_result(value: &JsonValue, limits: Limits) -> Result<Value, QueryCommandError> {
    // The host owns its JSON. Check depth without cloning it, including for
    // caller-built JSON. Keep conversion at this boundary for result/cache
    // size admission, which will use the same entry point.
    host_json_bytes(value, limits, usize::MAX)?;
    Ok(Value::from_json(value))
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
        if let Some(name) = name {
            bytes = bytes.saturating_add(openui_lang::json_string_len(name).saturating_add(3));
        }
        let (own, children): (usize, Option<Level<'_>>) = match value {
            JsonValue::Null | JsonValue::Bool(true) => (4, None),
            JsonValue::Bool(false) => (5, None),
            JsonValue::Number(number) => (
                openui_lang::json_number_len(number.as_f64().unwrap_or(f64::NAN)),
                None,
            ),
            JsonValue::String(text) => (openui_lang::json_string_len(text).saturating_add(2), None),
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
