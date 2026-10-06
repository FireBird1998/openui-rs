//! Differential fuzzing of edit-mode merge and element serialization
//! (`--edits`): the core's `merge_statements` and `json_to_openui` against
//! the reference's `mergeStatements` and `jsonToOpenUI`, compared in full.
//!
//! Merge inputs are corpus or generated programs with patches made from them
//! (edited lines, deletions, fences and prose, other programs, mangled text).
//! Serialize inputs are real parse results, sometimes with a field deleted,
//! retyped or renamed. Mismatches are saved to `conformance/merge-fuzz.json`
//! and `conformance/serialize-fuzz.json`.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use openui_lang::{json_to_openui, merge_statements, parse, parse_json, stringify_json};
use serde_json::{Value, json};

use crate::generate::{Generator, mutate};
use crate::reference::Reference;
use crate::rng::Rng;
use crate::{BATCH, Corpus, MAX_SAVES, fingerprint};

#[derive(Clone, Debug)]
enum Edit {
    Merge {
        existing: String,
        patch: String,
        root: Option<String>,
    },
    Serialize {
        lib: String,
        element: Value,
        state: Option<Value>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Outcome {
    Match,
    Mismatch,
    ReferenceThrew,
    CorePanicked,
}

/// A few lines of `text`, with a string literal edited now and then.
fn some_lines(rng: &mut Rng, text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    for _ in 0..1 + rng.below(3) {
        let line = *rng.pick(&lines);
        out.push(match (rng.chance(50), line.find('"')) {
            (true, Some(start)) => match line[start + 1..].find('"') {
                Some(len) => format!("{}\"Edited\"{}", &line[..start], &line[start + len + 2..]),
                None => line.to_owned(),
            },
            _ => line.to_owned(),
        });
    }
    out.join("\n")
}

/// Statement ids at the start of `text`'s lines.
fn ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let id: String = l
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            (!id.is_empty() && l.contains('=')).then_some(id)
        })
        .collect()
}

fn merge_edit(rng: &mut Rng, corpus: &Corpus) -> Edit {
    let (lib, base, _) = rng.pick(&corpus.cases).clone();
    let existing = match rng.below(10) {
        0..=2 => mutate(rng, &base),
        3 => Generator::new(rng, &corpus.libraries[&lib]).program(),
        4 => String::new(),
        _ => base,
    };
    let known = ids(&existing);
    let patch = match rng.below(8) {
        0 | 1 => some_lines(rng, &existing),
        2 => Generator::new(rng, &corpus.libraries[&lib]).program(),
        3 => mutate(rng, &existing),
        4 if !known.is_empty() => {
            let id = rng.pick(&known).clone();
            format!("{id} = null\n{}", some_lines(rng, &existing))
        }
        5 => format!(
            "Here's the change:\n\n```openui-lang\n{}\n```\n\nAnything else?",
            some_lines(rng, &existing)
        ),
        6 => {
            let lines = some_lines(rng, &existing);
            mutate(rng, &lines)
        }
        _ => rng
            .pick(&["", "  \n", "Nothing to change.", "// none"])
            .to_string(),
    };
    let root = rng.chance(15).then(|| {
        if !known.is_empty() && rng.chance(70) {
            rng.pick(&known).clone()
        } else {
            rng.pick(&["nope", "", "$x"]).to_string()
        }
    });
    Edit::Merge {
        existing,
        patch,
        root,
    }
}

/// Count the nodes of a tree.
fn size(v: &Value) -> usize {
    1 + match v {
        Value::Array(items) => items.iter().map(size).sum(),
        Value::Object(map) => map.values().map(size).sum(),
        _ => 0,
    }
}

