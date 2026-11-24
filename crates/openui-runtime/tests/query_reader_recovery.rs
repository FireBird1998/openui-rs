//! Public completion protocols and the shared reader evaluation allowance.
use openui_lang::{JsonValue, Library, Limits, parse};
use openui_runtime::{
    EvaluationFailure, QueryCommand, QueryCommandError, QueryManager, ReaderDiagnostic, Store,
    ToolCallOutcome, ToolOutcome, TreeInput, Value,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

fn audit_calls(manager: &QueryManager, calls: &[openui_runtime::ToolCall]) {
    manager
        .audit_cache(calls.iter().map(|call| &call.id), true)
        .unwrap();
}

#[test]
fn both_completion_paths_require_each_borrowed_reader_context() {
    use openui_runtime::{BindingProps, LibraryFunctions};

    let lib = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        r#"a=Query("source")
b=Query("reader",{x:a})"#,
        &lib,
        None,
    );
    let functions = LibraryFunctions::new(&JsonValue::Null);
    let actions = openui_lang::parse_json("{}").unwrap();
    let props = BindingProps::new();
    let input = TreeInput::new(&program);
    let store = Store::default();
    for (name, input) in [
        ("library", input.clone().with_library(&lib)),
        ("actions", input.clone().with_actions(&actions)),
        ("functions", input.clone().with_functions(&functions)),
        ("binding props", input.with_binding_props(&props)),
    ] {
        let mut manager = QueryManager::default();
        let (_, update) = manager
            .evaluate_transaction(&input, &store, |_| Ok::<_, QueryCommandError>(()))
            .unwrap();
        let [source] = update.calls.as_slice() else {
            panic!("expected the source call")
        };
        audit_calls(&manager, &update.calls);
        let before = manager.snapshot_json().to_owned();
        let result = || ToolCallOutcome::Result(Value::Num(7.0));
        for owned in [false, true] {
            let completion = if owned {
                manager.complete_tool_call(&source.id, result())
            } else {
                manager.apply(QueryCommand::Complete {
                    id: &source.id,
                    outcome: ToolOutcome::Result(Some(&JsonValue::from(7))),
                })
            };
            let Err(error) = completion else {
                panic!("{name} alone must require context")
            };
            let expected = "QueryManager::complete_tool_call_with_context";
            assert!(
                matches!(error, QueryCommandError::Evaluation(EvaluationFailure::MissingContext { recovery }) if recovery == expected)
            );
            assert!(error.to_string().contains(expected));
            assert!(source.id.is_pending());
            assert_eq!(manager.snapshot_json(), before);
            audit_calls(&manager, &update.calls);
        }

        let update = manager
            .complete_tool_call_with_context(&source.id, result(), &input, &store)
            .unwrap();
        assert!(!source.id.is_pending());
        let [reader] = update.calls.as_slice() else {
            panic!("expected the reader call")
        };
        assert_eq!(reader.tool_name, "reader");
        assert_eq!(reader.arguments_json().as_deref(), Some(r#"{"x":7}"#));
        audit_calls(&manager, &update.calls);
    }
}

#[test]
fn a_dropped_store_keeps_its_last_committed_reader_inputs() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let p = parse("a=Query(\"a\")\nb=Query(\"b\",{r:a,n:$n})", &lib, None);
    let mut m = QueryManager::default();
    let id = {
        let mut store = Store::default();
        store.set("$n", Some(Value::Num(1.))).unwrap();
        let call = m
            .apply(QueryCommand::Evaluate {
                program: &p,
                store: &store,
            })
            .unwrap()
            .calls
            .pop()
            .unwrap();
        store.set("$n", Some(Value::Num(9.))).unwrap();
        call.id
    };
    let u = m
        .apply(QueryCommand::Complete {
            id: &id,
            outcome: ToolOutcome::Result(None),
        })
        .unwrap();
    assert_eq!(
        u.calls[0].arguments_json().as_deref(),
        Some(r#"{"r":null,"n":9}"#)
    );
}

#[test]
fn a_wide_fanout_spends_one_allowance_and_releases_the_provider_on_a_cut() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let store = Store::default();
    let mut source = String::from("a=Query(\"a\")");
    for i in 0..64 {
        source.push_str(&format!("\nb{i}=Query(\"b{i}\",{{v:a+a}})"));
    }
    let p = parse(&source, &lib, None);
    let result = Value::Str("x".repeat(300));
    let check = |p: &openui_lang::ParseResult, room, expected| {
        let mut m =
            QueryManager::with_limits(Limits::DEFAULT.with_work(room).with_result_size(4_000_000));
        let mut first = m
            .apply(QueryCommand::Evaluate {
                program: p,
                store: &store,
            })
            .unwrap();
        let id = first.calls.pop().unwrap().id;
        let u = m
            .complete_tool_call(&id, ToolCallOutcome::Result(result.clone()))
            .unwrap();
        assert_eq!(u.calls.len(), expected);
        assert!(!id.is_pending());
        if expected == 0 {
            assert!(matches!(
                m.reader_diagnostic(),
                Some(ReaderDiagnostic::PropagationLimit(_))
            ));
            assert_eq!(m.snapshot().get("a"), Some(&result));
            let snapshot = openui_lang::parse_json(m.snapshot_json()).unwrap();
            let rows = snapshot["__openui_errors"].as_array().unwrap();
            assert_eq!(rows.len(), 64);
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(row["statementId"], format!("b{i}"));
                assert_eq!(row["code"], "reader-limit");
                assert_eq!(m.snapshot().get(&format!("b{i}")), Some(&Value::Null));
                assert!(m.current_entry_is_served(&format!("b{i}"), true, std::iter::empty()));
            }
            assert!(m.cache_entry_count() <= 130);
            let fixed = parse("a=Query(\"a\")\nb=Query(\"reader\",{v:a})", &lib, None);
            assert_eq!(
                m.apply(QueryCommand::Evaluate {
                    program: &fixed,
                    store: &store
                })
                .unwrap()
                .calls
                .len(),
                1
            );
            assert!(m.reader_diagnostic().is_none());
        } else {
            assert!(m.reader_diagnostic().is_none());
        }
    };
    check(&p, 50_000, 0);
    check(
        &parse("a=Query(\"a\")\nb=Query(\"b\",{v:a+a})", &lib, None),
        50_000,
        1,
    );
    check(&p, 4_000_000, 64);
}

