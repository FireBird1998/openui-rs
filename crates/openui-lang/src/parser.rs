//! One-shot parsing, and the result building shared with streaming, as the
//! reference parser's `parse` and `buildResult`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexSet;

use crate::Limits;
use crate::ast::{Ast, RefType};
use crate::builtins::{is_builtin, is_reserved_call};
use crate::evaluate::{Reuse, has_runtime, state_defaults};
use crate::expr::parse_expression_with_limit;
use crate::js::{JsObject, json_string_len};
use crate::lexer::tokenize;
use crate::library::Library;
use crate::materialize::{
    Ctx, Frame, flatten_errors, flatten_unresolved, materialize_call_args, materialize_state,
    materialize_value, tag_statement,
};
use crate::preprocess::preprocess;
use crate::progress;
use crate::result::{Meta, MutationStatement, ParseResult, QueryStatement, Value};
use crate::statements::{IdKind, RawStatement, auto_close, split};
use crate::table::{Cache, Reach, Slot, Table};
use crate::validation::{Issue, NoRootReason, query_cycle_message_len};

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
    /// `name = Query(...)`.
    Query {
        expr: Ast,
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
    if let Ast::Comp { name, .. } = &expr {
        if name == "Query" {
            return Statement::Query { expr };
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

/// The queries whose results these expressions read, in the order first met.
fn query_reads(nodes: &[Ast]) -> Vec<String> {
    let mut reads: IndexSet<String> = IndexSet::new();
    for node in nodes {
        node.walk(&mut |n| {
            if let Ast::RuntimeRef {
                n,
                ref_type: RefType::Query,
            } = n
            {
                reads.insert(n.clone());
            }
        });
    }
    reads.into_iter().collect()
}

/// A loop of queries reading each other's results, as the search for them
/// found it: how many queries and how long their names are in all, known
/// without gathering them.
struct QueryLoop<'a> {
    queries: &'a [QueryStatement],
    /// The queries in the loop: the end of the search's path.
    path: &'a [(usize, usize)],
    /// Their names' bytes in all.
    bytes: usize,
}

impl QueryLoop<'_> {
    /// The queries' names, in statement order.
    fn names(&self) -> Vec<String> {
        let mut members: Vec<usize> = self.path.iter().map(|&(query, _)| query).collect();
        members.sort_unstable();
        members
            .into_iter()
            .map(|query| self.queries[query].statement_id.clone())
            .collect()
    }
}

/// Loops of queries reading each other's results (one reading itself is
/// one), given to `found` in the order a depth-first walk over the queries in
/// statement order meets them.
///
/// The walk takes a step per query and per read. A long chain of queries
/// can hold as many loops as it has queries, each most of the chain long, so
/// gathering a loop's names is left to `found`, which knows whether there's
/// room to report it.
fn query_cycles(queries: &[QueryStatement], mut found: impl FnMut(QueryLoop)) {
    let order: HashMap<&str, usize> = queries
        .iter()
        .enumerate()
        .map(|(i, q)| (q.statement_id.as_str(), i))
        .collect();
    let reads =
        |query: usize| -> &[String] { queries[query].wait_for.as_deref().unwrap_or_default() };
    let name_len = |query: usize| queries[query].statement_id.len();
    let mut done = vec![false; queries.len()];
    // Where on the walk's path each query is, while it is.
    let mut on_path: Vec<Option<usize>> = vec![None; queries.len()];
    // The walk's path, each with how many of its reads it has followed:
    // the reference recurses, and this keeps its order without the stack.
    let mut path: Vec<(usize, usize)> = Vec::new();
    // The bytes of the names before each query on the path, and of them all.
    let mut bytes_before: Vec<usize> = Vec::new();
    let mut bytes = 0;
    for start in 0..queries.len() {
        if done[start] {
            continue;
        }
        on_path[start] = Some(0);
        path.push((start, 0));
        bytes_before.push(0);
        bytes += name_len(start);
        while let Some(&mut (query, ref mut next)) = path.last_mut() {
            let Some(dep) = reads(query).get(*next) else {
                path.pop();
                bytes_before.pop();
                bytes -= name_len(query);
                on_path[query] = None;
                done[query] = true;
                continue;
            };
            *next += 1;
            let Some(&dep) = order.get(dep.as_str()) else {
                continue;
            };
            if let Some(at) = on_path[dep] {
                found(QueryLoop {
                    queries,
                    path: &path[at..],
                    bytes: bytes - bytes_before[at],
                });
            } else if !done[dep] {
                on_path[dep] = Some(path.len());
                path.push((dep, 0));
                bytes_before.push(bytes);
                bytes += name_len(dep);
            }
        }
    }
}

/// Parse a complete (or partial) program in one pass. `root_name` is the
/// library's root component: with no `root` statement, the first statement
/// that calls it is the entry.
pub fn parse(input: &str, library: &Library, root_name: Option<&str>) -> ParseResult {
    parse_with_limits(input, library, root_name, Limits::DEFAULT)
}

/// [`parse`] within `limits` instead of the defaults.
pub fn parse_with_limits(
    input: &str,
    library: &Library,
    root_name: Option<&str>,
    limits: Limits,
) -> ParseResult {
    crate::stack::grow_stack!(parse_with_limits(input, library, root_name, limits));
    // Past the program-size limit, the rest is ignored as if the stream had
    // stopped there.
    let (input, cut) = Limits::fit(input, limits.program_size());
    // The result-size and work limits scale with the program read.
    let limits = limits.for_program(input.len());
    let (mut result, stalled) = progress::watch(|| parse_text(input, library, root_name, limits));
    result.size_limited |= stalled || cut;
    result
}

fn parse_text(
    input: &str,
    library: &Library,
    root_name: Option<&str>,
    limits: Limits,
) -> ParseResult {
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
        if progress::stalled_so_far() {
            break;
        }
        let (expr, limited) = parse_expression_with_limit(&stmt.tokens, limits.depth());
        depth_limited |= limited;
        table.define(&stmt.id, classify(stmt, expr));
    }
    let count = table.len();
    let mut cache = Cache::default();
    let mut result = build_result(
        &mut table,
        &mut cache,
        was_incomplete,
        count,
        Against {
            library,
            root_name,
            limits,
        },
        None,
        None,
    );
    result.depth_limited |= depth_limited;
    result
}

