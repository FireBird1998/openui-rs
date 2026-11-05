//! Differential fuzzing against the reference parser.
//!
//!   cargo run --release --example fuzz -- [--seed N] [--count N] [--from K] [--only K]
//!       [--seconds S] [--findings DIR] [--slow-ms N] [--no-save] [--edits | --invariants [--rule R]]
//!
//! Each program is generated from a component library's grammar or mangled
//! from a corpus case, then parsed by the core and by the reference (a
//! long-lived `bun reference/serve.ts`): one-shot, tokens, and a random push
//! chunking with the stream's end. Program K of seed S is the same on every machine (`--only K`
//! replays it). Mismatches are shrunk to a minimal program and saved as corpus
//! cases in `conformance/fuzz.json`; run `scripts/regen-golden.sh` afterwards.
//! With `--edits`, merge and serialization are fuzzed instead (see `edits.rs`),
//! and with `--invariants`, the rules in `invariants.rs` are checked on both
//! parsers.
//! The core taking longer than `--slow-ms` (default 1000) on an input is a
//! blow-up. Hunts add `--seconds` and `--findings` (see `findings.rs`).
//! Needs Bun.

mod edits;
mod engine;
mod findings;
mod generate;
mod invariants;
mod reference;
mod rng;
mod rules;

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use openui_lang::{Library, StreamParser, parse, tokenize, tokens_to_json};
use serde_json::{Value, json};

use findings::{Outcome, batches, run};
use generate::{Generator, mutate};
use reference::{Answer, Reference};
use rng::Rng;

pub(crate) const BATCH: usize = 200;

/// Whether a shrunk program shows a documented divergence, and which.
type Explains = (fn(&str) -> bool, &'static str);

/// Divergences CONFORMANCE.md documents: a minimal mismatch one of these
/// recognizes is explained, not a bug to save.
pub(crate) const EXPLAINED: [Explains; 7] = [
    (
        assigns_proto,
        "object or null assigned to __proto__ (prototype chain)",
    ),
    (
        root_is_an_element_shaped_object,
        "object literal shaped like an element as the root",
    ),
    (
        root_is_an_ast_shaped_object,
        "object literal shaped like an AST node as the root",
    ),
    (
        sorts_text_outside_ascii,
        "a state default sorting text outside ASCII (ICU collation)",
    ),
    (
        reads_a_property_of_a_builtin,
        "a state default reading a property of a built-in method or prototype",
    ),
    (
        ast_shaped_object_in_a_state_default,
        "object literal shaped like an AST node in a state default",
    ),
    (
        stringifies_an_own_to_string,
        "a state default turning an object with its own toString or valueOf into text",
    ),
];

/// JavaScript can't turn an object whose own `toString` (or `valueOf`) isn't
/// a function into text: in a state default (`+ ""`, `@Sort`, `==`) the
/// reference's `parse` throws a TypeError. The core writes "[object Object]".
fn stringifies_an_own_to_string(program: &str) -> bool {
    program.contains('$') && (program.contains("toString") || program.contains("valueOf"))
}

/// `@Sort` in a state default orders text with ICU's root collation; the
/// core has it exactly for ASCII only.
fn sorts_text_outside_ascii(program: &str) -> bool {
    program.contains("@Sort")
        && program.contains('$')
        && (!program.is_ascii() || program.contains("\\u"))
}

/// Names a value inherits as built-in methods, read as a step in a longer
/// path: `x.toString.length`, `[].map[0]`. The core reads such properties as
/// undefined.
fn reads_a_property_of_a_builtin(program: &str) -> bool {
    const INHERITED: [&str; 14] = [
        "constructor",
        "toString",
        "valueOf",
        "toLocaleString",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "map",
        "filter",
        "length",
        "at",
        "slice",
        "concat",
        "indexOf",
    ];
    program.contains('$')
        && INHERITED.iter().any(|name| {
            program.contains(&format!(".{name}.")) || program.contains(&format!(".{name}["))
        })
}

/// The reference evaluates such an object in a state default as the node it
/// looks like; the core keeps it as the object.
/// Shrunk, the key's colon or quotes may be gone (`$q={k"Ref`).
fn ast_shaped_object_in_a_state_default(program: &str) -> bool {
    program.contains('$')
        && program.contains('{')
        && program.contains('k')
        && crate::AST_KIND_NAMES
            .iter()
            .any(|kind| program.contains(&format!("\"{kind}")))
}

/// The AST kinds the reference recognizes in an object's `k`.
pub(crate) const AST_KIND_NAMES: [&str; 17] = [
    "Comp",
    "Str",
    "Num",
    "Bool",
    "Null",
    "Arr",
    "Obj",
    "Ref",
    "Ph",
    "StateRef",
    "RuntimeRef",
    "BinOp",
    "UnaryOp",
    "Ternary",
    "Member",
    "Index",
    "Assign",
];

/// Assigning an object or null to `__proto__` (in an object literal, or to a
/// param named `__proto__`) replaces the prototype, and the reference then
/// reads AST or element fields, and `in` checks, through the new prototype
/// chain. The core drops the assignment.
fn assigns_proto(program: &str) -> bool {
    program.contains("__proto__")
}

/// The reference returns such an object as the root; the core's root is
/// always a component's element. Shrunk, the program is that statement.
fn root_is_an_element_shaped_object(program: &str) -> bool {
    root_is_an_object(program) && program.contains("element")
}

/// The reference walks such an object as an AST node: it returns it as the
/// root expression when it holds a component call, and throws when a node in
/// it lacks the children its kind has (`root = {k: "Comp"}`). The core has no
/// root then. Shrunk, the program is that statement, and its kind string may
/// have lost its closing quote.
fn root_is_an_ast_shaped_object(program: &str) -> bool {
    const WITH_CHILDREN: [&str; 9] = [
        "\"Comp",
        "\"Arr",
        "\"Obj",
        "\"BinOp",
        "\"UnaryOp",
        "\"Ternary",
        "\"Member",
        "\"Index",
        "\"Assign",
    ];
    root_is_an_object(program) && WITH_CHILDREN.iter().any(|kind| program.contains(kind))
}

/// Whether the root's value is an object literal: the `root` statement's (or
/// the first statement's), or through a chain of references to one (`root =
/// c`, `c = {...}`). Other lines don't count: they may be prose, or taken in
/// by the object if it's unclosed.
fn root_is_an_object(program: &str) -> bool {
    let statements: Vec<(&str, &str)> = program
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            let named = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            named.then(|| (name, value.trim()))
        })
        .collect();
    let value_of = |name: &str| {
        statements
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
    };
    let mut value = value_of("root").or_else(|| statements.first().map(|(_, v)| *v));
    for _ in 0..10 {
        match value {
            Some(v) if v.starts_with('{') => return true,
            Some(v) if !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || c == '_') => {
                value = value_of(v);
            }
            _ => return false,
        }
    }
    false
}
pub(crate) const MAX_SAVES: usize = 25;

