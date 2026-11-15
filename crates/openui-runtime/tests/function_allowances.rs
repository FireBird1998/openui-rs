//! Function validation and callback values share work, room and depth.
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    LibraryFunctions, Limits, Store, TreeInput, TreeResult, Value, evaluate_tree_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
fn run(p: &ParseResult, l: &Library, f: &LibraryFunctions<'_>, limits: Limits) -> TreeResult {
    evaluate_tree_with_limits(
        TreeInput::new(p).with_library(l).with_functions(f),
        &Store::new(),
        limits,
    )
}
fn threshold(p: &ParseResult, l: &Library, f: &LibraryFunctions<'_>, work: bool) -> usize {
    let (mut low, mut high) = (0, 8_000_000);
    while low < high {
        let mid = low + (high - low) / 2;
        let limits = if work {
            Limits::DEFAULT.with_work(mid).with_result_size(8_000_000)
        } else {
            Limits::DEFAULT.with_work(8_000_000).with_result_size(mid)
        };
        if run(p, l, f, limits).size_limited {
            low = mid + 1
        } else {
            high = mid
        }
    }
    assert!(low < 8_000_000);
    low
}
#[test]
fn replacements_release_intermediates_and_thresholds_fit_exactly() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Make":{"params":{"type":"object","properties":{}}},"Identity":{"params":{"type":"object","properties":{"value":{}}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Make", |_| {
        Ok(Some(Value::Arr(Arc::new(
            (0..100).map(|n| Value::Num(n as f64)).collect(),
        ))))
    });
    f.register("Identity", |args| {
        Ok(match &args {
            Value::Obj(o) => o.get("value").cloned(),
            _ => None,
        })
    });
    let values = (0..100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let main = parse(&format!("root=Box(@Each([{values}],i,i))"), &l, None);
    assert_eq!(threshold(&main, &l, &f, false), 406);
    assert!(405 <= threshold(&main, &l, &f, false));
    for (source, expected_room, expected_work) in [
        ("root=Box(@Make())", 405, 541),
        ("root=Box(@Identity(@Make()))", 405, 733),
        ("root=Box(@Identity(@Identity(@Make())))", 405, 871),
        ("root=Box(@Each([1,2],i,@Make()))", 700, 954),
    ] {
        let p = parse(source, &l, None);
        let room = threshold(&p, &l, &f, false);
        let work = threshold(&p, &l, &f, true);
        assert_eq!((room, work), (expected_room, expected_work), "{source}");
        for (limits, cut) in [
            (Limits::DEFAULT.with_result_size(room), false),
            (Limits::DEFAULT.with_result_size(room - 1), true),
            (Limits::DEFAULT.with_work(work), false),
            (Limits::DEFAULT.with_work(work - 1), true),
        ] {
            let r = run(&p, &l, &f, limits);
            assert_eq!(r.size_limited, cut);
            assert!(!r.depth_limited);
        }
    }
}
#[test]
fn schema_only_work_is_paid_once_and_value_checks_scale_linearly() {
    fn cost(description: usize, rows: usize) -> usize {
        let mut j=parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Probe":{"params":{"type":"object","properties":{"value":{"type":"array","items":{"type":"number"}}}}}}}"#).unwrap();
        j["functions"]["Probe"]["params"]["description"] =
            JsonValue::String("d".repeat(description));
        let l = Library::from_json_schema(&j);
        let mut f = LibraryFunctions::new(&j["functions"]);
        f.register("Probe", |_| Ok(Some(Value::Num(1.0))));
        let xs = (0..rows)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let p = parse(
            &format!("root=Box(@Each([{xs}],i,@Probe([i,2,3])))"),
            &l,
            None,
        );
        threshold(&p, &l, &f, true)
    }
    let a = cost(0, 100);
    let b = cost(0, 200);
    let described = cost(20_000, 100);
    let described_twice = cost(20_000, 200);
    assert!(b <= a * 3);
    assert_eq!(
        described - a,
        described_twice - b,
        "schema text must be charged once"
    );
}
#[test]
fn large_error_messages_and_shared_dags_cut_without_panicking() {
    let j = parse_json(
        r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Host":{"params":{}}}}"#,
    )
    .unwrap();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Host())", &l, None);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Host", |_| Err("x".repeat(200_000)));
    let r = run(&p, &l, &f, Limits::DEFAULT.with_work(1000));
    assert!(r.size_limited);
    assert!(r.errors.is_empty());
    let mut v = Value::Null;
    for _ in 0..40 {
        v = Value::Arr(Arc::new(vec![v.clone(), v]));
    }
    f.register("Host", move |_| Ok(Some(v.clone())));
    let r = run(&p, &l, &f, Limits::DEFAULT);
    assert!(r.size_limited || r.depth_limited);
}

#[test]
fn builtin_steps_ignore_shadowing_functions_in_values_and_costs() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"ToAssistant":{"params":{}},"OpenUrl":{"params":{}},"Set":{"params":{}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let absent = LibraryFunctions::new(&j["functions"]);
    let mut shadowing = LibraryFunctions::new(&j["functions"]);
    for name in ["ToAssistant", "OpenUrl", "Set"] {
        shadowing.register(name, |_| panic!("builtins win"));
    }
    for source in [
        r#"root=Box(@ToAssistant("hi"))"#,
        r#"root=Box(@OpenUrl("url"))"#,
        "root=Box(@Set($n,1))",
    ] {
        let p = parse(source, &l, None);
        for work in [false, true] {
            assert_eq!(
                threshold(&p, &l, &absent, work),
                threshold(&p, &l, &shadowing, work),
                "{source} work={work}"
            );
        }
        assert_eq!(
            run(&p, &l, &absent, Limits::DEFAULT).to_json(),
            run(&p, &l, &shadowing, Limits::DEFAULT).to_json()
        );
    }
}

#[test]
fn wide_undefined_arguments_are_projected_in_one_pass() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Echo", |args| {
        assert!(matches!(&args,Value::Obj(o) if o.is_empty()));
        Ok(Some(args))
    });
    let mut costs = Vec::new();
    for width in [1000, 2000] {
        let props = (0..width)
            .map(|n| format!("key{n}:{{k:\"Str\"}}"))
            .collect::<Vec<_>>()
            .join(",");
        let p = parse(
            &format!("root=Box({{k:\"Comp\",name:\"Echo\",args:[],mappedProps:{{{props}}}}})"),
            &l,
            None,
        );
        let r = run(&p, &l, &f, Limits::DEFAULT);
        assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
        costs.push(threshold(&p, &l, &f, true));
    }
    assert!(costs[1] < costs[0] * 3);
}

#[test]
fn temporary_arguments_with_each_replacements_never_refund_live_output() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Make":{"params":{}},"Identity":{"params":{"properties":{"value":{}}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Make", |_| Ok(Some(Value::Str("x".repeat(1000)))));
    f.register("Identity", |v| {
        Ok(match &v {
            Value::Obj(o) => o.get("value").cloned(),
            _ => None,
        })
    });
    for suffix in [0, 1, 2, 4, 8] {
        let more = ",@Make()".repeat(suffix);
        let p = parse(
            &format!("root=Box([@Identity(@Each([1,2],i,Box(@Make()))){more}])"),
            &l,
            None,
        );
        let room = threshold(&p, &l, &f, false);
        let r = run(&p, &l, &f, Limits::DEFAULT.with_result_size(room));
        let json = parse_json(&r.to_json()).unwrap();
        let bytes = openui_lang::stringify_json(&json["root"]).len();
        assert!(
            bytes <= room,
            "{bytes} output bytes with only {room} room (suffix {suffix})"
        );
    }
}