/// Change the `target`-th node (pre-order) one way or another.
fn mangle_at(rng: &mut Rng, v: &mut Value, target: &mut usize) -> bool {
    if *target == 0 {
        let leaf = rng
            .pick(&[
                json!(null),
                json!(0),
                json!(7),
                json!(true),
                json!("null"),
                json!("x"),
                json!([]),
                json!({}),
                json!({"toString": 1}),
                json!({"k": "Str"}),
            ])
            .clone();
        match v {
            Value::Object(map) if !map.is_empty() && rng.chance(70) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                let key = rng.pick(&keys).clone();
                match rng.below(4) {
                    0 => {
                        map.shift_remove(&key);
                    }
                    1 => {
                        map.insert(key, leaf);
                    }
                    2 => {
                        let name = rng
                            .pick(&["statementId", "__proto__", "toString", "k", "length", "0"])
                            .to_string();
                        map.insert(name, leaf);
                    }
                    _ => {
                        let kind = rng.pick(&["Str", "Ref", "BinOp", "Comp", "Arr", "Obj", "Nope"]);
                        map.insert("k".to_owned(), json!(kind));
                    }
                }
            }
            _ => *v = leaf,
        }
        return true;
    }
    *target -= 1;
    match v {
        Value::Array(items) => items.iter_mut().any(|item| mangle_at(rng, item, target)),
        Value::Object(map) => map.values_mut().any(|item| mangle_at(rng, item, target)),
        _ => false,
    }
}

fn serialize_edit(rng: &mut Rng, corpus: &Corpus) -> Edit {
    for _ in 0..8 {
        let (lib, base, root) = rng.pick(&corpus.cases).clone();
        let input = match rng.below(10) {
            0..=3 => mutate(rng, &base),
            4 | 5 => Generator::new(rng, &corpus.libraries[&lib]).program(),
            _ => base,
        };
        let result = parse(&input, &corpus.libraries[&lib], root.as_deref()).to_json();
        let result = parse_json(&result).expect("a parse result reads back");
        if result["root"].is_null() {
            continue;
        }
        let mut element = result["root"].clone();
        let mut state = result["stateDeclarations"].clone();
        if rng.chance(40) {
            for _ in 0..1 + rng.below(3) {
                let mut target = rng.below(size(&element));
                mangle_at(rng, &mut element, &mut target);
            }
        }
        if rng.chance(10) {
            let mut target = rng.below(size(&state));
            mangle_at(rng, &mut state, &mut target);
        }
        return Edit::Serialize {
            lib,
            element,
            state: rng.chance(85).then_some(state),
        };
    }
    Edit::Serialize {
        lib: "bench".to_owned(),
        element: json!({"type": "element", "typeName": "Stack", "props": {}, "partial": false}),
        state: None,
    }
}

fn edit(seed: u64, k: u64, corpus: &Corpus) -> Edit {
    let mut rng = Rng::for_program(seed ^ 0x0ED1_75ED, k);
    if rng.chance(50) {
        merge_edit(&mut rng, corpus)
    } else {
        serialize_edit(&mut rng, corpus)
    }
}

fn request(e: &Edit) -> String {
    match e {
        Edit::Merge {
            existing,
            patch,
            root,
        } => {
            let mut r = json!({"op": "merge", "existing": existing, "patch": patch});
            if let Some(root) = root {
                r["rootId"] = json!(root);
            }
            // Programs never hold the code points standing in for lone surrogates.
            r.to_string()
        }
        Edit::Serialize {
            lib,
            element,
            state,
        } => {
            let state = state
                .as_ref()
                .map(|s| format!(",\"state\":{}", stringify_json(s)))
                .unwrap_or_default();
            format!(
                "{{\"op\":\"serialize\",\"lib\":{},\"element\":{}{state}}}",
                stringify_json(&json!(lib)),
                stringify_json(element)
            )
        }
    }
}

