//! The invariants, each written once against the engine interface. The
//! runner (`invariants.rs`) checks each on both parsers and sorts what breaks
//! them; TESTING.md ("Invariants") lists what each rule covers, where it
//! doesn't apply, and its exceptions.

use openui_lang::{
    Ast, Token, auto_close, parse_expression, parse_json, preprocess, split, stringify_json,
    tokenize,
};
use serde_json::{Value, json};

use crate::Program;
use crate::engine::{Engine, MergeCall, ParseCall, SerializeCall, StreamCall};
use crate::invariants::{Input, Rule, Verdict};

pub const RULES: [Rule; 8] = [
    Rule {
        name: "round-trip",
        check: round_trip,
    },
    Rule {
        name: "chunking",
        check: chunking,
    },
    Rule {
        name: "stream-end",
        check: stream_end,
    },
    Rule {
        name: "fences",
        check: fences,
    },
    Rule {
        name: "merge-twice",
        check: merge_twice,
    },
    Rule {
        name: "merge-empty",
        check: merge_empty,
    },
    Rule {
        name: "merge-own-source",
        check: merge_own_source,
    },
    Rule {
        name: "finished-stay-put",
        check: finished_stay_put,
    },
];

// ─── Reading trees ──────────────────────────────────────────────────────────

/// Where two values first differ, as a JSON pointer and both values.
fn first_difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    let short = |v: &Value| {
        let s = stringify_json(v);
        if s.chars().count() > 120 {
            format!("{}…", s.chars().take(120).collect::<String>())
        } else {
            s
        }
    };
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    Some(w) => {
                        if let Some(d) = first_difference(v, w, &format!("{path}/{k}")) {
                            return Some(d);
                        }
                    }
                    None => return Some(format!("{path}/{k}: {} before, missing after", short(v))),
                }
            }
            y.keys()
                .find(|k| !x.contains_key(*k))
                .map(|k| format!("{path}/{k}: missing before, {} after", short(&y[k])))
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (v, w))| first_difference(v, w, &format!("{path}/{i}"))),
        _ if a == b => None,
        _ => Some(format!(
            "{}: {} before, {} after",
            if path.is_empty() { "/" } else { path },
            short(a),
            short(b)
        )),
    }
}

/// Every node of a tree, depth first: elements, AST nodes, data.
fn nodes<'v>(value: &'v Value, out: &mut Vec<&'v Value>) {
    out.push(value);
    match value {
        Value::Array(items) => items.iter().for_each(|v| nodes(v, out)),
        Value::Object(map) => map.values().for_each(|v| nodes(v, out)),
        _ => {}
    }
}

/// The AST kinds a materialized tree holds.
const AST_KINDS: [&str; 17] = [
    "Comp",
    "Str",
    "Num",
    "Bool",
    "Null",
    "Ph",
    "Ref",
    "StateRef",
    "RuntimeRef",
    "Arr",
    "Obj",
    "BinOp",
    "UnaryOp",
    "Ternary",
    "Member",
    "Index",
    "Assign",
];

/// An AST node's kind; `None` for anything else, data with a `k` key too.
fn kind(value: &Value) -> Option<&str> {
    value
        .get("k")
        .and_then(Value::as_str)
        .filter(|k| AST_KINDS.contains(k))
}

/// Whether a program has an object literal written to look like a tree
/// node: `{type: "element", ...}` or `{k: "Comp", ...}`. Converting back to
/// source can't tell such data from the real thing, the same family as the
/// root-shaped divergences in CONFORMANCE.md. A shrunk program may have lost
/// a kind string's closing quote.
fn has_node_shaped_literal(input: &str) -> bool {
    const KINDS: [&str; 18] = [
        "\"element",
        "\"Comp",
        "\"Str",
        "\"Num",
        "\"Bool",
        "\"Null",
        "\"Ph",
        "\"Ref",
        "\"StateRef",
        "\"RuntimeRef",
        "\"Arr",
        "\"Obj",
        "\"BinOp",
        "\"UnaryOp",
        "\"Ternary",
        "\"Member",
        "\"Index",
        "\"Assign",
    ];
    input.contains('{') && KINDS.iter().any(|k| input.contains(k))
}

// ─── Round trip ─────────────────────────────────────────────────────────────

/// An element tree as the round trip compares it: every element without its
/// `partial` and `hasDynamicProps` flags, which upstream's own round-trip test
/// ignores too (here at every depth), and without null props. Converting back
/// to source leaves trailing null args out on purpose, so a null prop comes
/// back missing, and the two mean the same.
fn normalized(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        Value::Object(map) => {
            let element = map.get("type") == Some(&json!("element"));
            Value::Object(
                map.iter()
                    .filter(|(k, _)| !(element && (*k == "partial" || *k == "hasDynamicProps")))
                    .map(|(k, v)| match (element && k == "props", v) {
                        (true, Value::Object(props)) => (
                            k.clone(),
                            Value::Object(
                                props
                                    .iter()
                                    .filter(|(_, v)| !v.is_null())
                                    .map(|(k, v)| (k.clone(), normalized(v)))
                                    .collect(),
                            ),
                        ),
                        _ => (k.clone(), normalized(v)),
                    })
                    .collect(),
            )
        }
        _ => value.clone(),
    }
}

/// Whether a tree holds what converting back to source can't give back, so
/// the round trip doesn't apply:
/// - a reference to a Query or Mutation statement, since `jsonToOpenUI` takes
///   the tree and state, not those statements (its output is a patch, and
///   merging it keeps them);
/// - a placeholder (`Ph`), where a statement refers to itself and the tree
///   cuts the cycle;
/// - a number JSON can't hold (`NaN` or `Infinity`, which JSON writes as
///   null), since trees cross the interface as JSON.
fn beyond_source(tree: &Value) -> bool {
    let mut all = Vec::new();
    nodes(tree, &mut all);
    has_cycle(tree, &mut Vec::new())
        || all.iter().any(|n| match kind(n) {
            Some("RuntimeRef") => matches!(
                n.get("refType").and_then(Value::as_str),
                Some("query" | "mutation")
            ),
            Some("Ph") => true,
            Some("Num") => n.get("v").is_some_and(Value::is_null),
            _ => false,
        })
}

