//! Deferred action execution, renderer event shapes, authority and refusals.
use openui_lang::{JsonValue, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    BindingWriteError, JsObject, LibraryFunctions, Limits, Store, TreeInput, Value, evaluate_tree,
    evaluate_tree_with_limits, get_field_value, run_action, run_action_with_limits,
    set_field_value, set_field_value_with_limits, write_binding,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn value(s: &str) -> Value {
    Value::from_json(&parse_json(s).unwrap())
}
fn schema() -> JsonValue {
    parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}},"open_url":{"params":{"type":"object","properties":{}}}}}"#).unwrap()
}
fn init(p: &ParseResult) -> Store {
    let mut s = Store::new();
    assert!(
        s.initialize(
            p.state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            []
        )
        .refused
        .is_empty()
    );
    s
}
fn prop(root: &Value, key: &str) -> Value {
    match root {
        Value::Element(e) => e.props.get(key).unwrap().clone(),
        Value::Obj(o) => {
            if let Some(Value::Obj(props)) = o.get("props") {
                props.get(key).unwrap().clone()
            } else {
                panic!("element")
            }
        }
        _ => panic!("element"),
    }
}
fn plan(p: &ParseResult, l: &Library, s: &Store, j: &JsonValue) -> Value {
    let r = evaluate_tree(
        TreeInput::new(p)
            .with_library(l)
            .with_actions(&j["actions"]),
        s,
    );
    assert!(!r.size_limited && !r.depth_limited);
    prop(&r.root.unwrap(), "a")
}

#[test]
fn kept_sets_read_current_state_and_reset_restores_streaming_pristine_defaults() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse("$n=3\nroot=Box(@Set($n,$n-1),@Reset($n))", &l, None);
    let mut s = init(&p);
    let r = evaluate_tree(TreeInput::new(&p).with_library(&l), &s);
    let root = r.root.unwrap();
    let set = prop(&root, "a");
    let reset = prop(&root, "b");
    for n in [2., 1., 0.] {
        let r = run_action(
            TreeInput::new(&p).with_library(&l),
            &mut s,
            Some(&set),
            "",
            None,
        );
        assert!(r.failure.is_none());
        assert_eq!(s.get("$n"), Some(Value::Num(n)));
        assert!(!s.is_pristine("$n"));
    }
    assert!(
        run_action(TreeInput::new(&p), &mut s, Some(&reset), "", None)
            .failure
            .is_none()
    );
    assert_eq!(s.get("$n"), Some(Value::Num(3.)));
    assert!(s.is_pristine("$n"));
    let next = parse("$n=12\nroot=Box(1)", &l, None);
    let _ = s.initialize(
        next.state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        [],
    );
    assert_eq!(s.get("$n"), Some(Value::Num(12.)));
}

#[test]
fn events_snapshot_each_step_and_unknown_missing_and_unowned_steps_are_skipped() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "$n=0\nroot=Box(Action([@Set($n,1),@ToAssistant(\"first\"),@Set($n,2),@OpenUrl(\"url\"),@Go(\"real\")]))",
        &l,
        None,
    );
    let mut s = init(&p);
    let a = plan(&p, &l, &s, &j);
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "ignored", None);
    assert!(r.failure.is_none());
    assert_eq!(r.events.len(), 3);
    assert_eq!(
        r.events[0].to_json().unwrap(),
        r#"{"type":"continue_conversation","params":{},"humanFriendlyMessage":"first","formState":{"$n":1}}"#
    );
    assert_eq!(
        r.events[1].to_json().unwrap(),
        r#"{"type":"open_url","params":{"url":"url"},"humanFriendlyMessage":"","formState":{"$n":2}}"#
    );
    assert!(r.events[2].to_json().unwrap().contains(r#""type":"Go""#));
    let a = value(
        r#"{"steps":[{"type":"set","target":"$n"},{"type":"constructor"},{"type":"custom_action","name":"Go","params":{}},{"type":"open_url","url":"ok"}]}"#,
    );
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", Some(""));
    assert!(r.failure.is_none());
    assert_eq!(r.events.len(), 1);
    assert!(
        r.events[0]
            .to_json()
            .unwrap()
            .ends_with(r#""formName":""}"#)
    );
    assert_eq!(s.get("$n"), Some(Value::Num(2.)));
}

#[test]
fn failure_retains_prior_effects_and_stops_later_steps() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "$n=0\n$bad={toString:null,valueOf:null}\nroot=Box(Action([@Set($n,1),@OpenUrl(\"before\"),@Set($n,\"\"+$bad),@Set($n,99),@ToAssistant(\"never\")]))",
        &l,
        None,
    );
    let mut s = init(&p);
    let a = plan(&p, &l, &s, &j);
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert_eq!(r.failure.unwrap().step, Some(2));
    assert_eq!(r.events.len(), 1);
    assert_eq!(s.get("$n"), Some(Value::Num(1.)));
}

