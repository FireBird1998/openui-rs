//! Conformance with the reference parser: for every corpus case, the one-shot
//! parse result, the result after every streaming push, and the result once
//! each stream has ended must serialize to exactly the JSON the reference
//! parser produced (see `conformance/golden/`,
//! written by `scripts/regen-golden.sh`). Needs no Bun unless a streaming step
//! mismatches, and then only to show the reference JSON for that step.
//!
//! Cases listed in `conformance/pending/parse.txt` aren't supported yet: they
//! may fail, but the suite fails if one starts passing (so it gets removed from
//! the list). `UPDATE_PENDING=1 cargo test --test conformance` rewrites the list
//! to exactly the cases failing now.

mod common;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use common::{
    Case, Pending, diff, explain as explain_step, fingerprint, golden_dir, golden_json,
    load_corpus, read, update_pending,
};
use openui_lang::{StreamParser, parse};

/// Split into consecutive chunks of the given sizes in chars, like slicing
/// `Array.from(text)` in the runner.
fn chunks<'a>(text: &'a str, sizes: &[usize]) -> Vec<&'a str> {
    let mut bounds = text.char_indices().map(|(i, _)| i).chain([text.len()]);
    let mut start = bounds.next().unwrap_or(0);
    sizes
        .iter()
        .map(|&size| {
            let end = bounds.nth(size - 1).unwrap_or(text.len());
            let part = &text[start..end];
            start = end;
            part
        })
        .collect()
}

/// Every check for one case; returns a report of each mismatch (empty when the
/// case conforms). `explain` fetches reference JSON for readable diffs.
fn check_case(case: &Case, chunkings: &[String], golden: &Path, explain: bool) -> String {
    let golden_steps = golden_json(&case.name, "steps.json");
    let mut failures = String::new();
    let root = case.root.as_deref();

    let expected = read(&golden.join(format!("{}.oneshot.json", case.name)));
    let actual = parse(&case.input, &case.library, root).to_json();
    if actual != expected {
        let _ = write!(failures, "  one-shot parse\n{}", diff(&expected, &actual));
    }

    for chunking in chunkings {
        let golden = &golden_steps[chunking];
        if let Some(ops) = golden.get("ops") {
            check_resync(case, chunking, ops, golden, explain, &mut failures);
            continue;
        }
        let expected: Vec<&str> = golden["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let sizes: Vec<usize> = match golden.get("sizes") {
            Some(sizes) => sizes
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_u64().unwrap() as usize)
                .collect(),
            None => {
                let size: usize = chunking.parse().unwrap();
                let n = case.input.chars().count();
                (0..n)
                    .step_by(size)
                    .map(|start| size.min(n - start))
                    .collect()
            }
        };
        let parts = chunks(&case.input, &sizes);
        // Every push, then the stream's end.
        if parts.len() + 1 != expected.len() {
            let _ = writeln!(
                failures,
                "  chunking {chunking}: {} pushes and the end, reference has {} steps",
                parts.len(),
                expected.len()
            );
            continue;
        }
        let mut parser = StreamParser::new(case.library.clone(), root);
        let mut so_far = String::new();
        for step in 0..expected.len() {
            let actual = match parts.get(step) {
                Some(part) => {
                    so_far.push_str(part);
                    parser.push(part).to_json()
                }
                None => parser.finish().to_json(),
            };
            if fingerprint(&actual) == expected[step] {
                continue;
            }
            let detail = if !explain {
                String::new()
            } else {
                explain_step(
                    &["show", &case.name, chunking, &(step + 1).to_string()],
                    &actual,
                )
            };
            let at = if step < parts.len() {
                format!("input so far: {so_far:?}")
            } else {
                "the stream's end".to_owned()
            };
            let _ = write!(
                failures,
                "  chunking {chunking}, step {} of {} ({at})\n{detail}",
                step + 1,
                expected.len()
            );
            break; // later steps of this stream usually repeat the same divergence
        }
    }
    failures
}

/// The program with its middle character changed; mirrors `edited` in the runner.
fn edited(input: &str) -> String {
    let mut chars: Vec<char> = input.chars().collect();
    let m = chars.len() / 2;
    if let Some(c) = chars.get_mut(m) {
        *c = if *c == 'Z' { 'Y' } else { 'Z' };
    }
    chars.into_iter().collect()
}

/// The first `n` chars of `text`.
fn prefix(text: &str, n: usize) -> &str {
    &text[..text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)]
}

/// A resync chunking: `set` with the program's (`p`) or the edited copy's
/// (`x`) first n chars, then `result` (`r`) and the stream's end (`e`).
fn check_resync(
    case: &Case,
    chunking: &str,
    ops: &serde_json::Value,
    golden: &serde_json::Value,
    explain: bool,
    failures: &mut String,
) {
    let other = edited(&case.input);
    let expected = golden["steps"].as_array().unwrap();
    let mut parser = StreamParser::new(case.library.clone(), case.root.as_deref());
    for (step, op) in ops.as_array().unwrap().iter().enumerate() {
        let kind = op[0].as_str().unwrap();
        let n = op[1].as_u64().unwrap() as usize;
        let (actual, call) = match kind {
            "r" => (parser.result().to_json(), "result()".to_owned()),
            "e" => (parser.finish().to_json(), "finish()".to_owned()),
            _ => {
                let text = prefix(if kind == "p" { &case.input } else { &other }, n);
                let call = format!(
                    "set({} chars of the {})",
                    n,
                    if kind == "p" {
                        "program"
                    } else {
                        "edited copy"
                    }
                );
                (parser.set(text).to_json(), call)
            }
        };
        if fingerprint(&actual) == expected[step].as_str().unwrap() {
            continue;
        }
        let detail = if explain {
            explain_step(
                &["show", &case.name, chunking, &(step + 1).to_string()],
                &actual,
            )
        } else {
            String::new()
        };
        let _ = write!(
            failures,
            "  chunking {chunking}, step {} of {}: {call}\n{detail}",
            step + 1,
            expected.len()
        );
        return;
    }
}