/// Whether an element holds an element of its own statement: a cycle the
/// tree cut, through a data object, say (`a = {t: root}`).
fn has_cycle<'v>(value: &'v Value, ancestors: &mut Vec<&'v str>) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|v| has_cycle(v, ancestors)),
        Value::Object(map) => {
            let id = (map.get("type") == Some(&json!("element")))
                .then(|| map.get("statementId").and_then(Value::as_str))
                .flatten();
            if id.is_some_and(|id| ancestors.contains(&id)) {
                return true;
            }
            ancestors.extend(id);
            let found = map.values().any(|v| has_cycle(v, ancestors));
            if id.is_some() {
                ancestors.pop();
            }
            found
        }
        _ => false,
    }
}

/// A key the serializer can write bare and the parser reads back: an
/// identifier, a `$name` or digits.
fn bare_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        // Digits read back as a number, so not with a leading zero: `01` is 1.
        Some(c) if c.is_ascii_digit() => {
            key.bytes().all(|b| b.is_ascii_digit()) && (key == "0" || c != '0')
        }
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        }
        _ => false,
    }
}

fn precedence(op: &str) -> u8 {
    match op {
        "||" => 2,
        "&&" => 3,
        "==" | "!=" => 4,
        ">" | "<" | ">=" | "<=" => 5,
        "+" | "-" => 6,
        "*" | "/" | "%" => 7,
        _ => 0,
    }
}

/// Names every object inherits from `Object.prototype`.
const PROTOTYPE_NAMES: [&str; 12] = [
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
    "__proto__",
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
];

/// Whether an AST node is written back starting with a digit, so a `-`
/// before it would be read as part of a number.
fn starts_with_number(node: &Value) -> bool {
    match kind(node) {
        Some("Num") => true,
        Some("Member" | "Index") => starts_with_number(&node["obj"]),
        Some("BinOp") => starts_with_number(&node["left"]),
        Some("Ternary") => starts_with_number(&node["cond"]),
        _ => false,
    }
}

/// Whether a program has a number literal JSON can't hold: one that reads
/// as `NaN` (`1e`, `2.5e+`) or overflows to `Infinity` (`1e400`). JSON
/// writes both as null, so such a tree can't cross the interface intact.
fn has_unjsonable_number(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let starts = (bytes[i].is_ascii_digit() || bytes[i] == b'.')
            && (i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric()
                    || bytes[i - 1] == b'_'
                    || bytes[i - 1] == b'$'));
        if !starts {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            i += 1;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            let digits = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == digits {
                return true;
            }
        }
        if input[start..i].parse::<f64>().is_ok_and(f64::is_infinite) {
            return true;
        }
    }
    false
}

/// Whether a program's statements refer to each other in a cycle (`x = [y]`,
/// `y = Root([x])`), as the parser splits it.
fn refers_in_a_cycle(input: &str) -> bool {
    let graph: Vec<(String, Vec<String>)> = split(&tokenize(&preprocess(input)))
        .into_iter()
        .map(|s| {
            let mut names = Vec::new();
            parse_expression(&s.tokens).walk(&mut |node| {
                if let Ast::Ref(name) = node {
                    names.push(name.clone());
                }
            });
            (s.id, names)
        })
        .collect();
    // Depth first, each name visited once: a name met again while it's still
    // on the path closes a cycle.
    fn closes<'g>(
        graph: &'g [(String, Vec<String>)],
        id: &'g str,
        path: &mut Vec<&'g str>,
        done: &mut Vec<&'g str>,
    ) -> bool {
        if path.contains(&id) {
            return true;
        }
        if done.contains(&id) {
            return false;
        }
        path.push(id);
        let found = graph
            .iter()
            .filter(|(name, _)| name == id)
            .flat_map(|(_, refs)| refs)
            .any(|r| closes(graph, r, path, done));
        path.pop();
        done.push(id);
        found
    }
    let mut done = Vec::new();
    graph
        .iter()
        .any(|(id, _)| closes(&graph, id, &mut Vec::new(), &mut done))
}

/// Whether one statement comes out as two different elements in a tree, as
/// it does reached through a reference cycle and around it. Converting back
/// to source writes each statement once, so one of the two is lost.
fn one_statement_twice(tree: &Value) -> bool {
    let mut all = Vec::new();
    nodes(tree, &mut all);
    let elements: Vec<(&Value, Value)> = all
        .into_iter()
        .filter(|n| n.get("type") == Some(&json!("element")))
        .filter_map(|n| Some((n.get("statementId")?, normalized(n))))
        .collect();
    elements.iter().enumerate().any(|(i, (id, element))| {
        elements[..i]
            .iter()
            .any(|(other, seen)| other == id && seen != element)
    })
}

/// The quirk behind a broken round trip, if a known one explains it.
fn round_trip_exception(input: &str, tree: &Value, why: &str) -> Option<&'static str> {
    let mut all = Vec::new();
    nodes(tree, &mut all);
    // A bare library call has no `fn` flag; written back with `@`, it has one.
    if why.contains("/fn: missing before, true after") {
        return Some("quirk 12: a bare library call");
    }
    let unquoted = all.iter().any(|n| match (kind(n), n) {
        (Some("Obj"), _) => n["entries"].as_array().is_some_and(|entries| {
            entries.iter().any(|e| {
                e.get(0)
                    .and_then(Value::as_str)
                    .is_some_and(|k| !bare_key(k))
            })
        }),
        (None, Value::Object(map)) if map.get("type") != Some(&json!("element")) => {
            map.keys().any(|k| !bare_key(k))
        }
        _ => false,
    });
    if unquoted {
        return Some("quirk 10: an object key that isn't bare");
    }
    let parens = all.iter().any(|n| {
        kind(n) == Some("BinOp")
            && kind(&n["right"]) == Some("BinOp")
            && precedence(n["op"].as_str().unwrap_or_default())
                == precedence(n["right"]["op"].as_str().unwrap_or_default())
    });
    if parens {
        return Some("quirk 9: an equal-precedence right operand");
    }
    // An operand that needs parentheses loses them: a unary operator's
    // (`!($a && $b)` comes back as `!$a && $b`, `-(3)` as the number `-3`), a
    // binary operator's ternary (`$a || ($b ? 1 : 2)`), and a member
    // access's or index's object (`(!$a).x`, and `(5).x` or `5. 1` as `5.1`).
    let needs_parens =
        |operand: &Value| matches!(kind(operand), Some("BinOp" | "Ternary" | "Assign"));
    let bare_operand = all.iter().any(|n| match kind(n) {
        Some("UnaryOp") => {
            needs_parens(&n["operand"])
                || n["op"] == json!("-") && starts_with_number(&n["operand"])
        }
        Some("BinOp") => {
            matches!(kind(&n["left"]), Some("Ternary" | "Assign"))
                || matches!(kind(&n["right"]), Some("Ternary" | "Assign"))
        }
        Some("Member" | "Index") => {
            needs_parens(&n["obj"])
                || kind(&n["obj"]) == Some("UnaryOp")
                || starts_with_number(&n["obj"])
                || kind(n) == Some("Member")
                    && !n["field"].as_str().is_some_and(|f| {
                        bare_key(f) && !f.starts_with(|c: char| c.is_ascii_digit())
                    })
        }
        _ => false,
    });
    if bare_operand {
        return Some("quirk 13: an operand that needs parentheses");
    }
    // A call the parser already made null stays an expression, which the
    // required-prop check lets through; written back as `null`, it isn't.
    if all.iter().any(|n| kind(n) == Some("Null")) {
        return Some("quirk 14: a call made null when parsing");
    }
    if all.iter().any(|n| {
        kind(n) == Some("Comp")
            && n["name"]
                .as_str()
                .is_some_and(|name| PROTOTYPE_NAMES.contains(&name))
    }) {
        return Some("quirk 2: a call named after an Object.prototype member");
    }
    if refers_in_a_cycle(input) && one_statement_twice(tree) {
        return Some("quirk 21: a statement reached through a reference cycle");
    }
    if has_node_shaped_literal(input) {
        return Some("an object literal shaped like a tree node");
    }
    None
}

