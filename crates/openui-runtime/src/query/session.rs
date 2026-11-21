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
    /// A JavaScript undefined result. Queries cache null and mutations omit data.
    MissingResult,
    /// Host data, including explicit null.
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
            crate::McpToolOutcome::ToolReported { message } => Self::McpError(message),
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

    /// Whether the manager still owns the outstanding call. A released query
    /// receipt can still trigger stale readers when its provider finishes.
    pub fn is_pending(&self) -> bool {
        !matches!(&self.0.target, CallTarget::Query(call) if call.strong_count() == 0)
            && self.0.lifecycle.strong_count() != 0
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
    /// When registering with library context, supply it again with
    /// [`Self::complete_tool_call_with_context`] so readers can call borrowed functions.
    pub fn evaluate_transaction<T, E: From<QueryCommandError>>(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        read: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<(T, QueryUpdate), E> {
        self.registration_transaction(input, store, false, None, read)
    }

    /// Stage both renderer effects on a settled program, then draw before committing either.
    pub fn settled_transaction<T, E: From<QueryCommandError>>(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        read: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<(T, QueryUpdate), E> {
        self.registration_transaction(input, store, true, None, read)
    }

    /// Install new intervals at the host sample only if the draw commits.
    pub fn registration_transaction<T, E: From<QueryCommandError>>(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        mutations: bool,
        now_ms: Option<u64>,
        read: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<(T, QueryUpdate), E> {
        if self.disposed {
            return Ok((read(self)?, self.idle_update()));
        }
        let mut candidate = self.candidate();
        if let Some(now) = now_ms {
            candidate.sample_clock(now)?;
        }
        let calls = candidate.register_readers(input, store)?;
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

    pub(super) fn candidate(&self) -> Self {
        Self {
            reader_limit: self.reader_limit,
            readers: self.readers.clone(),
            last_result: self.last_result,
            reader_rounds_limited: false,
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

    pub(super) fn publish(
        &mut self,
        calls: Vec<ToolCall>,
        previous: &Self,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.admit_snapshot_values()?;
        Ok(self.publish_admitted(calls, previous))
    }

    // Delivery has already reserved the source snapshot, and any reader cascade
    // has reserved its replacement. Removing unheld entries changes neither.
    pub(super) fn publish_admitted(
        &mut self,
        calls: Vec<ToolCall>,
        previous: &Self,
    ) -> QueryUpdate {
        self.release_unreferenced();
        self.rebuild_snapshot();
        QueryUpdate {
            next_refresh_due_ms: self.next_refresh_due_ms(),
            calls,
            hidden_change: self
                .mutations
                .hidden_changed(&previous.mutations.hidden_results()),
            changed: self.snapshot_json != previous.snapshot_json,
            reader_diagnostic_changed: self.reader_diagnostic() != previous.reader_diagnostic(),
            reader_rounds_limited: self.reader_rounds_limited,
        }
    }

    pub(super) fn admit_snapshot_values(&self) -> Result<(), QueryCommandError> {
        #[cfg(test)]
        round_tests::snapshot_pass();
        // The same entry can be projected at many statement ids. Reserve all
        // result and error bodies together before copying/serializing them.
        let room = EvaluationSession::with_limits(&(), 0, crate::admission::fixed(self.limits));
        room.with_evaluator(|eval| {
            for q in self.queries.values() {
                // The fixed null placeholder is visible even before a call.
                // A fetched null, like every delivered result, is charged.
                let default = q.defaults.as_ref().filter(|v| !matches!(v, Value::Null));
                for value in [
                    self.data(q).or(default),
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
        check_cuts(room.flags())
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

    /// Whether the payload can be discarded before parsing or admission.
    /// A receipt still needs completion to retire it and process stale readers.
    pub fn discards_tool_outcome(&self, id: &ToolCallId) -> Result<bool, QueryCommandError> {
        self.check_call(id)?;
        Ok(id.0.lifecycle.strong_count() == 0 || self.disposed || self.delivery_holders(id).0 == 0)
    }

    pub(super) fn delivery_holders(&self, id: &ToolCallId) -> (usize, usize) {
        match &id.0.target {
            CallTarget::Mutation(call) => (1, call.dispatched_tool.len()),
            CallTarget::Query(target) => target.upgrade().map_or((0, 0), |p| {
                (
                    self.queries
                        .values()
                        .filter(|q| !q.suspended && q.key == p.key)
                        .count(),
                    p.tool_name.len(),
                )
            }),
        }
    }

    /// Deliver a result refused before reading it as the call's tool-error.
    pub fn complete_tool_call_refused(
        &mut self,
        id: &ToolCallId,
        limit: ResultLimit,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.stage_tool_call_refused(id, limit)?.finish(None)
    }

    /// Stage a refusal so a session can supply context to any stale readers.
    pub fn stage_tool_call_refused<'a>(
        &'a mut self,
        id: &'a ToolCallId,
        limit: ResultLimit,
    ) -> Result<ToolDelivery<'a>, QueryCommandError> {
        self.stage_delivery(id, ToolOutcome::CallFailed(&limit.to_string()), Some(limit))
    }

    /// Deliver a provider outcome. A result over a limit ends the call as a tool-error.
    /// Admission counts shared results and errors
    /// at every position in the snapshot. A downstream reader limit consumes
    /// the admitted receipt and is available through [`Self::reader_diagnostic`].
    /// Readers see committed Store writes until it is dropped, then its last
    /// committed state. If a stale reader requires the registration's borrowed
    /// context, retry the receipt with [`Self::complete_tool_call_with_context`].
    /// A result refusal always settles. If readers left stale at the round
    /// bound cannot run with this context, the update keeps the cut flag.
    pub fn complete_tool_call(
        &mut self,
        id: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.stage_tool_call(id, outcome)?.finish(None)
    }

    /// Complete a call using the live Store and library context. Reader expressions
    /// come from the last successful registration, including during a streaming edit.
    pub fn complete_tool_call_with_context(
        &mut self,
        id: &ToolCallId,
        outcome: ToolCallOutcome,
        input: &TreeInput<'_>,
        store: &Store,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.stage_tool_call(id, outcome)?
            .finish(Some((input, store)))
    }

    /// Admit a provider outcome before constructing borrowed reader context.
    pub fn stage_tool_call<'a>(
        &'a mut self,
        id: &'a ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<ToolDelivery<'a>, QueryCommandError> {
        if self.discards_tool_outcome(id)? {
            return self.stage_borrowed(id, ToolOutcome::Result(None));
        }
        let limit = results::result_limit(self.limits);
        let outcome = match &outcome {
            ToolCallOutcome::MissingResult => ToolOutcome::Result(None),
            ToolCallOutcome::Result(value) => ToolOutcome::ResultValue(value),
            ToolCallOutcome::ToolNotFound(names) => {
                // Bound the temporary list of borrowed names before conversion.
                if names.len() > limit / 2 {
                    return Err(QueryCommandError::Limit(crate::admission::size_error()));
                }
                ToolOutcome::ToolNotFound(names.iter().map(String::as_str).collect())
            }
            ToolCallOutcome::McpError(text) => ToolOutcome::McpError(text),
            ToolCallOutcome::CallFailed(text) => ToolOutcome::CallFailed(text),
        };
        self.stage_borrowed(id, outcome)
    }
}
