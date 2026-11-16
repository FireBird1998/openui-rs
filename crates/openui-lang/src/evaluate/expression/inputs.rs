//! Core-measured supplied input, separate from evaluation work and ownership.
use super::*;

/// A core-measured program. Contains no program owner or supplied data.
#[derive(Clone, Copy)]
pub struct PreparedProgram {
    pub(super) bytes: usize,
    limits: Limits,
    flags: EvaluationFlags,
}
impl PreparedProgram {
    /// Measure parser output without evaluation, cloning it or allocating JSON.
    pub fn new(result: &crate::ParseResult, limits: Limits) -> Self {
        let mut walk = super::input_size::InputSize::new(limits);
        walk.program(result);
        // Preserve drawing's existing setup refusals independently of the
        // shared byte measure. Actions keep the successfully measured parts
        // and raise a refused entry's flag only when that entry is evaluated.
        let flags = match super::program_setup::check(result, limits) {
            Ok(()) => EvaluationFlags::default(),
            Err(depth) => EvaluationFlags {
                depth_limited: depth,
                size_limited: !depth,
            },
        };
        Self {
            bytes: walk.measured_bytes(),
            limits,
            flags,
        }
    }
    /// Measurement depth/work flags. Action setup keeps partial credit; drawing
    /// can propagate these flags before evaluating an unusable program.
    pub fn flags(&self) -> EvaluationFlags {
        self.flags
    }
    /// Bytes contributing to scaled evaluation allowances.
    pub fn input_bytes(&self) -> usize {
        self.bytes
    }
}
/// Starting action, message and form-name measurement. No data is retained.
#[derive(Clone, Copy)]
pub struct PreparedStart {
    bytes: usize,
    limits: Limits,
}
/// Current program, either measured now or cached under the same limits.
pub enum SuppliedProgram<'a> {
    /// A borrowed program measured as caller setup.
    Raw(&'a crate::ParseResult),
    /// A core receipt; reuse under different limits is refused.
    Prepared(&'a PreparedProgram),
}
/// Starting input, either measured now or retained as a receipt across resumes.
pub enum SuppliedStart<'a> {
    /// Measure each supplied part independently, retaining successful prefixes.
    Raw {
        /// Deferred action retained for this run.
        action: Option<&'a Value>,
        /// Original user message.
        message: &'a str,
        /// Optional named form.
        form: Option<&'a str>,
    },
    /// The original run's starting receipt, with no retained supplied data.
    Prepared(&'a PreparedStart),
}
/// One supply. Each named category occurs once, so receipts cannot be summed
/// with themselves. Computed callback results must never be supplied as input.
pub struct SuppliedInput<'a> {
    /// Current program.
    pub program: SuppliedProgram<'a>,
    /// Retained starting input.
    pub start: SuppliedStart<'a>,
    /// Current reference preparation, using drawing's own measurement.
    pub references: Option<&'a PreparedReferenceInput>,
}
/// Verified bytes from one complete reference preparation, including shared
/// bodies once and successful keys. Only the core can construct this receipt.
pub struct PreparedReferenceInput {
    pub(super) bytes: usize,
}
impl PreparedReferenceInput {
    /// Bytes earned by the successfully prepared entries.
    pub fn input_bytes(&self) -> usize {
        self.bytes
    }
}
/// Cached setup receipts returned after raising the one run-wide input floor.
pub struct SuppliedInputReceipt {
    /// Cache for the current immutable program.
    pub program: PreparedProgram,
    /// Cache for the original action/message/form inputs.
    pub start: PreparedStart,
}
/// A supplied receipt was measured under different evaluation limits, or the
/// owning evaluation had already cut. Refusal changes no accounting state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSupplyError {
    /// Prepare raw input again under the current evaluation's limits.
    DifferentLimits,
    /// This allowance has already cut; no input can revive it.
    Cut,
}
impl std::fmt::Display for InputSupplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for InputSupplyError {}

