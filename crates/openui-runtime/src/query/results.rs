//! Tool results are supplied input (ADR 0003). Every path that puts a result
//! into the cache or a mutation's result goes through [`admit`], which decides
//! in one order: whether the result is JSON (the embedder's error, refusing
//! the command), then the depth limit, then the result-size limit, then the
//! cache cap. A result past a limit is delivered as the call's failure, a
//! `tool-error` naming the limit, as any other failure is (ADR 0013 rule 3).
//! Nothing already admitted is evicted or changed (#90). A value result's
//! validity and depth are read only within the depth limit, a shared body
//! once rather than at every position (again only where it was cut and is
//! met with more room), and its size stops at the first byte over the limit
//! (#141).
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
    work: &mut work::Allowance,
    fits: impl FnOnce(usize) -> bool,
) -> Result<Option<Admitted>, QueryCommandError> {
    let size = result_limit(limits);
    let depth = limits.depth();
    work.phase(limits, |eval| {
        let charge = |units| eval.charge_work(units);
        let measured = match outcome {
            ToolOutcome::Result(None) if !missing_is_null => return Ok(Some(Admitted::Missing)),
            ToolOutcome::Result(None) => value_measure(&Value::Null, size, depth, &charge)?,
            ToolOutcome::ResultValue(value) => value_measure(value, size, depth, &charge)?,
            ToolOutcome::Result(Some(json)) => match json_depth_fits(json, depth, &charge) {
                Some(true) => host_json_bytes(json, limits, size, &charge)
                    .map_err(|_| ResultLimit::Size(size)),
                Some(false) => Err(ResultLimit::Depth(depth)),
                None => Err(ResultLimit::Size(size)),
            },
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
    })
    .0
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
            .filter_map(|entry| entry.data.as_deref())
            .chain(self.mutations.data())
            .map(|data| value_bytes(data, usize::MAX, &|_| true).unwrap_or(usize::MAX))
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
        key: &CacheKey,
        outcome: &ToolOutcome<'_>,
        work: &mut work::Allowance,
    ) -> Result<Option<Admitted>, QueryCommandError> {
        admit(
            outcome,
            self.limits,
            true,
            self.cache_limit,
            work,
            |bytes| self.cache_after_success(key, bytes) <= self.cache_limit,
        )
    }

    // What the manager would hold once a success of `bytes` for `key` takes
    // effect: the entry's data replaced and each current reader's previous
    // entry released if no other query holds it (ADR 0013).
    fn cache_after_success(&self, key: &CacheKey, bytes: usize) -> usize {
        let mut drops: HashMap<&CacheKey, usize> = HashMap::new();
        for query in self
            .queries
            .values()
            .filter(|query| !query.suspended() && query.key == *key)
        {
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
    pub(super) fn withhold_errors(&mut self, key: &CacheKey) {
        for query in self
            .queries
            .values_mut()
            .filter(|q| !q.suspended() && q.key == *key)
        {
            query.withheld = true;
        }
    }

    /// Failed readers in registration order, including those represented by
    /// a compact diagnostic. Retry needs their ids even when it cannot list
    /// their individual error records in the snapshot.
    pub fn failed_query_ids(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|id| self.queries.get(*id).is_some_and(|q| q.error.is_some()))
            .cloned()
            .collect()
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
        let delivery = self.stage_tool_call(id, outcome)?;
        let refused = delivery.result_refusal();
        Ok((delivery.finish(None)?, refused))
    }
}

/// `"…"` for `text`, if it fits `room`. Stops before scanning a string whose
/// bytes alone exceed the room, as its escaped form is never shorter.
pub(super) fn string_bytes(
    text: &str,
    room: usize,
    charge: &impl Fn(usize) -> bool,
) -> Option<usize> {
    if text.len() > room.saturating_sub(2) || !charge(text.len()) {
        return None;
    }
    let bytes = openui_lang::json_string_len(text).saturating_add(2);
    (bytes <= room).then_some(bytes)
}

// Whether host JSON nests at most `depth` arrays and objects.
fn json_depth_fits(
    value: &JsonValue,
    depth: usize,
    charge: &impl Fn(usize) -> bool,
) -> Option<bool> {
    let mut path: Vec<Box<dyn Iterator<Item = &JsonValue> + '_>> =
        vec![Box::new(std::iter::once(value))];
    while let Some(level) = path.last_mut() {
        let Some(value) = level.next() else {
            path.pop();
            continue;
        };
        if !charge(1) {
            return None;
        }
        let children: Box<dyn Iterator<Item = &JsonValue>> = match value {
            JsonValue::Array(items) => Box::new(items.iter()),
            JsonValue::Object(fields) => Box::new(fields.values()),
            _ => continue,
        };
        if path.len() > depth {
            return Some(false);
        }
        path.push(children);
    }
    Some(true)
}

// A value: refused as not JSON if it holds an element or syntax anywhere
// within `depth` levels, whatever the order of its members; otherwise as too
// deep if any array or object nests past `depth` (what lies past it is not
// read); then its size, stopping at the first byte over.
fn value_measure(
    value: &Value,
    size: usize,
    depth: usize,
    charge: &impl Fn(usize) -> bool,
) -> Result<Result<usize, ResultLimit>, QueryCommandError> {
    // Until the validity walk finishes, unread members may contain syntax.
    // Refuse the command if its allowance cannot establish a JSON result.
    let shape = shape(value, depth, charge)
        .ok_or(QueryCommandError::Limit(crate::admission::size_error()))?;
    // An element or syntax value under `n` arrays and objects is read when
    // each of them is entered, that is when the innermost is within `depth`.
    if shape.syntax.is_some_and(|around| around <= depth) {
        return Err(QueryCommandError::Limit(crate::admission::size_error()));
    }
    if shape.height > depth {
        return Ok(Err(ResultLimit::Depth(depth)));
    }
    Ok(value_bytes(value, size, charge).ok_or(ResultLimit::Size(size)))
}

/// What deciding validity and depth needs of a value, counted in arrays and
/// objects, as far down as it was read.
#[derive(Clone, Copy)]
struct Shape {
    /// The most arrays and objects nested in it, itself included; an array
    /// or object that was not entered counts as one.
    height: usize,
    /// The fewest arrays and objects around an element or syntax value read
    /// in it, if any.
    syntax: Option<usize>,
}

const SCALAR: Shape = Shape {
    height: 0,
    syntax: None,
};

// An array or object being read, and what its members have shown so far.
struct Open<'a> {
    // Its body's address when another reference to the body exists, so the
    // body can be reached again and its shape is remembered.
    shared: Option<usize>,
    // How many levels, itself included, lie within the depth limit here.
    room: usize,
    members: Box<dyn Iterator<Item = &'a Value> + 'a>,
    shape: Shape,
}

