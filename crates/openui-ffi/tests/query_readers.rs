//! The native worker exposes a bounded reader cycle through its regular drain.
use openui_ffi::*;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn native_reader_cycle_reports_a_cut_once_and_preserves_the_final_snapshot() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../conformance/query-readers.json")).unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "cached-cycle-stops-at-the-statement-round-bound")
        .unwrap();
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session.attach_tool_provider().unwrap();
    let replace = |source: &str| {
        let p = parse(format!("{source}\nroot=Box(a)"), library.clone(), None).program;
        session.replace_program(p, None, None, true).unwrap()
    };
    replace(case["program"].as_str().unwrap());
    let initial = session.take_query_update().unwrap();
    assert!(!initial.reader_rounds_limited);
    let steps = case["steps"].as_array().unwrap();
    for step in steps.iter().filter(|s| s["kind"] == "complete") {
        let i = usize::try_from(step["call"].as_u64().unwrap()).unwrap();
        let call = initial.calls.get(i).unwrap();
        let outcome = serde_json::json!({"value":step["outcome"]["value"]});
        session
            .complete_tool_call(call.clone(), outcome.to_string())
            .unwrap();
    }
    let edit = steps.iter().find(|s| s["kind"] == "replace").unwrap();
    let draw = replace(edit["program"].as_str().unwrap());
    assert!(draw.evaluation.json.contains(r#""value":1"#));
    let update = session.take_query_update().unwrap();
    assert!(update.reader_rounds_limited && update.calls.is_empty());
    assert!(!session.take_query_update().unwrap().reader_rounds_limited);
}