#[derive(Clone)]
pub struct Program {
    pub lib: String,
    pub input: String,
    pub root: Option<String>,
    /// Push chunk sizes in chars; empty means one-shot and tokens only.
    pub sizes: Vec<usize>,
}

pub(crate) struct Corpus {
    pub libraries: BTreeMap<String, Arc<Library>>,
    pub cases: Vec<(String, String, Option<String>)>, // (lib, input, root)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_corpus(conformance: &Path) -> Corpus {
    let mut libraries = BTreeMap::new();
    let mut cases = Vec::new();
    for file in [
        "manifest.json",
        "imported.json",
        "fuzz.json",
        "recorded.json",
    ] {
        let path = conformance.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let manifest: Value = serde_json::from_str(&text).unwrap();
        for (name, schema_file) in manifest["schemas"].as_object().unwrap() {
            let schema: Value = serde_json::from_str(
                &std::fs::read_to_string(conformance.join(schema_file.as_str().unwrap())).unwrap(),
            )
            .unwrap();
            libraries.insert(name.clone(), Arc::new(Library::from_json_schema(&schema)));
        }
        for case in manifest["cases"].as_array().unwrap() {
            let input =
                std::fs::read_to_string(conformance.join(case["file"].as_str().unwrap())).unwrap();
            let root = case.get("root").and_then(Value::as_str).map(str::to_owned);
            cases.push((case["schema"].as_str().unwrap().to_owned(), input, root));
        }
    }
    Corpus { libraries, cases }
}

