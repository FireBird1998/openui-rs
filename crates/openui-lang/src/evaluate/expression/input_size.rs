//! Allocation-free AST traversal; only shared allocations need identity memoization.
use super::*;
pub(super) struct InputSize {
    pub(super) bytes: usize,
    ceiling: usize,
    depth: usize,
    work: usize,
    done: bool,
    refused: bool,
    seen: HashMap<(u8, usize), usize>,
}
impl InputSize {
    pub(super) fn new(limits: Limits) -> Self {
        Self {
            bytes: 0,
            ceiling: Limits::DEFAULT.result_size().max(limits.result_size()),
            depth: limits.depth().saturating_mul(VALUE_LEVELS_PER_LEVEL),
            work: Limits::DEFAULT
                .work_for(usize::MAX)
                .max(limits.work_for(usize::MAX)),
            done: false,
            refused: false,
            seen: HashMap::new(),
        }
    }
    fn clamp(&mut self) {
        self.bytes = self.ceiling;
        self.done = true;
    }
    fn add(&mut self, n: usize) {
        self.bytes = self.bytes.saturating_add(n).min(self.ceiling);
        if self.bytes == self.ceiling {
            self.done = true;
        }
    }
    fn enter(&mut self, d: usize) -> bool {
        if self.done {
            return false;
        }
        if d >= self.depth || self.work == 0 {
            self.refused = true;
            self.done = true;
            return false;
        }
        self.work -= 1;
        true
    }
    pub(super) fn text(&mut self, s: &str) {
        if self.done {
            return;
        }
        if s.len() > self.work {
            self.refused = true;
            self.done = true;
            return;
        }
        if s.len() > self.ceiling.saturating_sub(self.bytes) {
            self.clamp();
            return;
        }
        self.work -= s.len();
        self.add(crate::json_string_len(s).saturating_add(2));
    }
    pub(super) fn measured_bytes(&self) -> usize {
        if self.refused { 0 } else { self.bytes }
    }
    fn first<T>(&mut self, kind: u8, a: &Arc<T>) -> bool {
        Arc::strong_count(a) == 1
            || self
                .seen
                .insert((kind, Arc::as_ptr(a) as usize), 0)
                .is_none()
    }
    pub(super) fn program(&mut self, p: &crate::ParseResult) {
        self.add(2);
        if let Some(e) = &p.root {
            self.element(e, 0)
        }
        if let Some(a) = &p.root_expression {
            self.shared_ast(a, 0)
        }
        for (k, v) in p.state_declarations.iter() {
            if self.done {
                return;
            }
            self.text(k);
            self.value(v, 0)
        }
        for q in &p.query_statements {
            if self.done {
                return;
            }
            self.text(&q.statement_id);
            if let Some(a) = &q.defaults {
                self.ast(a, 0)
            }
        }
        for m in &p.mutation_statements {
            if self.done {
                return;
            }
            self.text(&m.statement_id);
            if let Some(a) = &m.args {
                self.ast(a, 0)
            }
        }
    }
    fn add_if_live(&mut self, n: usize) {
        if !self.done {
            self.add(n)
        }
    }
    pub(super) fn value(&mut self, v: &Value, d: usize) {
        crate::stack::grow_stack!(self.value(v, d));
        if !self.enter(d) {
            return;
        }
        match v {
            Value::Null => self.add(4),
            Value::Bool(b) => self.add(if *b { 4 } else { 5 }),
            Value::Num(n) => self.add(crate::json_number_len(*n)),
            Value::Str(s) => self.text(s),
            Value::Ast(a) => {
                if self.first(3, a) {
                    self.shared_ast(a, d.saturating_add(1))
                }
            }
            Value::Element(e) => {
                if self.first(2, e) {
                    self.element(e, d.saturating_add(1))
                }
            }
            Value::Arr(a) => {
                if !self.first(0, a) {
                    return;
                }
                self.add(2usize.saturating_add(a.len().saturating_sub(1)));
                for v in a.iter() {
                    if self.done {
                        break;
                    }
                    self.value(v, d.saturating_add(1))
                }
            }
            Value::Obj(o) => {
                if !self.first(1, o) {
                    return;
                }
                self.add(2usize.saturating_add(o.len().saturating_sub(1)));
                for (k, v) in o.iter() {
                    if self.done {
                        break;
                    }
                    self.text(k);
                    self.add_if_live(1);
                    self.value(v, d.saturating_add(1))
                }
            }
        }
    }
    fn element(&mut self, e: &crate::Element, d: usize) {
        if !self.enter(d) {
            return;
        }
        self.add(2);
        self.text("type");
        self.text("element");
        self.text("typeName");
        self.text(&e.type_name);
        self.text("props");
        self.add(2usize.saturating_add(e.props.len().saturating_sub(1)));
        for (k, v) in e.props.iter() {
            if self.done {
                return;
            }
            self.text(k);
            self.add_if_live(1);
            self.value(v, d.saturating_add(1))
        }
        self.add_if_live(9);
        self.text("partial");
        self.add_if_live(if e.partial { 4 } else { 5 });
        self.text("hasDynamicProps");
        self.add_if_live(if e.has_dynamic_props { 4 } else { 5 });
        if let Some(id) = &e.statement_id {
            self.add_if_live(2);
            self.text("statementId");
            self.text(id)
        }
    }
    fn shared_ast(&mut self, a: &Arc<Ast>, d: usize) {
        if self.first(9, a) {
            self.ast(a, d)
        }
    }
    fn nodes(&mut self, a: &Arc<Vec<Ast>>, d: usize) {
        self.add_if_live(2usize.saturating_add(a.len().saturating_sub(1)));
        if self.first(20, a) {
            for n in a.iter() {
                if self.done {
                    break;
                }
                self.ast(n, d)
            }
        }
    }
    fn props(&mut self, p: &Arc<JsObject<Ast>>, d: usize) {
        self.add_if_live(2usize.saturating_add(p.len().saturating_sub(1)));
        let key = (21, Arc::as_ptr(p) as usize);
        if Arc::strong_count(p) > 1
            && let Some(bytes) = self.seen.get(&key)
        {
            self.add_if_live(*bytes);
            return;
        }
        let mut keys = 0usize;
        for (k, v) in p.iter() {
            if self.done {
                break;
            }
            let before = self.bytes;
            self.text(k);
            self.add_if_live(1);
            keys = keys.saturating_add(self.bytes.saturating_sub(before));
            self.ast(v, d)
        }
        if Arc::strong_count(p) > 1 {
            self.seen.insert(key, keys);
        }
    }
    fn ast(&mut self, a: &Ast, d: usize) {
        crate::stack::grow_stack!(self.ast(a, d));
        if !self.enter(d) {
            return;
        }
        let next = d.saturating_add(1);
        match a {
            Ast::Str(s) => {
                self.add(r#"{"k":"Str","v":}"#.len());
                self.text(s)
            }
            Ast::Num(n) => {
                self.add(
                    r#"{"k":"Num","v":}"#
                        .len()
                        .saturating_add(crate::json_number_len(*n)),
                );
            }
            Ast::Bool(b) => {
                self.add(
                    r#"{"k":"Bool","v":}"#
                        .len()
                        .saturating_add(if *b { 4 } else { 5 }),
                );
            }
            Ast::Null => {
                self.add(r#"{"k":"Null"}"#.len());
            }
            Ast::Ref(n) => {
                self.add(r#"{"k":"Ref","n":}"#.len());
                self.text(n)
            }
            Ast::Ph(n) => {
                self.add(r#"{"k":"Ph","n":}"#.len());
                self.text(n)
            }
            Ast::StateRef(n) => {
                self.add(r#"{"k":"StateRef","n":}"#.len());
                self.text(n)
            }
            Ast::RuntimeRef { n, ref_type } => {
                self.add(r#"{"k":"RuntimeRef","n":,"refType":}"#.len());
                self.text(n);
                self.text(ref_type.as_str())
            }
            Ast::Arr(a) => {
                self.add(r#"{"k":"Arr","els":}"#.len());
                self.nodes(a, next)
            }
            Ast::Obj(entries) => {
                self.add(
                    r#"{"k":"Obj","entries":[]}"#
                        .len()
                        .saturating_add(entries.len().saturating_sub(1)),
                );
                let key = (22, Arc::as_ptr(entries) as usize);
                if Arc::strong_count(entries) > 1
                    && let Some(bytes) = self.seen.get(&key)
                {
                    self.add_if_live(*bytes);
                    return;
                }
                let mut keys = 0usize;
                for (k, v) in entries.iter() {
                    if self.done {
                        break;
                    }
                    let before = self.bytes;
                    self.text(k);
                    self.add_if_live(3);
                    keys = keys.saturating_add(self.bytes.saturating_sub(before));
                    self.ast(v, next)
                }
                if Arc::strong_count(entries) > 1 {
                    self.seen.insert(key, keys);
                }
            }
            Ast::Comp {
                name,
                args,
                function,
                mapped_props,
            } => {
                self.add(r#"{"k":"Comp","name":,"args":}"#.len());
                self.text(name);
                self.nodes(args, next);
                if *function {
                    self.add_if_live(r#","fn":true"#.len())
                }
                if let Some(p) = mapped_props {
                    self.add_if_live(r#","mappedProps":"#.len());
                    self.props(p, next)
                }
            }
            Ast::BinOp { op, left, right } => {
                self.add(r#"{"k":"BinOp","op":,"left":,"right":}"#.len());
                self.text(op);
                self.shared_ast(left, next);
                self.shared_ast(right, next)
            }
            Ast::UnaryOp { op, operand } => {
                self.add(r#"{"k":"UnaryOp","op":,"operand":}"#.len());
                self.text(op);
                self.shared_ast(operand, next)
            }
            Ast::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.add(r#"{"k":"Ternary","cond":,"then":,"else":}"#.len());
                self.shared_ast(cond, next);
                self.shared_ast(then, next);
                self.shared_ast(otherwise, next)
            }
            Ast::Member { obj, field } => {
                self.add(r#"{"k":"Member","obj":,"field":}"#.len());
                self.shared_ast(obj, next);
                self.text(field)
            }
            Ast::Index { obj, index } => {
                self.add(r#"{"k":"Index","obj":,"index":}"#.len());
                self.shared_ast(obj, next);
                self.shared_ast(index, next)
            }
            Ast::Assign { target, value } => {
                self.add(r#"{"k":"Assign","target":,"value":}"#.len());
                self.text(target);
                self.shared_ast(value, next)
            }
        }
    }
}
