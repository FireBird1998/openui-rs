//! Stage-level conformance: each pipeline stage the reference package exports
//! matches it on every prefix of every corpus program, so a mismatch points at
//! the stage that diverged rather than at the whole parse result.

mod common;

use std::fmt::Write as _;

use common::{explain, fingerprint, golden_json, load_corpus};
use openui_lang::{
    auto_close, parse_expression, preprocess, split, strip_fences, tokenize, tokens_to_json,
};

/// Bun lookups for readable diffs; enough to start on.
const EXPLANATIONS: usize = 5;

/// Compare `stage(prefix)` with the golden fingerprint for every code-point
/// prefix of every case; `show` is the runner command that prints one.
fn check_every_prefix(golden_suffix: &str, show: &str, stage: impl Fn(&str) -> String) {
    let corpus = load_corpus();
    let mut checked = 0;
    let mut failed = 0;
    let mut report = String::new();
    for case in &corpus.cases {
        let golden = golden_json(&case.name, golden_suffix);
        let expected: Vec<&str> = golden
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let ends: Vec<usize> = case
            .input
            .char_indices()
            .map(|(i, c)| i + c.len_utf8())
            .collect();
        assert_eq!(
            ends.len(),
            expected.len(),
            "{}: golden has a different length",
            case.name
        );
        for (k, &end) in ends.iter().enumerate() {
            checked += 1;
            let prefix = &case.input[..end];
            let actual = stage(prefix);
            if fingerprint(&actual) == expected[k] {
                continue;
            }
            failed += 1;
            let tail: String = prefix
                .chars()
                .rev()
                .take(40)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let length = (k + 1).to_string();
            let detail = if failed <= EXPLANATIONS {
                explain(&[show, &case.name, &length], &actual)
            } else {
                String::new()
            };
            let _ = write!(
                report,
                "\n✗ {} — first {length} chars (ending {tail:?})\n{detail}",
                case.name
            );
            break; // longer prefixes of this case usually repeat the divergence
        }
    }
    println!(
        "{golden_suffix}: {} of {} cases match on every prefix ({checked} prefixes checked)",
        corpus.cases.len() - failed,
        corpus.cases.len()
    );
    assert!(
        report.is_empty(),
        "{failed} cases differ from the reference:\n{report}"
    );
}

#[test]
fn tokens_match_the_reference_for_every_prefix() {
    check_every_prefix("tokens.json", "show-tokens", |prefix| {
        tokens_to_json(&tokenize(prefix))
    });
}

/// autoClose, then split(tokenize(closed text)), each statement with
/// parseExpression of its tokens; the document `show-statements` prints.
fn statements_json(text: &str) -> String {
    let (closed, was_incomplete) = auto_close(text);
    let mut out = format!(
        "{{\"autoClose\":{{\"text\":{},\"wasIncomplete\":{was_incomplete}}},\"statements\":[",
        serde_json::to_string(&closed).unwrap()
    );
    for (i, stmt) in split(&tokenize(&closed)).iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"id\":{},\"idTokenType\":{},\"tokens\":{},\"expr\":{}}}",
            serde_json::to_string(&stmt.id).unwrap(),
            stmt.id_kind.token_kind(),
            tokens_to_json(&stmt.tokens),
            parse_expression(&stmt.tokens).to_json()
        );
    }
    out.push_str("]}");
    out
}

#[test]
fn statements_and_expressions_match_the_reference_for_every_prefix() {
    check_every_prefix("statements.json", "show-statements", statements_json);
}

#[test]
fn fences_and_comments_are_stripped_like_the_reference_for_every_prefix() {
    check_every_prefix("preprocess.json", "show-preprocess", |prefix| {
        format!(
            "{{\"stripFences\":{},\"preprocess\":{}}}",
            serde_json::to_string(&strip_fences(prefix)).unwrap(),
            serde_json::to_string(&preprocess(prefix)).unwrap()
        )
    });
}
