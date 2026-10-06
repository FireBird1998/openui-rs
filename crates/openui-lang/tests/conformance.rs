//! Conformance with the reference parser: for every corpus case, the one-shot
//! parse result and the result after every streaming push must serialize to
//! exactly the JSON the reference parser produced (see `conformance/golden/`,
//! written by `scripts/regen-golden.sh`). Needs no Bun unless a streaming step
//! mismatches, and then only to show the reference JSON for that step.
//!
//! Cases listed in `conformance/pending.txt` aren't supported yet: they may
//! fail, but the suite fails if one starts passing (so it gets removed from the
//! list). `UPDATE_PENDING=1 cargo test --test conformance` rewrites the list to
//! exactly the cases failing now.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use openui_lang::{Library, StreamParser, parse};
use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// FNV-1a (32-bit) over UTF-8 bytes, as 8 hex digits; mirrors `reference/run.ts`.
fn fingerprint(json: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in json.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

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

struct Case {
    name: String,
    library: Arc<Library>,
    input: String,
    root: Option<String>,
}

struct Corpus {
    chunkings: Vec<String>,
    cases: Vec<Case>,
}

/// The hand-written manifest plus the imported one.
fn load_corpus(conformance: &Path) -> Corpus {
    let manifests: Vec<Value> = ["manifest.json", "imported.json"]
        .iter()
        .map(|f| conformance.join(f))
        .filter(|p| p.exists())
        .map(|p| serde_json::from_str(&read(&p)).unwrap())
        .collect();
    let mut libraries: HashMap<String, Arc<Library>> = HashMap::new();
    for manifest in &manifests {
        for (name, file) in manifest["schemas"].as_object().unwrap() {
            let schema: Value =
                serde_json::from_str(&read(&conformance.join(file.as_str().unwrap()))).unwrap();
            libraries.insert(name.clone(), Arc::new(Library::from_json_schema(&schema)));
        }
    }
    let chunkings = manifests[0]["chunkings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    let cases = manifests
        .iter()
        .flat_map(|m| m["cases"].as_array().unwrap())
        .map(|c| Case {
            name: c["name"].as_str().unwrap().to_owned(),
            library: libraries[c["schema"].as_str().unwrap()].clone(),
            input: read(&conformance.join(c["file"].as_str().unwrap())),
            root: c.get("root").and_then(Value::as_str).map(str::to_owned),
        })
        .collect();
    Corpus { chunkings, cases }
}

fn load_pending(path: &Path) -> BTreeSet<String> {
    if !path.exists() {
        return BTreeSet::new();
    }
    read(path)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The reference JSON for one streaming step (counted from 1), if Bun is
/// available to run it.
fn reference_step(case: &str, chunking: &str, step: usize) -> Result<String, String> {
    let step = step.to_string();
    let args = ["run.ts", "show", case, chunking, &step];
    let command = format!("(cd reference && bun {})", args.join(" "));
    match Command::new("bun")
        .args(args)
        .current_dir(repo_root().join("reference"))
        .env("OPENUI_TELEMETRY_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .output()
    {
        Ok(out) if out.status.success() => Ok(String::from_utf8(out.stdout).unwrap()),
        _ => Err(format!("run `{command}` to see the reference JSON")),
    }
}

fn pretty(json: &str) -> String {
    serde_json::from_str::<Value>(json)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| json.to_owned())
}

/// A unified diff of the pretty-printed JSON, or both raw strings when they only
/// differ in ways pretty-printing hides (such as number formatting).
fn diff(reference: &str, rust: &str) -> String {
    let (a, b) = (pretty(reference), pretty(rust));
    if a == b {
        return format!("  reference: {reference}\n  rust:      {rust}\n");
    }
    similar::TextDiff::from_lines(&a, &b)
        .unified_diff()
        .context_radius(3)
        .header("reference", "rust")
        .to_string()
}

/// Every check for one case; returns a report of each mismatch (empty when the
/// case conforms). `explain` fetches reference JSON for readable diffs.
fn check_case(case: &Case, chunkings: &[String], golden: &Path, explain: bool) -> String {
    let mut failures = String::new();
    let root = case.root.as_deref();

    let expected = read(&golden.join(format!("{}.oneshot.json", case.name)));
    let actual = parse(&case.input, &case.library, root).to_json();
    if actual != expected {
        let _ = write!(failures, "  one-shot parse\n{}", diff(&expected, &actual));
    }

    let steps: Value =
        serde_json::from_str(&read(&golden.join(format!("{}.steps.json", case.name)))).unwrap();
    for chunking in chunkings {
        let golden = &steps[chunking];
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
        if parts.len() != expected.len() {
            let _ = writeln!(
                failures,
                "  chunking {chunking}: {} pushes, reference has {}",
                parts.len(),
                expected.len()
            );
            continue;
        }
        let mut parser = StreamParser::new(case.library.clone(), root);
        let mut so_far = String::new();
        for (step, part) in parts.iter().enumerate() {
            so_far.push_str(part);
            let actual = parser.push(part).to_json();
            if fingerprint(&actual) == expected[step] {
                continue;
            }
            let detail = if !explain {
                String::new()
            } else {
                match reference_step(&case.name, chunking, step + 1) {
                    Ok(reference) => diff(&reference, &actual),
                    Err(hint) => format!("  rust: {actual}\n  {hint}\n"),
                }
            };
            let _ = write!(
                failures,
                "  chunking {chunking}, step {} of {} (input so far: {so_far:?})\n{detail}",
                step + 1,
                parts.len()
            );
            break; // later steps of this stream usually repeat the same divergence
        }
    }
    failures
}

#[test]
fn every_case_matches_the_reference_parser() {
    let root = repo_root();
    let conformance = root.join("conformance");
    let golden = conformance.join("golden");
    let pending_path = conformance.join("pending.txt");
    let corpus = load_corpus(&conformance);
    let pending = load_pending(&pending_path);
    let update_pending = std::env::var_os("UPDATE_PENDING").is_some();

    // Fetching reference JSON runs Bun once per mismatch; enough to start on.
    let mut explanations_left = 5;
    let mut failing = BTreeSet::new();
    let mut report = String::new();
    for case in &corpus.cases {
        let explain = !pending.contains(&case.name) && !update_pending && explanations_left > 0;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            check_case(case, &corpus.chunkings, &golden, explain)
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
        let is_pending = pending.contains(&case.name);
        if failures.is_empty() {
            if is_pending {
                let _ = writeln!(
                    report,
                    "\n✓ {} now conforms: remove it from pending.txt",
                    case.name
                );
            }
        } else {
            failing.insert(case.name.clone());
            if explain {
                explanations_left -= 1;
            }
            if !is_pending {
                let _ = write!(report, "\n✗ {}\n{failures}", case.name);
            }
        }
    }
    let names: BTreeSet<&str> = corpus.cases.iter().map(|c| c.name.as_str()).collect();
    for stale in pending.iter().filter(|p| !names.contains(p.as_str())) {
        let _ = writeln!(report, "\n? {stale} is in pending.txt but isn't a case");
    }

    let total = corpus.cases.len();
    println!(
        "conformance: {} of {total} cases conform to the reference parser ({} pending)",
        total - failing.len(),
        failing.len()
    );
    if update_pending {
        let mut text = String::from(
            "# Cases the core doesn't conform on yet. The conformance test fails if one\n\
             # starts passing; regenerate with UPDATE_PENDING=1 cargo test --test conformance\n",
        );
        for name in &failing {
            text.push_str(name);
            text.push('\n');
        }
        std::fs::write(&pending_path, text).unwrap();
        println!(
            "wrote {} pending cases to {}",
            failing.len(),
            pending_path.display()
        );
        return;
    }
    assert!(
        report.is_empty(),
        "conformance doesn't match pending.txt:\n{report}"
    );
}
