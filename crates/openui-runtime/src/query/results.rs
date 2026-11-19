//! Tool results are supplied input (ADR 0003). Every path that puts a result
//! into the cache or a mutation's result goes through [`admit`], which decides
//! in one order: whether the result is JSON (the embedder's error, refusing
//! the command), then the depth limit, then the result-size limit, then the
//! cache cap. A result past a limit is delivered as the call's failure, a
//! `tool-error` naming the limit, as any other failure is (ADR 0013 rule 3).
//! Nothing already admitted is evicted or changed (#90).
use super::*;

/// The limit a tool result was refused under. Its text is the message of the
/// `tool-error` delivered in the result's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResultLimit {
    /// The result's JSON is longer than this many bytes, the result-size limit.
    Size(usize),
    /// The result nests more arrays and objects than this, the depth limit.
    Depth(usize),
    /// With the result, the data the manager holds would exceed this many
    /// bytes, the cache cap.
    Cache(usize),
    /// The result fits, but its copies at every query that reads it do not
    /// fit the snapshot's room, this many bytes.
    Snapshot(usize),
}

impl fmt::Display for ResultLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size(bytes) => {
                write!(
                    f,
                    "tool result exceeds the result size limit of {bytes} bytes"
                )
            }
            Self::Depth(levels) => {
                write!(
                    f,
                    "tool result nests deeper than the depth limit of {levels} levels"
                )
            }
            Self::Cache(bytes) => {
                write!(
                    f,
                    "tool result does not fit the cache limit of {bytes} bytes"
                )
            }
            Self::Snapshot(bytes) => write!(
                f,
                "tool result does not fit the result size limit of {bytes} bytes at every query that reads it"
            ),
        }
    }
}

/// A result's admission.
pub(super) enum Admitted {
    /// The copy to store and its charge.
    Data(Value, usize),
    /// A missing mutation result: no data.
    Missing,
    /// Past this limit: deliver as the call's failure.
    Refused(ResultLimit),
}

/// The result-size limit for a tool result: the result-size ceiling, or the
/// work limit if smaller, as measuring a result is input work.
pub(super) fn result_limit(limits: Limits) -> usize {
    limits.result_size().min(limits.work_for(usize::MAX))
}

/// The one admission of a result. `None` for an outcome that is a failure.
/// A missing query result is `null`; a missing mutation result has no data.
/// `fits` answers for the cache cap, `cap`, given the result's bytes.
pub(super) fn admit(
    outcome: &ToolOutcome<'_>,
    limits: Limits,
    missing_is_null: bool,
    cap: usize,
    fits: impl FnOnce(usize) -> bool,
) -> Result<Option<Admitted>, QueryCommandError> {
    let size = result_limit(limits);
    let depth = limits.depth();
    let measured = match outcome {
        ToolOutcome::Result(None) if !missing_is_null => return Ok(Some(Admitted::Missing)),
        ToolOutcome::Result(None) => value_measure(&Value::Null, size, depth)?,
        ToolOutcome::Result(Some(json)) if json_depth_fits(json, depth) => {
            host_json_bytes(json, limits, size).map_err(|_| ResultLimit::Size(size))
        }
        ToolOutcome::Result(Some(_)) => Err(ResultLimit::Depth(depth)),
        ToolOutcome::ResultValue(value) => value_measure(value, size, depth)?,
        _ => return Ok(None),
    };
    Ok(Some(match measured {
        Err(limit) => Admitted::Refused(limit),
        Ok(bytes) if !fits(bytes) => Admitted::Refused(ResultLimit::Cache(cap)),
        Ok(bytes) => Admitted::Data(
            match outcome {
                ToolOutcome::Result(Some(json)) => Value::from_json(json),
                ToolOutcome::ResultValue(value) => (*value).clone(),
                _ => Value::Null,
            },
            bytes,
        ),
    }))
}

impl QueryManager {
    /// Set the cache cap: the most bytes of result data the manager holds at
    /// once, counted as JSON. It covers every live cache entry, including the
    /// entry a query shows while its new key loads, and every mutation's
    /// successful data. The default is the result-size limit. A result that
    /// would take the held data past the cap after its completion is refused;
    /// nothing held is evicted. Lowering the cap below what is held refuses
    /// later results until enough is released.
    pub fn with_cache_limit(mut self, bytes: usize) -> Self {
        self.cache_limit = bytes;
        self
    }

    /// The cache cap in bytes.
    pub fn cache_limit(&self) -> usize {
        self.cache_limit
    }

