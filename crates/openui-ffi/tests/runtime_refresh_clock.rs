//! The native driver needs one clock and the existing effects drain.
use openui_ffi::*;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn native_driver_pauses_and_rearms_one_deadline_on_resume() {
    let library = ComponentLibrary::new(r#"{"$defs":{}}"#.into()).unwrap();
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    s.attach_tool_provider_with_minimum(50).unwrap();
    let p = parse("q=Query(\"read\",{},null,0.01)".into(), library, None).program;
    s.replace_program(p, None, None, false).unwrap();
    let first = s.take_query_update().unwrap();
    assert_eq!(first.next_refresh_due_ms, Some(50));
    s.complete_tool_call(first.calls[0].clone(), r#"{"value":3}"#.into())
        .unwrap();
    s.advance_query_clock(50).unwrap();
    let tick = s.take_query_update().unwrap();
    assert_eq!(tick.calls.len(), 1);
    s.pause_queries().unwrap();
    assert_eq!(s.take_query_update().unwrap().next_refresh_due_ms, None);
    s.advance_query_clock(1000).unwrap();
    s.complete_tool_call(tick.calls[0].clone(), r#"{"failed":"late"}"#.into())
        .unwrap();
    s.resume_queries().unwrap();
    let resumed = s.take_query_update().unwrap();
    assert_eq!(resumed.next_refresh_due_ms, Some(1050));
    assert!(resumed.calls.is_empty());
    s.advance_query_clock(1050).unwrap();
    let wanted = s.take_query_update().unwrap();
    assert_eq!(wanted.calls.len(), 1);
    s.dispose();
    assert!(matches!(
        s.advance_query_clock(1100),
        Err(OpenUiError::Disposed)
    ));
}