/// A program's tree, as JSON, and its state declarations' JSON, if the
/// round trip applies to it (see [`round_trip`]).
fn round_trip_tree(
    answer: &Result<String, String>,
    p: &Program,
) -> Option<(Value, String, String)> {
    let result = parse_json(answer.as_ref().ok()?).ok()?;
    if result["meta"]["incomplete"] != json!(false) || has_unjsonable_number(&p.input) {
        return None;
    }
    let root = result
        .get("root")
        .filter(|r| !r.is_null() && !beyond_source(r))?;
    Some((
        root.clone(),
        stringify_json(root),
        stringify_json(&result["stateDeclarations"]),
    ))
}

/// **Round trip:** parsing, converting the tree back to source and parsing
/// that again gives the same tree. It applies to complete programs with a
/// tree, short of what [`beyond_source`] lists and of numbers JSON can't hold
/// ([`has_unjsonable_number`]): while a program is still arriving, values
/// that are still being written are let through that the finished program
/// would reject.
fn round_trip(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let programs: Vec<&Program> = inputs.iter().map(|i| &i.program).collect();
    let calls: Vec<ParseCall> = programs
        .iter()
        .map(|p| ParseCall {
            lib: &p.lib,
            input: &p.input,
            root: p.root.as_deref(),
        })
        .collect();
    let first = engine.parse(&calls);
    // The tree and state declarations, for each program the rule applies to.
    let trees: Vec<Option<(Value, String, String)>> = first
        .iter()
        .zip(&programs)
        .map(|(answer, p)| round_trip_tree(answer, p))
        .collect();
    let applies: Vec<usize> = (0..programs.len())
        .filter(|&i| trees[i].is_some())
        .collect();
    let calls: Vec<SerializeCall> = applies
        .iter()
        .map(|&i| {
            let (_, element, state) = trees[i].as_ref().expect("a tree");
            SerializeCall {
                lib: &programs[i].lib,
                element,
                state: Some(state),
            }
        })
        .collect();
    let sources = engine.serialize(&calls);
    let reparse: Vec<ParseCall> = applies
        .iter()
        .zip(&sources)
        .filter_map(|(&i, source)| {
            let p = programs[i];
            Some(ParseCall {
                lib: &p.lib,
                input: source.as_ref().ok()?,
                root: p.root.as_deref(),
            })
        })
        .collect();
    let mut second = engine.parse(&reparse).into_iter();

    let mut verdicts = vec![Verdict::Skipped; programs.len()];
    for (&i, source) in applies.iter().zip(&sources) {
        let before = &trees[i].as_ref().expect("a tree").0;
        let why = match source {
            Err(error) => Some(format!("converting back to source threw: {error}")),
            Ok(source) => {
                let after = second
                    .next()
                    .expect("a second parse")
                    .ok()
                    .and_then(|json| parse_json(&json).ok())
                    .map(|result| result["root"].clone())
                    .unwrap_or(Value::Null);
                first_difference(&normalized(before), &normalized(&after), "")
                    .map(|d| format!("{d}; the source: {source:?}"))
            }
        };
        verdicts[i] = match why {
            None => Verdict::Holds,
            Some(why) => Verdict::Broken {
                exception: round_trip_exception(&programs[i].input, before, &why),
                why,
            },
        };
    }
    verdicts
}

// ─── Streams ────────────────────────────────────────────────────────────────

/// Rules that stream programs apply to ones up to this long, since each push
/// of a longer one costs a full result from the reference.
const STREAM_LIMIT: usize = 4000;

/// `n` code points in chunks of `size`.
fn fixed_chunks(n: usize, size: usize) -> Vec<usize> {
    (0..n.div_ceil(size))
        .map(|i| size.min(n - i * size))
        .collect()
}

fn read(answer: &Result<String, String>) -> Result<Value, String> {
    let json = answer.as_ref().map_err(|e| format!("threw: {e}"))?;
    parse_json(json).map_err(|e| format!("unreadable JSON: {e}"))
}

/// **Chunking doesn't matter:** a stream's result once it has ended is the
/// same however the program was cut: in one chunk, in the fuzzer's random
/// chunks, and in chunks of three code points.
fn chunking(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let applies: Vec<&Program> = inputs
        .iter()
        .map(|i| &i.program)
        .filter(|p| !p.input.is_empty() && p.input.chars().count() <= STREAM_LIMIT)
        .collect();
    let whole: Vec<Vec<usize>> = applies
        .iter()
        .map(|p| vec![p.input.chars().count()])
        .collect();
    let threes: Vec<Vec<usize>> = applies
        .iter()
        .map(|p| fixed_chunks(p.input.chars().count(), 3))
        .collect();
    let mut ends: Vec<Vec<Result<String, String>>> = Vec::new();
    for sizes in [
        whole,
        applies.iter().map(|p| p.sizes.clone()).collect(),
        threes,
    ] {
        let calls: Vec<StreamCall> = applies
            .iter()
            .zip(&sizes)
            .map(|(p, sizes)| StreamCall {
                lib: &p.lib,
                input: &p.input,
                root: p.root.as_deref(),
                sizes,
                steps: false,
            })
            .collect();
        ends.push(
            engine
                .stream(&calls)
                .into_iter()
                .map(|a| a.map(|s| s.end))
                .collect(),
        );
    }
    let mut verdicts = Vec::new();
    let mut at = 0;
    for input in inputs {
        let p = &input.program;
        if p.input.is_empty() || p.input.chars().count() > STREAM_LIMIT {
            verdicts.push(Verdict::Skipped);
            continue;
        }
        let why = (|| {
            let one = read(&ends[0][at])?;
            for (name, chunked) in [("random", &ends[1][at]), ("three-code-point", &ends[2][at])] {
                if let Some(d) = first_difference(&one, &read(chunked)?, "") {
                    return Err(format!("{name} chunks against one chunk: {d}"));
                }
            }
            Ok(())
        })();
        at += 1;
        verdicts.push(match why {
            Ok(()) => Verdict::Holds,
            Err(why) => Verdict::Broken {
                exception: stream_exception(&p.input, &why),
                why,
            },
        });
    }
    verdicts
}