/// A call to a library component: not an `@` function, a builtin, nor a reserved call. The
/// reference tells reserved calls apart with `name in { Query, Mutation }`, so
/// names every object inherits (`toString`, `constructor`, ...) don't count
/// either.
fn is_component_call(node: &Ast) -> Option<&str> {
    match node {
        Ast::Comp {
            name,
            function: false,
            ..
        } if !is_builtin(name) && !is_reserved_call(name) => Some(name),
        _ => None,
    }
}

/// A value statement that calls a component, and which one.
fn is_component_statement(stmt: &Statement) -> Option<&str> {
    match stmt {
        Statement::Value { expr } => is_component_call(expr),
        _ => None,
    }
}

/// The statement the tree grows from: `root`; else the first statement whose
/// value calls the library's root component, standing in for it (the flag);
/// else none.
fn pick_entry(table: &Table, root_name: Option<&str>) -> Option<(Slot, bool)> {
    if let Some(slot) = table
        .slot(DEFAULT_ROOT_STATEMENT_ID)
        .filter(|&slot| table.get(slot).is_some())
    {
        return Some((slot, false));
    }
    let root = root_name?;
    table
        .order()
        .iter()
        .find(|&&slot| table.info(slot).component.as_deref() == Some(root))
        .map(|&slot| (slot, true))
}

/// Whether the entry's expression is a component call once references to
/// other statements are followed. A call that validation dropped still counts.
fn resolves_to_component_call<'a>(table: &'a Table, mut node: &'a Ast) -> bool {
    let mut seen = HashSet::new();
    while let Ast::Ref(name) = node {
        if !seen.insert(name.as_str()) {
            break;
        }
        match table.by_name(name) {
            Some(stmt) => node = stmt.expr(),
            None => return false,
        }
    }
    is_component_call(node).is_some()
}

/// Whether a runtime expression can produce a component, as `$tab ? A : B` does.
fn contains_component(node: &Ast) -> bool {
    let mut found = false;
    node.walk(&mut |n| found |= is_component_call(n).is_some());
    found
}

/// A `$name` at the very end of a stream's text so far, which may still be
/// growing, and the pending statements it's in: those don't declare it.
pub(crate) struct PartialRef<'a> {
    pub name: &'a str,
    pub slots: &'a [Slot],
}

/// What a parse is against: the component library, its root component, and
/// the limits, those that scale with the program fixed for the text so far
/// (`Limits::for_program`), the same for a stream's push as for a one-shot
/// parse of it.
#[derive(Clone, Copy)]
pub(crate) struct Against<'a> {
    pub library: &'a Library,
    pub root_name: Option<&'a str>,
    pub limits: Limits,
}