/// The core's answer: `Ok(text)`, `Err` where it refuses the input, or `None`
/// if it panicked.
fn core(e: &Edit, corpus: &Corpus) -> Option<Result<String, String>> {
    catch_unwind(AssertUnwindSafe(|| match e {
        Edit::Merge {
            existing,
            patch,
            root,
        } => Ok(merge_statements(existing, patch, root.as_deref())),
        Edit::Serialize {
            lib,
            element,
            state,
        } => json_to_openui(element, &corpus.libraries[lib], state.as_ref())
            .map_err(|e| e.to_string()),
    }))
    .ok()
}

fn judge(e: &Edit, answer: &str, corpus: &Corpus) -> Outcome {
    let answer = parse_json(answer).expect("reference answer is JSON");
    let Some(ours) = core(e, corpus) else {
        return Outcome::CorePanicked;
    };
    match (&ours, answer.get("text"), answer.get("threw")) {
        (Ok(text), Some(Value::String(theirs)), _) if text == theirs => Outcome::Match,
        (Err(_), _, Some(_)) => Outcome::Match,
        (Ok(_), _, Some(_)) if matches!(e, Edit::Merge { .. }) => Outcome::ReferenceThrew,
        _ => Outcome::Mismatch,
    }
}

/// A smaller merge with the same outcome: whole lines of each text, then runs
/// of characters.
fn shrink(e: &Edit, outcome: Outcome, reference: &mut Reference, corpus: &Corpus) -> Edit {
    let Edit::Merge {
        existing,
        patch,
        root,
    } = e
    else {
        return e.clone();
    };
    let (mut existing, mut patch) = (existing.clone(), patch.clone());
    let keeps = |existing: &str, patch: &str, reference: &mut Reference| {
        let candidate = Edit::Merge {
            existing: existing.to_owned(),
            patch: patch.to_owned(),
            root: root.clone(),
        };
        let answer = reference.ask_lines(&[request(&candidate)]).pop().unwrap();
        judge(&candidate, &answer, corpus) == outcome
    };
    for which in 0..2 {
        // Lines first, then runs of characters.
        loop {
            let text = if which == 0 { &existing } else { &patch };
            let lines: Vec<&str> = text.split('\n').collect();
            let mut shrunk = None;
            for i in 0..lines.len() {
                let candidate = [&lines[..i], &lines[i + 1..]].concat().join("\n");
                let ok = if which == 0 {
                    keeps(&candidate, &patch, reference)
                } else {
                    keeps(&existing, &candidate, reference)
                };
                if ok {
                    shrunk = Some(candidate);
                    break;
                }
            }
            match shrunk {
                Some(s) if which == 0 => existing = s,
                Some(s) => patch = s,
                None => break,
            }
        }
        let mut run = (if which == 0 { &existing } else { &patch })
            .chars()
            .count()
            .div_ceil(2)
            .max(1);
        loop {
            let mut progressed = false;
            let mut start = 0;
            loop {
                let chars: Vec<char> = (if which == 0 { &existing } else { &patch })
                    .chars()
                    .collect();
                if start >= chars.len() {
                    break;
                }
                let end = (start + run).min(chars.len());
                let candidate: String = chars[..start].iter().chain(&chars[end..]).collect();
                let ok = if which == 0 {
                    keeps(&candidate, &patch, reference)
                } else {
                    keeps(&existing, &candidate, reference)
                };
                if ok {
                    if which == 0 {
                        existing = candidate;
                    } else {
                        patch = candidate;
                    }
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
    }
    Edit::Merge {
        existing,
        patch,
        root: root.clone(),
    }
}

/// Append found mismatches to `merge-fuzz.json` / `serialize-fuzz.json`, one
/// case per line, written as `JSON.stringify` would.
fn save(found: &[(Edit, String)], conformance: &Path) -> usize {
    let mut saved = 0;
    for (e, origin) in found {
        let (file, name, case) = match e {
            Edit::Merge {
                existing,
                patch,
                root,
            } => {
                let name = format!(
                    "fuzz/{}",
                    fingerprint(&format!("{existing}\0{patch}\0{root:?}"))
                );
                let mut case = json!({"name": name, "existing": existing, "patch": patch});
                if let Some(root) = root {
                    case["rootId"] = json!(root);
                }
                ("merge-fuzz.json", name, case)
            }
            Edit::Serialize {
                lib,
                element,
                state,
            } => {
                let name = format!(
                    "fuzz/{}",
                    fingerprint(&format!("{lib}\0{}", stringify_json(element)))
                );
                let mut case = json!({"name": name, "schema": lib, "element": element});
                if let Some(state) = state {
                    case["stateDeclarations"] = state.clone();
                }
                ("serialize-fuzz.json", name, case)
            }
        };
        let path = conformance.join(file);
        let mut manifest = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| parse_json(&t).ok())
            .unwrap_or_else(|| json!({"cases": []}));
        let cases = manifest["cases"].as_array_mut().unwrap();
        if cases.iter().any(|c| c["name"] == name) {
            continue;
        }
        let mut case = case;
        case["source"] = json!("fuzz");
        case["origin"] = json!(origin);
        cases.push(case);
        let lines: Vec<String> = cases.iter().map(stringify_json).collect();
        std::fs::write(
            &path,
            format!("{{\"cases\": [\n{}\n]}}\n", lines.join(",\n")),
        )
        .unwrap();
        saved += 1;
    }
    saved
}

pub fn run(
    seed: u64,
    indices: &[u64],
    corpus: &Corpus,
    reference: &mut Reference,
    conformance: &Path,
    save_cases: bool,
) {
    let mut tally: HashMap<(&str, Outcome), usize> = HashMap::new();
    let mut found: Vec<(Edit, String)> = Vec::new();
    for batch in indices.chunks(BATCH) {
        let edits: Vec<Edit> = batch.iter().map(|&k| edit(seed, k, corpus)).collect();
        let lines: Vec<String> = edits.iter().map(request).collect();
        let answers = reference.ask_lines(&lines);
        for ((e, answer), &k) in edits.iter().zip(&answers).zip(batch) {
            let outcome = judge(e, answer, corpus);
            let kind = if matches!(e, Edit::Merge { .. }) {
                "merge"
            } else {
                "serialize"
            };
            *tally.entry((kind, outcome)).or_default() += 1;
            if outcome == Outcome::Match || found.len() >= MAX_SAVES {
                continue;
            }
            let small = shrink(e, outcome, reference, corpus);
            let origin = format!("edits seed {seed}, input {k}");
            let shown = match &small {
                Edit::Merge {
                    existing,
                    patch,
                    root,
                } => {
                    format!("merge existing {existing:?} patch {patch:?} root {root:?}")
                }
                Edit::Serialize {
                    lib,
                    element,
                    state,
                } => format!(
                    "serialize ({lib}) {} state {}",
                    stringify_json(element),
                    state.as_ref().map(stringify_json).unwrap_or_default()
                ),
            };
            println!("{outcome:?} at {origin}: {shown}");
            println!("  reference: {}", answer.trim_end());
            println!("  core:      {:?}", core(&small, corpus));
            found.push((small, origin));
        }
    }
    for kind in ["merge", "serialize"] {
        let count = |o| tally.get(&(kind, o)).copied().unwrap_or(0);
        let total: usize = [
            Outcome::Match,
            Outcome::Mismatch,
            Outcome::ReferenceThrew,
            Outcome::CorePanicked,
        ]
        .into_iter()
        .map(count)
        .sum();
        println!(
            "edits seed {seed}, {kind}: {total} inputs, {} match, {} mismatch, {} core panics, {} reference exceptions",
            count(Outcome::Match),
            count(Outcome::Mismatch),
            count(Outcome::CorePanicked),
            count(Outcome::ReferenceThrew),
        );
    }
    if save_cases && !found.is_empty() {
        let saved = save(&found, conformance);
        println!("saved {saved} new edit cases; run scripts/regen-golden.sh");
    }
}
