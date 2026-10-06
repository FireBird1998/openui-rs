//! Differential fuzzing against the reference parser.
//!
//!   cargo run --release --example fuzz -- [--seed N] [--count N] [--only K] [--no-save]
//!
//! Each program is generated from a component library's grammar or mangled
//! from a corpus case, then parsed by the core and by the reference (a
//! long-lived `bun reference/serve.ts`): one-shot, tokens, and a random push
//! chunking. Program K of seed S is the same on every machine (`--only K`
//! replays it). Mismatches are shrunk to a minimal program and saved as corpus
//! cases in `conformance/fuzz.json`; run `scripts/regen-golden.sh` afterwards.
//! Needs Bun.

mod generate;
mod reference;
mod rng;

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openui_lang::{Library, StreamParser, parse, tokenize, tokens_to_json};
use serde_json::{Value, json};

use generate::{Generator, mutate};
use reference::{Answer, Reference};
use rng::Rng;

const BATCH: usize = 200;

/// Documented limitations: a minimal mismatch containing one of these is
/// explained, not a bug to save. Assigning an object or null to `__proto__`
/// (in an object literal, or to a param named `__proto__`) replaces the
/// prototype, and the reference then reads AST or element fields, and `in`
/// checks, through the new prototype chain. The core drops the assignment.
const EXPLAINED: [(&str, &str); 1] = [(
    "__proto__",
    "object or null assigned to __proto__ (prototype chain)",
)];
const MAX_SAVES: usize = 25;