/// Whether a program has a closing bracket that doesn't match the one open.
fn mismatched_bracket(input: &str) -> bool {
    let mut stack = Vec::new();
    let mut string: Option<char> = None;
    let mut escaped = false;
    for c in input.chars() {
        if let Some(quote) = string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                string = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => string = Some(c),
            '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => {
                let open = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                if stack.last() == Some(&open) {
                    stack.pop();
                } else {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Whether a line starts with something the lexer skips and then `?`: the
/// one-shot split sees a ternary's `?` (a token), the stream's scan doesn't
/// (a character).
fn skipped_before_question(input: &str) -> bool {
    input.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('?') && tokenize(trimmed).first() == Some(&Token::Question)
    })
}

/// The quirk behind a stream ending differently, if a known one explains it.
fn stream_exception(input: &str, why: &str) -> Option<&'static str> {
    if skipped_before_question(input) {
        return Some("quirk 19: a stray character before a ternary's `?`");
    }
    if why.contains("/meta/incomplete") && split(&tokenize(&preprocess(input))).is_empty() {
        return Some("quirk 18: a program without statements");
    }
    if why.contains("/meta/statementCount") && (redefines(input) || redefined_empty(input)) {
        return Some("quirk 4: streaming counts a redefinition");
    }
    if mismatched_bracket(&preprocess(input)) {
        return Some("quirk 5: a mismatched closing bracket");
    }
    if redefines(input) {
        return Some("quirk 11: a final redefinition that needed closing");
    }
    None
}

/// **Streaming ends where one-shot parsing ends:** a stream in the fuzzer's
/// random chunks gives, once it has ended, what parsing the whole program at
/// once gives.
fn stream_end(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let applies: Vec<&Program> = inputs
        .iter()
        .map(|i| &i.program)
        .filter(|p| !p.input.is_empty() && p.input.chars().count() <= STREAM_LIMIT)
        .collect();
    let parses = engine.parse(
        &applies
            .iter()
            .map(|p| ParseCall {
                lib: &p.lib,
                input: &p.input,
                root: p.root.as_deref(),
            })
            .collect::<Vec<_>>(),
    );
    let streams = engine.stream(
        &applies
            .iter()
            .map(|p| StreamCall {
                lib: &p.lib,
                input: &p.input,
                root: p.root.as_deref(),
                sizes: &p.sizes,
                steps: false,
            })
            .collect::<Vec<_>>(),
    );
    let mut answers = parses.iter().zip(streams);
    inputs
        .iter()
        .map(|input| {
            let p = &input.program;
            if p.input.is_empty() || p.input.chars().count() > STREAM_LIMIT {
                return Verdict::Skipped;
            }
            let (parsed, streamed) = answers.next().expect("an answer per program");
            let why = (|| {
                let once = read(parsed)?;
                let end = read(&streamed.map(|s| s.end))?;
                match first_difference(&once, &end, "") {
                    Some(d) => Err(format!("the stream's end against one-shot: {d}")),
                    None => Ok(()),
                }
            })();
            match why {
                Ok(()) => Verdict::Holds,
                Err(why) => Verdict::Broken {
                    exception: stream_exception(&p.input, &why),
                    why,
                },
            }
        })
        .collect()
}

// ─── Fences ─────────────────────────────────────────────────────────────────

/// The program inside a fence, with prose before and after (and no
/// apostrophe in the prose, which quirk 1 is about).
fn fenced(input: &str) -> String {
    format!("Here is the UI:\n\n```openui-lang\n{input}\n```\n\nAnything else?")
}

/// Whether a text, as parsing reads it, ends inside a string or a bracket:
/// an unfinished statement.
fn unfinished(text: &str) -> bool {
    auto_close(&preprocess(text)).1
}

/// **Fences don't matter:** a program inside a markdown fence, with prose
/// before and after, parses the same as the bare program. It applies to
/// finished programs without a fence of their own: an unclosed string or
/// bracket would take in the closing fence and the prose.
fn fences(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let fence_applies = |p: &Program| !p.input.contains("```") && !unfinished(&p.input);
    let applies: Vec<&Program> = inputs
        .iter()
        .map(|i| &i.program)
        .filter(|p| fence_applies(p))
        .collect();
    let wrapped: Vec<String> = applies.iter().map(|p| fenced(&p.input)).collect();
    let calls: Vec<ParseCall> = applies
        .iter()
        .map(|p| ParseCall {
            lib: &p.lib,
            input: &p.input,
            root: p.root.as_deref(),
        })
        .chain(applies.iter().zip(&wrapped).map(|(p, w)| ParseCall {
            lib: &p.lib,
            input: w,
            root: p.root.as_deref(),
        }))
        .collect();
    let answers = engine.parse(&calls);
    let (bare, inside) = answers.split_at(applies.len());
    let mut at = 0;
    inputs
        .iter()
        .map(|input| {
            if !fence_applies(&input.program) {
                return Verdict::Skipped;
            }
            let why = (|| match first_difference(&read(&bare[at])?, &read(&inside[at])?, "") {
                Some(d) => Err(format!("fenced against bare: {d}")),
                None => Ok(()),
            })();
            at += 1;
            match why {
                Ok(()) => Verdict::Holds,
                Err(why) => Verdict::Broken {
                    // `parse` gives an empty program `incomplete: true`.
                    exception: preprocess(&input.program.input)
                        .is_empty()
                        .then_some("quirk 18: an empty program reads as incomplete"),
                    why,
                },
            }
        })
        .collect()
}

// ─── Merging ────────────────────────────────────────────────────────────────

