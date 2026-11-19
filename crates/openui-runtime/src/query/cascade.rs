//! One trigger compares all readers once. Later rounds revisit readers whose
//! source could have moved, in declaration order. All expressions in a round
//! see the same registrations before any of its rebuilt nodes are applied.
use super::*;
use std::cell::OnceCell;
use std::collections::BTreeSet;

pub(super) struct RoundResults {
    initial: HashMap<String, u64>,
    // A key can become visible again through another holder or a previous
    // slot. No new result body can arrive in the middle of one cascade.
    versions: HashMap<u64, OnceCell<PreparedReferenceValue>>,
}

impl QueryScope<'_, '_> {
    pub(super) fn read_reference(
        &self,
        name: &str,
    ) -> Result<Option<&PreparedReferenceValue>, EvaluationFlags> {
        if let Some(round) = self.cascade
            && !self.manager.mutations.contains(name)
        {
            let Some(entry) = self
                .manager
                .queries
                .get(name)
                .and_then(|q| self.manager.data_entry(q))
            else {
                return Ok(None);
            };
            if round.initial.get(name) != Some(&entry.result_version) {
                return Ok(round
                    .versions
                    .get(&entry.result_version)
                    .zip(entry.data.as_deref())
                    .map(|(cell, value)| cell.get_or_init(|| PreparedReferenceValue::new(value))));
            }
        }
        Ok(self.prepared.get(name)?.map(|(_, receipt)| receipt))
    }
}

impl QueryManager {
    // Charge key hashes and primitive comparisons before probing a reader.
    fn charge_reader(
        &self,
        node: &Node,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<(), QueryCommandError> {
        eval.charge_work(node.id.len().saturating_add(1));
        for read in node.reads.iter() {
            eval.charge_work(read.id.len().saturating_add(1));
            check_cuts(eval.flags())?;
            if let Some(q) = self.queries.get(&read.id) {
                eval.charge_work(
                    q.key
                        .len()
                        .saturating_add(q.previous.as_ref().map_or(0, |k| k.len())),
                );
                check_cuts(eval.flags())?;
                if let Some(Value::Str(text)) = read.primitive.as_deref()
                    && read.version != self.version(&read.id)
                {
                    eval.charge_work(text.len());
                }
            }
        }
        check_cuts(eval.flags())
    }

    pub(super) fn propagate(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
        budget: &EvaluationSession<'_>,
        measured: &PreparedProgram,
        mut nodes: Arc<[Arc<Node>]>,
        requests: &mut runs::QueryRequests,
    ) -> Result<Arc<[Arc<Node>]>, QueryCommandError> {
        if !nodes.iter().any(|n| self.stale(n)) {
            return Ok(nodes);
        }
        let prepared = self.statement_results()?;
        let tree = TreeScope::new(input, store, None, self.limits);
        let results = RoundResults {
            initial: self
                .queries
                .iter()
                .filter_map(|(id, q)| self.data_entry(q).map(|e| (id.clone(), e.result_version)))
                .collect(),
            versions: self
                .cache
                .values()
                .filter(|e| e.data.is_some())
                .map(|e| (e.result_version, OnceCell::new()))
                .collect(),
        };
        let statements: Vec<_> = input
            .result
            .query_statements
            .iter()
            .filter(|s| s.complete)
            .collect();
        let mut candidates = BTreeSet::new();
        let mut ids = HashSet::new();
        let mut duplicate = false;
        let mut dependents = HashMap::<&str, Vec<usize>>::new();
        for (i, statement) in statements.iter().enumerate() {
            duplicate |= !ids.insert(&statement.statement_id);
            for source in statement.wait_for.iter().flatten() {
                candidates.insert(i);
                dependents.entry(source).or_default().push(i);
            }
        }
        // A manually constructed program can register the same id twice. Keep
        // declaration-order last writes, including those of non-reader nodes.
        if duplicate {
            candidates.extend(0..statements.len());
        }
        budget.with_evaluator(|eval| {
            eval.supply_input(openui_lang::SuppliedInput {
                program: openui_lang::SuppliedProgram::Prepared(measured),
                start: openui_lang::SuppliedStart::Raw {
                    action: None,
                    message: "",
                    form: None,
                },
                references: Some(&prepared.input),
            })
            .map_err(|_| QueryCommandError::Limit(crate::admission::size_error()))
        })?;
        let mut timers = self.clock.edit();
        let nodes_mut = Arc::make_mut(&mut nodes);
        let mut finished = false;
        for _ in 0..=input.result.query_statements.len() {
            let scope = QueryScope {
                manager: self,
                cloud_scripts: input.cloud_scripts,
                prepared: &prepared,
                cascade: Some(&results),
                calls: &tree,
                inspection: Cell::new(Inspection::None),
                inspected: Cell::new(None),
                seconds: Cell::new(0.0),
            };
            let changed = budget.with_scope(&scope, |session| {
                session.with_evaluator(|eval| {
                    let mut changed = BTreeSet::new();
                    for &i in &candidates {
                        let Some((node, statement)) = nodes_mut.get_mut(i).zip(statements.get(i))
                        else {
                            continue;
                        };
                        self.charge_reader(node, eval)?;
                        if self.stale(node) {
                            #[cfg(test)]
                            {
                                if changed.is_empty() {
                                    round_tests::begin();
                                }
                                round_tests::rebuilt(&node.id);
                            }
                            *node = Arc::new(scope.node(statement, eval)?);
                            changed.insert(i);
                        }
                    }
                    Ok::<_, QueryCommandError>(changed)
                })
            })?;
            if changed.is_empty() {
                finished = true;
                break;
            }
            // Reapply previous candidates for waiting transitions deferred by
            // declaration order. Also apply dependents of this round's moves.
            let next = budget.with_evaluator(|eval| {
                let mut next = changed.clone();
                for i in changed {
                    if let Some(s) = statements.get(i)
                        && let Some(readers) = dependents.get(s.statement_id.as_str())
                    {
                        eval.charge_work(readers.len());
                        check_cuts(eval.flags())?;
                        next.extend(readers);
                    }
                }
                if duplicate {
                    eval.charge_work(statements.len());
                    check_cuts(eval.flags())?;
                    next.extend(0..statements.len());
                }
                for &i in candidates.union(&next) {
                    if let Some(node) = nodes_mut.get(i) {
                        self.charge_reader(node, eval)?;
                        let text_len = |value: Option<&Value>| match value {
                            Some(Value::Str(s)) => s.len(),
                            _ => 0,
                        };
                        let old = self.queries.get(&node.id);
                        let bytes = node
                            .key
                            .len()
                            .saturating_add(old.map_or(0, |q| {
                                q.key
                                    .len()
                                    .saturating_add(q.previous.as_ref().map_or(0, |k| k.len()))
                            }))
                            .saturating_add(text_len(node.defaults.as_ref()))
                            .saturating_add(text_len(node.request.args.as_ref()))
                            .saturating_add(node.request.tool_name.len());
                        eval.charge_work(bytes);
                        check_cuts(eval.flags())?;
                        self.apply_node(node, requests);
                        if !self.waits(&node.id) {
                            timers.set(&node.id, node.interval.as_ref());
                        }
                    }
                }
                Ok::<_, QueryCommandError>(next)
            })?;
            candidates = next;
        }
        self.clock.install(timers);
        if !finished {
            #[cfg(test)]
            round_tests::remaining(self, nodes_mut);
            self.reader_rounds_limited = candidates
                .iter()
                .filter_map(|&i| nodes_mut.get(i))
                .any(|n| self.stale(n));
        }
        Ok(nodes)
    }
}
