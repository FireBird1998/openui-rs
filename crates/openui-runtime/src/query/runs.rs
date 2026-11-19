//! Running queries again: upstream's `invalidate`, with a remembered run that
//! belongs to the key (ADR 0013).
use super::*;

/// What a query last evaluated to call, kept so that a run can start a call
/// without evaluating the statement again, as upstream's `executeFetch` reads
/// the query's stored tool name and arguments.
#[derive(Clone, Default)]
pub(super) struct Request {
    pub(super) tool_name: String,
    pub(super) raw_tool_name: Option<Value>,
    pub(super) args: Option<Value>,
}

// A reader can visit several keys while one command is still evaluating.
// Preserve the first request and its order, but create receipts only once
// the current holders are known. A previous key can retain an older call.
#[derive(Default)]
pub(super) struct QueryRequests {
    keys: HashSet<Arc<str>>,
    ordered: Vec<(Arc<str>, Arc<Request>)>,
}

impl QueryRequests {
    pub(super) fn request(&mut self, key: Arc<str>, request: &Arc<Request>) {
        if self.keys.insert(key.clone()) {
            self.ordered.push((key, request.clone()));
        }
    }

    pub(super) fn dispatch(
        self,
        manager: &mut QueryManager,
        remembered: Option<(Arc<str>, Arc<Request>)>,
    ) -> Vec<ToolCall> {
        if self.ordered.is_empty() && remembered.is_none() {
            return Vec::new();
        }
        let current: HashSet<_> = manager
            .queries
            .values()
            .filter(|q| !q.suspended)
            .map(|q| q.key.as_ref())
            .collect();
        remembered
            .into_iter()
            .chain(self.ordered)
            .filter_map(|(key, request)| {
                if !current.contains(key.as_ref()) {
                    return None;
                }
                let entry = manager.cache.get_mut(&key)?;
                entry
                    .pending
                    .is_none()
                    .then(|| start_call(&manager.owner, &manager.lifecycle, entry, key, &request))
            })
            .collect()
    }
}

impl QueryManager {
    /// Whether a run of this query must do nothing because it waits for a
    /// query it reads. The only place the rule is decided.
    pub(super) fn waits(&self, statement_id: &str) -> bool {
        self.queries
            .get(statement_id)
            .is_some_and(|q| q.waiting() || q.suspended)
    }

    /// Upstream's `invalidate`. For each named query that is registered and
    /// does not wait, in the order given: start a call for its current key,
    /// or, while that key has a call in flight, remember one more run for the
    /// key. No names means every registered query, in registration order, as
    /// upstream treats an empty list. A repeated name counts again, so it can
    /// remember a run behind the call it just started.
    pub(super) fn invalidate(&mut self, statement_ids: &[&str]) -> Vec<ToolCall> {
        let targets: Vec<String> = if statement_ids.is_empty() {
            self.order.clone()
        } else {
            statement_ids
                .iter()
                .filter(|id| self.queries.contains_key(**id))
                .map(|id| (*id).to_owned())
                .collect()
        };
        let mut calls = Vec::new();
        for id in targets {
            if self.waits(&id) {
                continue;
            }
            let Some(query) = self.queries.get(&id) else {
                continue;
            };
            let Some(entry) = self.cache.get_mut(&query.key) else {
                continue;
            };
            if entry.pending.is_some() {
                // One remembered run per key, however many are asked for.
                // It keeps the first asker's request: two queries can share
                // a key while calling different tools (ADR 0013 keys collide
                // as upstream's do).
                entry.run_again.get_or_insert_with(|| query.request.clone());
            } else {
                calls.push(start_call(
                    &self.owner,
                    &self.lifecycle,
                    entry,
                    query.key.clone(),
                    &query.request,
                ));
            }
        }
        calls
    }

    /// After a call for `key` was delivered, take the run remembered for the
    /// key, if any, with the request that was asked for, whether or not the
    /// query that asked is still registered. With no query on the key the
    /// run is forgotten.
    pub(super) fn take_remembered(&mut self, key: Arc<str>) -> Option<(Arc<str>, Arc<Request>)> {
        let read = self
            .queries
            .values()
            .any(|query| !query.suspended && query.key == key);
        let entry = self.cache.get_mut(&key)?;
        let asked = entry.run_again.take()?;
        if !read || entry.pending.is_some() {
            return None;
        }
        Some((key, asked))
    }
}

/// Start a call for an entry with none in flight. The call belongs to the
/// entry, not to the query whose request it uses.
pub(super) fn start_call(
    owner: &Arc<()>,
    lifecycle: &Arc<()>,
    entry: &mut CacheEntry,
    key: Arc<str>,
    request: &Request,
) -> ToolCall {
    let pending = Arc::new(PendingCall {
        key,
        tool_name: request.tool_name.clone(),
        raw_tool_name: request.raw_tool_name.clone(),
    });
    let id = ToolCallId(Arc::new(CallIdentity {
        manager: Arc::downgrade(owner),
        lifecycle: Arc::downgrade(lifecycle),
        target: CallTarget::Query(Arc::downgrade(&pending)),
        consumed: AtomicBool::new(false),
    }));
    entry.pending = Some(pending);
    ToolCall {
        id,
        tool_name: request.tool_name.clone(),
        args: request.args.clone(),
    }
}
