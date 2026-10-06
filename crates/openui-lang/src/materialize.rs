//! Lowering expression trees to elements and plain values: resolving
//! references, mapping positional args to named props, and reporting invalid
//! props, as the reference parser's `materializeValue`.

use std::collections::HashSet;

use indexmap::{IndexMap, IndexSet};

use crate::ast::Ast;
use crate::library::{Library, Param};
use crate::parser::Statement;
use crate::result::{Element, ValidationError, Value};
use crate::validation::{Issue, params_signature};

pub(crate) struct Ctx<'a> {
    pub statements: &'a IndexMap<String, Statement>,
    pub library: &'a Library,
    pub errors: Vec<ValidationError>,
    pub unresolved: Vec<String>,
    /// Statements being materialized right now, to break reference cycles.
    pub visited: HashSet<String>,
    pub partial: bool,
    /// The statement errors are attributed to.
    pub current_statement_id: Option<String>,
    /// Value statements not reached yet; what's left at the end is orphaned.
    pub unreached: IndexSet<String>,
}

impl Ctx<'_> {
    fn push_issue(&mut self, component: &str, path: &str, issue: Issue) {
        self.errors.push(ValidationError {
            code: issue.code(),
            component: component.to_owned(),
            path: path.to_owned(),
            message: issue.message(component, path),
            statement_id: self.current_statement_id.clone(),
        });
    }
}

pub(crate) fn materialize_value(node: &Ast, ctx: &mut Ctx) -> Value {
    match node {
        Ast::Ref(name) => resolve_ref(name, ctx),
        Ast::Str(v) => Value::Str(v.clone()),
        Ast::Null | Ast::Ph(_) => Value::Null,
        Ast::Comp { name, args } => materialize_comp(name, args, ctx),
    }
}

/// Inline the referenced statement, tagging an element with the statement's id.
fn resolve_ref(name: &str, ctx: &mut Ctx) -> Value {
    let statements = ctx.statements;
    let target = match statements.get(name) {
        Some(stmt) if !ctx.visited.contains(name) => stmt.expr(),
        _ => {
            ctx.unresolved.push(name.to_owned());
            return Value::Null;
        }
    };
    ctx.unreached.shift_remove(name);
    ctx.visited.insert(name.to_owned());
    let previous = ctx.current_statement_id.replace(name.to_owned());
    let mut result = materialize_value(target, ctx);
    if let Value::Element(element) = &mut result {
        element.statement_id = Some(name.to_owned());
    }
    ctx.current_statement_id = previous;
    ctx.visited.remove(name);
    result
}

fn materialize_comp(name: &str, args: &[Ast], ctx: &mut Ctx) -> Value {
    let library = ctx.library;
    let Some(def) = library.component(name) else {
        let available = library.component_names().map(str::to_owned).collect();
        ctx.push_issue(name, "", Issue::UnknownComponent { available });
        return Value::Null;
    };

    let mut props = IndexMap::new();
    for (param, arg) in def.params.iter().zip(args) {
        let value = materialize_value(arg, ctx);
        props.insert(param.name.clone(), value);
    }

    // Schema defaults don't fill missing required props yet: that needs a
    // Value for every JSON type.
    let missing: Vec<&Param> = def
        .params
        .iter()
        .filter(|p| p.required && matches!(props.get(&p.name), None | Some(Value::Null)))
        .collect();
    if !missing.is_empty() {
        let signature = params_signature(name, &def.params);
        for p in missing {
            let signature = signature.clone();
            let issue = if props.contains_key(&p.name) {
                Issue::NullRequired { signature }
            } else {
                Issue::MissingRequired { signature }
            };
            ctx.push_issue(name, &format!("/{}", p.name), issue);
        }
        return Value::Null;
    }

    let has_dynamic_props = props.values().any(contains_dynamic_value);
    Value::Element(Box::new(Element {
        type_name: name.to_owned(),
        props,
        partial: ctx.partial,
        has_dynamic_props,
        statement_id: None,
    }))
}

/// Whether a prop value holds anything that needs runtime evaluation.
fn contains_dynamic_value(value: &Value) -> bool {
    match value {
        Value::Element(e) => e.props.values().any(contains_dynamic_value),
        Value::Null | Value::Str(_) => false,
    }
}