#[test]
fn a_waiting_self_reader_is_released_by_another_holders_first_result() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let store = Store::default();
    let mut m = QueryManager::default();
    let original = parse("b=Query(\"T\",{})\na=Query(\"T\",{},\"ready\")", &lib, None);
    let call = m
        .apply(QueryCommand::Evaluate {
            program: &original,
            store: &store,
        })
        .unwrap()
        .calls
        .pop()
        .unwrap();
    let edited = parse("b=Query(\"T\",{})\na=Query(\"U\",{v:a})", &lib, None);
    assert!(
        m.apply(QueryCommand::Evaluate {
            program: &edited,
            store: &store
        })
        .unwrap()
        .calls
        .is_empty()
    );
    let v = JsonValue::from("ready");
    let u = m
        .apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(Some(&v)),
        })
        .unwrap();
    assert_eq!(u.calls.len(), 1);
    assert_eq!(
        u.calls[0].arguments_json().as_deref(),
        Some(r#"{"v":"ready"}"#)
    );
}

#[test]
fn a_full_diagnostic_list_cannot_prevent_an_admitted_source_from_settling() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let mut source = String::from("a=Query(\"a\")");
    for i in 0..24 {
        source.push_str(&format!("\nb{i}=Query(\"b\",{{v:a+a}})"));
    }
    let p = parse(&source, &lib, None);
    let mut m = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1200));
    let id = m
        .apply(QueryCommand::Evaluate {
            program: &p,
            store: &Store::default(),
        })
        .unwrap()
        .calls
        .pop()
        .unwrap()
        .id;
    let value = Value::Str("x".repeat(750));
    let u = m
        .complete_tool_call(&id, ToolCallOutcome::Result(value.clone()))
        .unwrap();
    assert_eq!(m.snapshot().get("a"), Some(&value));
    assert!(matches!(
        m.reader_diagnostic(),
        Some(ReaderDiagnostic::PropagationLimit(_))
    ));
    assert!(u.calls.is_empty() && !id.is_pending());
    let snapshot = openui_lang::parse_json(m.snapshot_json()).unwrap();
    let rows = snapshot["__openui_errors"].as_array().unwrap();
    assert_eq!(rows.len(), 24);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["statementId"], format!("b{i}"));
        assert_eq!(row["code"], "reader-limit");
        assert_eq!(m.snapshot().get(&format!("b{i}")), Some(&Value::Null));
    }
}

