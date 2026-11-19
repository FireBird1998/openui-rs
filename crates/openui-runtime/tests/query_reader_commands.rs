//! Transaction boundaries and live inputs observed by registered query readers.
use openui_lang::{JsObject, JsonValue, Library, Limits, parse};
use openui_runtime::{QueryCommand, QueryCommandError, QueryManager, Store, ToolOutcome, Value};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn a_reader_cut_consumes_the_raw_receipt_and_reports_the_suspended_reader() {
    let source = r#"a=Query("source")
b=Query("reader",{x:a+a})"#;
    let program = parse(source, &Library::from_json_schema(&JsonValue::Null), None);
    let store = Store::default();
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1024));
    let mut foreign = QueryManager::default();
    let call = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        })
        .unwrap()
        .calls
        .pop()
        .unwrap();
    let before = (
        manager.snapshot_json().to_owned(),
        manager.cache_entry_count(),
    );
    let long = JsonValue::String("x".repeat(700));
    let complete = |value| QueryCommand::Complete {
        id: &call.id,
        outcome: ToolOutcome::Result(value),
    };
    assert!(matches!(
        foreign.apply(complete(None)),
        Err(QueryCommandError::InvalidCall)
    ));
    assert_eq!(
        (manager.snapshot_json(), manager.cache_entry_count()),
        (before.0.as_str(), before.1),
        "foreign delivery changed the source"
    );
    let update = manager.apply(complete(Some(&long))).unwrap();
    assert!(update.calls.is_empty());
    assert_eq!(manager.snapshot().get("a"), Some(&Value::from_json(&long)));
    assert!(matches!(
        manager.reader_diagnostic(),
        Some(openui_runtime::ReaderDiagnostic::PropagationLimit(_))
    ));
    assert!(manager.current_entry_is_served("b", true, std::iter::empty()));
    assert!(!call.id.is_pending());
    assert!(manager.cache_entry_count() <= 4);
    assert!(matches!(
        manager.apply(complete(Some(&long))),
        Err(QueryCommandError::InvalidCall)
    ));
}

#[test]
fn readers_observe_only_committed_store_initialization_even_after_the_store_is_dropped() {
    let program = parse(
        "a=Query(\"a\")\nb=Query(\"b\",{x:a,n:$n})",
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let mut store = Store::default();
    let mut manager = QueryManager::default();
    let call = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        })
        .unwrap()
        .calls
        .pop()
        .unwrap();
    let mut persisted = JsObject::new();
    persisted.define("$n", Value::Num(12.0));
    let empty = JsObject::new();
    let (_, initialization, ()) = store
        .initialize_transaction(&empty, &persisted, None, |_| Ok::<_, ()>(()))
        .unwrap();
    assert!(initialization.changed && initialization.refused.is_empty());
    persisted.define("$n", Value::Num(99.0));
    assert!(
        store
            .initialize_transaction(&empty, &persisted, None, |_| Err::<(), _>(()))
            .is_err()
    );
    drop(store);
    let update = manager
        .apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(None),
        })
        .unwrap();
    assert_eq!(
        update.calls.first().unwrap().arguments_json().as_deref(),
        Some(r#"{"x":null,"n":12}"#)
    );
}

#[test]
fn a_long_waiting_chain_inherits_loading_without_recursing() {
    // Construct the same public metadata produced by the parser, so this
    // exercises scheduling depth independently of parser admission.
    let mut program = parse(
        "source=Query(\"source\")",
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let mut previous = String::from("source");
    let template = program.query_statements.first().unwrap().clone();
    for i in 0..4096 {
        let mut q = template.clone();
        q.statement_id = format!("reader{i}");
        q.tool = Some(openui_lang::Ast::Str(q.statement_id.clone()));
        q.wait_for = Some(vec![previous]);
        previous = q.statement_id.clone();
        program.query_statements.push(q);
    }
    let mut manager = QueryManager::with_limits(
        Limits::DEFAULT
            .with_work(16_000_000)
            .with_result_size(2_000_000),
    );
    let store = Store::default();
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        })
        .unwrap();
    assert_eq!(update.calls.len(), 1);
    let loading = |m: &QueryManager| match m.snapshot().get("__openui_loading") {
        Some(Value::Arr(items)) => items.len(),
        _ => panic!("loading list missing"),
    };
    assert_eq!(loading(&manager), 4097);
    assert_eq!(manager.cache_entry_count(), 4097);
    let call = update.calls.first().unwrap();
    manager
        .apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::CallFailed("offline"),
        })
        .unwrap();
    assert_eq!(loading(&manager), 0);
}

