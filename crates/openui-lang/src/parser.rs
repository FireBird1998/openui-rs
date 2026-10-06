//! One-shot parsing, and the result building shared with streaming, as the
//! reference parser's `parse` and `buildResult`.

use std::collections::HashSet;

use indexmap::{IndexMap, IndexSet};

use crate::ast::Ast;
use crate::builtins::{is_builtin, is_reserved_call};
use crate::expr::parse_expression;
use crate::js;
use crate::lexer::tokenize;
use crate::library::Library;
use crate::materialize::{Ctx, materialize_value};
use crate::result::{Meta, ParseResult, Value};
use crate::statements::{RawStatement, auto_close, split};

const DEFAULT_ROOT_STATEMENT_ID: &str = "root";

/// A parsed statement, classified by what it declares.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Statement {
    Value { expr: Ast },
}

impl Statement {
    pub(crate) fn expr(&self) -> &Ast {
        match self {
            Statement::Value { expr } => expr,
        }
    }
}

pub(crate) fn classify(_raw: &RawStatement, expr: Ast) -> Statement {
    Statement::Value { expr }
}

/// Clean up model output before parsing. Markdown fences and comments aren't
/// stripped yet, only surrounding whitespace.
pub(crate) fn preprocess(input: &str) -> String {
    js::trim(input).to_owned()
}

/// Parse a complete (or partial) program in one pass.
pub fn parse(input: &str, library: &Library, root_name: Option<&str>) -> ParseResult {
    let trimmed = preprocess(input);
    if trimmed.is_empty() {
        return ParseResult::empty(true);
    }
    let (text, was_incomplete) = auto_close(&trimmed);
    let raw = split(&tokenize(&text));
    if raw.is_empty() {
        return ParseResult::empty(was_incomplete);
    }

    // Re-defining an id replaces its statement but keeps its original position.
    let mut statements = IndexMap::new();
    for stmt in &raw {
        let expr = parse_expression(&stmt.tokens);
        statements.insert(stmt.id.clone(), classify(stmt, expr));
    }
    let count = statements.len();
    build_result(
        &statements,
        &raw[0].id,
        was_incomplete,
        count,
        library,
        root_name,
    )
}

fn is_component_statement(stmt: &Statement) -> Option<&str> {
    match stmt {
        Statement::Value {
            expr: Ast::Comp { name, .. },
        } if !is_builtin(name) && !is_reserved_call(name) => Some(name),
        _ => None,
    }
}

/// The statement the tree grows from: `root`, then the requested root name,
/// then the first component statement, then the first statement.
fn pick_entry_id<'a>(
    statements: &'a IndexMap<String, Statement>,
    first_id: &'a str,
    root_name: Option<&'a str>,
) -> &'a str {
    if statements.contains_key(DEFAULT_ROOT_STATEMENT_ID) {
        return DEFAULT_ROOT_STATEMENT_ID;
    }
    if let Some(root) = root_name {
        if statements.contains_key(root) {
            return root;
        }
        let preferred = statements
            .iter()
            .find(|(_, stmt)| is_component_statement(stmt) == Some(root));
        if let Some((id, _)) = preferred {
            return id;
        }
    }
    statements
        .iter()
        .find(|(_, stmt)| is_component_statement(stmt).is_some())
        .map_or(first_id, |(id, _)| id)
}

pub(crate) fn build_result(
    statements: &IndexMap<String, Statement>,
    first_id: &str,
    was_incomplete: bool,
    statement_count: usize,
    library: &Library,
    root_name: Option<&str>,
) -> ParseResult {
    let entry_id = pick_entry_id(statements, first_id, root_name);
    let Some(entry) = statements.get(entry_id) else {
        return ParseResult::empty(was_incomplete);
    };

    let unreached: IndexSet<String> = statements
        .keys()
        .filter(|id| *id != entry_id)
        .cloned()
        .collect();
    let mut ctx = Ctx {
        statements,
        library,
        errors: Vec::new(),
        unresolved: Vec::new(),
        visited: HashSet::new(),
        partial: was_incomplete,
        current_statement_id: Some(entry_id.to_owned()),
        unreached,
    };
    let root = match materialize_value(entry.expr(), &mut ctx) {
        Value::Element(mut element) => {
            element.statement_id = Some(entry_id.to_owned());
            Some(*element)
        }
        _ => None,
    };

    ParseResult {
        root,
        meta: Meta {
            incomplete: was_incomplete,
            unresolved: ctx.unresolved,
            orphaned: ctx.unreached.into_iter().collect(),
            statement_count,
            errors: ctx.errors,
        },
        state_declarations: IndexMap::new(),
    }
}
