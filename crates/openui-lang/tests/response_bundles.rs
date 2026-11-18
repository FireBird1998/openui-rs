#[path = "support/response_bundles.rs"]
mod support;

use openui_lang::{
    Limits, ResponseBundleError, parse_message, parse_response_bundle, response_bundle_json_utf16,
    utf16_unit_string, utf16_units,
};
use serde_json::Value;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const GOLDEN: &str = include_str!("../../../conformance/golden/response-bundles.json");
const SCRIPTS: &str = "]]>openui:scripts";

fn case_units(case: &Value) -> Option<Vec<u16>> {
    support::units(case.get("units").unwrap_or(&case["raw"]))
}

fn matches_goldens(
    selected: impl Fn(&str) -> bool,
    read: impl Fn(Option<&[u16]>, &mut Vec<String>) -> Result<Value, ResponseBundleError>,
) {
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
        let actual = read(input.as_deref(), &mut Vec::new()).unwrap();
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
    matches_goldens(|_| true, support::read);
}

#[test]
fn the_public_rust_reader_matches_goldens_outside_the_pinned_string_collisions() {
    matches_goldens(
        |name| {
            !matches!(
                name,
                "reserved-and-lone-program"
                    | "reserved-and-lone-name"
                    | "surrogates-in-script-strings"
            )
        },
        |input, revisions| {
            support::read_with(input, revisions, |input, streaming| {
                let mut text = String::new();
                for decoded in char::decode_utf16(input.iter().copied()) {
                    match decoded {
                        Ok(scalar) => text.push(scalar),
                        Err(error) => text.push_str(&utf16_unit_string(error.unpaired_surrogate())),
                    }
                }
                parse_response_bundle(Some(&text), streaming, Limits::DEFAULT)
                    .map(|bundle| bundle.to_json())
            })
        },
    );
}

#[test]
fn script_revision_equality_identifies_the_same_set_and_never_splits_an_upstream_match() {
    let corpus: Value = serde_json::from_str(support::CASES).unwrap();
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    let pairs = corpus["revisionPairs"].as_array().unwrap();
    let expected = golden["revisionPairs"].as_array().unwrap();
    let read = |name: &str| {
        let case = cases.iter().find(|case| case["name"] == name).unwrap();
        let raw = case["raw"].as_str().unwrap();
        let units: Vec<_> = raw.encode_utf16().collect();
        (
            parse_response_bundle(Some(raw), false, Limits::DEFAULT).unwrap(),
            response_bundle_json_utf16(&units, false, Limits::DEFAULT).unwrap(),
        )
    };
    assert_eq!(pairs.len(), expected.len());
    for (pair, comparison) in pairs.iter().zip(expected) {
        let left = pair["left"].as_str().unwrap();
        let right = pair["right"].as_str().unwrap();
        let (left_bundle, left_exact) = read(left);
        let (right_bundle, right_exact) = read(right);
        let actual = left_bundle.script_revision() == right_bundle.script_revision();
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
        assert_eq!(
            actual,
            wire_revision(&left_exact) == wire_revision(&right_exact),
            "WebAssembly revision equality for {left} vs {right}"
        );
        let upstream_equal = comparison["upstreamEqual"].as_bool().unwrap();
        let same_scripts = comparison["sameScripts"].as_bool().unwrap();
        assert!(
            !upstream_equal || actual,
            "split an upstream equality for {left} vs {right}"
        );
        assert_eq!(
            actual, same_scripts,
            "name and code sets for {left} vs {right}"
        );
        if pair["localeTie"] == true {
            assert!(
                same_scripts && !upstream_equal,
                "upstream order dependence for {left} vs {right}"
            );
        } else {
            assert_eq!(actual, upstream_equal, "{left} vs {right}");
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
            response_bundle_json_utf16(&[0xd83d, 0xde00], streaming, limits.with_program_size(3)),
            Err(ResponseBundleError::ProgramSize)
        );
        assert_eq!(
            response_bundle_json_utf16(&[0xd800], streaming, limits.with_program_size(2)),
            Err(ResponseBundleError::ProgramSize)
        );
    }
    assert!(response_bundle_json_utf16(&[0xd800], true, limits.with_program_size(3)).is_ok());
}