// A shared body's shape, read with `room` levels.
#[derive(Clone, Copy)]
struct Known {
    room: usize,
    shape: Shape,
}

// A value's shape as the walk before #141 read it: an array or object is
// entered only within the depth limit, and one past it counts as one level
// unread. So the walk holds at most `depth` open levels, however tall the
// value is.
//
// Reading a body with `room` levels decides everything the limit can need of
// it at any place with that much room or less: its height up to `room` + 1
// and any element or syntax value within `room` levels. Past that nothing
// changes the answer: the height is already over the limit there, and a
// deeper element or syntax value is not read. So a body another reference can
// reach again is remembered by address with its room and not read again
// where it has no more room, and a body with one reference is reached only
// through its one parent. A body whose height is at most the room it was read
// with was not cut anywhere: every member was read, so its height and the
// distance to its nearest element or syntax value are exact, and it is not
// read again at any room. A cut body counts an unread array or object as one
// level at room 0, so its height is over its room, and it is read again only
// where it has more room than before, so at most once per level, at most
// `depth` times. The work is linear in the distinct bodies within the limit;
// the depth limit multiplies it only for bodies cut when met deep and met
// again shallower, and a cut body makes the whole value too deep, so such a
// value is refused anyway. Allocates one iterator per body it enters.
fn shape(value: &Value, depth: usize, charge: &impl Fn(usize) -> bool) -> Option<Shape> {
    let mut known: HashMap<usize, Known> = HashMap::new();
    let mut path: Vec<Open<'_>> = Vec::new();
    let mut next = Some(value);
    'walk: loop {
        let done = 'read: {
            let Some(value) = next.take() else {
                let Some(open) = path.last_mut() else {
                    return Some(SCALAR);
                };
                if let Some(member) = open.members.next() {
                    next = Some(member);
                    continue 'walk;
                }
                let Some(open) = path.pop() else {
                    return Some(SCALAR);
                };
                if let Some(address) = open.shared {
                    let (room, shape) = (open.room, open.shape);
                    known.insert(address, Known { room, shape });
                }
                break 'read open.shape;
            };
            if !charge(1) {
                return None;
            }
            let (address, count) = match value {
                Value::Element(_) | Value::Ast(_) => {
                    break 'read Shape {
                        height: 0,
                        syntax: Some(0),
                    };
                }
                Value::Arr(items) => (Arc::as_ptr(items).cast::<()>(), Arc::strong_count(items)),
                Value::Obj(fields) => (Arc::as_ptr(fields).cast::<()>(), Arc::strong_count(fields)),
                _ => break 'read SCALAR,
            };
            // Past the depth limit an array or object is too deep, unread.
            let room = depth.saturating_sub(path.len());
            if room == 0 {
                break 'read Shape {
                    height: 1,
                    syntax: None,
                };
            }
            let shared = (count > 1).then_some(address as usize);
            if let Some(known) = shared
                .and_then(|address| known.get(&address))
                .filter(|known| known.room >= room || known.shape.height <= known.room)
            {
                break 'read known.shape;
            }
            let members: Box<dyn Iterator<Item = &Value>> = match value {
                Value::Arr(items) => Box::new(items.iter()),
                Value::Obj(fields) => Box::new(fields.iter_unordered().map(|(_, item)| item)),
                _ => break 'read SCALAR,
            };
            path.push(Open {
                shared,
                room,
                members,
                shape: Shape {
                    height: 1,
                    syntax: None,
                },
            });
            continue 'walk;
        };
        let Some(parent) = path.last_mut() else {
            return Some(done);
        };
        parent.shape.height = parent.shape.height.max(done.height.saturating_add(1));
        let around = done.syntax.map(|around| around.saturating_add(1));
        parent.shape.syntax = match (parent.shape.syntax, around) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
}

