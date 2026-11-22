//! Stage a provider result before deciding whether its readers need host functions.
use super::*;

/// A staged delivery. Dropping it leaves the manager and receipt untouched.
pub struct ToolDelivery<'a> {
    pub(super) manager: &'a mut QueryManager,
    pub(super) candidate: QueryManager,
    pub(super) id: &'a ToolCallId,
    refused: Option<ResultLimit>,
}

impl ToolDelivery<'_> {
    /// The result limit delivered as a tool-error, if any.
    pub fn result_refusal(&self) -> Option<ResultLimit> {
        self.refused
    }

    /// Whether finishing this delivery needs the registration's borrowed context.
    pub fn needs_reader_context(&self) -> bool {
        !self.candidate.disposed
            && self.id.0.lifecycle.strong_count() != 0
            && matches!(self.id.0.target, CallTarget::Query(_))
            && self
                .candidate
                .readers
                .as_ref()
                .is_some_and(|p| p.needs_context && p.nodes.iter().any(|n| self.candidate.stale(n)))
    }

    /// Publish the accepted outcome and the resulting calls. An admitted result
    /// survives a downstream evaluation limit. Affected readers expose the cut
    /// until a new registration evaluates them successfully.
    pub fn finish(
        mut self,
        context: Option<(&TreeInput<'_>, &Store)>,
    ) -> Result<QueryUpdate, QueryCommandError> {
        if self.candidate.disposed || self.id.0.lifecycle.strong_count() == 0 {
            self.id.0.consumed.store(true, Ordering::Relaxed);
            return Ok(self.manager.idle_update());
        }
        let remembered = self
            .manager
            .owned_call(self.id)
            .and_then(|call| self.candidate.take_remembered(call.key.clone()));
        let mut requests = runs::QueryRequests::default();
        // A refusal cannot change source data. Without its borrowed context,
        // leave a previously bounded cascade stale for the next trigger.
        if self.refused.is_some() && context.is_none() && self.needs_reader_context() {
            self.candidate.reader_rounds_limited = true;
        } else if matches!(self.id.0.target, CallTarget::Query(_))
            && let Some(mut plan) = self.candidate.readers.clone()
            && plan.nodes.iter().any(|n| self.candidate.stale(n))
        {
            if plan.needs_context && context.is_none() {
                return Err(QueryCommandError::Evaluation(
                    EvaluationFailure::MissingContext {
                        recovery: "QueryManager::complete_tool_call_with_context",
                    },
                ));
            }
            let mut input = context
                .map(|(i, _)| i.clone())
                .unwrap_or_else(|| TreeInput::new(&plan.program));
            input.result = &plan.program;
            input.shared_result = Some(&plan.program);
            let saved;
            let store = if let Some((_, store)) = context {
                store
            } else {
                saved = plan.store.read();
                &saved
            };
            let budget = EvaluationSession::with_limits(
                &(),
                plan.measured.input_bytes(),
                self.candidate.limits,
            );
            let mut cascade = self.candidate.candidate();
            match cascade
                .propagate(
                    &input,
                    store,
                    &budget,
                    &plan.measured,
                    plan.nodes.clone(),
                    &mut requests,
                )
                .and_then(|nodes| {
                    cascade.admit_snapshot_values()?;
                    Ok(nodes)
                }) {
                Ok(nodes) => {
                    plan.nodes = nodes;
                    plan.rounds_limited = cascade.reader_rounds_limited;
                    cascade.readers = Some(plan);
                    self.candidate = cascade;
                }
                Err(QueryCommandError::Limit(limit)) => {
                    requests = runs::QueryRequests::default();
                    self.candidate.suspend_stale_readers(&plan, limit);
                }
                Err(QueryCommandError::Evaluation(_)) if self.refused.is_some() => {
                    requests = runs::QueryRequests::default();
                    self.candidate.reader_rounds_limited = true;
                }
                Err(e) => return Err(e),
            }
        }
        let calls = requests.dispatch(&mut self.candidate, remembered);
        let update = self.candidate.publish_admitted(calls, self.manager);
        *self.manager = self.candidate;
        self.id.0.consumed.store(true, Ordering::Relaxed);
        Ok(update)
    }
}

/// A diagnostic beside the query snapshot, also available on a session's draw and drain.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReaderDiagnostic {
    /// A change to this query's own result did not re-evaluate its arguments.
    SelfInputChanged(String),
    /// A downstream evaluation was cut after the provider's result was accepted.
    PropagationLimit(LimitError),
}

