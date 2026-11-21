use openui_ffi::{OpenUiError, ParseLimits, extract_mcp_tool_result};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

#[test]
fn native_export_returns_reference_values_and_tool_reported_errors_as_json() {
    let golden = include_str!("../../../conformance/golden/mcp-results.jsonl");
    for line in golden.lines() {
        let row = openui_lang::parse_json(line).unwrap();
        let input = row["input"].as_str().unwrap();
        let outcome = &row["outcome"];
        let actual = extract_mcp_tool_result(input.to_owned(), ParseLimits::default());
        if outcome.get("thrown").is_some() {
            match actual {
                Err(OpenUiError::Evaluation { reason }) => {
                    assert_eq!(Some(reason.as_str()), outcome["reason"].as_str(), "{line}");
                }
                actual => panic!("expected a typed evaluation error, got {actual:?}: {line}"),
            }
        } else {
            assert_eq!(
                actual.unwrap(),
                openui_lang::stringify_json(outcome),
                "{line}"
            );
        }
    }
}

#[test]
fn native_export_maps_invalid_json_and_explicit_limits() {
    let limits = ParseLimits::default();
    assert!(matches!(
        extract_mcp_tool_result("{".into(), limits.clone()),
        Err(OpenUiError::InvalidJson { .. })
    ));
    for limits in [
        ParseLimits {
            result_size: Some(1),
            ..limits.clone()
        },
        ParseLimits {
            work: Some(1),
            ..limits.clone()
        },
        ParseLimits {
            depth: Some(1),
            ..limits
        },
    ] {
        assert!(matches!(
            extract_mcp_tool_result(r#"{"structuredContent":[]}"#.into(), limits),
            Err(OpenUiError::LimitExceeded { .. })
        ));
    }
}
