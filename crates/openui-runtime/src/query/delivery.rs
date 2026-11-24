//! Stage a provider result before deciding whether its readers need host functions.
use super::*;

/// A staged delivery. Dropping it leaves the manager and receipt untouched.
pub struct ToolDelivery<'a> {
    pub(super) manager: &'a mut QueryManager,
    pub(super) candidate: QueryManager,
    pub(super) id: &'a ToolCallId,
    refused: Option<ResultLimit>,
    rebuild_snapshot: bool,
    // The delivery's one work allowance, after staging spent its share.
    work: work::Allowance,
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
            self.rebuild_snapshot = true;
            let mut cascade = self.candidate.candidate();
            let work = &mut self.work;
            match cascade
                .propagate(
                    &input,
                    store,
                    &budget,
                    &plan.measured,
                    plan.nodes.clone(),
                    cascade::Trigger::Changed,
                    &mut requests,
                    work,
                )
                .and_then(|nodes| {
                    cascade.admit_snapshot_values_in(work)?;
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
                    self.candidate
                        .suspend_stale_readers(&plan, limit, &mut self.work);
                }
                Err(QueryCommandError::Evaluation(_)) if self.refused.is_some() => {
                    requests = runs::QueryRequests::default();
                    self.candidate.reader_rounds_limited = true;
                }
                Err(e) => return Err(e),
            }
        }
        let calls = requests.dispatch(&mut self.candidate, remembered);
        self.candidate.release_unreferenced();
        #[cfg(feature = "query-audit")]
        let before_publication = self.work.audit().spent;
        let mut changed = false;
        let mut hidden_change = false;
        if self.rebuild_snapshot || !calls.is_empty() {
            let limits = self.candidate.limits;
            hidden_change = self.candidate.mutations.hidden_changed_in(
                &self.manager.mutations,
                limits,
                &mut self.work,
            )?;
            let mut snapshot = self.candidate.project_snapshot();
            changed = snapshot.changed(&self.manager.snapshot, limits, &mut self.work)?;
            if changed {
                snapshot.prepare_json(limits, &mut self.work);
            }
            self.candidate.snapshot = Arc::new(snapshot);
        }
        let update =
            self.candidate
                .update_after_publication(calls, self.manager, changed, hidden_change);
        #[cfg(feature = "query-audit")]
        {
            let mut audit = self.work.audit();
            audit.publication = audit.spent - before_publication;
            self.candidate.last_delivery_work = Some(audit);
        }
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

    pub(super) fn suspend_stale_readers(
        &mut self,
        plan: &ReaderPlan,
        limit: LimitError,
        _allowance: &mut work::Allowance,
    ) {
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut work = Vec::new();
        for node in plan.nodes.iter() {
            // A self read normally only contributes a diagnostic. If its
            // comparison itself was cut, that reader also needs a listed Retry.
            let unresolved_self = node.reads.iter().filter(|r| r.id == node.id).any(|r| {
                self.queries
                    .get(&r.id)
                    .and_then(|q| self.data_entry(q))
                    .and_then(|e| r.string_pair(e))
                    .is_some_and(|pair| !self.reader_comparisons.contains_key(&pair.versions))
            });
            if unresolved_self || self.stale(node) {
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
                q.defaults = Some(Arc::new(Value::Null));
                let message = limit.to_string();
                let fields = [
                    ("source", "query"),
                    ("statementId", id),
                    ("code", "reader-limit"),
                    ("message", &message),
                ];
                #[cfg(feature = "query-audit")]
                _allowance.listing_row(&fields);
                let mut error = JsObject::with_capacity(fields.len());
                for (key, value) in fields {
                    error.define(key, Value::Str(value.into()));
                }
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
        let mut work = work::Allowance::new(self.limits);
        if let Some(call) = self.owned_call(id) {
            // A delivery pays for its data-derived lookup once. All preflight,
            // candidate and reader probes reuse the canonical key's digest.
            let (_, flags) = work.phase(self.limits, |eval| eval.charge_work(call.key.work));
            check_cuts(flags)?;
        }
        let mut rebuild_snapshot = false;
        if !discard {
            if refusal.is_none() {
                let (holders, tool_bytes) = self.delivery_holders(id);
                outcome.admit_failure(tool_bytes, holders, self.limits, &mut work)?;
            }
            let success = matches!(
                outcome,
                ToolOutcome::Result(_) | ToolOutcome::ResultValue(_)
            );
            let (applied, limit) = candidate.complete(id, outcome, &mut work)?;
            rebuild_snapshot = applied;
            refused = refusal.or(limit);
            if candidate.admit_snapshot_values_in(&mut work).is_err() {
                let errors_fit = if success && refused.is_none() {
                    // Undo just the staged success. The receipt is still live.
                    let limit = ResultLimit::Snapshot(results::result_limit(self.limits));
                    candidate = self.candidate();
                    candidate.complete(
                        id,
                        ToolOutcome::CallFailed(&limit.to_string()),
                        &mut work,
                    )?;
                    refused = Some(limit);
                    candidate.admit_snapshot_values_in(&mut work).is_ok()
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
            candidate.complete(id, ToolOutcome::Result(None), &mut work)?;
        }
        candidate.release_unreferenced();
        if !discard
            && matches!(id.0.target, CallTarget::Query(_))
            && let Some(plan) = candidate.readers.clone()
        {
            match candidate.refresh_reader_comparisons(
                &plan.nodes,
                &self.reader_comparisons,
                &mut work,
            ) {
                Ok(()) => {}
                Err(QueryCommandError::Limit(limit)) => {
                    candidate.suspend_stale_readers(&plan, limit, &mut work)
                }
                Err(err) => return Err(err),
            }
        }
        Ok(ToolDelivery {
            manager: self,
            candidate,
            id,
            refused,
            rebuild_snapshot,
            work,
        })
    }
}
