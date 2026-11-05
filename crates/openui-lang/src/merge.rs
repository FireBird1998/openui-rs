//! Edit mode: merging a patch program into an existing one, as the reference
//! parser's `mergeStatements`.
//!
//! This works on source text, not parse results: each statement keeps the
//! exact text it was written with, a patch statement replaces the existing one
//! with the same name (or is appended), `name = null` deletes, and statements
//! no longer reachable from the root are dropped.

use std::collections::{HashMap, HashSet};

use crate::ast::Ast;
use crate::expr::parse_expression_with_limit;
use crate::js;
use crate::lexer::tokenize;
use crate::preprocess::strip_fences;
use crate::statements::split;

struct Parsed<'a> {
    id: String,
    ast: Ast,
    raw: &'a str,
}

/// The program's statements as the parser splits them, each with its
/// expression and its source text: from its name to the line break that ends
/// it, trimmed at the end. Comments inside that span stay in the text.
fn parse_statements(input: &str) -> Vec<Parsed<'_>> {
    split(&tokenize(input))
        .into_iter()
        .map(|statement| {
            let (ast, _) = parse_expression_with_limit(&statement.tokens);
            Parsed {
                raw: js::trim_end(&input[statement.start.byte..statement.end.byte]),
                id: statement.id,
                ast,
            }
        })
        .collect()
}

/// Drop statements unreachable from `root_id`, following references from its
/// expression. `$` statements stay: they're bound at runtime, not referenced.
/// Without a root statement nothing is dropped.
fn gc_unreachable(
    order: &mut Vec<String>,
    merged: &mut HashMap<String, String>,
    asts: &HashMap<String, Ast>,
    root_id: &str,
) {
    if !asts.contains_key(root_id) {
        return;
    }
    let mut reachable: HashSet<&str> = HashSet::from([root_id]);
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        let Some(ast) = asts.get(id) else { continue };
        let mut refs: Vec<String> = Vec::new();
        ast.walk(&mut |node| match node {
            Ast::Ref(name) | Ast::RuntimeRef { n: name, .. } => refs.push(name.clone()),
            _ => {}
        });
        for name in refs {
            if let Some((key, _)) = asts.get_key_value(name.as_str())
                && reachable.insert(key)
            {
                queue.push(key);
            }
        }
    }
    let mut keep: HashSet<String> = reachable.into_iter().map(str::to_owned).collect();
    keep.extend(order.iter().filter(|id| id.starts_with('$')).cloned());
    order.retain(|id| {
        let kept = keep.contains(id);
        if !kept {
            merged.remove(id);
        }
        kept
    });
}

/// Merge `patch` into `existing`, as the reference's `mergeStatements`.
///
/// Patch statements replace existing ones by name, keeping their position;
/// new names are appended; `name = null` deletes `name`. Then statements
/// unreachable from `root_id` (default `"root"`) are dropped, except `$`
/// state. The patch may be wrapped in code fences with prose around them; the
/// existing program is taken as it is. With no existing statements the result
/// is the patch's statements; with no patch statements it's `existing`
/// unchanged.
pub fn merge_statements(existing: &str, patch: &str, root_id: Option<&str>) -> String {
    let existing_statements = parse_statements(existing);
    let fenced = strip_fences(patch);
    let patch_statements = parse_statements(&fenced);

    if existing_statements.is_empty() {
        return patch_statements
            .iter()
            .map(|s| s.raw)
            .collect::<Vec<_>>()
            .join("\n");
    }
    if patch_statements.is_empty() {
        return existing.to_owned();
    }

    // An id defined twice in the existing program keeps its first place, with
    // the latest text.
    let mut merged: HashMap<String, String> = HashMap::new();
    let mut asts: HashMap<String, Ast> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for s in existing_statements {
        if !merged.contains_key(&s.id) {
            order.push(s.id.clone());
        }
        merged.insert(s.id.clone(), s.raw.to_owned());
        asts.insert(s.id, s.ast);
    }
    for s in patch_statements {
        if s.ast == Ast::Null {
            merged.remove(&s.id);
            asts.remove(&s.id);
            if let Some(at) = order.iter().position(|id| *id == s.id) {
                order.remove(at);
            }
            continue;
        }
        if !merged.contains_key(&s.id) {
            order.push(s.id.clone());
        }
        merged.insert(s.id.clone(), s.raw.to_owned());
        asts.insert(s.id, s.ast);
    }

    gc_unreachable(&mut order, &mut merged, &asts, root_id.unwrap_or("root"));

    order
        .iter()
        .filter_map(|id| merged.get(id).map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}
