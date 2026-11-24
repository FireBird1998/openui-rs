//! One tool delivery spends one work allowance (#222, answering #200).
//!
//! Raw validation, reference preparation, snapshot admission and the reader cascade of one
//! delivery all draw on one allowance. The program below makes every reader
//! read a cached result version that was hidden when the delivery began, so
//! each read prepares a fresh reference. Before #222 each preparation ran on a
//! counter of its own and one delivery could do many times its work limit.
//! The hidden `last_delivery_work` record sums what every phase of the last
//! delivery measured on its own counter, with suspended-reader rows recorded separately.
use openui_lang::{JsonValue, Library, Limits, parse};
use openui_runtime::{
    QueryCommand, QueryManager, ReaderDiagnostic, Store, ToolCall, ToolOutcome, Value,
};
use std::collections::HashMap;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);

type Calls = HashMap<String, ToolCall>;

// Index each wanted call by "tool|first string argument".
fn take(calls: &mut Calls, update: Vec<ToolCall>) {
    for c in update {
        let arg = match c.args.as_ref() {
            Some(Value::Obj(o)) => o
                .iter()
                .find_map(|(_, v)| match v {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
        calls.insert(format!("{}|{}", c.tool_name, arg), c);
    }
}

fn complete(m: &mut QueryManager, calls: &mut Calls, key: &str, v: &JsonValue) {
    let call = calls
        .remove(key)
        .unwrap_or_else(|| panic!("no call {key}: {:?}", calls.keys().collect::<Vec<_>>()));
    let u = m
        .apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(Some(v)),
        })
        .unwrap_or_else(|e| panic!("complete {key}: {e:?}"));
    take(calls, u.calls);
}

fn invalidate(m: &mut QueryManager, calls: &mut Calls, id: &str) {
    let u = m
        .apply(QueryCommand::Invalidate {
            statement_ids: &[id],
        })
        .unwrap();
    take(calls, u.calls);
}

fn s(v: &str) -> JsonValue {
    JsonValue::String(v.into())
}

struct Outcome {
    cut: bool,
    work: openui_runtime::DeliveryWork,
}

/// Per i, `b{i}` ends on key A (data "a") with key B's big result hidden in
/// its previous slot. Completing `s` moves every `g{i}` to a cached key, so
/// every `b{i}` returns to B and every `r{i}` reads a version that was hidden
/// when the delivery began.
fn run(n: usize, big: usize, limits: Limits, cache: Option<usize>) -> Outcome {
    let mut src = String::from("s=Query(\"s\")\np=Query(\"p\")\n");
    for i in 0..n {
        src.push_str(&format!(
            "g{i}=Query(\"g{i}\",{{t:s}})\nd{i}=Query(\"g{i}\",{{t:\"S2\"}})\n\
             b{i}=Query(\"b{i}\",{{x:g{i}}})\nc{i}=Query(\"b{i}\",{{x:\"A\"}})\n\
             r{i}=Query(\"r{i}\",{{y:b{i}==null}})\n"
        ));
    }
    let p = parse(&src, &Library::from_json_schema(&JsonValue::Null), None);
    assert!(!p.size_limited && !p.depth_limited);
    let store = Store::default();
    let mut m = QueryManager::with_limits(limits);
    if let Some(c) = cache {
        m = m.with_cache_limit(c);
    }
    let mut calls = Calls::new();
    let u = m
        .apply(QueryCommand::Evaluate {
            program: &p,
            store: &store,
        })
        .unwrap();
    take(&mut calls, u.calls);
    complete(&mut m, &mut calls, "s|", &s("S1"));
    complete(&mut m, &mut calls, "p|", &s(&"p".repeat(big + 8)));
    for i in 0..n {
        complete(&mut m, &mut calls, &format!("g{i}|S2"), &s("B"));
        complete(&mut m, &mut calls, &format!("g{i}|S1"), &s("A"));
        complete(&mut m, &mut calls, &format!("b{i}|A"), &s("a"));
    }
    let mut keys: Vec<_> = calls.keys().cloned().collect();
    keys.sort();
    for k in keys {
        if k.starts_with('r') {
            complete(&mut m, &mut calls, &k, &JsonValue::Null);
        }
    }
    for i in 0..n {
        invalidate(&mut m, &mut calls, &format!("g{i}"));
        complete(&mut m, &mut calls, &format!("g{i}|S1"), &s("B"));
        let bigv = s(&format!("{i}{}", "x".repeat(big)));
        complete(&mut m, &mut calls, &format!("b{i}|B"), &bigv);
        assert!(
            matches!(m.snapshot().get(&format!("b{i}")), Some(Value::Str(v)) if v.len() > big),
            "big result for b{i} refused: {:?}",
            m.snapshot().get("__openui_errors")
        );
        invalidate(&mut m, &mut calls, &format!("g{i}"));
        complete(&mut m, &mut calls, &format!("g{i}|S1"), &s("A"));
        assert_eq!(
            m.snapshot().get(&format!("b{i}")),
            Some(&Value::Str("a".into()))
        );
    }
    assert!(m.reader_diagnostic().is_none());
    invalidate(&mut m, &mut calls, "s");
    let call = calls.remove("s|").unwrap();
    let sv = s("S2");
    m.apply(QueryCommand::Complete {
        id: &call.id,
        outcome: ToolOutcome::Result(Some(&sv)),
    })
    .unwrap();
    assert_listing_bound(&m);
    Outcome {
        cut: matches!(
            m.reader_diagnostic(),
            Some(ReaderDiagnostic::PropagationLimit(_))
        ),
        work: m
            .last_delivery_work()
            .expect("the delivery records its work"),
    }
}

/// The delivery is cut, its ordinary debits stay within the limit, and
/// every reference it prepared was paid for from that one allowance.
fn assert_within(o: &Outcome, limit: usize) {
    let w = o.work;
    println!(
        "limit {limit}: spent {} ({:.3}x), prepared {} bytes",
        w.spent,
        w.spent as f64 / limit as f64,
        w.prepared_bytes
    );
    assert!(o.cut, "the delivery runs out of work");
    assert_eq!(w.limit, limit);
    assert!(w.spent <= limit, "spent {} of {limit}", w.spent);
    assert!(
        w.prepared_bytes <= w.spent,
        "prepared {} bytes but spent only {}",
        w.prepared_bytes,
        w.spent
    );
}

// The parameters of #222's own reproduction (2,200-byte results at 10,000,
// 78,000 and 240,000 at a million) no longer set up: the deliveries that put
// the big results in the cache are themselves cut at the limit now. These are
// the largest that still do; the work before #222 is the uncharged
// preparation alone, measured with counters in the core's preparation walk.

// With the former 400-byte fixture before #222: 4,874 units of uncharged
// preparation of read versions and
// 570 of the statement results (0.54x), on top of the cut cascade's own 1x
// and the snapshot admissions, each on a counter of its own. At 350 bytes the
// charged name walks still allow the setup to reach the final delivery.
#[test]
fn a_small_limit_cuts_the_delivery_within_the_limit() {
    let limit = 10_000;
    let o = run(12, 350, Limits::DEFAULT.with_work(limit), None);
    assert_within(&o, limit);
}

// Before #222: 4,000,690 units of uncharged preparation (4.0x) on top of the
// cut cascade, at the default cache cap.
#[test]
fn a_million_with_the_default_cache_cap_stays_within_the_limit() {
    let limit = 1_000_000;
    let o = run(100, 40_000, Limits::DEFAULT.with_work(limit), None);
    assert_within(&o, limit);
    assert!(o.work.prepared_bytes > limit / 2);
}

// Before #222: 6,000,690 units (6.0x) once the cache cap no longer bounds it.
#[test]
fn a_million_with_a_raised_cache_cap_stays_within_the_limit() {
    let limit = 1_000_000;
    let o = run(
        100,
        60_000,
        Limits::DEFAULT.with_work(limit),
        Some(2_000_000_000),
    );
    assert_within(&o, limit);
    assert!(o.work.prepared_bytes > limit / 2);
}

// Before #222: 7,202,290 units (7.2x) with three hundred readers.
#[test]
fn many_readers_with_a_raised_cache_cap_stay_within_the_limit() {
    let limit = 1_000_000;
    let o = run(
        300,
        24_000,
        Limits::DEFAULT.with_work(limit),
        Some(2_000_000_000),
    );
    assert_within(&o, limit);
}

/// #200: the worst ordinary work of one delivery, over sizes that move the cut
/// between the cascade, admission and preparation, is at most 1x the limit.
#[test]
fn the_worst_total_of_one_delivery_is_at_most_the_limit() {
    let limit = 10_000;
    let mut worst = 0usize;
    for n in [1, 2, 4, 8, 12] {
        for big in [10, 100, 200, 350] {
            let o = run(n, big, Limits::DEFAULT.with_work(limit), None);
            assert!(o.work.spent <= limit, "n {n} big {big}: {}", o.work.spent);
            assert!(o.work.prepared_bytes <= o.work.spent);
            worst = worst.max(o.work.spent);
        }
    }
    println!("worst total {worst} of {limit}");
    assert!(worst <= limit);
}

/// At default limits the same program is not cut and its total is a small
/// fraction of the allowance.
#[test]
fn default_limits_do_not_cut_the_reproduction() {
    let o = run(12, 2_200, Limits::DEFAULT, None);
    assert!(!o.cut);
    assert!(o.work.spent < o.work.limit / 10);
    assert!(o.work.prepared_bytes <= o.work.spent);
}

// Measure published error bodies independently of the code that constructs them.
fn assert_listing_bound(m: &QueryManager) {
    let work = m.last_delivery_work().unwrap();
    let Value::Arr(rows) = m.snapshot().get("__openui_errors").unwrap() else {
        panic!("error array")
    };
    let mut total = 0;
    let mut largest = 0;
    assert_eq!(rows.len(), m.failed_query_ids().len());
    for (row, id) in rows.iter().zip(m.failed_query_ids()) {
        let Value::Obj(fields) = row else {
            panic!("error row")
        };
        assert_eq!(fields.get("statementId"), Some(&Value::Str(id)));
        let session = openui_lang::EvaluationSession::new(&(), 0);
        let cost = session.with_evaluator(|e| {
            assert!(e.reserve_value(row));
            e.work_debits()
        });
        total += cost;
        largest = largest.max(cost);
    }
    assert_eq!(work.listing, total);
    assert!(work.spent <= work.limit);
    assert!(work.spent + total <= work.limit + rows.len() * largest);
    println!(
        "limit {} ordinary {} rows {} listing {total} largest {largest}",
        work.limit,
        work.spent,
        rows.len()
    );
}

#[test]
fn the_only_charged_excess_over_w_is_the_suspended_readers_error_listing() {
    // The reverse-dependency walk visits 1 or 64 edges after the cut.
    // Charging an extra unit per edge would give 248 vs 247 and 61,820 vs
    // 61,776. These 1 and 44 units of apparent excess come from including
    // program bookkeeping in the bound on data work and listing.
    for (n, limit, listing_excess) in [(1, 65, 182), (64, 50_000, 11_756)] {
        let mut text = String::from("a=Query(\"a\")");
        for i in 0..n {
            let (tool, expr) = if n == 1 {
                ("b".into(), "a")
            } else {
                (format!("b{i}"), "a+a")
            };
            text.push_str(&format!("\nb{i}=Query(\"{tool}\",{{v:{expr}}})"));
        }
        let p = parse(&text, &Library::from_json_schema(&JsonValue::Null), None);
        let mut m = QueryManager::with_limits(Limits::DEFAULT.with_work(limit));
        let mut calls = m
            .apply(QueryCommand::Evaluate {
                program: &p,
                store: &Store::default(),
            })
            .unwrap()
            .calls;
        let source = calls.pop().unwrap();
        let value = if n == 1 {
            Value::Num(1.)
        } else {
            Value::Str("x".repeat(300))
        };
        m.apply(QueryCommand::Complete {
            id: &source.id,
            outcome: ToolOutcome::ResultValue(&value),
        })
        .unwrap();
        assert!(matches!(
            m.reader_diagnostic(),
            Some(ReaderDiagnostic::PropagationLimit(_))
        ));
        let expected: Vec<_> = (0..n).map(|i| format!("b{i}")).collect();
        assert_eq!(m.failed_query_ids(), expected);
        let work = m.last_delivery_work().unwrap();
        assert_eq!(work.spent, limit);
        assert_eq!(work.listing, listing_excess);
        assert_listing_bound(&m);
    }
}

#[test]
fn readers_share_one_paid_string_comparison_on_an_unchanged_refetch() {
    const BYTES: usize = 16_384;
    // Raw validation (B + 2), snapshot admission (2B + 3), and one comparison (B).
    const LIMIT: usize = 4 * BYTES + 5;
    let lib = Library::from_json_schema(&JsonValue::Null);
    let source = r#"s=Query("s")"#;
    let value = s(&"x".repeat(BYTES));
    for (count, limit) in [(0, LIMIT), (1, LIMIT), (128, LIMIT), (128, LIMIT - 1)] {
        let mut m = QueryManager::with_limits(Limits::DEFAULT.with_work(limit));
        let store = Store::default();
        let mut calls = Calls::new();
        let initial = parse(source, &lib, None);
        take(
            &mut calls,
            m.apply(QueryCommand::Evaluate {
                program: &initial,
                store: &store,
            })
            .unwrap()
            .calls,
        );
        complete(&mut m, &mut calls, "s|", &value);
        let mut program = source.to_owned();
        for i in 0..count {
            program.push_str(&format!("\nr{i}=Query(\"reader\",{{v:s==null}})"));
        }
        let p = parse(&program, &lib, None);
        let mut input = openui_runtime::TreeInput::new(&p);
        input.library = Some(&lib);
        let (_, registered) = m
            .evaluate_transaction(&input, &store, |_| {
                Ok::<_, openui_runtime::QueryCommandError>(())
            })
            .unwrap();
        let pending = registered.calls;
        assert_eq!(pending.len(), usize::from(count > 0));
        invalidate(&mut m, &mut calls, "s");
        let source = calls.remove("s|").unwrap();
        let staged = m
            .stage_tool_call(
                &source.id,
                openui_runtime::ToolCallOutcome::Result(Value::Str("x".repeat(BYTES))),
            )
            .unwrap();
        // A host can inspect the staged delivery repeatedly before finishing it.
        for _ in 0..3 {
            assert!(!staged.needs_reader_context());
        }
        assert!(staged.finish(None).unwrap().calls.is_empty());
        assert!(!source.id.is_pending());
        let work = m.last_delivery_work().unwrap();
        assert_eq!(
            work.spent - work.publication,
            if count == 0 { LIMIT - BYTES } else { limit }
        );
        assert_eq!(m.snapshot().get("s"), Some(&Value::from_json(&value)));
        if limit == LIMIT {
            assert!(m.reader_diagnostic().is_none());
            assert!(pending.iter().all(|c| c.id.is_pending()));
            assert_eq!(work.listing, 0);
        } else {
            assert!(matches!(
                m.reader_diagnostic(),
                Some(ReaderDiagnostic::PropagationLimit(_))
            ));
            assert!(pending.iter().all(|c| !c.id.is_pending()));
            assert_eq!(
                m.failed_query_ids(),
                (0..count).map(|i| format!("r{i}")).collect::<Vec<_>>()
            );
            assert_listing_bound(&m);
        }
    }
}

#[test]
fn input_credit_from_a_store_read_preserves_every_work_debit() {
    let p = parse(
        r#"$pref="small"
src=Query("source")
reader=Query("reader",{v:src,p:$pref})"#,
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    for (len, expected) in [(8, 476), (2048, 29_036)] {
        let mut m = QueryManager::default();
        let mut store = Store::default();
        let pref = Value::Str("p".repeat(len));
        store.set("$pref", Some(pref.clone())).unwrap();
        let call = m
            .apply(QueryCommand::Evaluate {
                program: &p,
                store: &store,
            })
            .unwrap()
            .calls
            .pop()
            .unwrap();
        let calls = m
            .apply(QueryCommand::Complete {
                id: &call.id,
                outcome: ToolOutcome::Result(Some(&s("tea"))),
            })
            .unwrap()
            .calls;
        let [reader] = calls.as_slice() else {
            panic!("one reader")
        };
        assert_eq!(reader.tool_name, "reader");
        let Some(Value::Obj(args)) = &reader.args else {
            panic!("object arguments")
        };
        assert_eq!(args.get("p"), Some(&pref));
        let work = m.last_delivery_work().unwrap();
        assert_eq!(work.spent - work.publication, expected);
        assert_eq!(work.listing, 0);
    }
}

#[test]
fn a_prior_provider_error_leaves_the_reader_its_program_scaled_allowance() {
    for (argument, pref_len) in [("v:s", 0), ("v:s,p:$pref", 2048)] {
        let p = parse(
            &format!(
                "notice=Query(\"notice\")\ns=Query(\"s\")\nr=Query(\"reader\",{{{argument}}})"
            ),
            &Library::from_json_schema(&JsonValue::Null),
            None,
        );
        let mut store = Store::default();
        let pref = Value::Str("p".repeat(pref_len));
        store.set("$pref", Some(pref.clone())).unwrap();
        for kib in [0, 64, 128, 192, 256] {
            let mut manager = QueryManager::default();
            let mut calls = Calls::new();
            take(
                &mut calls,
                manager
                    .apply(QueryCommand::Evaluate {
                        program: &p,
                        store: &store,
                    })
                    .unwrap()
                    .calls,
            );
            let notice = calls.remove("notice|").unwrap();
            let error = "x".repeat(kib * 1024);
            let update = manager
                .apply(QueryCommand::Complete {
                    id: &notice.id,
                    outcome: ToolOutcome::CallFailed(&error),
                })
                .unwrap();
            assert!(update.calls.is_empty());
            let source = calls.remove("s|").unwrap();
            let update = manager
                .apply(QueryCommand::Complete {
                    id: &source.id,
                    outcome: ToolOutcome::Result(Some(&JsonValue::from(1))),
                })
                .unwrap();
            let [reader] = update.calls.as_slice() else {
                panic!("missing reader at {kib} KiB, preference length {pref_len}")
            };
            assert_eq!(reader.tool_name, "reader");
            let Some(Value::Obj(args)) = &reader.args else {
                panic!("expected reader arguments")
            };
            assert_eq!(args.get("v"), Some(&Value::Num(1.0)));
            assert_eq!(args.get("p"), (pref_len > 0).then_some(&pref));
            assert_eq!(args.len(), if pref_len == 0 { 1 } else { 2 });
            assert!(!source.id.is_pending());
            assert_eq!(manager.failed_query_ids(), ["notice"]);
            assert!(manager.reader_diagnostic().is_none());
            assert_eq!(manager.last_delivery_work().unwrap().listing, 0);
        }
    }
}

#[test]
fn default_limit_refetch_charges_raw_text_and_the_previous_reader_values() {
    let mut text = String::from("s=Query(\"s\")");
    for i in 0..2000 {
        text.push_str(&format!("\nr{i}=Query(\"r\",{{v:s==null}})"));
    }
    let p = parse(&text, &Library::from_json_schema(&JsonValue::Null), None);
    for (previous, cut) in [
        (31_602, false),
        (31_603, true),
        (31_635, true),
        (31_636, true),
    ] {
        let mut m = QueryManager::default();
        let mut calls = Calls::new();
        take(
            &mut calls,
            m.apply(QueryCommand::Evaluate {
                program: &p,
                store: &Store::default(),
            })
            .unwrap()
            .calls,
        );
        complete(&mut m, &mut calls, "s|", &s(&"x".repeat(previous)));
        let reader = calls.remove("r|").unwrap();
        invalidate(&mut m, &mut calls, "s");
        complete(&mut m, &mut calls, "s|", &s(&"y".repeat(65_000)));
        assert!(calls.is_empty());
        assert_eq!(reader.id.is_pending(), !cut, "previous length {previous}");
        assert_eq!(
            matches!(
                m.reader_diagnostic(),
                Some(ReaderDiagnostic::PropagationLimit(_))
            ),
            cut
        );
        assert_eq!(m.failed_query_ids().len(), if cut { 2000 } else { 0 });
        assert_eq!(m.snapshot().get("s"), Some(&Value::Str("y".repeat(65_000))));
        assert_listing_bound(&m);
    }
}

#[test]
fn mutation_result_validation_spends_the_delivery_allowance() {
    let p = parse(
        r#"m=Mutation("m")"#,
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    for (json, units) in [
        (s("tea"), 5),
        (openui_lang::parse_json(r#"{"a":["茶"]}"#).unwrap(), 10),
    ] {
        for native in [false, true] {
            let mut m = QueryManager::default();
            m.register_mutations(&openui_runtime::TreeInput::new(&p), &Store::default())
                .unwrap();
            let call = m.fire_mutation("m", None).unwrap().calls.pop().unwrap();
            let value = Value::from_json(&json);
            let outcome = if native {
                ToolOutcome::ResultValue(&value)
            } else {
                ToolOutcome::Result(Some(&json))
            };
            m.apply(QueryCommand::Complete {
                id: &call.id,
                outcome,
            })
            .unwrap();
            let Value::Obj(result) = m.snapshot().get("m").unwrap() else {
                panic!("mutation result")
            };
            assert_eq!(result.get("data"), Some(&value));
            let work = m.last_delivery_work().unwrap();
            assert_eq!((work.spent - work.publication, work.listing), (units, 0));
        }
    }
}

#[test]
fn raw_validation_stops_at_the_allowance_before_an_unchecked_value_can_settle() {
    let p = parse(
        r#"m=Mutation("m")"#,
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let mut m = QueryManager::with_limits(Limits::DEFAULT.with_work(128));
    m.register_mutations(&openui_runtime::TreeInput::new(&p), &Store::default())
        .unwrap();
    let call = m.fire_mutation("m", None).unwrap().calls.pop().unwrap();
    let before = m.snapshot_json().to_owned();
    let audit = m.last_delivery_work();
    let wide = JsonValue::Array(vec![JsonValue::Null; 256]);
    let native = Value::from_json(&wide);
    // The native representation can contain syntax. If the validity walk
    // cannot finish, the host still owns the receipt.
    assert!(matches!(
        m.apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::ResultValue(&native),
        }),
        Err(openui_runtime::QueryCommandError::Limit(_))
    ));
    assert!(call.id.is_pending());
    assert_eq!(m.snapshot_json(), before);
    assert_eq!(m.last_delivery_work(), audit);
    assert_eq!(m.cache_bytes(), 0);
    // Host JSON is already known to be data. Exceeding the scan allowance
    // delivers the ordinary result-limit failure on that same receipt.
    m.apply(QueryCommand::Complete {
        id: &call.id,
        outcome: ToolOutcome::Result(Some(&wide)),
    })
    .unwrap();
    assert!(!call.id.is_pending());
    let Value::Obj(result) = m.snapshot().get("m").unwrap() else {
        panic!("mutation result")
    };
    assert_eq!(result.get("status"), Some(&Value::Str("error".into())));
    let work = m.last_delivery_work().unwrap();
    assert_eq!((work.spent, work.listing), (128, 0));
}

#[test]
fn missing_tool_validation_and_hint_text_spend_delivery_work() {
    let p = parse(
        r#"m=Mutation("m")"#,
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    let mut m = QueryManager::default();
    m.register_mutations(&openui_runtime::TreeInput::new(&p), &Store::default())
        .unwrap();
    let call = m.fire_mutation("m", None).unwrap().calls.pop().unwrap();
    m.apply(QueryCommand::Complete {
        id: &call.id,
        outcome: ToolOutcome::ToolNotFound(vec!["b", "a"]),
    })
    .unwrap();
    let work = m.last_delivery_work().unwrap();
    // Two name visits, two name bytes plus two separator bytes per name,
    // and one byte for the tool name in the failure.
    assert_eq!((work.spent - work.publication, work.listing), (9, 0));
}

#[test]
fn publication_of_eight_retained_mutation_results_spends_only_the_delivery_balance() {
    let library = Library::from_json_schema(&JsonValue::Null);
    let statements = (0..9)
        .map(|i| format!("m{i}=Mutation(\"tool\")"))
        .collect::<Vec<_>>()
        .join("\n");
    let program = parse(&statements, &library, None);
    let source = parse("s=Query(\"s\")", &library, None);
    for bytes in [8, 256, 900] {
        let store = Store::default();
        let mut m = QueryManager::with_limits(Limits::DEFAULT.with_work(1024));
        let receipt = m
            .apply(QueryCommand::Evaluate {
                program: &source,
                store: &store,
            })
            .unwrap()
            .calls
            .pop()
            .unwrap();
        m.apply(QueryCommand::Evaluate {
            program: &program,
            store: &store,
        })
        .unwrap();
        m.register_mutations(&openui_runtime::TreeInput::new(&program), &store)
            .unwrap();
        let payload = Value::Str("x".repeat(bytes));
        for i in 0..9 {
            let call = m
                .fire_mutation(&format!("m{i}"), None)
                .unwrap()
                .calls
                .pop()
                .unwrap();
            let outcome = if i == 8 {
                ToolOutcome::Result(None)
            } else {
                ToolOutcome::ResultValue(&payload)
            };
            assert!(
                m.apply(QueryCommand::Complete {
                    id: &call.id,
                    outcome
                })
                .unwrap()
                .changed
            );
        }
        let work = m.last_delivery_work().unwrap();
        println!(
            "eight retained at {bytes}: {} publication, {} ordinary",
            work.publication, work.spent
        );
        assert_eq!(work.publication, work.spent);
        assert_eq!(work.listing, 0);
        if bytes == 8 {
            assert!((8 * bytes..1024).contains(&work.publication));
        } else {
            assert_eq!(work.publication, 1024);
        }
        let before = m.snapshot_json().to_owned();
        let published_buffer = m.snapshot_json().as_ptr();
        let Value::Obj(last) = m.snapshot().get("m8").unwrap() else {
            panic!("ninth mutation")
        };
        assert_eq!(last.get("status"), Some(&Value::Str("success".into())));
        assert!(last.get("data").is_none());
        for i in 0..8 {
            let Value::Obj(row) = m.snapshot().get(&format!("m{i}")).unwrap() else {
                panic!("mutation result")
            };
            assert_eq!(row.get("data"), Some(&payload));
        }
        let update = m
            .apply(QueryCommand::Complete {
                id: &receipt.id,
                outcome: ToolOutcome::Result(None),
            })
            .unwrap();
        assert!(!update.changed);
        assert_eq!(m.last_delivery_work().unwrap().spent, 0);
        assert_eq!(m.snapshot_json(), before);
        assert_eq!(m.snapshot_json().as_ptr(), published_buffer);
    }
}

#[test]
fn a_store_derived_cache_key_is_charged_once_on_delivery() {
    let p = parse(
        r#"s=Query("s",{arg:$token})"#,
        &Library::from_json_schema(&JsonValue::Null),
        None,
    );
    for bytes in [1, 1024, 16384, 65536] {
        let mut store = Store::default();
        store
            .set("$token", Some(Value::Str("x".repeat(bytes))))
            .unwrap();
        let mut m = QueryManager::default();
        let call = m
            .apply(QueryCommand::Evaluate {
                program: &p,
                store: &store,
            })
            .unwrap()
            .calls
            .pop()
            .unwrap();
        m.apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(Some(&JsonValue::from(1))),
        })
        .unwrap();
        assert_eq!(m.snapshot().get("s"), Some(&Value::Num(1.0)));
        let work = m.last_delivery_work().unwrap();
        println!(
            "store {bytes}: {} ordinary, {} publication",
            work.spent, work.publication
        );
        // Both the serialized argument and its state dependency contain the token.
        assert_eq!(work.spent - work.publication, 4 + 28 + 2 * bytes);
        assert_eq!(work.listing, 0);
    }
}

#[test]
fn an_older_mutation_receipt_compares_two_individually_admitted_results() {
    let library = Library::from_json_schema(&JsonValue::Null);
    let store = Store::default();
    let mut m = QueryManager::with_limits(Limits::DEFAULT.with_result_size(1024));
    let mut calls = Vec::new();
    for tool in ["first", "second", "third"] {
        let p = parse(&format!("m=Mutation(\"{tool}\")"), &library, None);
        m.register_mutations(&openui_runtime::TreeInput::new(&p), &store)
            .unwrap();
        calls.push(m.fire_mutation("m", None).unwrap().calls.pop().unwrap());
    }
    // The first two objects have the same bytes. Property order distinguishes the third.
    let text = "x".repeat(900);
    for (call, reversed, changed) in [
        (&calls[2], false, true),
        (&calls[1], false, false),
        (&calls[0], true, true),
    ] {
        let fields = if reversed {
            [("b", ""), ("a", &text[..])]
        } else {
            [("a", &text[..]), ("b", "")]
        };
        let mut object = openui_runtime::JsObject::new();
        for (key, value) in fields {
            object.define(key, Value::Str(value.to_owned()));
        }
        let value = Value::Obj(std::sync::Arc::new(object));
        let before = m.snapshot_json().to_owned();
        let update = m
            .apply(QueryCommand::Complete {
                id: &call.id,
                outcome: ToolOutcome::ResultValue(&value),
            })
            .unwrap();
        assert_eq!(update.changed, changed);
        assert_eq!(m.snapshot_json() != before, changed);
        assert!(!call.id.is_pending());
        if !changed {
            assert!(m.last_delivery_work().unwrap().publication >= 900);
        }
    }
}
