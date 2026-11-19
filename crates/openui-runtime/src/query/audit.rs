//! State observations for the conformance drivers. These checks do not run in
//! the command path. They derive ownership from registrations and loading from
//! embedder-held receipts, independently of cache counters and pending_queries.
use super::*;

impl QueryManager {
    pub(super) fn explains_suspension(&self, id: &str, query: &Query) -> bool {
        self.reader_limit.is_some()
            && (matches!((&query.defaults, &query.error), (None, None))
                || matches!((&query.defaults, &query.error),
                (Some(Value::Null), Some(Value::Obj(e)))
                if matches!(e.get("code"), Some(Value::Str(s)) if s == "reader-limit")
                    && matches!(e.get("statementId"), Some(Value::Str(s)) if s == id)))
    }

    /// Audit a command's *new* calls before adding them to the receipt history.
    /// Historical receipts may legitimately have been retired by a later command.
    /// Outside the supported interface.
    #[doc(hidden)]
    pub fn audit_output(&self, calls: &[ToolCall]) -> Result<(), String> {
        if calls.is_empty() {
            return Ok(());
        }
        let current: HashSet<_> = self
            .queries
            .values()
            .filter(|q| !q.suspended)
            .map(|q| q.key.as_ref())
            .collect();
        let mut keys = HashSet::new();
        let mut mutations = HashSet::new();
        for call in calls {
            if !call.id.is_pending() || self.check_call(&call.id).is_err() {
                return Err("returned a retired or foreign receipt".into());
            }
            match &call.id.0.target {
                CallTarget::Query(target) => {
                    let pending = target
                        .upgrade()
                        .ok_or("returned a released query receipt")?;
                    if !current.contains(pending.key.as_ref())
                        || self
                            .cache
                            .get(&pending.key)
                            .and_then(|e| e.pending.as_ref())
                            .is_none_or(|p| !Arc::ptr_eq(p, &pending))
                    {
                        return Err(format!(
                            "returned a receipt without its current entry: {}",
                            pending.key
                        ));
                    }
                    if !keys.insert(pending.key.clone()) {
                        return Err(format!("returned two calls for {}", pending.key));
                    }
                }
                CallTarget::Mutation(target) => {
                    let id = self
                        .mutations
                        .pending_registration(target)
                        .ok_or("returned a receipt without its mutation registration")?;
                    if !mutations.insert(id) {
                        return Err(format!("returned two calls for mutation {id}"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Check the four cache rules' persistent state against the replay driver's
    /// call identities. Check each newly returned batch with [`Self::audit_output`]
    /// before mixing it with historical receipts. Outside the supported interface.
    #[doc(hidden)]
    pub fn audit_cache<'a>(
        &self,
        calls: impl Iterator<Item = &'a ToolCallId>,
        registered: bool,
    ) -> Result<(), String> {
        let mut held = HashMap::new();
        for q in self.queries.values() {
            for key in std::iter::once(&q.key).chain(q.previous.as_ref()) {
                *held.entry(key).or_insert(0usize) += 1;
            }
        }
        if held.len() != self.cache.len()
            || held
                .iter()
                .any(|(k, n)| self.cache.get(*k).is_none_or(|e| e.references != *n))
        {
            return Err(
                "rule 1: cache entries or their holder counts differ from registrations".into(),
            );
        }
        let mut live = HashSet::new();
        for id in calls.filter(|id| id.is_pending()) {
            if let CallTarget::Query(target) = &id.0.target
                && let Some(call) = target.upgrade()
                && (self
                    .cache
                    .get(&call.key)
                    .and_then(|e| e.pending.as_ref())
                    .is_none_or(|p| !Arc::ptr_eq(p, &call))
                    || !live.insert(call.key.clone()))
            {
                return Err(format!(
                    "rule 2: misplaced or duplicated receipt for {}",
                    call.key
                ));
            }
        }
        for (key, e) in &self.cache {
            if e.pending.is_some() != live.contains(key) {
                return Err(format!("rule 2: missing receipt for {key}"));
            }
        }
        // A wait propagates loading transitively. Follow edges from real
        // receipts, so an unexplained waiting flag cannot establish loading.
        let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut loading = HashSet::new();
        let mut queue = Vec::new();
        let mut waits = HashMap::new();
        let errors = match self.snapshot.get("__openui_errors") {
            Some(Value::Arr(errors)) => errors.as_slice(),
            _ => &[],
        };
        let visible: HashSet<_> = errors
            .iter()
            .filter_map(|error| {
                let Value::Obj(e) = error else { return None };
                match (e.get("source"), e.get("statementId")) {
                    (Some(Value::Str(source)), Some(Value::Str(id))) if source == "query" => {
                        Some(id.as_str())
                    }
                    _ => None,
                }
            })
            .collect();
        let compact = self
            .withheld_diagnostic()
            .is_some_and(|e| errors.contains(&e));
        let shows_failure = |id: &str, q: &Query| {
            q.error.is_some() && (visible.contains(id) || (q.withheld && compact))
        };
        for (id, q) in &self.queries {
            let Some(entry) = self.cache.get(&q.key) else {
                return Err(format!("missing key for {id}"));
            };
            if q.suspended {
                if !self.explains_suspension(id, q) {
                    return Err(format!("rule 4: unexplained suspended reader {id}"));
                }
                continue;
            }
            if live.contains(&q.key) {
                queue.push(id.as_str());
            }
            if q.waiting() {
                let mut missing = Vec::new();
                for read in q.reads.iter() {
                    reverse.entry(&read.id).or_default().push(id.as_str());
                    if self.version(&read.id).is_none() {
                        missing.push(read.id.as_str());
                    }
                }
                waits.insert(id.as_str(), missing);
            } else if registered
                && !self.disposed
                && entry.data.is_none()
                && !live.contains(&q.key)
                && !(entry.failed && shows_failure(id, q))
            {
                return Err(format!(
                    "rule 4: idle query {id} has no result, error or receipt"
                ));
            }
        }
        while let Some(id) = queue.pop() {
            if loading.insert(id) {
                queue.extend(reverse.get(id).into_iter().flatten().copied());
            }
        }
        for (name, history) in [("__openui_loading", false), ("__openui_refetching", true)] {
            let expected: Vec<_> = self
                .order
                .iter()
                .filter(|id| {
                    loading.contains(id.as_str())
                        && (!history || self.queries.get(*id).is_some_and(|q| q.ever_fetched))
                })
                .map(|id| Value::Str(id.clone()))
                .collect();
            if !matches!(self.snapshot.get(name),Some(Value::Arr(v)) if v.as_ref()==&expected) {
                return Err(format!("rule 2: {name} differs from outstanding calls"));
            }
        }
        if self.disposed || !registered {
            return Ok(());
        }
        // Every wait must have an edge to a justified terminal or another
        // waiting registration. In a finite graph, following those edges
        // reaches a terminal or proves a cycle (including a self-read).
        for (id, missing) in &waits {
            let self_delivery = missing.is_empty()
                && self.queries.get(*id).is_some_and(|q| {
                    self.cache.get(&q.key).is_some_and(|e| e.data.is_some())
                        && q.reads.iter().any(|r| r.id == *id && self.read_changed(r))
                });
            let bounded_wait = missing.is_empty()
                && self
                    .readers
                    .as_ref()
                    .is_some_and(|plan| plan.rounds_limited)
                && self.queries.get(*id).is_some_and(|q| {
                    q.reads
                        .iter()
                        .any(|r| r.version.is_none() && self.read_changed(r))
                });
            if self_delivery || bounded_wait {
                continue;
            }
            if !missing.iter().any(|src| {
                self.queries.get(*src).is_some_and(|q| {
                    (q.suspended && self.reader_limit.is_some())
                        || live.contains(&q.key)
                        || self
                            .cache
                            .get(&q.key)
                            .is_some_and(|e| e.failed && shows_failure(src, q))
                        || waits.contains_key(*src)
                })
            }) {
                return Err(format!(
                    "rule 4: wait at {id} has no pending, failed, suspended or cyclic source ({missing:?})"
                ));
            }
        }
        Ok(())
    }

    /// Check delivery to exactly the holders at the instant the outcome arrives,
    /// before reader expressions can move their keys. Dropping the probe keeps
    /// the original receipt live. Outside the supported interface.
    #[doc(hidden)]
    pub fn audit_delivery(
        &mut self,
        id: &ToolCallId,
        outcome: &ToolOutcome<'_>,
    ) -> Result<(), String> {
        let Ok(stage) = self.stage_borrowed(id, outcome.clone()) else {
            return Ok(());
        };
        stage.audit_outcome(outcome)
    }
}

impl ToolDelivery<'_> {
    fn audit_outcome(&self, outcome: &ToolOutcome<'_>) -> Result<(), String> {
        let id = self.id;
        let before = &self.manager;
        let after = &self.candidate;
        // Determine the target by identity, independently of the key on the receipt.
        let key = before.cache.iter().find_map(|(key, entry)| {
            let pending = entry.pending.as_ref()?;
            matches!(&id.0.target, CallTarget::Query(weak) if weak.ptr_eq(&Arc::downgrade(pending)))
                .then_some(key)
        });
        let delivered = !before.disposed
            && id.0.lifecycle.strong_count() != 0
            && key.is_some_and(|k| before.queries.values().any(|q| !q.suspended && &q.key == k));
        let success = self.result_refusal().is_none()
            && matches!(
                outcome,
                ToolOutcome::Result(_) | ToolOutcome::ResultValue(_)
            );
        for (id, old) in &before.queries {
            let Some(q) = after.queries.get(id) else {
                return Err(format!("rule 3: delivery removed {id}"));
            };
            if !old.suspended && delivered && Some(&old.key) == key {
                if success {
                    if !q.ever_fetched || q.previous.is_some() || q.error.is_some() {
                        return Err(format!("rule 3: success missed {id}"));
                    }
                } else if q.ever_fetched != old.ever_fetched
                    || q.previous != old.previous
                    || !matches!(&q.error,Some(Value::Obj(e)) if matches!(e.get("statementId"),Some(Value::Str(s)) if s==id))
                {
                    return Err(format!("rule 3: failure missed {id}"));
                }
            } else if q.ever_fetched != old.ever_fetched
                || q.previous != old.previous
                || q.error != old.error
                || q.withheld != old.withheld
            {
                return Err(format!("rule 3: delivery changed unrelated query {id}"));
            }
        }
        for (k, entry) in &after.cache {
            let Some(old) = before.cache.get(k) else {
                return Err(format!("rule 3: delivery created {k}"));
            };
            let expected_pending = if Some(k) == key {
                None
            } else {
                old.pending.as_ref()
            };
            if entry.pending.as_ref().map(Arc::as_ptr) != expected_pending.map(Arc::as_ptr) {
                return Err(format!("rule 3: wrong receipt ended for {k}"));
            }
            if delivered && Some(k) == key && success {
                let actual = entry.data.as_deref();
                let correct = match outcome {
                    ToolOutcome::Result(Some(json)) => actual == Some(&Value::from_json(json)),
                    ToolOutcome::Result(None) => matches!(actual, Some(Value::Null)),
                    // Host containers retain their identity, even with nested NaN.
                    ToolOutcome::ResultValue(v) => crate::store::same(actual, Some(v)),
                    _ => false,
                };
                if entry.failed || entry.result_version == old.result_version || !correct {
                    return Err(format!("rule 3: wrong data delivered to {k}"));
                }
            } else if (delivered && Some(k) == key && !entry.failed)
                || entry.result_version != old.result_version
                || entry.data.as_ref().map(Arc::as_ptr) != old.data.as_ref().map(Arc::as_ptr)
                || ((!delivered || Some(k) != key) && entry.failed != old.failed)
            {
                return Err(format!("rule 3: unexpected change to {k}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered() -> (QueryManager, ToolCallId) {
        let library = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let program = openui_lang::parse(
            "a=Query(\"shared\")\nb=Query(\"shared\")\nr=Query(\"r\",{x:a})",
            &library,
            None,
        );
        let mut manager = QueryManager::default();
        let update = manager
            .apply(QueryCommand::Evaluate {
                program: &program,
                store: &Store::default(),
            })
            .unwrap();
        assert_eq!(update.calls.len(), 1);
        let id = update.calls.into_iter().next().unwrap().id;
        manager.audit_cache(std::iter::once(&id), true).unwrap();
        (manager, id)
    }

    #[test]
    fn the_output_audit_rejects_retired_duplicate_and_unowned_requests() {
        let (mut manager, id) = registered();
        let call = || ToolCall {
            id: id.clone(),
            tool_name: "shared".into(),
            args: None,
        };
        manager.audit_output(&[call()]).unwrap();
        assert!(
            manager
                .audit_output(&[call(), call()])
                .unwrap_err()
                .contains("two calls")
        );
        let live = id.query_key().unwrap();
        let entry = manager.cache.remove(&live).unwrap();
        // A transaction candidate can keep the weak target alive even when the
        // command's own manager has no entry. Liveness alone is insufficient.
        assert!(id.is_pending());
        assert!(
            manager
                .audit_output(&[call()])
                .unwrap_err()
                .contains("current entry")
        );
        manager.cache.insert(live.clone(), entry);
        manager.queries.retain(|_, q| q.key != live);
        assert!(
            manager
                .audit_output(&[call()])
                .unwrap_err()
                .contains("current entry")
        );
        manager.cache.remove(&live);
        assert!(
            manager
                .audit_output(&[call()])
                .unwrap_err()
                .contains("retired")
        );
    }

    #[test]
    fn an_orphan_below_the_two_entry_bound_still_violates_ownership() {
        let (mut m, id) = registered();
        m.cache.insert(Arc::from("orphan"), CacheEntry::default());
        assert!(m.cache.len() <= 2 * m.queries.len());
        assert!(
            m.audit_cache(std::iter::once(&id), true)
                .unwrap_err()
                .contains("rule 1")
        );
    }

    #[test]
    fn a_loading_flag_requires_a_receipt_in_the_driver() {
        let (m, _) = registered();
        assert!(
            m.audit_cache(std::iter::empty(), true)
                .unwrap_err()
                .contains("rule 2")
        );
    }

    #[test]
    fn a_delivery_that_skips_a_current_holder_fails_the_recipient_check() {
        let (mut m, id) = registered();
        let value = Value::Arr(Arc::new(vec![Value::Num(f64::NAN)]));
        let outcome = ToolOutcome::ResultValue(&value);
        m.audit_delivery(&id, &outcome).unwrap();
        let mut staged = m.stage_borrowed(&id, outcome.clone()).unwrap();
        staged.candidate.queries.get_mut("b").unwrap().ever_fetched = false;
        assert!(
            staged
                .audit_outcome(&outcome)
                .unwrap_err()
                .contains("rule 3")
        );
    }

    #[test]
    fn a_failed_entry_needs_a_receipt_for_a_new_holder_without_a_visible_error() {
        let lib = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let store = Store::default();
        let mut manager = QueryManager::default();
        let first = openui_lang::parse("a=Query(\"u\")", &lib, None);
        let call = manager
            .apply(QueryCommand::Evaluate {
                program: &first,
                store: &store,
            })
            .unwrap()
            .calls
            .pop()
            .unwrap();
        manager
            .apply(QueryCommand::Complete {
                id: &call.id,
                outcome: ToolOutcome::CallFailed("offline"),
            })
            .unwrap();
        manager.audit_cache(std::iter::empty(), true).unwrap();
        let shared = openui_lang::parse("a=Query(\"u\")\nb=Query(\"u\")", &lib, None);
        let update = manager
            .apply(QueryCommand::Evaluate {
                program: &shared,
                store: &store,
            })
            .unwrap();
        manager
            .audit_cache(update.calls.iter().map(|c| &c.id), true)
            .unwrap();
        assert_eq!(update.calls.len(), 1);

        // A private error cannot excuse an idle registration if it was never published.
        let key = manager.queries["b"].key.clone();
        manager.cache.get_mut(&key).unwrap().pending = None;
        manager.rebuild_snapshot();
        manager.queries.get_mut("b").unwrap().error = manager.queries["a"].error.clone();
        assert!(
            manager
                .audit_cache(std::iter::empty(), true)
                .unwrap_err()
                .contains("rule 4: idle query b")
        );
    }

    #[test]
    fn waiting_on_a_missing_version_alone_does_not_explain_a_wait() {
        let (mut m, id) = registered();
        m.queries.get_mut("r").unwrap().reads = vec![Read {
            id: "removed".into(),
            version: None,
            primitive: None,
        }]
        .into();
        m.rebuild_snapshot();
        assert!(
            m.audit_cache(std::iter::once(&id), true)
                .unwrap_err()
                .contains("rule 4")
        );
    }
}
