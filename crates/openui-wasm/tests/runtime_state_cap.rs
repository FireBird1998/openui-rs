//! Store cap boundaries through exported binding calls, without another store.
mod support;
use std::sync::Arc;
use support::*;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

const CAP: usize = 1024;
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#;
const MARKER: &str =
    r#"{"__reactive":"assign","target":"$x","expr":{"k":"StateRef","n":"$value"}}"#;

fn session(cap: usize, source: &str) -> Arc<RuntimeSession> {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            evaluation: ParseLimits {
                result_size: Some(cap as u64),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    session
        .set_program(parse(source.into(), library, None).program)
        .unwrap();
    assert!(session.initialize(None).unwrap().refused.is_empty());
    session
}
fn action(session: &RuntimeSession) -> Arc<ActionHandle> {
    let tree = session.evaluate().unwrap();
    assert!(!tree.size_limited && !tree.depth_limited);
    session
        .action_at(tree.evaluation, vec!["props".into(), "a".into()])
        .unwrap()
}
fn observe(session: &RuntimeSession) -> (String, u64, Vec<bool>) {
    (
        session.snapshot().unwrap(),
        session.version().unwrap(),
        ["$x", "$y", "$padding", "a", "b", "c", "d"]
            .iter()
            .map(|s| session.is_pristine((*s).into()).unwrap())
            .collect(),
    )
}
fn refusal(error: OpenUiError, cap: usize) {
    let OpenUiError::RefusedWrite { reason } = error else {
        panic!("wrong refusal: {error:?}")
    };
    assert!(reason.contains("state cap"), "cap is not named: {reason}");
    assert!(
        reason.contains(&cap.to_string()),
        "configured cap is absent: {reason}"
    );
}
fn fill(session: &RuntimeSession, final_fields: &str) -> String {
    let base = format!(r#"{{{final_fields},"$padding":""}}"#);
    let padding = "p".repeat(CAP - base.len());
    session
        .set(
            "$padding".into(),
            Some(serde_json::to_string(&padding).unwrap()),
        )
        .unwrap();
    format!(r#"{{{final_fields},"$padding":"{padding}"}}"#)
}

#[test]
fn set_form_binding_action_and_persisted_writes_share_the_exact_total_state_edge() {
    let value = serde_json::to_string(&"v".repeat(32)).unwrap();
    for route in [
        "set",
        "field",
        "named_field",
        "binding",
        "action",
        "persisted",
    ] {
        for cap in [CAP, CAP - 1] {
            let source = if route == "action" {
                format!("$x=0\nroot=Box(@Set($x,{value}))")
            } else {
                "$x=0\nroot=Box(1)".into()
            };
            let session = session(cap, &source);
            let handle = (route == "action").then(|| action(&session));
            let target = match route {
                "field" => format!(r#"{{"value":{value},"componentType":"Field"}}"#),
                "named_field" => {
                    format!(r#"{{"input":{{"value":{value},"componentType":"Field"}}}}"#)
                }
                _ => value.clone(),
            };
            let expected = fill(&session, &format!(r#""$x":{target}"#));
            let before = observe(&session);
            assert!(before.0.len() <= cap);
            assert!(session.is_pristine("$x".into()).unwrap());
            let result = match route {
                "set" => session
                    .set("$x".into(), Some(value.clone()))
                    .map(|v| assert!(v)),
                "field" | "named_field" => session
                    .set_field(
                        (route == "named_field").then(|| "$x".into()),
                        Some("Field".into()),
                        if route == "field" { "$x" } else { "input" }.into(),
                        Some(value.clone()),
                        true,
                    )
                    .map(|v| assert!(v.changed && v.save_requested)),
                "binding" => session
                    .write_binding(MARKER.into(), value.clone())
                    .map(|v| assert!(v)),
                "action" => {
                    let output = session.run(handle, "".into(), None).unwrap();
                    assert!(output.events.is_empty() && output.continuation.is_none());
                    if let Some(failure) = output.failure {
                        assert!(output.size_limited);
                        assert_eq!(failure.step, Some(0));
                        Err(failure.error)
                    } else {
                        assert!(!output.size_limited && !output.depth_limited);
                        Ok(())
                    }
                }
                _ => {
                    let initialized = session
                        .initialize(Some(format!(r#"{{"$x":{value}}}"#)))
                        .unwrap();
                    if initialized.refused.is_empty() {
                        assert!(initialized.changed);
                        Ok(())
                    } else {
                        assert!(!initialized.changed);
                        assert_eq!(initialized.refused.len(), 1);
                        let entry = initialized.refused.into_iter().next().unwrap();
                        assert_eq!(entry.name, "$x");
                        Err(entry.error)
                    }
                }
            };
            if cap == CAP {
                result.unwrap();
                assert_eq!(session.snapshot().unwrap(), expected);
                assert_eq!(session.version().unwrap(), before.1 + 1);
                assert!(!session.is_pristine("$x".into()).unwrap());
            } else {
                refusal(result.unwrap_err(), cap);
                assert_eq!(observe(&session), before, "{route}");
            }
            assert!(session.snapshot().unwrap().len() <= cap);
        }
    }
}

#[test]
fn reset_preflights_all_targets_and_preserves_values_version_and_pristine_on_refusal() {
    let x = serde_json::to_string(&"v".repeat(16)).unwrap();
    let y = serde_json::to_string(&"w".repeat(16)).unwrap();
    for cap in [CAP, CAP - 1] {
        let session = session(cap, &format!("$x={x}\n$y={y}\nroot=Box(@Reset($x,$y))"));
        let handle = action(&session);
        session.set("$x".into(), Some("0".into())).unwrap();
        session.set("$y".into(), Some("0".into())).unwrap();
        let expected = fill(&session, &format!(r#""$x":{x},"$y":{y}"#));
        let before = observe(&session);
        let result = session.run(Some(handle), "".into(), None).unwrap();
        if cap == CAP {
            assert!(result.failure.is_none());
            assert_eq!(session.snapshot().unwrap(), expected);
            assert_eq!(session.version().unwrap(), before.1 + 2);
            assert!(
                session.is_pristine("$x".into()).unwrap()
                    && session.is_pristine("$y".into()).unwrap()
            );
        } else {
            assert!(result.size_limited);
            refusal(result.failure.unwrap().error, cap);
            assert_eq!(observe(&session), before);
        }
        assert!(session.snapshot().unwrap().len() <= cap);
    }
}

#[test]
fn resume_admits_against_current_state_and_consumes_a_refused_outcome() {
    let value = serde_json::to_string(&"v".repeat(32)).unwrap();
    for cap in [CAP, CAP - 1] {
        let session = session(
            cap,
            &format!("$x=0\nm=Mutation(\"m\",{{}})\nroot=Box(Action([@Run(m),@Set($x,{value})]))"),
        );
        let pending = session
            .run(Some(action(&session)), "".into(), None)
            .unwrap()
            .continuation
            .unwrap();
        // Fill after suspension so resume must use the current total, not the
        // state size from when the action began.
        let expected = fill(&session, &format!(r#""$x":{value}"#));
        let before = observe(&session);
        let output = session.resume(pending.clone(), true).unwrap();
        assert!(output.events.is_empty() && output.continuation.is_none());
        if cap == CAP {
            assert!(output.failure.is_none());
            assert_eq!(session.snapshot().unwrap(), expected);
        } else {
            assert!(output.size_limited);
            refusal(output.failure.unwrap().error, cap);
            assert_eq!(observe(&session), before);
        }
        let after = observe(&session);
        assert!(matches!(
            session.resume(pending, true),
            Err(OpenUiError::ConsumedHandle)
        ));
        assert_eq!(observe(&session), after);
        assert!(session.snapshot().unwrap().len() <= cap);
    }
}

#[test]
fn persisted_initialization_reports_each_refused_entry_and_loads_independent_values() {
    let session = session(64, "$x=0\nroot=Box(1)");
    let initial = session.version().unwrap();
    let huge = serde_json::to_string(&"z".repeat(80)).unwrap();
    let medium = serde_json::to_string(&"z".repeat(40)).unwrap();
    let output = session
        .initialize(Some(format!(
            r#"{{"$x":{huge},"a":1,"b":{huge},"c":2,"d":{medium}}}"#
        )))
        .unwrap();
    assert!(output.changed);
    assert_eq!(
        output
            .refused
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["$x", "b", "d"]
    );
    for entry in output.refused {
        refusal(entry.error, 64);
    }
    assert_eq!(session.snapshot().unwrap(), r#"{"$x":0,"a":1,"c":2}"#);
    assert_eq!(session.version().unwrap(), initial + 1);
    assert!(session.is_pristine("$x".into()).unwrap());
    let before = observe(&session);
    let output = session
        .initialize(Some(format!(r#"{{"$x":{huge},"b":{huge},"d":{huge}}}"#)))
        .unwrap();
    assert!(!output.changed);
    assert_eq!(output.refused.len(), 3);
    for entry in output.refused {
        refusal(entry.error, 64);
    }
    assert_eq!(observe(&session), before);
    assert!(session.snapshot().unwrap().len() <= 64);
}

#[test]
fn state_cap_uses_the_evaluation_ceiling_independently_of_scaled_drawing_room() {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::with_limits(
        library,
        None,
        RuntimeLimits {
            evaluation: ParseLimits {
                result_floor: Some(64),
                amplification: Some(0),
                result_ceiling: Some(CAP as u64),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    session
        .set(
            "$x".into(),
            Some(serde_json::to_string(&"v".repeat(CAP - 9)).unwrap()),
        )
        .unwrap();
    assert_eq!(session.snapshot().unwrap().len(), CAP);
    let before = observe(&session);
    refusal(
        session.set("other".into(), Some("0".into())).unwrap_err(),
        CAP,
    );
    assert_eq!(observe(&session), before);
}