/// A stream passes its [`Reuse`] of the state defaults' evaluation, which a
/// one-shot parse has no use for.
pub(crate) fn build_result(
    table: &mut Table,
    cache: &mut Cache,
    was_incomplete: bool,
    statement_count: usize,
    against: Against,
    partial: Option<PartialRef>,
    reuse: Option<&mut Reuse>,
) -> ParseResult {
    let Against {
        library,
        root_name,
        limits,
    } = against;
    let entry = pick_entry(table, root_name);
    let entry_id = entry.map(|(slot, _)| table.name(slot).to_owned());
    let mut ctx = Ctx {
        table,
        cache,
        library,
        partial: was_incomplete,
        visited: Vec::new(),
        current_statement_id: entry_id.clone(),
        depth: 0,
        size: 0,
        limits,
        frame: Frame::default(),
    };
    let mut root = None;
    let mut root_expression = None;
    // An object literal shaped like an element is the reference's root, but
    // can't be the core's (a documented divergence): it still counts as one.
    let mut element_shaped = false;
    if let Some((slot, _)) = entry {
        let stmt = ctx.table.get(slot).expect("defined statement");
        let mut materialized = materialize_value(stmt.expr(), &mut ctx);
        element_shaped = tag_statement(&mut materialized, ctx.table.name(slot));
        // A value frees its tree itself, so nothing moves out of it: the
        // element is shared out, and owned alone once the value is gone.
        match &materialized {
            Value::Element(element) => {
                let element = Arc::clone(element);
                drop(materialized);
                root = Some(Arc::unwrap_or_clone(element));
            }
            Value::Ast(expression) if contains_component(expression) => {
                root_expression = Some(Arc::clone(expression));
            }
            _ => {}
        }
    }

    // Fixed for this program by `Limits::for_program`, whatever the length.
    let work = limits.work_for(0);
    let (state_declarations, query_statements, mutation_statements) =
        extract_statements(ctx.table, &mut ctx, partial.as_ref(), work, reuse);

    // `no-root`, which streaming consumers read only once the stream ends.
    ctx.current_statement_id = entry_id;
    let rendered = element_shaped || root_expression.is_some();
    let reason = match entry {
        None => Some(NoRootReason::Missing),
        Some((slot, _))
            if !rendered
                && !resolves_to_component_call(
                    ctx.table,
                    ctx.table.get(slot).expect("defined statement").expr(),
                ) =>
        {
            Some(NoRootReason::NotComponent)
        }
        Some((_, true)) if element_shaped => Some(NoRootReason::Fallback),
        Some((_, true)) => Some(NoRootReason::Missing),
        Some(_) => None,
    };
    if let Some(reason) = reason {
        ctx.push_issue(root_name.unwrap_or(""), "", Issue::NoRoot { reason });
    }

    let Ctx { frame, .. } = ctx;
    // With no entry nothing is reachable, and `no-root` says so: no orphans.
    let orphaned = match entry {
        Some((slot, _)) => {
            let reach = Arc::new(Reach {
                direct: frame.reached,
                children: frame.children,
            });
            table.finish_result(slot, reach)
        }
        None => Vec::new(),
    };

    ParseResult {
        root,
        root_expression,
        meta: Meta {
            incomplete: was_incomplete,
            unresolved: flatten_unresolved(&frame.unresolved),
            orphaned,
            statement_count,
            errors: flatten_errors(&frame.errors),
        },
        state_declarations,
        query_statements,
        mutation_statements,
        depth_limited: frame.limited,
        size_limited: frame.size_limited,
    }
}

/// What a state declaration writes around its name and value: `"":,`.
const STATE_WRAPPER: usize = 4;

/// What a query statement writes around its name and its parts, each of the
/// four taken as the `null` a missing one is.
const QUERY_WRAPPER: usize =
    r#"{"statementId":"","toolAST":,"argsAST":,"defaultsAST":,"refreshAST":,"complete":false},"#
        .len()
        + 4 * "null".len();

/// What a mutation statement writes around its name and its parts, as
/// [`QUERY_WRAPPER`].
const MUTATION_WRAPPER: usize =
    r#"{"statementId":"","toolAST":,"argsAST":},"#.len() + 2 * "null".len();

/// What a query's list of the names it depends on or waits for writes, if it
/// has any: `,"waitFor":[]` and each name, quoted, with its comma.
fn names_size(names: &[String]) -> usize {
    if names.is_empty() {
        return 0;
    }
    r#","waitFor":[]"#.len() + names.iter().map(|n| json_string_len(n) + 3).sum::<usize>()
}

