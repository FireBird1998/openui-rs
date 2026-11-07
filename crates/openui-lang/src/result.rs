//! The parse result and everything in it, written as the reference parser's
//! `JSON.stringify` output: same keys, same order, same formatting.

use std::sync::Arc;

use serde_json::Value as Json;

use crate::ast::{Ast, write_ast};
use crate::js::{JsObject, json_number, write_json_number, write_json_string};

/// A materialized prop value: what the reference parser puts in an element's props.
///
/// It frees its tree with a loop rather than recursion, so dropping one needs
/// no stack however deep it nests. That makes it [`Drop`]: a pattern can't
/// move a field out, so borrow it and clone the `Arc` instead.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `null`: a literal, a dropped component, or an unresolved reference.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Num(f64),
    /// A string.
    Str(String),
    /// Shared, as everything below an element is: a statement inlined in many
    /// places is stored once. Clones are cheap; changes copy on write.
    Arr(Arc<Vec<Value>>),
    /// A plain object, keys in JavaScript's property order.
    Obj(Arc<JsObject<Value>>),
    /// A component instance. Shared: a streaming parse keeps elements that
    /// haven't changed between results, so an element may be part of several
    /// results at once.
    Element(Arc<Element>),
    /// A builtin call or runtime expression, kept for evaluation.
    Ast(Arc<Ast>),
}

impl Value {
    /// A JSON value as `JSON.parse` would give it (a schema default, say).
    pub fn from_json(json: &Json) -> Value {
        crate::stack::grow_stack!(Value::from_json(json));
        match json {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => Value::Num(json_number(n)),
            Json::String(s) => Value::Str(s.clone()),
            Json::Array(items) => {
                Value::Arr(Arc::new(items.iter().map(Value::from_json).collect()))
            }
            Json::Object(map) => {
                let mut obj = JsObject::new();
                for (k, v) in map {
                    obj.define(k.clone(), Value::from_json(v));
                }
                Value::Obj(Arc::new(obj))
            }
        }
    }
}

/// A component instance with its positional arguments mapped to named props.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    /// The component's name.
    pub type_name: String,
    /// Positional args mapped to the library's param names, validated, with
    /// defaults filled in.
    pub props: JsObject<Value>,
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
    /// A required prop wasn't given and has no default.
    MissingRequired,
    /// A required prop is `null` and has no default.
    NullRequired,
    /// The component isn't in the library or the builtins.
    UnknownComponent,
    /// `Query(...)` or `Mutation(...)` used as a value instead of a statement.
    InlineReserved,
    /// More positional args than the component has params.
    ExcessArgs,
    /// A prop's value doesn't match its schema.
    TypeMismatch,
    /// No usable entry: no `root` statement and no statement calling the
    /// library's root component, or a `root` that isn't a component. A
    /// warning when another statement renders in its place.
    NoRoot,
    /// `@Name(...)` with a name that's neither a builtin nor one of the
    /// library's functions or actions; the call is null.
    UnknownFunction,
    /// Queries that read each other's results, around a loop: none can run
    /// first, so none runs.
    QueryCycle,
}

impl ValidationErrorCode {
    /// The code as in the JSON: `"missing-required"` and so on.
    pub fn as_str(self) -> &'static str {
        match self {
            ValidationErrorCode::MissingRequired => "missing-required",
            ValidationErrorCode::NullRequired => "null-required",
            ValidationErrorCode::UnknownComponent => "unknown-component",
            ValidationErrorCode::InlineReserved => "inline-reserved",
            ValidationErrorCode::ExcessArgs => "excess-args",
            ValidationErrorCode::TypeMismatch => "type-mismatch",
            ValidationErrorCode::NoRoot => "no-root",
            ValidationErrorCode::UnknownFunction => "unknown-function",
            ValidationErrorCode::QueryCycle => "query-cycle",
        }
    }
}

/// How serious a validation problem is, when it's less than an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The program still renders: today only `no-root` when another statement
    /// stands in for the missing `root`.
    Warning,
}

impl Severity {
    /// The severity as in the JSON: `"warning"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
        }
    }
}

/// A prop validation problem.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationError {
    /// What kind of problem.
    pub code: ValidationErrorCode,
    /// The component it's about.
    pub component: String,
    /// JSON Pointer into the component's props, such as `/title`, or empty.
    pub path: String,
    /// The reference's wording, with signatures and expected types inlined.
    pub message: String,
    /// The statement the error is in, when there is one.
    pub statement_id: Option<String>,
    /// `Some` when the program renders despite it; `None` means an error.
    pub severity: Option<Severity>,
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
    /// How many statements the program has (streaming counts a redefinition
    /// again: see quirk 4).
    pub statement_count: usize,
    /// Prop validation problems, in the order they were met.
    pub errors: Vec<ValidationError>,
}