fn merges(engine: &mut dyn Engine, pairs: &[(&str, &str)]) -> Vec<Result<String, String>> {
    let calls: Vec<MergeCall> = pairs
        .iter()
        .map(|(existing, patch)| MergeCall {
            existing,
            patch,
            root_id: None,
        })
        .collect();
    engine.merge(&calls)
}

/// The quirk behind a merge that misbehaves, if a known one explains it.
/// `merged` is the first merge's result, for a rule that merges again.
fn merge_exception(input: &Input, merged: Option<&str>) -> Option<&'static str> {
    // `mergeStatements` returns a patch into a program without statements
    // as it is: no deletions, no duplicates resolved, nothing unreachable
    // removed. (A program the merge finds none in, prose with an apostrophe
    // included, which is quirk 7.) A first merge that deleted everything
    // leaves such a program for the second.
    let empty = |text: &str| split(&tokenize(text)).is_empty();
    if empty(&input.program.input) || merged.is_some_and(empty) {
        return Some("quirk 16: merging into a program without statements");
    }
    // The merge doesn't extract the existing program's fences (quirk 7's
    // cause), so the prose around them reads as code.
    if input.program.input.contains("```") {
        return Some("quirk 7: the existing program's fences");
    }
    // An unclosed string or bracket (the text's own, or one line's, which
    // may close against a later line), or a stray closing bracket, joins
    // statements that merging then sees differently.
    let unbalanced = |text: &str| {
        unfinished(text)
            || text
                .lines()
                .any(|line| line.contains('=') && (auto_close(line).1 || mismatched_bracket(line)))
    };
    if unbalanced(&input.program.input) || unbalanced(&input.patch) {
        return Some("quirk 15: an unfinished statement takes in what follows it");
    }
    // A ternary still open (a `?` without its `:`) keeps its statement going
    // across lines, so what the merge writes after it joins it; the next
    // merge, splitting as the parser does (1.1's fix for quirk 6), sees it as
    // part of that statement.
    [
        &input.program.input,
        &input.patch,
        merged.unwrap_or_default(),
    ]
    .iter()
    .any(|text| open_ternary(text))
    .then_some("quirk 22: an open ternary takes in what the merge writes after it")
}

/// Whether a statement, as the parser splits the text, ends with a ternary
/// still open: more `?` than `:` outside brackets.
fn open_ternary(text: &str) -> bool {
    split(&tokenize(&preprocess(text))).iter().any(|s| {
        let (mut depth, mut open) = (0usize, 0isize);
        for token in s.tokens.iter() {
            match token {
                Token::LParen | Token::LBrack | Token::LBrace => depth += 1,
                Token::RParen | Token::RBrack | Token::RBrace => depth = depth.saturating_sub(1),
                Token::Question if depth == 0 => open += 1,
                Token::Colon if depth == 0 => open -= 1,
                _ => {}
            }
        }
        open > 0
    })
}

/// **Merging behaves, twice:** applying a patch twice gives what applying it
/// once gives: the two programs parse the same. (The text may differ in
/// order: a patch that deletes a name and defines it again moves it to the
/// end the second time.)
fn merge_twice(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let pairs: Vec<(&str, &str)> = inputs
        .iter()
        .map(|i| (i.program.input.as_str(), i.patch.as_str()))
        .collect();
    let once = merges(engine, &pairs);
    let again_pairs: Vec<(&str, &str)> = once
        .iter()
        .zip(inputs)
        .map(|(m, i)| (m.as_deref().unwrap_or(""), i.patch.as_str()))
        .collect();
    let twice = merges(engine, &again_pairs);
    let texts = |merged: &[Result<String, String>]| -> Vec<String> {
        merged
            .iter()
            .map(|m| m.clone().unwrap_or_default())
            .collect()
    };
    let (once_texts, twice_texts) = (texts(&once), texts(&twice));
    let calls: Vec<ParseCall> = inputs
        .iter()
        .zip(&once_texts)
        .chain(inputs.iter().zip(&twice_texts))
        .map(|(i, text)| ParseCall {
            lib: &i.program.lib,
            input: text,
            root: i.program.root.as_deref(),
        })
        .collect();
    let parsed = engine.parse(&calls);
    let (once_parsed, twice_parsed) = parsed.split_at(inputs.len());
    (0..inputs.len())
        .map(|i| {
            let why = (|| {
                once[i]
                    .as_ref()
                    .map_err(|e| format!("merging threw: {e}"))?;
                twice[i]
                    .as_ref()
                    .map_err(|e| format!("merging again threw: {e}"))?;
                match first_difference(&read(&once_parsed[i])?, &read(&twice_parsed[i])?, "") {
                    Some(d) => Err(format!(
                        "{d}; once {:?}, twice {:?}",
                        once_texts[i], twice_texts[i]
                    )),
                    None => Ok(()),
                }
            })();
            match why {
                Ok(()) => Verdict::Holds,
                Err(why) => Verdict::Broken {
                    exception: merge_exception(inputs[i], Some(&once_texts[i])),
                    why,
                },
            }
        })
        .collect()
}

/// **Merging behaves, empty:** an empty patch changes nothing: the merged
/// program parses as the program did.
fn merge_empty(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let pairs: Vec<(&str, &str)> = inputs
        .iter()
        .map(|i| (i.program.input.as_str(), ""))
        .collect();
    let merged = merges(engine, &pairs);
    let texts: Vec<&str> = merged.iter().map(|m| m.as_deref().unwrap_or("")).collect();
    let calls: Vec<ParseCall> = inputs
        .iter()
        .map(|i| ParseCall {
            lib: &i.program.lib,
            input: &i.program.input,
            root: i.program.root.as_deref(),
        })
        .chain(inputs.iter().zip(&texts).map(|(i, text)| ParseCall {
            lib: &i.program.lib,
            input: text,
            root: i.program.root.as_deref(),
        }))
        .collect();
    let answers = engine.parse(&calls);
    let (before, after) = answers.split_at(inputs.len());
    (0..inputs.len())
        .map(|i| {
            let why = (|| {
                if let Err(e) = &merged[i] {
                    return Err(format!("merging threw: {e}"));
                }
                match first_difference(&read(&before[i])?, &read(&after[i])?, "") {
                    Some(d) => Err(format!("{d}; merged: {:?}", texts[i])),
                    None => Ok(()),
                }
            })();
            match why {
                Ok(()) => Verdict::Holds,
                Err(why) => Verdict::Broken {
                    // An empty result means the merge found no statements.
                    exception: merge_exception(inputs[i], Some(texts[i])),
                    why,
                },
            }
        })
        .collect()
}

