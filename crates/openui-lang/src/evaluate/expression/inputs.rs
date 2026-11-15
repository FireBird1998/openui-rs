//! Core-measured supplied input, separate from evaluation work and ownership.
use super::*;

/// A bounded input measurement, containing only bytes and no retained owners.
#[derive(Clone, Copy)]
pub struct PreparedInput {
    pub(super) bytes: usize,
}
impl PreparedInput {
    /// Measure parser output without evaluation, cloning it or allocating JSON.
    /// Measurement has its own hard work/size bounds and the supplied depth.
    pub fn from_parse_result(result: &crate::ParseResult, limits: Limits) -> Self {
        Self::measure(limits, |walk| walk.program(result))
    }
    /// Measure a supplied value without retaining it or spending evaluation work.
    /// A part exceeding the measurement depth/work bound earns no credit.
    pub fn from_value(value: &Value, limits: Limits) -> Self {
        Self::measure(limits, |walk| walk.value(value, 0))
    }
    /// Combine cached reference contributions from one prior bounded preparation.
    /// Refused entries must be omitted. Each value's size has the same prepaid
    /// measurement contract as [`PreparedReferenceValue::new`]. Keys and object
    /// punctuation are included; no value is measured again or retained.
    pub fn from_prepared_references<'a>(
        references: impl IntoIterator<Item = (&'a str, &'a PreparedReferenceValue)>,
    ) -> Self {
        let mut bytes = 2usize;
        for (name, value) in references {
            // Match supplied-object sizing, including literal encoded surrogates.
            let key = name.chars().fold(2usize, |n, c| {
                n.saturating_add(match c {
                    '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                    c if c < '\u{20}' => 6,
                    c => c.len_utf8(),
                })
            });
            bytes = bytes
                .saturating_add(key)
                .saturating_add(value.bytes())
                .saturating_add(2);
        }
        Self { bytes }
    }
    /// Measure supplied text including its JSON quotes and escapes.
    pub fn from_text(text: &str, limits: Limits) -> Self {
        Self::measure(limits, |walk| walk.text(text))
    }
    /// Add independently measured contributions to one supply, saturating only
    /// on integer overflow. No input or ownership is retained. Registering this
    /// combined token raises one floor; successive supplies are never summed.
    #[must_use]
    pub fn combined(self, other: Self) -> Self {
        Self {
            bytes: self.bytes.saturating_add(other.bytes),
        }
    }
    fn measure(limits: Limits, f: impl FnOnce(&mut super::input_size::InputSize)) -> Self {
        let mut walk = super::input_size::InputSize::new(limits);
        f(&mut walk);
        Self {
            bytes: walk.measured_bytes(),
        }
    }
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
            super::check_input_depth(value);
            return;
        }
    }
    InputWalk.value(value);
}