/// A `name = Query(tool, args, defaults, refresh)` statement, unevaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryStatement {
    /// The statement's name.
    pub statement_id: String,
    /// The tool name expression (first arg).
    pub tool: Option<Ast>,
    /// The tool arguments (second arg).
    pub args: Option<Ast>,
    /// Data to show before the query returns (third arg).
    pub defaults: Option<Ast>,
    /// The refresh interval in seconds (fourth arg).
    pub refresh: Option<Ast>,
    /// State variables the args read, when there are any.
    pub deps: Option<Vec<String>>,
    /// Queries whose results the tool name or args read, when there are any:
    /// it runs once each has a result.
    pub wait_for: Option<Vec<String>>,
    /// Always `true` in a parse result; the runtime tracks loading.
    pub complete: bool,
}

/// A `name = Mutation(tool, args)` statement, unevaluated.
#[derive(Clone, Debug, PartialEq)]
pub struct MutationStatement {
    /// The statement's name.
    pub statement_id: String,
    /// The tool name expression (first arg).
    pub tool: Option<Ast>,
    /// The tool arguments (second arg).
    pub args: Option<Ast>,
}

/// What parsing returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParseResult {
    /// The element tree, from the entry statement; `None` when it isn't a component.
    pub root: Option<Element>,
    /// The entry's value when it's an expression that picks a component at
    /// runtime, such as `root = $tab ? A : B`; `root` is then `None`.
    pub root_expression: Option<Arc<Ast>>,
    /// Facts about the parse as a whole.
    pub meta: Meta,
    /// `$name` → initial value, including `$name`s used but never declared (null).
    pub state_declarations: JsObject<Value>,
    /// `Query(...)` statements, in program order.
    pub query_statements: Vec<QueryStatement>,
    /// `Mutation(...)` statements, in program order.
    pub mutation_statements: Vec<MutationStatement>,
    /// The program nested deeper than the depth limit ([`crate::Limits::depth`])
    /// and the too-deep parts were dropped. (The reference throws on such
    /// input.) Not part of the JSON.
    pub depth_limited: bool,
    /// The result was cut, and what didn't fit was dropped: the program was
    /// longer than the program-size limit and the rest was ignored
    /// ([`crate::Limits::program_size`]); or the result would have grown past
    /// the result-size limit ([`crate::Limits::result_size`]: statements
    /// inlined into each other until it doubles at every level, say); or
    /// evaluating state defaults ran out of work ([`crate::Limits::work_for`])
    /// and a default came out null; or, in a release build, a loop reading
    /// the input stalled (a bug in the core) and the parse ended there. Not
    /// part of the JSON.
    pub size_limited: bool,
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
        crate::stack::grow_stack!(self.to_json());
        let mut out = String::with_capacity(256);
        out.push_str("{\"root\":");
        match &self.root {
            Some(root) => write_element(&mut out, root),
            None => out.push_str("null"),
        }
        if let Some(expression) = &self.root_expression {
            out.push_str(",\"rootExpression\":");
            write_ast(&mut out, expression);
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
        write_object(&mut out, &self.state_declarations);
        out.push_str(",\"queryStatements\":[");
        for (i, q) in self.query_statements.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"statementId\":");
            write_json_string(&mut out, &q.statement_id);
            for (key, node) in [
                ("toolAST", &q.tool),
                ("argsAST", &q.args),
                ("defaultsAST", &q.defaults),
                ("refreshAST", &q.refresh),
            ] {
                out.push_str(",\"");
                out.push_str(key);
                out.push_str("\":");
                write_optional_ast(&mut out, node);
            }
            if let Some(deps) = &q.deps {
                out.push_str(",\"deps\":");
                write_strings(&mut out, deps);
            }
            if let Some(wait_for) = &q.wait_for {
                out.push_str(",\"waitFor\":");
                write_strings(&mut out, wait_for);
            }
            out.push_str(",\"complete\":");
            write_bool(&mut out, q.complete);
            out.push('}');
        }
        out.push_str("],\"mutationStatements\":[");
        for (i, m) in self.mutation_statements.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"statementId\":");
            write_json_string(&mut out, &m.statement_id);
            out.push_str(",\"toolAST\":");
            write_optional_ast(&mut out, &m.tool);
            out.push_str(",\"argsAST\":");
            write_optional_ast(&mut out, &m.args);
            out.push('}');
        }
        out.push_str("]}");
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

fn write_optional_ast(out: &mut String, node: &Option<Ast>) {
    match node {
        Some(node) => write_ast(out, node),
        None => out.push_str("null"),
    }
}

fn write_object(out: &mut String, obj: &JsObject<Value>) {
    out.push('{');
    for (i, (key, value)) in obj.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_string(out, key);
        out.push(':');
        write_value(out, value);
    }
    out.push('}');
}

pub(crate) fn write_value(out: &mut String, value: &Value) {
    crate::stack::grow_stack!(write_value(out, value));
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => write_bool(out, *b),
        Value::Num(n) => write_json_number(out, *n),
        Value::Str(s) => write_json_string(out, s),
        Value::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Obj(obj) => write_object(out, obj),
        Value::Element(e) => write_element(out, e),
        Value::Ast(node) => write_ast(out, node),
    }
}