/// Program `k` of the run with this seed.
pub(crate) fn program(seed: u64, k: u64, corpus: &Corpus) -> Program {
    let mut rng = Rng::for_program(seed, k);
    let (lib, input, root) = if rng.chance(50) {
        let names: Vec<&String> = corpus.libraries.keys().collect();
        let lib = (*rng.pick(&names)).clone();
        let input = Generator::new(&mut rng, &corpus.libraries[&lib]).program();
        let root = rng
            .chance(10)
            .then(|| rng.pick(&["Main", "Card", "header", "Missing"]).to_string());
        (lib, input, root)
    } else {
        let (lib, input, root) = rng.pick(&corpus.cases).clone();
        (lib, mutate(&mut rng, &input), root)
    };
    let n = input.chars().count();
    let mut sizes = Vec::new();
    let mut covered = 0;
    while covered < n {
        let size = (1 + rng.below(16)).min(n - covered);
        sizes.push(size);
        covered += size;
    }
    Program {
        lib,
        input,
        root,
        sizes,
    }
}

pub(crate) fn fingerprint(json: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in json.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// One-shot, tokens, then every push and the stream's end.
type Fingerprints = (String, String, Vec<String>);

/// The core's fingerprints for a program and how long they took, or None if
/// it panicked.
fn core_answer(p: &Program, library: &Arc<Library>) -> Option<(Fingerprints, Duration)> {
    let started = Instant::now();
    catch_unwind(AssertUnwindSafe(|| {
        let oneshot = fingerprint(&parse(&p.input, library, p.root.as_deref()).to_json());
        let tokens = fingerprint(&tokens_to_json(&tokenize(&p.input)));
        let mut parser = StreamParser::new(library.clone(), p.root.as_deref());
        let mut steps = Vec::new();
        let mut chars = p
            .input
            .char_indices()
            .map(|(i, _)| i)
            .chain([p.input.len()]);
        let mut start = chars.next().unwrap_or(0);
        for &size in &p.sizes {
            let end = chars.nth(size - 1).unwrap_or(p.input.len());
            steps.push(fingerprint(&parser.push(&p.input[start..end]).to_json()));
            start = end;
        }
        steps.push(fingerprint(&parser.finish().to_json()));
        (oneshot, tokens, steps)
    }))
    .ok()
    .map(|answer| (answer, started.elapsed()))
}

/// The outcome for a program, and how long the core took when it finished.
fn judge(p: &Program, answer: &Answer, corpus: &Corpus) -> (Outcome, Option<Duration>) {
    let Answer::Fingerprints {
        oneshot,
        tokens,
        steps,
        end,
        ..
    } = answer
    else {
        return (Outcome::ReferenceThrew, None);
    };
    let steps: Vec<&String> = steps.iter().chain([end]).collect();
    match core_answer(p, &corpus.libraries[&p.lib]) {
        None => (Outcome::CorePanicked, None),
        Some((_, took)) if took > run().slow => (Outcome::BlowUp, Some(took)),
        Some(((o, t, s), took))
            if &o == oneshot && &t == tokens && s.iter().eq(steps.iter().copied()) =>
        {
            (Outcome::Match, Some(took))
        }
        Some((_, took)) => (Outcome::Mismatch, Some(took)),
    }
}

fn outcome_of(p: &Program, reference: &mut Reference, corpus: &Corpus) -> Outcome {
    let answer = reference.ask(&[p]).pop().unwrap();
    judge(p, &answer, corpus).0
}

/// The smallest program (by removing lines, then ever smaller runs of chars)
/// with the same outcome. Streaming mismatches shrink under one-char pushes,
/// so any chunking reproduces them. Blow-ups aren't shrunk: each try could
/// take as long as the original. When the run's time is up, shrinking stops
/// and returns what it has; the flag says whether it finished.
fn shrink(
    p: &Program,
    outcome: Outcome,
    reference: &mut Reference,
    corpus: &Corpus,
) -> (Program, bool) {
    if outcome == Outcome::BlowUp {
        return (p.clone(), false);
    }
    let with_sizes = |input: String, streaming: bool| {
        let n = input.chars().count();
        Program {
            input,
            sizes: if streaming { vec![1; n] } else { Vec::new() },
            ..p.clone()
        }
    };
    let streaming = outcome_of(&with_sizes(p.input.clone(), false), reference, corpus) != outcome;
    if streaming && outcome_of(&with_sizes(p.input.clone(), true), reference, corpus) != outcome {
        return (p.clone(), true); // only this exact chunking reproduces it
    }
    let mut best = p.input.clone();
    let keeps = |text: &str, reference: &mut Reference| {
        outcome_of(&with_sizes(text.to_owned(), streaming), reference, corpus) == outcome
    };
    // Whole lines first.
    let mut i = 0;
    loop {
        let lines: Vec<&str> = best.split('\n').collect();
        if i >= lines.len() || lines.len() == 1 {
            break;
        }
        if run().out_of_time() {
            return (with_sizes(best, streaming), false);
        }
        let candidate: String = lines
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, l)| *l)
            .collect::<Vec<_>>()
            .join("\n");
        if keeps(&candidate, reference) {
            best = candidate;
        } else {
            i += 1;
        }
    }
    // Then runs of characters, halving the run length.
    let mut step = best.chars().count().div_ceil(2).max(1);
    loop {
        let mut start = 0;
        let mut progressed = false;
        loop {
            let chars: Vec<char> = best.chars().collect();
            if start >= chars.len() {
                break;
            }
            if run().out_of_time() {
                return (with_sizes(best, streaming), false);
            }
            let end = (start + step).min(chars.len());
            let candidate: String = chars[..start].iter().chain(&chars[end..]).collect();
            if keeps(&candidate, reference) {
                best = candidate;
                progressed = true;
            } else {
                start += step;
            }
        }
        if step == 1 && !progressed {
            break;
        }
        if !progressed {
            step = step.div_ceil(2);
        }
    }
    (with_sizes(best, streaming), true)
}

