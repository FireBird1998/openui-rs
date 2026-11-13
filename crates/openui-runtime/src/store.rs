//! The pinned store's state transitions, without embedder callbacks.

use crate::{Ast, JsObject, Limits, Value};
use std::{collections::HashSet, fmt, sync::Arc};

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
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionExhausted => f.write_str("store version exhausted"),
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
    /// Why it was refused. #39 can add admission-limit errors here.
    pub error: StoreError,
}

/// Initialization's accepted changes and per-name refusals.
/// Limits are not enforced yet; #39 will report and skip each refused entry,
/// continuing the batch. A successful entry is not rolled back by a later refusal.
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
/// Pristine-only changes and empty disposal do not advance it.
///
/// Writes consume values. Reads and retained-snapshot copies clone `Value`:
/// container bodies stay shared; scalar strings and copied keys allocate.
/// Unchanged initialization and ignored scalar sets do not copy the snapshot
/// or pristine set. Use borrowed iterators to avoid constructing input maps.
///
/// Admission limits belong to #39. A total-state cap must count pristine names
/// too: `set_pristine(name, None)` can add an absent name without changing the
/// snapshot. `with_limits` retains configuration for that work.
pub struct Store {
    snapshot: StoreSnapshot,
    pristine: HashSet<String>,
    version: u64,
    // Retained for #39; there is no current read-only configuration API to maintain.
    _limits: Limits,
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
    /// An empty store retaining these limits for state admission (#39).
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            snapshot: Arc::new(JsObject::new()),
            pristine: HashSet::new(),
            version: 0,
            _limits: limits,
        }
    }
    /// Read a value. Absent and explicitly undefined both return `None`;
    /// snapshot membership distinguishes their presence.
    pub fn get(&self, name: &str) -> StateValue {
        self.snapshot.get(name).cloned().flatten()
    }
    /// Whether a later declared default may replace this name, including absent
    /// names marked pristine by an ignored undefined set.
    pub fn is_pristine(&self, name: &str) -> bool {
        self.pristine.contains(name)
    }
    /// Read the current immutable snapshot. Reads and ignored sets retain identity.
    pub fn snapshot(&self) -> StoreSnapshot {
        self.snapshot.clone()
    }
    /// Upstream notification count, plus nonempty disposals (see `Store`).
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
    fn mark_pristine(&mut self, name: &str) {
        if !self.pristine.contains(name) {
            self.pristine.insert(name.to_owned());
        }
    }
    fn put(&mut self, name: &str, value: StateValue) {
        let state = Arc::make_mut(&mut self.snapshot);
        if let Some(existing) = state.get_mut(name) {
            *existing = value;
        } else {
            state.define(name, value);
        }
    }
    fn write(&mut self, name: &str, value: StateValue, pristine: bool) -> Result<bool, StoreError> {
        let existing = self.snapshot.get(name).and_then(Option::as_ref);
        let changed = !same(existing, value.as_ref()) && !shallow_equal(existing, value.as_ref());
        let version = if changed {
            self.next_version()?
        } else {
            self.version
        };
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
    /// Apply persisted entries first, then declared defaults, retaining names.
    ///
    /// Inputs are borrowed `(name, value)` entries in JavaScript property order;
    /// parsed declarations use `.iter().map(|(k,v)| (k.as_str(),Some(v)))`;
    /// optional persisted values use `v.as_ref()`. No staging map is required.
    /// Core `JsObject::iter` builds an ordering vector; slice and ordinary map
    /// iterators avoid even that input-adapter allocation. Within each input,
    /// the last value for a duplicate name wins, as in a JavaScript object.
    /// Normalization is deferred until a candidate value change, preserving
    /// the allocation-free path for unchanged, unique entries.
    ///
    /// Each refusal leaves that entry untouched and is reported by name; the
    /// rest of the batch continues. #39 will use this for individual admission
    /// failures, rather than dropping every default in a streaming chunk.
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
        if changed && !self.admit_initial_change(name, result) {
            return;
        }
        if changed {
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
    /// Clear values and pristine status; the store can be reused.
    /// Upstream clears listeners without notifying. This runtime extends that
    /// behavior by advancing version for nonempty state. Empty disposal replaces
    /// the snapshot too, but does not advance version. Disposal always clears;
    /// at version exhaustion it leaves the version at `u64::MAX`.
    pub fn dispose(&mut self) -> bool {
        let changed = !self.snapshot.is_empty();
        if changed {
            self.version = self.version.saturating_add(1);
        }
        self.snapshot = Arc::new(JsObject::new());
        self.pristine.clear();
        changed
    }
    fn next_version(&self) -> Result<u64, StoreError> {
        self.version
            .checked_add(1)
            .ok_or(StoreError::VersionExhausted)
    }
}

// Object.is, not Rust's structural Value equality or JavaScript's ===.
fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
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
            (Self::Undefined, Self::Undefined) => true,
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
