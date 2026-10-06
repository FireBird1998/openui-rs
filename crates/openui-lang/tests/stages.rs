//! Stage-level conformance: each pipeline stage the reference package exports
//! matches it on every corpus program, so a mismatch points at the stage that
//! diverged rather than at the whole parse result.

mod common;

use std::fmt::Write as _;

use common::{explain, fingerprint, golden_json, load_corpus};
use openui_lang::{tokenize, tokens_to_json};

/// Bun lookups for readable diffs; enough to start on.
const EXPLANATIONS: usize = 5;

#[test]
fn tokens_match_the_reference_for_every_prefix() {
    let corpus = load_corpus();
    let mut checked = 0;
    let mut failed = 0;
    let mut report = String::new();
    for case in &corpus.cases {
        let golden = golden_json(&case.name, "tokens.json");
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
            let actual = tokens_to_json(&tokenize(prefix));
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
                explain(&["show-tokens", &case.name, &length], &actual)
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
        "tokens: {} of {} cases match on every prefix ({checked} prefixes checked)",
        corpus.cases.len() - failed,
        corpus.cases.len()
    );
    assert!(
        report.is_empty(),
        "{failed} cases tokenize differently from the reference:\n{report}"
    );
}
