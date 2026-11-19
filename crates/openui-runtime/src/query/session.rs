//! Session integration for tool outcomes, registration and drawing.
use super::*;

impl<'a> ToolOutcome<'a> {
    /// Compatibility constructor for the original borrowed provider interface.
    #[doc(hidden)]
    #[allow(non_snake_case)]
    pub fn ToolError(message: &'a str) -> Self {
        Self::CallFailed(message)
    }
}

/// The owned outcome returned by a session's tool provider. Data stays in
/// the evaluator's representation through admission and cache delivery.
#[derive(Debug)]
#[non_exhaustive]
pub enum ToolCallOutcome {
    /// Host data. A missing result is represented by null.
    Result(Value),
    /// Available names in provider order.
    ToolNotFound(Vec<String>),
    /// The tool reported an MCP error.
    McpError(String),
    /// The invocation itself failed.
    CallFailed(String),
}

impl From<crate::McpToolOutcome> for ToolCallOutcome {
    fn from(outcome: crate::McpToolOutcome) -> Self {
        match outcome {
            crate::McpToolOutcome::Result(value) => Self::Result(value),
            crate::McpToolOutcome::ToolError { message } => Self::McpError(message),
        }
    }
}

impl ToolCallId {
    pub(super) fn query_key(&self) -> Option<Arc<str>> {
        match &self.0.target {
            CallTarget::Query(target) => target.upgrade().map(|call| call.key.clone()),
            CallTarget::Mutation(_) => None,
        }
    }

    /// Whether this receipt can still dispatch work for its manager.
    pub fn is_pending(&self) -> bool {
        !matches!(&self.0.target, CallTarget::Query(call) if call.strong_count() == 0)
            && !matches!(&self.0.target, CallTarget::Mutation(call) if call.lifecycle.strong_count() == 0)
            && self.0.manager.strong_count() != 0
            && !self.0.consumed.load(Ordering::Relaxed)
    }
}

impl ToolCall {
    /// Arguments in the binding's JSON projection. Undefined has no projection.
    pub fn arguments_json(&self) -> Option<String> {
        self.args.as_ref().map(crate::tree::value_json)
    }
}

