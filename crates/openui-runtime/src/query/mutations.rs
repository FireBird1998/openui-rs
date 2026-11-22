//! Mutation registrations have JavaScript Map lifetimes. A rename updates the
//! same entry, whereas removing and declaring the name again creates a new one.
use super::*;

#[derive(Clone, Default)]
pub(super) struct Mutations {
    entries: HashMap<String, Entry>,
    order: Vec<String>,
}

#[derive(Clone)]
struct Entry {
    identity: Arc<()>,
    tool: String,
    raw_tool: Option<Value>,
    result: Value,
    error: Option<Value>,
    // The JSON bytes of the result's data, charged to the cache cap (#90).
    bytes: usize,
}

impl Entry {
    fn accepts_submission(&self) -> bool {
        let Value::Obj(fields) = &self.result else {
            return false;
        };
        !matches!(fields.get("status"), Some(Value::Str(s)) if s == "loading")
    }
}

pub(super) struct MutationCall {
    statement: String,
    registration: Weak<()>,
    dispatched_tool: String,
}

impl MutationCall {
    fn belongs_to(&self, entry: &Entry) -> bool {
        self.registration.ptr_eq(&Arc::downgrade(&entry.identity))
    }
}

fn result(status: &str, data: Option<Value>, error: Option<Value>) -> Value {
    let mut fields = JsObject::new();
    fields.define("status", Value::Str(status.into()));
    if let Some(value) = data {
        fields.define("data", value);
    }
    if let Some(value) = error {
        fields.define("error", value);
    }
    Value::Obj(Arc::new(fields))
}

impl QueryManager {
    /// Register a program's mutation statements, evaluating only their tool names.
    /// An empty program removes the registrations.
    pub fn register_mutations(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
    ) -> Result<QueryUpdate, QueryCommandError> {
        let previous = self.mutations.hidden_results();
        self.register_mutation_statements(input, store)?;
        Ok(QueryUpdate {
            next_refresh_due_ms: self.next_refresh_due_ms(),
            calls: Vec::new(),
            changed: self.rebuild_snapshot(),
            hidden_change: self.mutations.hidden_changed(&previous),
            ..QueryUpdate::default()
        })
    }

    pub(super) fn register_mutation_statements(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
    ) -> Result<(), QueryCommandError> {
        let statements = &input.result.mutation_statements;
        if statements.is_empty() {
            self.mutations = Mutations::default();
            return Ok(());
        }
        let measured = PreparedProgram::new(input.result, self.limits);
        check_cuts(measured.flags())?;
        let budget = EvaluationSession::with_limits(&(), measured.input_bytes(), self.limits);
        let nodes =
            self.with_statement_evaluator(input, store, &budget, &measured, |scope, eval| {
                let mut nodes = Vec::new();
                for statement in statements {
                    eval.charge_work(statement.statement_id.len().saturating_add(1));
                    check_cuts(eval.flags())?;
                    let (tool, raw) = if let Some(ast) = &statement.tool {
                        let raw = scope.evaluate(ast, Inspection::Tool, eval)?;
                        (scope.inspected.take().unwrap_or_default(), raw)
                    } else {
                        (String::new(), Some(Value::Str(String::new())))
                    };
                    nodes.push((statement.statement_id.as_str(), tool, raw));
                }
                Ok(nodes)
            })?;
        let active: HashSet<_> = nodes.iter().map(|(id, _, _)| *id).collect();
        self.mutations
            .order
            .retain(|id| active.contains(id.as_str()));
        self.mutations
            .entries
            .retain(|id, _| active.contains(id.as_str()));
        for (id, tool, raw) in nodes {
            if let Some(entry) = self.mutations.entries.get_mut(id) {
                // JavaScript strict equality differs from Object.is for signed zero and NaN.
                let equal = match (&entry.raw_tool, &raw) {
                    (Some(Value::Num(a)), Some(Value::Num(b))) => a == b,
                    (a, b) => crate::store::same(a.as_ref(), b.as_ref()),
                };
                if !equal {
                    entry.tool = tool;
                    entry.raw_tool = raw;
                    entry.result = result("idle", Some(Value::Null), Some(Value::Null));
                    entry.error = None;
                    entry.bytes = 0;
                }
            } else {
                self.mutations.order.push(id.into());
                self.mutations.entries.insert(
                    id.into(),
                    Entry {
                        identity: Arc::new(()),
                        tool,
                        raw_tool: raw,
                        result: result("idle", None, None),
                        error: None,
                        bytes: 0,
                    },
                );
            }
        }
        Ok(())
    }