/// State initial values (materialized), plus queries and mutations as written.
/// Any `$name` used anywhere but never declared is declared as null, except
/// one still growing at the end of a stream, in the statements it ends.
fn extract_statements(
    table: &Table,
    ctx: &mut Ctx,
    partial: Option<&PartialRef>,
    work: usize,
    reuse: Option<&mut Reuse>,
) -> (JsObject<Value>, Vec<QueryStatement>, Vec<MutationStatement>) {
    let mut state = JsObject::new();
    let mut queries = Vec::new();
    let mut mutations = Vec::new();
    // What the expression defaults cost materialized: evaluated, they're
    // written in its place.
    let mut expressions_size = 0;
    // A statement didn't fit: nothing more is written.
    let mut full = false;
    for &slot in table.specials() {
        let id = table.name(slot);
        let stmt = table.get(slot).expect("defined statement");
        // What the statement writes around its values counts too. One that
        // doesn't fit is dropped, and every later one with it.
        let wrapper = match stmt {
            Statement::State { .. } => STATE_WRAPPER,
            Statement::Query { .. } => QUERY_WRAPPER,
            Statement::Mutation { .. } => MUTATION_WRAPPER,
            Statement::Value { .. } => 0,
        };
        if wrapper > 0 && !ctx.grow(wrapper + json_string_len(id)) {
            full = true;
            break;
        }
        match stmt {
            Statement::State { init } => {
                let before = ctx.size;
                let value = materialize_state(slot, init, ctx);
                if has_runtime(&value) {
                    expressions_size += ctx.size - before;
                }
                state.set(id.to_owned(), value);
            }
            Statement::Query { .. } => {
                let args = materialize_call_args(stmt.call_args(), ctx, id);
                // Aliases can hide state reads the statement as written doesn't show.
                let deps = collect_state_refs(args.get(1));
                let wait_for = query_reads(&args[..args.len().min(2)]);
                if !ctx.grow(names_size(&deps) + names_size(&wait_for)) {
                    full = true;
                    break;
                }
                queries.push(QueryStatement {
                    statement_id: id.to_owned(),
                    tool: args.first().cloned(),
                    args: args.get(1).cloned(),
                    defaults: args.get(2).cloned(),
                    refresh: args.get(3).cloned(),
                    deps: (!deps.is_empty()).then_some(deps),
                    wait_for: (!wait_for.is_empty()).then_some(wait_for),
                    complete: true,
                });
            }
            Statement::Mutation { .. } => {
                let args = materialize_call_args(stmt.call_args(), ctx, id);
                mutations.push(MutationStatement {
                    statement_id: id.to_owned(),
                    tool: args.first().cloned(),
                    args: args.get(1).cloned(),
                });
            }
            Statement::Value { .. } => {}
        }
    }
    // Evaluating builds no more than the result-size limit leaves room for,
    // and takes no more than `work` steps.
    let room = ctx
        .limits
        .result_size()
        .saturating_sub(ctx.size - expressions_size);
    let evaluated = state_defaults(state, room, ctx.limits.depth(), work, reuse);
    ctx.frame.size_limited |= evaluated.size_limited;
    ctx.frame.limited |= evaluated.depth_limited;
    let (mut state, undefined) = (evaluated.state, evaluated.undefined);
    // A query waits for the queries it reads; ones that read each other can
    // never start.
    query_cycles(&queries, |found| {
        // One that doesn't fit is cut before its names are gathered: they
        // cost as much to gather as to write.
        let message_len = query_cycle_message_len(found.path.len(), found.bytes);
        if !ctx.issue_fits("Query", "", message_len) {
            return;
        }
        let cycle = found.names();
        let outer = ctx.current_statement_id.replace(cycle[0].clone());
        ctx.push_issue("Query", "", Issue::QueryCycle { cycle });
        ctx.current_statement_id = outer;
    });
    // Nor is a `$name` declared null once one doesn't fit, or a statement
    // didn't: a dropped declaration isn't written as null instead.
    'declare: for &slot in table.specials() {
        if full {
            break;
        }
        for name in &table.info(slot).state_refs {
            if partial.is_some_and(|p| p.name == name && p.slots.contains(&slot)) {
                continue;
            }
            if !state.has(name) && !undefined.contains(name) {
                if !ctx.grow(STATE_WRAPPER + json_string_len(name) + "null".len()) {
                    break 'declare;
                }
                state.set(name.clone(), Value::Null);
            }
        }
    }
    (state, queries, mutations)
}
