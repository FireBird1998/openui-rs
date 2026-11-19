//! The WebAssembly binding's limit object sets the query cache cap, and a
//! result refused under a limit is the query's `tool-error` (#90).
mod support;
use support::*;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn wasm_limits_carry_the_cache_cap_and_refused_results_become_tool_errors() {
    let mut exported = openui_wasm::RuntimeLimits::new();
    assert_eq!(exported.max_cache_bytes(), None);
    exported.set_max_cache_bytes(Some(100));
    assert_eq!(exported.max_cache_bytes(), Some(100));
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let limits = RuntimeLimits {
        max_cache_bytes: Some(100),
        ..RuntimeLimits::default()
    };
    let session = RuntimeSession::with_limits(library.clone(), None, limits).unwrap();
    session.attach_tool_provider().unwrap();
    let program = parse(
        "a = Query(\"a\")\nb = Query(\"b\")\nroot = Box([a, b])".into(),
        library,
        None,
    )
    .program;
    session.replace_program(program, None, None, true).unwrap();
    let calls = session.take_query_update().unwrap().calls;
    let value = |bytes: usize| format!(r#"{{"value":"{}"}}"#, "x".repeat(bytes - 2));
    session
        .complete_tool_call(calls[0].clone(), value(40))
        .unwrap();
    session
        .complete_tool_call(calls[1].clone(), value(61))
        .unwrap();
    let draw = session.evaluate().unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_str(&draw.query_snapshot_json.unwrap()).unwrap();
    assert_eq!(snapshot["a"], "x".repeat(38));
    assert_eq!(
        snapshot["__openui_errors"][0]["message"],
        "Query \"b\" failed: tool result does not fit the cache limit of 100 bytes"
    );
}
