//! Exact retained-output room and deterministic loop work.
use openui_lang::{Element, Library, ParseResult, parse, parse_json};
use openui_runtime::{JsObject, Limits, Store, TreeInput, Value, evaluate_tree_with_limits};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn library() -> Library {
    Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}}}"#).unwrap(),
    )
}
fn element(props: Vec<Value>) -> Value {
    let mut fields = JsObject::new();
    for (key, value) in ["a", "b", "c"].into_iter().zip(props) {
        fields.define(key, value);
    }
    Value::Element(Arc::new(Element {
        type_name: "Box".into(),
        props: fields,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    }))
}
fn expr(source: &str) -> Value {
    Value::Ast(Arc::new(openui_lang::parse_expression(
        &openui_lang::tokenize(source),
    )))
}
fn shaped_value(value: &Value) -> Value {
    let mut props = JsObject::new();
    props.define("a", value.clone());
    let p = ParseResult {
        root: Some(Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..ParseResult::default()
    };
    Value::from_json(&parse_json(&p.to_json()).unwrap()["root"]["props"]["a"])
}
fn minimum(p: &ParseResult, s: &Store, work: bool) -> usize {
    let (mut lo, mut hi) = (0, 1_000_000_000);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let limits = Limits::DEFAULT
            .with_result_size(if work { 128_000_000 } else { mid })
            .with_work(if work { mid } else { 1_000_000_000 });
        let r = evaluate_tree_with_limits(TreeInput::new(p), s, limits);
        if r.size_limited || r.depth_limited {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    lo
}
#[test]
fn replacements_need_only_the_room_for_their_retained_result() {
    let l = library();
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
    let grow = element(vec![expr("$s+$s")]);
    let reference = element(vec![expr("$s")]);
    let mut child = element(vec![expr(&format!("\"{}\"", "x".repeat(1000)))]);
    if let Value::Element(e) = &mut child {
        Arc::make_mut(e).type_name = "Nested".into();
    }
    let nested = element(vec![child; 3]);
    s.set("$grow", Some(grow.clone())).unwrap();
    s.set("$ref", Some(reference)).unwrap();
    s.set("$el", Some(nested)).unwrap();
    s.set("$list", Some(Value::Arr(Arc::new(vec![grow; 10]))))
        .unwrap();
    // Measured in a separate 729567f build, before loop replacement accounting:
    // grow 42,812; grow-list 44,672; reference 12,172; nested 68,222 bytes.
    // Both native and AST-shaped data have those exact thresholds. The nested
    // fixture uses literal AST props and never regressed: db80c86 was 67,742.
    // The state-reference children regression is pinned separately below.
    for data in [false, true] {
        let mut supplied = Store::new();
        for (key, value) in s.snapshot().iter() {
            supplied
                .set(
                    key,
                    value
                        .as_ref()
                        .map(|v| if data { shaped_value(v) } else { v.clone() }),
                )
                .unwrap();
        }
        for (body, expected) in [
            ("@Each([0,1,2,3,4,5,6,7,8,9],x,$grow)", 23020),
            ("@Each($list,x,x)", 23020),
            ("@Each([0,1,2,3,4,5,6,7,8,9],x,$ref)", 11032),
            ("@Each([0,1,2,3,4,5,6,7,8,9],x,$el)", 33932),
        ] {
            let p = parse(&format!("root=Box({body})"), &l, None);
            let room = minimum(&p, &supplied, false);
            let work = minimum(&p, &supplied, true);
            println!("data={data} {body}: room={room} work={work}");
            assert_eq!(room, expected);
            for (n, cut) in [(room, false), (room - 1, true)] {
                let r = evaluate_tree_with_limits(
                    (&p).into(),
                    &supplied,
                    Limits::DEFAULT.with_result_size(n).with_work(1_000_000_000),
                );
                assert_eq!(r.size_limited, cut);
            }
        }
    }
}
#[test]
fn dynamic_element_work_stays_linear_under_the_default_ceiling() {
    let l = library();
    // Items have a dynamic StateRef prop ($n=1).
    // 729567f: native 37,600,148 / data 43,000,199 at 100,000 items.
    // db80c86: native 70,800,148 / data 76,200,199 (exact work edges).
    for data in [false, true] {
        for n in [100, 200, 100_000] {
            let mut s = Store::with_limits(Limits::DEFAULT.with_result_size(64_000_000));
            s.set("$n", Some(Value::Num(1.))).unwrap();
            s.set(
                "$list",
                Some(Value::Arr(Arc::new(vec![
                    if data {
                        shaped_value(&element(vec![expr("$n")]))
                    } else {
                        element(vec![expr("$n")])
                    };
                    n
                ]))),
            )
            .unwrap();
            let p = parse("root=Box(@Each($list,x,x))", &l, None);
            let expected = if data {
                410usize.saturating_mul(n).saturating_add(199)
            } else {
                358usize.saturating_mul(n).saturating_add(148)
            };
            let work = if n < 1000 {
                minimum(&p, &s, true)
            } else {
                expected
            };
            assert_eq!(work, expected);
            println!("data={data} {n} dynamic elements: {work} work");
            let r = evaluate_tree_with_limits(
                (&p).into(),
                &s,
                Limits::DEFAULT.with_result_size(64_000_000),
            );
            assert!(!r.size_limited && !r.depth_limited);
            assert!(work <= 64_000_000);
            // 100,000 element JSON exceeds the independent default 8 MiB room
            // ceiling. Raise only room; retain the default 64M work cap above.
            for (units, cut) in [(work, false), (work - 1, true)] {
                let r = evaluate_tree_with_limits(
                    (&p).into(),
                    &s,
                    Limits::DEFAULT
                        .with_result_size(64_000_000)
                        .with_work(units),
                );
                assert_eq!(r.size_limited, cut, "n={n} work={units}");
            }
        }
    }
}
#[test]
fn stripped_markers_and_component_wrappers_keep_one_output_reservation() {
    let l = library();
    let mut s = Store::new();
    s.set("$big", Some(Value::Str("x".repeat(20_000)))).unwrap();
    let mut marker = JsObject::new();
    marker.define("__reactive", Value::Str("assign".into()));
    marker.define("target", Value::Str("$big".into()));
    s.set("$el", Some(element(vec![Value::Obj(Arc::new(marker))])))
        .unwrap();
    let rows = (0..100)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    for (body, expected) in [
        (format!("@Each([{rows}],x,$el)"), 2_009_312),
        (format!("true?Box(@Each([{rows}],x,x)):null"), 380),
        (format!("true?Box(@Each([{rows}],x,@Set($n,x))):null"), 7180),
    ] {
        let source = if body.starts_with("true") {
            format!("root={body}")
        } else {
            format!("root=Box({body})")
        };
        let p = parse(&source, &l, None);
        let room = minimum(&p, &s, false);
        assert_eq!(room, expected);
        let r = evaluate_tree_with_limits(
            (&p).into(),
            &s,
            Limits::DEFAULT
                .with_work(1_000_000_000)
                .with_result_size(room),
        );
        let json = r.to_json();
        let bytes = json.rfind(",\"errors\":").unwrap() - 8;
        println!(
            "wrapper={} plans={} room={room} bytes={bytes}",
            body.starts_with("true"),
            body.contains("Set")
        );
        assert!(!r.size_limited && !r.depth_limited);
        assert!(room <= bytes + 256, "room={room} output={bytes}");
        assert!(
            evaluate_tree_with_limits(
                (&p).into(),
                &s,
                Limits::DEFAULT
                    .with_work(1_000_000_000)
                    .with_result_size(room - 1)
            )
            .size_limited
        );
    }
}
#[test]
fn generated_nested_loops_keep_generated_output_inside_their_room() {
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}},"functions":{"Id":{"params":{"properties":{"value":{}}}},"Throw":{"params":{"properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
    functions.register("Id", |args| {
        let Value::Obj(o) = &args else { panic!("args") };
        Ok(o.get("value").cloned())
    });
    functions.register("Throw", |_| Err("failure".repeat(50)));
    let mut s = Store::new();
    s.set("$el", Some(element(vec![expr("$n+1")]))).unwrap();
    s.set("$n", Some(Value::Num(3.))).unwrap();
    s.set("$bad", Some(element(vec![expr("@Throw()")])))
        .unwrap();
    let templates = [
        "@Id(j)",
        "Box(@Id(j))",
        "$bad",
        "j",
        "[j,j]",
        "{n:j}",
        "Box(j)",
        "$el",
        "@Set($n,j)",
        "Box(@Set($n,j))",
        "{__reactive:\"assign\",target:\"$n\"}",
    ];
    let mut completed = 0usize;
    let mut with_errors = 0usize;
    for seed in 0..384 {
        let n = 1 + seed % 5;
        let rows = (0..n).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        let inner = templates[seed % templates.len()];
        let body = format!("@Each([{rows}],i,@Each([{rows}],j,{inner}))");
        let source = if seed % 2 == 0 {
            format!("root=Box({body})")
        } else {
            format!("root=true?Box({body}):null")
        };
        let parsed = parse(&source, &l, None);
        for data in [false, true] {
            let p = if data {
                let mut p = parsed.clone();
                let json = parse_json(&p.to_json()).unwrap();
                if let (Some(root), Value::Obj(props)) =
                    (&mut p.root, &Value::from_json(&json["root"]["props"]))
                {
                    root.props = props.as_ref().clone();
                }
                p
            } else {
                parsed.clone()
            };
            for room in [128, 512, 2048, 8192] {
                let r = evaluate_tree_with_limits(
                    TreeInput::new(&p)
                        .with_library(&l)
                        .with_functions(&functions),
                    &s,
                    Limits::DEFAULT.with_result_size(room),
                );
                if !r.size_limited && !r.depth_limited {
                    completed += 1;
                    with_errors += usize::from(!r.errors.is_empty());
                    let json = r.to_json();
                    let bytes = json.rfind(",\"errors\":").unwrap() - 8;
                    assert!(
                        json.len() <= room + 21,
                        "diagnostics must remain reserved: seed={seed} bytes={} room={room}",
                        json.len()
                    );
                    assert!(
                        bytes <= room,
                        "seed={seed} room={room} bytes={bytes}: {source}"
                    );
                }
            }
        }
    }
    assert!(completed > 100 && with_errors > 0);
    println!(
        "768 generated programs / 3072 configurations: {completed} uncut, {with_errors} uncut with diagnostics"
    );
}
#[test]
fn a_second_element_pass_interprets_a_descriptor_returned_by_the_first() {
    let l = library();
    for data in [false, true] {
        let mut descriptor = JsObject::new();
        descriptor.define("k", Value::Str("Num".into()));
        descriptor.define(
            "v",
            if data {
                shaped_value(&Value::Ast(Arc::new(openui_lang::Ast::Num(7.))))
            } else {
                Value::Ast(Arc::new(openui_lang::Ast::Num(7.)))
            },
        );
        let mut store = Store::new();
        store
            .set(
                "$rows",
                Some(Value::Arr(Arc::new(vec![Value::Obj(Arc::new(descriptor))]))),
            )
            .unwrap();
        let mut p = parse("root=Box(@Each($rows,x,Box(x)))", &l, None);
        if data
            && let Some(root) = &mut p.root
            && let Some(value) = root.props.get("a").cloned()
        {
            root.props.define("a", shaped_value(&value));
        }
        let r = evaluate_tree_with_limits((&p).into(), &store, Limits::DEFAULT);
        assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
        // Pinned against runtime/each-item-descriptors in the upstream harness.
        assert!(
            r.to_json().contains("\"props\":{\"a\":7}"),
            "{}",
            r.to_json()
        );
    }
}

// Archived 729567f: 53,000,165 native / 59,000,234 data work units.
#[test]
fn arithmetic_element_work_fits_below_the_pre_replacement_cost() {
    let l = library();
    for dynamic in [true] {
        for data in [false, true] {
            let build = |n| {
                let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(64_000_000));
                store.set("$n", Some(Value::Num(1.))).unwrap();
                store
                    .set(
                        "$list",
                        Some(Value::Arr(Arc::new(vec![
                            if data {
                                shaped_value(&element(vec![if dynamic {
                                    expr("$n+1")
                                } else {
                                    Value::Num(1.)
                                }]))
                            } else {
                                element(vec![if dynamic {
                                    expr("$n+1")
                                } else {
                                    Value::Num(1.)
                                }])
                            };
                            n
                        ]))),
                    )
                    .unwrap();
                store
            };
            let p = parse("root=Box(@Each($list,x,x))", &l, None);
            let a = minimum(&p, &build(100), true);
            let b = minimum(&p, &build(200), true);
            let per = (b - a) / 100;
            let fixed = a - per * 100;
            assert_eq!((per, fixed), if data { (415, 234) } else { (362, 165) });
            let work = per * 100_000 + fixed;
            println!(
                "dynamic={dynamic} data={data} work100={a} work200={b} per_item={per} fixed={fixed} work100000={work}"
            );
            let store = build(100_000);
            for (units, cut) in [(work, false), (work - 1, true)] {
                let r = evaluate_tree_with_limits(
                    (&p).into(),
                    &store,
                    Limits::DEFAULT
                        .with_result_size(64_000_000)
                        .with_work(units),
                );
                assert_eq!(r.size_limited, cut);
                assert!(!r.depth_limited);
            }
        }
    }
}

#[test]
fn referenced_dynamic_children_need_only_their_retained_output_room() {
    let l = library();
    for data in [false, true] {
        let mut s = Store::new();
        s.set("$s", Some(Value::Str("x".repeat(1000)))).unwrap();
        let mut child = element(vec![expr(&format!("\"{}\"", "x".repeat(1000)))]);
        if let Value::Element(e) = &mut child {
            Arc::make_mut(e).type_name = "Nested".into();
        }
        let parent = element(vec![expr("$kid"); 3]);
        for (name, value) in [("$kid", child), ("$parent", parent)] {
            s.set(name, Some(if data { shaped_value(&value) } else { value }))
                .unwrap();
        }
        let p = parse("root=Box(@Each([0,1,2,3,4,5,6,7,8,9],x,$parent))", &l, None);
        let room = minimum(&p, &s, false);
        // State-reference children are the ticket's regressed case. Separate
        // archive builds measured 729567f=69,032 and db80c86=101,042 bytes.
        println!("referenced children data={data} room={room}");
        assert_eq!(room, 33_936);
        for (room, cut) in [(33_936, false), (33_935, true)] {
            assert_eq!(
                evaluate_tree_with_limits((&p).into(), &s, Limits::DEFAULT.with_result_size(room))
                    .size_limited,
                cut
            );
        }
    }
}
