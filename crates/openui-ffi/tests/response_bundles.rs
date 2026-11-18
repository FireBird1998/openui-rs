use openui_ffi::{OpenUiError, parse_response_bundle};
use serde_json::Value;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn raw(cases: &Value, name: &str) -> String {
    cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()["raw"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn the_native_binding_reads_stored_and_streamed_bundles() {
    let cases: Value =
        serde_json::from_str(include_str!("../../../conformance/response-bundles.json")).unwrap();
    let golden: Value = serde_json::from_str(include_str!(
        "../../../conformance/golden/response-bundles.json"
    ))
    .unwrap();
    let name = "crlf";
    let raw = raw(&cases, name);
    let read = |text, streaming| {
        let json = parse_response_bundle(text, streaming).unwrap();
        let (prefix, revision) = json.rsplit_once(",\"scriptRevision\":").unwrap();
        assert!(serde_json::from_str::<String>(revision.strip_suffix('}').unwrap()).is_ok());
        format!("{prefix},\"scriptRevision\":0}}")
    };
    assert_eq!(
        read(Some(raw.clone()), false),
        golden["cases"][name]["complete"].as_str().unwrap()
    );
    let (prefix, _) = raw.rsplit_once('\n').unwrap();
    assert_eq!(
        read(Some(prefix.into()), true),
        golden["cases"][name]["cuts"][prefix.encode_utf16().count()]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        parse_response_bundle(None, true).unwrap(),
        golden["cases"]["null"]["cuts"][0].as_str().unwrap()
    );
}

#[test]
fn the_native_binding_preserves_revision_equality() {
    let cases: Value =
        serde_json::from_str(include_str!("../../../conformance/response-bundles.json")).unwrap();
    let revision = |name| {
        let json = parse_response_bundle(Some(raw(&cases, name)), false).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        parsed["scriptRevision"].as_str().unwrap().to_owned()
    };
    assert_eq!(
        revision("revision-original-order"),
        revision("revision-reordered")
    );
    assert_ne!(
        revision("revision-original-order"),
        revision("revision-changed-code")
    );
}

#[test]
fn the_native_binding_returns_typed_bundle_limit_errors() {
    let oversized = "x".repeat(openui_lang::MAX_PROGRAM_SIZE + 1);
    let too_deep = format!("]]>openui:scripts {{\"count\":0}}\n{}", "[".repeat(801));
    for (raw, suffix) in [(oversized, "program-size limit"), (too_deep, "depth limit")] {
        let error = parse_response_bundle(Some(raw), true).unwrap_err();
        assert!(matches!(error, OpenUiError::LimitExceeded { reason } if reason.ends_with(suffix)));
    }
}
