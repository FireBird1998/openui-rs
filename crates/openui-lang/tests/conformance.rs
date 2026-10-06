//! Conformance with the reference parser: for every corpus case, the one-shot
//! parse result and the result after every streaming push must serialize to
//! exactly the JSON the reference parser produced (see `conformance/golden/`,
//! written by `scripts/regen-golden.sh`). Needs no Bun unless a streaming step
//! mismatches, and then only to show the reference JSON for that step.

use std::collections::HashMap;
use std::fmt::Write as _;
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

/// "<fnv1a-32 hex>:<byte length>" over UTF-8 bytes; mirrors `reference/run.ts`.
fn fingerprint(json: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in json.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}:{}", json.len())
}

/// Split into chunks of `size` chars, like `Array.from(text)` sliced in the runner.
fn chunks(text: &str, size: usize) -> Vec<&str> {
    let ends: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .skip(size)
        .step_by(size)
        .collect();
    let mut out = Vec::new();
    let mut start = 0;
    for end in ends.into_iter().chain([text.len()]) {
        if end > start {
            out.push(&text[start..end]);
            start = end;
        }
    }
    out
}

struct Case {
    name: String,
    library: Arc<Library>,
    input: String,
    root: Option<String>,
}

fn load_cases(conformance: &Path) -> (Vec<usize>, Vec<Case>) {
    let manifest: Value = serde_json::from_str(&read(&conformance.join("manifest.json"))).unwrap();
    let mut libraries: HashMap<String, Arc<Library>> = HashMap::new();
    for (name, file) in manifest["schemas"].as_object().unwrap() {
        let schema: Value =
            serde_json::from_str(&read(&conformance.join(file.as_str().unwrap()))).unwrap();
        libraries.insert(name.clone(), Arc::new(Library::from_json_schema(&schema)));
    }
    let chunkings = manifest["chunkings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap() as usize)
        .collect();
    let cases = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| Case {
            name: c["name"].as_str().unwrap().to_owned(),
            library: libraries[c["schema"].as_str().unwrap()].clone(),
            input: read(&conformance.join(c["file"].as_str().unwrap())),
            root: c.get("root").and_then(Value::as_str).map(str::to_owned),
        })
        .collect();
    (chunkings, cases)
}

/// The reference JSON for one streaming step (counted from 1), if Bun is
/// available to run it.
fn reference_step(case: &str, chunking: usize, step: usize) -> Result<String, String> {
    let args = [
        "run.ts",
        "show",
        case,
        &chunking.to_string(),
        &step.to_string(),
    ];
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

#[test]
fn every_case_matches_the_reference_parser() {
    let conformance = repo_root().join("conformance");
    let golden = conformance.join("golden");
    let (chunkings, cases) = load_cases(&conformance);

    let mut checks = 0;
    let mut failures = String::new();
    for case in &cases {
        let root = case.root.as_deref();

        checks += 1;
        let expected = read(&golden.join(format!("{}.oneshot.json", case.name)));
        let actual = parse(&case.input, &case.library, root).to_json();
        if actual != expected {
            let _ = write!(
                failures,
                "\n✗ {} — one-shot parse\n{}",
                case.name,
                diff(&expected, &actual)
            );
        }

        let steps: Value =
            serde_json::from_str(&read(&golden.join(format!("{}.steps.json", case.name)))).unwrap();
        for &size in &chunkings {
            let expected: Vec<&str> = steps[size.to_string()]
                .as_array()
                .unwrap_or_else(|| panic!("{}: no golden steps for chunk size {size}", case.name))
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect();
            let parts = chunks(&case.input, size);
            checks += 1;
            if parts.len() != expected.len() {
                let _ = write!(
                    failures,
                    "\n✗ {} — chunk size {size}: {} pushes, reference has {}\n",
                    case.name,
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
                let detail = match reference_step(&case.name, size, step + 1) {
                    Ok(reference) => diff(&reference, &actual),
                    Err(hint) => format!("  rust: {actual}\n  {hint}\n"),
                };
                let _ = write!(
                    failures,
                    "\n✗ {} — chunk size {size}, step {} of {} (input so far: {so_far:?})\n{detail}",
                    case.name,
                    step + 1,
                    parts.len()
                );
                break; // later steps of this stream usually repeat the same divergence
            }
        }
    }

    let failed = failures.matches("\n✗ ").count();
    println!(
        "conformance: {} of {checks} checks match the reference parser",
        checks - failed
    );
    assert!(
        failures.is_empty(),
        "{failed} of {checks} checks differ from the reference parser:\n{failures}"
    );
}