impl QueryManager {
    /// Stage query registration and its drawing together. Only the read-only
    /// result view escapes to the callback. A refusal preserves pending identities.
    pub fn evaluate_transaction<T, E: From<QueryCommandError>>(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        read: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<(T, QueryUpdate), E> {
        self.registration_transaction(input, store, false, read)
    }

    /// Stage both renderer effects on a settled program, then draw before committing either.
    pub fn settled_transaction<T, E: From<QueryCommandError>>(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        read: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<(T, QueryUpdate), E> {
        self.registration_transaction(input, store, true, read)
    }

    fn registration_transaction<T, E: From<QueryCommandError>>(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        mutations: bool,
        read: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<(T, QueryUpdate), E> {
        if self.disposed {
            return Ok((read(self)?, self.idle_update()));
        }
        let mut candidate = self.candidate();
        let nodes = candidate.evaluate_nodes(input, store)?;
        let calls = candidate.apply_nodes(input.result, nodes);
        if mutations {
            candidate.register_mutation_statements(input, store)?;
        }
        let update = candidate.publish(calls, self)?;
        let output = read(&candidate)?;
        *self = candidate;
        Ok((output, update))
    }

    /// Prepare registered results for the evaluator, independently of the
    /// status properties in the serialized snapshot.
    pub fn prepared_results(&self, limits: Limits) -> PreparedReferenceValues<'static> {
        let mut values = JsObject::new();
        let mut undefined = HashSet::new();
        for id in &self.order {
            if let Some(query) = self.queries.get(id) {
                if let Some(value) = self.data(query).or(query.defaults.as_ref()) {
                    values.define(id, value.clone());
                } else {
                    undefined.insert(id.clone());
                }
            }
        }
        self.mutations.write_results(&mut values);
        PreparedReferenceValues::managed(values, undefined, limits)
    }

    fn candidate(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            limits: self.limits,
            lifecycle: self.lifecycle.clone(),
            clock: self.clock.clone(),
            disposed: self.disposed,
            mutations: self.mutations.clone(),
            queries: self.queries.clone(),
            order: self.order.clone(),
            cache: self.cache.clone(),
            extra_keys: self.extra_keys.clone(),
            snapshot: JsObject::new(),
            snapshot_json: String::new(),
            cache_limit: self.cache_limit,
            cache_bytes: self.cache_bytes,
        }
    }

    fn publish(
        &mut self,
        calls: Vec<ToolCall>,
        previous: &Self,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.release_unreferenced();
        // The same entry can be projected at many statement ids. Reserve all
        // result and error bodies together before copying/serializing them.
        let room = EvaluationSession::with_limits(&(), 0, crate::admission::fixed(self.limits));
        room.with_evaluator(|eval| {
            for q in self.queries.values() {
                for value in [
                    self.data(q).or(q.defaults.as_ref()),
                    q.error.as_ref().filter(|_| !q.withheld),
                ]
                .into_iter()
                .flatten()
                {
                    if !eval.reserve_value(value) {
                        return;
                    }
                }
            }
        });
        check_cuts(room.flags())?;
        self.rebuild_snapshot();
        Ok(QueryUpdate {
            next_refresh_due_ms: self.next_refresh_due_ms(),
            calls,
            changed: self.snapshot_json != previous.snapshot_json,
            hidden_change: self
                .mutations
                .hidden_changed(&previous.mutations.hidden_results()),
        })
    }

    /// Validate a capability before reading or copying a host outcome.
    pub fn check_call(&self, id: &ToolCallId) -> Result<(), QueryCommandError> {
        if !id.0.manager.ptr_eq(&Arc::downgrade(&self.owner))
            || id.0.consumed.load(Ordering::Relaxed)
        {
            return Err(QueryCommandError::InvalidCall);
        }
        Ok(())
    }

    /// Complete a session call atomically, including admission of shared
    /// results/error messages at every position in the new snapshot.
    ///
    /// A result is admitted under the result-size and depth limits and the
    /// cache cap before it is copied. A refused result is delivered as the
    /// call's `tool-error`, naming the limit, to every query on its key (or
    /// to the mutation), and the call is consumed. Nothing held is evicted.
    /// A refusal of the command itself (an invalid identity, a value that is
    /// not JSON, failure text past the limits, or a snapshot that cannot hold
    /// the delivery) changes nothing and leaves the call completable.
    pub fn complete_tool_call(
        &mut self,
        id: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.complete_tool_call_reporting(id, outcome)
            .map(|(update, _)| update)
    }

    pub(super) fn complete_tool_call_reporting(
        &mut self,
        id: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<(QueryUpdate, Option<ResultLimit>), QueryCommandError> {
        self.check_call(id)?;
        // A completion for a released entry is discarded whatever it holds,
        // on the borrowed and owned paths alike.
        if self.disposed || self.readers(id).0 == 0 {
            return self.complete_command(id, ToolOutcome::Result(None));
        }
        let limit = results::result_limit(self.limits);
        let outcome = match &outcome {
            ToolCallOutcome::Result(value) => ToolOutcome::ResultValue(value),
            ToolCallOutcome::ToolNotFound(names) => {
                let mut bytes = 0usize;
                for name in names {
                    bytes = bytes.saturating_add(name.len()).saturating_add(2);
                    if bytes > limit {
                        return Err(QueryCommandError::Limit(crate::admission::size_error()));
                    }
                }
                ToolOutcome::ToolNotFound(names.iter().map(String::as_str).collect())
            }
            ToolCallOutcome::McpError(text) => ToolOutcome::McpError(text),
            ToolCallOutcome::CallFailed(text) => ToolOutcome::CallFailed(text),
        };
        self.complete_command(id, outcome)
    }

    /// The one completion path, for borrowed and owned outcomes alike. It
    /// never leaves a call that cannot be completed: if the delivery does not
    /// fit the snapshot's room, a result is delivered as a `tool-error`
    /// instead; if those errors do not fit either, the queries keep them
    /// unlisted and the snapshot shows one compact diagnostic (#90).
    pub(super) fn complete_command(
        &mut self,
        id: &ToolCallId,
        outcome: ToolOutcome<'_>,
    ) -> Result<(QueryUpdate, Option<ResultLimit>), QueryCommandError> {
        self.complete_delivery(id, outcome, None)
    }

    /// Deliver a result the caller refused under `limit` before reading it
    /// (a session's outcome JSON too long or too deep to parse) as the
    /// call's `tool-error`, exactly as a result the manager refuses: it
    /// always ends the call, whatever the number of readers.
    pub fn complete_tool_call_refused(
        &mut self,
        id: &ToolCallId,
        limit: ResultLimit,
    ) -> Result<QueryUpdate, QueryCommandError> {
        let message = limit.to_string();
        self.complete_delivery(id, ToolOutcome::CallFailed(&message), Some(limit))
            .map(|(update, _)| update)
    }

    fn readers(&self, id: &ToolCallId) -> (usize, usize) {
        match &id.0.target {
            CallTarget::Mutation(call) => (1, call.dispatched_tool.len()),
            CallTarget::Query(target) => target.upgrade().map_or((0, 0), |p| {
                (
                    self.queries.values().filter(|q| q.key == p.key).count(),
                    p.tool_name.len(),
                )
            }),
        }
    }

    fn complete_delivery(
        &mut self,
        id: &ToolCallId,
        outcome: ToolOutcome<'_>,
        refusal: Option<ResultLimit>,
    ) -> Result<(QueryUpdate, Option<ResultLimit>), QueryCommandError> {
        self.check_call(id)?;
        let (readers, tool_bytes) = self.readers(id);
        if self.disposed || readers == 0 {
            // ADR 0013: discard the outcome for a released or previous-only
            // entry. Retire its pending flag and consume the receipt once.
            self.complete(id, ToolOutcome::Result(None))?;
            id.0.consumed.store(true, Ordering::Relaxed);
            return Ok((self.idle_update(), None));
        }
        let limit = results::result_limit(self.limits);
        // Error records own their formatted messages. Bound the copies before
        // formatting any of them, even if a single failed key has many readers.
        let text = match &outcome {
            ToolOutcome::ToolNotFound(names) => names.iter().fold(0usize, |bytes, name| {
                bytes.saturating_add(name.len()).saturating_add(2)
            }),
            ToolOutcome::McpError(text) | ToolOutcome::CallFailed(text) => text.len(),
            _ => 0,
        };
        // A refused result's message is the manager's own: the fallback below
        // bounds it, so it never refuses the command.
        if refusal.is_none()
            && text.saturating_add(tool_bytes).saturating_mul(readers) > limit
            && text != 0
        {
            return Err(QueryCommandError::Limit(crate::admission::size_error()));
        }
        let success = refusal.is_some()
            || matches!(
                outcome,
                ToolOutcome::Result(_) | ToolOutcome::ResultValue(_)
            );
        let (candidate, published, refused) = self.attempt(id, outcome.clone(), false)?;
        let refused = refused.or(refusal);
        let (candidate, update, refused) = match published {
            Ok(update) => (candidate, update, refused),
            Err(QueryCommandError::Limit(_)) => {
                // The room cannot hold the delivery. A result becomes a
                // tool-error; a failure keeps its own error.
                let limit = refused.unwrap_or(ResultLimit::Snapshot(limit));
                let message = limit.to_string();
                let fallback = if success {
                    ToolOutcome::CallFailed(&message)
                } else {
                    outcome
                };
                let refused = success.then_some(limit);
                let (candidate, published, _) = self.attempt(id, fallback.clone(), false)?;
                match published {
                    Ok(update) => (candidate, update, refused),
                    Err(QueryCommandError::Limit(_)) if id.query_key().is_some() => {
                        let (candidate, published, _) = self.attempt(id, fallback, true)?;
                        (candidate, published?, refused)
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        *self = candidate;
        id.0.consumed.store(true, Ordering::Relaxed);
        Ok((update, refused))
    }

    // Complete on a candidate and publish it, without committing.
    #[allow(clippy::type_complexity)]
    fn attempt(
        &self,
        id: &ToolCallId,
        outcome: ToolOutcome<'_>,
        withhold: bool,
    ) -> Result<
        (
            Self,
            Result<QueryUpdate, QueryCommandError>,
            Option<ResultLimit>,
        ),
        QueryCommandError,
    > {
        let key = id.query_key();
        let mut candidate = self.candidate();
        let (_, refused) = candidate.complete(id, outcome)?;
        if withhold && let Some(key) = &key {
            candidate.withhold_errors(key);
        }
        let calls = key
            .and_then(|key| candidate.run_remembered(&key))
            .into_iter()
            .collect();
        // A withheld delivery stores no data and lists no new error: it adds
        // only the one compact diagnostic, so it is published whatever the
        // room, and the call can always end.
        let published = if withhold {
            candidate.release_unreferenced();
            candidate.rebuild_snapshot();
            Ok(QueryUpdate {
                next_refresh_due_ms: candidate.next_refresh_due_ms(),
                calls,
                changed: candidate.snapshot_json != self.snapshot_json,
                hidden_change: false,
            })
        } else {
            candidate.publish(calls, self)
        };
        Ok((candidate, published, refused))
    }
}
