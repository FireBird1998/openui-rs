//! One-shot parsing, and the result building shared with streaming, as the
//! reference parser's `parse` and `buildResult`.

use std::collections::HashSet;

use indexmap::{IndexMap, IndexSet};

use crate::ast::Ast;
use crate::builtins::is_builtin;
use crate::expr::parse_expression;
use crate::js::JsObject;
use crate::lexer::tokenize;
use crate::library::Library;
use crate::materialize::{Ctx, materialize_value, tag_statement};
use crate::preprocess::preprocess;
use crate::result::{Meta, MutationStatement, ParseResult, QueryStatement, Value};
use crate::statements::{IdKind, RawStatement, auto_close, split};

const DEFAULT_ROOT_STATEMENT_ID: &str = "root";

/// A parsed statement, classified by what it declares.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Statement {
    Value {
        expr: Ast,
    },
    /// `$name = init`.
    State {
        init: Ast,
    },
    /// `name = Query(...)`, with the state variables its args read.
    Query {
        expr: Ast,
        deps: Option<Vec<String>>,
    },
    /// `name = Mutation(...)`.
    Mutation {
        expr: Ast,
    },
}

impl Statement {
    /// What a reference to this statement inlines.
    pub(crate) fn expr(&self) -> &Ast {
        match self {
            Statement::Value { expr }
            | Statement::Query { expr, .. }
            | Statement::Mutation { expr } => expr,
            Statement::State { init } => init,
        }
    }

    /// A Query's or Mutation's call args.
    fn call_args(&self) -> &[Ast] {
        match self {
            Statement::Query {
                expr: Ast::Comp { args, .. },
                ..
            }
            | Statement::Mutation {
                expr: Ast::Comp { args, .. },
            } => args,
            _ => &[],
        }
    }
}

/// Classify by expression shape, then by the id's token: Query and Mutation
/// calls first (so `$x = Query(...)` is a query), then `$` ids as state.
pub(crate) fn classify(raw: &RawStatement, expr: Ast) -> Statement {
    if let Ast::Comp { name, args, .. } = &expr {
        if name == "Query" {
            let deps = collect_state_refs(args.get(1));
            let deps = (!deps.is_empty()).then_some(deps);
            return Statement::Query { expr, deps };
        }
        if name == "Mutation" {
            return Statement::Mutation { expr };
        }
    }
    if raw.id_kind == IdKind::StateVar {
        return Statement::State { init: expr };
    }
    Statement::Value { expr }
}

/// The distinct `$name`s a node reads, in walk order.
fn collect_state_refs(node: Option<&Ast>) -> Vec<String> {
    let mut refs: IndexSet<String> = IndexSet::new();
    if let Some(node) = node {
        node.walk(&mut |n| {
            if let Ast::StateRef(name) = n {
                refs.insert(name.clone());
            }
        });
    }
    refs.into_iter().collect()
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

/// A value statement that calls a component. The reference compares against
/// `Query` and `Mutation` by name here, not with `isReservedCall`, so
/// `r = @toString()` does count.
fn is_component_statement(stmt: &Statement) -> Option<&str> {
    match stmt {
        Statement::Value {
            expr: Ast::Comp { name, .. },
        } if !is_builtin(name) && name != "Query" && name != "Mutation" => Some(name),
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

    // Orphan tracking covers value statements; state, queries and mutations
    // are consumed separately.
    let unreached: IndexSet<String> = statements
        .iter()
        .filter(|(id, stmt)| *id != entry_id && matches!(stmt, Statement::Value { .. }))
        .map(|(id, _)| id.clone())
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
    let mut materialized = materialize_value(entry.expr(), &mut ctx);
    tag_statement(&mut materialized, entry_id);
    let root = match materialized {
        Value::Element(element) => Some(*element),
        _ => None,
    };

    let (state_declarations, query_statements, mutation_statements) =
        extract_statements(statements, &mut ctx);

    ParseResult {
        root,
        meta: Meta {
            incomplete: was_incomplete,
            unresolved: ctx.unresolved,
            orphaned: ctx.unreached.into_iter().collect(),
            statement_count,
            errors: ctx.errors,
        },
        state_declarations,
        query_statements,
        mutation_statements,
    }
}

/// State initial values (materialized), plus queries and mutations as written.
/// Any `$name` used anywhere but never declared is declared as null.
fn extract_statements(
    statements: &IndexMap<String, Statement>,
    ctx: &mut Ctx,
) -> (JsObject<Value>, Vec<QueryStatement>, Vec<MutationStatement>) {
    let mut state = JsObject::new();
    let mut queries = Vec::new();
    let mut mutations = Vec::new();
    for (id, stmt) in statements {
        match stmt {
            Statement::State { init } => {
                let value = materialize_value(init, ctx);
                state.set(id.clone(), value);
            }
            Statement::Query { deps, .. } => {
                let args = stmt.call_args();
                queries.push(QueryStatement {
                    statement_id: id.clone(),
                    tool: args.first().cloned(),
                    args: args.get(1).cloned(),
                    defaults: args.get(2).cloned(),
                    refresh: args.get(3).cloned(),
                    deps: deps.clone(),
                    complete: true,
                });
            }
            Statement::Mutation { .. } => {
                let args = stmt.call_args();
                mutations.push(MutationStatement {
                    statement_id: id.clone(),
                    tool: args.first().cloned(),
                    args: args.get(1).cloned(),
                });
            }
            Statement::Value { .. } => {}
        }
    }
    for stmt in statements.values() {
        let nodes: &[Ast] = match stmt {
            Statement::State { init } => std::slice::from_ref(init),
            Statement::Value { expr } => std::slice::from_ref(expr),
            Statement::Query { .. } | Statement::Mutation { .. } => stmt.call_args(),
        };
        for node in nodes {
            for name in collect_state_refs(Some(node)) {
                if !state.has(&name) {
                    state.set(name, Value::Null);
                }
            }
        }
    }
    (state, queries, mutations)
}