#[test]
fn every_case_matches_the_reference_parser() {
    let golden = golden_dir();
    let corpus = load_corpus();
    let pending = Pending::load("parse");

    // Fetching reference JSON runs Bun once per mismatch; enough to start on.
    let mut explanations_left = 5;
    let mut failing = BTreeSet::new();
    let mut report = String::new();
    for case in &corpus.cases {
        let explain = !pending.contains(&case.name) && !update_pending() && explanations_left > 0;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            check_case(case, &case.chunkings(&corpus.chunkings), &golden, explain)
        }));
        let failures = match outcome {
            Ok(failures) => failures,
            Err(panic) => {
                let message = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("(non-string panic)");
                format!("  panicked: {message}\n")
            }
        };
        if failures.is_empty() {
            continue;
        }
        failing.insert(case.name.clone());
        if explain {
            explanations_left -= 1;
        }
        if !pending.contains(&case.name) {
            let _ = write!(report, "\n✗ {}\n{failures}", case.name);
        }
    }
    let checked = corpus.cases.iter().map(|c| c.name.clone()).collect();
    let total = corpus.cases.len();
    println!(
        "conformance: {} of {total} cases conform to the reference parser ({} pending)",
        total - failing.len(),
        failing.len()
    );
    let Some(problems) = pending.settle(&checked, &failing) else {
        return;
    };
    report.push_str(&problems);
    assert!(
        report.is_empty(),
        "conformance doesn't match pending/parse.txt:\n{report}"
    );
}

/// One-shot parsing alone: every case must match, except those pending (the
/// test above keeps the list honest).
#[test]
fn every_one_shot_parse_matches_the_reference_parser() {
    let corpus = load_corpus();
    let golden = golden_dir();
    let pending = Pending::load("parse");
    let mut explanations_left = 5;
    let mut report = String::new();
    let mut failed = 0;
    for case in corpus.cases.iter().filter(|c| !pending.contains(&c.name)) {
        let expected = read(&golden.join(format!("{}.oneshot.json", case.name)));
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            parse(&case.input, &case.library, case.root.as_deref()).to_json()
        }));
        let actual = match outcome {
            Ok(json) => json,
            Err(_) => "(panicked)".to_owned(),
        };
        if actual == expected {
            continue;
        }
        failed += 1;
        let detail = if explanations_left > 0 {
            explanations_left -= 1;
            diff(&expected, &actual)
        } else {
            String::new()
        };
        let _ = write!(report, "\n✗ {}\n{detail}", case.name);
    }
    println!(
        "one-shot: {failed} of {} cases differ ({} more pending)",
        corpus.cases.len(),
        corpus
            .cases
            .iter()
            .filter(|c| pending.contains(&c.name))
            .count()
    );
    // Rewriting the list is the test above's; this one read it before then.
    if update_pending() {
        return;
    }
    assert!(
        report.is_empty(),
        "{failed} one-shot parses differ from the reference:\n{report}"
    );
}

/// A stream of the whole program, once it has ended, gives what parsing it
/// one-shot does, except where the reference itself ends differently
/// (`conformance/QUIRKS.md`: redefinitions counted twice, quirk 4; mismatched
/// brackets, quirk 5; a trailing redefinition that needed closing, quirk 11).
#[test]
fn streaming_ends_where_parsing_ends() {
    let corpus = load_corpus();
    let mut reference_divergent = Vec::new();
    let mut report = String::new();
    for case in &corpus.cases {
        let golden = golden_json(&case.name, "steps.json");
        // Needs the reference's result at the end of the single-character
        // stream; large cases skip that chunking.
        let Some(reference_end) = golden["1"]["steps"].as_array().and_then(|s| s.last()) else {
            continue;
        };
        let root = case.root.as_deref();
        let oneshot = parse(&case.input, &case.library, root).to_json();
        let mut stream = StreamParser::new(case.library.clone(), root);
        if case.input.is_empty() {
            continue;
        }
        for (i, c) in case.input.char_indices() {
            stream.push(&case.input[i..i + c.len_utf8()]);
        }
        let end = stream.finish().to_json();
        let reference_oneshot = read(&golden_dir().join(format!("{}.oneshot.json", case.name)));
        let reference_diverges =
            reference_end.as_str() != Some(fingerprint(&reference_oneshot).as_str());
        if reference_diverges {
            reference_divergent.push(case.name.as_str());
        } else if end != oneshot {
            let _ = write!(report, "\n✗ {}\n{}", case.name, diff(&oneshot, &end));
        }
    }
    println!(
        "stream end = one-shot on every case except {} where the reference also differs: {}",
        reference_divergent.len(),
        reference_divergent.join(", ")
    );
    assert!(
        report.is_empty(),
        "streaming ends differently from one-shot parsing:\n{report}"
    );
}
