//! Invariants (`--invariants`): rules a result should keep whatever the input,
//! each written once against the engine interface and checked on the core and
//! the reference alike, over the fuzzer's programs.
//!
//! A program that breaks a rule on both parsers is a candidate quirk: the
//! maintainer judges whether the reference or the rule is wrong. A program
//! that breaks it on one parser only is a mismatch, since the two disagree.
//! A break a known quirk causes is that rule's exception: the rule names the
//! quirk from what it sees, and it's counted, not reported. So is a
//! disagreement one of CONFORMANCE.md's divergences explains.

use std::collections::BTreeMap;

use openui_lang::{parse_json, stringify_json};
use serde_json::{Value, json};

use crate::engine::{Core, Engine, ParseCall, Ref, SerializeCall};
use crate::findings::run;
use crate::reference::Reference;
use crate::{Corpus, EXPLAINED, MAX_SAVES, Program, program};

/// What a rule makes of one program on one parser.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Holds,
    /// The rule doesn't apply (no tree to round-trip, say).
    Skipped,
    /// Broken: how, and the known quirk that explains it, if one does.
    Broken {
        why: String,
        exception: Option<&'static str>,
    },
}

pub struct Rule {
    pub name: &'static str,
    pub check: fn(&mut dyn Engine, &[&Program]) -> Vec<Verdict>,
}

pub const RULES: [Rule; 1] = [Rule {
    name: "round-trip",
    check: round_trip,
}];

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
    if has_node_shaped_literal(input) {
        return Some("an object literal shaped like a tree node");
    }
    None
}

