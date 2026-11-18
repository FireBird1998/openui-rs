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
        Err(OpenUiError::InvalidJson { .. })
    ));
}