/// **Merging behaves, its own source:** the program's tree converted back
/// to source works as a patch: merging it into the program gives the same
/// tree. It applies where the round trip does, and has its exceptions.
fn merge_own_source(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let programs: Vec<&Program> = inputs.iter().map(|i| &i.program).collect();
    let first = engine.parse(
        &programs
            .iter()
            .map(|p| ParseCall {
                lib: &p.lib,
                input: &p.input,
                root: p.root.as_deref(),
            })
            .collect::<Vec<_>>(),
    );
    let trees: Vec<Option<(Value, String, String)>> = first
        .iter()
        .zip(&programs)
        .map(|(answer, p)| round_trip_tree(answer, p))
        .collect();
    let applies: Vec<usize> = (0..programs.len())
        .filter(|&i| trees[i].is_some())
        .collect();
    let sources = engine.serialize(
        &applies
            .iter()
            .map(|&i| {
                let (_, element, state) = trees[i].as_ref().expect("a tree");
                SerializeCall {
                    lib: &programs[i].lib,
                    element,
                    state: Some(state),
                }
            })
            .collect::<Vec<_>>(),
    );
    let pairs: Vec<(&str, &str)> = applies
        .iter()
        .zip(&sources)
        .map(|(&i, s)| (programs[i].input.as_str(), s.as_deref().unwrap_or("")))
        .collect();
    let merged = merges(engine, &pairs);
    let texts: Vec<&str> = merged.iter().map(|m| m.as_deref().unwrap_or("")).collect();
    let reparsed = engine.parse(
        &applies
            .iter()
            .zip(&texts)
            .map(|(&i, text)| ParseCall {
                lib: &programs[i].lib,
                input: text,
                root: programs[i].root.as_deref(),
            })
            .collect::<Vec<_>>(),
    );
    let mut verdicts = vec![Verdict::Skipped; programs.len()];
    for (n, &i) in applies.iter().enumerate() {
        let before = &trees[i].as_ref().expect("a tree").0;
        let why = (|| {
            let source = sources[n]
                .as_ref()
                .map_err(|e| format!("converting back to source threw: {e}"))?;
            merged[n]
                .as_ref()
                .map_err(|e| format!("merging threw: {e}"))?;
            let after = read(&reparsed[n])?["root"].clone();
            match first_difference(&normalized(before), &normalized(&after), "") {
                Some(d) => Err(format!(
                    "{d}; the source: {source:?}; merged: {:?}",
                    texts[n]
                )),
                None => Ok(()),
            }
        })();
        verdicts[i] = match why {
            Ok(()) => Verdict::Holds,
            Err(why) => Verdict::Broken {
                exception: round_trip_exception(&programs[i].input, before, &why)
                    .or_else(|| merge_exception(inputs[i], Some(texts[n]))),
                why,
            },
        };
    }
    verdicts
}

// ─── Finished statements ────────────────────────────────────────────────────

/// Rule 5 streams with every push's result, so it takes shorter programs.
const STAY_PUT_LIMIT: usize = 2000;

/// A statement of a program, as a stream reveals it.
struct Statement {
    name: String,
    /// Code points received once it's complete, as language.md, section 4,
    /// rule 1 defines it: its ending newline has arrived at bracket depth
    /// zero, outside strings, with no ternary open, and the next line's first
    /// non-whitespace character has arrived and isn't `?`. `None` for the last
    /// statement, which only the stream's end completes.
    complete_at: Option<usize>,
    /// The names its expression refers to.
    references: Vec<String>,
}

/// A program's statements, in one pass: lines without a `name = ...` shape
/// are skipped, as the parser skips them.
fn statements(input: &str) -> Vec<Statement> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let add = |from: usize, to: usize, complete_at: Option<usize>, out: &mut Vec<Statement>| {
        let text: String = chars[from..to].iter().collect();
        let Some(eq) = text.find('=') else {
            return;
        };
        // `a = ` with nothing after it is no statement to the parser either.
        let value = &text[eq + 1..];
        if value.starts_with('=') || value.trim().is_empty() {
            return;
        }
        let Some(name) = statement_name(&text) else {
            return;
        };
        out.push(Statement {
            name,
            complete_at,
            references: references(text.trim()),
        });
    };
    let (mut depth, mut ternary) = (0usize, 0usize);
    let (mut string, mut escaped) = (None::<char>, false);
    let mut start = 0;
    for i in 0..chars.len() {
        let c = chars[i];
        if let Some(quote) = string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                string = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => string = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '?' if depth == 0 => ternary += 1,
            ':' if depth == 0 && ternary > 0 => ternary -= 1,
            '\n' if depth == 0 && ternary == 0 => {
                match (i + 1..chars.len()).find(|&q| !chars[q].is_whitespace()) {
                    // Nothing follows: the stream's end completes it.
                    None => break,
                    Some(q) if chars[q] == '?' => {}
                    Some(q) => {
                        add(start, i, Some(q + 1), &mut out);
                        start = i + 1;
                    }
                }
            }
            _ => {}
        }
    }
    add(start, chars.len(), None, &mut out);
    out
}

/// The name the parser gives a statement's text, if it's a statement: the
/// lexer skips some characters, so `` a `= `` defines `a`.
fn statement_name(text: &str) -> Option<String> {
    split(&tokenize(text)).into_iter().next().map(|s| s.id)
}

/// Whether a program defines a name twice, as the parser splits it, or as
/// its lines start (a stray bracket can hide statements from the split that
/// streaming still counts).
fn redefines(input: &str) -> bool {
    let all = split(&tokenize(&preprocess(input)));
    let split_twice = all
        .iter()
        .enumerate()
        .any(|(i, s)| all[..i].iter().any(|t| t.id == s.id));
    let names: Vec<String> = input.lines().filter_map(statement_name).collect();
    let lines_twice = names
        .iter()
        .enumerate()
        .any(|(i, n)| names[..i].contains(n));
    split_twice || lines_twice
}

/// Whether a line defines a name again with nothing after its `=` (`a=`),
/// which the split doesn't count as a statement but a stream's end does,
/// behind a stray opening bracket (`[`, `a=("")`, `a=`).
fn redefined_empty(input: &str) -> bool {
    let mut names: Vec<String> = Vec::new();
    for line in input.lines() {
        if let Some(name) = statement_name(line) {
            names.push(name);
            continue;
        }
        let tokens = tokenize(line);
        if let [
            Token::Ident(name) | Token::Type(name) | Token::StateVar(name),
            Token::Equals,
            Token::Newline | Token::Eof,
            ..,
        ] = &tokens[..]
            && names.contains(name)
        {
            return true;
        }
    }
    false
}