    /// The bytes of result data the manager holds and charges to its cache
    /// cap: each live cache entry's data and each mutation's successful data,
    /// as JSON. A mutation's idle `data: null` after a rename is upstream's
    /// reset marker, not a result, and is not charged.
    pub fn cache_bytes(&self) -> usize {
        self.cache_bytes.saturating_add(self.mutations.data_bytes())
    }

    /// Conformance-test hook measuring all held result data afresh, to compare
    /// with [`QueryManager::cache_bytes`]. Outside the supported interface.
    #[doc(hidden)]
    pub fn measured_cache_bytes(&self) -> usize {
        self.cache
            .values()
            .filter_map(|entry| entry.data.as_ref())
            .chain(self.mutations.data())
            .map(|data| value_bytes(data, usize::MAX).unwrap_or(usize::MAX))
            .fold(0usize, usize::saturating_add)
    }

    /// Remove the entries no query holds, releasing their charge.
    pub(super) fn release_unreferenced(&mut self) {
        let mut released = 0usize;
        self.cache.retain(|_, entry| {
            if entry.references == 0 {
                released = released.saturating_add(entry.bytes);
            }
            entry.references != 0
        });
        self.cache_bytes = self.cache_bytes.saturating_sub(released);
    }

    /// The cap, and the cache entries' data, for admitting a mutation's data.
    pub(super) fn mutation_room(&self) -> (usize, usize) {
        (self.cache_limit, self.cache_bytes)
    }

    /// Admit a success for the entry at `key`. `None` for a failure.
    pub(super) fn admit_query_result(
        &self,
        key: &Arc<str>,
        outcome: &ToolOutcome<'_>,
    ) -> Result<Option<Admitted>, QueryCommandError> {
        admit(outcome, self.limits, true, self.cache_limit, |bytes| {
            self.cache_after_success(key, bytes) <= self.cache_limit
        })
    }

    // What the manager would hold once a success of `bytes` for `key` takes
    // effect: the entry's data replaced and each current reader's previous
    // entry released if no other query holds it (ADR 0013).
    fn cache_after_success(&self, key: &Arc<str>, bytes: usize) -> usize {
        let mut drops: HashMap<&Arc<str>, usize> = HashMap::new();
        for query in self.queries.values().filter(|query| query.key == *key) {
            if let Some(previous) = query.previous.as_ref().filter(|p| *p != key) {
                let count = drops.entry(previous).or_default();
                *count = count.saturating_add(1);
            }
        }
        let released = drops
            .into_iter()
            .filter_map(|(previous, count)| {
                self.cache.get(previous).filter(|e| e.references <= count)
            })
            .fold(0usize, |sum, entry| sum.saturating_add(entry.bytes));
        let replaced = self.cache.get(key).map_or(0, |entry| entry.bytes);
        self.cache_bytes
            .saturating_sub(replaced)
            .saturating_sub(released)
            .saturating_add(bytes)
            .saturating_add(self.mutations.data_bytes())
    }

    /// Mark the queries on `key` as having an error the snapshot cannot list.
    /// They keep it, so their failed entry is not called again, and the
    /// snapshot shows one compact diagnostic for all such queries instead.
    pub(super) fn withhold_errors(&mut self, key: &Arc<str>) {
        for query in self.queries.values_mut().filter(|q| q.key == *key) {
            query.withheld = true;
        }
    }

    /// The compact diagnostic shown for queries whose errors are withheld.
    pub(super) fn withheld_diagnostic(&self) -> Option<Value> {
        let count = self.queries.values().filter(|q| q.withheld).count();
        if count == 0 {
            return None;
        }
        let size = result_limit(self.limits);
        let mut error = JsObject::new();
        error.define("source", Value::Str("query".into()));
        error.define("component", Value::Str("Query".into()));
        error.define("code", Value::Str("tool-error".into()));
        error.define(
            "message",
            Value::Str(format!(
                "{count} queries failed with errors that do not fit the result size limit of {size} bytes"
            )),
        );
        Some(Value::Obj(Arc::new(error)))
    }

    /// [`QueryManager::complete_tool_call`], also reporting a result refused
    /// under the limits and delivered as the call's `tool-error`. A session
    /// uses it to know whether a mutation succeeded.
    pub fn complete_tool_call_admitted(
        &mut self,
        id: &ToolCallId,
        outcome: ToolCallOutcome,
    ) -> Result<(QueryUpdate, Option<ResultLimit>), QueryCommandError> {
        self.complete_tool_call_reporting(id, outcome)
    }
}

