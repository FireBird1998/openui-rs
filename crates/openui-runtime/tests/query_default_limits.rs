//! Default cuts belong to reads; eager computation still spends shared work.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, evaluate_tree, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
fn library() -> Library {
    Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap())
}
#[test]
fn unread_default_room_and_depth_cuts_are_local_but_work_cuts_are_shared() {
    let l = library();
    for m in [142, 143, 300, 1000, 2000, 3000, 4000] {
        let list = (0..m).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let p = parse(
            &format!("L=[{list}]\nq=Query(\"t\",{{}},@Each(L,i,L))\nroot=Box(1)"),
            &l,
            None,
        );
        let r = evaluate_tree((&p).into(), &Store::new());
        println!(
            "m={m} bytes={} parser={} size={} depth={}",
            list.len(),
            p.size_limited,
            r.size_limited,
            r.depth_limited
        );
        assert!(!r.depth_limited);
        assert!(!r.size_limited);
    }
    for (source, read) in [("root=Box(1)", false), ("root=Box(q)", true)] {
        let p = parse(
            &format!("q=Query(\"t\",{{}},@Each([1,2,3,4,5],i,[1,2,3,4,5]))\n{source}"),
            &l,
            None,
        );
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p),
            &Store::new(),
            Limits::DEFAULT.with_result_size(120),
        );
        assert_eq!(r.size_limited, read);
        assert!(!r.depth_limited);
    }
}
#[test]
fn default_cycles_cut_reads_and_do_not_flag_unread_trees() {
    for declarations in [
        "q=Query(\"t\",{},q)",
        "q=Query(\"t\",{},r)\nr=Query(\"t\",{},q)",
        "q=Query(\"t\",{},r)\nr=Query(\"t\",{},s)\ns=Query(\"t\",{},q)",
    ] {
        for read in [false, true] {
            let p = parse(
                &format!("{declarations}\nroot=Box({})", if read { "q" } else { "1" }),
                &library(),
                None,
            );
            let r = evaluate_tree((&p).into(), &Store::new());
            assert_eq!(r.depth_limited, read, "{}", r.to_json().unwrap());
            assert!(!r.size_limited);
        }
    }
}
#[test]
fn unread_defaults_stop_flagging_only_at_actual_shared_work_exhaustion() {
    let l = library();
    for (m, cut, bytes) in [(2652, false, 12194), (2653, false, 12199)] {
        let list = (0..m).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let source = format!("L=[{list}]\nq=Query(\"t\",{{}},@Each(L,i,L))\nroot=Box(1)");
        assert_eq!(source.len(), bytes);
        let p = parse(&source, &l, None);
        assert!(!p.size_limited && !p.depth_limited);
        let r = evaluate_tree((&p).into(), &Store::new());
        assert_eq!(r.size_limited, cut, "m={m}");
        assert!(!r.depth_limited);
        let fixed = evaluate_tree_with_limits(
            (&p).into(),
            &Store::new(),
            Limits::DEFAULT.with_work(1_000_000),
        );
        assert!(
            fixed.size_limited && !fixed.depth_limited,
            "one allowance remains shared"
        );
        let r = evaluate_tree_with_limits(
            (&p).into(),
            &Store::new(),
            Limits::DEFAULT.with_work(128_000_000),
        );
        assert!(
            !r.size_limited && !r.depth_limited,
            "the default's room cut remains local: m={m}"
        );
    }
}
#[test]
fn cycle_divergences_pin_reference_throws_rust_output_and_flags() {
    for reason in ["self", "pair", "long", "unread"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../conformance/divergences/runtime-query-default-cycle-{reason}.json"
        ));
        let pin = parse_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let p = parse(pin["program"].as_str().unwrap(), &library(), None);
        for scenario in ["defaults", "persisted"] {
            let r = evaluate_tree((&p).into(), &Store::new());
            let actual = format!(
                "{},\"snapshot\":{{}}}}",
                r.to_json().unwrap().strip_suffix('}').unwrap()
            );
            assert_eq!(
                actual,
                pin["rust"][scenario].to_string(),
                "{reason}/{scenario}"
            );
            assert_eq!(
                r.depth_limited,
                pin["flags"]["depth_limited"].as_bool().unwrap()
            );
            assert_eq!(
                r.size_limited,
                pin["flags"]["size_limited"].as_bool().unwrap()
            );
            assert_eq!(
                pin["reference"][scenario]["throw"]["name"].as_str(),
                Some("RangeError")
            );
            assert_eq!(
                pin["reference"][scenario]["throw"]["message"].as_str(),
                Some("Maximum call stack size exceeded.")
            );
        }
    }
}
#[test]
fn action_defaults_keep_local_cuts_lazy_and_raise_them_on_reads() {
    use openui_runtime::{Value, run_action_with_limits};
    let l = library();
    for cycle in [false, true] {
        for read in [false, true] {
            let default = if cycle {
                "q".to_owned()
            } else {
                format!(
                    "@Each([{}],i,[{}])",
                    (0..100)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    (0..100)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            let source = format!(
                "q=Query(\"t\",{{}},{default})\nroot=Box(Action([@Set($out,1){}]))",
                if read { ",@Set($out,q)" } else { "" }
            );
            let p = parse(&source, &l, None);
            let draw = evaluate_tree((&p).into(), &Store::new());
            assert!(!draw.size_limited && !draw.depth_limited);
            let Some(Value::Element(root)) = &draw.root else {
                panic!("root")
            };
            let mut store = Store::new();
            let run = run_action_with_limits(
                TreeInput::new(&p),
                &mut store,
                root.props.get("a"),
                "",
                None,
                Limits::DEFAULT.with_result_size(512),
            );
            assert_eq!(run.size_limited, read && !cycle);
            assert_eq!(run.depth_limited, read && cycle);
            assert_eq!(store.get("$out"), Some(Value::Num(1.)));
        }
    }
}