#[test]
fn accepted_reader_limits_pin_both_managers_calls_and_snapshots() {
    let name = "admitted-success-exhausts-the-reader-allowance";
    let pin = openui_lang::parse_json(include_str!(
        "../../../conformance/divergences/admitted-success-exhausts-the-reader-allowance.json"
    ))
    .unwrap();
    let golden = openui_lang::parse_json(include_str!(
        "../../../conformance/golden/query-reader-fixes.json"
    ))
    .unwrap();
    assert_eq!(pin["reference"], golden[name]["reference"]);
    let cases =
        openui_lang::parse_json(include_str!("../../../conformance/query-reader-fixes.json"))
            .unwrap();
    let c = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let p = parse(
        c["program"].as_str().unwrap(),
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let mut m = QueryManager::with_limits(
        Limits::DEFAULT.with_result_size(pin["resultSize"].as_u64().unwrap() as usize),
    );
    let first = m
        .apply(QueryCommand::Evaluate {
            program: &p,
            store: &Store::default(),
        })
        .unwrap();
    let value = c["steps"][1]["outcome"]["value"].as_str().unwrap();
    let observe = |m: &QueryManager, u: &openui_runtime::QueryUpdate, first: usize| {
        let calls = u
            .calls
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    r#"{{"call":{},"toolName":{},"args":{}}}"#,
                    first + i,
                    JsonValue::from(c.tool_name.clone()),
                    c.arguments_json().unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"snapshot":{},"calls":[{}],"changed":{}}}"#,
            m.snapshot_json(),
            calls,
            u.changed
        )
    };
    assert_eq!(pin["rust"][0], observe(&m, &first, 0));
    let u = m
        .complete_tool_call(
            &first.calls[0].id,
            ToolCallOutcome::Result(Value::Str(value.into())),
        )
        .unwrap();
    assert_eq!(pin["rust"][1], observe(&m, &u, 1));
    assert!(matches!(
        m.reader_diagnostic(),
        Some(ReaderDiagnostic::PropagationLimit(_))
    ));
}

#[test]
fn a_reader_cut_keeps_the_admitted_sources_remembered_run() {
    let p = parse(
        "a=Query(\"source\")\nb=Query(\"reader\",{v:a+a})",
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let mut m = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1024));
    let first = m
        .apply(QueryCommand::Evaluate {
            program: &p,
            store: &Store::default(),
        })
        .unwrap()
        .calls
        .pop()
        .unwrap();
    let run = |id| QueryCommand::Invalidate { statement_ids: id };
    assert!(m.apply(run(&["a", "b"])).unwrap().calls.is_empty());
    let value = JsonValue::from("x".repeat(700));
    let accepted = m
        .apply(QueryCommand::Complete {
            id: &first.id,
            outcome: ToolOutcome::Result(Some(&value)),
        })
        .unwrap();
    assert!(matches!(
        m.complete_tool_call(&first.id, ToolCallOutcome::Result(Value::from_json(&value))),
        Err(QueryCommandError::InvalidCall)
    ));
    assert!(!first.id.is_pending());
    assert_eq!(accepted.calls.len(), 1);
    assert_eq!(accepted.calls[0].tool_name, "source");
    assert!(accepted.calls[0].id.is_pending());
    assert!(matches!(
        m.reader_diagnostic(),
        Some(ReaderDiagnostic::PropagationLimit(_))
    ));
    assert!(m.apply(run(&["b"])).unwrap().calls.is_empty());
    assert!(
        m.complete_tool_call(&accepted.calls[0].id, ToolCallOutcome::Result(Value::Null))
            .unwrap()
            .calls
            .is_empty()
    );
}

#[test]
fn a_preflight_refusal_retires_the_call_when_a_bounded_reader_needs_context() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        "a=Query(\"source\")\nb=Query(\"reader\",{x:a,n:$n})\nc=Query(\"other\")",
        &lib,
        None,
    );
    let input = TreeInput::new(&program).with_library(&lib);
    let mut store = Store::default();
    let mut manager = QueryManager::default();
    let (_, update) = manager
        .evaluate_transaction(&input, &store, |_| Ok::<_, QueryCommandError>(()))
        .unwrap();
    let [source, other] = update.calls.as_slice() else {
        panic!("expected source and other")
    };
    let bounded = manager
        .complete_tool_call_with_context(
            &source.id,
            ToolCallOutcome::Result(Value::Num(f64::NAN)),
            &input,
            &store,
        )
        .unwrap();
    assert!(bounded.reader_rounds_limited);
    store.set("$n", Some(Value::Num(3.))).unwrap();
    let update = manager
        .complete_tool_call_refused(&other.id, openui_runtime::ResultLimit::Depth(4))
        .unwrap();
    assert!(update.reader_rounds_limited && update.calls.is_empty());
    assert!(matches!(
        manager.check_call(&other.id),
        Err(QueryCommandError::InvalidCall)
    ));
    let (_, update) = manager
        .evaluate_transaction(&input, &store, |_| Ok::<_, QueryCommandError>(()))
        .unwrap();
    assert_eq!(update.calls.len(), 1);
    assert_eq!(
        update.calls[0].arguments_json().as_deref(),
        Some(r#"{"x":null,"n":3}"#)
    );
}

