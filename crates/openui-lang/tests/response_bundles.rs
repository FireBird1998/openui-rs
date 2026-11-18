#[path = "support/response_bundles.rs"]
mod support;

use openui_lang::{
    Limits, ResponseBundleError, parse_response_bundle, parse_response_bundle_utf16,
};
use serde_json::Value;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const GOLDEN: &str = include_str!("../../../conformance/golden/response-bundles.json");
const SCRIPTS: &str = "]]>openui:scripts";

fn case_units(case: &Value) -> Option<Vec<u16>> {
    support::units(case.get("units").unwrap_or(&case["raw"]))
}

fn matches_goldens(selected: impl Fn(&str) -> bool) {
    let corpus: Value = serde_json::from_str(support::CASES).unwrap();
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let mut compared = 0;
    for case in corpus["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if !selected(name) {
            continue;
        }
        compared += 1;
        let input = case_units(case);
        let actual = support::read(input.as_deref(), &mut Vec::new()).unwrap();
        let expected = &golden["cases"][name];
        assert_eq!(actual["complete"], expected["complete"], "{name} stored");
        let actual = actual["cuts"].as_array().unwrap();
        let expected = expected["cuts"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "{name} cuts");
        for (cut, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(actual, expected, "{name} streaming cut {cut}");
        }
    }
    assert!(compared > 0);
}

#[test]
fn renderer_reads_stored_responses_and_every_utf16_streaming_cut() {
    matches_goldens(|_| true);
}

#[test]
fn script_revisions_track_name_and_code_changes_and_ignore_order_and_other_fields() {
    let corpus: Value = serde_json::from_str(support::CASES).unwrap();
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    let pairs = corpus["revisionPairs"].as_array().unwrap();
    let expected = golden["revisionPairs"].as_array().unwrap();
    let read = |name: &str| {
        let case = cases.iter().find(|case| case["name"] == name).unwrap();
        support::complete(case_units(case).as_deref()).unwrap()
    };
    assert_eq!(pairs.len(), expected.len());
    for (pair, equal) in pairs.iter().zip(expected) {
        let left = pair["left"].as_str().unwrap();
        let right = pair["right"].as_str().unwrap();
        let left_bundle = read(left);
        let right_bundle = read(right);
        let actual = left_bundle.script_revision == right_bundle.script_revision;
        let wire_revision = |json: &str| {
            // The revision is the last field, and the bundle may contain lone
            // UTF-16 units which serde_json cannot decode into a Rust String.
            json.rsplit_once(",\"scriptRevision\":")
                .map(|(_, token)| token.to_owned())
        };
        assert_eq!(
            actual,
            wire_revision(&left_bundle.to_json()) == wire_revision(&right_bundle.to_json()),
            "serialized revision equality for {left} vs {right}"
        );
        let upstream = equal.as_bool().unwrap();
        if (left, right)
            == (
                "revision-locale-tie-original",
                "revision-locale-tie-reversed",
            )
        {
            assert!(
                !upstream,
                "upstream preserves insertion order for this locale tie"
            );
            assert!(actual, "plain name ordering gives a canonical revision");
        } else {
            assert_eq!(actual, upstream, "{left} vs {right}");
        }
    }
}

#[test]
fn size_admission_counts_the_original_response_and_not_the_mask_or_program() {
    let limits = Limits::DEFAULT.with_program_size(SCRIPTS.len());
    let bundle = parse_response_bundle(Some(SCRIPTS), false, limits).unwrap();
    assert_eq!(bundle.error, Some("Invalid scripts bundle"));
    for streaming in [false, true] {
        let nested_scripts = format!("{SCRIPTS} {{\"count\":0}}\n{}", "[".repeat(801));
        assert_eq!(
            parse_response_bundle(Some(&nested_scripts), streaming, limits),
            Err(ResponseBundleError::ProgramSize)
        );
        let over = format!("{SCRIPTS}\n");
        assert_eq!(
            parse_response_bundle(Some(&over), streaming, limits),
            Err(ResponseBundleError::ProgramSize)
        );
        assert_eq!(
            parse_response_bundle(Some("é"), streaming, limits.with_program_size(1)),
            Err(ResponseBundleError::ProgramSize)
        );
        assert_eq!(
            parse_response_bundle_utf16(&[0xd83d, 0xde00], streaming, limits.with_program_size(3)),
            Err(ResponseBundleError::ProgramSize)
        );
        assert_eq!(
            parse_response_bundle_utf16(&[0xd800], streaming, limits.with_program_size(2)),
            Err(ResponseBundleError::ProgramSize)
        );
    }
    assert!(parse_response_bundle_utf16(&[0xd800], true, limits.with_program_size(3)).is_ok());
}

