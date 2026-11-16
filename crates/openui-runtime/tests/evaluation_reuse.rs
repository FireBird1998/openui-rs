//! Reusing admitted values preserves first-use work, depth and live memory.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{
    JsObject, LibraryFunctions, Limits, PreparedReferenceValues, Store, TreeInput, Value,
    evaluate_tree_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
fn schema() -> openui_lang::JsonValue {
    parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}},"functions":{"Id":{"params":{"properties":{"value":{}}}},"Len":{"params":{"properties":{"value":{}}}},"Clone":{"params":{"properties":{"value":{}}}},"Grow":{"params":{"properties":{"value":{}}}},"Throw":{"params":{"properties":{}}}}}"#).unwrap()
}
fn functions(j: &openui_lang::JsonValue) -> LibraryFunctions<'_> {
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Id", |v| {
        Ok(match &v {
            Value::Obj(o) => o.get("value").cloned(),
            _ => None,
        })
    })
    .unwrap();
    f.register("Len", |v| {
        Ok(Some(Value::Num(match &v {
            Value::Obj(o) => match o.get("value") {
                Some(Value::Arr(a)) => a.len() as f64,
                Some(Value::Str(s)) => s.len() as f64,
                _ => 0.,
            },
            _ => 0.,
        })))
    })
    .unwrap();
    f.register("Clone", |v| {
        Ok(match &v {
            Value::Obj(o) => o.get("value").map(|v| match v {
                Value::Arr(a) => Value::Arr(Arc::new(a.as_ref().clone())),
                _ => v.clone(),
            }),
            _ => None,
        })
    })
    .unwrap();
    f.register("Grow", |v| {
        Ok(Some(Value::Arr(Arc::new(vec![v.clone(), v]))))
    })
    .unwrap();
    f.register("Throw", |_| Err("boom".into())).unwrap();
    f
}
fn shaped(mut p: ParseResult) -> ParseResult {
    let j = parse_json(&p.to_json()).unwrap();
    if let (Some(root), Value::Obj(o)) = (&mut p.root, &Value::from_json(&j["root"]["props"])) {
        root.props = o.as_ref().clone();
    }
    p
}
fn deep(n: usize, object: bool) -> Value {
    let mut v = Value::Null;
    for _ in 0..n {
        v = if object {
            let mut o = JsObject::new();
            o.define("a", v);
            Value::Obj(Arc::new(o))
        } else {
            Value::Arr(Arc::new(vec![v]))
        };
    }
    v
}
fn nested(mut body: String, n: usize, items: usize, elements: bool) -> String {
    let list = (0..items)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    for i in 0..n {
        body = format!(
            "@Each([{list}],i{i},{})",
            if elements {
                format!("Box({body})")
            } else {
                body
            }
        );
    }
    body
}
fn work(source: &str, store: &Store, refs: &JsObject<Value>) -> usize {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let f = functions(&j);
    let p = parse(source, &l, None);
    let input = TreeInput::new(&p)
        .with_library(&l)
        .with_functions(&f)
        .with_reference_values(refs);
    let (mut lo, mut hi) = (0, 1_000_000_000);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let r = evaluate_tree_with_limits(
            input.clone(),
            store,
            Limits::DEFAULT.with_result_size(128_000_000).with_work(mid),
        );
        if r.size_limited || r.depth_limited {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    lo
}
#[test]
fn admitted_string_arrays_do_not_need_another_body_walk() {
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
    let n = work("root=Box(@Len(@Id([$s,$s,$s,$s])))", &s, &JsObject::new());
    println!("string-array work={n}");
    assert!(
        n <= 10259,
        "already measured string-array work {n} exceeds main 10259"
    );
}
#[test]
fn repeated_reference_uses_have_the_same_depth_boundary_as_the_first() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    for data in [false, true] {
        for object in [false, true] {
            for prepared in [false, true] {
                let mut refs = JsObject::new();
                refs.define("qd", deep(744, object));
                let cached = PreparedReferenceValues::new(&refs);
                let w = nested("[qd]".into(), 12, 1, false);
                for first in ["null".to_string(), w.clone()] {
                    let p = parse(
                        &format!("qd=Query(\"t\",{{}})\nroot=Box([{first},{w}])"),
                        &l,
                        None,
                    );
                    let p = if data { shaped(p) } else { p };
                    let input = TreeInput::new(&p);
                    let input = if prepared {
                        input.with_prepared_reference_values(&cached)
                    } else {
                        input.with_reference_values(&refs)
                    };
                    let r = evaluate_tree_with_limits(input, &Store::new(), Limits::DEFAULT);
                    assert!(
                        !r.depth_limited,
                        "data={data} object={object} prepared={prepared} first={first}"
                    );
                }
            }
        }
    }
}
#[test]
fn nested_element_loops_release_discarded_live_values() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
    // These are bounds for this finite grid and these leaf shapes, not a
    // universal room-to-heap bound. Empty containers have greater bookkeeping
    // overhead per serialized byte than the 1,000-byte string leaf.
    for (leaf, factor) in [
        ("$s", 5),
        ("[[],[],[],[],[],[],[],[]]", 20),
        ("{\"a\":[]}", 20),
    ] {
        let value = if leaf == "$s" {
            Value::Str("x".repeat(1000))
        } else {
            Value::from_json(&parse_json(leaf).unwrap())
        };
        s.set("$leaf", Some(value)).unwrap();
        for items in [1, 3] {
            for levels in 1..=8 {
                for room in [65_536, 1_048_576, 4_100_000, 128_000_000] {
                    for data in [false, true] {
                        s.set(
                            "$rows",
                            Some(Value::Arr(Arc::new(vec![Value::Num(1.); items]))),
                        )
                        .unwrap();
                        let body = nested("$leaf".into(), levels, items, true).replace(
                            &format!(
                                "[{}]",
                                (0..items)
                                    .map(|i| i.to_string())
                                    .collect::<Vec<_>>()
                                    .join(",")
                            ),
                            "$rows",
                        );
                        let p = parse(&format!("root=Box({body})"), &l, None);
                        assert!(!p.size_limited && !p.depth_limited);
                        let p = if data { shaped(p) } else { p };
                        test_alloc::reset_peak();
                        let before = test_alloc::peak();
                        let r = evaluate_tree_with_limits(
                            TreeInput::new(&p),
                            &s,
                            Limits::DEFAULT
                                .with_result_size(room)
                                .with_work(1_000_000_000),
                        );
                        let peak = test_alloc::peak().saturating_sub(before);
                        if items == 1
                            || room == 128_000_000
                            || (items == 3 && levels == 7 && room == 4_100_000)
                        {
                            assert!(
                                !r.size_limited && !r.depth_limited,
                                "new cut: items={items} levels={levels} room={room} data={data}"
                            );
                        }
                        println!(
                            "leaf={leaf} items={items} levels={levels} room={room} data={data} peak={peak} cut={}",
                            r.size_limited
                        );
                        assert!(
                            peak <= room * factor,
                            "leaf={leaf} peak={peak} room={room} factor={factor}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn repeated_large_arguments_fit_the_default_work_allowance() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let mut f = functions(&j);
    f.register("Len", |v| {
        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Value::Obj(o) = &v else { panic!("args") };
        let Some(Value::Arr(a)) = o.get("value") else {
            panic!("array")
        };
        Ok(Some(Value::Num(a.len() as f64)))
    })
    .unwrap();
    let rows = (0..65).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    let source = format!("root=Box(@Each([{rows}],v0,@Len(@Id([$s,$s,$s,$s]))))");
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(100_000)))).unwrap();
    for data in [false, true] {
        calls.store(0, std::sync::atomic::Ordering::Relaxed);
        let p = parse(&source, &l, None);
        let p = if data { shaped(p) } else { p };
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_functions(&f),
            &s,
            Limits::DEFAULT,
        );
        let completed = calls.load(std::sync::atomic::Ordering::Relaxed);
        println!(
            "large arguments data={data} completed={completed} cut={}",
            r.size_limited
        );
        assert!(!r.size_limited && !r.depth_limited);
        assert_eq!(completed, 65);
        let Some(Value::Element(root)) = &r.root else {
            panic!("expected root");
        };
        let Some(Value::Arr(values)) = root.props.get("a") else {
            panic!("expected loop values");
        };
        assert_eq!(values.len(), 65);
        assert!(values.iter().all(|v| v == &Value::Num(4.)));
    }
}

#[test]
fn direct_iterator_optimization_preserves_the_host_argument_representation() {
    let mut j = schema();
    j["functions"]["Inspect"] = j["functions"]["Id"].clone();
    let l = Library::from_json_schema(&j);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Inspect", |v| {
        let mut pending = vec![&v];
        while let Some(v) = pending.pop() {
            match v {
                Value::Ast(_) => panic!("host saw a native AST instead of literal data"),
                Value::Element(_) => panic!("host saw a native element instead of literal data"),
                Value::Arr(a) => pending.extend(a.iter()),
                Value::Obj(o) => pending.extend(o.values()),
                _ => {}
            }
        }
        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Some(Value::Num(1.)))
    })
    .unwrap();
    let (s, _) = crate_supplied();
    for data in [false, true] {
        let p = parse("root=Box(@Inspect(@Each($elements,e,e)))", &l, None);
        let p = if data { shaped(p) } else { p };
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_functions(&f),
            &s,
            Limits::DEFAULT,
        );
        assert!(!r.size_limited && !r.depth_limited);
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
}
fn crate_supplied() -> (Store, JsObject<Value>) {
    let mut s = Store::new();
    s.set("$n", Some(Value::Num(1.))).unwrap();
    let mut props = JsObject::new();
    props.define(
        "a",
        Value::Ast(Arc::new(openui_lang::parse_expression(
            &openui_lang::tokenize("$n+1"),
        ))),
    );
    let e = Value::Element(Arc::new(openui_lang::Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    }));
    s.set("$elements", Some(Value::Arr(Arc::new(vec![e; 3]))))
        .unwrap();
    (s, JsObject::new())
}

#[test]
fn changing_argument_combinations_keep_only_the_current_text_projection() {
    let mut j = schema();
    j["functions"]["Pair"] =
        parse_json(r#"{"params":{"properties":{"text":{},"index":{}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let mut f = functions(&j);
    f.register("Pair", |_| Ok(Some(Value::Num(1.)))).unwrap();
    let mut s = Store::new();
    s.set("$big", Some(Value::Str("x".repeat(500_000))))
        .unwrap();
    s.set(
        "$rows",
        Some(Value::Arr(Arc::new(
            (0..1000).map(|n| Value::Num(n as f64)).collect(),
        ))),
    )
    .unwrap();
    for data in [false, true] {
        // The unused second field changes every call. It deliberately misses
        // the projection cache without constructing a new copy of the source.
        let p = parse("root=Box(@Each($rows,r,@Pair($big,r)))", &l, None);
        let p = if data { shaped(p) } else { p };
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_functions(&f),
            &s,
            Limits::DEFAULT.with_work(4_000_000_000),
        );
        let peak = test_alloc::peak() - before;
        println!("changing argument combinations data={data} peak={peak}");
        assert!(!r.size_limited && !r.depth_limited);
        assert!(peak < 2_100_000, "data={data} peak={peak}");
    }
}
