//! The native worker owns provider-managed plans and rejects late completions.
use openui_ffi::*;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn native_completion_drains_the_resumed_event_and_disposal_revokes_the_next_call() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"action":{}}}}}"#.into()).unwrap();
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    s.attach_tool_provider().unwrap();
    let p = parse(
        "m=Mutation(\"save\",{})\nroot=Box(Action([@Run(m),@OpenUrl(\"done\")]))".into(),
        library,
        None,
    )
    .program;
    let draw = s.replace_program(p, None, None, true).unwrap().evaluation;
    let button = s
        .action_at(draw.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    for round in 0..2 {
        let out = s.run(Some(button.clone()), "go".into(), None).unwrap();
        assert!(out.events.is_empty() && out.continuation.is_none());
        let call = s.take_query_update().unwrap().calls.remove(0);
        assert_eq!(call.tool_name(), "save");
        if round == 1 {
            s.dispose();
            assert!(matches!(
                s.complete_tool_call(call, "{\"value\":1}".into()),
                Err(OpenUiError::Disposed)
            ));
            break;
        }
        s.complete_tool_call(call, "{\"value\":1}".into()).unwrap();
        let update = s.take_query_update().unwrap();
        assert!(update.changed && update.actions.len() == 1);
        assert!(update.actions[0].events[0].contains("\"url\":\"done\""));
        assert!(s.take_query_update().unwrap().actions.is_empty());
    }
}

#[test]
fn native_without_provider_still_hands_off_the_mutation() {
    let lib =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"action":{}}}}}"#.into()).unwrap();
    let s = RuntimeSession::new(lib.clone(), None).unwrap();
    let draw = s
        .replace_program(
            parse(
                "m=Mutation(\"post\",{x:3})\nroot=Box(Action([@Run(m),@OpenUrl(\"done\")]))".into(),
                lib,
                None,
            )
            .program,
            None,
            None,
            true,
        )
        .unwrap()
        .evaluation;
    let button = s
        .action_at(draw.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    let run = s.run(Some(button), "click".into(), None).unwrap();
    assert!(run.events[0].contains(r#""refType":"mutation","args":{"x":3}"#));
    let tail = s.resume(run.continuation.unwrap(), true).unwrap();
    assert!(tail.events[0].contains("done"));
    let effects = s.take_query_update().unwrap();
    assert!(effects.actions.is_empty() && effects.calls.is_empty());
}
