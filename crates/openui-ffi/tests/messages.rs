use openui_ffi::{OpenUiError, build_message, parse_message};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn message_text_and_parts_cross_the_native_binding() {
    let golden =
        openui_lang::parse_json(include_str!("../../../conformance/golden/messages.json")).unwrap();
    let cases =
        openui_lang::parse_json(include_str!("../../../conformance/messages.json")).unwrap();
    let case = cases.as_array().unwrap().first().unwrap();
    let expected = &golden[case["name"].as_str().unwrap()];
    let parts = openui_lang::stringify_json(&case["build"]);
    let written = build_message(parts).unwrap();
    assert_eq!(Some(written.as_str()), expected["text"].as_str());
    let parsed = parse_message(written, false).unwrap();
    assert_eq!(Some(parsed.as_str()), expected["read"]["complete"].as_str());
    let too_long = "x".repeat(openui_lang::MAX_PROGRAM_SIZE + 1);
    assert!(matches!(
        parse_message(too_long, true),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert!(matches!(
        build_message(r#"{"content":"body","attributes":{"x":"\ud800"}}"#.into()),
        Err(OpenUiError::InvalidAttribute { .. })
    ));
}

#[test]
fn message_attribute_encoding_error_has_its_own_kind() {
    // Upstream's URIError: a lone surrogate in an attribute's value or name.
    for parts in [
        r#"{"content":"body","attributes":{"x":"\ud800"}}"#,
        r#"{"content":"body","attributes":{"\udfff":"x"}}"#,
    ] {
        let error = build_message(parts.into()).unwrap_err();
        assert!(
            matches!(
                &error,
                OpenUiError::InvalidAttribute { reason }
                    if reason == "cannot URI-encode message attribute containing a lone surrogate or reserved scalar"
            ),
            "{parts}: {error:?}"
        );
    }
    // Malformed parts are still InvalidJson, and a valid pair still encodes.
    for parts in [
        r#"{"content":1}"#,
        "[",
        r#"{"content":"body","attributes":{"x":1}}"#,
    ] {
        assert!(
            matches!(
                build_message(parts.into()),
                Err(OpenUiError::InvalidJson { .. })
            ),
            "{parts}"
        );
    }
    assert!(
        build_message(r#"{"content":"body","attributes":{"x":"\ud83d\ude00"}}"#.into()).is_ok()
    );
}
