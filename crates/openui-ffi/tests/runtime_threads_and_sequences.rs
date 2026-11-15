//! Concurrency, capability lifetimes, and seeded sequences use exported APIs only.
use openui_ffi::*;
use std::sync::{Arc, Barrier, mpsc};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#;
const PROGRAM: &str =
    "$n=0\nm=Mutation(\"save\",{n:$n})\nroot=Box(@Set($n,$n+1),Action([@Run(m),@Set($n,$n+1)]))";
fn session() -> (Arc<RuntimeSession>, Arc<ComponentLibrary>) {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session
        .set_program(parse(PROGRAM.into(), library.clone(), None).program)
        .unwrap();
    session.initialize(None).unwrap();
    (session, library)
}
fn path(prop: &str) -> Vec<String> {
    vec!["props".into(), prop.into()]
}

#[test]
fn sixteen_threads_serialize_state_writes_and_action_runs() {
    let (session, _) = session();
    let tree = session.evaluate().unwrap();
    let action = session.action_at(tree.evaluation, path("a")).unwrap();
    let barrier = Arc::new(Barrier::new(17));
    let mut threads = Vec::new();
    for i in 0..16 {
        let session = session.clone();
        let action = action.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            for j in 0..200 {
                assert!(
                    session
                        .set(format!("thread{i}"), Some(j.to_string()))
                        .unwrap()
                );
                assert!(
                    session
                        .run(Some(action.clone()), "".into(), None)
                        .unwrap()
                        .failure
                        .is_none()
                );
                assert!(session.snapshot().unwrap().contains("$n"));
            }
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(session.get("$n".into()).unwrap().as_deref(), Some("3200"));
    assert_eq!(session.version().unwrap(), 6401);
    for i in 0..16 {
        assert_eq!(
            session.get(format!("thread{i}")).unwrap().as_deref(),
            Some("199")
        );
    }
}

#[test]
fn owners_can_drop_on_other_threads_while_calls_and_disposal_race() {
    for _ in 0..32 {
        let (session, _) = session();
        let tree = session.evaluate().unwrap();
        let action = session.action_at(tree.evaluation, path("b")).unwrap();
        let continuation = session
            .run(Some(action), "".into(), None)
            .unwrap()
            .continuation
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let caller = session.clone();
        let gate = barrier.clone();
        let thread = std::thread::spawn(move || {
            gate.wait();
            for _ in 0..100 {
                match caller.evaluate() {
                    Ok(_) => {}
                    Err(OpenUiError::Disposed) => break,
                    Err(e) => panic!("{e}"),
                }
            }
            // The last owner may be this thread; non-Send run state never is.
        });
        let disposer = session.clone();
        let gate = barrier.clone();
        let disposal = std::thread::spawn(move || {
            gate.wait();
            disposer.dispose();
        });
        barrier.wait();
        drop(session);
        drop(continuation);
        thread.join().unwrap();
        disposal.join().unwrap();
    }
}

#[test]
fn last_owner_drop_reclaims_pending_runs_on_a_different_thread() {
    let (session, _) = session();
    let tree = session.evaluate().unwrap();
    let action = session.action_at(tree.evaluation, path("b")).unwrap();
    let continuation = session
        .run(Some(action), "".into(), None)
        .unwrap()
        .continuation
        .unwrap();
    let weak = Arc::downgrade(&session);
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        drop(session);
        send.send(()).unwrap();
    })
    .join()
    .unwrap();
    recv.recv().unwrap();
    assert!(
        weak.upgrade().is_none(),
        "handles must not retain the session"
    );
    std::thread::spawn(move || drop(continuation))
        .join()
        .unwrap();
}

fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn seeded_exported_sequences_keep_refused_calls_atomic() {
    // Sixteen deterministic seeds, 500 operations each. Mix two sessions so
    // foreign capabilities occur naturally alongside consumed and stale ones.
    const STATE_CAP: u64 = 256;
    let mut state_refusals = 0;
    for initial in 1..=16 {
        let make_session = || {
            let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
            let session = RuntimeSession::with_limits(
                library.clone(),
                None,
                RuntimeLimits {
                    evaluation: ParseLimits {
                        result_size: Some(STATE_CAP),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            session
                .set_program(parse(PROGRAM.into(), library.clone(), None).program)
                .unwrap();
            assert!(session.initialize(None).unwrap().refused.is_empty());
            (session, library)
        };
        let (a, library) = make_session();
        let (b, _) = make_session();
        let sessions = [a, b];
        let mut actions = Vec::new();
        let mut continuations = Vec::new();
        let mut seed = initial;
        for step in 0..500 {
            let n = next(&mut seed);
            let i = (n % 2) as usize;
            let session = &sessions[i];
            let before = session.snapshot().unwrap();
            let version = session.version().unwrap();
            let names = [
                "$n", "$v0", "$v1", "$v2", "$v3", "$v4", "$v5", "$v6", "$v7", "form",
            ];
            let pristine: Vec<_> = names
                .iter()
                .map(|name| session.is_pristine((*name).into()).unwrap())
                .collect();
            let result: Result<(), OpenUiError> = match (n >> 8) % 12 {
                0 => session
                    .set(
                        format!("$v{}", n % 8),
                        Some(format!(
                            "{{\"n\":{step},\"text\":\"{}\"}}",
                            "x".repeat(((n >> 16) % 64) as usize)
                        )),
                    )
                    .map(|_| ()),
                1 => session.set("$n".into(), Some("{".into())).map(|_| ()),
                2 => session.get("$n".into()).map(|_| ()),
                3 => session
                    .evaluate()
                    .and_then(|tree| {
                        session.action_at(
                            tree.evaluation,
                            path(if n.is_multiple_of(3) { "b" } else { "a" }),
                        )
                    })
                    .map(|action| {
                        if actions.len() < 64 {
                            actions.push(action);
                        }
                    }),
                4 if !actions.is_empty() => session
                    .run(
                        Some(actions[(n as usize) % actions.len()].clone()),
                        "".into(),
                        None,
                    )
                    .and_then(|r| {
                        if let Some(failure) = r.failure {
                            return Err(failure.error);
                        }
                        if let Some(c) = r.continuation
                            && continuations.len() < 64
                        {
                            continuations.push(c);
                        }
                        Ok(())
                    }),
                5 if !continuations.is_empty() => session
                    .resume(
                        continuations[(n as usize) % continuations.len()].clone(),
                        n & 4 == 0,
                    )
                    .and_then(|r| match r.failure {
                        Some(failure) => Err(failure.error),
                        None => Ok(()),
                    }),
                6 => session.write_binding("{}".into(), "0".into()).map(|_| ()),
                7 => session.set_reference_values(
                    if n & 4 == 0 {
                        "{bad"
                    } else {
                        "{\"q\":{\"n\":1}}"
                    }
                    .into(),
                ),
                8 => session
                    .set_field(
                        Some("form".into()),
                        Some("Box".into()),
                        "answer".into(),
                        Some(step.to_string()),
                        true,
                    )
                    .map(|_| ()),
                9 => session.set_program(parse(PROGRAM.into(), library.clone(), None).program),
                10 => session
                    .initialize(Some(if n & 4 == 0 { "[]" } else { "{}" }.into()))
                    .map(|_| ()),
                _ => session
                    .run_legacy(
                        r#"{"steps":[{"type":"custom_action","name":"Go","params":{}}]}"#.into(),
                        "".into(),
                        None,
                    )
                    .map(|r| assert!(r.events.is_empty())),
            };
            assert!(
                session.snapshot().unwrap().len() as u64 <= STATE_CAP,
                "seed {initial}, step {step}: state passed its cap"
            );
            if let Err(error) = result {
                if let OpenUiError::RefusedWrite { reason } = &error {
                    assert!(reason.contains("state cap: 256"));
                    state_refusals += 1;
                }
                assert!(
                    matches!(
                        error,
                        OpenUiError::InvalidJson { .. }
                            | OpenUiError::InvalidMarker
                            | OpenUiError::InvalidHandle
                            | OpenUiError::RefusedWrite { .. }
                            | OpenUiError::ForeignHandle
                            | OpenUiError::StaleHandle
                            | OpenUiError::EvictedHandle
                            | OpenUiError::ConsumedHandle
                            | OpenUiError::LimitExceeded { .. }
                    ),
                    "seed {initial}, step {step}: {error}"
                );
                assert_eq!(
                    session.snapshot().unwrap(),
                    before,
                    "seed {initial}, step {step}"
                );
                assert_eq!(session.version().unwrap(), version);
                let after: Vec<_> = names
                    .iter()
                    .map(|name| session.is_pristine((*name).into()).unwrap())
                    .collect();
                assert_eq!(after, pristine);
            }
        }
        for session in sessions {
            session.dispose();
            assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
        }
    }
    assert!(
        state_refusals > 0,
        "the seeded sequences never exercised Store admission"
    );
    println!(
        "8000 exported calls under a 256-byte state cap: {state_refusals} atomic state refusals"
    );
}