#[test]
fn a_reader_with_a_borrowed_function_can_retry_completion_in_its_library_context() {
    use openui_runtime::{LibraryFunctions, ToolCallOutcome, TreeInput};
    let schema = openui_lang::parse_json(
        r#"{"$defs":{},"functions":{"Read":{"params":{"properties":{"value":{}}}}}}"#,
    )
    .unwrap();
    let library = Library::from_json_schema(&schema);
    let mut functions = LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Read", |value| match &value {
            Value::Obj(obj) => Ok(obj.get("value").cloned()),
            _ => Err("expected named arguments".into()),
        })
        .unwrap();
    let program = parse(
        "a=Query(\"a\")\nb=Query(\"b\",{v:@Read(a)})",
        &library,
        None,
    );
    let input = TreeInput::new(&program)
        .with_library(&library)
        .with_functions(&functions);
    let store = Store::default();
    let mut manager = QueryManager::default();
    let (_, mut update) = manager
        .evaluate_transaction(&input, &store, |_| Ok::<_, QueryCommandError>(()))
        .unwrap();
    let call = update.calls.pop().unwrap();
    let value = || ToolCallOutcome::Result(Value::Num(42.0));
    let before = manager.snapshot_json().to_owned();
    assert!(matches!(
        manager.complete_tool_call(&call.id, value()),
        Err(QueryCommandError::Evaluation(_))
    ));
    assert_eq!(manager.snapshot_json(), before);
    assert!(call.id.is_pending());
    let update = manager
        .complete_tool_call_with_context(&call.id, value(), &input, &store)
        .unwrap();
    assert_eq!(
        update.calls.first().unwrap().arguments_json().as_deref(),
        Some(r#"{"v":42}"#)
    );
}

#[test]
fn a_refused_registration_releases_its_store_snapshot() {
    use openui_runtime::TreeInput;
    let program = parse(
        "a=Query(\"a\")\nb=Query(\"b\",{x:a})",
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let store = Store::default();
    let snapshot = store.snapshot();
    let before = std::sync::Arc::strong_count(&snapshot);
    let mut manager = QueryManager::default();
    let rejected = manager.evaluate_transaction(&TreeInput::new(&program), &store, |_| {
        Err::<(), _>(QueryCommandError::InvalidCall)
    });
    assert!(rejected.is_err());
    assert_eq!(manager.cache_entry_count(), 0);
    assert_eq!(std::sync::Arc::strong_count(&snapshot), before);
}

#[test]
fn a_raw_nan_completion_settles_once_when_a_thousand_readers_exhaust_the_allowance() {
    let mut source = String::from("s=Query(\"s\")");
    for i in 0..1000 {
        source.push_str(&format!("\nr{i}=Query(\"r\",{{x:s}})"));
    }
    let program = parse(&source, &Library::from_json_schema(&JsonValue::Null), None);
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(1_000_000));
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &Store::default(),
        })
        .unwrap();
    assert_eq!(update.calls.len(), 1);
    let id = &update.calls.first().unwrap().id;
    let value = Value::Num(f64::NAN);
    let complete = || QueryCommand::Complete {
        id,
        outcome: ToolOutcome::ResultValue(&value),
    };
    let cut = manager.apply(complete()).unwrap();
    assert!(cut.calls.is_empty());
    assert!(matches!(manager.snapshot().get("s"), Some(Value::Num(n)) if n.is_nan()));
    assert!(matches!(
        manager.reader_diagnostic(),
        Some(openui_runtime::ReaderDiagnostic::PropagationLimit(_))
    ));
    assert!(!id.is_pending());
    assert!(matches!(
        manager.apply(complete()),
        Err(QueryCommandError::InvalidCall)
    ));
    manager.audit_cache(std::iter::empty(), true).unwrap();
}

#[test]
fn a_later_declaration_still_owns_a_repeated_id_after_a_reader_round() {
    let mut program = parse(
        "s=Query(\"source\")\nr=Query(\"reader\",{x:s})\nlast=Query(\"fixed\")",
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    // Direct callers can supply statement lists the parser would never emit.
    program.query_statements.last_mut().unwrap().statement_id = "r".into();
    let mut manager = QueryManager::default();
    let calls = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &Store::default(),
        })
        .unwrap()
        .calls;
    for tool in ["fixed", "source"] {
        let call = calls.iter().find(|c| c.tool_name == tool).unwrap();
        manager
            .apply(QueryCommand::Complete {
                id: &call.id,
                outcome: ToolOutcome::Result(None),
            })
            .unwrap();
    }
    let run = manager
        .apply(QueryCommand::Invalidate {
            statement_ids: &["r"],
        })
        .unwrap();
    assert_eq!(run.calls.len(), 1);
    assert_eq!(run.calls.first().unwrap().tool_name, "fixed");
}
