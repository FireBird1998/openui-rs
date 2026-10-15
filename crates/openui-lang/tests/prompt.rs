//! Prompt generation conformance: every spec in `conformance/prompts/` gives
//! exactly the prompt the reference's `generateSystemPrompt` gives, or the
//! same error message where the reference throws. Where the reference crashes
//! inside the JavaScript engine instead (its golden records an `errorType`),
//! the core only has to return an error too.

mod common;

use std::fmt::Write as _;

use common::{conformance_dir, golden_dir, read};
use openui_lang::generate_system_prompt;
use serde_json::Value;

fn spec_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(conformance_dir().join("prompts"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .map(|p| p.file_stem().unwrap().to_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

fn text_diff(reference: &str, rust: &str) -> String {
    similar::TextDiff::from_lines(reference, rust)
        .unified_diff()
        .context_radius(2)
        .header("reference", "rust")
        .to_string()
}

#[test]
fn every_prompt_spec_matches_the_reference() {
    let names = spec_names();
    assert!(
        names.len() >= 40,
        "the prompt corpus has {} specs",
        names.len()
    );
    let mut report = String::new();
    let mut failed = 0;
    for name in &names {
        let spec: Value = serde_json::from_str(&read(
            &conformance_dir().join(format!("prompts/{name}.json")),
        ))
        .unwrap();
        let golden: Value =
            serde_json::from_str(&read(&golden_dir().join(format!("prompts/{name}.json"))))
                .unwrap();
        let expected = match (golden.get("prompt"), golden.get("error")) {
            (Some(Value::String(p)), _) => Ok(p.clone()),
            (_, Some(Value::String(e))) => Err(e.clone()),
            _ => panic!("{name}: golden has neither prompt nor error"),
        };
        let actual = generate_system_prompt(&spec).map_err(|e| e.to_string());
        // The reference crashed inside the JavaScript engine (a TypeError, say):
        // the core has to refuse the spec too, but not in the engine's words.
        let engine_error = golden.get("errorType").is_some();
        if actual == expected || (engine_error && actual.is_err()) {
            continue;
        }
        failed += 1;
        let show = |r: &Result<String, String>| match r {
            Ok(p) => p.clone(),
            Err(e) => format!("error: {e}"),
        };
        let _ = write!(
            report,
            "\n✗ {name}\n{}",
            text_diff(&show(&expected), &show(&actual))
        );
    }
    println!(
        "prompts: {} of {} specs match",
        names.len() - failed,
        names.len()
    );
    assert!(
        report.is_empty(),
        "{failed} prompts differ from the reference:\n{report}"
    );
}

#[test]
fn the_corpus_covers_every_mode() {
    // Each combination of the flags that change the prompt's shape has a spec.
    let mut seen = std::collections::HashSet::new();
    for name in spec_names() {
        let spec: Value = serde_json::from_str(&read(
            &conformance_dir().join(format!("prompts/{name}.json")),
        ))
        .unwrap();
        if spec.get("cloud") == Some(&Value::Bool(true)) {
            seen.insert("cloud".to_owned());
            continue;
        }
        let options = spec.get("promptOptions").unwrap_or(&spec);
        let has_tools = options
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|t| !t.is_empty());
        let flag = |key: &str, default: bool| match options.get(key) {
            None | Some(Value::Null) => default,
            Some(Value::Bool(b)) => *b,
            Some(Value::Number(n)) => n.as_f64() != Some(0.0),
            Some(Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        };
        let tool_calls = flag("toolCalls", has_tools);
        let bindings = flag("bindings", tool_calls);
        seen.insert(format!(
            "tools={has_tools} toolCalls={tool_calls} bindings={bindings} edit={} inline={}",
            flag("editMode", false),
            flag("inlineMode", false)
        ));
    }
    for required in [
        "cloud",
        "tools=false toolCalls=false bindings=false edit=false inline=false",
        "tools=true toolCalls=true bindings=true edit=false inline=false",
        "tools=true toolCalls=false bindings=false edit=false inline=false",
        "tools=true toolCalls=true bindings=false edit=false inline=false",
        "tools=false toolCalls=true bindings=true edit=false inline=false",
        "tools=false toolCalls=false bindings=true edit=false inline=false",
        "tools=false toolCalls=true bindings=false edit=false inline=false",
        "tools=false toolCalls=false bindings=false edit=true inline=false",
        "tools=false toolCalls=false bindings=false edit=false inline=true",
        "tools=true toolCalls=true bindings=true edit=true inline=true",
    ] {
        assert!(seen.contains(required), "no spec for {required}");
    }
}