/// `"…"` for `text`, if it fits `room`. Stops before scanning a string whose
/// bytes alone exceed the room, as its escaped form is never shorter.
pub(super) fn string_bytes(text: &str, room: usize) -> Option<usize> {
    if text.len() > room.saturating_sub(2) {
        return None;
    }
    let bytes = openui_lang::json_string_len(text).saturating_add(2);
    (bytes <= room).then_some(bytes)
}

// Whether host JSON nests at most `depth` arrays and objects.
fn json_depth_fits(value: &JsonValue, depth: usize) -> bool {
    let mut path: Vec<Box<dyn Iterator<Item = &JsonValue> + '_>> =
        vec![Box::new(std::iter::once(value))];
    while let Some(level) = path.last_mut() {
        let Some(value) = level.next() else {
            path.pop();
            continue;
        };
        let children: Box<dyn Iterator<Item = &JsonValue>> = match value {
            JsonValue::Array(items) => Box::new(items.iter()),
            JsonValue::Object(fields) => Box::new(fields.values()),
            _ => continue,
        };
        if path.len() > depth {
            return false;
        }
        path.push(children);
    }
    true
}

// A value: refused as not JSON if it holds an element or syntax anywhere
// within `depth` levels, whatever the order of its members; otherwise as too
// deep if any array or object nests past `depth` (what lies past it is not
// read); then its size, stopping at the first byte over.
fn value_measure(
    value: &Value,
    size: usize,
    depth: usize,
) -> Result<Result<usize, ResultLimit>, QueryCommandError> {
    let mut too_deep = false;
    let mut path: Vec<Box<dyn Iterator<Item = &Value> + '_>> =
        vec![Box::new(std::iter::once(value))];
    while let Some(level) = path.last_mut() {
        let Some(value) = level.next() else {
            path.pop();
            continue;
        };
        let children: Box<dyn Iterator<Item = &Value>> = match value {
            Value::Element(_) | Value::Ast(_) => {
                return Err(QueryCommandError::Limit(crate::admission::size_error()));
            }
            Value::Arr(items) => Box::new(items.iter()),
            Value::Obj(fields) => Box::new(fields.iter_unordered().map(|(_, item)| item)),
            _ => continue,
        };
        if path.len() > depth {
            too_deep = true;
            continue;
        }
        path.push(children);
    }
    if too_deep {
        return Ok(Err(ResultLimit::Depth(depth)));
    }
    Ok(value_bytes(value, size).ok_or(ResultLimit::Size(size)))
}

// `JSON.stringify`'s length of a JSON value, or `None` at the first byte past
// `budget`. Shared bodies count at every position, as their JSON is written
// there. Allocates one iterator per open level, nothing per member.
fn value_bytes(value: &Value, budget: usize) -> Option<usize> {
    type Level<'a> = Box<dyn Iterator<Item = (Option<&'a str>, &'a Value)> + 'a>;
    let mut bytes = 0usize;
    let mut path: Vec<Level<'_>> = vec![Box::new(std::iter::once((None, value)))];
    while let Some(level) = path.last_mut() {
        let Some((name, value)) = level.next() else {
            path.pop();
            continue;
        };
        if let Some(name) = name {
            let written = string_bytes(name, budget.saturating_sub(bytes))?;
            bytes = bytes.saturating_add(written).saturating_add(1);
        }
        let room = budget.saturating_sub(bytes);
        let (own, children): (usize, Option<Level<'_>>) = match value {
            Value::Null | Value::Bool(true) => (4, None),
            Value::Bool(false) => (5, None),
            Value::Num(number) => (openui_lang::json_number_len(*number), None),
            Value::Str(text) => (string_bytes(text, room)?, None),
            Value::Arr(items) => (
                items.len().saturating_add(1).max(2),
                Some(Box::new(items.iter().map(|item| (None, item)))),
            ),
            Value::Obj(fields) => (
                fields.len().saturating_add(1).max(2),
                Some(Box::new(
                    fields
                        .iter_unordered()
                        .map(|(name, item)| (Some(name.as_str()), item)),
                )),
            ),
            Value::Element(_) | Value::Ast(_) => return None,
        };
        bytes = bytes.saturating_add(own);
        if bytes > budget {
            return None;
        }
        if let Some(children) = children {
            path.push(children);
        }
    }
    Some(bytes)
}