fn write_element(out: &mut String, e: &Element) {
    out.push_str("{\"type\":\"element\",\"typeName\":");
    write_json_string(out, &e.type_name);
    out.push_str(",\"props\":");
    write_object(out, &e.props);
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

/// The value as reading its JSON back gives it (`JSON.parse(JSON.stringify(
/// value))`), without writing or reading any text: an element or an AST node
/// is the plain object its JSON is, and a number that isn't finite is null,
/// `-0` is `0`. Everything else is as it was.
pub(crate) fn json_shaped(value: &Value) -> Value {
    crate::stack::grow_stack!(json_shaped(value));
    match value {
        Value::Num(n) => json_number_value(*n),
        Value::Arr(items) => Value::Arr(Arc::new(items.iter().map(json_shaped).collect())),
        Value::Obj(obj) => Value::Obj(Arc::new(json_shaped_object(obj))),
        Value::Element(e) => {
            let mut out = JsObject::new();
            out.define("type", Value::Str("element".to_owned()));
            out.define("typeName", Value::Str(e.type_name.clone()));
            out.define("props", Value::Obj(Arc::new(json_shaped_object(&e.props))));
            out.define("partial", Value::Bool(e.partial));
            out.define("hasDynamicProps", Value::Bool(e.has_dynamic_props));
            if let Some(id) = &e.statement_id {
                out.define("statementId", Value::Str(id.clone()));
            }
            Value::Obj(Arc::new(out))
        }
        Value::Ast(node) => crate::ast::json_shaped_ast(node),
        Value::Null | Value::Bool(_) | Value::Str(_) => value.clone(),
    }
}

fn json_shaped_object(obj: &JsObject<Value>) -> JsObject<Value> {
    let mut out = JsObject::new();
    for (key, v) in obj.iter() {
        out.define(key.clone(), json_shaped(v));
    }
    out
}

/// A number as its JSON reads back: null if it isn't finite, and `-0` as `0`.
pub(crate) fn json_number_value(n: f64) -> Value {
    if !n.is_finite() {
        Value::Null
    } else if n == 0.0 {
        Value::Num(0.0)
    } else {
        Value::Num(n)
    }
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
    if let Some(severity) = e.severity {
        out.push_str(",\"severity\":");
        write_json_string(out, severity.as_str());
    }
    out.push('}');
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Value, json_shaped, write_value};
    use crate::{Library, parse};

    /// Every value in the result's root and state, nested ones included.
    fn values(value: &Value, out: &mut Vec<Value>) {
        out.push(value.clone());
        match value {
            Value::Arr(items) => items.iter().for_each(|v| values(v, out)),
            Value::Obj(obj) => obj.values().for_each(|v| values(v, out)),
            Value::Element(e) => e.props.values().for_each(|v| values(v, out)),
            _ => {}
        }
    }

    #[test]
    fn a_value_shaped_directly_is_what_its_json_reads_back_as() {
        let schema = serde_json::json!({"$defs": {
            "Text": {"properties": {"value": {}}, "required": ["value"]},
            "Stack": {"properties": {"children": {"type": "array"}}}
        }});
        let library = Library::from_json_schema(&schema);
        let programs = [
            // Elements, nested, with props whose keys are array indexes.
            "root = Stack([t, Text({\"2\": 1, \"a\": -0, \"1\": [1e21, 1e-7]})])\nt = Text(\"x\")",
            // Runtime expressions of every kind, in props and in state.
            "root = Stack([Text($a ? -1 : !$b), Text($s.n[0] + $q.data), Text(@Count([1]))])\n\
             $a = {k: \"Num\", \"0\": 1}\n$b = $a == null || $a != 1\n\
             $s = Text(\"y\")\n$q = 1\nq = Query(\"tool\", {a: $s}, {})\n\
             $e = @Each([1, 2], \"x\", {v: x, w: [x, \"s\"]})\n$f = @Fn(1, $a)\n\
             $g = $h = 2",
            // Unicode, escapes and lone surrogates.
            "root = Text(\"\\u00e9 \\ud800 \\\" \\n\")",
        ];
        for program in programs {
            let result = parse(program, &library, None);
            let mut all = Vec::new();
            if let Some(root) = &result.root {
                values(&Value::Element(Arc::new(root.clone())), &mut all);
            }
            result
                .state_declarations
                .values()
                .for_each(|v| values(v, &mut all));
            if let Some(expression) = &result.root_expression {
                values(&Value::Ast(Arc::clone(expression)), &mut all);
            }
            assert!(all.iter().any(|v| matches!(v, Value::Element(_))));
            if program.contains('$') {
                assert!(all.iter().any(|v| matches!(v, Value::Ast(_))));
            }
            for value in &all {
                let mut json = String::new();
                write_value(&mut json, value);
                let read = crate::parse_json(&json).unwrap();
                assert_eq!(json_shaped(value), Value::from_json(&read), "{json}");
            }
        }
    }
}