// `JSON.stringify`'s length of a JSON value, or `None` at the first byte past
// `budget`. Shared bodies count at every position, as their JSON is written
// there. Allocates one iterator per open level, nothing per member.
fn value_bytes(value: &Value, budget: usize, charge: &impl Fn(usize) -> bool) -> Option<usize> {
    type Level<'a> = Box<dyn Iterator<Item = (Option<&'a str>, &'a Value)> + 'a>;
    let mut bytes = 0usize;
    let mut path: Vec<Level<'_>> = vec![Box::new(std::iter::once((None, value)))];
    while let Some(level) = path.last_mut() {
        let Some((name, value)) = level.next() else {
            path.pop();
            continue;
        };
        if !charge(1) {
            return None;
        }
        if let Some(name) = name {
            let written = string_bytes(name, budget.saturating_sub(bytes), charge)?;
            bytes = bytes.saturating_add(written).saturating_add(1);
        }
        let room = budget.saturating_sub(bytes);
        let (own, children): (usize, Option<Level<'_>>) = match value {
            Value::Null | Value::Bool(true) => (4, None),
            Value::Bool(false) => (5, None),
            Value::Num(number) => (openui_lang::json_number_len(*number), None),
            Value::Str(text) => (string_bytes(text, room, charge)?, None),
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

#[cfg(test)]
mod tests {
    use super::*;

    // The walk before #141, verbatim from c6431d45 but for the two names: the
    // oracle the bounded walk must agree with on every value, over the limits
    // too. It reads every position of a shared body, so it is fed only small
    // values.
    // A value: refused as not JSON if it holds an element or syntax anywhere
    // within `depth` levels, whatever the order of its members; otherwise as too
    // deep if any array or object nests past `depth` (what lies past it is not
    // read); then its size, stopping at the first byte over.
    fn old_value_measure(
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
        Ok(old_value_bytes(value, size).ok_or(ResultLimit::Size(size)))
    }

    // `JSON.stringify`'s length of a JSON value, or `None` at the first byte past
    // `budget`. Shared bodies count at every position, as their JSON is written
    // there. Allocates one iterator per open level, nothing per member.
    fn old_value_bytes(value: &Value, budget: usize) -> Option<usize> {
        type Level<'a> = Box<dyn Iterator<Item = (Option<&'a str>, &'a Value)> + 'a>;
        let mut bytes = 0usize;
        let mut path: Vec<Level<'_>> = vec![Box::new(std::iter::once((None, value)))];
        while let Some(level) = path.last_mut() {
            let Some((name, value)) = level.next() else {
                path.pop();
                continue;
            };
            if let Some(name) = name {
                let written = string_bytes(name, budget.saturating_sub(bytes), &|_| true)?;
                bytes = bytes.saturating_add(written).saturating_add(1);
            }
            let room = budget.saturating_sub(bytes);
            let (own, children): (usize, Option<Level<'_>>) = match value {
                Value::Null | Value::Bool(true) => (4, None),
                Value::Bool(false) => (5, None),
                Value::Num(number) => (openui_lang::json_number_len(*number), None),
                Value::Str(text) => (string_bytes(text, room, &|_| true)?, None),
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

    #[derive(Debug, PartialEq)]
    enum Outcome {
        NotJson,
        Refused(ResultLimit),
        Bytes(usize),
    }

    fn outcome(measured: Result<Result<usize, ResultLimit>, QueryCommandError>) -> Outcome {
        match measured {
            Err(_) => Outcome::NotJson,
            Ok(Err(limit)) => Outcome::Refused(limit),
            Ok(Ok(bytes)) => Outcome::Bytes(bytes),
        }
    }

    /// A small deterministic generator, so a failure replays.
    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % n
        }
    }

    fn element() -> Value {
        Value::Element(Arc::new(openui_lang::Element {
            type_name: "Card".into(),
            props: JsObject::new(),
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }))
    }

    fn leaves() -> Vec<Value> {
        vec![
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Num(f64::NAN),
            Value::Num(-0.0),
            Value::Num(1e21),
            Value::Str(String::new()),
            Value::Str("\u{1}\"x".into()),
            Value::Ast(Arc::new(Ast::Null)),
            element(),
        ]
    }

    // Positions, counted through every occurrence, or `None` past `cap`.
    fn positions(value: &Value, cap: usize) -> Option<usize> {
        let members: Vec<&Value> = match value {
            Value::Arr(items) => items.iter().collect(),
            Value::Obj(fields) => fields.iter_unordered().map(|(_, v)| v).collect(),
            _ => Vec::new(),
        };
        members.into_iter().try_fold(1usize, |sum, member| {
            let sum = sum.checked_add(positions(member, cap)?)?;
            (sum <= cap).then_some(sum)
        })
    }

    fn height(value: &Value) -> usize {
        match value {
            Value::Arr(items) => 1 + items.iter().map(height).max().unwrap_or(0),
            Value::Obj(fields) => {
                1 + fields
                    .iter_unordered()
                    .map(|(_, v)| height(v))
                    .max()
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }

    const KEYS: [&str; 6] = ["", "\u{0}", "a", "1", "\"\n", "a"];

    fn container(lcg: &mut Lcg, members: Vec<Value>) -> Value {
        if lcg.below(2) == 0 {
            return Value::Arr(Arc::new(members));
        }
        let mut fields = JsObject::new();
        for member in members {
            fields.define(KEYS[lcg.below(KEYS.len())], member);
        }
        Value::Obj(Arc::new(fields))
    }

    /// Compares the bounded walk with the oracle at depths and sizes around
    /// `value`'s own height, position count and byte count. Gives the cases.
    fn compare(value: &Value) -> usize {
        let count = positions(value, usize::MAX).unwrap_or(usize::MAX);
        let tall = height(value);
        let exact = old_value_bytes(value, usize::MAX).unwrap_or(0);
        let mut sizes: Vec<usize> = (0..=8).collect();
        for around in [count, exact] {
            sizes.extend([around.saturating_sub(1), around, around.saturating_add(1)]);
        }
        let mut depths = vec![
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            10,
            256,
            usize::MAX - 1,
            usize::MAX,
        ];
        depths.extend([tall.saturating_sub(1), tall, tall + 1]);
        let mut cases = 0;
        for &depth in &depths {
            for &size in &sizes {
                assert_eq!(
                    outcome(value_measure(value, size, depth, &|_| true)),
                    outcome(old_value_measure(value, size, depth)),
                    "depth {depth} size {size} height {tall} positions {count}"
                );
                cases += 1;
            }
        }
        cases
    }

    #[test]
    fn the_bounded_walk_decides_every_value_as_the_walk_before_it_did() {
        let mut lcg = Lcg(0x141);
        let mut cases = 0;
        for _ in 0..200 {
            let mut pool = leaves();
            for _ in 0..24 {
                let width = lcg.below(4);
                let members = (0..width)
                    .map(|_| {
                        // Mostly recent bodies, so sharing nests; sometimes a
                        // fresh body with one reference.
                        let pick = pool.len() - 1 - lcg.below(pool.len().min(6));
                        match pool.get(pick) {
                            Some(body) if lcg.below(5) != 0 => body.clone(),
                            _ => container(&mut lcg, vec![Value::Num(0.0)]),
                        }
                    })
                    .collect();
                let built = container(&mut lcg, members);
                if positions(&built, 2048).is_some() {
                    cases += compare(&built);
                    pool.push(built);
                }
            }
        }
        // Chains around the default depth limit of 256, sharing one small
        // body at every level, with syntax, an element or nothing at the end.
        let bottoms = [Value::Null, element(), Value::Ast(Arc::new(Ast::Null))];
        for (bottom, levels) in bottoms
            .iter()
            .flat_map(|bottom| [250, 254, 255, 256, 257, 258, 262].map(|n| (bottom.clone(), n)))
        {
            let side = container(
                &mut lcg,
                vec![Value::Str(String::new()), Value::Num(f64::NAN)],
            );
            let value = (0..levels).fold(bottom, |inner, _| {
                container(&mut lcg, vec![side.clone(), inner, side.clone()])
            });
            cases += compare(&value);
            // The chain's own body shared at its root, read twice there.
            cases += compare(&Value::Arr(Arc::new(vec![value.clone(), value])));
        }
        // One body placed at two levels of the same value, the deeper first
        // or the shallower first, with an element or syntax value at a
        // distance inside it, and either shared or copied: around depth
        // limits small enough that the body reaches past them at one level
        // and not at the other, and a body far taller than the limit.
        for inner in [0usize, 1, 2, 3, 4, 6, 40] {
            for bottom in [Value::Null, element(), Value::Ast(Arc::new(Ast::Null))] {
                let tail = (0..inner).fold(bottom.clone(), |value, _| {
                    container(&mut lcg, vec![value, Value::Num(1.0)])
                });
                let body = container(&mut lcg, vec![tail, Value::Str(String::new())]);
                let fresh = |lcg: &mut Lcg| {
                    let copy = (0..inner).fold(bottom.clone(), |value, _| {
                        container(lcg, vec![value, Value::Num(1.0)])
                    });
                    container(lcg, vec![copy, Value::Str(String::new())])
                };
                for (deep, shallow) in [(1usize, 0usize), (3, 1), (5, 2), (6, 0), (9, 4)] {
                    for deeper_first in [true, false] {
                        for copied in [false, true] {
                            let place = |lcg: &mut Lcg, levels: usize| {
                                let start = if copied { fresh(lcg) } else { body.clone() };
                                (0..levels).fold(start, |value, _| container(lcg, vec![value]))
                            };
                            let (a, b) = (place(&mut lcg, deep), place(&mut lcg, shallow));
                            let members = if deeper_first { vec![a, b] } else { vec![b, a] };
                            cases += compare(&container(&mut lcg, members));
                        }
                    }
                }
            }
        }
        // A ladder: one body under a chain of one-member containers, every
        // rung also held at the root, the tallest first, so each rung is met
        // deep first and again with more room. The bottom body is read whole
        // or cut, and holds an element, syntax or neither.
        for levels in [0usize, 1, 2, 3, 5, 8, 12, 253, 254, 255] {
            for bottom in [Value::Null, element(), Value::Ast(Arc::new(Ast::Null))] {
                let empty = container(&mut lcg, Vec::new());
                let mut rung = container(&mut lcg, vec![empty, bottom, Value::Num(2.0)]);
                let mut rungs = Vec::new();
                for _ in 0..levels {
                    rung = container(&mut lcg, vec![rung]);
                    rungs.push(rung.clone());
                }
                rungs.reverse();
                cases += compare(&Value::Arr(Arc::new(rungs)));
            }
        }
        assert!(cases > 50_000, "{cases} cases");
        eprintln!("the bounded walk agreed with the old walk on {cases} cases");
    }
}
