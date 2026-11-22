//! Replacement is one transaction across program, store and private marks.
use openui_lang::{JsObject, Library};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use std::sync::{Arc, Mutex};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn library() -> Arc<ComponentLibrary> {
    let schema = serde_json::json!({"$defs":{"Box":{"properties":{"value":{},"action":{}}}},
        "functions":{"Echo":{"params":{"type":"object","properties":{}}}},
        "actions":{"Go":{"params":{"type":"object","properties":{}}}}});
    Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
        defined: None,
    })
}
fn program(library: &ComponentLibrary, source: &str) -> Arc<ParsedProgram> {
    let inner = Arc::new(openui_lang::parse(source, &library.inner, None));
    Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    })
}
fn path() -> Vec<String> {
    vec!["props".into(), "action".into()]
}

#[test]
fn refused_replacement_preserves_program_store_bindings_handles_and_registrations() {
    let library = library();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut session = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let calls = Arc::new(Mutex::new(0));
    let seen = calls.clone();
    session
        .register_function(
            "Echo".into(),
            Arc::new(move |_| {
                *seen.lock().unwrap() += 1;
                Ok(Some("7".into()))
            }),
        )
        .unwrap();
    session
        .mark_binding(&refs, "Box".into(), "value".into())
        .unwrap();
    let old = program(&library, "$n=1\n$keep=2\n$edited=0\nroot=Box($n,@Go())");
    let before = session
        .replace_program(
            &refs,
            old.clone(),
            Some("{\"$edited\":5}".into()),
            None,
            true,
        )
        .unwrap();
    let action = session
        .action_at(&refs, before.evaluation.evaluation.clone(), path())
        .unwrap();
    let oversized = Arc::new(ParsedProgram {
        inner: old.inner.clone(),
        json_bytes: u64::MAX,
    });
    let candidate = program(&library, "$n=99\n$new=3\nroot=Box(@Echo(),@Go())");
    // Invalid program, invalid initialization/projection, then failed display
    // admission after initialization AND evaluation have already run.
    let scalar = program(
        &library,
        r#"$n=99
$new=3
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Box()"#,
    );
    for (p, persisted, projected, native) in [
        (oversized, None, None, true),
        (candidate.clone(), Some("[]".to_owned()), None, true),
        (candidate, None, Some("[]".to_owned()), true),
        (
            scalar,
            Some("{\"$extra\":8}".to_owned()),
            Some("{\"$n\":77}".to_owned()),
            true,
        ),
    ] {
        assert!(
            session
                .replace_program(&refs, p, persisted, projected, native)
                .is_err()
        );
        assert_eq!(session.snapshot(&refs).unwrap(), before.snapshot);
        assert_eq!(session.version(&refs).unwrap(), before.version);
        for name in ["$n", "$keep", "$edited", "$new", "$extra"] {
            assert_eq!(
                session.is_pristine(&refs, name.into()).unwrap(),
                matches!(name, "$n" | "$keep")
            );
        }
        assert_eq!(session.pending_continuations(&refs).unwrap(), 0);
        let result = session
            .run(&refs, Some(action.clone()), "go".into(), None)
            .unwrap();
        assert!(result.failure.is_none());
        assert!(result.events.first().unwrap().contains("Go"));
        assert!(session.undecided_bindings(&refs).unwrap().is_empty());
    }
    session.set(&refs, "$n".into(), Some("2".into())).unwrap();
    let after = session.evaluate(&refs).unwrap();
    assert!(after.json.contains("\"target\":\"$n\""));
    assert!(!after.json.contains("$new"));
    assert_eq!(*calls.lock().unwrap(), 0);
    session
        .replace_program(
            &refs,
            program(&library, "root=Box(@Echo())"),
            None,
            None,
            true,
        )
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn replacement_matches_separate_operations_including_batch_refusals_and_cuts() {
    for work in [None, Some(0), Some(40), Some(1000)] {
        let library = library();
        let values = JsObject::new();
        let refs = PreparedReferenceValues::new(&values);
        let limits = RuntimeLimits {
            evaluation: ParseLimits {
                work,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut atomic = SessionCore::new(library.clone(), None, limits.clone()).unwrap();
        let mut separate = SessionCore::new(library.clone(), None, limits).unwrap();
        let p = program(&library, "$n=1\nroot=Box($n+1,@Go())");
        let persisted = Some("{\"$n\":2}".to_owned());
        let projected = Some("{\"edited\":3}".to_owned());
        let a = atomic
            .replace_program(
                &refs,
                p.clone(),
                persisted.clone(),
                projected.clone(),
                false,
            )
            .unwrap();
        separate.set_program(&refs, p).unwrap();
        let projection = separate.initialize(&refs, projected).unwrap();
        let initialized = separate.initialize(&refs, persisted).unwrap();
        let b = separate.evaluate(&refs).unwrap();
        assert_eq!(a.evaluation.json, b.json);
        assert_eq!(
            (a.evaluation.size_limited, a.evaluation.depth_limited),
            (b.size_limited, b.depth_limited)
        );
        assert_eq!(a.snapshot, separate.snapshot(&refs).unwrap());
        assert_eq!(a.version, separate.version(&refs).unwrap());
        for name in ["$n", "edited", "$absent"] {
            assert_eq!(
                atomic.is_pristine(&refs, name.into()).unwrap(),
                separate.is_pristine(&refs, name.into()).unwrap()
            );
        }
        assert_eq!(
            atomic.pending_continuations(&refs).unwrap(),
            separate.pending_continuations(&refs).unwrap()
        );
        assert_eq!(a.initialization.changed, initialized.changed);
        assert_eq!(a.projection.changed, projection.changed);
        assert_eq!(
            format!("{:?}", a.initialization.refused),
            format!("{:?}", initialized.refused)
        );
        assert_eq!(
            format!("{:?}", a.projection.refused),
            format!("{:?}", projection.refused)
        );
    }
}

#[test]
fn native_display_admission_is_opt_in() {
    // Native display admission is intentionally opt-in; arbitrary roots still
    // work for ordinary Rust/wasm callers and existing evaluate.
    let library = library();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut session = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let p = program(
        &library,
        r#"$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:42}
root=true ? $shape : Box()"#,
    );
    let result = session
        .replace_program(&refs, p, None, None, false)
        .unwrap();
    assert!(result.evaluation.json.contains("42"));
    assert_eq!(session.snapshot(&refs).unwrap(), result.snapshot);
}

#[test]
fn refused_replacement_preserves_suspended_work_room_and_results() {
    let mut saw_cut = false;
    let mut saw_complete = false;
    for work in [1000, 2000, 4000, 100_000] {
        let library = library();
        let mut values = JsObject::new();
        values.define(
            "q",
            openui_lang::Value::Arr(Arc::new(
                (0..300)
                    .map(|n| openui_lang::Value::Num(n as f64))
                    .collect(),
            )),
        );
        let refs = PreparedReferenceValues::new(&values);
        let limits = RuntimeLimits {
            evaluation: ParseLimits {
                work: Some(work),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut control = SessionCore::new(library.clone(), None, limits.clone()).unwrap();
        let mut subject = SessionCore::new(library.clone(), None, limits).unwrap();
        let p = program(
            &library,
            "$n=0\nq=Query(\"q\",{})\nm=Mutation(\"m\",{})\nroot=Box(0,Action([@Go(),@Run(m),@Set($n,@Each(q,i,i+1)),@Go()]))",
        );
        let mut pending = Vec::new();
        for session in [&mut control, &mut subject] {
            let out = session
                .replace_program(&refs, p.clone(), None, None, false)
                .unwrap();
            let action = session
                .action_at(&refs, out.evaluation.evaluation, path())
                .unwrap();
            pending.push(session.run(&refs, Some(action), "go".into(), None).unwrap());
        }
        let invalid = program(
            &library,
            r#"$n=99
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Box()"#,
        );
        // Even a cut is a successful draw. Keep the early JSON refusal at
        // every allowance, then exercise post-evaluation refusal across the
        // work settings that retain a mutation continuation.
        assert!(
            subject
                .replace_program(&refs, invalid.clone(), Some("[]".into()), None, true)
                .is_err()
        );
        if work >= 2000 {
            assert!(pending[1].continuation.is_some());
            assert!(
                subject
                    .replace_program(&refs, invalid, None, None, true)
                    .is_err()
            );
            assert_eq!(
                subject.snapshot(&refs).unwrap(),
                control.snapshot(&refs).unwrap()
            );
            assert_eq!(
                subject.version(&refs).unwrap(),
                control.version(&refs).unwrap()
            );
            for name in ["$n", "$shape"] {
                assert_eq!(
                    subject.is_pristine(&refs, name.into()).unwrap(),
                    control.is_pristine(&refs, name.into()).unwrap()
                );
            }
            assert_eq!(
                subject.pending_continuations(&refs).unwrap(),
                control.pending_continuations(&refs).unwrap()
            );
        }
        for (session, out) in [
            (&mut control, pending.remove(0)),
            (&mut subject, pending.remove(0)),
        ] {
            if let Some(token) = out.continuation {
                let resumed = session.resume(&refs, token, true).unwrap();
                pending.push(resumed);
            } else {
                pending.push(out);
            }
        }
        let a = &pending[0];
        let b = &pending[1];
        saw_cut |= a.size_limited;
        saw_complete |= !a.size_limited && a.failure.is_none();
        assert_eq!(a.events, b.events);
        assert_eq!(
            (a.size_limited, a.depth_limited),
            (b.size_limited, b.depth_limited)
        );
        assert_eq!(format!("{:?}", a.failure), format!("{:?}", b.failure));
        assert_eq!(
            subject.snapshot(&refs).unwrap(),
            control.snapshot(&refs).unwrap()
        );
        assert_eq!(
            subject.version(&refs).unwrap(),
            control.version(&refs).unwrap()
        );
    }
    assert!(
        saw_cut && saw_complete,
        "work controls must exercise both cuts and completion"
    );
}

#[test]
fn native_json_depth_admission_accounts_for_the_envelope_and_escaped_strings() {
    let library = library();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let limits = RuntimeLimits {
        evaluation: ParseLimits {
            depth: Some(2000),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut session = SessionCore::new(library.clone(), None, limits).unwrap();
    let old = session
        .replace_program(
            &refs,
            program(&library, "root=Box(1,@Go())"),
            None,
            None,
            true,
        )
        .unwrap();
    for depth in [510, 511, 512, 600] {
        let nested = format!(
            "{}\"[\\\"quoted\\\"]\"{}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        // Supply to the snapshot independently of tree evaluation, so this
        // checks admission of BOTH exported JSON strings.
        let persisted = format!("{{\"deep\":{nested}}}");
        let result = session.replace_program(
            &refs,
            program(&library, "root=Box(2)"),
            Some(persisted),
            None,
            true,
        );
        if depth >= 511 {
            assert!(result.is_err());
            assert_eq!(session.snapshot(&refs).unwrap(), old.snapshot);
        } else {
            assert!(result.is_ok());
            // Reset to the same original store for the next independent edge.
            session = SessionCore::new(
                library.clone(),
                None,
                RuntimeLimits {
                    evaluation: ParseLimits {
                        depth: Some(2000),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            session
                .replace_program(
                    &refs,
                    program(&library, "root=Box(1,@Go())"),
                    None,
                    None,
                    true,
                )
                .unwrap();
        }
    }
}

#[test]
fn a_refused_candidate_cannot_mint_custom_authority_in_the_live_store() {
    let library = library();
    let parsed = openui_lang::parse("root=Box(0,@Go())", &library.inner, None);
    let actions = library.schema.get("actions").unwrap();
    let input = || openui_runtime::TreeInput::new(&parsed).with_actions(actions);
    let mut store = openui_runtime::Store::new();
    let mut leaked = None;
    let empty = JsObject::new();
    let result = store.initialize_transaction(&empty, &empty, None, |candidate| {
        let tree = openui_runtime::evaluate_tree(input(), candidate);
        let Some(openui_lang::Value::Element(root)) = tree.root.as_ref() else {
            panic!("missing root")
        };
        leaked = root.props.get("action").cloned();
        Err::<(), ()>(())
    });
    assert!(result.is_err());
    let refused = openui_runtime::run_action(input(), &mut store, leaked.as_ref(), "go", None);
    assert!(refused.events.is_empty());
    let committed: Result<_, ()> =
        store.initialize_transaction(&empty, &empty, None, |candidate| {
            let tree = openui_runtime::evaluate_tree(input(), candidate);
            let Some(openui_lang::Value::Element(root)) = tree.root.as_ref() else {
                panic!("missing root")
            };
            Ok(root.props.get("action").cloned().unwrap())
        });
    let accepted =
        openui_runtime::run_action(input(), &mut store, Some(&committed.unwrap().2), "go", None);
    assert!(accepted.failure.is_none() && accepted.events.len() == 1);
}

#[test]
fn a_refused_replacement_does_not_drain_abandoned_continuations() {
    let library = library();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut session = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let old = program(&library, "m=Mutation(\"m\",{})\nroot=Box(0,@Run(m))");
    let tree = session
        .replace_program(&refs, old.clone(), None, None, true)
        .unwrap();
    let action = session
        .action_at(&refs, tree.evaluation.evaluation, path())
        .unwrap();
    let run = session
        .run(&refs, Some(action), "run".into(), None)
        .unwrap();
    let token = run.continuation.unwrap();
    drop(token);
    assert!(
        session
            .replace_program(&refs, old, Some("[]".into()), None, true)
            .is_err()
    );
    assert_eq!(session.drain_releases(), 1);
}

#[test]
fn persisted_admission_keeps_swift_shows_error_precedence() {
    let library = library();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut session = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let old = session
        .replace_program(&refs, program(&library, "root=Box(1)"), None, None, true)
        .unwrap();
    let oversized = Arc::new(ParsedProgram {
        inner: program(&library, "root=Box(2)").inner.clone(),
        json_bytes: u64::MAX,
    });
    assert!(matches!(
        session.replace_program(&refs, oversized, Some("[]".into()), None, true),
        Err(OpenUiError::InvalidJson { .. })
    ));
    assert_eq!(session.snapshot(&refs).unwrap(), old.snapshot);
    assert_eq!(session.version(&refs).unwrap(), old.version);
}

#[test]
fn post_evaluation_refusal_preserves_pending_mutation_and_all_store_marks() {
    let library = library();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    let mut control = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let mut subject = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let old = program(
        &library,
        "$n=1\n$pristine=2\nm=Mutation(\"m\",{})\nroot=Box($n,Action([@Run(m),@Set($n,$n+1),@OpenUrl(\"done\")]))",
    );
    let mut tokens = Vec::new();
    for session in [&mut control, &mut subject] {
        let shown = session
            .replace_program(&refs, old.clone(), None, None, true)
            .unwrap();
        session.set(&refs, "$n".into(), Some("5".into())).unwrap();
        session
            .set_field(
                &refs,
                Some("f".into()),
                None,
                "x".into(),
                Some("7".into()),
                false,
            )
            .unwrap();
        let action = session
            .action_at(
                &refs,
                shown.evaluation.evaluation,
                vec!["props".into(), "action".into()],
            )
            .unwrap();
        let run = session.run(&refs, Some(action), "go".into(), None).unwrap();
        assert!(!run.size_limited && !run.depth_limited && run.failure.is_none());
        assert_eq!(run.events.len(), 1);
        tokens.push(run.continuation.unwrap());
        assert_eq!(session.pending_continuations(&refs).unwrap(), 1);
    }
    let candidate = program(
        &library,
        r#"$n=99
$pristine=88
$new=3
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Box()"#,
    );
    assert!(matches!(
        subject
            .replace_program(
                &refs,
                candidate,
                Some("{\"$n\":99,\"$extra\":8}".into()),
                Some("{\"$pristine\":77}".into()),
                true,
            ),
        Err(OpenUiError::Evaluation { reason }) if reason == "native view root is not an element"
    ));
    assert_eq!(
        subject.snapshot(&refs).unwrap(),
        control.snapshot(&refs).unwrap()
    );
    assert_eq!(
        subject.version(&refs).unwrap(),
        control.version(&refs).unwrap()
    );
    for name in ["$n", "$pristine", "$new", "$extra", "$shape", "f"] {
        assert_eq!(
            subject.is_pristine(&refs, name.into()).unwrap(),
            control.is_pristine(&refs, name.into()).unwrap(),
            "{name}"
        );
    }
    assert!(!subject.is_pristine(&refs, "$n".into()).unwrap());
    assert!(subject.is_pristine(&refs, "$pristine".into()).unwrap());
    assert_eq!(subject.pending_continuations(&refs).unwrap(), 1);
    assert_eq!(control.pending_continuations(&refs).unwrap(), 1);
    assert_eq!(
        subject
            .get_field(&refs, Some("f".into()), "x".into())
            .unwrap(),
        Some("7".into())
    );
    assert_eq!(
        subject
            .get_field(&refs, Some("f".into()), "x".into())
            .unwrap(),
        control
            .get_field(&refs, Some("f".into()), "x".into())
            .unwrap()
    );
    let a = subject.evaluate(&refs).unwrap();
    let b = control.evaluate(&refs).unwrap();
    assert_eq!(a.json, b.json);
    assert_eq!(
        (a.size_limited, a.depth_limited),
        (b.size_limited, b.depth_limited)
    );
    let mut resumed = Vec::new();
    for (session, token) in [&mut control, &mut subject].into_iter().zip(tokens) {
        let result = session.resume(&refs, token, true).unwrap();
        assert_eq!(session.pending_continuations(&refs).unwrap(), 0);
        resumed.push((
            result.events,
            result.size_limited,
            result.depth_limited,
            format!("{:?}", result.failure),
            result.continuation.is_some(),
        ));
    }
    assert_eq!(resumed[0], resumed[1]);
    assert_eq!(
        subject.snapshot(&refs).unwrap(),
        control.snapshot(&refs).unwrap()
    );
    assert_eq!(
        subject.version(&refs).unwrap(),
        control.version(&refs).unwrap()
    );
    for session in [&mut control, &mut subject] {
        session.set(&refs, "$n".into(), Some("9".into())).unwrap();
    }
    assert_eq!(
        subject.snapshot(&refs).unwrap(),
        control.snapshot(&refs).unwrap()
    );
    assert_eq!(
        subject.version(&refs).unwrap(),
        control.version(&refs).unwrap()
    );
    assert_eq!(
        subject.evaluate(&refs).unwrap().json,
        control.evaluate(&refs).unwrap().json
    );
}
