//! Public callback boundary: bytes, lazy defaults, refusal and containment.
use openui_ffi::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}},"returns":{}}}}"#;
struct Function<F>(F);
impl<F: Fn(String) -> Result<Option<String>, FunctionError> + Send + Sync> LibraryFunction
    for Function<F>
{
    fn call(&self, json: String) -> Result<Option<String>, FunctionError> {
        (self.0)(json)
    }
}
fn setup(program: &str) -> (Arc<ComponentLibrary>, Arc<RuntimeSession>) {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    s.set_program(parse(program.into(), library.clone(), None).program)
        .unwrap();
    s.initialize(None).unwrap();
    (library, s)
}
#[test]
fn function_json_and_lazy_query_default_call_counts_match_runtime() {
    let source = "q=Query(\"q\",{},@Echo(7))\nroot=Box([q,q])";
    let (library, s) = setup(source);
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    s.register_function(
        "Echo".into(),
        Box::new(Function(move |json: String| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(json, r#"{"value":7}"#);
            Ok(Some(json))
        })),
    )
    .unwrap();
    let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
    let lib = openui_lang::Library::from_json_schema(&schema);
    let program = openui_lang::parse(source, &lib, None);
    let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
    functions.register("Echo", |args| Ok(Some(args))).unwrap();
    let expected = openui_runtime::evaluate_tree(
        openui_runtime::TreeInput::new(&program)
            .with_library(&lib)
            .with_functions(&functions),
        &openui_runtime::Store::new(),
    );
    assert_eq!(s.evaluate().unwrap().json, expected.to_json().unwrap());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    s.evaluate().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    s.set_reference_values(r#"{"q":9}"#.into()).unwrap();
    s.evaluate().unwrap();
    // Drawing registers defaults once even with a supplied result, as Rust does.
    assert_eq!(count.load(Ordering::SeqCst), 3);
    drop(library);
}
#[test]
fn thrown_function_messages_match_runtime_exactly_and_panics_do_not_escape() {
    let (_, s) = setup("root=Box(@Echo(1))");
    s.register_function(
        "Echo".into(),
        Box::new(Function(|_| {
            Err(FunctionError::Thrown {
                reason: "bad value".into(),
            })
        })),
    )
    .unwrap();
    let tree = s.evaluate().unwrap();
    let json: serde_json::Value = serde_json::from_str(&tree.json).unwrap();
    assert_eq!(
        json["errors"][0]["message"],
        "Evaluating prop \"value\" on Box failed: @Echo threw: bad value"
    );
    let before = s.snapshot().unwrap();
    s.register_function("Echo".into(), Box::new(Function(|_| panic!("host panic"))))
        .unwrap();
    let tree = s.evaluate().unwrap();
    assert!(tree.json.contains("@Echo threw: host function panicked"));
    assert_eq!(s.snapshot().unwrap(), before);
}
#[test]
fn callback_reentry_from_its_thread_is_refused_without_effects() {
    let (_, s) = setup("$n=0\nroot=Box(@Echo(1))");
    let weak = Arc::downgrade(&s);
    s.register_function(
        "Echo".into(),
        Box::new(Function(move |_| {
            let session = weak.upgrade().unwrap();
            assert!(matches!(session.version(), Err(OpenUiError::Worker { .. })));
            assert!(matches!(
                session.set("$n".into(), Some("8".into())),
                Err(OpenUiError::Worker { .. })
            ));
            assert!(matches!(
                session.evaluate(),
                Err(OpenUiError::Worker { .. })
            ));
            assert!(matches!(
                session.set_reference_values("{}".into()),
                Err(OpenUiError::Worker { .. })
            ));
            session.dispose(); // infallible clean refusal
            Ok(Some("1".into()))
        })),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    let tree = s.evaluate().unwrap();
    let json: serde_json::Value = serde_json::from_str(&tree.json).unwrap();
    assert_eq!(json["errors"], serde_json::json!([]));
    assert_eq!(s.snapshot().unwrap(), before);
}
#[test]
fn registration_and_json_results_are_bounded_and_replacement_is_atomic() {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let s = RuntimeSession::with_limits(
        library,
        None,
        RuntimeLimits {
            max_json_bytes: 128,
            ..Default::default()
        },
    )
    .unwrap();
    s.register_function(
        "Echo".into(),
        Box::new(Function(|_| Ok(Some("null".into())))),
    )
    .unwrap();
    assert!(matches!(
        s.register_function("missing".into(), Box::new(Function(|_| Ok(None)))),
        Err(OpenUiError::InvalidJson { .. })
    ));
    // A replacement needs no additional registration room.
    s.register_function(
        "Echo".into(),
        Box::new(Function(|_| Ok(Some("null".into())))),
    )
    .unwrap();
    let (_, s) = setup("root=Box(@Echo(1))");
    s.register_function("Echo".into(), Box::new(Function(|_| Ok(Some("{".into())))))
        .unwrap();
    assert!(s.evaluate().unwrap().json.contains("@Echo threw:"));
}
#[test]
fn action_runs_evaluate_query_defaults_lazily_once_per_run() {
    let (_, s) =
        setup("$n=0\nq=Query(\"q\",{},@Echo(7))\nroot=Box(1,Action([@Set($n,q),@Set($n,q)]))");
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    s.register_function(
        "Echo".into(),
        Box::new(Function(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some("7".into()))
        })),
    )
    .unwrap();
    let out = s.evaluate().unwrap();
    let action = s
        .action_at(out.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    count.store(0, Ordering::SeqCst);
    s.run(Some(action.clone()), String::new(), None).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    s.run(Some(action), String::new(), None).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(s.get("$n".into()).unwrap().as_deref(), Some("7"));
}
#[test]
fn every_native_session_method_refuses_callback_reentry_before_queueing() {
    let (library, s) =
        setup("$n=0\nm=Mutation(\"m\",{})\nroot=Box(@Echo(7),Action([@Run(m),@Set($n,3)]))");
    let program = parse("root=Box(9)".into(), library.clone(), None).program;
    let tree = s.evaluate().unwrap();
    let evaluation = tree.evaluation;
    let action = s
        .action_at(evaluation.clone(), vec!["props".into(), "action".into()])
        .unwrap();
    let pending = s
        .run(Some(action.clone()), String::new(), None)
        .unwrap()
        .continuation
        .unwrap();
    let weak = Arc::downgrade(&s);
    s.register_function(
        "Echo".into(),
        Box::new(Function(move |_| {
            let session = weak.upgrade().unwrap();
            macro_rules! busy {
                ($call:expr) => {
                    let error = $call.unwrap_err();
                    assert!(matches!(error, OpenUiError::Worker { .. }));
                    assert_eq!(error.to_string(), "session already in use");
                };
            }
            busy!(session.action_at(evaluation.clone(), vec![]));
            busy!(session.evaluate());
            busy!(session.get("$n".into()));
            busy!(session.get_field(None, "n".into()));
            busy!(session.initialize(None));
            busy!(session.initialize_field(None, None, "n".into(), None, false));
            busy!(session.is_pristine("$n".into()));
            busy!(session.mark_binding("Box".into(), "value".into()));
            busy!(session.pending_continuations());
            busy!(session.register_function("Echo".into(), Box::new(Function(|_| Ok(None)))));
            busy!(session.resume(pending.clone(), true));
            busy!(session.run(Some(action.clone()), String::new(), None));
            busy!(session.run_legacy("{}".into(), String::new(), None));
            busy!(session.set("$n".into(), Some("9".into())));
            busy!(session.set_field(None, None, "n".into(), None, false));
            busy!(session.set_program(program.clone()));
            busy!(session.set_reference_values("{}".into()));
            busy!(session.snapshot());
            busy!(session.undecided_bindings());
            busy!(session.version());
            busy!(session.write_binding("{}".into(), "1".into()));
            session.dispose();
            Ok(Some("7".into()))
        })),
    )
    .unwrap();
    let before = s.snapshot().unwrap();
    let version = s.version().unwrap();
    let tree = s.evaluate().unwrap();
    let json: serde_json::Value = serde_json::from_str(&tree.json).unwrap();
    assert_eq!(json["errors"], serde_json::json!([]));
    assert_eq!(s.snapshot().unwrap(), before);
    assert_eq!(s.version().unwrap(), version);
    assert_eq!(s.pending_continuations().unwrap(), 1);
}
#[test]
fn registration_count_and_byte_refusals_preserve_existing_implementations() {
    let definitions: serde_json::Map<String, serde_json::Value> = (0..257)
        .map(|n| (format!("F{n}"), serde_json::json!({"params":{}})))
        .collect();
    let schema = serde_json::json!({"$defs":{}, "functions":definitions}).to_string();
    let library = ComponentLibrary::new(schema.clone()).unwrap();
    let session = RuntimeSession::new(library, None).unwrap();
    for n in 0..256 {
        session
            .register_function(format!("F{n}"), Box::new(Function(|_| Ok(None))))
            .unwrap();
    }
    assert!(matches!(
        session.register_function("F256".into(), Box::new(Function(|_| Ok(None)))),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    session
        .register_function("F0".into(), Box::new(Function(|_| Ok(Some("7".into())))))
        .unwrap();
    let library = ComponentLibrary::new(schema).unwrap();
    let session = RuntimeSession::with_limits(
        library,
        None,
        RuntimeLimits {
            max_json_bytes: 67,
            ..Default::default()
        },
    )
    .unwrap();
    session
        .register_function("F0".into(), Box::new(Function(|_| Ok(None))))
        .unwrap();
    assert!(matches!(
        session.register_function("F1".into(), Box::new(Function(|_| Ok(None)))),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    session
        .register_function("F0".into(), Box::new(Function(|_| Ok(None))))
        .unwrap();
}
#[test]
fn callback_argument_and_result_json_obey_the_session_byte_bound() {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::with_limits(
        library.clone(),
        None,
        RuntimeLimits {
            max_json_bytes: 2048,
            ..Default::default()
        },
    )
    .unwrap();
    session
        .set_program(parse("$s=\"\"\nroot=Box(@Echo($s+$s))".into(), library, None).program)
        .unwrap();
    session.initialize(None).unwrap();
    session
        .set("$s".into(), Some(format!("\"{}\"", "x".repeat(1100))))
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    session
        .register_function(
            "Echo".into(),
            Box::new(Function(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Some("1".into()))
            })),
        )
        .unwrap();
    assert!(
        session
            .evaluate()
            .unwrap()
            .json
            .contains("host function argument byte limit")
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    session.set("$s".into(), Some("\"x\"".into())).unwrap();
    session
        .register_function(
            "Echo".into(),
            Box::new(Function(|_| Ok(Some(format!("\"{}\"", "x".repeat(2048)))))),
        )
        .unwrap();
    assert!(
        session
            .evaluate()
            .unwrap()
            .json
            .contains("host function result byte limit")
    );
}
#[test]
fn query_defaults_remain_cached_across_mutation_resume_and_function_replacement() {
    let (_, s) = setup(
        "$n=0\nq=Query(\"q\",{},@Echo(7))\nm=Mutation(\"m\",{})\nroot=Box(1,Action([@Set($n,q),@Run(m),@Set($n,q)]))",
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    s.register_function(
        "Echo".into(),
        Box::new(Function(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Some("7".into()))
        })),
    )
    .unwrap();
    let out = s.evaluate().unwrap();
    let action = s
        .action_at(out.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    calls.store(0, Ordering::SeqCst);
    let continuation = s
        .run(Some(action), String::new(), None)
        .unwrap()
        .continuation
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let count = calls.clone();
    s.register_function(
        "Echo".into(),
        Box::new(Function(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Some("9".into()))
        })),
    )
    .unwrap();
    s.resume(continuation, true).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(s.get("$n".into()).unwrap().as_deref(), Some("7"));
}
struct ReenterOnDrop {
    session: std::sync::Weak<RuntimeSession>,
    dropped: Arc<AtomicUsize>,
}
impl LibraryFunction for ReenterOnDrop {
    fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
        Ok(Some("1".into()))
    }
}
impl Drop for ReenterOnDrop {
    fn drop(&mut self) {
        let session = self.session.upgrade().unwrap();
        assert!(matches!(session.version(), Err(OpenUiError::Worker { .. })));
        session.dispose();
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn foreign_callback_destruction_refuses_reentry_on_replacement_and_disposal() {
    let (_, s) = setup("root=Box(@Echo(1))");
    let dropped = Arc::new(AtomicUsize::new(0));
    let make = || {
        Box::new(ReenterOnDrop {
            session: Arc::downgrade(&s),
            dropped: dropped.clone(),
        }) as Box<dyn LibraryFunction>
    };
    s.register_function("Echo".into(), make()).unwrap();
    s.register_function("Echo".into(), make()).unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(matches!(
        s.register_function("missing".into(), make()),
        Err(OpenUiError::InvalidJson { .. })
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 2);
    s.dispose();
    assert_eq!(dropped.load(Ordering::SeqCst), 3);
    assert!(matches!(s.version(), Err(OpenUiError::Disposed)));
}
#[test]
fn json_function_results_cannot_restore_custom_authority_even_from_real_arguments() {
    let schema = r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}},"actions":{"Go":{"params":{}}}}"#;
    let library = ComponentLibrary::new(schema.into()).unwrap();
    let source = "root=Box(1,@Echo(@Go()))";
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    s.set_program(parse(source.into(), library, None).program)
        .unwrap();
    s.register_function(
        "Echo".into(),
        Box::new(Function(|json: String| {
            let json: serde_json::Value = serde_json::from_str(&json).unwrap();
            Ok(Some(json["value"].to_string()))
        })),
    )
    .unwrap();
    let out = s.evaluate().unwrap();
    let tree: serde_json::Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(
        tree["root"]["props"]["action"]["steps"],
        serde_json::json!([])
    );
    let action = s
        .action_at(out.evaluation, vec!["props".into(), "action".into()])
        .unwrap();
    assert!(
        s.run(Some(action), String::new(), None)
            .unwrap()
            .events
            .is_empty()
    );
    // The direct Rust identity implementation can return the original marked
    // allocation. JSON cannot carry that authority; accepting the bytes would
    // let a host reconstruct a finished custom step, contrary to ADR 0004.
    let schema: serde_json::Value = serde_json::from_str(schema).unwrap();
    let library = openui_lang::Library::from_json_schema(&schema);
    let program = openui_lang::parse(source, &library, None);
    let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Echo", |args| {
            Ok(match &args {
                openui_lang::Value::Obj(o) => o.get("value").cloned(),
                _ => None,
            })
        })
        .unwrap();
    let tree = openui_runtime::evaluate_tree(
        openui_runtime::TreeInput::new(&program)
            .with_library(&library)
            .with_actions(&schema["actions"])
            .with_functions(&functions),
        &openui_runtime::Store::new(),
    );
    let tree: serde_json::Value = serde_json::from_str(&tree.to_json().unwrap()).unwrap();
    assert_eq!(
        tree["root"]["props"]["action"]["steps"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn failed_function_results_become_null_and_run_continues_exactly_as_rust() {
    let source = "$a=0\n$b=0\n$c=0\nroot=Box(1,Action([@Set($a,1),@Set($b,@Echo(1)),@Set($c,2)]))";
    for fault in 0..4 {
        let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
        let session = RuntimeSession::with_limits(
            library.clone(),
            None,
            RuntimeLimits {
                max_json_bytes: 2048,
                ..Default::default()
            },
        )
        .unwrap();
        session
            .set_program(parse(source.into(), library, None).program)
            .unwrap();
        session.initialize(None).unwrap();
        session
            .register_function(
                "Echo".into(),
                Box::new(Function(move |_| match fault {
                    0 => Err(FunctionError::Thrown {
                        reason: "bad value".into(),
                    }),
                    1 => Ok(Some("{".into())),
                    2 => panic!("host panic"),
                    _ => Ok(Some(format!("\"{}\"", "x".repeat(2048)))),
                })),
            )
            .unwrap();
        let out = session.evaluate().unwrap();
        let action = session
            .action_at(out.evaluation, vec!["props".into(), "action".into()])
            .unwrap();
        let actual = session.run(Some(action), String::new(), None).unwrap();
        let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
        let library = openui_lang::Library::from_json_schema(&schema);
        let program = openui_lang::parse(source, &library, None);
        let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
        functions
            .register("Echo", |_| Err("a runtime function failure".into()))
            .unwrap();
        let input = openui_runtime::TreeInput::new(&program)
            .with_library(&library)
            .with_functions(&functions);
        let mut store = openui_runtime::Store::new();
        assert!(
            store
                .initialize(
                    program
                        .state_declarations
                        .iter()
                        .map(|(k, v)| (k.as_str(), Some(v))),
                    [],
                )
                .refused
                .is_empty()
        );
        let tree = openui_runtime::evaluate_tree(input.clone(), &store);
        let Some(openui_lang::Value::Element(ref root)) = tree.root else {
            panic!()
        };
        let expected =
            openui_runtime::run_action(input, &mut store, root.props.get("action"), "", None);
        assert!(actual.failure.is_none() && expected.failure.is_none());
        assert_eq!(
            actual.events,
            expected
                .events
                .iter()
                .map(|e| e.to_json().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(actual.size_limited, expected.size_limited);
        assert_eq!(actual.depth_limited, expected.depth_limited);
        assert!(actual.continuation.is_none() && !expected.is_suspended());
        assert_eq!(
            session.snapshot().unwrap(),
            store.snapshot_json().unwrap(),
            "fault {fault}"
        );
        assert_eq!(session.snapshot().unwrap(), r#"{"$a":1,"$b":null,"$c":2}"#);
    }
}