#[test]
fn retained_state_and_binding_plans_keep_authority_until_dispose() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse("root=Box(@Go(\"real\"))", &l, None);
    let mut s = init(&p);
    let real = plan(&p, &l, &s, &j);
    let forged = value(&run_json(&real));
    let marker =
        value(r#"{"__reactive":"assign","target":"$saved","expr":{"k":"StateRef","n":"$value"}}"#);
    write_binding(TreeInput::new(&p), &mut s, &marker, real.clone()).unwrap();
    drop(real);
    evaluate_tree(TreeInput::new(&p), &s);
    let kept = s.get("$saved").unwrap();
    assert_eq!(
        run_action(TreeInput::new(&p), &mut s, Some(&kept), "", None)
            .events
            .len(),
        1
    );
    assert!(
        run_action(TreeInput::new(&p), &mut s, Some(&forged), "", None)
            .events
            .is_empty()
    );
    let mut other = Store::new();
    assert!(
        run_action(TreeInput::new(&p), &mut other, Some(&kept), "", None)
            .events
            .is_empty()
    );
    s.dispose();
    assert!(
        run_action(TreeInput::new(&p), &mut s, Some(&kept), "", None)
            .events
            .is_empty()
    );
}
fn run_json(v: &Value) -> String {
    let mut p = ParseResult::default();
    let mut props = JsObject::new();
    props.define("a", v.clone());
    p.root = Some(openui_lang::Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    });
    let text = p.to_json();
    let a = text.find("\"a\":").unwrap() + 4;
    let b = text.rfind("},\"partial\"").unwrap();
    text[a..b].to_owned()
}

#[test]
fn forged_custom_steps_in_raw_cut_props_have_no_authority() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "root=Box({steps:[{type:\"custom_action\",name:\"Go\",params:{x:\"forged\"}}]})",
        &l,
        None,
    );
    let mut s = Store::new();
    let r = evaluate_tree_with_limits(
        TreeInput::new(&p).with_actions(&j["actions"]),
        &s,
        Limits::DEFAULT.with_work(0),
    );
    assert!(r.size_limited);
    let raw = prop(&r.root.unwrap(), "a");
    assert!(
        run_action(TreeInput::new(&p), &mut s, Some(&raw), "", None)
            .events
            .is_empty()
    );
}