/// The names a statement's expression refers to.
fn references(statement: &str) -> Vec<String> {
    let tokens = tokenize(statement);
    let mut names = Vec::new();
    for raw in split(&tokens) {
        parse_expression(&raw.tokens).walk(&mut |node| {
            if let Ast::Ref(name) = node {
                names.push(name.clone());
            }
        });
    }
    names
}

/// The statement, and every one it reaches, if all of them are complete;
/// `None` if one isn't, or the references close a cycle.
fn fully_resolved(id: &str, complete: &[&Statement]) -> Option<Vec<String>> {
    fn walk(
        id: &str,
        complete: &[&Statement],
        reached: &mut Vec<String>,
        path: &mut Vec<String>,
    ) -> bool {
        // The last definition wins.
        let Some(statement) = complete.iter().rev().find(|s| s.name == id) else {
            return false;
        };
        for name in &statement.references {
            if path.contains(name) {
                return false;
            }
            if reached.contains(name) {
                continue;
            }
            reached.push(name.clone());
            path.push(name.clone());
            if !walk(name, complete, reached, path) {
                return false;
            }
            path.pop();
        }
        true
    }
    let mut reached = vec![id.to_owned()];
    let mut path = vec![id.to_owned()];
    walk(id, complete, &mut reached, &mut path).then_some(reached)
}

/// Every element in a result whose `statementId` is `id`.
fn elements_of<'v>(result: &'v Value, id: &str) -> Vec<&'v Value> {
    let mut all = Vec::new();
    nodes(&result["root"], &mut all);
    all.into_iter()
        .filter(|n| n.get("type") == Some(&json!("element")) && n["statementId"] == json!(id))
        .collect()
}