#[test]
fn an_own_delivery_keeps_waiting_for_another_missing_input() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let store = Store::default();
    let mut manager = QueryManager::default();
    let first = parse("q=Query(\"u\",{x:null,y:null})\ns=Query(\"s\")", &lib, None);
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &first,
            store: &store,
        })
        .unwrap();
    let [own, other] = update.calls.as_slice() else {
        panic!("expected two calls")
    };
    let next = parse("q=Query(\"u\",{x:q,y:s})\ns=Query(\"s\")", &lib, None);
    assert!(
        manager
            .apply(QueryCommand::Evaluate {
                program: &next,
                store: &store
            })
            .unwrap()
            .calls
            .is_empty()
    );
    assert!(
        manager
            .complete_tool_call(&own.id, ToolCallOutcome::Result(Value::Num(7.0)))
            .unwrap()
            .calls
            .is_empty()
    );
    assert!(
        matches!(manager.reader_diagnostic(), Some(ReaderDiagnostic::SelfInputChanged(id)) if id == "q")
    );
    assert!(
        manager
            .apply(QueryCommand::Invalidate {
                statement_ids: &["q"]
            })
            .unwrap()
            .calls
            .is_empty()
    );
    let awake = manager
        .complete_tool_call(&other.id, ToolCallOutcome::Result(Value::Num(9.0)))
        .unwrap();
    assert_eq!(awake.calls.len(), 1);
    assert_eq!(
        awake.calls[0].arguments_json().as_deref(),
        Some(r#"{"x":7,"y":9}"#)
    );
}

#[test]
fn a_parse_cut_that_drops_a_waited_on_source_preserves_the_previous_calls() {
    let cases =
        openui_lang::parse_json(include_str!("../../../conformance/query-reader-fixes.json"))
            .unwrap();
    let pin = openui_lang::parse_json(include_str!(
        "../../../conformance/divergences/a-parser-limit-removes-a-readers-source.json"
    ))
    .unwrap();
    let golden = openui_lang::parse_json(include_str!(
        "../../../conformance/golden/query-reader-fixes.json"
    ))
    .unwrap();
    let name = pin["case"].as_str().unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    assert_eq!(golden[name]["reference"][2], pin["reference"]);
    let limits = Limits::DEFAULT
        .with_result_floor(1024)
        .with_amplification(4)
        .with_work(100_000)
        .with_depth(16);
    let library = Library::from_json_schema(&JsonValue::Null);
    let store = Store::with_limits(limits);
    let mut manager = QueryManager::with_limits(limits);
    let seed = parse(case["program"].as_str().unwrap(), &library, None);
    let keep = manager
        .apply(QueryCommand::Evaluate {
            program: &seed,
            store: &store,
        })
        .unwrap();
    let partial = openui_lang::parse_with_limits(
        case["steps"][1]["program"].as_str().unwrap(),
        &library,
        None,
        limits,
    );
    assert!(partial.size_limited);
    assert_eq!(partial.query_statements.len(), 1);
    let refused = manager.apply(QueryCommand::Evaluate {
        program: &partial,
        store: &store,
    });
    let Err(error @ QueryCommandError::SourceCut { limit, .. }) = refused else {
        panic!("a reader whose source was cut must refuse registration");
    };
    // The refusal names the reader and the source the parser dropped (#119).
    assert_eq!(
        error.to_string(),
        "query q0 reads q1, which the parser dropped (work or size limit exceeded); \
         the program's queries were not registered"
    );
    assert_eq!(pin["rust"]["sizeLimited"], limit.flags.size_limited);
    assert_eq!(pin["rust"]["depthLimited"], limit.flags.depth_limited);
    assert_eq!(
        openui_lang::parse_json(manager.snapshot_json()).unwrap(),
        pin["rust"]["snapshot"]
    );
    let pending: Vec<_> = keep
        .calls
        .iter()
        .filter(|c| c.id.is_pending())
        .map(|c| JsonValue::from(c.tool_name.clone()))
        .collect();
    assert_eq!(JsonValue::Array(pending), pin["rust"]["pending"]);
    manager
        .audit_cache(keep.calls.iter().map(|c| &c.id), true)
        .unwrap();

    // A cut elsewhere can still leave a usable query registration.
    let source = case["steps"][1]["program"]
        .as_str()
        .unwrap()
        .replace("r0:q1", "r0:42");
    let unrelated = openui_lang::parse_with_limits(&source, &library, None, limits);
    assert!(unrelated.size_limited);
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &unrelated,
            store: &store,
        })
        .unwrap();
    assert_eq!(update.calls.len(), 1);
    assert_eq!(update.calls[0].tool_name, "t0");
    manager.audit_output(&update.calls).unwrap();
}

