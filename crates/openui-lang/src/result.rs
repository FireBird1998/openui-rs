//! The parse result and everything in it, written as the reference parser's
//! `JSON.stringify` output: same keys, same order, same formatting.

use indexmap::IndexMap;

use crate::js::write_json_string;

/// A materialized prop value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Str(String),
    Element(Box<Element>),
}

/// A component instance with its positional arguments mapped to named props.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub type_name: String,
    pub props: IndexMap<String, Value>,
    /// The element's text hasn't fully arrived yet.
    pub partial: bool,
    /// Some prop needs runtime evaluation.
    pub has_dynamic_props: bool,
    /// The statement this element came from; unset for inline components.
    pub statement_id: Option<String>,
}

/// What kind of validation problem was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationErrorCode {
    MissingRequired,
    NullRequired,
    UnknownComponent,
}

impl ValidationErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ValidationErrorCode::MissingRequired => "missing-required",
            ValidationErrorCode::NullRequired => "null-required",
            ValidationErrorCode::UnknownComponent => "unknown-component",
        }
    }
}

/// A prop validation problem.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationError {
    pub code: ValidationErrorCode,
    pub component: String,
    /// JSON Pointer into the component's props, such as `/title`, or empty.
    pub path: String,
    pub message: String,
    pub statement_id: Option<String>,
}

/// Facts about the parse as a whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// The program's text hasn't fully arrived yet.
    pub incomplete: bool,
    /// References that point at no statement, in the order they were met.
    pub unresolved: Vec<String>,
    /// Statements nothing reaches from the root.
    pub orphaned: Vec<String>,
    pub statement_count: usize,
    pub errors: Vec<ValidationError>,
}

/// What parsing returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParseResult {
    pub root: Option<Element>,
    pub meta: Meta,
    pub state_declarations: IndexMap<String, Value>,
}

impl ParseResult {
    /// No root, nothing found.
    pub(crate) fn empty(incomplete: bool) -> ParseResult {
        ParseResult {
            meta: Meta {
                incomplete,
                ..Meta::default()
            },
            ..ParseResult::default()
        }
    }

    /// The result as the reference parser's `JSON.stringify` writes it.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push_str("{\"root\":");
        match &self.root {
            Some(root) => write_element(&mut out, root),
            None => out.push_str("null"),
        }
        out.push_str(",\"meta\":{\"incomplete\":");
        write_bool(&mut out, self.meta.incomplete);
        out.push_str(",\"unresolved\":");
        write_strings(&mut out, &self.meta.unresolved);
        out.push_str(",\"orphaned\":");
        write_strings(&mut out, &self.meta.orphaned);
        out.push_str(",\"statementCount\":");
        out.push_str(&self.meta.statement_count.to_string());
        out.push_str(",\"errors\":[");
        for (i, error) in self.meta.errors.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_error(&mut out, error);
        }
        out.push_str("]},\"stateDeclarations\":");
        write_map(&mut out, &self.state_declarations);
        // Query and mutation statements aren't extracted yet.
        out.push_str(",\"queryStatements\":[],\"mutationStatements\":[]}");
        out
    }
}

fn write_bool(out: &mut String, b: bool) {
    out.push_str(if b { "true" } else { "false" });
}

fn write_strings(out: &mut String, items: &[String]) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_string(out, item);
    }
    out.push(']');
}

fn write_map(out: &mut String, map: &IndexMap<String, Value>) {
    out.push('{');
    for (i, (key, value)) in map.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_string(out, key);
        out.push(':');
        write_value(out, value);
    }
    out.push('}');
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Str(s) => write_json_string(out, s),
        Value::Element(e) => write_element(out, e),
    }
}

fn write_element(out: &mut String, e: &Element) {
    out.push_str("{\"type\":\"element\",\"typeName\":");
    write_json_string(out, &e.type_name);
    out.push_str(",\"props\":");
    write_map(out, &e.props);
    out.push_str(",\"partial\":");
    write_bool(out, e.partial);
    out.push_str(",\"hasDynamicProps\":");
    write_bool(out, e.has_dynamic_props);
    if let Some(id) = &e.statement_id {
        out.push_str(",\"statementId\":");
        write_json_string(out, id);
    }
    out.push('}');
}

fn write_error(out: &mut String, e: &ValidationError) {
    out.push_str("{\"code\":");
    write_json_string(out, e.code.as_str());
    out.push_str(",\"component\":");
    write_json_string(out, &e.component);
    out.push_str(",\"path\":");
    write_json_string(out, &e.path);
    out.push_str(",\"message\":");
    write_json_string(out, &e.message);
    if let Some(id) = &e.statement_id {
        out.push_str(",\"statementId\":");
        write_json_string(out, id);
    }
    out.push('}');
}
