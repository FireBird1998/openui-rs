//! Shared by the conformance tests: the corpus, goldens, fingerprints, and
//! readable diffs against the reference parser.
#![allow(dead_code)] // each test binary uses a different subset

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use openui_lang::Library;
use serde_json::Value;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn conformance_dir() -> PathBuf {
    repo_root().join("conformance")
}

pub fn golden_dir() -> PathBuf {
    conformance_dir().join("golden")
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A case's golden file with the given suffix, parsed as JSON.
pub fn golden_json(case: &str, suffix: &str) -> Value {
    serde_json::from_str(&read(&golden_dir().join(format!("{case}.{suffix}")))).unwrap()
}

/// FNV-1a (32-bit) over UTF-8 bytes, as 8 hex digits; mirrors `reference/run.ts`.
pub fn fingerprint(json: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in json.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

pub struct Case {
    pub name: String,
    pub library: Arc<Library>,
    pub input: String,
    pub root: Option<String>,
}

pub struct Corpus {
    pub chunkings: Vec<String>,
    pub cases: Vec<Case>,
}

/// The hand-written manifest plus the imported one.
pub fn load_corpus() -> Corpus {
    let conformance = conformance_dir();
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

/// Run `bun run.ts <args>` in `reference/` for reference output, if Bun is
/// installed; otherwise a hint with the command to run.
pub fn reference_output(args: &[&str]) -> Result<String, String> {
    let command = format!("(cd reference && bun run.ts {})", args.join(" "));
    match Command::new("bun")
        .arg("run.ts")
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
pub fn diff(reference: &str, rust: &str) -> String {
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

/// The reference's diff for `args`, or the Rust output plus a hint without Bun.
pub fn explain(args: &[&str], actual: &str) -> String {
    match reference_output(args) {
        Ok(reference) => diff(&reference, actual),
        Err(hint) => format!("  rust: {actual}\n  {hint}\n"),
    }
}
