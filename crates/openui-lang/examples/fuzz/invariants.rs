//! Invariants (`--invariants`): rules a result should keep whatever the input,
//! each written once against the engine interface (`engine.rs`) and checked on
//! the core and the reference alike, over the fuzzer's programs. The rules
//! are in `rules.rs`.
//!
//! A program that breaks a rule on both parsers is a candidate quirk: the
//! maintainer judges whether the reference or the rule is wrong. A program
//! that breaks it on one parser only is a mismatch, since the two disagree.
//! A break a known quirk causes is that rule's exception: the rule names the
//! quirk from what it sees, and it's counted, not reported. So is a
//! disagreement one of CONFORMANCE.md's divergences explains.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::engine::{Core, Engine, Ref};
use crate::findings::run;
use crate::reference::Reference;
use crate::rng::Rng;
use crate::rules::RULES;
use crate::{Corpus, EXPLAINED, MAX_SAVES, Program, edits, program};

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

/// A program, and a patch for the rules that merge.
#[derive(Clone)]
pub struct Input {
    pub program: Program,
    pub patch: String,
}

/// Programs whose result passes this many bytes of JSON (as the core
/// measures it) are left out: the reference's results double with them, and
/// it runs out of memory (a divergence in CONFORMANCE.md).
const RESULT_LIMIT: usize = 1 << 20;

/// Whether a program's result is past [`RESULT_LIMIT`].
fn too_big(p: &Program, corpus: &Corpus) -> bool {
    let result = openui_lang::parse(&p.input, &corpus.libraries[&p.lib], p.root.as_deref());
    result.size_limited || result.to_json().len() > RESULT_LIMIT
}

pub struct Rule {
    pub name: &'static str,
    pub check: fn(&mut dyn Engine, &[&Input]) -> Vec<Verdict>,
}

/// Input `k` of a run: the differential fuzzer's program `k`, and a patch
/// for it from the edit fuzzer's generator, on a stream of its own.
fn input(seed: u64, k: u64, corpus: &Corpus) -> Input {
    let program = program(seed, k, corpus);
    let mut rng = Rng::for_program(seed ^ 0x7061_7463_6865_7321, k);
    let patch = edits::patch_for(&mut rng, corpus, &program.lib, &program.input);
    Input { program, patch }
}

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

fn classify(core: &Verdict, reference: &Verdict, input: &Input) -> Class {
    let class = match (core, reference) {
        (Verdict::Broken { exception: a, .. }, Verdict::Broken { exception: b, .. }) => {
            match (a, b) {
                (Some(a), Some(b)) if a == b => Class::Explained(a),
                _ => Class::Both,
            }
        }
        (Verdict::Broken { .. }, _) | (_, Verdict::Broken { .. }) => Class::Mismatch,
        _ => Class::Fine,
    };
    // The two parsers see different trees where CONFORMANCE.md says they
    // do, so they disagree, or break the rule apart.
    match class {
        Class::Mismatch | Class::Both => EXPLAINED
            .iter()
            .find(|(explains, _)| explains(&input.program.input) || explains(&input.patch))
            .map_or(class, |(_, why)| Class::Explained(why)),
        _ => class,
    }
}

fn verdicts(
    rule: &Rule,
    corpus: &Corpus,
    reference: &mut Reference,
    inputs: &[&Input],
) -> (Vec<Verdict>, Vec<Verdict>) {
    let core = (rule.check)(
        &mut Core {
            libraries: &corpus.libraries,
        },
        inputs,
    );
    let refs = (rule.check)(&mut Ref { reference }, inputs);
    (core, refs)
}

/// How long one finding may take to shrink.
const SHRINK_SECONDS: u64 = 15;