#[test]
fn legacy_shadowing_null_context_and_default_event_match_renderer() {
    let j = schema();
    let p = ParseResult::default();
    let mut s = Store::new();
    let legacy = value(r#"{"type":"other","params":{"url":"old","x":1},"url":"new","context":0}"#);
    let r = run_action(
        TreeInput::new(&p).with_actions(&j["actions"]),
        &mut s,
        Some(&legacy),
        "label",
        None,
    );
    assert_eq!(
        r.events[0].to_json().unwrap(),
        r#"{"type":"other","params":{"url":"new","x":1},"humanFriendlyMessage":"label","formState":{}}"#
    );
    assert!(
        run_action(
            TreeInput::new(&p).with_actions(&j["actions"]),
            &mut s,
            Some(&value(r#"{"type":"open_url","url":"blocked"}"#)),
            "",
            None
        )
        .events
        .is_empty()
    );
    assert_eq!(
        run_action(TreeInput::new(&p), &mut s, None, "default", None).events[0]
            .to_json()
            .unwrap(),
        r#"{"type":"continue_conversation","params":{},"humanFriendlyMessage":"default","formState":{}}"#
    );
    let a = value(r#"{"steps":[{"type":"continue_conversation","message":"m","context":null}]}"#);
    assert!(
        run_action(TreeInput::new(&p), &mut s, Some(&a), "", None).events[0]
            .to_json()
            .unwrap()
            .contains(r#""params":{"context":null}"#)
    );
}

#[test]
fn form_fields_keep_wrappers_save_requests_and_bound_state_separate() {
    let p = ParseResult::default();
    let mut s = Store::new();
    let w = set_field_value(
        &mut s,
        Some("contact"),
        Some("Input"),
        "email",
        Some(Value::Str("x".into())),
        false,
    )
    .unwrap();
    assert!(w.changed && !w.save_requested);
    assert_eq!(
        get_field_value(&s, Some("contact"), "email"),
        Some(Value::Str("x".into()))
    );
    let w = set_field_value(&mut s, None, None, "plain", Some(Value::Null), true).unwrap();
    assert!(w.save_requested);
    let version = s.version();
    let repeated = set_field_value(&mut s, None, None, "plain", Some(Value::Null), true).unwrap();
    assert!(!repeated.changed && repeated.save_requested);
    assert_eq!(s.version(), version);
    let a = run_action(TreeInput::new(&p), &mut s, None, "submit", Some("contact"));
    assert_eq!(
        a.events[0].to_json().unwrap(),
        r#"{"type":"continue_conversation","params":{},"humanFriendlyMessage":"submit","formState":{"contact":{"email":{"value":"x","componentType":"Input"}}},"formName":"contact"}"#
    );
    let marker =
        value(r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#);
    write_binding(TreeInput::new(&p), &mut s, &marker, Value::Num(7.)).unwrap();
    assert!(s.get("$n").is_some());
    assert_eq!(get_field_value(&s, Some("contact"), "$n"), None);
    set_field_value(
        &mut s,
        Some("__proto__"),
        None,
        "__proto__",
        Some(Value::Bool(true)),
        false,
    )
    .unwrap();
    assert_eq!(
        get_field_value(&s, Some("__proto__"), "__proto__"),
        Some(Value::Bool(true))
    );
}

#[test]
fn cuts_refuse_the_write_stop_the_plan_and_preserve_pristine() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse(
        "$n=0\nroot=Box(Action([@Set($n,$huge),@OpenUrl(\"never\")]))",
        &l,
        None,
    );
    let mut s = init(&p);
    s.set("$huge", Some(Value::Str("x".repeat(100_000))))
        .unwrap();
    let a = plan(&p, &l, &s, &j);
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_result_size(1024),
    );
    assert!(r.size_limited);
    assert!(r.events.is_empty());
    assert!(r.failure.is_some());
    assert_eq!(s.get("$n"), Some(Value::Num(0.)));
    assert!(s.is_pristine("$n"));
    let before = s.snapshot();
    assert!(matches!(
        set_field_value_with_limits(
            &mut s,
            None,
            None,
            "x",
            Some(Value::Str("x".repeat(10000))),
            true,
            Limits::DEFAULT.with_result_size(64)
        ),
        Err(BindingWriteError::Cut(_))
    ));
    assert!(Arc::ptr_eq(&before, &s.snapshot()));
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        Some(&a),
        "",
        None,
        Limits::DEFAULT.with_work(0),
    );
    assert!(r.size_limited);
}

#[test]
fn set_functions_run_at_click_time_and_library_errors_write_null_and_continue() {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Identity":{"params":{"type":"object","properties":{"value":{}}}},"Throw":{"params":{"type":"object","properties":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = count.clone();
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Identity", move |a| {
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(if let Value::Obj(o) = &a {
            o.get("value").cloned()
        } else {
            None
        })
    })
    .unwrap();
    f.register("Throw", |_| Err("failure".into())).unwrap();
    let p = parse(
        "$n=0\nroot=Box(Action([@Set($n,@Identity($n+1)),@Set($bad,@Throw()),@ToAssistant(\"ok\")]))",
        &l,
        None,
    );
    let mut s = init(&p);
    let input = TreeInput::new(&p).with_library(&l).with_functions(&f);
    let a = prop(&evaluate_tree(input.clone(), &s).root.unwrap(), "a");
    assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 0);
    for n in [1., 2.] {
        let r = run_action(input.clone(), &mut s, Some(&a), "", None);
        assert!(r.failure.is_none());
        assert_eq!(r.events.len(), 1);
        assert_eq!(s.get("$n"), Some(Value::Num(n)));
        assert_eq!(s.get("$bad"), Some(Value::Null));
    }
    assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 2);
}