#[test]
fn a_reader_argument_error_keeps_the_source_receipt_pending() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        r#"a=Query("source")
b=Query("reader",{x:""+a})"#,
        &lib,
        None,
    );
    let mut manager = QueryManager::default();
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &Store::default(),
        })
        .unwrap();
    let [source] = update.calls.as_slice() else {
        panic!("expected the source call")
    };
    audit_calls(&manager, &update.calls);
    let before = manager.snapshot_json().to_owned();
    let value = openui_lang::parse_json(r#"{"toString":null}"#).unwrap();
    let completion = manager.apply(QueryCommand::Complete {
        id: &source.id,
        outcome: ToolOutcome::Result(Some(&value)),
    });
    assert!(
        matches!(
            completion,
            Err(QueryCommandError::Evaluation(
                EvaluationFailure::CannotConvertToPrimitive
            ))
        ),
        "reader coercion must fail the delivery: {completion:?}"
    );
    assert!(source.id.is_pending());
    assert_eq!(manager.snapshot_json(), before);
    audit_calls(&manager, &update.calls);
}

#[test]
fn a_refusal_settles_even_when_a_stale_reader_argument_throws() {
    let lib = Library::from_json_schema(&JsonValue::Null);
    let program = parse(
        r#"a=Query("source")
b=Query("reader",{x:a,label:""+$label})"#,
        &lib,
        None,
    );
    let mut store = Store::default();
    let mut manager = QueryManager::default();
    let update = manager
        .apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        })
        .unwrap();
    let [source] = update.calls.as_slice() else {
        panic!("expected the source call")
    };
    audit_calls(&manager, &update.calls);

    // NaN differs from itself, so the reader is still stale after the last round.
    let bounded = manager
        .complete_tool_call(&source.id, ToolCallOutcome::Result(Value::Num(f64::NAN)))
        .unwrap();
    assert!(bounded.reader_rounds_limited);
    let [reader] = bounded.calls.as_slice() else {
        panic!("expected the reader call")
    };
    audit_calls(&manager, &bounded.calls);
    let bad = openui_lang::parse_json(r#"{"toString":null}"#).unwrap();
    store.set("$label", Some(Value::from_json(&bad))).unwrap();
    audit_calls(&manager, &bounded.calls);

    let limit = openui_runtime::ResultLimit::Depth(4);
    let update = manager
        .complete_tool_call_refused(&reader.id, limit)
        .expect("refusal must settle despite reader coercion");
    assert!(update.reader_rounds_limited);
    assert!(update.calls.is_empty());
    assert!(!reader.id.is_pending());
    let snapshot = openui_lang::parse_json(manager.snapshot_json()).unwrap();
    assert_eq!(
        snapshot["__openui_errors"][0]["message"],
        format!("Query \"reader\" failed: {limit}").as_str()
    );
    manager.audit_cache(std::iter::empty(), true).unwrap();
}

#[test]
fn refusing_a_suspended_readers_retry_preserves_its_error_and_receipts() {
    let program = parse(
        "s=Query(\"s\")\nr=Query(\"r\",{v:s+s})\nu=Query(\"u\")",
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let input = TreeInput::new(&program);
    let store = Store::default();
    let mut manager = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1200));
    let (_, update) = manager
        .evaluate_transaction(&input, &store, |_| Ok::<_, QueryCommandError>(()))
        .unwrap();
    let [source, sibling] = update.calls.as_slice() else {
        panic!("source and sibling")
    };
    manager
        .complete_tool_call(
            &source.id,
            ToolCallOutcome::Result(Value::Str("x".repeat(750))),
        )
        .unwrap();
    let before = manager.snapshot_json().to_owned();
    let diagnostic = manager.reader_diagnostic();
    let work = manager.last_delivery_work();
    for _ in 0..2 {
        assert!(matches!(
            manager.retry_suspended_queries(&input, &store),
            Err(QueryCommandError::Limit(_))
        ));
        assert_eq!(manager.snapshot_json(), before);
        assert_eq!(manager.reader_diagnostic(), diagnostic);
        assert_eq!(manager.failed_query_ids(), ["r"]);
        assert_eq!(manager.last_delivery_work(), work);
        assert!(sibling.id.is_pending());
        manager
            .audit_cache(std::iter::once(&sibling.id), true)
            .unwrap();
    }
}
