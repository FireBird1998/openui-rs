//! One-shot parsing, and the result building shared with streaming, as the
//! reference parser's `parse` and `buildResult`.

use std::sync::Arc;

use indexmap::IndexSet;

use crate::ast::Ast;
use crate::builtins::is_builtin;
use crate::expr::parse_expression_with_limit;
use crate::js::JsObject;
use crate::lexer::tokenize;
use crate::library::Library;
use crate::materialize::{Ctx, Frame, materialize_value, tag_statement};
use crate::preprocess::preprocess;
use crate::result::{Meta, MutationStatement, ParseResult, QueryStatement, Value};
use crate::statements::{IdKind, RawStatement, auto_close, split};
use crate::table::{Cache, Reach, Slot, Table};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Value,
    State,
    Query,
    Mutation,
}

/// Facts about a statement that building a result needs every time, derived
/// once when the statement is defined.
#[derive(Clone, Debug)]
pub(crate) struct Info {
    pub kind: Kind,
    /// The library component a value statement calls, if that's what it is.
    pub component: Option<String>,
    /// The distinct `$name`s the statement reads, in walk order.
    pub state_refs: Vec<String>,
    /// The slots its expression references, sorted; filled in by the table.
    pub refs: Vec<Slot>,
}

impl Info {
    /// Whether every result has to look at the statement, beyond reaching it
    /// from the entry: state, queries and mutations are listed, and `$name`s
    /// read anywhere are declared.
    pub(crate) fn special(&self) -> bool {
        self.kind != Kind::Value || !self.state_refs.is_empty()
    }

    pub(crate) fn of(stmt: &Statement) -> Info {
        let kind = match stmt {
            Statement::Value { .. } => Kind::Value,
            Statement::State { .. } => Kind::State,
            Statement::Query { .. } => Kind::Query,
            Statement::Mutation { .. } => Kind::Mutation,
        };
        let nodes: &[Ast] = match stmt {
            Statement::State { init } => std::slice::from_ref(init),
            Statement::Value { expr } => std::slice::from_ref(expr),
            Statement::Query { .. } | Statement::Mutation { .. } => stmt.call_args(),
        };
        let mut refs: IndexSet<String> = IndexSet::new();
        for node in nodes {
            node.walk(&mut |n| {
                if let Ast::StateRef(name) = n {
                    refs.insert(name.clone());
                }
            });
        }
        Info {
            kind,
            component: is_component_statement(stmt).map(str::to_owned),
            state_refs: refs.into_iter().collect(),
            refs: Vec::new(),
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
    let mut table = Table::default();
    let mut depth_limited = false;
    for stmt in &raw {
        let (expr, limited) = parse_expression_with_limit(&stmt.tokens);
        depth_limited |= limited;
        table.define(&stmt.id, classify(stmt, expr));
    }
    let count = table.len();
    let mut cache = Cache::default();
    let mut result = build_result(
        &mut table,
        &mut cache,
        &raw[0].id,
        was_incomplete,
        count,
        library,
        root_name,
    );
    result.depth_limited |= depth_limited;
    result
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
fn pick_entry_id<'a>(table: &'a Table, first_id: &'a str, root_name: Option<&'a str>) -> &'a str {
    if table.by_name(DEFAULT_ROOT_STATEMENT_ID).is_some() {
        return DEFAULT_ROOT_STATEMENT_ID;
    }
    if let Some(root) = root_name {
        if table.by_name(root).is_some() {
            return root;
        }
        let preferred = table
            .order()
            .iter()
            .find(|&&slot| table.info(slot).component.as_deref() == Some(root));
        if let Some(&slot) = preferred {
            return table.name(slot);
        }
    }
    table
        .order()
        .iter()
        .find(|&&slot| table.info(slot).component.is_some())
        .map_or(first_id, |&slot| table.name(slot))
}

pub(crate) fn build_result(
    table: &mut Table,
    cache: &mut Cache,
    first_id: &str,
    was_incomplete: bool,
    statement_count: usize,
    library: &Library,
    root_name: Option<&str>,
) -> ParseResult {
    let entry_id = pick_entry_id(table, first_id, root_name);
    let Some(entry_slot) = table.slot(entry_id) else {
        return ParseResult::empty(was_incomplete);
    };
    let Some(entry) = table.get(entry_slot) else {
        return ParseResult::empty(was_incomplete);
    };

    let mut ctx = Ctx {
        table,
        cache,
        library,
        partial: was_incomplete,
        visited: Vec::new(),
        current_statement_id: Some(entry_id.to_owned()),
        depth: 0,
        size: 0,
        frame: Frame::default(),
    };
    let mut materialized = materialize_value(entry.expr(), &mut ctx);
    let _ = tag_statement(&mut materialized, entry_id);
    let root = match materialized {
        Value::Element(element) => Some(Arc::unwrap_or_clone(element)),
        _ => None,
    };

    let (state_declarations, query_statements, mutation_statements) =
        extract_statements(table, &mut ctx);
    let Ctx { frame, .. } = ctx;
    let reach = Arc::new(Reach {
        direct: frame.reached,
        children: frame.children,
    });
    let orphaned = table.finish_result(entry_slot, reach);

    ParseResult {
        root,
        meta: Meta {
            incomplete: was_incomplete,
            unresolved: frame.unresolved,
            orphaned,
            statement_count,
            errors: frame.errors,
        },
        state_declarations,
        query_statements,
        mutation_statements,
        depth_limited: frame.limited,
        size_limited: frame.size_limited,
    }
}

/// State initial values (materialized), plus queries and mutations as written.
/// Any `$name` used anywhere but never declared is declared as null.
fn extract_statements(
    table: &Table,
    ctx: &mut Ctx,
) -> (JsObject<Value>, Vec<QueryStatement>, Vec<MutationStatement>) {
    let mut state = JsObject::new();
    let mut queries = Vec::new();
    let mut mutations = Vec::new();
    for &slot in table.specials() {
        let id = table.name(slot);
        let stmt = table.get(slot).expect("defined statement");
        match stmt {
            Statement::State { init } => {
                let value = materialize_value(init, ctx);
                state.set(id.to_owned(), value);
            }
            Statement::Query { deps, .. } => {
                let args = stmt.call_args();
                queries.push(QueryStatement {
                    statement_id: id.to_owned(),
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
                    statement_id: id.to_owned(),
                    tool: args.first().cloned(),
                    args: args.get(1).cloned(),
                });
            }
            Statement::Value { .. } => {}
        }
    }
    for &slot in table.specials() {
        for name in &table.info(slot).state_refs {
            if !state.has(name) {
                state.set(name.clone(), Value::Null);
            }
        }
    }
    (state, queries, mutations)
}