    /// Whether the registration exists and accepts a submission now.
    pub fn can_fire_mutation(&self, statement: &str) -> bool {
        !self.disposed
            && self
                .mutations
                .entries
                .get(statement)
                .is_some_and(Entry::accepts_submission)
    }

    /// Start a mutation with click-time arguments. Unknown or loading statements
    /// return an empty update. A dispatched call retains the registration identity.
    ///
    /// The manager keeps no receipt of a mutation call: the returned
    /// [`ToolCall`] owns its memory, its statement id, a copy of the tool name
    /// for the receipt and another for the call, and the arguments, which
    /// share the caller's value. A rename (a new tool name for the same
    /// statement) resets the registration to idle, so it accepts another
    /// submission while earlier calls are still out, and their receipts stay
    /// valid. A caller that keeps every call across renames therefore holds
    /// one of these per submission, and a manager used directly sets no
    /// bound on how many: each stays until the caller drops it, completing
    /// it does not free it, and once the registration is removed it keeps
    /// only an empty identity allocation alive. A managed session bounds
    /// them with its pending continuation limit, `max_continuations`.
    pub fn fire_mutation(
        &mut self,
        statement: &str,
        args: Option<&Value>,
    ) -> Result<QueryUpdate, QueryCommandError> {
        if self.disposed {
            return Ok(self.idle_update());
        }
        let Some(entry) = self
            .mutations
            .entries
            .get_mut(statement)
            .filter(|entry| entry.accepts_submission())
        else {
            return Ok(QueryUpdate {
                next_refresh_due_ms: self.next_refresh_due_ms(),
                calls: Vec::new(),
                changed: false,
                ..QueryUpdate::default()
            });
        };
        if let Some(value) = args {
            crate::admission::measure(value, self.limits).map_err(QueryCommandError::Limit)?;
        }
        let call = ToolCall {
            id: ToolCallId(Arc::new(CallIdentity {
                manager: Arc::downgrade(&self.owner),
                lifecycle: Arc::downgrade(&self.lifecycle),
                target: CallTarget::Mutation(MutationCall {
                    statement: statement.into(),
                    registration: Arc::downgrade(&entry.identity),
                    dispatched_tool: entry.tool.clone(),
                }),
                consumed: AtomicBool::new(false),
            })),
            tool_name: entry.tool.clone(),
            args: args.cloned(),
        };
        let hidden_change = HIDDEN_NAMES.contains(&statement);
        entry.result = result("loading", None, None);
        entry.bytes = 0;
        Ok(QueryUpdate {
            next_refresh_due_ms: self.next_refresh_due_ms(),
            calls: vec![call],
            hidden_change,
            changed: self.rebuild_snapshot(),
            ..QueryUpdate::default()
        })
    }
}

impl PartialEq for ToolCallId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ToolCallId {}

// Prototype assignment also has no own JSON property in the upstream snapshot.
const HIDDEN_NAMES: [&str; 4] = [
    "__openui_loading",
    "__openui_refetching",
    "__openui_errors",
    "__proto__",
];

impl Mutations {
    pub(super) fn hidden_results(&self) -> [Option<Value>; HIDDEN_NAMES.len()] {
        HIDDEN_NAMES.map(|id| self.entries.get(id).map(|m| m.result.clone()))
    }

    pub(super) fn hidden_changed(&self, previous: &[Option<Value>; HIDDEN_NAMES.len()]) -> bool {
        HIDDEN_NAMES.iter().zip(previous).any(|(id, before)| {
            let after = self.entries.get(*id).map(|m| &m.result);
            match (before.as_ref(), after) {
                (None, None) => false,
                (Some(a), Some(b)) => {
                    !crate::store::same(Some(a), Some(b))
                        && crate::tree::value_json(a) != crate::tree::value_json(b)
                }
                _ => true,
            }
        })
    }

    pub(super) fn pending_registration(&self, call: &MutationCall) -> Option<&str> {
        let (name, entry) = self.entries.get_key_value(&call.statement)?;
        (call.belongs_to(entry) && !entry.accepts_submission()).then_some(name.as_str())
    }

