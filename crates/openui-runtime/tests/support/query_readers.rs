use super::*;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

pub(super) fn check_identities(
    manager: &QueryManager,
    foreign: &QueryManager,
    calls: &[ToolCallId],
    outstanding: &BTreeMap<usize, ToolCallId>,
) {
    for (i, id) in calls.iter().enumerate() {
        assert_eq!(manager.check_call(id).is_ok(), outstanding.contains_key(&i));
        assert!(matches!(
            foreign.check_call(id),
            Err(openui_runtime::QueryCommandError::InvalidCall)
        ));
    }
}

#[test]
fn waiting_chains_refetches_and_cycles_match_the_reference_manager() {
    let cases = parse_json(include_str!("../../../../conformance/query-readers.json")).unwrap();
    let goldens = parse_json(include_str!(
        "../../../../conformance/golden/query-readers.json"
    ))
    .unwrap();
    let pins: BTreeMap<_, _> = std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/divergences"
    ))
    .unwrap()
    .map(Result::unwrap)
    .filter(|f| f.file_name().to_string_lossy().starts_with("readers-"))
    .map(|f| {
        let pin = parse_json(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
        let name = pin["case"].as_str().unwrap().to_owned();
        assert_eq!(pin["reference"], goldens[&name]);
        assert_ne!(pin["rust"], pin["reference"]);
        let situation = pin["situation"].as_str().unwrap_or(&name);
        assert!(include_str!("../../../../CONFORMANCE.md").contains(&format!("| `{situation}` |")));
        (name, pin)
    })
    .collect();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), goldens.as_object().unwrap().len());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let expected = pins.get(name).map_or(&goldens[name], |pin| &pin["rust"]);
        assert_eq!(
            case["steps"].as_array().unwrap().len(),
            expected.as_array().unwrap().len()
        );
        replay(case, |i, actual, _| {
            assert_eq!(
                Some(actual.as_str()),
                expected[i].as_str(),
                "{name}, step {i}"
            )
        });
    }
}