/// **Round trip:** parsing, converting the tree back to source and parsing
/// that again gives the same tree. It applies to complete programs with a
/// tree, short of what [`beyond_source`] lists and of numbers JSON can't hold
/// ([`has_unjsonable_number`]): while a program is still arriving, values
/// that are still being written are let through that the finished program
/// would reject.
fn round_trip(engine: &mut dyn Engine, programs: &[&Program]) -> Vec<Verdict> {
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
        .zip(programs)
        .map(|(answer, p)| {
            let result = parse_json(answer.as_ref().ok()?).ok()?;
            if result["meta"]["incomplete"] != json!(false) || has_unjsonable_number(&p.input) {
                return None;
            }
            let root = result
                .get("root")
                .filter(|r| !r.is_null() && !beyond_source(r))?;
            let element = stringify_json(root);
            let state = stringify_json(&result["stateDeclarations"]);
            Some((root.clone(), element, state))
        })
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

// ─── Running them ───────────────────────────────────────────────────────────

/// How a program fares under a rule on the two parsers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Fine,
    /// Broken on both, the way a known quirk explains.
    Explained(&'static str),
    /// Broken on both, unexplained: a candidate quirk.
    Both,
    /// Broken on one parser only: the parsers disagree.
    Mismatch,
}

fn classify(core: &Verdict, reference: &Verdict) -> Class {
    match (core, reference) {
        (Verdict::Broken { exception: a, .. }, Verdict::Broken { exception: b, .. }) => {
            match (a, b) {
                (Some(a), Some(b)) if a == b => Class::Explained(a),
                _ => Class::Both,
            }
        }
        (Verdict::Broken { .. }, _) | (_, Verdict::Broken { .. }) => Class::Mismatch,
        _ => Class::Fine,
    }
}

fn verdicts(
    rule: &Rule,
    corpus: &Corpus,
    reference: &mut Reference,
    programs: &[&Program],
) -> (Vec<Verdict>, Vec<Verdict>) {
    let core = (rule.check)(
        &mut Core {
            libraries: &corpus.libraries,
        },
        programs,
    );
    let refs = (rule.check)(&mut Ref { reference }, programs);
    (core, refs)
}

/// The smallest program, by removing lines and then ever smaller runs of
/// chars, that the rule still classifies the same way. True if it finished
/// before the run's time was up.
fn shrink(
    rule: &Rule,
    p: &Program,
    class: Class,
    corpus: &Corpus,
    reference: &mut Reference,
) -> (Program, bool) {
    let same = |candidate: &Program, reference: &mut Reference| {
        let (c, r) = verdicts(rule, corpus, reference, &[candidate]);
        classify(&c[0], &r[0]) == class
    };
    let with = |input: String| Program { input, ..p.clone() };
    let mut best = p.clone();
    loop {
        if run().out_of_time() {
            return (best, false);
        }
        let lines: Vec<&str> = best.input.split('\n').collect();
        let fewer = (0..lines.len()).find_map(|i| {
            let mut kept = lines.clone();
            kept.remove(i);
            let candidate = with(kept.join("\n"));
            same(&candidate, reference).then_some(candidate)
        });
        if let Some(candidate) = fewer {
            best = candidate;
            continue;
        }
        let chars: Vec<char> = best.input.chars().collect();
        let mut cut = None;
        let mut size = chars.len() / 2;
        while size > 0 && cut.is_none() {
            cut = (0..=chars.len() - size).step_by(size).find_map(|at| {
                let candidate = with(chars[..at].iter().chain(&chars[at + size..]).collect());
                same(&candidate, reference).then_some(candidate)
            });
            size /= 2;
        }
        match cut {
            Some(candidate) => best = candidate,
            None => return (best, true),
        }
    }
}

#[derive(Default)]
struct Tally {
    inputs: usize,
    holds: usize,
    skipped: usize,
    explained: BTreeMap<&'static str, usize>,
    both: usize,
    mismatch: usize,
}

pub fn run_all(
    seed: u64,
    batches: impl Iterator<Item = Vec<u64>>,
    corpus: &Corpus,
    reference: &mut Reference,
) {
    let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
    // Shrunk finds so far, so a repeat isn't reported twice; each kind has
    // its own cap, so candidate quirks can't crowd out mismatches.
    let mut found: Vec<(&str, Class, String)> = Vec::new();
    for batch in batches {
        run().progress(
            "invariants",
            &format!(
                "scripts/fuzz.sh --invariants --seed {seed} --from {} --count {} --no-save",
                batch[0],
                batch.len()
            ),
        );
        let programs: Vec<Program> = batch.iter().map(|&k| program(seed, k, corpus)).collect();
        let refs: Vec<&Program> = programs.iter().collect();
        for rule in &RULES {
            let (core, reference_verdicts) = verdicts(rule, corpus, reference, &refs);
            for ((p, &k), (c, r)) in programs
                .iter()
                .zip(&batch)
                .zip(core.iter().zip(&reference_verdicts))
            {
                let tally = tallies.entry(rule.name).or_default();
                tally.inputs += 1;
                let mut class = classify(c, r);
                // The two parsers see different trees where CONFORMANCE.md
                // says they do, so they disagree, or break the rule apart.
                if matches!(class, Class::Mismatch | Class::Both)
                    && let Some((_, why)) =
                        EXPLAINED.iter().find(|(explains, _)| explains(&p.input))
                {
                    class = Class::Explained(why);
                }
                match class {
                    Class::Fine if *c == Verdict::Holds => tally.holds += 1,
                    Class::Fine => tally.skipped += 1,
                    Class::Explained(why) => *tally.explained.entry(why).or_default() += 1,
                    Class::Both => tally.both += 1,
                    Class::Mismatch => tally.mismatch += 1,
                }
                if !matches!(class, Class::Both | Class::Mismatch)
                    || found.iter().filter(|(_, c, _)| *c == class).count() >= MAX_SAVES
                {
                    continue;
                }
                let (small, complete) = shrink(rule, p, class, corpus, reference);
                if found
                    .iter()
                    .any(|(name, _, input)| *name == rule.name && *input == small.input)
                {
                    continue;
                }
                let (c, r) = verdicts(rule, corpus, reference, &[&small]);
                let detail = |v: &Verdict| match v {
                    Verdict::Broken { why, exception } => {
                        json!({"why": why, "exception": exception})
                    }
                    Verdict::Holds => json!("holds"),
                    Verdict::Skipped => json!("doesn't apply"),
                };
                println!(
                    "{} at seed {seed}, program {k}, rule {}, library {:?} ({} → {} chars): {:?}\n  core: {}\n  reference: {}",
                    if class == Class::Both {
                        "Broken on both"
                    } else {
                        "Mismatch"
                    },
                    rule.name,
                    small.lib,
                    p.input.chars().count(),
                    small.input.chars().count(),
                    small.input,
                    detail(&c[0]),
                    detail(&r[0]),
                );
                run().record(&json!({
                    "kind": if class == Class::Both { "quirk?" } else { "mismatch" },
                    "check": "invariant",
                    "rule": rule.name,
                    "seed": seed,
                    "index": k,
                    "replay": format!("scripts/fuzz.sh --invariants --seed {seed} --only {k} --no-save"),
                    "library": small.lib,
                    "root": small.root,
                    "chars": [p.input.chars().count(), small.input.chars().count()],
                    "shrunk": complete,
                    "input": small.input,
                    "core": detail(&c[0]),
                    "reference": detail(&r[0]),
                }));
                found.push((rule.name, class, small.input));
            }
        }
    }
    let mut summary = serde_json::Map::new();
    for (name, t) in &tallies {
        let explained: usize = t.explained.values().sum();
        println!(
            "invariants seed {seed}, {name}: {} programs, {} hold, {} don't apply, {} explained, {} broken on both, {} mismatches",
            t.inputs, t.holds, t.skipped, explained, t.both, t.mismatch
        );
        for (why, n) in &t.explained {
            println!("  explained by {why}: {n}");
        }
        summary.insert(
            (*name).to_owned(),
            // Broken on both counts the explained ones too, as the other
            // checks' summaries count known divergences among their finds.
            json!({"inputs": t.inputs, "holds": t.holds, "skipped": t.skipped, "explained": explained,
                   "exceptions": t.explained, "quirk?": t.both + explained, "mismatch": t.mismatch}),
        );
    }
    run().summary("invariants", &Value::Object(summary));
}