/// The smallest program, by removing lines and then ever smaller runs of
/// chars, that the rule still classifies the same way (the patch stays as
/// it is). True if it finished before the run's time, or its own
/// [`SHRINK_SECONDS`], was up.
fn shrink(
    rule: &Rule,
    start: &Input,
    class: Class,
    corpus: &Corpus,
    reference: &mut Reference,
) -> (Input, bool) {
    let started = std::time::Instant::now();
    let same = |candidate: &Input, reference: &mut Reference| {
        let core = (rule.check)(
            &mut Core {
                libraries: &corpus.libraries,
            },
            &[candidate],
        );
        // Broken on both needs the core broken: no need to ask the reference.
        if class == Class::Both && !matches!(core[0], Verdict::Broken { .. }) {
            return false;
        }
        let r = (rule.check)(&mut Ref { reference }, &[candidate]);
        classify(&core[0], &r[0], candidate) == class
    };
    let with = |text: String| {
        let mut candidate = start.clone();
        candidate.program.input = text;
        // A stream's chunks are cut again, at most 16 code points each.
        let n = candidate.program.input.chars().count();
        candidate.program.sizes = (0..n.div_ceil(16)).map(|i| 16.min(n - i * 16)).collect();
        candidate
    };
    let mut best = start.clone();
    loop {
        if run().out_of_time() || started.elapsed().as_secs() >= SHRINK_SECONDS {
            return (best, false);
        }
        let lines: Vec<&str> = best.program.input.split('\n').collect();
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
        let chars: Vec<char> = best.program.input.chars().collect();
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
    /// Time spent checking, both parsers, shrinking aside.
    seconds: f64,
}

/// Checks every rule, or with `only`, the rules of that name.
pub fn run_all(
    seed: u64,
    batches: impl Iterator<Item = Vec<u64>>,
    corpus: &Corpus,
    reference: &mut Reference,
    only: Option<&str>,
) {
    let rules: Vec<&Rule> = RULES
        .iter()
        .filter(|r| only.is_none_or(|o| o == r.name))
        .collect();
    assert!(!rules.is_empty(), "no rule is called {only:?}");
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
        let (inputs, batch): (Vec<Input>, Vec<u64>) = batch
            .iter()
            .map(|&k| (input(seed, k, corpus), k))
            .filter(|(i, _)| {
                let big = too_big(&i.program, corpus);
                if big {
                    for &rule in &rules {
                        let tally = tallies.entry(rule.name).or_default();
                        tally.inputs += 1;
                        tally.skipped += 1;
                    }
                }
                !big
            })
            .unzip();
        let refs: Vec<&Input> = inputs.iter().collect();
        for &rule in &rules {
            let started = std::time::Instant::now();
            let (core, reference_verdicts) = verdicts(rule, corpus, reference, &refs);
            tallies.entry(rule.name).or_default().seconds += started.elapsed().as_secs_f64();
            for ((input, &k), (c, r)) in inputs
                .iter()
                .zip(&batch)
                .zip(core.iter().zip(&reference_verdicts))
            {
                let tally = tallies.entry(rule.name).or_default();
                tally.inputs += 1;
                let class = classify(c, r, input);
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
                let (small, complete) = shrink(rule, input, class, corpus, reference);
                let key = format!("{}\u{0}{}", small.program.input, small.patch);
                if found
                    .iter()
                    .any(|(name, _, text)| *name == rule.name && *text == key)
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
                let p = &small.program;
                println!(
                    "{} at seed {seed}, program {k}, rule {}, library {:?} ({} → {} chars): {:?}{}\n  core: {}\n  reference: {}",
                    if class == Class::Both {
                        "Broken on both"
                    } else {
                        "Mismatch"
                    },
                    rule.name,
                    p.lib,
                    input.program.input.chars().count(),
                    p.input.chars().count(),
                    p.input,
                    if rule.name.starts_with("merge") {
                        format!(", patch {:?}", small.patch)
                    } else {
                        String::new()
                    },
                    detail(&c[0]),
                    detail(&r[0]),
                );
                let mut finding = json!({
                    "kind": if class == Class::Both { "quirk?" } else { "mismatch" },
                    "check": "invariant",
                    "rule": rule.name,
                    "seed": seed,
                    "index": k,
                    "replay": format!("scripts/fuzz.sh --invariants --seed {seed} --only {k} --no-save"),
                    "library": p.lib,
                    "root": p.root,
                    "chars": [input.program.input.chars().count(), p.input.chars().count()],
                    "shrunk": complete,
                    "input": p.input,
                    "core": detail(&c[0]),
                    "reference": detail(&r[0]),
                });
                if rule.name.starts_with("merge") {
                    finding["patch"] = json!(small.patch);
                }
                run().record(&finding);
                found.push((rule.name, class, key));
            }
        }
    }
    let mut summary = serde_json::Map::new();
    for (name, t) in &tallies {
        let explained: usize = t.explained.values().sum();
        println!(
            "invariants seed {seed}, {name}: {} programs, {} hold, {} don't apply, {} explained, {} broken on both, {} mismatches ({:.0} s)",
            t.inputs, t.holds, t.skipped, explained, t.both, t.mismatch, t.seconds
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
