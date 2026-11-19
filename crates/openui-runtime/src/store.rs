//! The pinned store's state transitions, without embedder callbacks.

pub(crate) mod query_state;
use query_state::QueryWatch;

use crate::{Ast, JsObject, Limits, Value};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

/// An owned value; `None` is top-level JavaScript undefined.
/// Cloning shares container bodies and copies scalar strings, as `Value` does.
pub type StateValue = Option<Value>;

/// An immutable snapshot, including names with an undefined value.
/// Writes copy it only when a reader still holds an older snapshot.
pub type StoreSnapshot = Arc<JsObject<StateValue>>;

/// A refused state write. That name's value and pristine status stay unchanged.
#[derive(Debug)]
#[non_exhaustive]
pub enum StoreError {
    /// A state write cannot advance the exhausted version.
    VersionExhausted,
    /// Measuring a write exhausted the configured work allowance.
    WorkLimited,
    /// The value exceeds the configured nesting depth.
    DepthLimited,
    /// Value measurement exhausted result room or its work allowance.
    ValueTooLarge,
    /// The combined state, including names and pristine-only names, is full.
    StateTooLarge,
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionExhausted => f.write_str("store version exhausted"),
            Self::WorkLimited => f.write_str("state measurement work exhausted"),
            Self::DepthLimited => f.write_str("state value depth exceeded"),
            Self::ValueTooLarge => f.write_str("state value size or measurement work exceeded"),
            Self::StateTooLarge => f.write_str("total state size exceeded"),
        }
    }
}
impl std::error::Error for StoreError {}

/// One refused initialization entry. Other names in the batch can still apply.
#[derive(Debug)]
#[non_exhaustive]
pub struct StateRefusal {
    /// The name whose write was refused.
    pub name: String,
    /// Why it was refused.
    pub error: StoreError,
}

/// Initialization's accepted changes and per-name refusals.
/// A refused entry is skipped and the batch continues. A successful entry is
/// not rolled back by a later refusal.
#[derive(Debug, Default)]
#[non_exhaustive]
#[must_use]
pub struct InitializationResult {
    /// At least one name's value or presence changed; version advanced once.
    pub changed: bool,
    /// Entries that could not be applied, in encounter order.
    pub refused: Vec<StateRefusal>,
}

