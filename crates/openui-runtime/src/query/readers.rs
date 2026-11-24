//! Registered reader expressions, their last inputs, and bounded propagation.
use super::*;

pub(super) struct Read {
    pub(super) id: String,
    pub(super) version: Option<u64>,
    pub(super) primitive: Option<Arc<Value>>,
}

// A result version is unique in the manager. Only pairs used by the current
// reader plan survive publication. The table never owns result bodies.
pub(super) type StringComparisons = HashMap<(u64, u64), bool>;

pub(super) struct StringPair<'a> {
    pub(super) versions: (u64, u64),
    old: &'a str,
    new: &'a str,
}

impl StringPair<'_> {
    pub(super) fn equal(&self) -> bool {
        self.old == self.new
    }
}

impl Read {
    pub(super) fn string_pair<'a>(&'a self, entry: &'a CacheEntry) -> Option<StringPair<'a>> {
        let old = self.version?;
        let new = entry.result_version;
        if old == new {
            return None;
        }
        match (self.primitive.as_deref(), entry.data.as_deref()) {
            (Some(Value::Str(a)), Some(Value::Str(b))) if a.len() == b.len() => Some(StringPair {
                versions: (old, new),
                old: a,
                new: b,
            }),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct ReaderPlan {
    pub(super) program: Arc<ParseResult>,
    pub(super) measured: PreparedProgram,
    pub(super) needs_context: bool,
    // A skipped suspension keeps its diagnostic through this plan's deliveries.
    pub(super) retained_limit: Option<LimitError>,
    // The round cut belongs to this plan even after an unrelated clock or run command.
    pub(super) rounds_limited: bool,
    pub(super) store: crate::store::query_state::QueryStateReader,
    pub(super) nodes: Arc<[Arc<Node>]>,
}

impl QueryManager {
    // Use the version of the entry supplying getData, including previous-key
    // fallback. Distinct entries can each be on their first successful fetch.
    pub(super) fn version(&self, id: &str) -> Option<u64> {
        let q = self.queries.get(id)?;
        let entry = self.data_entry(q)?;
        Some(entry.result_version)
    }

    pub(super) fn data_entry(&self, q: &Query) -> Option<&CacheEntry> {
        if q.suspended() {
            return None;
        }
        std::iter::once(&q.key)
            .chain(q.previous.as_ref())
            .find_map(|key| self.cache.get(key).filter(|entry| entry.data.is_some()))
    }

    pub(super) fn read_results(
        &self,
        statement: &QueryStatement,
        eval: &ExpressionEvaluator<'_>,
    ) -> Result<Arc<[Read]>, QueryCommandError> {
        let mut reads = Vec::new();
        for id in statement.wait_for.iter().flatten() {
            eval.charge_work(id.len().saturating_add(1));
            check_cuts(eval.flags())?;
            let entry = self.queries.get(id).and_then(|q| self.data_entry(q));
            reads.push(Read {
                id: id.clone(),
                version: entry.map(|e| e.result_version),
                primitive: entry
                    .and_then(|e| e.data.as_ref())
                    .filter(|v| {
                        matches!(
                            v.as_ref(),
                            Value::Null | Value::Bool(_) | Value::Num(_) | Value::Str(_)
                        )
                    })
                    .cloned(),
            });
        }
        Ok(reads.into())
    }

    pub(super) fn stale(&self, node: &Node) -> bool {
        self.stale_with(node, &self.reader_comparisons)
    }

    pub(super) fn stale_with(&self, node: &Node, comparisons: &StringComparisons) -> bool {
        #[cfg(test)]
        round_tests::compared();
        if self.queries.get(&node.id).is_some_and(Query::suspended) {
            return false;
        }
        node.reads.iter().any(|read| {
            if read.id == node.id {
                // Waits are decided after installing the key. The expression
                // may already have seen this same value through a displaced key.
                // Only another holder can deliver to our previous entry.
                return self
                    .queries
                    .get(&node.id)
                    .is_some_and(|q| q.waiting_for_self && self.data_entry(q).is_some());
            }
            self.read_changed_with(read, comparisons)
        })
    }

    pub(super) fn read_changed(&self, read: &Read) -> bool {
        self.read_changed_with(read, &self.reader_comparisons)
    }

    fn read_changed_with(&self, read: &Read, comparisons: &StringComparisons) -> bool {
        let entry = self.queries.get(&read.id).and_then(|q| self.data_entry(q));
        if read.version == entry.map(|e| e.result_version) {
            // A native tool can supply NaN. JavaScript's strict equality
            // considers it changed even within the same result version.
            return matches!(read.primitive.as_deref(), Some(Value::Num(n)) if n.is_nan());
        }
        // A new entry/version can carry the same JS primitive. Running a
        // reader again here can observe a subsequent Store edit or callback.
        match (
            read.primitive.as_deref(),
            entry.and_then(|e| e.data.as_deref()),
        ) {
            (Some(Value::Null), Some(Value::Null)) => false,
            (Some(Value::Bool(a)), Some(Value::Bool(b))) => a != b,
            (Some(Value::Num(a)), Some(Value::Num(b))) => a != b,
            (Some(Value::Str(_)), Some(Value::Str(_))) => !entry
                .and_then(|entry| read.string_pair(entry))
                .is_some_and(|pair| comparisons.get(&pair.versions) == Some(&true)),
            _ => true,
        }
    }

    // Pay for equal-length string comparisons before any unmetered staleness
    // probe (including context selection, suspension and self diagnostics).
    // Every reader observing the same version pair gets the same answer.
    pub(super) fn refresh_reader_comparisons(
        &mut self,
        nodes: &[Arc<Node>],
        known: &StringComparisons,
        work: &mut work::Allowance,
    ) -> Result<(), QueryCommandError> {
        let mut current = StringComparisons::new();
        let ((), flags) = work.phase(self.limits, |eval| {
            let mut sources = HashMap::new();
            let active = nodes
                .iter()
                .filter(|n| !self.queries.get(&n.id).is_some_and(Query::suspended));
            for read in active.flat_map(|node| node.reads.iter()) {
                if !matches!(read.primitive.as_deref(), Some(Value::Str(_))) {
                    continue;
                }
                let entry = sources
                    .entry(read.id.as_str())
                    .or_insert_with(|| self.queries.get(&read.id).and_then(|q| self.data_entry(q)));
                if let Some(pair) = entry.and_then(|entry| read.string_pair(entry))
                    && let std::collections::hash_map::Entry::Vacant(slot) =
                        current.entry(pair.versions)
                {
                    if let Some(&equal) = known.get(&pair.versions) {
                        slot.insert(equal);
                    } else if eval.charge_work(pair.old.len()) {
                        slot.insert(pair.equal());
                    }
                }
            }
        });
        self.reader_comparisons = Arc::new(current);
        check_cuts(flags)
    }

    pub(super) fn register_readers(
        &mut self,
        input: &TreeInput<'_>,
        store: &Store,
    ) -> Result<Vec<ToolCall>, QueryCommandError> {
        // TreeScope indexes statement ids before it evaluates an expression.
        // Bound those copies even for caller-constructed, incomplete statements.
        let ids = input
            .result
            .query_statements
            .iter()
            .map(|q| &q.statement_id)
            .chain(
                input
                    .result
                    .mutation_statements
                    .iter()
                    .map(|m| &m.statement_id),
            );
        let limit = self.limits.work_for(usize::MAX);
        let mut bytes = 0usize;
        for id in ids {
            bytes = bytes.saturating_add(id.len()).saturating_add(1);
            if bytes > limit {
                return Err(QueryCommandError::Limit(crate::admission::size_error()));
            }
        }
        let measured = PreparedProgram::new(input.result, self.limits);
        check_cuts(measured.flags())?;
        let budget = EvaluationSession::with_limits(&(), measured.input_bytes(), self.limits);
        let nodes = self.evaluate_nodes(input, store, &budget, &measured)?;
        let mut requests = self.apply_nodes(input.result, &nodes);
        // A bounded parse can keep a waitFor edge but drop its source. No
        // completion could wake that reader. Refuse this registration atomically
        // with the parser's actual limit, keeping usable partial programs
        // valid, and name the first such reader and source (#119).
        if input.result.size_limited || input.result.depth_limited {
            let cut = nodes.iter().find_map(|node| {
                let read = node
                    .reads
                    .iter()
                    .find(|read| !self.queries.contains_key(&read.id))?;
                Some((node.id.clone(), read.id.clone()))
            });
            if let Some((reader, source)) = cut {
                // The ids reach the error's text; keep both within the
                // result-size limit, as any text the session reports is.
                let room = results::result_limit(self.limits) / 4;
                let (reader, source) = (bounded_id(reader, room), bounded_id(source, room));
                let mut flags = EvaluationFlags::default();
                flags.size_limited = input.result.size_limited;
                flags.depth_limited = input.result.depth_limited;
                return Err(QueryCommandError::SourceCut {
                    reader,
                    source,
                    limit: LimitError { flags },
                });
            }
        }
        self.readers = None;
        let mut work = work::Allowance::new(self.limits);
        self.refresh_reader_comparisons(&nodes, &self.reader_comparisons.clone(), &mut work)?;
        if nodes.iter().any(|n| !n.reads.is_empty()) {
            let nodes = self.propagate(
                input,
                store,
                &budget,
                &measured,
                nodes.into(),
                cascade::Trigger::Changed,
                &mut requests,
                &mut work,
            )?;
            let program = input.shared_result.cloned().unwrap_or_else(|| {
                Arc::new(ParseResult {
                    // Only stale readers need their expressions again. Preserve
                    // positions and completeness without retaining unrelated ASTs.
                    query_statements: input
                        .result
                        .query_statements
                        .iter()
                        .map(|s| {
                            let reader = (s.complete
                                && s.wait_for.as_ref().is_some_and(|reads| !reads.is_empty()))
                            .then_some(s);
                            QueryStatement {
                                statement_id: s.statement_id.clone(),
                                complete: s.complete,
                                tool: reader.and_then(|s| s.tool.clone()),
                                args: reader.and_then(|s| s.args.clone()),
                                defaults: reader.and_then(|s| s.defaults.clone()),
                                deps: reader.and_then(|s| s.deps.clone()),
                                wait_for: reader.and_then(|s| s.wait_for.clone()),
                                refresh: reader.and_then(|s| s.refresh.clone()),
                            }
                        })
                        .collect(),
                    ..ParseResult::default()
                })
            });
            self.readers = Some(ReaderPlan {
                program,
                measured,
                rounds_limited: self.reader_rounds_limited,
                retained_limit: self.reader_limit,
                nodes,
                store: store.query_state(),
                needs_context: input.cloud_scripts.is_some()
                    || input.library.is_some()
                    || input.actions.is_some()
                    || input.functions.is_some()
                    || input.binding_props.is_some(),
            });
        }
        Ok(requests.dispatch(self, None))
    }

    // Walk the reverse waiting edges from all outstanding calls at once. This
    // computes upstream's isPending for the entire snapshot without recursion.
    pub(super) fn pending_queries(&self) -> HashSet<&str> {
        let mut readers: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut pending = HashSet::new();
        let mut work = Vec::new();
        for (id, q) in &self.queries {
            if q.suspended() {
                continue;
            }
            if self
                .cache
                .get(&q.key)
                .is_some_and(|entry| entry.pending.is_some())
            {
                pending.insert(id.as_str());
                work.push(id.as_str());
            }
            if q.waiting() {
                for read in q.reads.iter() {
                    readers.entry(&read.id).or_default().push(id);
                }
            }
        }
        while let Some(source) = work.pop() {
            for &reader in readers.get(source).into_iter().flatten() {
                if pending.insert(reader) {
                    work.push(reader);
                }
            }
        }
        pending
    }
}

/// `id`, or its first bytes within `room` (at a character boundary) and an
/// ellipsis, for a message. A room too small for the ellipsis holds nothing
/// of a cut id: a bare prefix would read as a whole, different id.
fn bounded_id(mut id: String, room: usize) -> String {
    const ELLIPSIS: char = '…';
    if id.len() <= room {
        return id;
    }
    let Some(mut end) = room.checked_sub(ELLIPSIS.len_utf8()) else {
        id.clear();
        return id;
    };
    while !id.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    id.truncate(end);
    id.push(ELLIPSIS);
    id
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_id_is_cut_at_a_character_boundary_within_its_room() {
        assert_eq!(super::bounded_id("q0".into(), 8), "q0");
        let cut = super::bounded_id("ééééé".into(), 6);
        assert_eq!(cut, "é…");
        assert!(cut.len() <= 6);
        assert_eq!(super::bounded_id("abcdefghij".into(), 8), "abcde…");
    }

    #[test]
    fn a_cut_id_never_exceeds_a_room_too_small_for_the_ellipsis() {
        for room in 0..3 {
            let cut = super::bounded_id("abcdefghij".into(), room);
            assert!(cut.len() <= room, "room {room}: {cut:?}");
            assert_eq!(cut, "", "room {room}");
            assert_eq!(super::bounded_id("éé".into(), room), "", "room {room}");
        }
        assert_eq!(super::bounded_id("ab".into(), 2), "ab");
        assert_eq!(super::bounded_id("abcd".into(), 3), "…");
    }
}