#[test]
#[ignore = "differential replay requires the cached Bun reference"]
fn seeded_reader_graphs_match_upstream_at_each_completion_and_edit() {
    let mut reference = Command::new("bun")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference"))
        .args(["--no-install", "query-scenarios.ts", "--random-readers"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut final_rows = BTreeMap::new();
    let mut final_cases = 0;
    let mut cases = 0;
    let mut steps = 0;
    let mut differing_cases = 0;
    let mut differences = BTreeMap::new();
    for line in BufReader::new(reference.stdout.take().unwrap()).lines() {
        let row = parse_json(&line.unwrap()).unwrap();
        let expected = row["byEntry"].as_array().unwrap();
        assert_eq!(
            expected.len(),
            row["scenario"]["steps"].as_array().unwrap().len()
        );
        let upstream = row["reference"].as_array().unwrap();
        assert_eq!(upstream.len(), expected.len());
        differing_cases += usize::from(upstream != expected);
        final_cases += usize::from(upstream != row["final"].as_array().unwrap());
        replay(&row["scenario"], |i, actual, _| {
            let reason = &row["reasons"][i.to_string()];
            if expected[i] == upstream[i] {
                assert!(reason.is_null());
            } else {
                let mut corrected = parse_json(expected[i].as_str().unwrap()).unwrap();
                let original = parse_json(upstream[i].as_str().unwrap()).unwrap();
                for field in ["__openui_loading", "__openui_refetching"] {
                    let before = original["snapshot"][field].as_array().unwrap();
                    let after = corrected["snapshot"][field].as_array().unwrap();
                    assert!(after.iter().all(|id| before.contains(id)));
                    corrected["snapshot"][field] = original["snapshot"][field].clone();
                }
                corrected["changed"] = original["changed"].clone();
                assert_eq!(
                    original, corrected,
                    "a loading attribution changed data or dispatch"
                );
                *differences
                    .entry(reason.as_str().unwrap().to_owned())
                    .or_insert(0usize) += 1;
            }

            let dispatch = row["final"][i].as_str().unwrap();
            if Some(dispatch) != upstream[i].as_str() {
                let cause = if Some(dispatch) == expected[i].as_str() {
                    reason.as_str().unwrap()
                } else {
                    "a-reader-only-dispatches-keys-still-current-after-the-cascade"
                };
                *final_rows.entry(cause.to_owned()).or_insert(0usize) += 1;
            }
            assert_eq!(actual, dispatch, "step {i}: {}", row["scenario"]);
        });
        cases += 1;
        steps += expected.len();
    }
    assert!(reference.wait().unwrap().success());
    assert_eq!((cases, steps, differing_cases), (512, 16_111, 60));
    assert_eq!(
        differences,
        BTreeMap::from([
            ("a-waiting-reader-leaves-its-own-pending-key".into(), 284),
            ("return-to-finished-key".into(), 3),
        ])
    );
    assert_eq!(final_cases, 134);
    assert_eq!(
        final_rows,
        BTreeMap::from([
            ("a-waiting-reader-leaves-its-own-pending-key".into(), 278),
            ("return-to-finished-key".into(), 3),
            (
                "a-reader-only-dispatches-keys-still-current-after-the-cascade".into(),
                476
            ),
        ])
    );
    eprintln!("with final dispatch: {final_rows:?}");
    eprintln!("{cases} reader graphs, {steps} observations");
}

#[derive(Default)]
struct GraphCounts {
    rows: BTreeMap<String, usize>,
    cases: usize,
    total: usize,
    affected: usize,
    extra_calls: u64,
    previous_key_wakes: usize,
}

fn replay_graphs(command: &str, limit: usize) -> GraphCounts {
    let mut child = Command::new("bun")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference"))
        .args(["--no-install", "query-scenarios.ts", command])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stats = GraphCounts::default();
    let output = child.stdout.take().unwrap();
    struct Reap(std::process::Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Reap(child);
    for line in BufReader::new(output).lines() {
        let row = parse_json(&line.unwrap()).unwrap();
        let scenario = &row["scenario"];
        stats.extra_calls += row["extraCalls"].as_u64().unwrap();
        let reference = row["reference"].as_array().unwrap();
        let expected = row["expected"].as_array().unwrap();
        assert_eq!(reference.len(), expected.len());
        assert_eq!(expected.len(), scenario["steps"].as_array().unwrap().len());
        assert!(expected.len() <= 256 && stats.cases < limit);
        replay(scenario, |i, actual, _| {
            assert_eq!(
                Some(actual.as_str()),
                expected[i].as_str(),
                "{} step {i}: {}",
                scenario["name"],
                scenario
            );
            if command == "--previous-self-graphs" && i == 7 {
                let actual = parse_json(&actual).unwrap();
                let calls = actual["calls"].as_array().unwrap();
                assert_eq!(calls.len(), 1);
                assert_eq!(
                    calls[0]["args"]["x"],
                    scenario["steps"][2]["outcome"]["value"]
                );
                stats.previous_key_wakes += 1;
            }
            let reason = row["reasons"].get(i.to_string().as_str());
            if reference[i] != expected[i] {
                let reason = reason.unwrap().as_str().unwrap();
                assert!(
                    include_str!("../../../../CONFORMANCE.md").contains(&format!("| `{reason}` |"))
                );
                *stats.rows.entry(reason.to_owned()).or_default() += 1;
            } else {
                assert!(reason.is_none());
            }
        });
        stats.cases += 1;
        stats.affected += usize::from(reference != expected);
        stats.total += reference.len();
    }
    assert!(child.0.wait().unwrap().success());
    stats
}

#[test]
#[ignore = "differential replay requires the cached Bun reference"]
fn generated_reader_edits_and_cycles_have_only_attributed_differences() {
    let GraphCounts {
        rows: counts,
        cases,
        total,
        affected,
        extra_calls,
        ..
    } = replay_graphs("--reader-graphs", 5000);
    eprintln!(
        "{cases} graphs; {total} observations; {affected} differing graphs; {extra_calls} extra calls: {counts:?}"
    );
    // #119 (a remembered run marks its key stale): the refetches on return
    // to a stale key are 11 more calls seen only on the corrected side, and
    // the driver settles each call seen only on a corrected side, so ten
    // graphs gain 12 completion steps (131,750 and 2,484 before).
    assert_eq!(
        (cases, total, affected, extra_calls),
        (5000, 131762, 2968, 2495)
    );
    // Changes for #119, each confined to the ten graphs above:
    // - a-remembered-run-marks-its-key-stale, new: 24 observations in six
    //   graphs refetch a stale key on return and show loading.
    // - return-to-finished-key, 1,526 to 1,516: in four of those graphs the
    //   query returning to its finished key now refetches it, so the
    //   attribution walk assigns those 10 observations to the later row.
    // - run-through-a-query-that-did-not-start-the-call (3,241 to 3,244),
    //   remove-statement-before-registering-its-replacement (9,382 to
    //   9,385), rapid-edits-revisit-a-cached-result (661 to 662) and
    //   change-flag-after-loading-converges (311 to 312): observations of
    //   the completion steps the driver now appends for the new calls,
    //   which differ for those existing reasons.
    let expected = BTreeMap::from([
        (
            "a-reader-only-dispatches-keys-still-current-after-the-cascade".to_owned(),
            141,
        ),
        ("a-remembered-run-marks-its-key-stale".to_owned(), 24),
        (
            "a-seeded-self-reader-does-not-follow-its-own-result".to_owned(),
            17268,
        ),
        (
            "a-waiting-reader-leaves-its-own-pending-key".to_owned(),
            1986,
        ),
        ("change-flag-after-loading-converges".to_owned(), 312),
        ("handoff-failure-leaves-a-reader-loading".to_owned(), 10),
        ("idle-query-takes-over-a-pending-call".to_owned(), 899),
        ("new-reader-of-a-retained-failure".to_owned(), 207),
        ("pending-key-changes-its-only-reader".to_owned(), 1264),
        ("rapid-edits-revisit-a-cached-result".to_owned(), 662),
        (
            "remove-statement-before-registering-its-replacement".to_owned(),
            9385,
        ),
        ("removed-call-failure-on-a-recreated-key".to_owned(), 7),
        ("removed-call-success-on-a-recreated-key".to_owned(), 31),
        ("return-to-a-displaced-failed-entry".to_owned(), 41),
        ("return-to-an-outstanding-key-after-failure".to_owned(), 1),
        ("return-to-failed-key".to_owned(), 125),
        ("return-to-finished-key".to_owned(), 1516),
        (
            "run-through-a-query-that-did-not-start-the-call".to_owned(),
            3244,
        ),
        ("shared-completion-leaves-a-reader-loading".to_owned(), 43),
        (
            "shared-entry-delivers-to-both-current-queries".to_owned(),
            8692,
        ),
    ]);
    assert_eq!(counts, expected);
    for (name, n) in counts {
        assert!(
            include_str!("../../../../CONFORMANCE.md").contains(&format!("| `{name}` | {n} |"))
        );
    }
}

#[test]
fn reader_delivery_and_primitive_transitions_match_the_pinned_manager() {
    let cases = parse_json(include_str!(
        "../../../../conformance/query-reader-fixes.json"
    ))
    .unwrap();
    let golden = parse_json(include_str!(
        "../../../../conformance/golden/query-reader-fixes.json"
    ))
    .unwrap();
    for scenario in cases.as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let row = &golden[name];
        let bytes = if let Some(reason) = scenario["situation"].as_str() {
            assert!(
                include_str!("../../../../CONFORMANCE.md").contains(&format!("| `{reason}` |"))
            );
            assert_ne!(row["rust"], row["reference"]);
            &row["rust"]
        } else {
            &row["reference"]
        };
        assert_eq!(
            bytes.as_array().unwrap().len(),
            scenario["steps"].as_array().unwrap().len()
        );
        replay(scenario, |i, actual, _| {
            assert_eq!(Some(actual.as_str()), bytes[i].as_str(), "{name}: {i}")
        });
    }
}

#[test]
#[ignore = "differential replay requires the cached Bun reference"]
fn generated_equal_deliveries_to_previous_keys_release_self_readers() {
    let stats = replay_graphs("--previous-self-graphs", 128);
    assert_eq!(
        (
            stats.cases,
            stats.total,
            stats.previous_key_wakes,
            stats.extra_calls
        ),
        (128, 1251, 128, 99)
    );
    assert_eq!(
        stats.rows,
        BTreeMap::from([
            ("a-previous-key-releases-a-waiting-self-reader".into(), 198),
            (
                "a-seeded-self-reader-does-not-follow-its-own-result".into(),
                157
            ),
            ("change-flag-after-loading-converges".into(), 6),
            ("idle-query-takes-over-a-pending-call".into(), 356),
            ("shared-entry-delivers-to-both-current-queries".into(), 128),
        ])
    );
    eprintln!(
        "{} previous-key wakes in {} audited commands; {:?}",
        stats.previous_key_wakes, stats.total, stats.rows
    );
}

fn compare_self_wait(seed: usize, value: JsonValue) {
    let corpus = parse_json(include_str!(
        "../../../../conformance/golden/query-self-wait.json"
    ))
    .unwrap();
    let case = &corpus[seed - 1];
    replay(&case["scenario"], |i, actual, _| {
        assert_eq!(Some(actual.as_str()), case["rust"][i].as_str());
        if i == 7 {
            let observed = parse_json(&actual).unwrap();
            let upstream = parse_json(case["reference"][i].as_str().unwrap()).unwrap();
            assert_eq!(upstream["snapshot"]["q1"], JsonValue::Null);
            assert_eq!(
                upstream["calls"].as_array().unwrap().len(),
                usize::from(value.is_null())
            );
            assert_eq!(observed["calls"].as_array().unwrap().len(), 1);
            assert_eq!(observed["calls"][0]["args"].get("x"), Some(&value));
            assert_eq!(
                observed["snapshot"]["__openui_loading"],
                parse_json(r#"["q1"]"#).unwrap()
            );
        }
    });
}

#[test]
fn an_equal_primitive_on_the_previous_key_dispatches_the_waiting_self_reader() {
    compare_self_wait(1, 1.into());
}

#[test]
fn a_redeclared_self_reader_is_released_by_null_on_its_previous_key() {
    compare_self_wait(2, JsonValue::Null);
}

#[test]
fn a_reader_that_moves_twice_only_dispatches_keys_with_a_final_holder() {
    let cases = parse_json(include_str!(
        "../../../../conformance/query-reader-fixes.json"
    ))
    .unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "a-reader-only-dispatches-keys-still-current-after-the-cascade")
        .unwrap();
    // With and without another current holder for the intermediate key.
    for shared in [false, true] {
        let mut scenario = case.clone();
        let steps = scenario["steps"].as_array_mut().unwrap();
        if shared {
            let program = steps[5]["program"].as_str().unwrap();
            steps[5]["program"] = format!("{program}\nkeep=Query(\"leaf\",{{v:\"one\"}})").into();
        }
        // Revisit the intermediate key in a later command. If the cascade hid
        // its receipt but left it pending, this visit cannot dispatch.
        steps.push(
            parse_json(r#"{"kind":"replace","program":"c=Query(\"leaf\",{v:\"one\"})"}"#).unwrap(),
        );
        steps.push(parse_json(r#"{"kind":"evaluate"}"#).unwrap());
        replay(&scenario, |i, actual, _| {
            if i == 6 || i == 8 {
                let row = parse_json(&actual).unwrap();
                let args: Vec<_> = row["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        assert_eq!(c["toolName"], "leaf");
                        c["args"]["v"].as_str().unwrap()
                    })
                    .collect();
                let expected: &[&str] = match (i, shared) {
                    (6, false) => &["two"],
                    (6, true) => &["one", "two"],
                    (8, false) => &["one"],
                    _ => &[],
                };
                assert_eq!(args, expected);
            }
        });
    }
}