/// State by name with upstream's pristine/default rules, without callbacks.
///
/// Version maps upstream listener notifications: one per changed set or
/// initialize batch. This runtime deliberately also advances it on disposal of
/// nonempty state; upstream clears its listeners without notifying them.
/// Pristine-only changes and empty disposal do not advance it. Completed
/// container defaults reused by Rust's streaming parser suppress notifications
/// that upstream's rebuilt defaults cause; CONFORMANCE.md pins that divergence.
///
/// Writes consume values. Reads and retained-snapshot copies clone `Value`:
/// container bodies stay shared; scalar strings and copied keys allocate.
/// Unchanged initialization and ignored scalar sets do not copy the snapshot
/// or pristine set. Use borrowed iterators to avoid constructing input maps.
///
/// Admission uses the configured result-size ceiling, independent of program
/// size. Total state is the exact JSON object size of all retained names and
/// values, projecting undefined (including pristine-only names) to null. Empty
/// state costs zero. Shared values count separately at each name. Each write
/// measures only its new value and name; retained per-name sizes make the total
/// update constant-time. Refusals preserve values, pristine flags and version.
pub struct Store {
    pub(crate) action_session: Arc<()>,
    pub(crate) actions: crate::tree::actions::Provenance,
    pub(crate) undefined_fields: Arc<crate::tree::actions::Provenance>,
    query_watch: QueryWatch,
    snapshot: StoreSnapshot,
    pristine: HashSet<String>,
    version: u64,
    limits: Limits,
    // Per-entry size includes a trailing comma. A nonempty object adds one
    // byte: two braces minus the final comma. This also bounds pristine names.
    sizes: HashMap<String, usize>,
    entry_bytes: usize,
}
impl Default for Store {
    fn default() -> Self {
        Self::with_limits(Limits::DEFAULT)
    }
}
impl Store {
    /// An empty store with default limits.
    pub fn new() -> Self {
        Self::default()
    }
    /// An empty store using these limits for every state admission route.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            action_session: Arc::new(()),
            actions: Default::default(),
            undefined_fields: Default::default(),
            query_watch: QueryWatch::default(),
            snapshot: Arc::new(JsObject::new()),
            pristine: HashSet::new(),
            version: 0,
            limits,
            sizes: HashMap::new(),
            entry_bytes: 0,
        }
    }
    /// Stage initialization and private action marks, committing only on success.
    /// Returns projection initialization, persisted initialization, and output.
    /// Both batches apply declarations, matching the separate initialize calls.
    /// The reader receives only a shared borrow: it cannot move out the staged
    /// Store and create an independent store with the same session authority.
    /// Existing custom identities/continuations survive commit. A refused read
    /// cannot mint authority in the live store. Host effects are not rolled back.
    ///
    /// ```compile_fail
    /// use openui_runtime::{JsObject, Store, Value};
    /// let mut store = Store::new();
    /// let empty = JsObject::<Value>::new();
    /// let _: Result<_, ()> = store.initialize_transaction(&empty, &empty, None,
    ///     |candidate| Ok(std::mem::take(candidate)));
    /// ```
    pub fn initialize_transaction<T, E>(
        &mut self,
        defaults: &JsObject<Value>,
        persisted: &JsObject<Value>,
        projected: Option<&JsObject<Value>>,
        read: impl FnOnce(&Store) -> Result<T, E>,
    ) -> Result<(InitializationResult, InitializationResult, T), E> {
        let mut candidate = Self {
            action_session: self.action_session.clone(),
            actions: self.actions.clone(),
            undefined_fields: Arc::new((*self.undefined_fields).clone()),
            query_watch: self.query_watch.staged(),
            snapshot: self.snapshot.clone(),
            pristine: self.pristine.clone(),
            version: self.version,
            limits: self.limits,
            sizes: self.sizes.clone(),
            entry_bytes: self.entry_bytes,
        };
        let projection = projected
            .map(|values| {
                candidate.initialize(
                    defaults.iter().map(|(k, v)| (k.as_str(), Some(v))),
                    values.iter().map(|(k, v)| (k.as_str(), Some(v))),
                )
            })
            .unwrap_or_default();
        let initialization = candidate.initialize(
            defaults.iter().map(|(k, v)| (k.as_str(), Some(v))),
            persisted.iter().map(|(k, v)| (k.as_str(), Some(v))),
        );
        let output = read(&candidate)?;
        candidate.query_watch.commit();
        candidate.query_watch.publish(
            &candidate.snapshot,
            candidate.entry_bytes,
            &candidate.undefined_fields,
        );
        *self = candidate;
        Ok((projection, initialization, output))
    }
    /// Read a value. Absent and explicitly undefined both return `None`;
    /// snapshot membership distinguishes their presence.
    pub fn get(&self, name: &str) -> StateValue {
        if !self.possible_name(name) {
            return None;
        }
        #[cfg(test)]
        lookup_work::record(name);
        self.snapshot.get(name).cloned().flatten()
    }
    /// Whether a later declared default may replace this name, including absent
    /// names marked pristine by an ignored undefined set.
    pub fn is_pristine(&self, name: &str) -> bool {
        if !self.possible_name(name) {
            return false;
        }
        #[cfg(test)]
        lookup_work::record(name);
        self.pristine.contains(name)
    }
    // Every retained key/field spends at least its raw UTF-8 length. Reject an
    // impossible lookup by length before hashing any caller-controlled text.
    pub(crate) fn possible_name(&self, name: &str) -> bool {
        name.len() <= self.entry_bytes
    }
    /// Read the current immutable snapshot. Reads and ignored sets retain identity.
    pub fn snapshot(&self) -> StoreSnapshot {
        self.snapshot.clone()
    }
    /// Serialize the already-admitted snapshot without a second allowance.
    /// Empty state still exports as `{}` even when its admission cap is zero.
    /// Undefined entries are omitted, matching JSON.stringify and the bindings;
    /// pristine-only names are not snapshot members. Re-initializing this JSON
    /// restores declared defaults for omitted undefined entries; older Rust
    /// exports wrote null instead. State was already admitted, so the shallow
    /// view is bounded.
    pub fn snapshot_json(&self) -> Result<String, crate::LimitError> {
        crate::value_to_json(&self.snapshot_value())
    }
    /// Serialize admitted state under explicit work, depth and output ceilings.
    pub fn snapshot_json_with_limits(&self, limits: Limits) -> Result<String, crate::LimitError> {
        crate::value_to_json_with_limits(&self.snapshot_value(), limits)
    }
    fn snapshot_value(&self) -> Value {
        let mut values = JsObject::new();
        for (key, value) in self.snapshot.iter() {
            if let Some(value) = value {
                values.define(key, value.clone());
            }
        }
        Value::Obj(Arc::new(values))
    }
    /// Notification version, including nonempty disposals (see `Store`).
    pub fn version(&self) -> u64 {
        self.version
    }
    /// Write a value and clear pristine even when the value change is ignored.
    pub fn set(&mut self, name: &str, value: StateValue) -> Result<bool, StoreError> {
        self.write(name, value, false)
    }
    /// Restore a default and mark pristine even when the value change is ignored.
    pub fn set_pristine(&mut self, name: &str, value: StateValue) -> Result<bool, StoreError> {
        self.write(name, value, true)
    }
    fn would_change(&self, name: &str, value: Option<&Value>) -> bool {
        let existing = self.snapshot.get(name).and_then(Option::as_ref);
        let undefined_shape_changed = existing.is_some_and(|v| self.undefined_fields.owns(v))
            != value.is_some_and(|v| self.undefined_fields.owns(v));
        undefined_shape_changed || (!same(existing, value) && !shallow_equal(existing, value))
    }
    // A reset is one atomic step. Preflight all possible Store refusals;
    // duplicate targets have the same declared default and change at most once.
    pub(crate) fn reset_batch(
        &mut self,
        writes: Vec<(String, StateValue)>,
    ) -> Result<(), StoreError> {
        let mut seen = HashSet::new();
        let mut changes = 0u64;
        let mut total = self.entry_bytes;
        let mut admitted = Vec::new();
        for (name, value) in writes {
            if seen.insert(name.clone()) {
                let changed = self.would_change(&name, value.as_ref());
                if changed {
                    changes = changes.checked_add(1).ok_or(StoreError::VersionExhausted)?;
                }
                let bytes = self.measure_entry(&name, value.as_ref())?;
                total = total.saturating_sub(self.sizes.get(&name).copied().unwrap_or(0));
                admitted.push((name, value, bytes, changed));
            }
        }
        // Remove all old entries before adding candidates. A fitting final
        // reset must not overflow merely because an early target grows before
        // a later target shrinks.
        for (_, _, bytes, _) in &admitted {
            total = total.checked_add(*bytes).ok_or(StoreError::StateTooLarge)?;
        }
        self.check_total(total)?;
        self.version
            .checked_add(changes)
            .ok_or(StoreError::VersionExhausted)?;
        // Apply after the combined check: intermediate reset order must not
        // reject a batch whose final state fits.
        // Use only core-measured preflight receipts. Re-measuring here could
        // change work pricing as earlier replacements release shared owners,
        // and a second refusal must never leave a reset prefix committed.
        for (name, value, bytes, changed) in admitted {
            self.record_size(&name, bytes);
            self.mark_pristine(&name);
            if changed {
                self.put(&name, value);
                self.version = self.version.saturating_add(1);
            }
        }
        // Intermediate replacement order may saturate the running counter;
        // the independently checked final sum is exact.
        self.entry_bytes = total;
        Ok(())
    }
    fn mark_pristine(&mut self, name: &str) {
        if !self.pristine.contains(name) {
            self.pristine.insert(name.to_owned());
        }
    }
    fn put(&mut self, name: &str, value: StateValue) {
        let observed = self.query_watch.writing();
        let watch = observed.as_ref().map(|state| {
            let mut view = state.lock().unwrap_or_else(|e| e.into_inner());
            view.begin_write();
            view
        });
        let state = Arc::make_mut(&mut self.snapshot);
        if let Some(existing) = state.get_mut(name) {
            *existing = value;
        } else {
            state.define(name, value);
        }
        if let Some(mut view) = watch {
            view.publish(&self.snapshot, self.entry_bytes, &self.undefined_fields);
        }
    }
    fn write(&mut self, name: &str, value: StateValue, pristine: bool) -> Result<bool, StoreError> {
        let changed = self.would_change(name, value.as_ref());
        let retained = changed || self.snapshot.has_own(name) || pristine;
        let bytes = if !retained {
            0
        } else if !changed {
            match self.sizes.get(name) {
                Some(bytes) => *bytes,
                None => self.measure_entry(name, value.as_ref())?,
            }
        } else {
            self.measure_entry(name, value.as_ref())?
        };
        self.check_total(self.replacement_total(name, bytes)?)?;
        let version = if changed {
            self.next_version()?
        } else {
            self.version
        };
        self.record_size(name, bytes);
        if pristine {
            self.mark_pristine(name);
        } else {
            self.pristine.remove(name);
        }
        if changed {
            self.put(name, value);
            self.version = version;
        }
        Ok(changed)
    }
    fn measure_entry(&self, name: &str, value: Option<&Value>) -> Result<usize, StoreError> {
        let work = self.limits.work_for(usize::MAX);
        if name.len() > work {
            return Err(StoreError::WorkLimited);
        }
        // The name scan and value walk share one admission-work budget.
        let value_work = work.saturating_sub(name.len());
        let key = openui_lang::json_string_len(name).saturating_add(2);
        // The core's scalar JSON lengths need no recursive walk or session.
        // Keep scalar replacement allocation-free, as it was before admission.
        let scalar = match value {
            None | Some(Value::Null) => Some(4usize),
            Some(Value::Bool(v)) => Some(if *v { 4 } else { 5 }),
            Some(Value::Num(v)) => Some(openui_lang::json_number_len(*v)),
            _ => None,
        };
        if let Some(bytes) = scalar {
            if self.limits.depth() == 0 {
                return Err(StoreError::DepthLimited);
            }
            if bytes.saturating_add(1) > value_work {
                return Err(StoreError::WorkLimited);
            }
            if bytes > self.limits.result_size() {
                return Err(StoreError::ValueTooLarge);
            }
            return key
                .checked_add(bytes)
                .and_then(|n| n.checked_add(2))
                .ok_or(StoreError::StateTooLarge);
        }
        let limits = self
            .limits
            .with_result_size(self.limits.result_size())
            .with_work(value_work);
        let session = crate::EvaluationSession::with_limits(&(), 0, limits);
        let bytes =
            session.with_evaluator(|eval| eval.reserve_value_size(value.unwrap_or(&Value::Null)));
        match bytes {
            Some(bytes) => key
                .checked_add(bytes)
                .and_then(|n| n.checked_add(2))
                .ok_or(StoreError::StateTooLarge),
            None if session.flags().depth_limited => Err(StoreError::DepthLimited),
            None => {
                // Both work and room cuts are typed refusals. Distinguishing
                // them is unnecessary for atomic admission.
                Err(StoreError::ValueTooLarge)
            }
        }
    }
    fn replacement_total(&self, name: &str, bytes: usize) -> Result<usize, StoreError> {
        self.entry_bytes
            .saturating_sub(self.sizes.get(name).copied().unwrap_or(0))
            .checked_add(bytes)
            .ok_or(StoreError::StateTooLarge)
    }
    fn check_total(&self, entries: usize) -> Result<(), StoreError> {
        let bytes = if entries == 0 {
            0
        } else {
            entries.checked_add(1).ok_or(StoreError::StateTooLarge)?
        };
        if bytes > self.limits.result_size() {
            Err(StoreError::StateTooLarge)
        } else {
            Ok(())
        }
    }
    fn record_size(&mut self, name: &str, bytes: usize) {
        let previous = if bytes == 0 {
            self.sizes.remove(name)
        } else if let Some(entry) = self.sizes.get_mut(name) {
            Some(std::mem::replace(entry, bytes))
        } else {
            self.sizes.insert(name.to_owned(), bytes);
            None
        };
        self.entry_bytes = self
            .entry_bytes
            .saturating_sub(previous.unwrap_or(0))
            .saturating_add(bytes);
    }
    /// Apply persisted entries first, then declared defaults, retaining names.
    ///
    /// Inputs are borrowed `(name, value)` entries in JavaScript property order;
    /// parsed declarations use `.iter().map(|(k,v)| (k.as_str(),Some(v)))`;
    /// optional persisted values use `v.as_ref()`. No staging map is required.
    /// Core `JsObject::iter` builds an ordering vector; slice and ordinary map
    /// iterators avoid even that input-adapter allocation. Within each input,
    /// the last value for a duplicate name wins, as in a JavaScript object.
    /// Default normalization is deferred until a candidate value change,
    /// preserving the allocation-free path for unchanged defaults. Persisted
    /// entries normalize first so duplicate refusals preserve pristine flags.
    ///
    /// Each refusal leaves that entry untouched and is reported by name; the
    /// rest of the batch continues. Persisted snapshots load what fits in input
    /// order, reporting each refusal, just like streaming defaults.
    pub fn initialize<'a>(
        &mut self,
        defaults: impl IntoIterator<Item = (&'a str, Option<&'a Value>)>,
        persisted: impl IntoIterator<Item = (&'a str, Option<&'a Value>)>,
    ) -> InitializationResult {
        let mut result = InitializationResult::default();
        self.initialize_entries(persisted, true, &mut result);
        self.initialize_entries(defaults, false, &mut result);
        result
    }
    fn initial_change(&self, name: &str, value: Option<&Value>, persisted: bool) -> bool {
        !self.snapshot.has_own(name)
            || ((persisted || self.is_pristine(name))
                && !same(self.snapshot.get(name).and_then(Option::as_ref), value))
    }
    fn initialize_entries<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (&'a str, Option<&'a Value>)>,
        persisted: bool,
        result: &mut InitializationResult,
    ) {
        let mut entries = entries.into_iter();
        // Persisted no-ops can clear pristine. Normalize their duplicates
        // before applying even those flags, so a refused last occurrence
        // cannot inherit an earlier duplicate's flag change.
        if persisted {
            let mut last = JsObject::new();
            for (name, value) in entries {
                last.define(name, value);
            }
            for (name, value) in last.iter() {
                self.initialize_entry(name, *value, true, result);
            }
            return;
        }
        while let Some((name, value)) = entries.next() {
            // A prefix of value no-ops can only change persisted pristine flags.
            // Once a value might change, normalize the remaining entries before
            // applying any of them. New names cannot have occurred in that prefix,
            // so this also preserves their first-occurrence insertion order.
            // At exhaustion normalize first even for no-ops: a later duplicate
            // refusal must not inherit an earlier entry's pristine update.
            if self.initial_change(name, value, persisted) || self.version == u64::MAX {
                let mut last = JsObject::new();
                last.define(name, value);
                for (name, value) in entries {
                    last.define(name, value);
                }
                for (name, value) in last.iter() {
                    self.initialize_entry(name, *value, persisted, result);
                }
                return;
            }
            self.initialize_entry(name, value, persisted, result);
        }
    }
    fn initialize_entry(
        &mut self,
        name: &str,
        value: Option<&Value>,
        persisted: bool,
        result: &mut InitializationResult,
    ) {
        let absent = !self.snapshot.has_own(name);
        let changed = self.initial_change(name, value, persisted);
        let bytes = if changed {
            let admission = self.measure_entry(name, value).and_then(|bytes| {
                self.check_total(self.replacement_total(name, bytes)?)?;
                Ok(bytes)
            });
            match admission {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    result.refused.push(StateRefusal {
                        name: name.to_owned(),
                        error,
                    });
                    return;
                }
            }
        } else {
            None
        };
        if changed && !self.admit_initial_change(name, result) {
            return;
        }
        if changed {
            if let Some(bytes) = bytes {
                self.record_size(name, bytes);
            }
            self.put(name, value.cloned());
        }
        if persisted {
            self.pristine.remove(name);
        } else if absent {
            self.mark_pristine(name);
        }
    }
    fn admit_initial_change(&mut self, name: &str, result: &mut InitializationResult) -> bool {
        if result.changed {
            return true;
        }
        match self.next_version() {
            Ok(version) => {
                self.version = version;
                result.changed = true;
                true
            }
            Err(error) => {
                result.refused.push(StateRefusal {
                    name: name.to_owned(),
                    error,
                });
                false
            }
        }
    }
    /// Clear values, pristine status and custom-call authority; the store can
    /// be reused, but retained plans lose their old custom marks.
    /// Upstream clears listeners without notifying. This runtime extends that
    /// behavior by advancing version for nonempty state. Empty disposal replaces
    /// the snapshot too, but does not advance version. Disposal always clears;
    /// at version exhaustion it leaves the version at `u64::MAX`.
    pub fn dispose(&mut self) -> bool {
        self.actions.clear();
        self.action_session = Arc::new(());
        let changed = !self.snapshot.is_empty();
        if changed {
            self.version = self.version.saturating_add(1);
        }
        self.snapshot = Arc::new(JsObject::new());
        self.query_watch
            .publish(&self.snapshot, 0, &self.undefined_fields);
        self.pristine.clear();
        self.sizes.clear();
        self.entry_bytes = 0;
        changed
    }
    fn next_version(&self) -> Result<u64, StoreError> {
        self.version
            .checked_add(1)
            .ok_or(StoreError::VersionExhausted)
    }
}