/// **Finished statements stay put:** while streaming, a statement that is
/// complete ([`Statement::complete_at`]) and fully resolved (it and every
/// statement it reaches are complete, none closes a cycle, and none is being
/// defined again in the text still arriving) shows the same
/// element in every later result, until it or a statement it reaches is
/// defined again. It applies to programs that preprocessing leaves alone (no
/// fences or comments) and that are up to [`STAY_PUT_LIMIT`] long, and it can
/// only see statements that are components, the ones the tree names.
fn finished_stay_put(engine: &mut dyn Engine, inputs: &[&Input]) -> Vec<Verdict> {
    let applies = |p: &Program| {
        !p.input.is_empty()
            && p.input.chars().count() <= STAY_PUT_LIMIT
            && preprocess(&p.input) == p.input
    };
    let chosen: Vec<&Program> = inputs
        .iter()
        .map(|i| &i.program)
        .filter(|p| applies(p))
        .collect();
    let streams = engine.stream(
        &chosen
            .iter()
            .map(|p| StreamCall {
                lib: &p.lib,
                input: &p.input,
                root: p.root.as_deref(),
                sizes: &p.sizes,
                steps: true,
            })
            .collect::<Vec<_>>(),
    );
    let mut answers = streams.into_iter();
    inputs
        .iter()
        .map(|input| {
            let p = &input.program;
            if !applies(p) {
                return Verdict::Skipped;
            }
            let streamed = match answers.next().expect("a stream per program") {
                Ok(s) => s,
                Err(e) => {
                    return Verdict::Broken {
                        why: format!("threw: {e}"),
                        exception: None,
                    };
                }
            };
            let all = statements(&p.input);
            let received: Vec<usize> = p
                .sizes
                .iter()
                .scan(0, |sum, &n| {
                    *sum += n;
                    Some(*sum)
                })
                .collect();
            // How many times the parser sees each name defined at each push:
            // the text so far, closed as the stream parser closes its tail.
            let counts: Vec<Vec<String>> = received
                .iter()
                .map(|&n| {
                    let text: String = p.input.chars().take(n).collect();
                    split(&tokenize(&auto_close(&text).0))
                        .into_iter()
                        .map(|s| s.id)
                        .collect()
                })
                .collect();
            let defined = |step: usize, name: &str| counts[step].iter().filter(|n| *n == name).count();
            let results: Vec<Value> = streamed
                .steps
                .iter()
                .map(|json| parse_json(json).unwrap_or(Value::Null))
                .collect();
            let text = |n: usize| p.input.chars().take(n).collect::<String>();
            // Each statement is checked from the first push it's finished at.
            let mut checked: Vec<&str> = Vec::new();
            for i in 0..received.len() {
                let complete: Vec<&Statement> = all
                    .iter()
                    .filter(|s| s.complete_at.is_some_and(|at| at <= received[i]))
                    .collect();
                for s in &complete {
                    if checked.contains(&s.name.as_str()) {
                        continue;
                    }
                    let Some(reached) = fully_resolved(&s.name, &complete) else {
                        continue;
                    };
                    // Nothing it reaches is being defined again: a pending
                    // redefinition takes over once it no longer needs closing
                    // (language.md, section 4, rule 3).
                    let redefining = reached.iter().any(|name| {
                        defined(i, name) > complete.iter().filter(|t| t.name == *name).count()
                    });
                    if redefining {
                        continue;
                    }
                    let Some(shown) = elements_of(&results[i], &s.name).first().map(|e| (*e).clone()) else {
                        continue;
                    };
                    checked.push(&s.name);
                    for j in i + 1..received.len() {
                        if reached.iter().any(|name| defined(j, name) > defined(i, name)) {
                            break;
                        }
                        for later in elements_of(&results[j], &s.name) {
                            if let Some(d) = first_difference(&shown, later, "") {
                                // Whether a result is partial is decided for the
                                // whole result: while a later statement is
                                // open, finished elements are marked partial
                                // and their props checked leniently.
                                let flipped = results[i]["meta"]["incomplete"]
                                    != results[j]["meta"]["incomplete"];
                                let exception = flipped
                                    .then_some("quirk 17: a finished statement treated as partial");
                                return Verdict::Broken {
                                    why: format!(
                                        "`{}` finished at push {} ({:?}), changed at push {} ({:?}): {d}",
                                        s.name,
                                        i + 1,
                                        text(received[i]),
                                        j + 1,
                                        text(received[j])
                                    ),
                                    exception,
                                };
                            }
                        }
                    }
                }
            }
            Verdict::Holds
        })
        .collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

/// Each exception against programs the invariants found broken on both
/// parsers (seed 82, against lang-core 1.1), run on the core, which matches
/// the reference on them; and near misses the exception mustn't explain.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Core;
    use crate::{load_corpus, repo_root};

    fn input(lib: &str, program: &str, patch: &str) -> Input {
        Input {
            program: Program {
                lib: lib.to_owned(),
                input: program.to_owned(),
                root: None,
                sizes: vec![1; program.chars().count()],
            },
            patch: patch.to_owned(),
        }
    }

    fn verdict(rule: &str, input: &Input) -> Verdict {
        let corpus = load_corpus(&repo_root().join("conformance"));
        let mut core = Core {
            libraries: &corpus.libraries,
        };
        let rule = RULES.iter().find(|r| r.name == rule).expect("a rule");
        (rule.check)(&mut core, &[input]).remove(0)
    }

    fn exception(verdict: &Verdict) -> Option<&'static str> {
        match verdict {
            Verdict::Broken { exception, .. } => *exception,
            other => panic!("expected the rule broken, got {other:?}"),
        }
    }

    fn is(found: Option<&str>, quirk: &str) -> bool {
        found.is_some_and(|q| q.starts_with(&format!("{quirk}:")))
    }

    #[test]
    fn a_reference_cycle_breaks_the_round_trip_as_quirk_21() {
        let program = "root=Root([x y])\nx=[y]\ny=Root([x])";
        let found = exception(&verdict("round-trip", &input("hand", program, "")));
        assert!(is(found, "quirk 21"), "{found:?}");
    }

    #[test]
    fn a_reference_cycle_breaks_merging_its_own_source_as_quirk_21() {
        let program = "root=Root([x y])\nx=Root([y])\ny=([x])";
        let found = exception(&verdict("merge-own-source", &input("hand", program, "")));
        assert!(is(found, "quirk 21"), "{found:?}");
    }

    #[test]
    fn a_patch_with_an_open_ternary_breaks_merging_twice_as_quirk_22() {
        let patch = "Here's the change:\n\n```openui-lang\nroot = $dark\n  ? darkPage\n```\n\nAnything else?";
        let found = exception(&verdict(
            "merge-twice",
            &input("spec-1.0", "root=a\ndarkPage=r", patch),
        ));
        assert!(is(found, "quirk 22"), "{found:?}");
    }

    #[test]
    fn a_program_with_an_open_ternary_breaks_merging_twice_as_quirk_22() {
        let patch = "$n = null\nroot = Stack([Text(s)])\r\r\n$n = 0\r\r\n$n = 0\r\r";
        let found = exception(&verdict(
            "merge-twice",
            &input("spec-1.0", "root=()\ns=0?", patch),
        ));
        assert!(is(found, "quirk 22"), "{found:?}");
    }

    #[test]
    fn an_empty_redefinition_after_a_stray_bracket_is_quirk_4_at_the_stream_end() {
        let program = "[\na=(\"\")\na=";
        let found = exception(&verdict("stream-end", &input("hand", program, "")));
        assert!(is(found, "quirk 4"), "{found:?}");
    }

    /// `y` as an element: in one place reached through the cycle (children
    /// `[]`), in the other around it (`[[]]`).
    fn y(children: Value) -> Value {
        json!({"type": "element", "typeName": "Root", "props": {"children": children},
               "partial": false, "hasDynamicProps": false, "statementId": "y"})
    }

    #[test]
    fn quirk_21_needs_the_cycle_and_a_statement_written_two_ways() {
        let why = "/props/children/1/props/children: [[]] before, [] after";
        let two_ways = json!({"type": "element", "typeName": "Root", "statementId": "root",
            "props": {"children": [[y(json!([]))], y(json!([[]]))]}});
        let one_way = json!({"type": "element", "typeName": "Root", "statementId": "root",
            "props": {"children": [[y(json!([[]]))], y(json!([[]]))]}});
        let cyclic = "root=Root([x y])\nx=[y]\ny=Root([x])";
        let acyclic = "root=Root([x y])\nx=[y]\ny=Root([z])\nz=[]";
        let quirk = |input, tree| round_trip_exception(input, tree, why);
        assert!(is(quirk(cyclic, &two_ways), "quirk 21"));
        assert_eq!(quirk(cyclic, &one_way), None);
        assert_eq!(quirk(acyclic, &two_ways), None);
    }

    #[test]
    fn quirk_22_needs_a_ternary_left_open() {
        let finished = input(
            "spec-1.0",
            "root=a\ndarkPage=r",
            "root = $dark\n  ? darkPage\n  : lightPage",
        );
        assert_eq!(
            merge_exception(
                &finished,
                Some("root = $dark\n  ? darkPage\n  : lightPage\ndarkPage=r")
            ),
            None
        );
        // A `?` and `:` inside brackets are an object's or a call's, not a
        // ternary of the statement's.
        let bracketed = input("spec-1.0", "root=Stack([a ? b : c])\na=1", "x = f({k: 1})");
        assert_eq!(merge_exception(&bracketed, None), None);
    }

    #[test]
    fn quirk_4_at_the_stream_end_needs_the_empty_line_to_redefine() {
        let why = "the stream's end against one-shot: /meta/statementCount: 1 before, 2 after";
        assert_eq!(stream_exception("[\na=1\nb=", why), None);
        assert!(is(stream_exception("[\na=1\na=", why), "quirk 4"));
    }

    #[test]
    fn a_sort_whose_comparison_loops_is_explained_narrowly() {
        let explained = crate::sorts_values_whose_comparison_loops;
        assert!(explained("$s = @Sort([2, \"10\", \"1a\", 9], \"\")"));
        assert!(explained("$s=@Sort([2 \"10\" \"1a\""));
        // No text that starts with a digit: nothing for the loop to go through.
        assert!(!explained("$s = @Sort([2, \"10\", \"x\"], \"\")"));
        // Fewer than two numbers.
        assert!(!explained("$s = @Sort([\"1a\", \"x\"], \"\")"));
        // Not a sort, or not in a state default.
        assert!(!explained("$s = [2, \"10\", \"1a\"]"));
        assert!(!explained(
            "root = Root([@Sort([2, \"10\", \"1a\"], \"\")])"
        ));
    }

    #[test]
    fn a_constructor_turned_into_text_is_explained_narrowly() {
        let explained = crate::turns_a_constructor_into_text;
        assert!(explained("$s = (1).constructor + \"\""));
        assert!(explained("$s=true.constructor+\""));
        assert!(!explained("$s = 1 + \"\""));
        assert!(!explained("root = Root([Text(\"constructor\")])"));
    }
}
