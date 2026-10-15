//! The binding hands Swift exactly what the core produces, for every case.

#[path = "../../openui-lang/tests/common/mod.rs"]
mod common;

use std::fmt::Write;
use std::sync::Arc;

use openui_ffi::{
    ComponentLibrary, OpenUiError, ParseOutput, StreamSession, generate_system_prompt, parse,
};
use openui_lang::{ParseResult, StreamParser};

/// Whether the binding's output is the core's result: the same JSON and flags.
fn same(output: &ParseOutput, result: &ParseResult) -> bool {
    output.json == result.to_json()
        && output.depth_limited == result.depth_limited
        && output.size_limited == result.size_limited
}

/// Consecutive chunks of `size` chars (code points), as the corpus chunkings cut.
fn chunks_of(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars.chunks(size).map(|c| c.iter().collect()).collect()
}

fn library(case: &common::Case) -> Arc<ComponentLibrary> {
    ComponentLibrary::new(case.schema.to_string()).unwrap()
}

#[test]
fn one_shot_parse_matches_the_core_on_every_case() {
    let corpus = common::load_corpus();
    let mut report = String::new();
    for case in &corpus.cases {
        let got = parse(case.input.clone(), library(case), case.root.clone());
        let want = openui_lang::parse(&case.input, &case.library, case.root.as_deref());
        if !same(&got, &want) {
            let _ = write!(report, "\n✗ {}", case.name);
        }
    }
    assert!(
        report.is_empty(),
        "the binding differs from the core:{report}"
    );
}

/// Every step of every case, under several chunk sizes: one character at a
/// time where the corpus streams the case that way (large cases don't).
#[test]
fn streaming_matches_the_core_at_every_step() {
    let corpus = common::load_corpus();
    let mut report = String::new();
    for case in &corpus.cases {
        let one_char = case
            .chunkings
            .as_ref()
            .is_none_or(|c| c.iter().any(|c| c == "1"));
        let sizes: &[usize] = if one_char { &[1, 3, 16] } else { &[3, 16] };
        for &size in sizes {
            let session = StreamSession::new(library(case), case.root.clone());
            let mut core = StreamParser::new(case.library.clone(), case.root.as_deref());
            for (i, chunk) in chunks_of(&case.input, size).into_iter().enumerate() {
                let want = core.push(&chunk);
                if !same(&session.push(chunk), &want) {
                    let _ = write!(
                        report,
                        "\n✗ {} at chunk size {size}, step {}",
                        case.name,
                        i + 1
                    );
                    break;
                }
            }
            if !same(&session.result(), &core.result()) {
                let _ = write!(
                    report,
                    "\n✗ {} at chunk size {size}, final result",
                    case.name
                );
            }
        }
    }
    assert!(
        report.is_empty(),
        "the binding's stream differs from the core's:{report}"
    );
}

#[test]
fn pushing_many_chunks_at_once_ends_where_pushing_them_one_by_one_does() {
    let corpus = common::load_corpus();
    let mut report = String::new();
    for case in &corpus.cases {
        let chunks = chunks_of(&case.input, 2);
        let mut core = StreamParser::new(case.library.clone(), case.root.as_deref());
        let want = chunks
            .iter()
            .map(|c| core.push(c))
            .last()
            .unwrap_or_else(|| core.result());
        let session = StreamSession::new(library(case), case.root.clone());
        let (first, rest) = chunks.split_at(chunks.len() / 2);
        session.push_all(first.to_vec());
        if !same(&session.push_all(rest.to_vec()), &want) {
            let _ = write!(report, "\n✗ {}", case.name);
        }
    }
    assert!(
        report.is_empty(),
        "push_all differs from pushing one by one:{report}"
    );
}

#[test]
fn pushing_no_chunks_returns_the_current_result() {
    let corpus = common::load_corpus();
    let case = &corpus.cases[0];
    let session = StreamSession::new(library(case), case.root.clone());
    session.push(case.input.clone());
    assert_eq!(session.push_all(Vec::new()).json, session.result().json);
}

#[test]
fn set_matches_the_core() {
    let corpus = common::load_corpus();
    let mut report = String::new();
    for case in &corpus.cases {
        let half: String = case
            .input
            .chars()
            .take(case.input.chars().count() / 2)
            .collect();
        let session = StreamSession::new(library(case), case.root.clone());
        let mut core = StreamParser::new(case.library.clone(), case.root.as_deref());
        for text in [&half, &case.input, &half] {
            if !same(&session.set(text.clone()), &core.set(text)) {
                let _ = write!(report, "\n✗ {}", case.name);
                break;
            }
        }
    }
    assert!(report.is_empty(), "set differs from the core:{report}");
}

/// Every spec in the prompt corpus gives the core's prompt, or the core's error.
#[test]
fn system_prompts_match_the_core() {
    let dir = common::conformance_dir().join("prompts");
    let mut specs: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    specs.retain(|p| p.extension().is_some_and(|e| e == "json"));
    assert!(specs.len() > 40, "found only {} prompt specs", specs.len());
    let mut report = String::new();
    for path in specs {
        let text = common::read(&path);
        let want = openui_lang::generate_system_prompt(&serde_json::from_str(&text).unwrap());
        let same = match (generate_system_prompt(text), want) {
            (Ok(got), Ok(want)) => got == want,
            (Err(OpenUiError::Prompt { message }), Err(want)) => message == want.to_string(),
            _ => false,
        };
        if !same {
            let _ = write!(report, "\n✗ {}", path.display());
        }
    }
    assert!(
        report.is_empty(),
        "the binding's prompts differ from the core's:{report}"
    );
}

#[test]
fn a_prompt_spec_that_is_not_json_is_an_error() {
    assert!(matches!(
        generate_system_prompt("{".to_owned()),
        Err(OpenUiError::InvalidPromptSpec { .. })
    ));
}

#[test]
fn a_schema_that_is_not_json_is_an_error() {
    let Err(error) = ComponentLibrary::new("{\"$defs\":".to_owned()) else {
        panic!("loaded a truncated schema");
    };
    assert!(matches!(error, OpenUiError::InvalidSchema { .. }));
}

#[test]
fn component_names_keep_the_schema_order() {
    let schema = r#"{"$defs": {"Stack": {"properties": {}}, "Card": {"properties": {}}, "Text": {"properties": {}}}}"#;
    let library = ComponentLibrary::new(schema.to_owned()).unwrap();
    assert_eq!(library.component_names(), ["Stack", "Card", "Text"]);
}
