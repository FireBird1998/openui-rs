//! Conformance drivers derive ownership from registrations and loading from
//! embedder-held receipts, independently of cache counters and pending_queries.
use super::*;

// apply_nodes carries registrations by statement id. A replacement program
// starts with just the registrations it inherits, including incomplete ids.
// Dropping an id from the program ends its history even if it returns later.
#[cfg(feature = "query-audit")]
#[derive(Default)]
pub(super) struct RegistrationHistory {
    program: Option<Arc<ParseResult>>,
    registered: HashSet<String>,
    retained_waits: HashMap<String, Arc<[Read]>>,
}

#[cfg(feature = "query-audit")]
impl RegistrationHistory {
    // Completeness is mutable evaluation metadata. A borrowed caller can
    // also replace the program in place, so its address cannot identify it.
    fn same_program(&self, program: &ParseResult) -> bool {
        let Some(previous) = &self.program else {
            return false;
        };
        if std::ptr::eq(previous.as_ref(), program) {
            return true;
        }
        let mut comparable = program.clone();
        for (new, old) in comparable
            .query_statements
            .iter_mut()
            .zip(&previous.query_statements)
        {
            new.complete = old.complete;
        }
        comparable == **previous
    }
}

impl QueryManager {
    #[cfg(feature = "query-audit")]
    pub(super) fn observe_registration(&mut self, input: &TreeInput<'_>, before: &Self) {
        let program = input.result;
        let history = &before.registration_history;
        // Only consult program identity if the registration was already
        // missing before this Evaluate. Live registrations carry over by id.
        let same_program = std::cell::LazyCell::new(|| history.same_program(program));
        let mut next = RegistrationHistory {
            program: Some(
                input
                    .shared_result
                    .cloned()
                    .unwrap_or_else(|| Arc::new(program.clone())),
            ),
            ..Default::default()
        };
        for s in &program.query_statements {
            let id = &s.statement_id;
            if self.queries.contains_key(id)
                || before.queries.contains_key(id)
                || (history.registered.contains(id) && *same_program)
            {
                next.registered.insert(id.clone());
            }
            if !s.complete
                && let Some(q) = before.queries.get(id).filter(|q| q.waiting())
            {
                next.retained_waits.insert(id.clone(), q.reads.clone());
            }
        }
        // A complete statement with the same id replaces the retained wait.
        for s in program.query_statements.iter().filter(|s| s.complete) {
            next.retained_waits.remove(&s.statement_id);
        }
        self.registration_history = Arc::new(next);
    }