#[derive(Clone)]
pub struct Program {
    pub lib: String,
    pub input: String,
    pub root: Option<String>,
    /// Push chunk sizes in chars; empty means one-shot and tokens only.
    pub sizes: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Outcome {
    Match,
    Mismatch,
    ReferenceThrew,
    CorePanicked,
}

struct Corpus {
    libraries: BTreeMap<String, Arc<Library>>,
    cases: Vec<(String, String, Option<String>)>, // (lib, input, root)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_corpus(conformance: &Path) -> Corpus {
    let mut libraries = BTreeMap::new();
    let mut cases = Vec::new();
    for file in ["manifest.json", "imported.json", "fuzz.json"] {
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
fn program(seed: u64, k: u64, corpus: &Corpus) -> Program {
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

fn fingerprint(json: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in json.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// The core's fingerprints for a program, or None if it panicked.
fn core_answer(p: &Program, library: &Arc<Library>) -> Option<(String, String, Vec<String>)> {
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
        (oneshot, tokens, steps)
    }))
    .ok()
}

fn judge(p: &Program, answer: &Answer, corpus: &Corpus) -> Outcome {
    let Answer::Fingerprints {
        oneshot,
        tokens,
        steps,
    } = answer
    else {
        return Outcome::ReferenceThrew;
    };
    match core_answer(p, &corpus.libraries[&p.lib]) {
        None => Outcome::CorePanicked,
        Some((o, t, s)) if &o == oneshot && &t == tokens && &s == steps => Outcome::Match,
        Some(_) => Outcome::Mismatch,
    }
}

fn outcome_of(p: &Program, reference: &mut Reference, corpus: &Corpus) -> Outcome {
    let answer = reference.ask(&[p]).pop().unwrap();
    judge(p, &answer, corpus)
}

/// The smallest program (by removing lines, then ever smaller runs of chars)
/// with the same outcome. Streaming mismatches shrink under one-char pushes,
/// so any chunking reproduces them.
fn shrink(p: &Program, outcome: Outcome, reference: &mut Reference, corpus: &Corpus) -> Program {
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
        return p.clone(); // only this exact chunking reproduces it
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
    let mut run = best.chars().count().div_ceil(2).max(1);
    loop {
        let mut start = 0;
        let mut progressed = false;
        loop {
            let chars: Vec<char> = best.chars().collect();
            if start >= chars.len() {
                break;
            }
            let end = (start + run).min(chars.len());
            let candidate: String = chars[..start].iter().chain(&chars[end..]).collect();
            if keeps(&candidate, reference) {
                best = candidate;
                progressed = true;
            } else {
                start += run;
            }
        }
        if run == 1 && !progressed {
            break;
        }
        if !progressed {
            run = run.div_ceil(2);
        }
    }
    with_sizes(best, streaming)
}

/// Add shrunk mismatches to `conformance/fuzz.json` as corpus cases.
fn save(found: &[(Program, String)], conformance: &Path) -> Vec<String> {
    let manifest_path = conformance.join("fuzz.json");
    let mut manifest: Value = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({ "schemas": {}, "cases": [] }));
    let mut saved = Vec::new();
    for (p, origin) in found {
        let name = format!(
            "fuzz/{}",
            fingerprint(&format!(
                "{}\0{}\0{}",
                p.lib,
                p.root.as_deref().unwrap_or(""),
                p.input
            ))
        );
        let cases = manifest["cases"].as_array_mut().unwrap();
        if cases.iter().any(|c| c["name"] == name) {
            continue;
        }
        let file = format!("cases/{name}.oui");
        std::fs::create_dir_all(conformance.join("cases/fuzz")).unwrap();
        std::fs::write(conformance.join(&file), &p.input).unwrap();
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
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let seed = flag("--seed").unwrap_or(1);
    let count = flag("--count").unwrap_or(1000);
    let only = flag("--only");
    let save_cases = !args.iter().any(|a| a == "--no-save");

    let root = repo_root();
    let conformance = root.join("conformance");
    let corpus = load_corpus(&conformance);
    let mut reference = Reference::start(&root.join("reference"));

    let indices: Vec<u64> = match only {
        Some(k) => vec![k],
        None => (0..count).collect(),
    };
    let mut tally: HashMap<Outcome, usize> = HashMap::new();
    let mut found: Vec<(Program, String)> = Vec::new();
    let mut threw: Vec<(Program, String)> = Vec::new();
    let mut explained = 0;
    for batch in indices.chunks(BATCH) {
        let programs: Vec<Program> = batch.iter().map(|&k| program(seed, k, &corpus)).collect();
        let answers = reference.ask(&programs.iter().collect::<Vec<_>>());
        for ((p, answer), &k) in programs.iter().zip(&answers).zip(batch) {
            let outcome = judge(p, answer, &corpus);
            *tally.entry(outcome).or_default() += 1;
            if outcome == Outcome::Match {
                continue;
            }
            let origin = format!("seed {seed}, program {k}");
            if found.len() + threw.len() >= MAX_SAVES {
                continue;
            }
            let small = shrink(p, outcome, &mut reference, &corpus);
            if let Some((_, why)) = EXPLAINED
                .iter()
                .find(|(pattern, _)| small.input.contains(pattern))
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
            if let (Outcome::ReferenceThrew, Answer::Threw(error)) = (outcome, answer) {
                println!("  reference threw: {error}");
                threw.push((small, origin));
            } else if !found
                .iter()
                .any(|(f, _)| f.input == small.input && f.lib == small.lib)
            {
                found.push((small, origin));
            }
        }
    }

    let total: usize = tally.values().sum();
    println!(
        "fuzz seed {seed}: {total} programs, {} match, {} mismatch ({explained} explained), {} core panics, {} reference exceptions",
        tally.get(&Outcome::Match).unwrap_or(&0),
        tally.get(&Outcome::Mismatch).unwrap_or(&0),
        tally.get(&Outcome::CorePanicked).unwrap_or(&0),
        tally.get(&Outcome::ReferenceThrew).unwrap_or(&0),
    );
    if save_cases && !found.is_empty() {
        let saved = save(&found, &conformance);
        println!(
            "saved {} new cases to conformance/fuzz.json; run scripts/regen-golden.sh",
            saved.len()
        );
    }
}