    pub(super) fn delivery_holders(&self, call: &MutationCall) -> (usize, usize) {
        // A rename keeps this registration, even if a newer call already settled.
        match self.entries.get(&call.statement) {
            Some(entry) if call.belongs_to(entry) => (1, call.dispatched_tool.len()),
            _ => (0, 0),
        }
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    fn ordered(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.order
            .iter()
            .filter_map(|id| Some((id, self.entries.get(id)?)))
    }
    pub(super) fn write_results(&self, values: &mut JsObject<Value>) {
        for (id, entry) in self.ordered() {
            values.define(id, entry.result.clone());
        }
    }
    pub(super) fn write_snapshot(&self, values: &mut JsObject<Value>) {
        for (id, entry) in self.ordered() {
            if !HIDDEN_NAMES.contains(&id.as_str()) {
                values.set(id, entry.result.clone());
            }
        }
    }
    /// The data the registrations hold, charged to the cache cap.
    pub(super) fn data_bytes(&self) -> usize {
        self.entries
            .values()
            .fold(0usize, |sum, entry| sum.saturating_add(entry.bytes))
    }
    pub(super) fn data(&self) -> impl Iterator<Item = &Value> {
        self.entries
            .values()
            // Only a success holds result data; an idle `data: null` is a marker.
            .filter_map(|entry| match &entry.result {
                Value::Obj(fields)
                    if matches!(fields.get("status"), Some(Value::Str(s)) if s == "success") =>
                {
                    fields.get("data")
                }
                _ => None,
            })
    }
    pub(super) fn append_errors(&self, errors: &mut Vec<Value>) {
        errors.extend(self.ordered().filter_map(|(_, entry)| entry.error.clone()));
    }

    pub(super) fn complete(
        &mut self,
        call: &MutationCall,
        outcome: ToolOutcome<'_>,
        limits: Limits,
        (cap, held): (usize, usize),
    ) -> Result<(bool, Option<ResultLimit>), QueryCommandError> {
        // The other registrations' data stays; this one's is replaced.
        let others = self
            .entries
            .iter()
            .filter(|(id, _)| **id != call.statement)
            .fold(0usize, |sum, (_, entry)| sum.saturating_add(entry.bytes));
        let Some(entry) = self
            .entries
            .get_mut(&call.statement)
            .filter(|entry| call.belongs_to(entry))
        else {
            return Ok((false, None));
        };
        let tool = &entry.tool;
        // The common staging path already bounded provider failure text.
        // A refusal message is generated by the manager itself.
        // Data is admitted as a query result is, by the same function (#90).
        // A missing result has no data and charges nothing.
        let admitted = results::admit(&outcome, limits, false, cap, |bytes| {
            held.saturating_add(others).saturating_add(bytes) <= cap
        })?;
        let refused = match admitted {
            Some(Admitted::Data(data, bytes)) => {
                entry.result = result("success", Some(data), None);
                entry.bytes = bytes;
                entry.error = None;
                return Ok((true, None));
            }
            Some(Admitted::Missing) => {
                entry.result = result("success", None, None);
                entry.bytes = 0;
                entry.error = None;
                return Ok((true, None));
            }
            Some(Admitted::Refused(limit)) => Some(limit),
            None => None,
        };
        // A refused result is delivered as a failed call, including for an
        // older submission: lang-core 1.1.0 lets a stale outcome replace a
        // newer one, and a refusal behaves as that call's failure.
        let message = refused.map(|limit| limit.to_string());
        let outcome = match &message {
            Some(message) => ToolOutcome::CallFailed(message),
            None => outcome,
        };
        let (code, detail, message, hint) = match outcome {
            ToolOutcome::Result(_) | ToolOutcome::ResultValue(_) => return Ok((false, None)),
            ToolOutcome::ToolNotFound(names) => {
                let available = names.join(", ");
                let detail = format!(
                    "[openui] No handler for tool \"{}\". Available: {}",
                    call.dispatched_tool,
                    if available.is_empty() {
                        "(none)"
                    } else {
                        &available
                    }
                );
                (
                    "tool-not-found",
                    detail,
                    format!("Mutation tool \"{tool}\" not found"),
                    (!names.is_empty()).then(|| format!("Available tools: {available}")),
                )
            }
            ToolOutcome::McpError(text) => (
                "mcp-error",
                format!(
                    "MCP tool error: {}",
                    if text.is_empty() {
                        "Unknown error"
                    } else {
                        text
                    }
                ),
                format!("Mutation \"{tool}\" returned an error: {text}"),
                None,
            ),
            ToolOutcome::CallFailed(text) => (
                "tool-error",
                text.to_owned(),
                format!("Mutation \"{tool}\" failed: {text}"),
                None,
            ),
        };
        let mut error = JsObject::new();
        for (key, value) in [
            ("source", "mutation"),
            ("code", code),
            ("message", &message),
            ("statementId", &call.statement),
            ("component", "Mutation"),
        ] {
            error.define(key, Value::Str(value.into()));
        }
        if let Some(raw) = &entry.raw_tool {
            error.define("toolName", raw.clone());
        }
        if let Some(hint) = hint {
            error.define("hint", Value::Str(hint));
        }
        entry.error = Some(Value::Obj(Arc::new(error)));
        entry.result = result("error", None, Some(Value::Str(detail)));
        entry.bytes = 0;
        Ok((true, refused))
    }
}
