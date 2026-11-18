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
    key: Arc<str>,
    tool_name: String,
    raw_tool_name: Option<Value>,
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
    snapshot: JsObject<Value>,
    snapshot_json: String,
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
    pending: Option<ToolCallId>,
    failed: bool,
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
            snapshot: JsObject::new(),
            snapshot_json: String::new(),
        };
        manager.rebuild_snapshot();
        manager
    }

    /// The most recent observable snapshot, including the three status lists.
    pub fn snapshot(&self) -> &JsObject<Value> {
        &self.snapshot
    }

    /// The snapshot in the reference's JSON property order and number format.
    pub fn snapshot_json(&self) -> &str {
        &self.snapshot_json
    }

    /// Apply a command. A late outcome for a key no query wants is discarded.
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
        let mut released = Vec::new();
        self.queries.retain(|id, query| {
            if active.contains(id.as_str()) {
                return true;
            }
            released.push(query.key.clone());
            released.extend(query.previous.take());
            false
        });
        self.release_unused(released);
        let mut calls = Vec::new();
        for node in nodes {
            let query = self.queries.entry(node.id.into()).or_insert_with(|| {
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
                query.previous = Some(std::mem::replace(&mut query.key, node.key.clone()));
            }
            query.defaults = node.defaults;
            let entry = self.cache.entry(node.key);
            let key = entry.key().clone();
            let entry = entry.or_default();
            if entry.data.is_none() && !entry.failed && entry.pending.is_none() {
                let id = ToolCallId(Arc::new(CallIdentity {
                    manager: Arc::downgrade(&self.owner),
                    key,
                    tool_name: node.tool_name.clone(),
                    raw_tool_name: node.raw_tool_name,
                    consumed: AtomicBool::new(false),
                }));
                entry.pending = Some(id.clone());
                calls.push(ToolCall {
                    id,
                    tool_name: node.tool_name,
                    args: node.args,
                });
            }
        }
        calls
    }

    // Upstream checks the current and previous keys on removal, and releases
    // the previous key on success. A second edit can displace a previous key
    // before either event; a later visit can still reuse that cache entry.
    fn release_unused(&mut self, released: Vec<Arc<str>>) {
        if released.is_empty() {
            return;
        }
        let referenced = self
            .queries
            .values()
            .flat_map(|query| std::iter::once(&query.key).chain(query.previous.as_ref()))
            .collect::<HashSet<_>>();
        for key in released {
            if !referenced.contains(&key) {
                self.cache.remove(&key);
            }
        }
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
        let key = &id.0.key;
        let wanted = self.queries.values().any(|query| &query.key == key);
        let entry = self.cache.get_mut(key).filter(|entry| {
            entry
                .pending
                .as_ref()
                .is_some_and(|pending| Arc::ptr_eq(&pending.0, &id.0))
        });
        // The result belongs to the entry in which the call started. A key
        // can be removed and recreated before the host reports the outcome.
        // The old call can still notify a query that wants that key, but it
        // cannot write data into the new entry or settle its pending call.
        if let Some(entry) = entry {
            if wanted {
                if let ToolOutcome::Result(value) = &outcome {
                    entry.data = Some(match value {
                        Some(value) => admit_result(value, self.limits)?,
                        None => Value::Null,
                    });
                    entry.failed = false;
                } else {
                    entry.failed = true;
                }
            }
            entry.pending = None;
            // A discarded first outcome leaves no reusable state.
            if !wanted && entry.data.is_none() && !entry.failed {
                self.cache.remove(key);
            }
        }
        id.0.consumed.store(true, Ordering::Relaxed);
        if !wanted {
            return Ok(false);
        }
        let mut released = Vec::new();
        for (statement_id, query) in &mut self.queries {
            if &query.key != key {
                continue;
            }
            if matches!(outcome, ToolOutcome::Result(_)) {
                query.error = None;
                query.ever_fetched = true;
                released.extend(query.previous.take());
            } else {
                query.error = Some(id.0.error(statement_id, &outcome));
            }
        }
        self.release_unused(released);
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
            if let Some(query) = self.queries.get(id) {
                if let Some(result) = self.data(query).or(query.defaults.as_ref()) {
                    snapshot.set(id, result.clone());
                } else {
                    snapshot.remove(id);
                }
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

impl CallIdentity {
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
        let deps = self.dependencies(statement, eval)?;
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
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Option<String>, QueryCommandError> {
        let Some(deps) = statement.deps.as_ref().filter(|deps| !deps.is_empty()) else {
            return Ok(None);
        };
        // Read the dependency record through the same scope and allowance as
        // the arguments, preserving an absent state's undefined cache marker.
        let mut fields = Vec::new();
        for name in deps {
            if let (Some(key), Some(name)) = (eval.copy_text(name), eval.copy_text(name)) {
                fields.push((key, Ast::StateRef(name)));
            }
            check_cuts(eval.flags())?;
        }
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
    let mut path: Vec<Box<dyn Iterator<Item = &JsonValue>>> =
        vec![Box::new(std::iter::once(value))];
    while let Some(level) = path.last_mut() {
        let Some(value) = level.next() else {
            path.pop();
            continue;
        };
        let children: Box<dyn Iterator<Item = &JsonValue>> = match value {
            JsonValue::Array(items) => Box::new(items.iter()),
            JsonValue::Object(fields) => Box::new(fields.values()),
            _ => continue,
        };
        if path.len() > limits.depth() {
            let mut flags = EvaluationFlags::default();
            flags.depth_limited = true;
            return Err(QueryCommandError::Limit(LimitError { flags }));
        }
        path.push(children);
    }
    Ok(Value::from_json(value))
}
