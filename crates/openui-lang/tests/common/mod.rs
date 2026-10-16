//! Shared by the conformance tests: the corpus, goldens, fingerprints, pending
//! lists, and readable diffs against the reference parser.
#![allow(dead_code)] // each test binary uses a different subset

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use openui_lang::Library;
use serde_json::Value;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The conformance folder: the repository's, or wherever `OPENUI_CONFORMANCE_DIR`
/// names, so the suite can run where the source tree isn't (a phone, a copy).
pub fn conformance_dir() -> PathBuf {
    match std::env::var_os("OPENUI_CONFORMANCE_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => repo_root().join("conformance"),
    }
}

/// Whether a failing test may run the reference to show a live diff. Not when
/// `OPENUI_NO_LIVE_DIFFS` is set: under mutation testing every caught mutant
/// would start Bun, and on a phone there's no Bun to start.
pub fn live_diffs() -> bool {
    std::env::var_os("OPENUI_NO_LIVE_DIFFS").is_none()
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

/// Each golden test's pending list in `conformance/pending/`, and what it lists.
pub const PENDING_LISTS: [(&str, &str); 8] = [
    ("parse", "parse results"),
    ("tokens", "tokens on every prefix"),
    (
        "statements",
        "statements and expression trees on every prefix",
    ),
    ("preprocess", "preprocessing on every prefix"),
    ("merge", "merges"),
    ("serialize", "trees back to source"),
    ("enrich", "enriched errors"),
    ("prompt", "system prompts"),
];

const PENDING_HEADER: &str = "\
# Cases the core doesn't match the reference on yet. Each may fail, but one
# that matches again, or a name that isn't a case, fails the test. Rewrite
# every list to the cases failing now: UPDATE_PENDING=1 cargo test -p openui-lang
";

/// Whether `UPDATE_PENDING` asks the golden tests to rewrite their pending lists.
pub fn update_pending() -> bool {
    std::env::var_os("UPDATE_PENDING").is_some()
}

/// The cases a golden test lets fail for now: `conformance/pending/<list>.txt`,
/// one name per line, `#` starting a comment line. Each test has a list of its
/// own, so tests running in parallel never write the same file.
pub struct Pending {
    list: String,
    path: PathBuf,
    names: BTreeSet<String>,
}

impl Pending {
    pub fn load(list: &str) -> Pending {
        assert!(
            PENDING_LISTS.iter().any(|(name, _)| *name == list),
            "{list} isn't one of PENDING_LISTS"
        );
        let path = conformance_dir()
            .join("pending")
            .join(format!("{list}.txt"));
        let names = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect();
        Pending {
            list: list.to_owned(),
            path,
            names,
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Squares the list with a run: `checked` is every name the test checked,
    /// `failing` those that failed. With `UPDATE_PENDING`, rewrites the list to
    /// `failing` and returns `None`. Otherwise returns what's wrong with the
    /// list, empty when nothing is: listed cases that match now, and listed
    /// names that aren't cases.
    pub fn settle(&self, checked: &BTreeSet<String>, failing: &BTreeSet<String>) -> Option<String> {
        let file = format!("pending/{}.txt", self.list);
        if update_pending() {
            let mut text = PENDING_HEADER.to_owned();
            for name in failing {
                text.push_str(name);
                text.push('\n');
            }
            std::fs::create_dir_all(self.path.parent().unwrap()).unwrap();
            std::fs::write(&self.path, text).unwrap();
            println!("wrote {} pending cases to {file}", failing.len());
            return None;
        }
        let mut problems = String::new();
        for name in &self.names {
            if !checked.contains(name) {
                problems.push_str(&format!("\n? {name} is in {file} but isn't a case\n"));
            } else if !failing.contains(name) {
                problems.push_str(&format!(
                    "\n✓ {name} matches the reference now: remove it from {file}\n"
                ));
            }
        }
        Some(problems)
    }
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
    /// The library's JSON Schema, as written.
    pub schema: Arc<Value>,
    pub input: String,
    pub root: Option<String>,
    /// Whether the case has per-prefix stage goldens (large ones skip them).
    pub stages: bool,
    /// The chunkings the case runs under, when not all of the manifest's.
    pub chunkings: Option<Vec<String>>,
    /// Chunk sizes in code points for a case that came with its own chunks:
    /// the `recorded` chunking.
    pub recorded: Option<Vec<usize>>,
}

impl Case {
    /// The chunkings this case's golden holds: its own or the corpus's, then
    /// `recorded` when it has recorded chunks.
    pub fn chunkings(&self, corpus: &[String]) -> Vec<String> {
        let mut out = self.chunkings.clone().unwrap_or_else(|| corpus.to_vec());
        if self.recorded.is_some() {
            out.push("recorded".to_owned());
        }
        out
    }
}

pub struct Corpus {
    pub chunkings: Vec<String>,
    pub cases: Vec<Case>,
}

/// The hand-written manifest, plus the imported, fuzz-found and synthetic cases.
pub fn load_corpus() -> Corpus {
    let conformance = conformance_dir();
    let manifests: Vec<Value> = [
        "manifest.json",
        "imported.json",
        "fuzz.json",
        "synthetic.json",
    ]
    .iter()
    .map(|f| conformance.join(f))
    .filter(|p| p.exists())
    .map(|p| serde_json::from_str(&read(&p)).unwrap())
    .collect();
    let mut libraries: HashMap<String, (Arc<Library>, Arc<Value>)> = HashMap::new();
    for manifest in &manifests {
        for (name, file) in manifest["schemas"].as_object().unwrap() {
            let schema: Value =
                serde_json::from_str(&read(&conformance.join(file.as_str().unwrap()))).unwrap();
            let library = Arc::new(Library::from_json_schema(&schema));
            libraries.insert(name.clone(), (library, Arc::new(schema)));
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
            library: libraries[c["schema"].as_str().unwrap()].0.clone(),
            schema: libraries[c["schema"].as_str().unwrap()].1.clone(),
            input: read(&conformance.join(c["file"].as_str().unwrap())),
            root: c.get("root").and_then(Value::as_str).map(str::to_owned),
            stages: c.get("stages").and_then(Value::as_bool).unwrap_or(true),
            chunkings: c
                .get("chunkings")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect()),
            recorded: c
                .get("recorded")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|v| v.as_u64().unwrap() as usize).collect()),
        })
        .collect();
    Corpus { chunkings, cases }
}

/// Run `bun run.ts <args>` in `reference/` for reference output, if Bun is
/// installed and live diffs are on; otherwise a hint with the command to run.
pub fn reference_output(args: &[&str]) -> Result<String, String> {
    let command = format!("(cd reference && bun run.ts {})", args.join(" "));
    if !live_diffs() {
        return Err(format!(
            "live diffs are off (OPENUI_NO_LIVE_DIFFS); run `{command}` to see the reference JSON"
        ));
    }
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
