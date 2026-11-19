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
    /// Whether an undelivered call still has a live cache entry.
    pub fn is_pending(&self) -> bool {
        self.0.target.strong_count() != 0 && !self.0.consumed.load(Ordering::Relaxed)
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
        let mut candidate = self.candidate();
        let nodes = candidate.evaluate_nodes(input, store)?;
        let calls = candidate.apply_nodes(input.result, nodes);
        let update = candidate.publish(calls, &self.snapshot_json)?;
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
        PreparedReferenceValues::managed(values, undefined, limits)
    }

    fn candidate(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            limits: self.limits,
            queries: self.queries.clone(),
            order: self.order.clone(),
            cache: self.cache.clone(),
            extra_keys: self.extra_keys.clone(),
            snapshot: JsObject::new(),
            snapshot_json: String::new(),
        }
    }

    fn publish(
        &mut self,
        calls: Vec<ToolCall>,
        previous: &str,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.cache.retain(|_, entry| entry.references != 0);
        // The same entry can be projected at many statement ids. Reserve all
        // result and error bodies together before copying/serializing them.
        let room = EvaluationSession::with_limits(&(), 0, crate::admission::fixed(self.limits));
        room.with_evaluator(|eval| {
            for q in self.queries.values() {
                for value in [self.data(q).or(q.defaults.as_ref()), q.error.as_ref()]
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
            calls,
            changed: self.snapshot_json != previous,
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
    pub fn complete_tool_call(
        &mut self,
        id: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<QueryUpdate, QueryCommandError> {
        self.check_call(id)?;
        let target = id.0.target.upgrade();
        let readers = target.as_ref().map_or(0, |p| {
            self.queries.values().filter(|q| q.key == p.key).count()
        });
        if readers == 0 {
            // ADR 0013: discard the outcome for a released or previous-only
            // entry. Retire its pending flag and consume the receipt once.
            return self.apply(QueryCommand::Complete {
                id,
                outcome: ToolOutcome::Result(None),
            });
        }
        let limit = self
            .limits
            .result_size()
            .min(self.limits.work_for(usize::MAX));
        // Error records own their formatted messages. Bound the copies before
        // formatting any of them, even if a single failed key has many readers.
        let check_text = |bytes: usize| {
            let bytes = bytes.saturating_add(target.as_ref().map_or(0, |p| p.tool_name.len()));
            if bytes.saturating_mul(readers) > limit {
                Err(QueryCommandError::Limit(crate::admission::size_error()))
            } else {
                Ok(())
            }
        };
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
                check_text(bytes)?;
                ToolOutcome::ToolNotFound(names.iter().map(String::as_str).collect())
            }
            ToolCallOutcome::McpError(text) => {
                check_text(text.len())?;
                ToolOutcome::McpError(text)
            }
            ToolCallOutcome::CallFailed(text) => {
                check_text(text.len())?;
                ToolOutcome::CallFailed(text)
            }
        };
        let mut candidate = self.candidate();
        candidate.complete(id, outcome)?;
        let calls = target
            .as_ref()
            .and_then(|pending| candidate.run_remembered(&pending.key))
            .into_iter()
            .collect();
        let update = candidate.publish(calls, &self.snapshot_json)?;
        *self = candidate;
        id.0.consumed.store(true, Ordering::Relaxed);
        Ok(update)
    }
}
