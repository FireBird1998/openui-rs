//! Tool results are supplied input (ADR 0003). Each one is measured under the
//! manager's result-size and depth limits, and the data the manager holds
//! under its cache cap, before it is copied. A refused result is delivered as
//! the call's failure, a `tool-error` naming the limit, as any other failure
//! is (ADR 0013 rule 3). Nothing already admitted is evicted or changed (#90).
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
        }
    }
}

/// A measured result: its copy and charge, or the limit it was refused under.
pub(super) enum Admission {
    Admitted(Value, usize),
    Refused(ResultLimit),
}

impl QueryManager {
    /// Set the cache cap: the most bytes of result data the manager holds at
    /// once, counted as JSON. It covers every live cache entry, including the
    /// entry a query shows while its new key loads, and every mutation's
    /// data. The default is the result-size limit. A result that would take
    /// the held data past the cap after its completion is refused; nothing
    /// held is evicted. Lowering the cap below what is held refuses later
    /// results until enough is released.
    pub fn with_cache_limit(mut self, bytes: usize) -> Self {
        self.cache_limit = bytes;
        self
    }

    /// The cache cap in bytes.
    pub fn cache_limit(&self) -> usize {
        self.cache_limit
    }

    /// The bytes of result data the manager holds and charges to its cache
    /// cap: each live cache entry's data and each mutation's data, as JSON.
    pub fn cache_bytes(&self) -> usize {
        self.cache_bytes.saturating_add(self.mutations.data_bytes())
    }

    /// Conformance-test hook measuring all held result data afresh, to compare
    /// with [`QueryManager::cache_bytes`]. Outside the supported interface.
    #[doc(hidden)]
    pub fn measured_cache_bytes(&self) -> usize {
        let unlimited = Limits::DEFAULT.with_depth(usize::MAX);
        self.cache
            .values()
            .filter_map(|entry| entry.data.as_ref())
            .chain(self.mutations.data())
            .map(|data| match value_bytes(data, unlimited, usize::MAX) {
                Ok(Ok(bytes)) => bytes,
                _ => usize::MAX,
            })
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

    /// Measure a success for the entry at `key` before copying it. `None` for
    /// a failure outcome. A value that is not JSON is the embedder's error
    /// and refuses the command.
    pub(super) fn admit_query_result(
        &self,
        key: &Arc<str>,
        outcome: &ToolOutcome<'_>,
    ) -> Result<Option<Admission>, QueryCommandError> {
        let measured = match outcome {
            ToolOutcome::Result(None) => Ok(NULL_BYTES),
            ToolOutcome::Result(Some(value)) => json_result_bytes(value, self.limits),
            ToolOutcome::ResultValue(value) => value_result_bytes(value, self.limits)?,
            _ => return Ok(None),
        };
        let bytes = match measured {
            Ok(bytes) => bytes,
            Err(limit) => return Ok(Some(Admission::Refused(limit))),
        };
        if self.cache_after_success(key, bytes) > self.cache_limit {
            return Ok(Some(Admission::Refused(ResultLimit::Cache(
                self.cache_limit,
            ))));
        }
        let value = match outcome {
            ToolOutcome::Result(Some(value)) => Value::from_json(value),
            ToolOutcome::ResultValue(value) => (*value).clone(),
            _ => Value::Null,
        };
        Ok(Some(Admission::Admitted(value, bytes)))
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

/// `null`, a missing query result.
pub(super) const NULL_BYTES: usize = 4;

/// The JSON length of host JSON as a result, refused past the result-size
/// limit or the depth limit. Stops at the first byte or level over.
pub(super) fn json_result_bytes(value: &JsonValue, limits: Limits) -> Result<usize, ResultLimit> {
    host_json_bytes(value, limits, limits.result_size()).map_err(|error| refusal(&error, limits))
}

/// The same measure for a result already in the evaluator's representation.
/// A value that is not JSON (an element or syntax) refuses the command.
pub(super) fn value_result_bytes(
    value: &Value,
    limits: Limits,
) -> Result<Result<usize, ResultLimit>, QueryCommandError> {
    value_bytes(value, limits, limits.result_size())
}

fn refusal(error: &QueryCommandError, limits: Limits) -> ResultLimit {
    match error {
        QueryCommandError::Limit(error) if error.flags.depth_limited => {
            ResultLimit::Depth(limits.depth())
        }
        _ => ResultLimit::Size(limits.result_size()),
    }
}

// `host_json_bytes` for a Value: `JSON.stringify`'s length, iterative, with
// at most `limits.depth()` nested arrays and objects. Shared bodies count at
// every position, as their JSON is written there.
fn value_bytes(
    value: &Value,
    limits: Limits,
    budget: usize,
) -> Result<Result<usize, ResultLimit>, QueryCommandError> {
    type Level<'a> = Box<dyn Iterator<Item = (Option<&'a str>, &'a Value)> + 'a>;
    let mut bytes = 0usize;
    let mut path: Vec<Level<'_>> = vec![Box::new(std::iter::once((None, value)))];
    while let Some(level) = path.last_mut() {
        let Some((name, value)) = level.next() else {
            path.pop();
            continue;
        };
        if let Some(name) = name {
            bytes = bytes.saturating_add(openui_lang::json_string_len(name).saturating_add(3));
        }
        let (own, children): (usize, Option<Level<'_>>) = match value {
            Value::Null | Value::Bool(true) => (4, None),
            Value::Bool(false) => (5, None),
            Value::Num(number) => (openui_lang::json_number_len(*number), None),
            Value::Str(text) => (openui_lang::json_string_len(text).saturating_add(2), None),
            Value::Arr(items) => (
                items.len().saturating_add(1).max(2),
                Some(Box::new(items.iter().map(|item| (None, item)))),
            ),
            Value::Obj(fields) => (
                fields.len().saturating_add(1).max(2),
                Some(Box::new(
                    fields
                        .iter()
                        .map(|(name, item)| (Some(name.as_str()), item)),
                )),
            ),
            Value::Element(_) | Value::Ast(_) => {
                return Err(QueryCommandError::Limit(crate::admission::size_error()));
            }
        };
        bytes = bytes.saturating_add(own);
        if bytes > budget {
            return Ok(Err(ResultLimit::Size(budget)));
        }
        if let Some(children) = children {
            if path.len() > limits.depth() {
                return Ok(Err(ResultLimit::Depth(limits.depth())));
            }
            path.push(children);
        }
    }
    Ok(Ok(bytes))
}
