use openui_runtime::{
    Limits, McpResponseError, McpToolOutcome, Value, extract_mcp_tool_result, value_to_json,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

const GOLDEN: &str = include_str!("../../../conformance/golden/mcp-results.jsonl");
const LIMITS: Limits = Limits::DEFAULT;

#[test]
fn extraction_matches_the_reference_bytes() {
    let mut actual = String::new();
    for line in GOLDEN.lines() {
        let row = openui_lang::parse_json(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let input = row["input"].as_str().unwrap();
        let (outcome, number_kind) = match extract_mcp_tool_result(input, LIMITS) {
            Ok(McpToolOutcome::Result(value)) => {
                let number_kind = match value {
                    Value::Num(n) if n == f64::INFINITY => Some("Infinity"),
                    Value::Num(n) if n == f64::NEG_INFINITY => Some("-Infinity"),
                    Value::Num(n) if n == 0.0 && n.is_sign_negative() => Some("-0"),
                    _ => None,
                };
                (
                    format!(r#"{{"value":{}}}"#, value_to_json(&value).unwrap()),
                    number_kind,
                )
            }
            Ok(McpToolOutcome::ToolReported { message }) => (
                format!(
                    r#"{{"error":{}}}"#,
                    value_to_json(&Value::Str(message)).unwrap()
                ),
                None,
            ),
            Err(McpResponseError::InvalidResponse { reason }) => (
                format!(
                    r#"{{"thrown":"TypeError","reason":{}}}"#,
                    value_to_json(&Value::Str(reason.to_owned())).unwrap()
                ),
                None,
            ),
            Err(error) => panic!("{name}: {error}"),
        };
        let number_kind =
            number_kind.map_or(String::new(), |kind| format!(",\"numberKind\":\"{kind}\""));
        let bytes = format!(
            r#"{{"name":{},"input":{},"outcome":{outcome}{number_kind}}}"#,
            openui_lang::stringify_json(&row["name"]),
            openui_lang::stringify_json(&row["input"])
        );
        assert_eq!(bytes, line, "{name}");
        actual.push_str(&bytes);
        actual.push('\n');
    }
    assert_eq!(actual, GOLDEN);
}

fn assert_limit(result: Result<McpToolOutcome, McpResponseError>, depth: bool) {
    let Err(McpResponseError::Limit(error)) = result else {
        panic!("expected a typed limit refusal, got {result:?}");
    };
    assert_eq!(error.flags.depth_limited, depth);
    assert_eq!(error.flags.size_limited, !depth);
}

#[test]
fn tool_results_keep_null_data_and_reject_null_content_parts() {
    let data = r#"{"structuredContent":{"n":null}}"#;
    assert_eq!(
        extract_mcp_tool_result(data, LIMITS).unwrap(),
        McpToolOutcome::Result(Value::from_json(
            &openui_lang::parse_json(r#"{"n":null}"#).unwrap()
        ))
    );
    assert!(matches!(
        extract_mcp_tool_result(r#"{"content":[null]}"#, LIMITS),
        Err(McpResponseError::InvalidResponse {
            reason: "cannot read a text part's type from null"
        })
    ));
}

#[test]
fn response_bytes_and_work_are_checked_before_extraction() {
    let input = r#"{"structuredContent":true,"ignored":"would otherwise be discarded"}"#;
    assert!(extract_mcp_tool_result(input, LIMITS.with_result_size(input.len())).is_ok());
    assert_limit(
        extract_mcp_tool_result(input, LIMITS.with_result_size(input.len() - 1)),
        false,
    );
    assert!(extract_mcp_tool_result(input, LIMITS.with_work(input.len())).is_ok());
    assert_limit(
        extract_mcp_tool_result(input, LIMITS.with_work(input.len() - 1)),
        false,
    );
    assert_limit(
        extract_mcp_tool_result(" malformed ", LIMITS.with_result_size(10)),
        false,
    );
}

#[test]
fn response_depth_includes_unused_fields() {
    let input = r#"{"structuredContent":true,"ignored":[[[]]]}"#;
    assert!(extract_mcp_tool_result(input, LIMITS.with_depth(4)).is_ok());
    assert_limit(extract_mcp_tool_result(input, LIMITS.with_depth(3)), true);
}

#[test]
fn depth_refusal_in_text_is_not_a_plain_text_fallback() {
    let input = r#"{"content":[{"type":"text","text":"[[[[0]]]]"}]}"#;
    assert!(extract_mcp_tool_result(input, LIMITS.with_depth(5)).is_ok());
    assert_limit(extract_mcp_tool_result(input, LIMITS.with_depth(3)), true);
    let input = r#"{"content":[{"type":"text","text":"not [[[[0]]]]"}]}"#;
    assert!(extract_mcp_tool_result(input, LIMITS.with_depth(3)).is_ok());
}

#[test]
fn coercion_and_result_bytes_are_bounded() {
    let json = r#"{"content":[{"type":"text","text":[{},{},{},{},{},{},{}]}]}"#;
    assert!(extract_mcp_tool_result(json, LIMITS).is_ok());
    assert_limit(
        extract_mcp_tool_result(json, LIMITS.with_result_size(json.len())),
        false,
    );
    assert_limit(
        extract_mcp_tool_result(json, LIMITS.with_work(json.len())),
        false,
    );

    let plain = r#"{"content":[{"type":"text","text":[{},{},{},{},{},{},{},{},{},{},{}]}]}"#;
    // 11 "[object Object]" entries, 10 commas and 2 surrounding JSON quotes.
    let output_bytes = 11 * 15 + 10 + 2;
    assert!(extract_mcp_tool_result(plain, LIMITS.with_result_size(output_bytes)).is_ok());
    assert_limit(
        extract_mcp_tool_result(plain, LIMITS.with_result_size(output_bytes - 1)),
        false,
    );

    let error = plain.replacen('{', r#"{"isError":true,"#, 1);
    assert!(extract_mcp_tool_result(&error, LIMITS.with_result_size(output_bytes)).is_ok());
    assert_limit(
        extract_mcp_tool_result(&error, LIMITS.with_result_size(output_bytes - 1)),
        false,
    );
}

#[test]
fn joined_text_is_bounded_even_if_the_parsed_result_would_fit() {
    // JSON parsing discards the first property value because the key repeats.
    // The array of objects expands into that discarded value during joining.
    let input = r#"{"content":[{"type":"text","text":"{\"a\":\""},{"type":"text","text":[{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}]},{"type":"text","text":"\",\"a\":0}"}]}"#;
    assert!(extract_mcp_tool_result(input, LIMITS).is_ok());
    assert_limit(
        extract_mcp_tool_result(input, LIMITS.with_result_size(input.len())),
        false,
    );
}

#[test]
fn malformed_response_json_is_a_typed_error() {
    for input in ["", "{", r#"{"content":[],"#, "[] {}"] {
        assert!(matches!(
            extract_mcp_tool_result(input, LIMITS),
            Err(McpResponseError::InvalidJson(_))
        ));
    }
}

#[test]
fn deep_complete_and_malformed_responses_are_stack_safe() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let levels = 12_000;
            let nested = format!("{}null{}", "[".repeat(levels), "]".repeat(levels));
            let input = format!(r#"{{"ignored":{nested}}}"#);
            assert_limit(extract_mcp_tool_result(&input, LIMITS), true);
            let raised = LIMITS.with_depth(levels + 2);
            assert!(extract_mcp_tool_result(&input, raised).is_ok());
            let malformed = format!(r#"{{"ignored":[{nested}!"#);
            assert!(matches!(
                extract_mcp_tool_result(&malformed, raised),
                Err(McpResponseError::InvalidJson(_))
            ));
            let duplicate = format!(r#"{{"ignored":{nested},"ignored":null}}"#);
            assert!(extract_mcp_tool_result(&duplicate, raised).is_ok());
        })
        .unwrap()
        .join()
        .unwrap();
}
