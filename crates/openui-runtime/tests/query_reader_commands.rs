//! Transaction boundaries and live inputs observed by registered query readers.
use openui_lang::{JsObject, JsonValue, Library, Limits, parse, parse_json};
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

fn reader_calls(
    manager: &mut QueryManager,
    command: QueryCommand<'_>,
) -> Vec<openui_runtime::ToolCall> {
    let update = manager.apply(command).unwrap();
    manager.audit_output(&update.calls).unwrap();
    update.calls
}

fn named<'a>(
    calls: &'a [openui_runtime::ToolCall],
    tool: &str,
    args: &str,
) -> &'a openui_runtime::ToolCall {
    calls
        .iter()
        .find(|c| c.tool_name == tool && c.arguments_json().as_deref() == Some(args))
        .unwrap_or_else(|| panic!("no call {tool}|{args}"))
}

#[test]
fn a_remembered_run_and_a_reader_moving_onto_its_failed_key_start_one_call() {
    // The `entry.pending.is_none()` guard in dispatch (#113, #119). `a`'s
    // call fails while a run is remembered for its key. The NaN that `r`
    // reads keeps `r` stale, so the delivery rebuilds it with the live
    // `$tool`, and it moves onto `a`'s failed key with no error of its own
    // and requests a call there. The remembered run starts first; the
    // request must not start a second call and orphan the first receipt.
    let library = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        "$tool=\"u\"\nn=Query(\"n\")\na=Query(\"t\",{})\nr=Query([$tool,n][0],{})",
        &library,
        None,
    );
    let mut store = Store::default();
    let _ = store.initialize(
        program
            .state_declarations
            .iter()
            .map(|(name, value)| (name.as_str(), Some(value))),
        [],
    );
    let mut manager = QueryManager::default();
    let evaluate = QueryCommand::Evaluate {
        program: &program,
        store: &store,
    };
    let calls = reader_calls(&mut manager, evaluate);
    let nan = Value::Num(f64::NAN);
    let after_nan = reader_calls(
        &mut manager,
        QueryCommand::Complete {
            id: &named(&calls, "n", "{}").id,
            outcome: ToolOutcome::ResultValue(&nan),
        },
    );
    let u = named(&after_nan, "u", "{}");
    assert!(
        reader_calls(
            &mut manager,
            QueryCommand::Invalidate {
                statement_ids: &["a"]
            }
        )
        .is_empty()
    );
    store.set("$tool", Some(Value::Str("t".into()))).unwrap();
    let delivered = reader_calls(
        &mut manager,
        QueryCommand::Complete {
            id: &named(&calls, "t", "{}").id,
            outcome: ToolOutcome::CallFailed("offline"),
        },
    );
    assert_eq!(delivered.len(), 1, "one call for the shared failed key");
    assert_eq!(delivered[0].tool_name, "t");
    manager
        .audit_cache([&u.id, &delivered[0].id].into_iter(), true)
        .unwrap();
}

