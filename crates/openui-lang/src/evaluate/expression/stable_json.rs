//! The stable JSON projection used in tool cache keys. Keep this opt-in so
//! ordinary evaluator and parser outputs retain their existing projection.
use super::*;

enum Item<'a> {
    Js(&'a Js),
    Data(&'a Value),
    Object(&'a JsObject<Value>),
    Ast(&'a Ast),
    AstField(Field<'a>),
    AstEntry(&'a str, &'a Ast),
}

impl Item<'_> {
    fn omitted(&self) -> bool {
        matches!(self, Self::Js(Js::Function { .. }))
    }
}

pub(super) fn stable_json(value: &Js) -> Option<String> {
    let item = Item::Js(value);
    if item.omitted() {
        return None;
    }
    let mut writer = Writer(String::new());
    writer.write(item)?;
    Some(writer.0)
}

struct Writer(String);

impl Writer {
    fn bytes(&mut self, text: &str) -> Option<()> {
        if !spend(text.len()) || !memory::text(text.len()) {
            return None;
        }
        self.0.push_str(text);
        Some(())
    }

    fn string(&mut self, text: &str) -> Option<()> {
        if !step(text.len()) {
            return None;
        }
        let bytes = js::json_string_len(text).saturating_add(2);
        if !spend(bytes) || !memory::text(bytes) {
            return None;
        }
        js::write_json_string(&mut self.0, text);
        Some(())
    }

    fn number(&mut self, number: f64) -> Option<()> {
        if number.is_nan() {
            return self.string("__NaN__");
        }
        if number.is_infinite() {
            return self.string(if number.is_sign_negative() {
                "__-Inf__"
            } else {
                "__Inf__"
            });
        }
        let mut buffer = ryu_js::Buffer::new();
        self.bytes(buffer.format_finite(number))
    }

    fn write(&mut self, item: Item<'_>) -> Option<()> {
        crate::stack::grow_stack!(self.write(item), budget else None);
        let item = match item {
            Item::Js(Js::Data(value)) => Item::Data(value),
            item => item,
        };
        let _level = Level::enter()?;
        match item {
            Item::Js(Js::Undefined) => self.string("__undefined__"),
            Item::Js(Js::Function { .. } | Js::Null) | Item::Data(Value::Null) => {
                self.bytes("null")
            }
            Item::Js(Js::Bool(value)) | Item::Data(Value::Bool(value)) => {
                self.bytes(if *value { "true" } else { "false" })
            }
            Item::Js(Js::Num(value)) | Item::Data(Value::Num(value)) => self.number(*value),
            Item::Js(Js::Str(value)) => self.string(value),
            Item::Data(Value::Str(value)) => self.string(value),
            Item::Js(Js::Arr(items)) => self.array(items.iter().map(Item::Js)),
            Item::Data(Value::Arr(items)) => self.array(items.iter().map(Item::Data)),
            Item::Js(Js::Obj(fields)) => {
                self.object(fields.iter().map(|(k, v)| (k.as_str(), Item::Js(v))))
            }
            Item::Data(Value::Obj(object)) => self.data_object(object),
            Item::Object(object) => self.data_object(object),
            Item::Js(Js::Props(element)) => self.data_object(&element.props),
            Item::Js(Js::Ast(ast)) | Item::Data(Value::Ast(ast)) => self.ast(ast),
            Item::Ast(ast) => self.ast(ast),
            Item::AstField(field) => self.ast_field(field),
            Item::AstEntry(key, ast) => {
                self.array([Item::AstField(Field::Text(key)), Item::Ast(ast)].into_iter())
            }
            Item::Data(Value::Element(element)) => self.element(element),
            Item::Js(Js::Proto(Kind::Array)) => self.bytes("[]"),
            Item::Js(Js::Proto(_)) => self.bytes("{}"),
            Item::Js(Js::Data(_)) => None,
        }
    }

    fn ast(&mut self, ast: &Ast) -> Option<()> {
        self.object(fields(ast).into_iter().map(|(k, v)| (k, Item::AstField(v))))
    }

    fn ast_field(&mut self, field: Field<'_>) -> Option<()> {
        match field {
            Field::Text(text) => self.string(text),
            Field::Num(number) => self.number(number),
            Field::Bool(value) => self.bytes(if value { "true" } else { "false" }),
            Field::Node(ast) => self.write(Item::Ast(ast)),
            Field::Nodes(items) => self.array(items.iter().map(Item::Ast)),
            Field::Entries(entries) => {
                self.array(entries.iter().map(|(key, ast)| Item::AstEntry(key, ast)))
            }
            Field::Props(props) => self.object(
                props
                    .iter()
                    .map(|(key, ast)| (key.as_str(), Item::Ast(ast))),
            ),
        }
    }

    fn element(&mut self, element: &Element) -> Option<()> {
        self.object(
            [
                ("type", Item::AstField(Field::Text("element"))),
                ("typeName", Item::AstField(Field::Text(&element.type_name))),
                ("props", Item::Object(&element.props)),
                ("partial", Item::AstField(Field::Bool(element.partial))),
                (
                    "hasDynamicProps",
                    Item::AstField(Field::Bool(element.has_dynamic_props)),
                ),
            ]
            .into_iter()
            .chain(
                element
                    .statement_id
                    .as_deref()
                    .map(|id| ("statementId", Item::AstField(Field::Text(id)))),
            ),
        )
    }

    fn array<'a>(&mut self, items: impl Iterator<Item = Item<'a>>) -> Option<()> {
        self.bytes("[")?;
        for (index, value) in items.enumerate() {
            if index != 0 {
                self.bytes(",")?;
            }
            self.write(value)?;
        }
        self.bytes("]")
    }

    fn data_object(&mut self, fields: &JsObject<Value>) -> Option<()> {
        self.object(fields.iter().map(|(k, v)| (k.as_str(), Item::Data(v))))
    }

    fn object<'a>(&mut self, fields: impl Iterator<Item = (&'a str, Item<'a>)>) -> Option<()> {
        let mut sorted = Vec::new();
        for (key, value) in fields {
            if !step(key.len().saturating_add(1)) {
                return None;
            }
            // The reference assigns each key to a fresh ordinary JS object.
            if key != "__proto__" && !value.omitted() {
                if !memory::container(1) {
                    return None;
                }
                sorted.push((key, value));
            }
        }
        // Charge comparisons before sorting. JS enumerates index keys first,
        // even after the lexically sorted keys have been assigned.
        let passes = usize::BITS.saturating_sub(sorted.len().leading_zeros()) as usize;
        for (key, _) in &sorted {
            if !step(key.len().saturating_add(1).saturating_mul(passes)) {
                return None;
            }
        }
        sorted.sort_unstable_by(
            |(a, _), (b, _)| match (js::array_index(a), js::array_index(b)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => js::utf16_units(a).cmp(js::utf16_units(b)),
            },
        );
        self.bytes("{")?;
        for (index, (key, value)) in sorted.into_iter().enumerate() {
            if index != 0 {
                self.bytes(",")?;
            }
            self.string(key)?;
            self.bytes(":")?;
            self.write(value)?;
        }
        self.bytes("}")
    }
}
