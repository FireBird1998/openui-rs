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
    pub(super) dispatched_tool: String,
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
            calls: Vec::new(),
            changed: self.rebuild_snapshot(),
            hidden_change: self.mutations.hidden_changed(&previous),
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
        let nodes = self.with_statement_evaluator(input, store, |scope, eval| {
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
                    },
                );
            }
        }
        Ok(())
    }

    /// Whether the registration exists and accepts a submission now.
    pub fn can_fire_mutation(&self, statement: &str) -> bool {
        self.mutations
            .entries
            .get(statement)
            .is_some_and(Entry::accepts_submission)
    }

    /// Start a mutation with click-time arguments. Unknown or loading statements
    /// return an empty update. A dispatched call retains the registration identity.
    pub fn fire_mutation(
        &mut self,
        statement: &str,
        args: Option<&Value>,
    ) -> Result<QueryUpdate, QueryCommandError> {
        let Some(entry) = self
            .mutations
            .entries
            .get_mut(statement)
            .filter(|entry| entry.accepts_submission())
        else {
            return Ok(QueryUpdate {
                calls: Vec::new(),
                changed: false,
                hidden_change: false,
            });
        };
        if let Some(value) = args {
            crate::admission::measure(value, self.limits).map_err(QueryCommandError::Limit)?;
        }
        let call = ToolCall {
            id: ToolCallId(Arc::new(CallIdentity {
                manager: Arc::downgrade(&self.owner),
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
        Ok(QueryUpdate {
            calls: vec![call],
            hidden_change,
            changed: self.rebuild_snapshot(),
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
    pub(super) fn append_errors(&self, errors: &mut Vec<Value>) {
        errors.extend(self.ordered().filter_map(|(_, entry)| entry.error.clone()));
    }

    pub(super) fn complete(
        &mut self,
        call: &MutationCall,
        outcome: ToolOutcome<'_>,
        limits: Limits,
    ) -> Result<bool, QueryCommandError> {
        let Some(entry) = self
            .entries
            .get_mut(&call.statement)
            .filter(|entry| call.registration.ptr_eq(&Arc::downgrade(&entry.identity)))
        else {
            return Ok(false);
        };
        let tool = &entry.tool;
        // Format host error text only after bounding its expansion. Result data
        // uses the existing tool-result admission path below.
        let text_bytes = match &outcome {
            ToolOutcome::ToolNotFound(names) => names
                .iter()
                .try_fold(names.len().saturating_mul(2), |bytes, name| {
                    bytes.checked_add(name.len())
                })
                .unwrap_or(usize::MAX),
            ToolOutcome::CallFailed(text) | ToolOutcome::McpError(text) => text.len(),
            _ => 0,
        };
        if text_bytes > limits.result_size().min(limits.work_for(usize::MAX)) {
            return Err(QueryCommandError::Limit(crate::admission::size_error()));
        }
        let (code, detail, message, hint) = match outcome {
            ToolOutcome::Result(value) => {
                entry.result = result(
                    "success",
                    value.map(|v| admit_result(v, limits)).transpose()?,
                    None,
                );
                entry.error = None;
                return Ok(true);
            }
            ToolOutcome::ResultValue(value) => {
                crate::admission::measure(value, limits).map_err(QueryCommandError::Limit)?;
                entry.result = result("success", Some(value.clone()), None);
                entry.error = None;
                return Ok(true);
            }
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
        Ok(true)
    }
}