#[test]
fn a_reader_cut_keeps_only_calls_with_a_healthy_holder() {
    let library = Library::from_json_schema(&JsonValue::Null);
    let initial = parse(
        r#"healthy=Query("shared",{v:0})
reader=Query("shared",{v:0})
solo=Query("solo",{v:0})"#,
        &library,
        None,
    );
    // Keep both keys in flight while editing two of their holders to read s.
    let readers = parse(
        r#"s=Query("source")
healthy=Query("shared",{v:0})
reader=Query("shared",{v:s ? s+s : 0})
solo=Query("solo",{v:s ? s+s : 0})"#,
        &library,
        None,
    );
    let store = Store::default();
    let evaluate = |program| QueryCommand::Evaluate {
        program,
        store: &store,
    };
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_result_size(2048));
    let mut receipts = Vec::new();
    let mut step = |manager: &mut QueryManager, command: QueryCommand<'_>| {
        let update = manager.apply(command).unwrap();
        manager.audit_output(&update.calls).unwrap();
        receipts.extend(update.calls.iter().map(|call| call.id.clone()));
        manager.audit_cache(receipts.iter(), true).unwrap();
        update
    };

    let first = step(&mut manager, evaluate(&initial));
    assert_eq!(first.calls.len(), 2);
    let shared = &named(&first.calls, "shared", r#"{"v":0}"#).id;
    let solo = &named(&first.calls, "solo", r#"{"v":0}"#).id;
    let edited = step(&mut manager, evaluate(&readers));
    assert_eq!(edited.calls.len(), 1);
    let source = &named(&edited.calls, "source", "{}").id;
    assert!(shared.is_pending() && solo.is_pending());

    // The source fits, but doubling it for a reader exceeds the result limit.
    let value = Value::Str("x".repeat(1300));
    let complete = |id, value| QueryCommand::Complete {
        id,
        outcome: ToolOutcome::ResultValue(value),
    };
    let cut = step(&mut manager, complete(source, &value));
    assert!(cut.calls.is_empty());
    assert_eq!(manager.snapshot().get("s"), Some(&value));
    assert!(matches!(
        manager.reader_diagnostic(),
        Some(openui_runtime::ReaderDiagnostic::PropagationLimit(_))
    ));
    assert!(shared.is_pending(), "healthy still wants the shared call");
    assert!(
        !solo.is_pending(),
        "the suspended solo reader kept its call"
    );

    let result = Value::Num(7.0);
    assert!(
        step(&mut manager, complete(shared, &result))
            .calls
            .is_empty()
    );
    assert_eq!(manager.snapshot().get("healthy"), Some(&result));

    // Registering again issues a new call while the old one is still out.
    let recovered = step(&mut manager, evaluate(&initial));
    assert!(manager.reader_diagnostic().is_none());
    assert_eq!(recovered.calls.len(), 1);
    let replacement = &named(&recovered.calls, "solo", r#"{"v":0}"#).id;
    let before = manager.snapshot_json().to_owned();
    let stale_result = Value::Num(99.0);
    let late = step(&mut manager, complete(solo, &stale_result));
    assert!(late.calls.is_empty() && !late.changed);
    assert_eq!(manager.snapshot_json(), before);
    assert!(
        step(&mut manager, complete(replacement, &result))
            .calls
            .is_empty()
    );
    assert_eq!(manager.snapshot().get("solo"), Some(&result));
}

#[test]
fn a_cascade_refused_by_the_snapshot_limit_starts_none_of_its_requests() {
    // The reset of collected requests on the delivery's limit path (#113,
    // #119). `s`'s result is admitted, and its readers' cascade moves `m`
    // onto `q`'s failed key, where `m` (with no error) requests a call. The
    // rebuilt default then makes the snapshot too large, so the cascade is
    // discarded and `m` suspended. Its request must go with it: `q` still
    // shows its error and no call starts for the failed key.
    let library = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        "s=Query(\"s\")\nq=Query(\"t\",{v:1})\nm=Query(\"t\",{v:s.v},s.pad)",
        &library,
        None,
    );
    let store = Store::default();
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1024));
    let calls = reader_calls(
        &mut manager,
        QueryCommand::Evaluate {
            program: &program,
            store: &store,
        },
    );
    let failed = reader_calls(
        &mut manager,
        QueryCommand::Complete {
            id: &named(&calls, "t", r#"{"v":1}"#).id,
            outcome: ToolOutcome::CallFailed("offline"),
        },
    );
    assert!(failed.is_empty());
    let mut source = JsObject::new();
    source.define("v", Value::Num(1.0));
    source.define("pad", Value::Str("x".repeat(600)));
    let source = Value::Obj(std::sync::Arc::new(source));
    let delivered = reader_calls(
        &mut manager,
        QueryCommand::Complete {
            id: &named(&calls, "s", "{}").id,
            outcome: ToolOutcome::ResultValue(&source),
        },
    );
    assert!(
        matches!(
            manager.reader_diagnostic(),
            Some(openui_runtime::ReaderDiagnostic::PropagationLimit(_))
        ),
        "the cascade was cut"
    );
    assert!(delivered.is_empty(), "no call from the discarded cascade");
    let snapshot: JsonValue = openui_lang::parse_json(manager.snapshot_json()).unwrap();
    assert_eq!(snapshot["__openui_errors"][0]["statementId"], "q");
    assert_eq!(snapshot["__openui_loading"], JsonValue::Array(Vec::new()));
    manager.audit_cache(std::iter::empty(), true).unwrap();
}

