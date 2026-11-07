//! Edit mode: merging a patch program into an existing one, as the reference
//! parser's `mergeStatements`.
//!
//! This works on source text, not parse results: each statement keeps the
//! exact text it was written with, a patch statement replaces the existing one
//! with the same name (or is appended), `name = null` deletes, and statements
//! no longer reachable from the root are dropped.

use std::collections::{HashMap, HashSet};

use crate::Limits;
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
/// it, trimmed at the end. Comments inside that span stay in the text. Sets
/// `depth_limited` when an expression nests past `depth` and is cut off.
fn parse_statements<'a>(input: &'a str, depth: usize, depth_limited: &mut bool) -> Vec<Parsed<'a>> {
    split(&tokenize(input))
        .into_iter()
        .map(|statement| {
            let (ast, limited) = parse_expression_with_limit(&statement.tokens, depth);
            *depth_limited |= limited;
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
///
/// It merges under the default [`Limits`]; see
/// [`merge_statements_with_limits`] for what they do here.
pub fn merge_statements(existing: &str, patch: &str, root_id: Option<&str>) -> String {
    merge_statements_with_limits(existing, patch, root_id, Limits::DEFAULT).text
}

/// A merge's program text, and whether a limit cut it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    /// The merged program.
    pub text: String,
    /// An input was longer than the program-size limit and was read only up
    /// to it, as if the stream had stopped there; or (in a release build) a
    /// loop stalled, a bug in the core, and the merge stopped there.
    pub size_limited: bool,
    /// An expression nested past the depth limit and was cut off there. Its
    /// statement's text is kept whole, but references past the cut weren't
    /// seen, so statements only they reach may have been dropped as
    /// unreachable.
    pub depth_limited: bool,
}

/// [`merge_statements`] under `limits`. The program-size limit applies to
/// each input: past it, the rest of that input is ignored as if the stream
/// had stopped there. The depth limit applies to each statement's
/// expression, read to find what the root reaches. The result-size limit
/// doesn't apply: a merge is never longer than its two inputs.
pub fn merge_statements_with_limits(
    existing: &str,
    patch: &str,
    root_id: Option<&str>,
    limits: Limits,
) -> Merged {
    crate::stack::grow_stack!(merge_statements_with_limits(
        existing, patch, root_id, limits
    ));
    let (existing, existing_cut) = Limits::fit(existing, limits.program_size());
    let (patch, patch_cut) = Limits::fit(patch, limits.program_size());
    let mut depth_limited = false;
    let (text, stalled) = crate::progress::watch(|| {
        merge(existing, patch, root_id, limits.depth(), &mut depth_limited)
    });
    Merged {
        text,
        size_limited: existing_cut || patch_cut || stalled,
        depth_limited,
    }
}

fn merge(
    existing: &str,
    patch: &str,
    root_id: Option<&str>,
    depth: usize,
    depth_limited: &mut bool,
) -> String {
    let existing_statements = parse_statements(existing, depth, depth_limited);
    let fenced = strip_fences(patch);
    let patch_statements = parse_statements(&fenced, depth, depth_limited);

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
