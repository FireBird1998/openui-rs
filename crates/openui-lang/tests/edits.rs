//! The rest of the language layer against the reference: edit-mode merge
//! (`mergeStatements`), element trees back to source (`jsonToOpenUI`), and
//! validation errors with hints (`enrichErrors`). Expected outputs are in
//! `conformance/golden/edits/`, written by `reference/edits.ts`. Each kind has
//! a pending list, `conformance/pending/<kind>.txt`.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use common::{Pending, conformance_dir, golden_dir, load_corpus, read};
use openui_lang::{
    Library, enrich_errors, enriched_errors_to_json, json_to_openui, merge_statements, parse,
    parse_json,
};
use serde_json::Value;

/// Readable failures reported per test; the rest are counted.
const SHOWN: usize = 8;

fn json_file(path: &Path) -> Value {
    parse_json(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A manifest's cases, or none if the file isn't there.
fn cases(file: &str) -> Vec<Value> {
    let path = conformance_dir().join(file);
    if !path.exists() {
        return Vec::new();
    }
    json_file(&path)["cases"].as_array().unwrap().clone()
}

fn golden(kind: &str) -> serde_json::Map<String, Value> {
    match json_file(&golden_dir().join(format!("edits/{kind}.json"))) {
        Value::Object(map) => map,
        _ => panic!("edits/{kind}.json is not an object"),
    }
}

fn text_diff(reference: &str, rust: &str) -> String {
    similar::TextDiff::from_lines(reference, rust)
        .unified_diff()
        .context_radius(2)
        .header("reference", "rust")
        .to_string()
}

/// Collects mismatches for one kind; the first few with a diff and the
/// command that shows the reference's output. Pending ones are only counted.
struct Report {
    kind: &'static str,
    pending: Pending,
    checked: usize,
    names: BTreeSet<String>,
    failing: BTreeSet<String>,
    failed: usize,
    text: String,
}

impl Report {
    fn new(kind: &'static str) -> Report {
        Report {
            kind,
            pending: Pending::load(kind),
            checked: 0,
            names: BTreeSet::new(),
            failing: BTreeSet::new(),
            failed: 0,
            text: String::new(),
        }
    }

    fn check(&mut self, name: &str, expected: &Value, actual: Result<String, String>) {
        self.checked += 1;
        self.names.insert(name.to_owned());
        let matches = match (&actual, expected.get("text"), expected.get("error")) {
            (Ok(text), Some(Value::String(want)), _) => text == want,
            (Err(_), _, Some(_)) => true,
            _ => false,
        };
        if matches {
            return;
        }
        self.failing.insert(name.to_owned());
        if self.pending.contains(name) {
            return;
        }
        self.failed += 1;
        if self.failed > SHOWN {
            return;
        }
        let kind = self.kind;
        let show = |o: Option<&Value>, e: Option<&Value>| match (o, e) {
            (Some(Value::String(t)), _) => t.clone(),
            (_, Some(e)) => format!("error: {e}"),
            _ => "(nothing)".to_owned(),
        };
        let want = show(expected.get("text"), expected.get("error"));
        let got = match &actual {
            Ok(t) => t.clone(),
            Err(e) => format!("error: {e}"),
        };
        let _ = write!(
            self.text,
            "\n✗ {name}  (bun reference/edits.ts show {kind} {name})\n{}",
            text_diff(&want, &got)
        );
    }

    fn finish(mut self) {
        let kind = self.kind;
        println!(
            "{kind}: {} of {} cases match ({} pending)",
            self.checked - self.failing.len(),
            self.checked,
            self.failing.len() - self.failed
        );
        let Some(problems) = self.pending.settle(&self.names, &self.failing) else {
            return;
        };
        self.text.push_str(&problems);
        assert!(
            self.text.is_empty(),
            "{} {kind} cases differ from the reference, apart from pending/{kind}.txt:\n{}",
            self.failed,
            self.text
        );
    }
}

#[test]
fn merging_patches_matches_the_reference() {
    let inputs: HashMap<String, String> = load_corpus()
        .cases
        .into_iter()
        .map(|c| (c.name, c.input))
        .collect();
    let text = |c: &Value, inline: &str, by_case: &str| -> String {
        match (c.get(inline), c.get(by_case)) {
            (Some(Value::String(t)), _) => t.clone(),
            (_, Some(Value::String(name))) => inputs[name].clone(),
            _ => panic!("{}: no {inline} or {by_case}", c["name"]),
        }
    };
    let golden = golden("merge");
    let mut report = Report::new("merge");
    let all: Vec<Value> = [
        "imported/merge.json",
        "merge.json",
        "merge-generated.json",
        "merge-fuzz.json",
        "merge-recorded.json",
    ]
    .iter()
    .flat_map(|f| cases(f))
    .collect();
    assert!(all.len() > 1000, "the merge corpus has {} cases", all.len());
    for c in &all {
        let name = c["name"].as_str().unwrap();
        let existing = text(c, "existing", "existingCase");
        let patch = text(c, "patch", "patchCase");
        let merged = merge_statements(&existing, &patch, c.get("rootId").and_then(Value::as_str));
        report.check(name, &golden[name], Ok(merged));
    }
    report.finish();
}

#[test]
fn serializing_element_trees_matches_the_reference() {
    // Schemas by name: the corpus manifests' files, plus inline ones.
    let mut schemas: HashMap<String, Value> = HashMap::new();
    for file in [
        "manifest.json",
        "imported.json",
        "fuzz.json",
        "synthetic.json",
    ] {
        let path = conformance_dir().join(file);
        if path.exists() {
            for (name, rel) in json_file(&path)["schemas"].as_object().unwrap() {
                let schema = json_file(&conformance_dir().join(rel.as_str().unwrap()));
                schemas.insert(name.clone(), schema);
            }
        }
    }
    let mut serialize_cases = Vec::new();
    for file in [
        "imported/serialize.json",
        "serialize.json",
        "serialize-fuzz.json",
    ] {
        let path = conformance_dir().join(file);
        if !path.exists() {
            continue;
        }
        let manifest = json_file(&path);
        for (name, schema) in manifest
            .get("schemas")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            schemas.insert(name.clone(), schema.clone());
        }
        serialize_cases.extend(manifest["cases"].as_array().unwrap().iter().cloned());
    }
    let libraries: HashMap<&String, Library> = schemas
        .iter()
        .map(|(name, schema)| (name, Library::from_json_schema(schema)))
        .collect();

    let golden = golden("serialize");
    let mut report = Report::new("serialize");
    for c in &serialize_cases {
        let name = c["name"].as_str().unwrap();
        let library = &libraries[&c["schema"].as_str().unwrap().to_owned()];
        let actual = json_to_openui(&c["element"], library, c.get("stateDeclarations"))
            .map_err(|e| e.to_string());
        report.check(name, &golden[name], actual);
    }
    // Every corpus case's one-shot root, as the reference parser gave it.
    let mut from_corpus = 0;
    for case in load_corpus().cases {
        let result = json_file(&golden_dir().join(format!("{}.oneshot.json", case.name)));
        if result["root"].is_null() {
            continue;
        }
        from_corpus += 1;
        let name = format!("corpus/{}", case.name);
        let actual = json_to_openui(
            &result["root"],
            &case.library,
            Some(&result["stateDeclarations"]),
        )
        .map_err(|e| e.to_string());
        report.check(&name, &golden[&name], actual);
    }
    assert!(from_corpus > 200, "only {from_corpus} corpus roots");
    report.finish();
}

/// `Object.keys(schema.$defs ?? {})` for an object: array-index keys first,
/// in numeric order, then the rest as written.
fn defs_keys(schema: &Value) -> Vec<String> {
    let Some(Value::Object(defs)) = schema.get("$defs") else {
        return Vec::new();
    };
    let index = |k: &str| {
        k.parse::<u32>()
            .ok()
            .filter(|i| *i != u32::MAX && i.to_string() == k)
    };
    let mut keys: Vec<&String> = defs.keys().collect();
    keys.sort_by_key(|k| index(k).map_or((1, 0), |i| (0, i)));
    keys.into_iter().cloned().collect()
}

#[test]
fn enriched_errors_match_the_reference() {
    let golden = golden("enrich");
    let mut report = Report::new("enrich");
    let mut codes = std::collections::HashSet::new();
    for case in load_corpus().cases {
        let result = parse(&case.input, &case.library, case.root.as_deref());
        let errors = &result.meta.errors;
        if errors.is_empty() {
            continue;
        }
        // The runner's names: `Object.keys(schema.$defs)`, action shapes included.
        let names = defs_keys(&case.schema);
        let mut variants = vec![(case.name.clone(), names)];
        if errors
            .iter()
            .any(|e| e.code == openui_lang::ValidationErrorCode::UnknownComponent)
        {
            variants.push((format!("{}#no-names", case.name), Vec::new()));
        }
        for (name, names) in variants {
            let enriched = enrich_errors(errors, &case.schema, &names);
            for e in &enriched {
                codes.insert((e.code.as_str(), e.hint.is_some()));
            }
            // A case the reference reports no errors for has no golden.
            let expected = match golden.get(&name) {
                Some(text) => serde_json::json!({ "text": text }),
                None => serde_json::json!({}),
            };
            report.check(&name, &expected, Ok(enriched_errors_to_json(&enriched)));
        }
    }
    for case in cases("enrich.json") {
        let name = case["name"].as_str().unwrap();
        let error = openui_lang::ValidationError {
            code: openui_lang::ValidationErrorCode::MissingRequired,
            component: case["component"].as_str().unwrap().to_owned(),
            path: "/title".to_owned(),
            message: "m".to_owned(),
            statement_id: None,
            severity: None,
        };
        let enriched = enrich_errors(&[error], &case["schema"], &[]);
        report.check(
            name,
            &serde_json::json!({"text": golden[name]}),
            Ok(enriched_errors_to_json(&enriched)),
        );
    }
    // Cases the reference reports errors for and the core doesn't.
    let unchecked: Vec<String> = golden
        .keys()
        .filter(|name| !report.names.contains(*name))
        .cloned()
        .collect();
    for name in unchecked {
        let expected = serde_json::json!({ "text": golden[&name] });
        report.check(&name, &expected, Ok(enriched_errors_to_json(&[])));
    }
    assert!(report.checked > 150, "only {} enrich cases", report.checked);
    for code in [
        "missing-required",
        "null-required",
        "unknown-component",
        "inline-reserved",
    ] {
        assert!(codes.contains(&(code, true)), "no {code} error with a hint");
    }
    assert!(
        codes.contains(&("unknown-component", false)),
        "no unknown-component without names"
    );
    report.finish();
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