#[test]
fn a_recreated_colliding_key_calls_with_the_first_request_as_upstream_does() {
    // #113's question, answered for #119: yes. In one cascade `a` requests
    // the key {"v":"__undefined__"} with arguments `{}` and then leaves it,
    // and `b` arrives on the same key with `{"v":"__undefined__"}`. The one
    // call carries `a`'s arguments, the first request, though only `b` holds
    // the key. Lang-core sends the same call (it fetches at `a`'s request);
    // keys collide as upstream's do (CONFORMANCE.md, pin
    // `readers-a-colliding-key-keeps-the-first-request-across-a-cascade`).
    let library = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        "s=Query(\"s\")\nx=Query(\"x\",{v:s})\ny=Query(\"x\",{v:2})\n\
         a=Query(\"t\",{v:x==\"k1\"?(s==2?[0][2]:\"a0\"):\"a2\"})\n\
         b=Query(\"t\",{v:x==\"k2\"?\"__undefined__\":\"b1\"})",
        &library,
        None,
    );
    let store = Store::default();
    let mut manager = QueryManager::default();
    let first = reader_calls(
        &mut manager,
        QueryCommand::Evaluate {
            program: &program,
            store: &store,
        },
    );
    let text = |s: &str| JsonValue::String(s.into());
    let (k1, k2, a0, b1) = (text("k1"), text("k2"), text("A0"), text("B1"));
    let (one, two) = (JsonValue::from(1), JsonValue::from(2));
    let finish = |manager: &mut QueryManager, call: &openui_runtime::ToolCall, value| {
        reader_calls(
            manager,
            QueryCommand::Complete {
                id: &call.id,
                outcome: ToolOutcome::Result(Some(value)),
            },
        )
    };
    finish(&mut manager, named(&first, "x", r#"{"v":2}"#), &k2);
    let x1 = finish(&mut manager, named(&first, "s", "{}"), &one);
    let readers = finish(&mut manager, named(&x1, "x", r#"{"v":1}"#), &k1);
    finish(&mut manager, named(&readers, "t", r#"{"v":"a0"}"#), &a0);
    finish(&mut manager, named(&readers, "t", r#"{"v":"b1"}"#), &b1);
    let rerun = reader_calls(
        &mut manager,
        QueryCommand::Invalidate {
            statement_ids: &["s"],
        },
    );
    let cascade = finish(&mut manager, named(&rerun, "s", "{}"), &two);
    assert_eq!(cascade.len(), 2);
    let shared = named(&cascade, "t", "{}");
    named(&cascade, "t", r#"{"v":"a2"}"#);
    let snapshot: JsonValue = openui_lang::parse_json(manager.snapshot_json()).unwrap();
    assert_eq!(
        snapshot["__openui_loading"],
        openui_lang::parse_json(r#"["a","b"]"#).unwrap(),
        "b, the key's only holder, is loading on the call made with a's request"
    );
    let result = text("K");
    finish(&mut manager, shared, &result);
    let snapshot: JsonValue = openui_lang::parse_json(manager.snapshot_json()).unwrap();
    assert_eq!(snapshot["b"], "K");
}

#[test]
fn an_error_naming_a_nan_tool_is_unchanged_by_another_querys_delivery() {
    // The query fuzz target's crash 5ca76c7b (#119): `b`'s tool name
    // evaluates to an array holding NaN, and its error keeps that raw name.
    // The delivery audit compared errors by value, and NaN never equals
    // itself, so `c`'s failure looked like a change to `b`. The audit now
    // compares by identity; a delivery that replaces an error makes a new one.
    let source = "a = Query(1 + 2, falsu)\nb = Query([1, null, 6666666666666666666666666666666666eu2t r,[, 3]], 0)\nc = Query({}, \"args\")\nd = Query(null, [])\ne = Query([0][2], W0][2])\nf = Query()";
    for limits in [
        Limits::DEFAULT,
        Limits::DEFAULT
            .with_result_floor(1024)
            .with_amplification(4)
            .with_work(100_000)
            .with_depth(16),
    ] {
        let library = Library::from_json_schema(&JsonValue::Null);
        let program = openui_lang::parse_with_limits(source, &library, None, limits);
        let store = Store::with_limits(limits);
        let mut manager = QueryManager::with_limits(limits);
        let calls = reader_calls(
            &mut manager,
            QueryCommand::Evaluate {
                program: &program,
                store: &store,
            },
        );
        let missing = ToolOutcome::ToolNotFound(Vec::new());
        manager.audit_delivery(&calls[1].id, &missing).unwrap();
        reader_calls(
            &mut manager,
            QueryCommand::Complete {
                id: &calls[1].id,
                outcome: missing,
            },
        );
        assert!(
            manager
                .snapshot_json()
                .contains(r#""toolName":[1,null,null"#)
        );
        let failed = ToolOutcome::McpError("");
        manager.audit_delivery(&calls[2].id, &failed).unwrap();
        reader_calls(
            &mut manager,
            QueryCommand::Complete {
                id: &calls[2].id,
                outcome: failed,
            },
        );
        manager
            .audit_cache(calls.iter().map(|c| &c.id), true)
            .unwrap();
    }
}

#[test]
fn a_reader_of_a_statement_that_is_not_a_query_waits_as_upstream_does() {
    // The query fuzz target's crash 2e7f2d70, found during #119 (#125 item
    // 1) and decided in #188: the parser (as lang-core's does) lists `f` in
    // `a`'s waitFor although `f` is not a query statement and the parse is
    // not cut. `a` waits for a source that cannot arrive and shows its
    // default with no call, byte for byte as lang-core's manager does. The
    // cache audit accepts that wait (ADR 0013).
    let source = "a = Query(2 + 2, f-----------------: 2], [0][2])\nf =@toString( Query$)";
    let library = Library::from_json_schema(&JsonValue::Null);
    let program = parse(source, &library, None);
    assert!(!program.size_limited && !program.depth_limited);
    assert_eq!(program.query_statements.len(), 1);
    assert_eq!(
        program.query_statements[0].wait_for.as_deref(),
        Some(&["f".to_owned()][..])
    );
    let store = Store::default();
    let mut manager = QueryManager::default();
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        })
        .unwrap();
    assert!(update.calls.is_empty());
    assert_eq!(
        manager.snapshot_json(),
        r#"{"__openui_loading":[],"__openui_refetching":[],"__openui_errors":[],"a":2}"#,
        "lang-core 1.1.0's snapshot for the same program"
    );
    manager.audit_cache(std::iter::empty(), true).unwrap();
    // A run of the waiting reader starts no call, as for any waiting reader.
    let update = manager
        .apply(QueryCommand::Invalidate {
            statement_ids: &["a"],
        })
        .unwrap();
    assert!(update.calls.is_empty());
    manager.audit_cache(std::iter::empty(), true).unwrap();
}

#[test]
fn a_query_wrapped_in_a_call_the_library_does_not_define_leaves_its_reader_waiting() {
    // #188's natural trigger: `toString` is no library function, but its
    // name is an `Object.prototype` member, so the parser counts the call as
    // a reserved one (quirk 2) and lists `f` in `a`'s waitFor, as lang-core
    // does (`hand/251-a-reader-of-a-wrapped-query`). `a` keeps its default
    // and never calls; `b` is unaffected. The reader golden
    // `a-reader-of-a-statement-that-is-not-a-query-keeps-its-default` pins
    // the same steps against lang-core's manager.
    let source =
        "f = @toString(Query(\"u\"))\na = Query(\"v\", {x: f}, \"none\")\nb = Query(\"w\")";
    let program = parse(source, &Library::from_json_schema(&JsonValue::Null), None);
    assert!(!program.size_limited && !program.depth_limited);
    let ids: Vec<_> = program
        .query_statements
        .iter()
        .map(|q| (q.statement_id.as_str(), q.wait_for.clone()))
        .collect();
    assert_eq!(ids, [("a", Some(vec!["f".to_owned()])), ("b", None)]);
    let mut manager = QueryManager::default();
    let mut receipts = Vec::new();
    let calls = audited_reader_calls(
        &mut manager,
        QueryCommand::Evaluate {
            program: &program,
            store: &Store::default(),
        },
        &mut receipts,
    );
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_name, "w");
    let value = JsonValue::from(1);
    let done = audited_reader_calls(
        &mut manager,
        QueryCommand::Complete {
            id: &calls[0].id,
            outcome: ToolOutcome::Result(Some(&value)),
        },
        &mut receipts,
    );
    assert!(done.is_empty());
    for ids in [&["a"][..], &[]] {
        let runs = audited_reader_calls(
            &mut manager,
            QueryCommand::Invalidate { statement_ids: ids },
            &mut receipts,
        );
        assert!(runs.iter().all(|call| call.tool_name == "w"));
    }
    assert_eq!(
        parse_json(manager.snapshot_json()).unwrap()["a"],
        JsonValue::from("none")
    );
}

fn audited_reader_calls(
    manager: &mut QueryManager,
    command: QueryCommand<'_>,
    receipts: &mut Vec<openui_runtime::ToolCallId>,
) -> Vec<openui_runtime::ToolCall> {
    let calls = reader_calls(manager, command);
    receipts.extend(calls.iter().map(|call| call.id.clone()));
    manager.audit_cache(receipts.iter(), true).unwrap();
    calls
}

fn register_readers(
    manager: &mut QueryManager,
    source: &str,
    receipts: &mut Vec<openui_runtime::ToolCallId>,
) -> Vec<openui_runtime::ToolCall> {
    let program = parse(source, &Library::from_json_schema(&JsonValue::Null), None);
    audited_reader_calls(
        manager,
        QueryCommand::Evaluate {
            program: &program,
            store: &Store::default(),
        },
        receipts,
    )
}

fn deliver_source<'a>(
    calls: &'a [openui_runtime::ToolCall],
    result: &'a JsonValue,
) -> QueryCommand<'a> {
    QueryCommand::Complete {
        id: &named(calls, "s", "{}").id,
        outcome: ToolOutcome::Result(Some(result)),
    }
}

fn check_reader_delivery(
    manager: &QueryManager,
    calls: &[openui_runtime::ToolCall],
    source_result: &JsonValue,
    expected_args: &str,
    expect_cut: bool,
) {
    assert_eq!(
        manager.snapshot().get("s"),
        Some(&Value::from_json(source_result))
    );
    let diagnostic = manager.reader_diagnostic();
    let snapshot = openui_lang::parse_json(manager.snapshot_json()).unwrap();
    let errors = snapshot["__openui_errors"].as_array().unwrap();
    if expect_cut {
        assert!(
            matches!(
                diagnostic,
                Some(openui_runtime::ReaderDiagnostic::PropagationLimit(_))
            ),
            "expected the long case to suspend r at the work limit, diagnostic {diagnostic:?}, new calls {}",
            calls.len()
        );
        assert!(calls.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["code"], "reader-limit");
        assert_eq!(errors[0]["statementId"], "r");
    } else {
        assert_eq!(diagnostic, None);
        assert!(errors.is_empty());
        assert_eq!(calls.len(), 1);
        named(calls, "r", expected_args);
    }
}

#[test]
fn a_cascade_charges_probes_of_unchanged_readers_by_statement_id_length() {
    for (prefix, expect_cut) in [("x".to_owned(), false), ("x".repeat(512), true)] {
        // Only r reads s. The other eight readers still wait for u when s settles.
        let mut source = String::from("s=Query(\"s\")\nr=Query(\"r\",{v:s})\nu=Query(\"u\")");
        for i in 0..8 {
            source.push_str(&format!("\n{prefix}{i}=Query(\"w\",{{v:u}})"));
        }
        let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(6_000));
        let mut receipts = Vec::new();
        let initial = register_readers(&mut manager, &source, &mut receipts);
        assert_eq!(initial.len(), 2);
        assert!(named(&initial, "u", "{}").id.is_pending());
        let result = JsonValue::from(1);
        let delivered = audited_reader_calls(
            &mut manager,
            deliver_source(&initial, &result),
            &mut receipts,
        );
        check_reader_delivery(&manager, &delivered, &result, r#"{"v":1}"#, expect_cut);
    }
}

#[test]
fn a_cascade_charges_the_previous_string_when_the_source_version_changes() {
    let source = "s=Query(\"s\")\nr=Query(\"r\",{v:s.v})";
    // Both strings have no v property, so r makes the same initial call. The
    // refetch supplies the same object in both cases. Only the old string varies.
    let new_field = "y".repeat(256);
    let args = format!(r#"{{"v":"{new_field}"}}"#);
    let new_result = openui_lang::parse_json(&args).unwrap();
    for (old_text, expect_cut) in [("x".to_owned(), false), ("x".repeat(512), true)] {
        let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(1_900));
        let mut receipts = Vec::new();
        let initial = register_readers(&mut manager, source, &mut receipts);
        assert_eq!(initial.len(), 1);
        let old_result = JsonValue::from(old_text);
        let first = audited_reader_calls(
            &mut manager,
            deliver_source(&initial, &old_result),
            &mut receipts,
        );
        assert_eq!(
            manager.snapshot().get("s"),
            Some(&Value::from_json(&old_result))
        );
        assert_eq!(manager.reader_diagnostic(), None);
        assert_eq!(first.len(), 1);
        named(&first, "r", "{}");
        let refetch = audited_reader_calls(
            &mut manager,
            QueryCommand::Invalidate {
                statement_ids: &["s"],
            },
            &mut receipts,
        );
        assert_eq!(refetch.len(), 1);
        let delivered = audited_reader_calls(
            &mut manager,
            deliver_source(&refetch, &new_result),
            &mut receipts,
        );
        check_reader_delivery(&manager, &delivered, &new_result, &args, expect_cut);
    }
}

#[test]
fn a_cascade_charges_the_string_default_when_reapplying_a_reader() {
    for (default, expect_cut) in [("x".to_owned(), false), ("x".repeat(512), true)] {
        // The string becomes the reader's default only after s delivers.
        let source = format!("s=Query(\"s\")\nr=Query(\"r\",{{v:s}},s ? \"{default}\" : null)");
        let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(1_080));
        let mut receipts = Vec::new();
        let initial = register_readers(&mut manager, &source, &mut receipts);
        assert_eq!(initial.len(), 1);
        let result = JsonValue::from(1);
        let delivered = audited_reader_calls(
            &mut manager,
            deliver_source(&initial, &result),
            &mut receipts,
        );
        check_reader_delivery(&manager, &delivered, &result, r#"{"v":1}"#, expect_cut);
    }
}

#[test]
fn query_registration_accepts_statement_id_bytes_at_the_work_limit() {
    // Query registration indexes mutation ids too, but does not evaluate their tool.
    let mutation_id = "m".repeat(63);
    let lib = Library::from_json_schema(&JsonValue::Null);
    let program = parse(&format!("{mutation_id}=Mutation(\"save\")"), &lib, None);
    assert_eq!(program.mutation_statements[0].statement_id, mutation_id);
    let store = Store::default();
    for work in [63, 64] {
        let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_work(work));
        let registration = manager.apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        });
        if work == 64 {
            let update = registration.expect("the id and its separator fit exactly");
            assert!(update.calls.is_empty());
        } else {
            assert!(
                matches!(registration, Err(QueryCommandError::Limit(e)) if e.flags.size_limited)
            );
        }
        manager.audit_cache(std::iter::empty(), work == 64).unwrap();
    }
}
