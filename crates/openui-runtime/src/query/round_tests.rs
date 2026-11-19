//! Compare the nodes actually evaluated during each round to upstream's loop.
use super::*;
use std::cell::RefCell;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

#[derive(Default)]
struct Observation {
    rounds: Vec<Vec<String>>,
    left: Vec<String>,
    key_moves: HashMap<String, usize>,
}
thread_local! { static TRACE: RefCell<Observation> = RefCell::new(Observation::default()); }

pub(super) fn moved(id: &str) {
    TRACE.with_borrow_mut(|t| *t.key_moves.entry(id.to_owned()).or_default() += 1);
}

pub(super) fn begin() {
    TRACE.with_borrow_mut(|t| t.rounds.push(Vec::new()));
}
pub(super) fn rebuilt(id: &str) {
    TRACE.with_borrow_mut(|t| {
        if let Some(r) = t.rounds.last_mut() {
            r.push(id.into())
        }
    });
}
pub(super) fn remaining(m: &QueryManager, nodes: &[Arc<Node>]) {
    TRACE.with_borrow_mut(|t| {
        t.left = nodes
            .iter()
            .filter(|n| m.stale(n))
            .map(|n| n.id.clone())
            .collect()
    });
}

#[test]
#[ignore = "differential replay requires the cached Bun reference"]
fn each_completion_order_rebuilds_the_same_nodes_in_the_same_rounds() {
    let mut child = Command::new("bun")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference"))
        .args(["--no-install", "query-scenarios.ts", "--reader-rounds"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    struct Reap(std::process::Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Reap(child);
    let lib = openui_lang::Library::from_json_schema(&JsonValue::Null);
    let mut cases = 0;
    let mut moved_twice = 0;
    let mut differences = std::collections::BTreeMap::<String, usize>::new();
    for line in BufReader::new(output).lines() {
        let row = openui_lang::parse_json(&line.unwrap()).unwrap();
        let scenario = &row["scenario"];
        let mut m = QueryManager::default();
        let store = Store::default();
        let mut program = openui_lang::parse(scenario["program"].as_str().unwrap(), &lib, None);
        let mut calls = Vec::new();
        let mut issued = 0;
        for (i, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
            TRACE.with_borrow_mut(|t| *t = Observation::default());
            let update = match step["kind"].as_str().unwrap() {
                "replace" => {
                    program = openui_lang::parse(step["program"].as_str().unwrap(), &lib, None);
                    if step["program"].as_str().unwrap().contains("hold1") {
                        let reported = program
                            .meta
                            .errors
                            .iter()
                            .any(|e| e.code == openui_lang::ValidationErrorCode::QueryCycle);
                        assert_eq!(
                            reported,
                            scenario["name"].as_str().unwrap().contains("-true-")
                        );
                    }
                    None
                }
                "evaluate" => Some(
                    m.apply(QueryCommand::Evaluate {
                        program: &program,
                        store: &store,
                    })
                    .unwrap(),
                ),
                "complete" => {
                    let addr = &step["dispatch"];
                    let target = if let Some(i) = step["call"].as_u64() {
                        calls.get(i as usize)
                    } else {
                        calls.iter().find(|(a, _)| a == addr)
                    };
                    target.map(|(_, id)| {
                        m.audit_delivery(id, &ToolOutcome::Result(step["outcome"].get("value")))
                            .unwrap();
                        m.apply(QueryCommand::Complete {
                            id,
                            outcome: ToolOutcome::Result(step["outcome"].get("value")),
                        })
                        .unwrap()
                    })
                }
                _ => panic!("unexpected round test command"),
            };
            let changed = update.as_ref().is_some_and(|u| u.changed);
            let mut dispatched = Vec::new();
            if let Some(u) = update {
                m.audit_output(&u.calls).unwrap();
                for call in u.calls {
                    let args = call.arguments_json().unwrap();
                    dispatched.push(format!(
                        r#"{{"call":{issued},"toolName":{},"args":{args}}}"#,
                        JsonValue::from(call.tool_name.clone())
                    ));
                    let mut addr = JsonValue::Object(Default::default());
                    addr["step"] = i.into();
                    addr["toolName"] = call.tool_name.into();
                    addr["args"] = openui_lang::parse_json(&args).unwrap();
                    issued += 1;
                    calls.push((addr, call.id));
                }
            }
            m.audit_cache(calls.iter().map(|(_, id)| id), true).unwrap();
            let actual = format!(
                r#"{{"snapshot":{},"calls":[{}],"changed":{changed}}}"#,
                m.snapshot_json(),
                dispatched.join(",")
            );
            assert_eq!(
                row["expected"][i].as_str(),
                Some(actual.as_str()),
                "{} step {i}",
                scenario["name"]
            );
            let want = &row["byRule"][i];
            let empty = JsonValue::Array(Vec::new());
            TRACE.with_borrow(|t| {
                moved_twice += usize::from(t.key_moves.values().any(|&moves| moves >= 2));
                assert_eq!(
                    JsonValue::from(t.rounds.clone()),
                    *want.get("rounds").unwrap_or(&empty),
                    "{} step {i}",
                    scenario["name"]
                );
                assert_eq!(
                    JsonValue::from(t.left.clone()),
                    *want.get("left").unwrap_or(&empty),
                    "{} step {i}",
                    scenario["name"]
                );
            });
            if row["traces"][i] != *want {
                let reason = row["reasons"][i.to_string()].as_str().unwrap();
                assert!(
                    include_str!("../../../../CONFORMANCE.md").contains(&format!("| `{reason}` |"))
                );
                *differences.entry(reason.into()).or_default() += 1;
            }
            assert!(i < 256);
        }
        cases += 1;
        assert!(cases <= 1236);
    }
    assert!(child.0.wait().unwrap().success());
    assert_eq!(cases, 1236);
    assert_eq!(moved_twice, 1235);
    eprintln!("{moved_twice} commands moved the same reader at least twice");
    assert_eq!(
        differences,
        std::collections::BTreeMap::from([
            ("pending-key-changes-its-only-reader".into(), 1390),
            ("rapid-edits-revisit-a-cached-result".into(), 3),
        ])
    );
    eprintln!(
        "{cases} completion schedules, {differences:?} differences in the two upstream round traces"
    );
}

#[derive(Default, Copy, Clone, Debug)]
struct Cost {
    snapshot_passes: usize,
    reference_passes: usize,
    applied: usize,
    compared: usize,
}
thread_local! { static COST: Cell<Cost> = Cell::new(Cost::default()); }
pub(super) fn snapshot_pass() {
    let c = COST.get();
    COST.set(Cost {
        snapshot_passes: c.snapshot_passes + 1,
        ..c
    });
}
pub(super) fn reference_pass() {
    let c = COST.get();
    COST.set(Cost {
        reference_passes: c.reference_passes + 1,
        ..c
    });
}
pub(super) fn applied() {
    let c = COST.get();
    COST.set(Cost {
        applied: c.applied + 1,
        ..c
    });
}

pub(super) fn compared() {
    let c = COST.get();
    COST.set(Cost {
        compared: c.compared + 1,
        ..c
    });
}

fn fillers(n: usize) -> String {
    (0..n).map(|i| format!("\nq{i}=Query(\"t\")")).collect()
}

#[test]
fn a_cached_cycle_does_not_rescan_unrelated_data_or_reapply_fillers_each_round() {
    let lib = openui_lang::Library::from_json_schema(&JsonValue::Null);
    let suffix = "a1=Query(\"A\",{v:1})\nb0=Query(\"B\",{v:0})\nbig=Query(\"big\")".to_owned()
        + &fillers(2000);
    let initial = openui_lang::parse(
        &format!("a=Query(\"A\",{{v:0}})\nb=Query(\"B\",{{v:1}})\n{suffix}"),
        &lib,
        None,
    );
    let next = openui_lang::parse(
        &format!(
            "a=Query(\"A\",{{v:b}})\nb=Query(\"B\",{{v:a}})\nholdA=Query(\"A\",{{v:0}})\nholdB=Query(\"B\",{{v:1}})\n{suffix}"
        ),
        &lib,
        None,
    );
    let store = Store::default();
    const ROOM: usize = 65536;
    let mut m = QueryManager::with_limits(Limits::DEFAULT.with_result_size(ROOM));
    let initial = m
        .apply(QueryCommand::Evaluate {
            program: &initial,
            store: &store,
        })
        .unwrap();
    for call in initial.calls {
        let value = match call.tool_name.as_str() {
            "A" => JsonValue::from(i32::from(
                call.arguments_json().as_deref() == Some(r#"{"v":0}"#),
            )),
            "B" => JsonValue::from(i32::from(
                call.arguments_json().as_deref() == Some(r#"{"v":1}"#),
            )),
            "big" => JsonValue::Array(vec![JsonValue::from(0); ROOM * 4 / 5 / 2]),
            _ => continue,
        };
        m.apply(QueryCommand::Complete {
            id: &call.id,
            outcome: ToolOutcome::Result(Some(&value)),
        })
        .unwrap();
    }
    COST.set(Cost::default());
    TRACE.with_borrow_mut(|t| *t = Observation::default());
    let start = std::time::Instant::now();
    let update = m
        .apply(QueryCommand::Evaluate {
            program: &next,
            store: &store,
        })
        .unwrap();
    let cost = COST.get();
    eprintln!("cached cycle: {cost:?}, {:?}", start.elapsed());
    assert!(update.reader_rounds_limited);
    TRACE.with_borrow(|t| assert_eq!(t.rounds.len(), next.query_statements.len() + 1));
    assert!(
        cost.snapshot_passes <= 4 && cost.reference_passes <= 2,
        "{cost:?}"
    );
    assert!(cost.applied <= 4 * next.query_statements.len(), "{cost:?}");
    assert!(cost.compared <= 4 * next.query_statements.len(), "{cost:?}");
}

#[test]
fn a_nan_completion_checks_the_exact_round_bound_without_visiting_unrelated_queries() {
    let lib = openui_lang::Library::from_json_schema(&JsonValue::Null);
    let p = openui_lang::parse(
        &("s=Query(\"s\")\nr=Query(\"r\",{x:s})".to_owned() + &fillers(4000)),
        &lib,
        None,
    );
    let mut m = QueryManager::default();
    let initial = m
        .apply(QueryCommand::Evaluate {
            program: &p,
            store: &Store::default(),
        })
        .unwrap();
    let source = initial.calls.iter().find(|c| c.tool_name == "s").unwrap();
    COST.set(Cost::default());
    TRACE.with_borrow_mut(|t| *t = Observation::default());
    let start = std::time::Instant::now();
    let update = m
        .apply(QueryCommand::Complete {
            id: &source.id,
            outcome: ToolOutcome::ResultValue(&Value::Num(f64::NAN)),
        })
        .unwrap();
    let cost = COST.get();
    eprintln!("NaN reader: {cost:?}, {:?}", start.elapsed());
    assert!(update.reader_rounds_limited);
    TRACE.with_borrow(|t| {
        assert_eq!(t.rounds.len(), p.query_statements.len() + 1);
        assert!(t.rounds.iter().all(|r| r == &["r"]));
    });
    assert!(
        cost.snapshot_passes <= 4 && cost.reference_passes <= 1,
        "{cost:?}"
    );
    assert!(cost.applied <= p.query_statements.len() + 1, "{cost:?}");
    assert!(cost.compared <= 3 * p.query_statements.len(), "{cost:?}");
}

#[test]
fn a_refused_drawing_rolls_back_the_reader_cascade_receipts_and_clock() {
    let lib = openui_lang::Library::from_json_schema(&JsonValue::Null);
    let store = Store::default();
    let anchors = "\na1=Query(\"A\",{v:1})\nb0=Query(\"B\",{v:0})";
    let initial = openui_lang::parse(
        &format!(
            "a=Query(\"A\",{{v:0}})\nb=Query(\"B\",{{v:1}}){anchors}\ns=Query(\"s\",{{}},null,1)\nr=Query(\"r\",{{v:s}})"
        ),
        &lib,
        None,
    );
    let cycle = openui_lang::parse(
        &format!(
            "a=Query(\"A\",{{v:b}})\nb=Query(\"B\",{{v:a}}){anchors}\nholdA=Query(\"A\",{{v:0}})\nholdB=Query(\"B\",{{v:1}})\nt=Query(\"new\",{{}},null,1)"
        ),
        &lib,
        None,
    );
    let mut m = QueryManager::default();
    let first = m
        .apply(QueryCommand::Evaluate {
            program: &initial,
            store: &store,
        })
        .unwrap();
    let (source, seeds) = first.calls.split_last().unwrap();
    assert_eq!(source.tool_name, "s");
    for (call, value) in seeds.iter().zip([1, 1, 0, 0]) {
        m.complete_tool_call(
            &call.id,
            ToolCallOutcome::Result(Value::Num(f64::from(value))),
        )
        .unwrap();
    }
    let before = m.snapshot_json().to_owned();
    let due = m.next_refresh_due_ms().unwrap();
    let result = m.registration_transaction(
        &TreeInput::new(&cycle),
        &store,
        true,
        Some(100),
        |candidate| {
            assert!(candidate.reader_rounds_limited);
            assert_eq!(candidate.next_refresh_due_ms(), Some(due + 100));
            Err::<(), _>(QueryCommandError::InvalidClock)
        },
    );
    assert!(matches!(result, Err(QueryCommandError::InvalidClock)));
    assert!(!m.reader_rounds_limited);
    assert_eq!(m.snapshot_json(), before);
    assert_eq!(m.next_refresh_due_ms(), Some(due));
    m.sample_clock(5).unwrap();
    assert!(source.id.is_pending());
    let update = m
        .complete_tool_call(&source.id, ToolCallOutcome::Result(Value::Num(7.0)))
        .unwrap();
    assert_eq!(update.calls.len(), 1);
    assert_eq!(
        update.calls[0].arguments_json().as_deref(),
        Some(r#"{"v":7}"#)
    );
}