/// A saved case's name: the fingerprint of its library, root and program.
fn case_name(p: &Program) -> String {
    format!(
        "fuzz/{}",
        fingerprint(&format!(
            "{}\0{}\0{}",
            p.lib,
            p.root.as_deref().unwrap_or(""),
            p.input
        ))
    )
}

/// Add shrunk finds to `fuzz.json` in `dir` as corpus cases: the corpus, or a
/// hunt's findings folder.
fn save(found: &[(Program, String)], dir: &Path) -> Vec<String> {
    let manifest_path = dir.join("fuzz.json");
    let mut manifest: Value = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({ "schemas": {}, "cases": [] }));
    let mut saved = Vec::new();
    for (p, origin) in found {
        let name = case_name(p);
        let cases = manifest["cases"].as_array_mut().unwrap();
        if cases.iter().any(|c| c["name"] == name) {
            continue;
        }
        let file = format!("cases/{name}.oui");
        std::fs::create_dir_all(dir.join("cases/fuzz")).unwrap();
        std::fs::write(dir.join(&file), &p.input).unwrap();
        let mut case = json!({ "name": name, "schema": p.lib, "file": file, "source": "fuzz", "origin": origin });
        if let Some(root) = &p.root {
            case["root"] = json!(root);
        }
        cases.push(case);
        saved.push(name);
    }
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap() + "\n",
    )
    .unwrap();
    saved
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let flag = |name: &str| value(name).and_then(|v| v.parse::<u64>().ok());
    let seed = flag("--seed").unwrap_or(1);
    let save_cases = !args.iter().any(|a| a == "--no-save");
    findings::init(
        flag("--seconds"),
        flag("--slow-ms").unwrap_or(1000),
        value("--findings").map(PathBuf::from),
    );
    let batches = batches(
        flag("--only"),
        flag("--from").unwrap_or(0),
        flag("--count"),
        BATCH,
    );

    let root = repo_root();
    let conformance = root.join("conformance");
    let corpus = load_corpus(&conformance);
    let mut reference = Reference::start(&root.join("reference"));

    if args.iter().any(|a| a == "--invariants") {
        invariants::run_all(
            seed,
            batches,
            &corpus,
            &mut reference,
            value("--rule").map(String::as_str),
        );
        return;
    }
    if args.iter().any(|a| a == "--edits") {
        edits::run(
            seed,
            batches,
            &corpus,
            &mut reference,
            &conformance,
            save_cases,
        );
        return;
    }
    let mut tally: HashMap<Outcome, usize> = HashMap::new();
    let mut found: Vec<(Program, String)> = Vec::new();
    let mut threw: Vec<(Program, String)> = Vec::new();
    let mut explained = 0;
    let mut slowest = (Duration::ZERO, 0);
    for batch in batches {
        run().progress(
            "parse",
            &format!(
                "scripts/fuzz.sh --seed {seed} --from {} --count {} --no-save",
                batch[0],
                batch.len()
            ),
        );
        let programs: Vec<Program> = batch.iter().map(|&k| program(seed, k, &corpus)).collect();
        let answers = reference.ask(&programs.iter().collect::<Vec<_>>());
        for ((p, answer), &k) in programs.iter().zip(&answers).zip(batch.iter()) {
            let (outcome, took) = judge(p, answer, &corpus);
            *tally.entry(outcome).or_default() += 1;
            if let Some(took) = took
                && took > slowest.0
            {
                slowest = (took, k);
            }
            if outcome == Outcome::Match {
                continue;
            }
            let origin = format!("seed {seed}, program {k}");
            if found.len() + threw.len() >= MAX_SAVES {
                continue;
            }
            let (small, complete) = shrink(p, outcome, &mut reference, &corpus);
            if let Some((_, why)) = EXPLAINED
                .iter()
                .find(|(explains, _)| explains(&small.input))
            {
                explained += 1;
                println!("explained at {origin}: {why}: {:?}", small.input);
                continue;
            }
            println!(
                "{outcome:?} at {origin}, library {:?}{} ({} → {} chars, {}): {:?}",
                small.lib,
                small
                    .root
                    .as_ref()
                    .map(|r| format!(", root {r:?}"))
                    .unwrap_or_default(),
                p.input.chars().count(),
                small.input.chars().count(),
                if small.sizes.is_empty() {
                    "one-shot"
                } else {
                    "streaming"
                },
                small.input
            );
            let mut finding = json!({
                "kind": outcome.kind(),
                "check": "parse",
                "seed": seed,
                "index": k,
                "replay": format!("scripts/fuzz.sh --seed {seed} --only {k} --no-save"),
                "library": small.lib,
                "root": small.root,
                "streaming": !small.sizes.is_empty(),
                "chars": [p.input.chars().count(), small.input.chars().count()],
                "shrunk": complete,
                "input": small.input,
            });
            if let Answer::Fingerprints { ms, .. } = answer {
                finding["reference_ms"] = json!(ms);
            }
            if let Some(took) = took {
                finding["core_ms"] = json!(took.as_secs_f64() * 1000.0);
            }
            let same = |(f, _): &(Program, String)| f.input == small.input && f.lib == small.lib;
            if let (Outcome::ReferenceThrew, Answer::Threw(error)) = (outcome, answer) {
                println!("  reference threw: {error}");
                if threw.iter().any(same) {
                    continue;
                }
                finding["reference"] = json!(error);
                threw.push((small, origin));
            } else {
                if found.iter().any(same) {
                    continue;
                }
                finding["case"] = json!(case_name(&small));
                found.push((small, origin));
            }
            run().record(&finding);
        }
    }

    let total: usize = tally.values().sum();
    let count = |o| tally.get(&o).copied().unwrap_or(0);
    println!(
        "fuzz seed {seed}: {total} programs, {} match, {} mismatch ({explained} explained), {} core panics, {} blow-ups, {} reference exceptions",
        count(Outcome::Match),
        count(Outcome::Mismatch),
        count(Outcome::CorePanicked),
        count(Outcome::BlowUp),
        count(Outcome::ReferenceThrew),
    );
    println!(
        "slowest core answer: {:.1} ms (program {})",
        slowest.0.as_secs_f64() * 1000.0,
        slowest.1
    );
    run().summary(
        "parse",
        &json!({
            "inputs": total,
            "match": count(Outcome::Match),
            "mismatch": count(Outcome::Mismatch),
            "explained": explained,
            "crash": count(Outcome::CorePanicked),
            "blow-up": count(Outcome::BlowUp),
            "quirk?": count(Outcome::ReferenceThrew),
            "slowest_ms": slowest.0.as_secs_f64() * 1000.0,
            "slowest_index": slowest.1,
        }),
    );
    if save_cases && !found.is_empty() {
        let dir = run().save_dir(&conformance);
        let saved = save(&found, dir);
        println!(
            "saved {} new cases to {}",
            saved.len(),
            dir.join("fuzz.json").display()
        );
        if run().dir.is_none() {
            println!("run scripts/regen-golden.sh");
        }
    }
}