#[test]
fn scripts_marker_line_and_content_use_the_existing_json_depth_limit() {
    let limits = Limits::DEFAULT.with_depth(0); // 32 open JSON containers.
    let array = |depth: usize| format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    for streaming in [false, true] {
        for (count_json, scripts_json) in [
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
            let raw = format!("{SCRIPTS} {count_json}\n{scripts_json}");
            assert!(parse_response_bundle(Some(&raw), streaming, limits).is_ok());
        }
        for (count_json, scripts_json) in [
            (
                format!("{{\"count\":0,\"extra\":{}}}", array(32)),
                "[]".to_owned(),
            ),
            (r#"{"count":0}"#.to_owned(), array(33)),
            // Depth refusal precedes the later JSON syntax error.
            (r#"{"count":0}"#.to_owned(), "[".repeat(33) + "oops"),
        ] {
            let raw = format!("{SCRIPTS} {count_json}\n{scripts_json}");
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
            .program,
        "new"
    );
    for count_json in ["null", "{invalid}"] {
        let raw = format!("{SCRIPTS} {count_json}\n{nested}");
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
            assert!(bundle.script_revision().is_none());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn rust_readers_share_the_reserved_scalar_convention() {
    let raw = "]]>openui:content?name=%F4%8F%A0%80\nroot = Text(\"\u{10f800}\")";
    for streaming in [false, true] {
        let message = parse_message(raw, streaming, Limits::DEFAULT).unwrap();
        let bundle = parse_response_bundle(Some(raw), streaming, Limits::DEFAULT).unwrap();
        assert_eq!(bundle.program, message.content);
        assert_eq!(bundle.name.as_ref(), message.attributes.get("name"));
        assert_eq!(
            utf16_units(bundle.name.as_deref().unwrap()).collect::<Vec<_>>(),
            [0xd800]
        );
        assert_eq!(message.to_json().matches(r"\ud800").count(), 2);
        assert_eq!(bundle.to_json().matches(r"\ud800").count(), 2);
        let units: Vec<_> = raw.encode_utf16().collect();
        let wire = response_bundle_json_utf16(&units, streaming, Limits::DEFAULT).unwrap();
        assert_eq!(wire.matches('\u{10f800}').count(), 2);
    }

    let corpus: Value = serde_json::from_str(support::CASES).unwrap();
    let collision = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "surrogates-in-script-strings")
        .unwrap()["raw"]
        .as_str()
        .unwrap();
    let bundle = parse_response_bundle(Some(collision), false, Limits::DEFAULT).unwrap();
    assert_eq!(bundle.error, Some("Invalid scripts bundle"));
    assert!(bundle.script_names.is_empty());

    let stand_ins = "\u{10f83d}\u{10fe00}";
    let message = parse_message(stand_ins, false, Limits::DEFAULT).unwrap();
    let bundle = parse_response_bundle(Some(stand_ins), false, Limits::DEFAULT).unwrap();
    assert_eq!(bundle.program, "😀");
    assert_eq!(bundle.program, message.content);
}

#[test]
fn exact_utf16_binding_json_distinguishes_lone_units_from_real_scalars() {
    let input = [0xd800, 0xdbfe, 0xdc00, 0xfdd0, 0xe000, 0xdc00];
    let json = response_bundle_json_utf16(&input, false, Limits::DEFAULT).unwrap();
    assert!(json.starts_with(&format!(
        r#"{{"program":"\ud800{}\udc00","#,
        "\u{10f800}\u{fdd0}\u{e000}"
    )));
}

#[test]
fn legacy_scripts_can_change_completion_while_still_streaming() {
    let start = "root = Text(\"Hello\")\n";
    let marker = format!("{start}{SCRIPTS} {{\"count\":1}}\n");
    let complete = format!("{marker}[{{\"name\":\"x\",\"code\":\"\"}}]");
    let extra = format!("{complete}x");
    for (raw, is_bundle, is_complete) in [
        (start, false, true),
        (&marker, true, false),
        (&complete, true, true),
        (&extra, true, false),
    ] {
        let bundle = parse_response_bundle(Some(raw), true, Limits::DEFAULT).unwrap();
        assert_eq!(
            (bundle.is_bundle, bundle.complete),
            (is_bundle, is_complete),
            "{raw:?}"
        );
        assert_eq!(bundle.error, None);
    }
}

#[test]
fn a_mid_line_retry_without_an_end_marker_streams_as_a_bare_program() {
    let raw = "cut-off]]>openui:content?name=Hidden\nnew";
    let bundle = parse_response_bundle(Some(raw), true, Limits::DEFAULT).unwrap();
    assert_eq!(bundle.program, "new");
    assert_eq!(bundle.name, None);
    assert!(!bundle.is_bundle);
    assert!(bundle.complete);
}
