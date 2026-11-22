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

/// #204: the session finishes or drops a staged delivery before the
/// completion returns, and the binding runs one command at a time, so no
/// program change runs while a delivery is staged and a late result cannot
/// settle a recreated query. Failure text that fits once but not at both of
/// `b`'s readers is refused after its delivery is staged; the program then
/// removes every query and registers `a` again.
#[test]
fn a_late_result_for_a_removed_query_leaves_its_recreated_query_loading() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let limits = RuntimeLimits {
        evaluation: ParseLimits {
            result_size: Some(256),
            ..ParseLimits::default()
        },
        ..RuntimeLimits::default()
    };
    let session = RuntimeSession::with_limits(library.clone(), None, limits).unwrap();
    session.attach_tool_provider().unwrap();
    let replace = |source: &str| {
        let program = parse(source.into(), library.clone(), None).program;
        session.replace_program(program, None, None, true).unwrap();
    };
    let query_snapshot = || session.evaluate().unwrap().query_snapshot_json.unwrap();
    replace("a = Query(\"a\")\nb = Query(\"b\")\nc = Query(\"b\")\nroot = Box([a, b, c])");
    let calls = session.take_query_update().unwrap().calls;
    let old = calls.iter().find(|c| c.tool_name == "a").unwrap().clone();
    let b = calls.iter().find(|c| c.tool_name == "b").unwrap().clone();
    let refused = format!(r#"{{"failed":"{}"}}"#, "x".repeat(200));
    assert!(matches!(
        session.complete_tool_call(b.clone(), refused),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert!(b.id.is_pending());
    replace("root = Box(1)");
    // Nothing the binding holds keeps the old calls alive.
    assert!(!old.id.is_pending() && !b.id.is_pending());
    replace("a = Query(\"a\")\nroot = Box(a)");
    let mut new = session.take_query_update().unwrap().calls;
    assert_eq!(new.len(), 1);
    let replacement = new.remove(0);
    let before = query_snapshot();

    session
        .complete_tool_call(old, r#"{"value":"stale"}"#.into())
        .unwrap();
    let late = session.take_query_update().unwrap();
    assert!(!late.changed && late.calls.is_empty());
    assert_eq!(query_snapshot(), before);
    session
        .complete_tool_call(replacement, r#"{"value":"fresh"}"#.into())
        .unwrap();
    let after: serde_json::Value = serde_json::from_str(&query_snapshot()).unwrap();
    assert_eq!(after["a"], "fresh");
    assert_eq!(after["__openui_loading"], serde_json::json!([]));
}