// Object.is, not Rust's structural Value equality or JavaScript's ===.
pub(crate) fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) | (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(a)), Some(Value::Bool(b))) => a == b,
        (Some(Value::Num(a)), Some(Value::Num(b))) => {
            (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => a == b,
        (Some(Value::Arr(a)), Some(Value::Arr(b))) => Arc::ptr_eq(a, b),
        (Some(Value::Obj(a)), Some(Value::Obj(b))) => Arc::ptr_eq(a, b),
        (Some(Value::Element(a)), Some(Value::Element(b))) => Arc::ptr_eq(a, b),
        (Some(Value::Ast(a)), Some(Value::Ast(b))) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

fn shallow_equal(a: Option<&Value>, b: Option<&Value>) -> bool {
    let (Some(a), Some(b)) = (a.and_then(Object::new), b.and_then(Object::new)) else {
        return false;
    };
    a.len() == b.len() && b.entries().all(|(key, value)| a.get(key).same(value))
}

// Typed core values stand for ordinary JS objects too. Borrow their own fields
// rather than serializing or copying a tree merely to compare its first level.
enum Object<'a> {
    Plain(&'a JsObject<Value>),
    Typed(Vec<(&'static str, Property<'a>)>),
}

#[derive(Clone, Copy)]
enum Property<'a> {
    Data(&'a Value),
    Text(&'a str),
    Bool(bool),
    Num(f64),
    Identity(u8, usize),
    Undefined,
    Inherited,
}

impl<'a> Property<'a> {
    fn scalar(self) -> Self {
        match self {
            Self::Data(Value::Str(v)) => Self::Text(v),
            Self::Data(Value::Bool(v)) => Self::Bool(*v),
            Self::Data(Value::Num(v)) => Self::Num(*v),
            Self::Data(Value::Ast(v)) => Self::Identity(0, Arc::as_ptr(v) as usize),
            _ => self,
        }
    }
    fn same(self, other: Self) -> bool {
        match (self.scalar(), other.scalar()) {
            (Self::Data(a), Self::Data(b)) => same(Some(a), Some(b)),
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Num(a), Self::Num(b)) => {
                (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
            }
            (Self::Identity(ta, a), Self::Identity(tb, b)) => ta == tb && a == b,
            _ => false,
        }
    }
}

impl<'a> Object<'a> {
    fn new(value: &'a Value) -> Option<Self> {
        use Property::{Bool, Identity, Num, Text};
        let mut fields = Vec::new();
        match value {
            Value::Obj(o) => return Some(Self::Plain(o)),
            Value::Element(e) => {
                fields.extend([
                    ("type", Text("element")),
                    ("typeName", Text(&e.type_name)),
                    ("props", Identity(4, Arc::as_ptr(e) as usize)),
                    ("partial", Bool(e.partial)),
                    ("hasDynamicProps", Bool(e.has_dynamic_props)),
                ]);
                if let Some(id) = &e.statement_id {
                    fields.push(("statementId", Text(id)));
                }
            }
            Value::Ast(node) => {
                let ast = |n: &'a Arc<Ast>| Identity(0, Arc::as_ptr(n) as usize);
                let kind = match node.as_ref() {
                    Ast::Comp {
                        name,
                        args,
                        function,
                        mapped_props,
                    } => {
                        fields.extend([
                            ("name", Text(name)),
                            ("args", Identity(1, Arc::as_ptr(args) as usize)),
                        ]);
                        if *function {
                            fields.push(("fn", Bool(true)));
                        }
                        if let Some(props) = mapped_props {
                            fields.push(("mappedProps", Identity(2, Arc::as_ptr(props) as usize)));
                        }
                        "Comp"
                    }
                    Ast::Str(v) => {
                        fields.push(("v", Text(v)));
                        "Str"
                    }
                    Ast::Num(v) => {
                        fields.push(("v", Num(*v)));
                        "Num"
                    }
                    Ast::Bool(v) => {
                        fields.push(("v", Bool(*v)));
                        "Bool"
                    }
                    Ast::Null => "Null",
                    Ast::Arr(v) => {
                        fields.push(("els", Identity(1, Arc::as_ptr(v) as usize)));
                        "Arr"
                    }
                    Ast::Obj(v) => {
                        fields.push(("entries", Identity(3, Arc::as_ptr(v) as usize)));
                        "Obj"
                    }
                    Ast::Ref(v) => {
                        fields.push(("n", Text(v)));
                        "Ref"
                    }
                    Ast::Ph(v) => {
                        fields.push(("n", Text(v)));
                        "Ph"
                    }
                    Ast::StateRef(v) => {
                        fields.push(("n", Text(v)));
                        "StateRef"
                    }
                    Ast::RuntimeRef { n, ref_type } => {
                        fields.extend([("n", Text(n)), ("refType", Text(ref_type.as_str()))]);
                        "RuntimeRef"
                    }
                    Ast::BinOp { op, left, right } => {
                        fields.extend([
                            ("op", Text(op)),
                            ("left", ast(left)),
                            ("right", ast(right)),
                        ]);
                        "BinOp"
                    }
                    Ast::UnaryOp { op, operand } => {
                        fields.extend([("op", Text(op)), ("operand", ast(operand))]);
                        "UnaryOp"
                    }
                    Ast::Ternary {
                        cond,
                        then,
                        otherwise,
                    } => {
                        fields.extend([
                            ("cond", ast(cond)),
                            ("then", ast(then)),
                            ("else", ast(otherwise)),
                        ]);
                        "Ternary"
                    }
                    Ast::Member { obj, field } => {
                        fields.extend([("obj", ast(obj)), ("field", Text(field))]);
                        "Member"
                    }
                    Ast::Index { obj, index } => {
                        fields.extend([("obj", ast(obj)), ("index", ast(index))]);
                        "Index"
                    }
                    Ast::Assign { target, value } => {
                        fields.extend([("target", Text(target)), ("value", ast(value))]);
                        "Assign"
                    }
                };
                fields.push(("k", Text(kind)));
            }
            _ => return None,
        }
        Some(Self::Typed(fields))
    }
    fn len(&self) -> usize {
        match self {
            Self::Plain(o) => o.len(),
            Self::Typed(fields) => fields.len(),
        }
    }
    fn entries(&self) -> impl Iterator<Item = (&str, Property<'_>)> {
        let plain = match self {
            Self::Plain(o) => Some(*o),
            _ => None,
        };
        let typed = match self {
            Self::Typed(fields) => fields.as_slice(),
            _ => &[],
        };
        plain
            .into_iter()
            .flat_map(|o| o.iter().map(|(k, v)| (k.as_str(), Property::Data(v))))
            .chain(typed.iter().map(|(k, v)| (*k, *v)))
    }
    fn get(&self, key: &str) -> Property<'_> {
        match self {
            Self::Plain(o) => o.get(key).map(Property::Data).unwrap_or_else(|| {
                if o.has(key) {
                    Property::Inherited
                } else {
                    Property::Undefined
                }
            }),
            Self::Typed(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| *v)
                .unwrap_or_else(|| {
                    if JsObject::<Value>::new().has(key) {
                        Property::Inherited
                    } else {
                        Property::Undefined
                    }
                }),
        }
    }
}

#[cfg(test)]
mod lookup_work {
    use super::*;
    thread_local! { static HASHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    pub(super) fn record(name: &str) {
        HASHED.set(HASHED.get().saturating_add(name.len()));
    }
    #[test]
    fn impossible_names_never_reach_store_hash_lookups() {
        // Library tests run before integration binaries. Probe construction in
        // a CPU-bounded child here too, so an optimized recursive constructor
        // cannot hang this first test before the public integration probe runs.
        #[cfg(unix)]
        {
            if std::env::var_os("OPENUI_STORE_CONSTRUCTOR_UNIT_CHILD").is_some() {
                let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(0));
                assert!(matches!(
                    store.set("a", Some(Value::Num(1.))),
                    Err(StoreError::ValueTooLarge)
                ));
                assert!(store.snapshot().is_empty());
                assert_eq!(store.version(), 0);
                assert!(!store.is_pristine("a"));
                return;
            }
            let status = std::process::Command::new("sh")
                .args([
                    "-c",
                    "ulimit -c 0; ulimit -t 1; exec \"$@\"",
                    "store-constructor",
                ])
                .arg(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "store::lookup_work::impossible_names_never_reach_store_hash_lookups",
                    "--test-threads=1",
                ])
                .env("OPENUI_STORE_CONSTRUCTOR_UNIT_CHILD", "1")
                .status()
                .unwrap();
            assert!(
                status.success(),
                "constructor child did not complete successfully: {status}"
            );
        }
        let mut store = Store::new();
        store.set("$n", Some(Value::Num(1.))).unwrap();
        let name = "n".repeat(1_000_000);
        HASHED.set(0);
        for _ in 0..1000 {
            assert!(store.get(&name).is_none());
            assert!(!store.is_pristine(&name));
        }
        assert_eq!(HASHED.get(), 0);
        assert!(store.get("$n").is_some());
        assert!(!store.is_pristine("$n"));
        assert_eq!(HASHED.get(), 4);
    }
}