#[test]
fn long_plans_share_work_and_preserve_depth_cuts() {
    let p = ParseResult::default();
    let mut s = Store::with_limits(Limits::DEFAULT.with_depth(20_000));
    s.set("$n", Some(Value::Num(0.))).unwrap();
    let steps=Value::Arr(Arc::new((0..1000).map(|_|value(r#"{"type":"set","target":"$n","valueAST":{"k":"BinOp","op":"+","left":{"k":"StateRef","n":"$n"},"right":{"k":"Num","v":1}}}"#)).collect()));
    let mut o = JsObject::new();
    o.define("steps", steps);
    let plan = Value::Obj(Arc::new(o));
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut s,
        Some(&plan),
        "",
        None,
        Limits::DEFAULT.with_result_size(64).with_work(1000),
    );
    assert!(r.size_limited && r.failure.is_some());
    assert!(matches!(s.get("$n"),Some(Value::Num(n))if n>0. && n<1000.));
    // Deep caller data and malformed step kinds must be refused/ignored safely.
    let mut deep = Value::Null;
    for _ in 0..10000 {
        deep = Value::Arr(Arc::new(vec![deep]));
    }
    let a = value(
        r#"{"steps":[{"type":"set","target":"$n","valueAST":{"k":"StateRef","n":"$deep"}}]}"#,
    );
    s.set("$deep", Some(deep)).unwrap();
    let r = run_action(TreeInput::new(&p), &mut s, Some(&a), "", None);
    assert!(r.depth_limited);
}

#[test]
fn runner_does_not_redraw_or_refresh_query_defaults_between_steps() {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}},"functions":{"Identity":{"params":{"type":"object","properties":{"value":{}}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let mut functions = LibraryFunctions::new(&j["functions"]);
    functions
        .register("Identity", move |v| {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(if let Value::Obj(o) = &v {
                o.get("value").cloned()
            } else {
                None
            })
        })
        .unwrap();
    let p = parse(
        "$n=0\nq=Query(\"q\",{},@Identity($n))\nroot=Box(Action([@Set($n,q+1),@Set($n,q+1)]),@Identity(99))",
        &l,
        None,
    );
    let mut s = init(&p);
    let input = TreeInput::new(&p)
        .with_library(&l)
        .with_functions(&functions);
    let a = prop(&evaluate_tree(input.clone(), &s).root.unwrap(), "a");
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    let r = run_action(input, &mut s, Some(&a), "", None);
    assert!(r.failure.is_none());
    assert_eq!(s.get("$n"), Some(Value::Num(1.)));
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 3);
}

#[test]
fn undefined_form_wrappers_keep_reads_and_default_eligibility_until_disposed() {
    let p = ParseResult::default();
    let mut s = Store::new();
    s.set("x", Some(value(r#"{"componentType":"Input"}"#)))
        .unwrap();
    let w = set_field_value(&mut s, None, Some("Input"), "x", None, false).unwrap();
    assert!(w.changed);
    assert_eq!(get_field_value(&s, None, "x"), None);
    let r = run_action(TreeInput::new(&p), &mut s, None, "", None);
    assert!(
        r.events[0]
            .to_json()
            .unwrap()
            .contains(r#""x":{"componentType":"Input"}"#)
    );
    let wrapper = s.get("x").unwrap();
    s.dispose();
    s.set("x", Some(wrapper)).unwrap();
    assert_eq!(get_field_value(&s, None, "x"), None);
    // A JSON round trip has no own undefined value property, as in JS.
    let raw = value(r#"{"componentType":"Input"}"#);
    s.set("x", Some(raw.clone())).unwrap();
    assert_eq!(get_field_value(&s, None, "x"), Some(raw));
}

#[test]
fn single_set_has_exact_result_and_work_boundaries_and_a_cut_does_not_poison_later_runs() {
    let p = ParseResult::default();
    let action =
        value(r#"{"steps":[{"type":"set","target":"$n","valueAST":{"k":"Str","v":"abcdefgh"}}]}"#);
    let run = |room, work| {
        let mut s = Store::new();
        let r = run_action_with_limits(
            TreeInput::new(&p),
            &mut s,
            Some(&action),
            "",
            None,
            Limits::DEFAULT.with_result_size(room).with_work(work),
        );
        (r, s)
    };
    assert!(run(10, 100000).0.failure.is_none());
    assert!(run(9, 100000).0.size_limited);
    let mut low = 0;
    let mut high = 100000;
    while low < high {
        let mid = low + (high - low) / 2;
        if run(100000, mid).0.failure.is_some() {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    assert!(low > 0);
    assert!(run(100000, low).0.failure.is_none());
    assert!(run(100000, low - 1).0.size_limited);
    let (cut, mut store) = run(9, 100000);
    assert!(cut.size_limited);
    assert_eq!(store.get("$n"), None);
    assert!(
        run_action(TreeInput::new(&p), &mut store, Some(&action), "", None)
            .failure
            .is_none()
    );
    assert_eq!(store.get("$n"), Some(Value::Str("abcdefgh".into())));
}

#[test]
fn malformed_plan_kinds_and_deep_inputs_are_safe_on_a_small_stack() {
    std::thread::Builder::new().name("malformed_plan_kinds_and_deep_inputs_are_safe_on_a_small_stack::worker".into()).stack_size(64*1024).spawn(|| {
        let p=ParseResult::default();let mut store=Store::with_limits(Limits::DEFAULT.with_depth(20_000));
        for input in ["null","false","0","\"\"","true","1","\"x\"","[]","{}",r#"{"steps":null}"#,r#"{"steps":false}"#,r#"{"steps":{}}"#,r#"{"steps":3}"#,r#"{"steps":"x"}"#,r#"{"steps":[null]}"#,r#"{"steps":[false,3,"x",[],{"type":["open_url"]}]}"#] {
            let action=value(input);let _=run_action(TreeInput::new(&p),&mut store,Some(&action),"",None);
        }
        let mut deep=Value::Null;for _ in 0..10000{deep=Value::Arr(Arc::new(vec![deep]));}
        let action=value(r#"{"steps":[{"type":"reset","targets":["$n"]},{"type":"set","target":"$n","valueAST":{"k":"StateRef","n":"$deep"}}]}"#);
        store.set("$deep",Some(deep)).unwrap();let r=run_action(TreeInput::new(&p),&mut store,Some(&action),"",None);assert!(r.depth_limited);assert_eq!(store.get("$n"),Some(Value::Null));assert!(store.is_pristine("$n"));
    }).unwrap().join().unwrap();
}

#[test]
fn defaults_wait_for_streaming_and_respect_null_undefined_and_inherited_fields() {
    let mut s = Store::new();
    for (streaming, expected) in [(true, false), (false, true), (false, false)] {
        let result = openui_runtime::initialize_field_default(
            &mut s,
            Some("form"),
            Some("Input"),
            "x",
            Some(Value::Null),
            streaming,
        )
        .unwrap();
        assert_eq!(result.is_some(), expected);
    }
    assert_eq!(get_field_value(&s, Some("form"), "x"), Some(Value::Null));
    assert!(
        openui_runtime::initialize_field_default(
            &mut s,
            Some("form"),
            None,
            "constructor",
            Some(Value::Num(1.)),
            false
        )
        .unwrap()
        .is_none()
    );
    assert!(
        openui_runtime::initialize_field_default(
            &mut s,
            Some("form"),
            None,
            "__proto__",
            Some(Value::Num(1.)),
            false
        )
        .unwrap()
        .is_none()
    );
    assert!(
        openui_runtime::initialize_field_default(
            &mut s,
            None,
            None,
            "constructor",
            Some(Value::Num(1.)),
            false
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn a_cut_reset_is_atomic_and_completed_resets_reuse_result_room() {
    let mut p = ParseResult::default();
    let mut store = Store::new();
    let mut defaults = JsObject::new();
    let default = value(&format!(r#"{{"text":"{}"}}"#, "x".repeat(1000)));
    let mut targets = Vec::new();
    for i in 0..20 {
        let name = format!("$n{i}");
        defaults.define(&name, default.clone());
        store.set(&name, Some(Value::Num(0.))).unwrap();
        targets.push(Value::Str(name));
    }
    p.state_declarations = defaults;
    let mut reset = JsObject::new();
    reset.define("type", Value::Str("reset".into()));
    reset.define("targets", Value::Arr(Arc::new(targets)));
    let mut plan = JsObject::new();
    plan.define(
        "steps",
        Value::Arr(Arc::new(vec![Value::Obj(Arc::new(reset))])),
    );
    let plan = Value::Obj(Arc::new(plan));
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&plan),
        "",
        None,
        Limits::DEFAULT.with_work(5000).with_result_size(2000),
    );
    assert!(r.size_limited);
    assert_eq!(r.failure.unwrap().step, Some(0));
    let changed = (0..20)
        .filter(|i| store.is_pristine(&format!("$n{i}")))
        .count();
    assert_eq!(changed, 0, "a cut reset commits none of its targets");
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&plan),
        "",
        None,
        Limits::DEFAULT.with_work(100000).with_result_size(2000),
    );
    assert!(
        r.failure.is_none(),
        "room is released between separately admitted writes"
    );
    let mut p = ParseResult::default();
    p.state_declarations
        .define("$n", Value::Str("x".repeat(1000)));
    let plan = value(&format!(
        r#"{{"steps":[{{"type":"reset","targets":[{}]}}]}}"#,
        std::iter::repeat_n("\"$n\"", 20)
            .collect::<Vec<_>>()
            .join(",")
    ));
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(0.))).unwrap();
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&plan),
        "",
        None,
        Limits::DEFAULT.with_work(5000).with_result_size(2000),
    );
    assert!(r.size_limited);
    assert_eq!(r.failure.unwrap().step, Some(0));
    assert!(!store.is_pristine("$n"));
    assert_eq!(store.get("$n"), Some(Value::Num(0.)));
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&plan),
        "",
        None,
        Limits::DEFAULT.with_work(100000).with_result_size(2000),
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
}

#[test]
fn legacy_string_spread_and_scalar_event_fields_cut_before_large_copies() {
    let p = ParseResult::default();
    let mut store = Store::new();
    let mut action = JsObject::new();
    action.define("params", Value::Str("x".repeat(1000000)));
    let action = Value::Obj(Arc::new(action));
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&action),
        "",
        None,
        Limits::DEFAULT.with_result_size(1024),
    );
    assert!(r.size_limited && r.events.is_empty());
    assert!(test_alloc::peak().saturating_sub(before) < 128 * 1024);
    let mut action = JsObject::new();
    action.define("params", Value::Str("x".repeat(1000)));
    let action = Value::Obj(Arc::new(action));
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&action),
        "",
        None,
        Limits::DEFAULT.with_result_size(2000),
    );
    assert!(
        r.size_limited && r.events.is_empty(),
        "spread output can exceed a small input's bound"
    );
    assert!(test_alloc::peak().saturating_sub(before) < 128 * 1024);
    store
        .set("invalid", Some(Value::Str("x".repeat(1000000))))
        .unwrap();
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        None,
        "",
        Some("invalid"),
        Limits::DEFAULT.with_result_size(1024),
    );
    assert!(r.size_limited);
    assert!(test_alloc::peak().saturating_sub(before) < 128 * 1024);
    assert!(
        set_field_value_with_limits(
            &mut store,
            Some("invalid"),
            None,
            "x",
            Some(Value::Num(1.)),
            false,
            Limits::DEFAULT.with_result_size(1024)
        )
        .is_ok()
    );
    assert_eq!(
        get_field_value(&store, Some("invalid"), "x"),
        Some(Value::Num(1.))
    );
    let message = "x".repeat(1000000);
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        None,
        &message,
        None,
        Limits::DEFAULT.with_result_size(1024),
    );
    assert!(r.size_limited);
    assert!(test_alloc::peak().saturating_sub(before) < 128 * 1024);
}

#[test]
fn legacy_shadowing_converts_property_keys_only_when_actions_are_truthy() {
    let p = ParseResult::default();
    let mut store = Store::new();
    let action = value(r#"{"type":{"toString":null,"valueOf":null}}"#);
    assert_eq!(
        run_action(TreeInput::new(&p), &mut store, Some(&action), "", None)
            .events
            .len(),
        1
    );
    for schema in ["{}", "[]", "\"actions\""] {
        let actions = parse_json(schema).unwrap();
        let r = run_action(
            TreeInput::new(&p).with_actions(&actions),
            &mut store,
            Some(&action),
            "",
            None,
        );
        assert!(r.failure.is_some() && r.events.is_empty());
    }
    for (schema, kind) in [
        ("[]", "length"),
        ("[1]", "0"),
        ("\"ab\"", "length"),
        ("\"ab\"", "1"),
    ] {
        let actions = parse_json(schema).unwrap();
        let action = value(&format!(r#"{{"type":"{kind}"}}"#));
        assert!(
            run_action(
                TreeInput::new(&p).with_actions(&actions),
                &mut store,
                Some(&action),
                "",
                None
            )
            .events
            .is_empty()
        );
    }
}

#[test]
fn renderer_legacy_lookup_matrix_matches_the_transcription() {
    let fixtures = parse_json(include_str!(
        "../../../conformance/golden/runtime/renderer-action-lookups.json"
    ))
    .unwrap();
    let cases = fixtures.as_array().unwrap();
    let p = ParseResult::default();
    for (index, case) in cases.iter().enumerate() {
        let action = Value::from_json(&case["action"]);
        let mut store = Store::new();
        store.set("$n", Some(Value::Num(1.))).unwrap();
        set_field_value(
            &mut store,
            Some("contact"),
            Some("Input"),
            "x",
            Some(Value::Num(2.)),
            false,
        )
        .unwrap();
        let mut input = TreeInput::new(&p);
        if let Some(actions) = case.get("actions") {
            input = input.with_actions(actions);
        }
        let r = run_action(
            input,
            &mut store,
            Some(&action),
            "legacy",
            case.get("formName").and_then(JsonValue::as_str),
        );
        assert_eq!(
            r.failure.is_some(),
            case["failed"].as_bool().unwrap(),
            "lookup {index}"
        );
        let actual = Value::Arr(Arc::new(
            r.events.iter().map(|e| e.value().clone()).collect(),
        ));
        assert_eq!(
            run_json(&actual),
            run_json(&Value::from_json(&case["events"])),
            "lookup {index}"
        );
    }
    let line = format!("Renderer action lookups: {} observations.", cases.len());
    assert!(include_str!("../../../CONFORMANCE.md").contains(&line));
}

#[test]
fn aliased_reset_defaults_cannot_bypass_the_per_write_json_bound() {
    let body = value(&format!(r#"{{"text":"{}"}}"#, "x".repeat(1000)));
    let mut p = ParseResult::default();
    p.state_declarations
        .define("$n", Value::Arr(Arc::new(vec![body; 20])));
    let mut store = Store::new();
    store.set_pristine("$n", Some(Value::Num(0.))).unwrap();
    let action =
        value(r#"{"steps":[{"type":"reset","targets":["$n"]},{"type":"open_url","url":"never"}]}"#);
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&action),
        "",
        None,
        Limits::DEFAULT.with_work(100000).with_result_size(2000),
    );
    assert!(r.size_limited && r.events.is_empty());
    assert_eq!(store.get("$n"), Some(Value::Num(0.)));
    assert!(store.is_pristine("$n"));
}

#[test]
fn native_function_reset_defaults_pin_the_existing_value_representation_boundary() {
    let pin = parse_json(include_str!(
        "../../../conformance/divergences/runtime-reset-native-defaults.json"
    ))
    .unwrap();
    let mut p = ParseResult::default();
    let mut props = JsObject::new();
    for name in ["constructor", "toString", "__proto__"] {
        props.define(
            name,
            Value::Ast(Arc::new(openui_lang::Ast::UnaryOp {
                op: "!",
                operand: Arc::new(openui_lang::Ast::StateRef(name.into())),
            })),
        );
    }
    p.root = Some(openui_lang::Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    });
    let action = value(
        r#"{"steps":[{"type":"reset","targets":["constructor","toString","__proto__"]},{"type":"continue_conversation","message":"after"}]}"#,
    );
    let mut store = Store::new();
    let r = run_action(TreeInput::new(&p), &mut store, Some(&action), "", None);
    assert!(r.failure.is_none());
    let mut actual = JsObject::new();
    actual.define(
        "events",
        Value::Arr(Arc::new(
            r.events.iter().map(|e| e.value().clone()).collect(),
        )),
    );
    let mut snapshot = JsObject::new();
    for (k, v) in store.snapshot().iter() {
        if let Some(v) = v {
            snapshot.define(k, v.clone());
        }
    }
    actual.define("snapshot", Value::Obj(Arc::new(snapshot)));
    actual.define(
        "tree",
        value(&evaluate_tree(TreeInput::new(&p), &store).to_json().unwrap()),
    );
    let actual = Value::Obj(Arc::new(actual));
    let reference = Value::from_json(&pin["reference"]);
    let expected = Value::from_json(&pin["rust"]);
    assert_eq!(run_json(&actual), run_json(&expected));
    assert_ne!(run_json(&reference), run_json(&expected));
    let Value::Obj(reference) = &reference else {
        panic!("pin")
    };
    let Value::Obj(expected) = &expected else {
        panic!("pin")
    };
    assert_eq!(reference.get("events"), expected.get("events"));
    assert_eq!(reference.get("snapshot"), expected.get("snapshot"));
    assert!(
        store.is_pristine("constructor")
            && store.is_pristine("toString")
            && store.is_pristine("__proto__")
    );
}

#[test]
fn registry_string_lookups_charge_only_the_units_they_inspect() {
    let p = ParseResult::default();
    let mut store = Store::new();
    let actions = JsonValue::String("x".repeat(100000));
    let limits = Limits::DEFAULT.with_work(500).with_result_size(4096);
    for key in ["length", "0"] {
        let action = value(&format!(r#"{{"type":"{key}"}}"#));
        let r = run_action_with_limits(
            TreeInput::new(&p).with_actions(&actions),
            &mut store,
            Some(&action),
            "",
            None,
            limits,
        );
        assert!(r.failure.is_none() && r.events.is_empty());
    }
    let action = value(r#"{"type":"99999"}"#);
    let r = run_action_with_limits(
        TreeInput::new(&p).with_actions(&actions),
        &mut store,
        Some(&action),
        "",
        None,
        limits,
    );
    assert!(r.size_limited && r.events.is_empty());
    let action = value(r#"{"type":"100000"}"#);
    let r = run_action_with_limits(
        TreeInput::new(&p).with_actions(&actions),
        &mut store,
        Some(&action),
        "",
        None,
        limits,
    );
    assert!(r.failure.is_none());
    assert_eq!(r.events.len(), 1);
}

#[test]
fn form_text_room_uses_exact_json_size_for_supplementary_scalars() {
    let component = "\u{f0000}".repeat(100);
    let bytes = format!(r#"{{"value":null,"componentType":"{component}"}}"#).len();
    let mut store = Store::new();
    assert!(
        set_field_value_with_limits(
            &mut store,
            None,
            Some(&component),
            "x",
            Some(Value::Null),
            false,
            Limits::DEFAULT.with_result_size(bytes)
        )
        .is_ok()
    );
    let mut store = Store::new();
    assert!(matches!(
        set_field_value_with_limits(
            &mut store,
            None,
            Some(&component),
            "x",
            Some(Value::Null),
            false,
            Limits::DEFAULT.with_result_size(bytes - 1)
        ),
        Err(BindingWriteError::Cut(_))
    ));
    assert_eq!(store.get("x"), None);
}

#[test]
fn form_getters_respect_null_prototypes_and_writes_create_ordinary_objects() {
    let j = parse_json(r#"{"$defs":{"Box":{"properties":{"__proto__":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let parsed = parse("root=Box(null)", &l, None);
    let mut fields = parsed.root.as_ref().unwrap().props.clone();
    fields.remove("__proto__");
    assert!(!fields.has("__proto__") && !fields.has("constructor"));
    let mut store = Store::new();
    store
        .set("form", Some(Value::Obj(Arc::new(fields))))
        .unwrap();
    assert_eq!(get_field_value(&store, Some("form"), "__proto__"), None);
    assert!(
        openui_runtime::initialize_field_default(
            &mut store,
            Some("form"),
            None,
            "constructor",
            Some(Value::Num(1.)),
            false
        )
        .unwrap()
        .is_some()
    );
    assert!(
        openui_runtime::initialize_field_default(
            &mut store,
            Some("form"),
            None,
            "toString",
            Some(Value::Num(2.)),
            false
        )
        .unwrap()
        .is_none()
    );
    let mut observation = JsObject::new();
    observation.define("protoBefore", Value::Bool(true));
    observation.define("constructorWritten", Value::Bool(true));
    observation.define("toStringWritten", Value::Bool(false));
    let mut snapshot = JsObject::new();
    for (k, v) in store.snapshot().iter() {
        if let Some(v) = v {
            snapshot.define(k, v.clone());
        }
    }
    observation.define("snapshot", Value::Obj(Arc::new(snapshot)));
    observation.define(
        "protoAfter",
        get_field_value(&store, Some("form"), "__proto__").unwrap(),
    );
    assert_eq!(
        run_json(&Value::Obj(Arc::new(observation))),
        run_json(&value(include_str!(
            "../../../conformance/golden/runtime/form-prototypes.json"
        )))
    );
    let Some(Value::Obj(ref fields)) = store.get("form") else {
        panic!("form")
    };
    assert!(fields.has("toString") && !fields.has_own("toString"));
}

#[test]
fn reset_input_targets_do_not_spend_the_write_result_room() {
    let p = ParseResult::default();
    let action = value(r#"{"steps":[{"type":"reset","targets":["$absent"]}]}"#);
    for (room, accepted) in [(4, true), (3, false)] {
        let mut store = Store::new();
        let r = run_action_with_limits(
            TreeInput::new(&p),
            &mut store,
            Some(&action),
            "",
            None,
            Limits::DEFAULT.with_result_size(room),
        );
        assert_eq!(r.failure.is_none(), accepted);
        assert_eq!(store.get("$absent"), accepted.then_some(Value::Null));
        assert_eq!(store.is_pristine("$absent"), accepted);
    }
}

#[test]
fn retained_loop_expressions_are_measured_as_input_to_the_fresh_run() {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let p = parse("$n=0\nroot=Box(@Each($rows,r,@Set($n,r)))", &l, None);
    let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(1000000));
    assert!(
        store
            .initialize(
                p.state_declarations
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v))),
                []
            )
            .refused
            .is_empty()
    );
    let item = "x".repeat(100000);
    store
        .set(
            "$rows",
            Some(Value::Arr(Arc::new(vec![Value::Str(item.clone())]))),
        )
        .unwrap();
    let root = evaluate_tree(TreeInput::new(&p).with_library(&l), &store)
        .root
        .unwrap();
    let plans = prop(&root, "a");
    let Value::Arr(plans) = &plans else {
        panic!("plans")
    };
    let r = run_action(
        TreeInput::new(&p).with_library(&l),
        &mut store,
        plans.first(),
        "",
        None,
    );
    assert!(r.failure.is_none(), "{:?}", r.failure);
    assert_eq!(store.get("$n"), Some(Value::Str(item)));
}

#[test]
fn mutation_literal_args_are_borrowed_until_their_bounded_evaluation() {
    let mut p = ParseResult::default();
    p.mutation_statements.push(openui_lang::MutationStatement {
        statement_id: "m".into(),
        tool: None,
        args: Some(openui_lang::Ast::Str("x".repeat(1000000))),
    });
    let action = value(r#"{"steps":[{"type":"run","statementId":"m","refType":"mutation"}]}"#);
    let mut store = Store::new();
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let r = run_action_with_limits(
        TreeInput::new(&p),
        &mut store,
        Some(&action),
        "",
        None,
        Limits::DEFAULT.with_work(100).with_result_size(1024),
    );
    assert!(r.size_limited && r.events.is_empty());
    assert!(test_alloc::peak().saturating_sub(before) < 128 * 1024);
}

#[test]
fn retained_typed_and_data_expressions_receive_the_same_input_credit() {
    let definitions =
        parse_json(r#"{"Build":{"params":{"type":"object","properties":{"xs":{}}}}}"#).unwrap();
    let mut functions = LibraryFunctions::new(&definitions);
    functions
        .register("Build", |_| Ok(Some(Value::Str("x".repeat(400000)))))
        .unwrap();
    let expr = Value::Ast(Arc::new(openui_lang::Ast::Comp {
        name: "Build".into(),
        args: Arc::new(vec![openui_lang::Ast::Arr(Arc::new(
            vec![openui_lang::Ast::Num(0.); 1000],
        ))]),
        function: true,
        mapped_props: None,
    }));
    let data = value(&run_json(&expr));
    let p = ParseResult::default();
    for expr in [expr, data] {
        let mut step = JsObject::new();
        step.define("type", Value::Str("set".into()));
        step.define("target", Value::Str("$n".into()));
        step.define("valueAST", expr);
        let mut plan = JsObject::new();
        plan.define(
            "steps",
            Value::Arr(Arc::new(vec![Value::Obj(Arc::new(step))])),
        );
        let plan = Value::Obj(Arc::new(plan));
        let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(1000000));
        let r = run_action(
            TreeInput::new(&p).with_functions(&functions),
            &mut store,
            Some(&plan),
            "",
            None,
        );
        assert!(r.failure.is_none(), "{:?}", r.failure);
        assert!(matches!(store.get("$n"),Some(Value::Str(ref text)) if text.len()==400000));
    }
}