impl QueryManager {
    /// The diagnostic for the current registration. An explicit registration
    /// evaluates complete self-inputs again and retries complete suspended readers.
    pub fn reader_diagnostic(&self) -> Option<ReaderDiagnostic> {
        if let Some(limit) = self.reader_limit {
            return Some(ReaderDiagnostic::PropagationLimit(limit));
        }
        self.readers.as_ref()?.nodes.iter().find_map(|node| {
            node.reads
                .iter()
                .any(|r| r.id == node.id && self.read_changed(r) && self.version(&r.id).is_some())
                .then(|| ReaderDiagnostic::SelfInputChanged(node.id.clone()))
        })
    }

    pub(super) fn suspend_stale_readers(&mut self, plan: &ReaderPlan, limit: LimitError) {
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut work = Vec::new();
        for node in plan.nodes.iter() {
            if self.stale(node) {
                work.push(node.id.as_str());
            }
            for read in node.reads.iter() {
                dependents.entry(&read.id).or_default().push(&node.id);
            }
        }
        while let Some(id) = work.pop() {
            if let Some(q) = self.queries.get_mut(id)
                && !q.suspended()
            {
                q.suspension = Some(limit);
                q.withheld = false;
                q.defaults = Some(Value::Null);
                let mut error = JsObject::new();
                error.define("source", Value::Str("query".into()));
                error.define("statementId", Value::Str(id.into()));
                error.define("code", Value::Str("reader-limit".into()));
                error.define("message", Value::Str(limit.to_string()));
                q.error = Some(Value::Obj(Arc::new(error)));
                work.extend(dependents.get(id).into_iter().flatten().copied());
            }
        }
        // A suspended registration no longer wants its old arguments. An
        // unaffected current or previous holder still owns its outstanding call.
        let healthy: HashSet<_> = self
            .queries
            .values()
            .filter(|q| !q.suspended())
            .flat_map(|q| std::iter::once(&q.key).chain(q.previous.as_ref()))
            .collect();
        for q in self.queries.values().filter(|q| q.suspended()) {
            if !healthy.contains(&q.key)
                && let Some(entry) = self.cache.get_mut(&q.key)
            {
                entry.pending = None;
                entry.run_again = None;
            }
        }
        self.reader_limit = plan.retained_limit.or(Some(limit));
        // The source already passed admission. If listing an error for every
        // suspended reader would exhaust the allowance, keep the diagnostic
        // beside the snapshot. Only readers in this plan can add errors;
        // an incomplete reader's existing error already passed admission.
        if self.admit_snapshot_values().is_err() {
            for node in plan.nodes.iter() {
                if let Some(q) = self.queries.get_mut(&node.id).filter(|q| q.suspended()) {
                    q.defaults = None;
                    q.error = None;
                }
            }
        }
    }

    pub(super) fn stage_borrowed<'a>(
        &'a mut self,
        id: &'a ToolCallId,
        outcome: ToolOutcome<'_>,
    ) -> Result<ToolDelivery<'a>, QueryCommandError> {
        self.stage_delivery(id, outcome, None)
    }

    pub(super) fn stage_delivery<'a>(
        &'a mut self,
        id: &'a ToolCallId,
        outcome: ToolOutcome<'_>,
        refusal: Option<ResultLimit>,
    ) -> Result<ToolDelivery<'a>, QueryCommandError> {
        let discard = self.discards_tool_outcome(id)?;
        let mut candidate = self.candidate();
        let mut refused = None;
        if !discard {
            if refusal.is_none() {
                let (holders, tool_bytes) = self.delivery_holders(id);
                outcome.admit_failure(tool_bytes, holders, self.limits)?;
            }
            let success = matches!(
                outcome,
                ToolOutcome::Result(_) | ToolOutcome::ResultValue(_)
            );
            let (_, limit) = candidate.complete(id, outcome)?;
            refused = refusal.or(limit);
            if candidate.admit_snapshot_values().is_err() {
                let errors_fit = if success && refused.is_none() {
                    // Undo just the staged success. The receipt is still live.
                    let limit = ResultLimit::Snapshot(results::result_limit(self.limits));
                    candidate = self.candidate();
                    candidate.complete(id, ToolOutcome::CallFailed(&limit.to_string()))?;
                    refused = Some(limit);
                    candidate.admit_snapshot_values().is_ok()
                } else {
                    false
                };
                if !errors_fit && let Some(call) = self.owned_call(id) {
                    // No data changed. Hiding the new errors restores at least
                    // as much room as there was before the delivery.
                    candidate.withhold_errors(&call.key);
                }
            }
        } else if id.0.lifecycle.strong_count() != 0 {
            // A previous-only entry may still own its pending call.
            candidate.complete(id, ToolOutcome::Result(None))?;
        }
        candidate.release_unreferenced();
        Ok(ToolDelivery {
            manager: self,
            candidate,
            id,
            refused,
        })
    }
}