    #[cfg(feature = "query-audit")]
    pub(super) fn explains_suspension(&self, id: &str, query: &Query) -> bool {
        self.reader_limit.is_some()
            && !query.withheld
            && matches!((query.defaults.as_deref(), &query.error),
                (Some(Value::Null), Some(Value::Obj(e)))
                if matches!(e.get("code"), Some(Value::Str(s)) if s == "reader-limit")
                    && matches!(e.get("statementId"), Some(Value::Str(s)) if s == id))
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
            .filter(|q| !q.suspended())
            .map(|q| &q.key)
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
                    if !current.contains(&pending.key)
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
    #[cfg(feature = "query-audit")]
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
            // A stale entry owes one run (#119): starting it clears the mark.
            if e.stale && e.pending.is_some() {
                return Err(format!("rule 5: stale entry {key} has a call in flight"));
            }
        }
        // A wait propagates loading transitively. Follow edges from real
        // receipts, so an unexplained waiting flag cannot establish loading.
        let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut loading = HashSet::new();
        let mut queue = Vec::new();
        let mut waits = HashMap::new();
        let errors = match self.snapshot().get("__openui_errors") {
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
            if q.suspended() {
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
            } else if registered && !self.disposed && entry.stale {
                // A stale entry is current for no ready query: one that makes
                // it current again requests the run it owes, whatever its
                // data or error (#119).
                return Err(format!(
                    "rule 5: query {id} shows a stale entry without refetching"
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
            if !matches!(self.snapshot().get(name),Some(Value::Arr(v)) if v.as_ref()==&expected) {
                return Err(format!("rule 2: {name} differs from outstanding calls"));
            }
        }
        if self.disposed || !registered {
            return Ok(());
        }
        // Every wait must have an edge to a justified terminal or another
        // waiting registration. In a finite graph, following those edges
        // reaches a terminal or proves a cycle (including a self-read).
        //
        // One terminal is upstream's (#188, ADR 0013): the parser can list a
        // statement that is not a query in a reader's waitFor (a Query wrapped
        // in `@toString(...)`, quirk 2). Nothing can deliver to it, so the
        // reader keeps its default with no call, as lang-core 1.1.0 does. A
        // never-registered source with only incomplete statements is also a
        // terminal (#210). A source registered before it became incomplete
        // must still be present. Without a reader plan a missing source alone
        // cannot explain a wait.
        let is_query_statement = |src: &str| {
            self.readers.as_ref().is_none_or(|plan| {
                plan.program
                    .query_statements
                    .iter()
                    .any(|s| s.complete && s.statement_id == src)
            })
        };
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
            if let Some(src) = missing.iter().find(|src| {
                !self.queries.contains_key(**src)
                    && (self.registration_history.registered.contains(**src)
                        || self.registration_history.program.as_ref().is_some_and(|p| {
                            p.query_statements
                                .iter()
                                .any(|s| s.complete && s.statement_id == **src)
                        }))
            }) {
                return Err(format!(
                    "rule 4: wait at {id} lost source registration {src}"
                ));
            }
            // A skipped reader can keep these exact reads even if an input
            // delivered or left the program since the reader last evaluated.
            if self
                .registration_history
                .retained_waits
                .get(*id)
                .is_some_and(|reads| {
                    !reads.is_empty() && Arc::ptr_eq(reads, &self.queries[*id].reads)
                })
            {
                continue;
            }
            if self_delivery || bounded_wait {
                continue;
            }
            if !missing.iter().any(|src| {
                (!self.registration_history.registered.contains(*src) && !is_query_statement(src))
                    || self.queries.get(*src).is_some_and(|q| {
                        (q.suspended() && self.reader_limit.is_some())
                            || live.contains(&q.key)
                            || self
                                .cache
                                .get(&q.key)
                                .is_some_and(|e| e.failed && shows_failure(src, q))
                            || waits.contains_key(*src)
                    })
            }) {
                return Err(format!(
                    "rule 4: wait at {id} has no pending, failed, suspended, cyclic, non-query or incomplete source ({missing:?})"
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
            && key.is_some_and(|k| {
                before
                    .queries
                    .values()
                    .any(|q| !q.suspended() && &q.key == k)
            });
        let success = self.result_refusal().is_none()
            && matches!(
                outcome,
                ToolOutcome::Result(_) | ToolOutcome::ResultValue(_)
            );
        for (id, old) in &before.queries {
            let Some(q) = after.queries.get(id) else {
                return Err(format!("rule 3: delivery removed {id}"));
            };
            if !old.suspended() && delivered && Some(&old.key) == key {
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
                // By identity: an error's raw tool name can hold NaN, which
                // never equals itself (fuzz crash 5ca76c7b, #119).
                || !crate::store::same(q.error.as_ref(), old.error.as_ref())
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

    fn register_one_call(manager: &mut QueryManager, source: &str) -> ToolCallId {
        let library = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let program = openui_lang::parse(source, &library, None);
        let update = manager
            .apply(QueryCommand::Evaluate {
                program: &program,
                store: &Store::default(),
            })
            .unwrap();
        manager.audit_output(&update.calls).unwrap();
        let [call] = update.calls.try_into().expect("one initial call");
        manager
            .audit_cache(std::iter::once(&call.id), true)
            .unwrap();
        call.id
    }

    fn registered() -> (QueryManager, ToolCallId) {
        let mut manager = QueryManager::default();
        let id = register_one_call(
            &mut manager,
            "a=Query(\"shared\")\nb=Query(\"shared\")\nr=Query(\"r\",{x:a})",
        );
        (manager, id)
    }

    fn settle(manager: &mut QueryManager, id: &ToolCallId, outcome: ToolOutcome<'_>) {
        let update = manager
            .apply(QueryCommand::Complete { id, outcome })
            .unwrap();
        assert!(update.calls.is_empty());
        manager.audit_cache(std::iter::once(id), true).unwrap();
    }

    #[test]
    fn a_suspended_reader_requires_its_propagation_limit() {
        let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1024));
        let call = register_one_call(&mut manager, "s=Query(\"s\")\nr=Query(\"r\",{x:s+s})");
        let long = JsonValue::from("x".repeat(700));
        settle(&mut manager, &call, ToolOutcome::Result(Some(&long)));
        assert!(manager.queries["r"].suspended());
        assert!(manager.reader_limit.take().is_some());
        assert_eq!(
            manager.audit_cache(std::iter::empty(), true).unwrap_err(),
            "rule 4: unexplained suspended reader r"
        );
    }

    #[test]
    fn an_idle_query_requires_its_own_visible_or_withheld_error() {
        let mut manager = QueryManager::default();
        let call = register_one_call(&mut manager, "q=Query(\"q\")");
        settle(&mut manager, &call, ToolOutcome::CallFailed("offline"));
        let audit = |m: &QueryManager| m.audit_cache(std::iter::empty(), true);
        let idle = "rule 4: idle query q has no result, error or receipt";

        // Leave the matching statement id in the snapshot and change only its source.
        let mut error = manager.queries["q"].error.clone().unwrap();
        let Value::Obj(record) = &mut error else {
            panic!("expected a query error record");
        };
        Arc::make_mut(record).set("source", Value::Str("mutation".into()));
        manager.set_snapshot_field_for_test("__openui_errors", Value::Arr(Arc::new(vec![error])));
        assert_eq!(audit(&manager).unwrap_err(), idle);

        manager.queries.get_mut("q").unwrap().withheld = true;
        manager.rebuild_snapshot();
        audit(&manager).unwrap();
        manager.set_snapshot_field_for_test("__openui_errors", Value::Arr(Arc::new(vec![])));
        assert_eq!(audit(&manager).unwrap_err(), idle);
    }

    #[test]
    fn a_self_delivery_can_explain_a_wait_on_the_readers_own_key() {
        // Both statements hold the same key before and after delivery.
        let mut manager = QueryManager::default();
        let call = register_one_call(
            &mut manager,
            "a=Query(\"shared\",{x:1})\nr=Query(\"shared\",{x:r ? 1 : 1})",
        );
        let before = manager.queries["r"].reads.clone();
        settle(&mut manager, &call, ToolOutcome::Result(Some(&1.into())));

        // Probe the audit with the wait and reads from before self-delivery.
        let r = manager.queries.get_mut("r").unwrap();
        r.waiting_for_self = true;
        r.reads = before;
        manager.audit_cache(std::iter::empty(), true).unwrap();
    }

    #[test]
    fn a_wait_on_a_ready_peer_requires_a_round_cut_and_a_previously_missing_read() {
        let mut manager = QueryManager::default();
        let call = register_one_call(
            &mut manager,
            "a=Query(\"shared\",{x:1})\nr=Query(\"shared\",{x:a ? 1 : 1})",
        );
        let missing = manager.queries["r"].reads.clone();
        settle(&mut manager, &call, ToolOutcome::Result(Some(&1.into())));
        let had_one = manager.queries["r"].reads.clone();

        let update = manager
            .apply(QueryCommand::Invalidate {
                statement_ids: &["a"],
            })
            .unwrap();
        manager.audit_output(&update.calls).unwrap();
        let [refresh] = update.calls.try_into().expect("one shared refetch");
        manager
            .audit_cache(std::iter::once(&refresh.id), true)
            .unwrap();
        settle(
            &mut manager,
            &refresh.id,
            ToolOutcome::Result(Some(&2.into())),
        );

        // Replay the two earlier observations against the settled key. Only
        // the missing-to-ready transition at a round cut can excuse this wait.
        let r = manager.queries.get_mut("r").unwrap();
        r.waiting_for_other = true;
        r.reads = missing;
        let audit = |m: &QueryManager| m.audit_cache(std::iter::empty(), true);
        let unexplained = "rule 4: wait at r has no pending, failed, suspended, cyclic, non-query or incomplete source ([])";
        assert_eq!(audit(&manager).unwrap_err(), unexplained);
        manager.readers.as_mut().unwrap().rounds_limited = true;
        audit(&manager).unwrap();
        manager.queries.get_mut("r").unwrap().reads = had_one;
        assert_eq!(audit(&manager).unwrap_err(), unexplained);
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
        m.cache.insert(
            CacheKey::new("orphan".into(), 0, m.cache.hasher()),
            CacheEntry::default(),
        );
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
    fn a_stale_entry_has_no_call_and_no_ready_current_holder() {
        let (mut m, id) = registered();
        let value = JsonValue::from(1);
        let update = m
            .apply(QueryCommand::Complete {
                id: &id,
                outcome: ToolOutcome::Result(Some(&value)),
            })
            .unwrap();
        let reader = &update.calls[0].id;
        m.audit_cache(std::iter::once(reader), true).unwrap();
        // Current for ready queries that are not refetching: the owed run is lost.
        let shared = m.queries["a"].key.clone();
        m.cache.get_mut(&shared).unwrap().stale = true;
        assert!(
            m.audit_cache(std::iter::once(reader), true)
                .unwrap_err()
                .contains("shows a stale entry without refetching")
        );
        // A stale entry with a call in flight was not cleared by the call.
        m.cache.get_mut(&shared).unwrap().stale = false;
        let key = m.queries["r"].key.clone();
        m.cache.get_mut(&key).unwrap().stale = true;
        assert!(
            m.audit_cache(std::iter::once(reader), true)
                .unwrap_err()
                .contains("rule 5: stale entry")
        );
    }

    fn reading(m: &mut QueryManager, source: &str) {
        m.queries.get_mut("r").unwrap().reads = vec![Read {
            id: source.into(),
            version: None,
            primitive: None,
        }]
        .into();
        m.rebuild_snapshot();
    }

    #[test]
    fn a_wait_on_a_query_statement_without_its_registration_is_reported() {
        // `b` is a query statement of the registered program. Without its
        // registration nothing explains the wait.
        let (mut m, id) = registered();
        let key = m.queries.remove("b").unwrap().key;
        m.cache.get_mut(&key).unwrap().references -= 1;
        m.order.retain(|q| q != "b");
        reading(&mut m, "b");
        assert!(
            m.audit_cache(std::iter::once(&id), true)
                .unwrap_err()
                .contains("rule 4: wait at r")
        );
    }

    #[test]
    fn a_wait_on_a_statement_that_is_not_a_query_is_upstreams_terminal() {
        // #188: the parser can list a statement that is not a query in a
        // wait list. Nothing delivers to it, as in lang-core.
        let (mut m, id) = registered();
        reading(&mut m, "removed");
        m.audit_cache(std::iter::once(&id), true).unwrap();
        // Every other rule 4 check still runs: an idle registration without
        // a result, an error or a call is reported.
        let key = m.queries["a"].key.clone();
        m.cache.get_mut(&key).unwrap().pending = None;
        m.rebuild_snapshot();
        assert!(
            m.audit_cache(std::iter::empty(), true)
                .unwrap_err()
                .contains("rule 4: idle query")
        );
    }

    fn waiting_on_incomplete_source() -> QueryManager {
        let library = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let mut program =
            openui_lang::parse("r=Query(\"r\",{x:s},42)\ns=Query(\"s\")", &library, None);
        program.query_statements[1].complete = false;
        let mut manager = QueryManager::default();
        let update = manager
            .apply(QueryCommand::Evaluate {
                program: &program,
                store: &Store::default(),
            })
            .unwrap();
        assert!(update.calls.is_empty());
        manager.audit_cache(std::iter::empty(), true).unwrap();
        manager
    }

    #[test]
    fn an_incomplete_source_does_not_excuse_an_idle_reader() {
        let mut manager = waiting_on_incomplete_source();
        manager.queries.get_mut("r").unwrap().waiting_for_other = false;
        assert!(
            manager
                .audit_cache(std::iter::empty(), true)
                .unwrap_err()
                .contains("rule 4: idle query r")
        );
    }

    #[test]
    fn a_complete_statement_with_the_same_id_still_needs_a_registration() {
        let mut manager = waiting_on_incomplete_source();
        let plan = manager.readers.as_mut().unwrap();
        let statements = &mut Arc::make_mut(&mut plan.program).query_statements;
        let mut complete = statements[1].clone();
        complete.complete = true;
        statements.push(complete);
        assert!(
            manager
                .audit_cache(std::iter::empty(), true)
                .unwrap_err()
                .contains("rule 4: wait at r")
        );
    }

    fn skip_reader(
        manager: &mut QueryManager,
        source_complete: bool,
        other_statements: &str,
    ) -> Vec<ToolCall> {
        let library = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let mut program = openui_lang::parse(
            &format!(
                "a=Query(\"shared\")\nb=Query(\"shared\")\nr=Query(\"r\",{{x:a}})\n{other_statements}"
            ),
            &library,
            None,
        );
        program.query_statements[0].complete = source_complete;
        program.query_statements[2].complete = false;
        manager
            .apply(QueryCommand::Evaluate {
                program: &program,
                store: &Store::default(),
            })
            .unwrap()
            .calls
    }

    #[test]
    fn a_retained_wait_still_needs_its_source_registration() {
        // A second active reader keeps the plan. Without it the plan is gone.
        let other = "other=Query(\"other\",{x:b})";
        for (source_complete, extra) in [(true, ""), (false, ""), (true, other), (false, other)] {
            let (mut manager, id) = registered();
            assert!(skip_reader(&mut manager, source_complete, extra).is_empty());
            assert_eq!(manager.readers.is_some(), !extra.is_empty());
            manager.audit_cache(std::iter::once(&id), true).unwrap();
            let key = manager.queries.remove("a").unwrap().key;
            manager.cache.get_mut(&key).unwrap().references -= 1;
            manager.order.retain(|q| q != "a");
            manager.rebuild_snapshot();
            assert!(
                manager
                    .audit_cache(std::iter::once(&id), true)
                    .unwrap_err()
                    .contains("rule 4: wait at r")
            );
        }
    }

    #[test]
    fn only_a_committed_program_change_resets_registration_history() {
        let (mut manager, receipt) = registered();
        let lib = openui_lang::Library::from_json_schema(&JsonValue::Null);
        let store = Store::default();
        let mut program = openui_lang::parse(
            "a=Query(\"shared\")\nb=Query(\"shared\")\nr=Query(\"r\",{x:a})",
            &lib,
            None,
        );
        program.query_statements[0].complete = false;
        let evaluate = |manager: &mut QueryManager, program: &ParseResult| {
            manager
                .apply(QueryCommand::Evaluate {
                    program,
                    store: &store,
                })
                .unwrap()
        };
        assert!(evaluate(&mut manager, &program).calls.is_empty());

        // Simulate a coherent removal, leaving the shared entry held by b.
        let key = manager.queries.remove("a").unwrap().key;
        manager.cache.get_mut(&key).unwrap().references -= 1;
        manager.order.retain(|id| id != "a");
        manager.rebuild_snapshot();
        let lost = "rule 4: wait at r lost source registration a";
        let audit = |m: &QueryManager| m.audit_cache(std::iter::once(&receipt), true);
        assert_eq!(audit(&manager).unwrap_err(), lost);
        program.query_statements[1].complete = false;
        for command in [
            QueryCommand::Complete {
                id: &receipt,
                outcome: ToolOutcome::Result(None),
            },
            QueryCommand::AdvanceClock { now_ms: 10 },
            QueryCommand::Invalidate {
                statement_ids: &["r"],
            },
            QueryCommand::Evaluate {
                program: &program,
                store: &store,
            },
        ] {
            assert!(manager.apply(command).unwrap().calls.is_empty());
            assert_eq!(audit(&manager).unwrap_err(), lost);
        }

        manager.apply(QueryCommand::Dispose).unwrap();
        manager.apply(QueryCommand::Activate).unwrap();
        assert_eq!(audit(&manager).unwrap_err(), lost);

        // The replacement uses exactly the same ids. It can inherit b and
        // r, but its incomplete a has never been registered in this program.
        program.query_statements[2].defaults = Some(Ast::Num(42.0));
        let refused: Result<_, QueryCommandError> =
            manager.evaluate_transaction(&TreeInput::new(&program), &store, |_| {
                Err::<(), _>(QueryCommandError::InvalidClock)
            });
        assert!(refused.is_err());
        assert_eq!(audit(&manager).unwrap_err(), lost);
        assert!(evaluate(&mut manager, &program).calls.is_empty());
        audit(&manager).unwrap();
    }

    #[test]
    fn a_retained_wait_can_keep_a_source_that_never_registered() {
        let mut manager = waiting_on_incomplete_source();
        let mut program = (*manager.readers.as_ref().unwrap().program).clone();
        // s was already incomplete and absent. Now r is skipped as well.
        program.query_statements[0].complete = false;
        let update = manager
            .apply(QueryCommand::Evaluate {
                program: &program,
                store: &Store::default(),
            })
            .unwrap();
        assert!(update.calls.is_empty());
        assert!(manager.readers.is_none());
        manager.audit_cache(std::iter::empty(), true).unwrap();
    }

    #[test]
    fn a_retained_wait_only_explains_the_reads_it_kept() {
        let (mut manager, id) = registered();
        assert!(skip_reader(&mut manager, true, "").is_empty());
        let calls = manager
            .apply(QueryCommand::Complete {
                id: &id,
                outcome: ToolOutcome::Result(Some(&JsonValue::from(1))),
            })
            .unwrap()
            .calls;
        assert!(calls.is_empty());
        manager.audit_cache(std::iter::once(&id), true).unwrap();
        reading(&mut manager, "b");
        assert!(
            manager
                .audit_cache(std::iter::once(&id), true)
                .unwrap_err()
                .contains("rule 4: wait at r")
        );
    }

    #[test]
    fn skipping_a_ready_reader_does_not_explain_a_new_wait() {
        let (mut manager, id) = registered();
        let calls = manager
            .apply(QueryCommand::Complete {
                id: &id,
                outcome: ToolOutcome::Result(Some(&JsonValue::from(1))),
            })
            .unwrap()
            .calls;
        assert_eq!(calls.len(), 1);
        assert!(skip_reader(&mut manager, true, "").is_empty());
        let reader = &calls[0].id;
        manager.audit_cache(std::iter::once(reader), true).unwrap();
        manager.queries.get_mut("r").unwrap().waiting_for_other = true;
        assert!(
            manager
                .audit_cache(std::iter::once(reader), true)
                .unwrap_err()
                .contains("rule 4: wait at r")
        );
    }

    #[test]
    fn a_wait_on_an_absent_source_without_a_reader_plan_is_reported() {
        let mut manager = waiting_on_incomplete_source();
        manager.readers = None;
        assert!(
            manager
                .audit_cache(std::iter::empty(), true)
                .unwrap_err()
                .contains("rule 4: wait at r")
        );
    }
}
