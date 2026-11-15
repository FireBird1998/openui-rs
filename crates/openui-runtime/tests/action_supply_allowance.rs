//! Supplying current inputs must not spend the work reserved for action steps.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{
    ActionRun, JsObject, Limits, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree,
    resume_action, run_action, run_action_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn library() -> Library {
    Library::from_json_schema(&parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap())
}
fn action(p: &ParseResult, l: &Library) -> Value {
    let tree = evaluate_tree(TreeInput::new(p).with_library(l), &Store::new());
    assert!(!tree.size_limited && !tree.depth_limited);
    let Some(Value::Element(ref e)) = tree.root else {
        panic!("root")
    };
    e.props.get("a").unwrap().clone()
}
fn source(n: usize, sets: bool) -> String {
    format!(
        "$n=0\n{}\nroot=Box(Action([{}]))",
        (0..n)
            .map(|i| format!("m{i}=Mutation(\"t\")"))
            .collect::<Vec<_>>()
            .join("\n"),
        (0..n)
            .map(|i| if sets {
                format!("@Run(m{i}),@Set($n,$n+1)")
            } else {
                format!("@Run(m{i})")
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn complete(
    mut r: ActionRun,
    mut resume: impl FnMut(&openui_runtime::ActionContinuation) -> ActionRun,
) -> usize {
    let mut count = 0;
    while let Some(c) = r.continuation.take() {
        count += 1;
        r = resume(&c);
    }
    assert!(
        r.failure.is_none(),
        "after {count} outcomes: {:?}",
        r.failure
    );
    count
}
#[test]
fn borrowed_and_reparsed_programs_complete_all_mutations() {
    let l = library();
    for n in [1000, 5000] {
        let text = source(n, false);
        let p = parse(&text, &l, None);
        let a = action(&p, &l);
        for mode in 0..4 {
            let arcs = [Arc::new(p.clone()), Arc::new(p.clone())];
            let mut s = Store::new();
            let mut index = 0;
            let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
            let count = complete(r, |c| {
                index += 1;
                let fresh;
                let fresh_arc;
                let input = match mode {
                    0 => TreeInput::new(&p),
                    1 => {
                        fresh = parse(&text, &l, None);
                        TreeInput::new(&fresh)
                    }
                    2 => TreeInput::from_shared_result(&arcs[index % 2]),
                    _ => {
                        fresh_arc = Arc::new(parse(&text, &l, None));
                        TreeInput::from_shared_result(&fresh_arc)
                    }
                };
                resume_action(input, &mut s, c, true).unwrap()
            });
            assert_eq!(count, n, "mode {mode}");
        }
    }
}
#[test]
fn mutation_outcomes_do_not_steal_work_from_later_sets() {
    let l = library();
    let p = parse(&source(1000, true), &l, None);
    let a = action(&p, &l);
    let mut s = Store::new();
    s.set("$n", Some(Value::Num(0.))).unwrap();
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert_eq!(
        complete(r, |c| resume_action(TreeInput::new(&p), &mut s, c, true)
            .unwrap()),
        1000
    );
    assert_eq!(s.get("$n"), Some(Value::Num(1000.)));
}
fn refs(n: usize) -> JsObject<Value> {
    let rows = Value::Arr(Arc::new(
        (0..n)
            .map(|i| {
                let mut o = JsObject::new();
                o.define("id", Value::Num(i as f64));
                Value::Obj(Arc::new(o))
            })
            .collect(),
    ));
    let mut q = JsObject::new();
    q.define("rows", rows);
    let mut out = JsObject::new();
    out.define("q", Value::Obj(Arc::new(q)));
    out
}
#[test]
fn repeated_raw_reference_input_completes_all_mutations() {
    let l = library();
    let p = Arc::new(parse(&source(5000, false), &l, None));
    let a = action(&p, &l);
    for rows in [3000, 30000] {
        let values = refs(rows);
        let mut s = Store::new();
        let input = TreeInput::from_shared_result(&p).with_reference_values(&values);
        let r = run_action(input.clone(), &mut s, Some(&a), "", None);
        assert_eq!(
            complete(r, |c| resume_action(input.clone(), &mut s, c, true)
                .unwrap()),
            5000
        );
    }
}
fn live() -> usize {
    test_alloc::reset_peak();
    test_alloc::peak()
}
#[test]
fn fetched_reference_values_are_released_between_mutation_outcomes() {
    let l = library();
    let p = Arc::new(parse(&source(5000, false), &l, None));
    let a = action(&p, &l);
    let mut s = Store::new();
    let mut r = run_action(
        TreeInput::from_shared_result(&p),
        &mut s,
        Some(&a),
        "",
        None,
    );
    let mut first = 0;
    let mut maximum = 0;
    for i in 0..5000 {
        let c = r.continuation.take().unwrap();
        drop(r);
        {
            let values = refs(3000);
            r = resume_action(
                TreeInput::from_shared_result(&p).with_reference_values(&values),
                &mut s,
                &c,
                true,
            )
            .unwrap();
        }
        drop(c);
        r.events.clear();
        let now = live();
        maximum = maximum.max(now);
        if i == 0 {
            first = now;
        } else {
            assert!(
                now < first + 256 * 1024,
                "retained supplies: resume {i}: {first} -> {now}"
            );
        }
        assert!(r.failure.is_none(), "{i}: {:?}", r.failure);
    }
    assert!(!r.is_suspended());
    println!(
        "5000 fetched supplies: first retained {first}, maximum {maximum}, growth {}",
        maximum.saturating_sub(first)
    );
}
fn observe(r: &ActionRun, s: &Store) -> String {
    format!(
        "{:?}|{}|{}",
        r.failure,
        r.events
            .iter()
            .map(|e| e.to_json().unwrap())
            .collect::<Vec<_>>()
            .join("\n"),
        match s.get("$n") {
            Some(Value::Str(ref v)) => format!("string({})", v.len()),
            v => format!("{v:?}"),
        }
    )
}
#[test]
fn supplied_input_forms_have_identical_effects_and_cut_points() {
    let l = library();
    let p = Arc::new(parse(&source(30, true), &l, None));
    let a = action(&p, &l);
    let values = refs(3000);
    let prepared = PreparedReferenceValues::new(&values);
    let mut expected = None;
    for shared in [false, true] {
        for cached in [false, true] {
            let input = if shared {
                TreeInput::from_shared_result(&p)
            } else {
                TreeInput::new(&p)
            };
            let input = if cached {
                input.with_prepared_reference_values(&prepared)
            } else {
                input.with_reference_values(&values)
            };
            let mut s = Store::new();
            s.set("$n", Some(Value::Num(0.))).unwrap();
            let mut r = run_action_with_limits(
                input.clone(),
                &mut s,
                Some(&a),
                "",
                None,
                Limits::DEFAULT.with_work(800),
            );
            let mut observations = vec![observe(&r, &s)];
            while let Some(c) = r.continuation.take() {
                r = resume_action(input.clone(), &mut s, &c, true).unwrap();
                observations.push(observe(&r, &s));
            }
            assert!(r.size_limited);
            assert!(
                s.get("$n")
                    .is_some_and(|v| matches!(v,Value::Num(n)if n>0.))
            );
            if let Some(expected) = &expected {
                assert_eq!(
                    expected, &observations,
                    "shared={shared}, prepared={cached}"
                );
            } else {
                expected = Some(observations);
            }
        }
    }
}
fn text_refs(n: usize) -> JsObject<Value> {
    let mut q = JsObject::new();
    q.define("s", Value::Str("x".repeat(n)));
    let mut out = JsObject::new();
    out.define("q", Value::Obj(Arc::new(q)));
    out
}
#[test]
fn one_write_uses_the_largest_supply_not_the_sum_of_supplies() {
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Large":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Large", |_| Ok(Some(Value::Str("x".repeat(5_800_000)))))
        .unwrap();
    let p = Arc::new(parse(
        "q=Query(\"t\",{},{})\nm=Mutation(\"t\")\nroot=Box(Action([@Run(m),@Run(m),@Run(m),@Run(m),@Run(m),@Set($n,@Large())]))",
        &l,
        None,
    ));
    let a = action(&p, &l);
    for sizes in [
        [60000; 6],
        [60000, 61000, 60000, 61000, 60000, 61000],
        [10000, 20000, 30000, 40000, 50000, 60000],
    ] {
        let largest = text_refs(*sizes.iter().max().unwrap());
        let mut fresh_store = Store::new();
        let input = TreeInput::from_shared_result(&p)
            .with_library(&l)
            .with_functions(&functions)
            .with_reference_values(&largest);
        let mut fresh = run_action(input.clone(), &mut fresh_store, Some(&a), "", None);
        while let Some(c) = fresh.continuation.take() {
            fresh = resume_action(input.clone(), &mut fresh_store, &c, true).unwrap();
        }
        assert!(fresh.size_limited, "fresh must refuse the 5.8 MB write");
        let mut s = Store::new();
        let values = text_refs(sizes[0]);
        let mut r = run_action(
            TreeInput::from_shared_result(&p)
                .with_library(&l)
                .with_functions(&functions)
                .with_reference_values(&values),
            &mut s,
            Some(&a),
            "",
            None,
        );
        for n in &sizes[1..] {
            let values = text_refs(*n);
            r = resume_action(
                TreeInput::from_shared_result(&p)
                    .with_library(&l)
                    .with_functions(&functions)
                    .with_reference_values(&values),
                &mut s,
                &r.continuation.take().unwrap(),
                true,
            )
            .unwrap();
        }
        assert_eq!(observe(&fresh, &fresh_store), observe(&r, &s));
    }
}
#[test]
fn mutation_handoffs_preserve_the_work_needed_by_following_filters() {
    let l = library();
    let sets = vec!["@Set($n,$n+@Count(@Filter(q.rows,\"id\",\">=\",0)))"; 4000].join(",");
    let fresh_program = parse(
        &format!(
            "$n=0\nq=Query(\"t\",{{}},{{rows:[]}})\nm=Mutation(\"t\")\nroot=Box(Action([{sets}]))"
        ),
        &l,
        None,
    );
    let fresh_action = action(&fresh_program, &l);
    let values = refs(3000);
    let mut fresh_store = Store::new();
    fresh_store.set("$n", Some(Value::Num(0.))).unwrap();
    let fresh = run_action(
        TreeInput::new(&fresh_program).with_reference_values(&values),
        &mut fresh_store,
        Some(&fresh_action),
        "",
        None,
    );
    assert!(
        fresh.failure.is_none(),
        "fresh filters: {:?}",
        fresh.failure
    );
    let p = parse(
        &format!(
            "$n=0\nq=Query(\"t\",{{}},{{rows:[]}})\nm=Mutation(\"t\")\nroot=Box(Action([{},{}]))",
            vec!["@Run(m)"; 200].join(","),
            sets
        ),
        &l,
        None,
    );
    let a = action(&p, &l);
    let values = refs(3000);
    let mut s = Store::new();
    s.set("$n", Some(Value::Num(0.))).unwrap();
    let input = TreeInput::new(&p).with_reference_values(&values);
    let r = run_action(input.clone(), &mut s, Some(&a), "", None);
    assert_eq!(
        complete(r, |c| resume_action(input.clone(), &mut s, c, true)
            .unwrap()),
        200
    );
    assert_eq!(s.get("$n"), Some(Value::Num(12_000_000.)));
    assert_eq!(s.get("$n"), fresh_store.get("$n"));
}
#[test]
fn prepared_reference_values_take_precedence_at_start_and_resume() {
    let l = library();
    let p = parse(
        "q=Query(\"t\",{},{})\nm=Mutation(\"t\",{n:@Count(q.rows)})\nroot=Box(Action([@Run(m),@Run(m)]))",
        &l,
        None,
    );
    let a = action(&p, &l);
    let small = refs(1);
    let prepared = PreparedReferenceValues::new(&small);
    let unused = refs(30000);
    let mut s = Store::new();
    let mut input = TreeInput::new(&p).with_prepared_reference_values(&prepared);
    input.reference_values = Some(&unused);
    let r = run_action_with_limits(
        input.clone(),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_work(10_000),
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert!(r.events[0].to_json().unwrap().contains(r#""args":{"n":1}"#));
    let r = resume_action(input, &mut s, &r.continuation.unwrap(), true).unwrap();
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert!(r.events[0].to_json().unwrap().contains(r#""args":{"n":1}"#));
}
fn mutation_allocations(n: usize, shared: bool) -> usize {
    let l = library();
    let p = Arc::new(parse(&source(n, false), &l, None));
    let a = action(&p, &l);
    let mut s = Store::new();
    let input = if shared {
        TreeInput::from_shared_result(&p)
    } else {
        TreeInput::new(&p)
    };
    let before = test_alloc::allocation_calls();
    let r = run_action_with_limits(
        input.clone(),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_work(1_000_000),
    );
    assert_eq!(
        complete(r, |c| resume_action(input.clone(), &mut s, c, true)
            .unwrap()),
        n
    );
    test_alloc::allocation_calls() - before
}
#[test]
fn borrowed_program_setup_scales_with_bytes_supplied_and_evaluation_with_steps() {
    let small = mutation_allocations(200, false);
    let large = mutation_allocations(800, false);
    let cached_small = mutation_allocations(200, true);
    let cached_large = mutation_allocations(800, true);
    println!("borrowed 200/800: {small}/{large}; shared: {cached_small}/{cached_large}");
    assert!(
        large < small * 20,
        "supplying four times the program four times as often costs at most quadratic setup"
    );
    assert!(large < 800 * 800 * 20 + 800 * 100 + 4096);
    assert!(cached_large < cached_small * 5);
}
#[test]
fn providing_raw_references_does_not_repeat_preparation() {
    let p = ParseResult::default();
    let values = refs(3000);
    let before = test_alloc::allocation_calls();
    let prepared = PreparedReferenceValues::new(&values);
    let preparation = test_alloc::allocation_calls() - before;
    let mut s = Store::new();
    let before = test_alloc::allocation_calls();
    let r = run_action(
        TreeInput::new(&p).with_prepared_reference_values(&prepared),
        &mut s,
        None,
        "",
        None,
    );
    assert!(r.failure.is_none());
    let cached = test_alloc::allocation_calls() - before;
    let before = test_alloc::allocation_calls();
    let r = run_action(
        TreeInput::new(&p).with_reference_values(&values),
        &mut s,
        None,
        "",
        None,
    );
    assert!(r.failure.is_none());
    let raw = test_alloc::allocation_calls() - before;
    assert!(
        raw.abs_diff(preparation + cached) <= 16,
        "raw input must prepare once: {raw} / cached {cached} + preparation {preparation}"
    );
}
#[test]
fn program_and_references_share_one_combined_supply_floor() {
    let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Large":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&schema);
    let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
    functions
        .register("Large", |_| Ok(Some(Value::Str("x".repeat(5_800_000)))))
        .unwrap();
    let text = "q=Query(\"t\",{},{})\nm=Mutation(\"t\")\nroot=Box(Action([@Run(m),@Run(m),@Set($n,@Large())]))";
    let small = parse(text, &l, None);
    let large = parse(
        &format!("$unused=\"{}\"\n{text}", "z".repeat(60_000)),
        &l,
        None,
    );
    let a = action(&large, &l);
    let short = text_refs(1000);
    let long = text_refs(60_000);
    let mut s = Store::new();
    let r = run_action(
        TreeInput::new(&large)
            .with_library(&l)
            .with_functions(&functions)
            .with_reference_values(&short),
        &mut s,
        Some(&a),
        "",
        None,
    );
    let r = resume_action(
        TreeInput::new(&small)
            .with_library(&l)
            .with_functions(&functions)
            .with_reference_values(&long),
        &mut s,
        &r.continuation.unwrap(),
        true,
    )
    .unwrap();
    let r = resume_action(
        TreeInput::new(&large)
            .with_library(&l)
            .with_functions(&functions)
            .with_reference_values(&short),
        &mut s,
        &r.continuation.unwrap(),
        true,
    )
    .unwrap();
    assert!(r.size_limited);
    assert_eq!(s.get("$n"), None);
}
#[test]
fn default_scaled_limits_give_all_input_forms_the_same_room_and_work_cuts() {
    for work_cut in [false, true] {
        let schema=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Make":{"params":{"type":"object","properties":{"size":{"type":"number"}}}}}}"#).unwrap();
        let l = Library::from_json_schema(&schema);
        let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
        functions
            .register("Make", |args| {
                let Value::Obj(o) = &args else {
                    return Ok(None);
                };
                let Some(Value::Num(n)) = o.get("size") else {
                    return Ok(None);
                };
                Ok(Some(Value::Str("x".repeat(*n as usize))))
            })
            .unwrap();
        let expr = "@Set($n,$n+@Count(@Each(q.rows,r,@Count(@Each(q.rows,x,r.id+x.id)))))";
        let source = if work_cut {
            format!(
                "$n=0\nq=Query(\"t\",{{}},{{}})\nm=Mutation(\"t\")\nroot=Box(Action([{}]))",
                vec![format!("@Run(m),{expr}"); 12].join(",")
            )
        } else {
            "$n=0\nq=Query(\"t\",{},{})\nm=Mutation(\"t\")\nroot=Box(Action([@Set($n,1),@Run(m),@Set($n,@Make(200000)),@Set($n,@Make(2000000))]))".into()
        };
        let p = Arc::new(parse(&source, &l, None));
        let a = action(&p, &l);
        let values = if work_cut { refs(300) } else { text_refs(6000) };
        let prepared = PreparedReferenceValues::new(&values);
        let mut expected = None;
        for shared in [false, true] {
            for cached in [false, true] {
                let input = if shared {
                    TreeInput::from_shared_result(&p)
                } else {
                    TreeInput::new(&p)
                }
                .with_library(&l)
                .with_functions(&functions);
                let input = if cached {
                    input.with_prepared_reference_values(&prepared)
                } else {
                    input.with_reference_values(&values)
                };
                let mut s = Store::new();
                s.set("$n", Some(Value::Num(0.))).unwrap();
                let mut r = run_action(input.clone(), &mut s, Some(&a), "", None);
                let mut observations = vec![observe(&r, &s)];
                while let Some(c) = r.continuation.take() {
                    r = resume_action(input.clone(), &mut s, &c, true).unwrap();
                    observations.push(observe(&r, &s));
                }
                assert!(r.size_limited);
                if work_cut {
                    assert!(matches!(s.get("$n"),Some(Value::Num(n))if n>0.));
                } else {
                    assert_eq!(r.failure.as_ref().unwrap().step, Some(3));
                    assert!(matches!(s.get("$n"),Some(Value::Str(ref s))if s.len()==200000));
                }
                if let Some(expected) = &expected {
                    assert_eq!(
                        expected, &observations,
                        "work={work_cut}, shared={shared}, cached={cached}"
                    )
                } else {
                    expected = Some(observations)
                }
            }
        }
    }
}