pub(super) fn supply(
    parts: SuppliedInput<'_>,
    limits: Limits,
) -> Result<SuppliedInputReceipt, InputSupplyError> {
    let program = match parts.program {
        SuppliedProgram::Raw(p) => PreparedProgram::new(p, limits),
        SuppliedProgram::Prepared(p) if p.limits == limits => *p,
        _ => return Err(InputSupplyError::DifferentLimits),
    };
    let start = match parts.start {
        SuppliedStart::Prepared(s) if s.limits == limits => *s,
        SuppliedStart::Prepared(_) => return Err(InputSupplyError::DifferentLimits),
        SuppliedStart::Raw {
            action,
            message,
            form,
        } => {
            let measure = |f: &dyn Fn(&mut super::input_size::InputSize)| {
                let mut walk = super::input_size::InputSize::new(limits);
                f(&mut walk);
                walk.measured_bytes()
            };
            let mut bytes = measure(&|w| w.text(message));
            if let Some(form) = form {
                bytes = bytes.saturating_add(measure(&|w| w.text(form)));
            }
            if let Some(action) = action {
                bytes = bytes.saturating_add(measure(&|w| w.value(action, 0)));
            }
            PreparedStart { bytes, limits }
        }
    };
    let bytes = program
        .bytes
        .saturating_add(start.bytes)
        .saturating_add(parts.references.map_or(0, |r| r.bytes));
    ACCOUNT.with(|a| {
        let mut a = a.borrow_mut();
        let extra = bytes.saturating_sub(a.supplied_floor);
        a.supplied_floor = a.supplied_floor.max(bytes);
        a.credit(extra);
    });
    Ok(SuppliedInputReceipt { program, start })
}
struct InputWalk;
impl InputWalk {
    fn bytes(&mut self, bytes: usize) {
        data_bytes(bytes, true);
    }
    fn text(&mut self, s: &str) {
        data_text(s, true);
        self.bytes(0);
    }
    fn alive(&self) -> bool {
        LIMITS.with(|l| l.cut.get().is_none())
    }
    fn value(&mut self, v: &Value) {
        crate::stack::grow_stack!(self.value(v), budget else ());
        let Some(_level) = Level::enter() else {
            return;
        };
        if let Some(key) = data_identity(v) {
            let fresh = ACCOUNT.with(|a| a.borrow_mut().inputs.insert(key));
            if !fresh {
                return;
            }
        }
        if !step(1) {
            return;
        }
        match v {
            Value::Str(s) => self.text(s),
            Value::Null => self.bytes(4),
            Value::Bool(b) => self.bytes(if *b { 4 } else { 5 }),
            Value::Num(n) => self.bytes(crate::json_number_len(*n)),
            Value::Ast(a) => self.ast(a),
            Value::Element(e) => self.element(e),
            Value::Arr(a) => {
                self.bytes(2usize.saturating_add(a.len().saturating_sub(1)));
                for v in a.iter() {
                    if !self.alive() {
                        break;
                    }
                    self.value(v);
                }
            }
            Value::Obj(o) => self.object(o),
        }
    }
    fn object(&mut self, o: &JsObject<Value>) {
        self.bytes(2usize.saturating_add(o.len().saturating_sub(1)));
        for (k, v) in o.iter() {
            if !self.alive() {
                break;
            }
            self.text(k);
            self.bytes(1);
            self.value(v);
        }
    }
    fn element(&mut self, e: &crate::Element) {
        let Some(_level) = Level::enter() else {
            return;
        };
        if !step(1) {
            return;
        }
        self.bytes(2);
        self.text("type");
        self.text("element");
        self.text("typeName");
        self.text(&e.type_name);
        self.text("props");
        self.bytes(2usize.saturating_add(e.props.len().saturating_sub(1)));
        for (k, v) in e.props.iter() {
            if !self.alive() {
                return;
            }
            self.text(k);
            self.bytes(1);
            self.value(v);
        }
        self.bytes(9);
        self.text("partial");
        self.bytes(if e.partial { 4 } else { 5 });
        self.text("hasDynamicProps");
        self.bytes(if e.has_dynamic_props { 4 } else { 5 });
        if let Some(id) = &e.statement_id {
            self.bytes(2);
            self.text("statementId");
            self.text(id);
        }
    }
    fn ast(&mut self, node: &Ast) {
        crate::stack::grow_stack!(self.ast(node), budget else ());
        let Some(_level) = Level::enter() else {
            return;
        };
        let key = (9, std::ptr::from_ref(node) as usize);
        let fresh = ACCOUNT.with(|a| a.borrow_mut().inputs.insert(key));
        if !fresh {
            return;
        }
        if !step(1) {
            return;
        }
        self.bytes(2);
        for (i, (key, field)) in fields(node).into_iter().enumerate() {
            if !step(1) {
                return;
            }
            self.text(key);
            self.bytes(1usize.saturating_add(usize::from(i != 0)));
            match field {
                Field::Text(s) => self.text(s),
                Field::Num(n) => self.bytes(crate::json_number_len(n)),
                Field::Bool(b) => self.bytes(if b { 4 } else { 5 }),
                Field::Node(n) => self.ast(n),
                Field::Nodes(nodes) => {
                    self.bytes(2usize.saturating_add(nodes.len().saturating_sub(1)));
                    for n in nodes.iter() {
                        if !self.alive() {
                            return;
                        }
                        self.ast(n);
                    }
                }
                Field::Entries(entries) => {
                    self.bytes(2usize.saturating_add(entries.len().saturating_sub(1)));
                    for (k, n) in entries.iter() {
                        if !self.alive() {
                            return;
                        }
                        self.text(k);
                        self.bytes(3);
                        self.ast(n);
                    }
                }
                Field::Props(props) => {
                    self.bytes(2usize.saturating_add(props.len().saturating_sub(1)));
                    for (k, v) in props.iter() {
                        if !self.alive() {
                            return;
                        }
                        self.text(k);
                        self.bytes(1);
                        self.ast(v);
                    }
                }
            }
        }
    }
}

pub(super) fn register_value(value: &Value) {
    if LIMITS.with(|l| l.cut.get().is_some()) {
        return;
    }
    if let Some(key) = data_identity(value) {
        let seen = ACCOUNT.with(|a| {
            let mut a = a.borrow_mut();
            if a.inputs.contains(&key) {
                true
            } else {
                a.supplied.owners.push(value.clone());
                false
            }
        });
        if seen {
            return;
        }
    }
    InputWalk.value(value);
}