#[test]
fn script_header_and_payload_use_the_existing_json_depth_limit() {
    let limits = Limits::DEFAULT.with_depth(0); // 32 open JSON containers.
    let array = |depth: usize| format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    for streaming in [false, true] {
        for (header, payload) in [
            (
                format!("{{\"count\":0,\"extra\":{}}}", array(31)),
                "[]".to_owned(),
            ),
            ("\"ignored\"".to_owned(), array(32)),
            (
                r#"{"count":1}"#.to_owned(),
                format!(r#"[{{"name":"x","code":"","extra":{}}}]"#, array(30)),
            ),
        ] {
            let raw = format!("{SCRIPTS} {header}\n{payload}");
            assert!(parse_response_bundle(Some(&raw), streaming, limits).is_ok());
        }
        for (header, payload) in [
            (
                format!("{{\"count\":0,\"extra\":{}}}", array(32)),
                "[]".to_owned(),
            ),
            (r#"{"count":0}"#.to_owned(), array(33)),
            // Depth refusal precedes the later JSON syntax error.
            (r#"{"count":0}"#.to_owned(), "[".repeat(33) + "oops"),
        ] {
            let raw = format!("{SCRIPTS} {header}\n{payload}");
            assert_eq!(
                parse_response_bundle(Some(&raw), streaming, limits),
                Err(ResponseBundleError::ScriptsDepth)
            );
        }
    }
}

#[test]
fn only_the_winning_context_is_checked_and_only_reached_scripts_json_is_read() {
    let limits = Limits::DEFAULT.with_depth(0);
    let nested = "[".repeat(33);
    let context = format!("body\n]]>openui:context\n{nested}");
    assert_eq!(
        parse_response_bundle(Some(&context), false, limits),
        Err(ResponseBundleError::ContextDepth)
    );
    let retry = format!("{context}\n]]>openui:content\nnew");
    assert_eq!(
        parse_response_bundle(Some(&retry), false, limits)
            .unwrap()
            .program
            .to_string_lossy(),
        "new"
    );
    for header in ["null", "{invalid}"] {
        let raw = format!("{SCRIPTS} {header}\n{nested}");
        assert_eq!(
            parse_response_bundle(Some(&raw), false, limits)
                .unwrap()
                .error,
            Some("Invalid scripts bundle")
        );
    }
    let blocked = format!("]]>unrecognized\n{SCRIPTS} {{\"count\":0}}\n{nested}");
    assert!(
        !parse_response_bundle(Some(&blocked), false, limits)
            .unwrap()
            .is_bundle
    );
}

#[test]
fn raised_and_exceeded_scripts_json_depth_is_stack_safe() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let containers = 20_000;
            let raw = format!(
                "{SCRIPTS} {{\"count\":0,\"unused\":{}0{}}}\n[]",
                "[".repeat(containers),
                "]".repeat(containers),
            );
            assert_eq!(
                parse_response_bundle(Some(&raw), false, Limits::DEFAULT),
                Err(ResponseBundleError::ScriptsDepth)
            );
            let bundle =
                parse_response_bundle(Some(&raw), false, Limits::DEFAULT.with_depth(containers))
                    .unwrap();
            assert!(bundle.complete);
            assert!(bundle.script_revision.is_none());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn response_text_exposes_exact_utf16_and_lossy_rust_text() {
    let input = [0xd800, 0xdbfe, 0xdc00, 0xfdd0, 0xe000, 0xdc00];
    let program = parse_response_bundle_utf16(&input, false, Limits::DEFAULT)
        .unwrap()
        .program;
    assert_eq!(program.utf16_units().collect::<Vec<_>>(), input);
    assert_eq!(
        program.to_string_lossy(),
        "\u{fffd}\u{10f800}\u{fdd0}\u{e000}\u{fffd}"
    );
}
