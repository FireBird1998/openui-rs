//! Binding markers, field writes, and admission boundaries through public APIs.
use openui_lang::{Element, Library, ParseResult, free_json, parse, parse_json};
use openui_runtime::{
    BindingProps, BindingWriteError, JsObject, Limits, Store, TreeInput, Value, evaluate_tree,
    evaluate_tree_with_limits, write_binding as write_in_context,
    write_binding_with_limits as write_in_context_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn value(s: &str) -> Value {
    let j = parse_json(s).unwrap();
    let v = Value::from_json(&j);
    free_json(j);
    v
}
fn setup(source: &str) -> (ParseResult, Library, BindingProps, Store) {
    let j=parse_json(r#"{"$defs":{"Field":{"properties":{"value":{},"display":{},"children":{},"transform":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&j);
    free_json(j);
    let parsed = parse(source, &library, None);
    let mut bindings = BindingProps::new();
    bindings.insert("Field", "value").unwrap();
    bindings.insert("Field", "transform").unwrap();
    let mut store = Store::new();
    let initialized = store.initialize(
        parsed
            .state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        std::iter::empty(),
    );
    assert!(initialized.refused.is_empty());
    (parsed, library, bindings, store)
}
fn prop<'a>(v: &'a Value, name: &str) -> &'a Value {
    match v {
        Value::Element(e) => e.props.get(name).unwrap(),
        Value::Obj(o) => o.get(name).unwrap(),
        _ => panic!("object"),
    }
}
#[test]
fn markers_preserve_ast_json_and_transform_writes_use_current_state() {
    let (p, l, b, mut store) =
        setup("root=Field($n,$n,null,$total=$value*2+$n)\n$n=3\n$total=0\n$value=7");
    let evaluated = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_binding_props(&b),
        &store,
    );
    assert!(evaluated.to_json().unwrap().contains(
        r#""value":{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#
    ));
    let root = evaluated.root.unwrap();
    assert_eq!(prop(&root, "display"), &Value::Num(3.));
    write_binding(&mut store, prop(&root, "value"), Value::Num(5.)).unwrap();
    write_binding(&mut store, prop(&root, "transform"), Value::Num(4.)).unwrap();
    assert_eq!(store.get("$total"), Some(Value::Num(13.)));
    write_binding(&mut store, prop(&root, "value"), Value::Null).unwrap();
    assert_eq!(
        store.get("$n"),
        Some(Value::Num(7.)),
        "null extra falls back to state"
    );
    let json = evaluated_json(prop(&root, "transform"));
    write_binding(&mut store, &value(&json), Value::Num(2.)).unwrap();
    assert_eq!(
        store.get("$total"),
        Some(Value::Num(11.)),
        "JSON round trip remains writable"
    );
}
fn evaluated_json(v: &Value) -> String {
    let mut props = JsObject::new();
    props.define("value", v.clone());
    let result = ParseResult {
        root: Some(Element {
            type_name: "F".into(),
            props,
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }),
        ..Default::default()
    };
    let s = result.to_json();
    let start = s.find(r#""value":"#).unwrap() + 8;
    let end = s.rfind(r#"},"partial":"#).unwrap();
    s[start..end].into()
}
#[test]
fn typed_and_data_assigns_and_markers_obey_the_same_prop_rules() {
    let (p, l, b, store) = setup(
        r#"root=Field($n=$value*2,{k:"Assign",target:"$n",value:{k:"Num",v:999}},null,{k:"Assign",target:"$n",value:{k:"StateRef",n:"$value"}})
$n=4"#,
    );
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_binding_props(&b),
        &store,
    );
    let root = r.root.unwrap();
    assert_eq!(prop(&root, "display"), &Value::Num(4.));
    assert_eq!(
        prop(prop(&root, "transform"), "target"),
        &Value::Str("$n".into())
    );
    let markers = r#"root=Field([{__reactive:"assign",target:"$n",expr:{k:"Num",v:1}}],[{__reactive:"assign",target:4},{__reactive:"assign",target:"constructor"},{__reactive:"assign",target:"$n"}])
$n=4"#;
    let (p, l, b, store) = setup(markers);
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_binding_props(&b),
        &store,
    );
    assert!(r.to_json().unwrap().contains(r#""display":[null,null,4]"#));
    assert!(
        r.to_json()
            .unwrap()
            .contains(r#""value":[{"__reactive":"assign""#)
    );
}
#[test]
fn refused_writes_leave_snapshot_version_and_pristine_unchanged() {
    let (_, _, _, mut store) = setup("root=Field($n)\n$n=3");
    for marker in [
        r#"null"#,
        r#"{"__reactive":"assign","target":3,"expr":{"k":"Num","v":1}}"#,
        r#"{"__reactive":"assign","target":"$n","expr":{"k":"Arr","els":null}}"#,
    ] {
        let before = store.snapshot();
        let version = store.version();
        assert!(write_binding(&mut store, &value(marker), Value::Num(5.)).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(store.version(), version);
        assert!(store.is_pristine("$n"));
    }
    let marker =
        value(r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#);
    for limits in [
        Limits::DEFAULT.with_work(1),
        Limits::DEFAULT.with_result_size(8),
        Limits::DEFAULT.with_depth(1),
    ] {
        let before = store.snapshot();
        assert!(matches!(
            write_binding_with_limits(
                &mut store,
                &marker,
                Value::Arr(Arc::new(vec![Value::Str("large".repeat(500))])),
                limits
            ),
            Err(BindingWriteError::Cut(_))
        ));
        assert_eq!(store.snapshot(), before);
        assert!(store.is_pristine("$n"));
    }
}
#[test]
fn helper_marks_only_direct_annotations_and_keeps_literal_names() {
    let b = BindingProps::from_prompt_spec(&value(
        r#"{"components":{"constructor":{"signature":"constructor(toString?: $binding<Record<string, boolean>>, __proto__: $binding<any>, nested: {x: $binding<string>}, list: $binding<string>[], boundList: $binding<string[]>, union: $binding<string> | number, boundUnion: $binding<string | number>, quoted: \"$binding<\")"}}}"#,
    )).unwrap();
    assert!(b.contains("constructor", "toString"));
    assert!(b.contains("constructor", "__proto__"));
    assert!(!b.contains("constructor", "list"));
    assert!(b.contains("constructor", "boundList"));
    assert!(!b.contains("constructor", "union"));
    assert!(b.contains("constructor", "boundUnion"));
    assert!(!b.contains("constructor", "nested"));
    assert!(!b.contains("constructor", "x"));
    assert!(!b.contains("constructor", "quoted"));
}
#[test]
fn plain_binding_has_exact_marker_room_and_cuts_one_byte_below() {
    let (p, l, b, store) = setup("root=Field($n)\n$n=3");
    let run = |room| {
        evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_binding_props(&b),
            &store,
            Limits::DEFAULT.with_result_size(room),
        )
    };
    let full = run(4096);
    let root = full.root.as_ref().unwrap();
    let room = evaluated_json(root).len();
    // Serialization helper expects a prop value, so this is exactly the root JSON.
    assert!(!run(room).size_limited, "room={room}");
    let cut = run(room - 1);
    assert!(cut.size_limited);
    assert!(
        matches!(prop(cut.root.as_ref().unwrap(),"value"),Value::Ast(a) if matches!(a.as_ref(),openui_runtime::Ast::StateRef(n) if n=="$n"))
    );
}

#[test]
fn loop_markers_are_charged_once_and_share_one_allowance() {
    let (p, l, b, store) = setup("root=Field(null,null,@Each([1,2],item,Field($n,item)))\n$n=3");
    let run = |room| {
        evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_binding_props(&b),
            &store,
            Limits::DEFAULT.with_result_size(room),
        )
    };
    // Each retained Field and its 74-byte marker are reserved once,
    // including the inline Each pass.
    let threshold = 513;
    assert!(!run(threshold).size_limited);
    assert!(run(threshold - 1).size_limited);
    let cut = evaluate_tree_with_limits(
        TreeInput::new(&p).with_binding_props(&b),
        &store,
        Limits::DEFAULT.with_work(100),
    );
    assert!(cut.size_limited);
    assert!(cut.root.is_some());
}

#[test]
fn writes_handle_undefined_large_values_and_deep_expressions_without_partial_state() {
    let (_, _, _, mut store) = setup("root=Field($n)\n$n=3");
    let undefined =
        value(r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$missing"}}"#);
    write_binding(&mut store, &undefined, Value::Num(1.)).unwrap();
    assert_eq!(store.get("$n"), None);
    let expr = (0..2000).fold(
        Arc::new(openui_runtime::Ast::StateRef("$value".into())),
        |operand, _| Arc::new(openui_runtime::Ast::UnaryOp { op: "+", operand }),
    );
    let mut fields = JsObject::new();
    fields.define("__reactive", Value::Str("assign".into()));
    fields.define("target", Value::Str("$n".into()));
    fields.define("expr", Value::Ast(expr));
    let before = store.snapshot();
    assert!(matches!(
        write_binding(&mut store, &Value::Obj(Arc::new(fields)), Value::Num(4.)),
        Err(BindingWriteError::Cut(_))
    ));
    assert_eq!(before, store.snapshot());
    let marker =
        value(r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#);
    let before = store.snapshot();
    assert!(matches!(
        write_binding_with_limits(
            &mut store,
            &marker,
            Value::Arr(Arc::new(vec![Value::Num(1.); 20000])),
            Limits::DEFAULT.with_result_size(1024).with_work(1000)
        ),
        Err(BindingWriteError::Cut(_))
    ));
    assert_eq!(before, store.snapshot());
}

#[test]
fn inherited_target_names_are_literal_store_keys_and_strings_pass_through() {
    let (p, l, b, mut store) = setup(
        r#"root=Field("name",$n)
$n=3"#,
    );
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_binding_props(&b),
        &store,
    );
    assert_eq!(
        prop(r.root.as_ref().unwrap(), "value"),
        &Value::Str("name".into())
    );
    for name in ["toString", "constructor", "__proto__"] {
        let mut marker = JsObject::new();
        marker.define("__reactive", Value::Str("assign".into()));
        marker.define("target", Value::Str(name.into()));
        marker.define(
            "expr",
            Value::Ast(Arc::new(openui_runtime::Ast::StateRef("$value".into()))),
        );
        write_binding(&mut store, &Value::Obj(Arc::new(marker)), Value::Num(9.)).unwrap();
        assert_eq!(store.get(name), Some(Value::Num(9.)));
        assert!(store.snapshot().has_own(name));
    }
}

#[test]
fn component_binding_lookup_uses_javascript_property_keys() {
    let (p, l, mut b, store) = setup(
        r#"root=Field(null,null,[{k:"Comp",name:1,mappedProps:{value:{k:"StateRef",n:"$n"}}},{k:"Comp",name:null,mappedProps:{value:{k:"StateRef",n:"$n"}}},{k:"Comp",name:{},mappedProps:{value:{k:"StateRef",n:"$n"}}},{k:"Comp",name:[],mappedProps:{value:{k:"StateRef",n:"$n"}}}])
$n=4"#,
    );
    for name in ["1", "null", "[object Object]", ""] {
        b.insert(name, "value").unwrap();
    }
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_binding_props(&b),
        &store,
    );
    assert!(r.errors.is_empty());
    let Value::Arr(items) = prop(r.root.as_ref().unwrap(), "children") else {
        panic!("list")
    };
    for item in items.iter() {
        assert_eq!(
            prop(prop(prop(item, "props"), "value"), "target"),
            &Value::Str("$n".into())
        );
    }
}

#[test]
fn marker_target_reads_keep_map_key_types_and_own_inherited_names() {
    let (p, l, b, mut store) = setup(
        r#"root=Field($n,[{__reactive:"assign",target:4},{__reactive:"assign",target:null},{__reactive:"assign"},{__reactive:"assign",target:{}},{__reactive:"assign",target:[]},{__reactive:"assign",target:false},{__reactive:"assign",target:"constructor"}])
$n=3"#,
    );
    for name in [
        "4",
        "null",
        "undefined",
        "[object Object]",
        "",
        "false",
        "constructor",
    ] {
        store.set(name, Some(Value::Num(99.))).unwrap();
    }
    let r = evaluate_tree(
        TreeInput::new(&p).with_library(&l).with_binding_props(&b),
        &store,
    );
    assert!(
        r.to_json()
            .unwrap()
            .contains(r#""display":[null,null,null,null,null,null,99]"#)
    );
}

fn write_binding(
    store: &mut Store,
    marker: &Value,
    value: Value,
) -> Result<bool, BindingWriteError> {
    write_in_context(
        TreeInput::new(&ParseResult::default()),
        store,
        marker,
        value,
    )
}
fn write_binding_with_limits(
    store: &mut Store,
    marker: &Value,
    value: Value,
    limits: Limits,
) -> Result<bool, BindingWriteError> {
    write_in_context_with_limits(
        TreeInput::new(&ParseResult::default()),
        store,
        marker,
        value,
        limits,
    )
}

#[test]
fn declaration_scope_keeps_extra_reads_separate_and_restores_them() {
    struct Scope;
    impl openui_runtime::EvaluationScope for Scope {
        fn extra_is_object(&self) -> bool {
            true
        }

        fn state(&self, _: &str, _: &openui_runtime::ExpressionEvaluator<'_>) -> Option<Value> {
            Some(Value::Num(2.))
        }
        fn extra(&self, _: &str, _: &openui_runtime::ExpressionEvaluator<'_>) -> Option<Value> {
            Some(Value::Num(3.))
        }
    }
    let node = openui_runtime::Ast::StateRef("$value".into());
    for base_first in [false, true] {
        let session = openui_runtime::EvaluationSession::new(&Scope, 0);
        if base_first {
            assert_eq!(
                session.with_evaluator(|e| e.without_extra(|| e.evaluate(&node))),
                Some(Some(Value::Num(2.)))
            );
        }
        assert_eq!(session.evaluate(&node).value, Some(Value::Num(3.)));
        assert_eq!(
            session.with_evaluator(|e| e.without_extra(|| e.evaluate(&node))),
            Some(Some(Value::Num(2.)))
        );
        assert_eq!(session.evaluate(&node).value, Some(Value::Num(3.)));
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.with_evaluator(|e| e.without_extra(|| panic!("scope restore")))
        }));
        assert!(unwind.is_err());
        assert_eq!(session.evaluate(&node).value, Some(Value::Num(3.)));
    }
    let session = openui_runtime::EvaluationSession::with_limits(
        &Scope,
        0,
        Limits::DEFAULT.with_result_size(2),
    );
    assert!(!session.evaluate(&node).size_limited);
    assert!(
        !session
            .with_evaluator(|e| e.without_extra(|| e.evaluate_result(&node)))
            .unwrap()
            .size_limited
    );
    assert!(session.evaluate(&node).size_limited);
}

#[test]
fn shared_marker_data_pays_for_each_output_position() {
    let (mut p, l, b, mut store) = setup("root=Field(true?$marker:null)\n$n=3");
    let mut marker = JsObject::new();
    marker.define("__reactive", Value::Str("assign".into()));
    marker.define("target", Value::Str("$n".into()));
    marker.define(
        "expr",
        Value::Ast(Arc::new(openui_runtime::Ast::Str("x".repeat(1000)))),
    );
    store
        .set("$marker", Some(Value::Obj(Arc::new(marker))))
        .unwrap();
    p.root
        .as_mut()
        .unwrap()
        .props
        .define("transform", store.get("$marker").unwrap());
    let run = |room| {
        evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_binding_props(&b),
            &store,
            Limits::DEFAULT.with_result_size(room),
        )
    };
    let full = run(8192);
    let room = evaluated_json(full.root.as_ref().unwrap()).len();
    assert!(!run(room).size_limited);
    assert!(
        run(room - 1).size_limited,
        "shared output needs {room} bytes"
    );
}

#[test]
fn assignment_markers_keep_their_loop_projection_reservation() {
    let (p, l, b, store) =
        setup("root=Field(null,null,@Each([1,2],item,Field($n=$value*2)))\n$n=3");
    let run = |room| {
        evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l).with_binding_props(&b),
            &store,
            Limits::DEFAULT.with_result_size(room),
        )
    };
    let threshold = 601;
    assert!(!run(threshold).size_limited);
    assert!(run(threshold - 1).size_limited);
}

#[test]
fn extra_scope_uses_object_keys_before_exact_state_map_keys() {
    let (_, _, _, mut store) = setup("root=Field($n)\n$n=3\n$value=9");
    for name in ["4", "null", "undefined"] {
        store.set(name, Some(Value::Num(99.))).unwrap();
    }
    store.set("constructor", Some(Value::Bool(false))).unwrap();
    let marker =
        value(r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":["$value"]}}"#);
    write_binding(&mut store, &marker, Value::Num(7.)).unwrap();
    assert_eq!(store.get("$n"), Some(Value::Num(7.)));
    write_binding(&mut store, &marker, Value::Null).unwrap();
    assert_eq!(
        store.get("$n"),
        None,
        "null extra falls back with the original array Map key"
    );
    let marker = value(
        r#"{"__reactive":"assign","target":"$n","expr":{"k":"UnaryOp","op":"!","operand":{"k":"StateRef","n":"constructor"}}}"#,
    );
    write_binding(&mut store, &marker, Value::Num(7.)).unwrap();
    assert_eq!(
        store.get("$n"),
        Some(Value::Bool(false)),
        "extra object inherits constructor before Map lookup"
    );
    for expr in [
        r#"{"k":"StateRef","n":4}"#,
        r#"{"k":"StateRef","n":null}"#,
        r#"{"k":"StateRef"}"#,
    ] {
        let marker = value(&format!(
            r#"{{"__reactive":"assign","target":"$n","expr":{expr}}}"#
        ));
        write_binding(&mut store, &marker, Value::Num(7.)).unwrap();
        assert_eq!(store.get("$n"), None);
    }
    let before = store.snapshot();
    let marker = value(
        r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":{"toString":3}}}"#,
    );
    assert!(matches!(
        write_binding(&mut store, &marker, Value::Num(7.)),
        Err(BindingWriteError::Evaluation(
            openui_runtime::EvaluationFailure::CannotConvertToPrimitive
        ))
    ));
    assert_eq!(store.snapshot(), before);
}

#[test]
fn signature_extraction_does_not_copy_the_component_name_per_prop() {
    let name = "C".repeat(100_000);
    let props = 256;
    let signature = format!(
        "{name}({})",
        (0..props)
            .map(|n| format!("p{n}: $binding<string>"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut component = JsObject::new();
    component.define("signature", Value::Str(signature));
    let mut components = JsObject::new();
    components.define(name.clone(), Value::Obj(Arc::new(component)));
    let mut spec = JsObject::new();
    spec.define("components", Value::Obj(Arc::new(components)));
    let spec = Value::Obj(Arc::new(spec));
    let before = test_alloc::allocation_calls();
    let bindings = BindingProps::from_prompt_spec(&spec).unwrap();
    let allocations = test_alloc::allocation_calls() - before;
    assert!(bindings.contains(&name, "p255"));
    assert!(
        allocations <= 2 * props,
        "component name must be owned once: {allocations} allocations for {props} props"
    );
}

#[test]
fn signature_properties_anchor_odd_names_and_unescaped_literals() {
    let spec = value(
        r#"{"components":{"Odd":{"signature":"Odd(e1: \"a\"b\", after: $binding<string>, a:b: $binding<string>, x?: $binding<string>, lt<: $binding<string>, p(: $binding<string>, c,: $binding<string>, it's: $binding<string>, tail: \"end\\\", last: $binding<string>)"}},"schema":{"$defs":{"Odd":{"properties":{"e1":{},"after":{},"a:b":{},"x?":{},"lt<":{},"p(":{},"c,":{},"it's":{},"tail":{},"last":{}}}}}}"#,
    );
    let bindings = BindingProps::from_prompt_spec(&spec).unwrap();
    for name in ["after", "a:b", "x?", "lt<", "p(", "c,", "it's", "last"] {
        assert!(bindings.contains("Odd", name), "missing {name}");
    }
    assert!(!bindings.contains("Odd", "e1"));
    assert!(!bindings.contains("Odd", "tail"));
}

#[test]
fn stripped_markers_fit_the_previous_room_and_work_allowance() {
    let schema = parse_json(
        r#"{"$defs":{"Field":{"properties":{"value":{},"display":{},"children":{},"change":{}}}}}"#,
    )
    .unwrap();
    let l = Library::from_json_schema(&schema);
    free_json(schema);
    let p = parse("root=Field(null,$a,null,$a=$value*2)", &l, None);
    let mut store = Store::new();
    let _ = store.initialize(
        p.state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        std::iter::empty(),
    );
    let run = |room, work| {
        evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&l),
            &store,
            Limits::DEFAULT.with_result_size(room).with_work(work),
        )
    };
    let mut bindings = BindingProps::new();
    bindings.insert("Field", "change").unwrap();
    let bound = |room, work| {
        evaluate_tree_with_limits(
            TreeInput::new(&p)
                .with_library(&l)
                .with_binding_props(&bindings),
            &store,
            Limits::DEFAULT.with_result_size(room).with_work(work),
        )
    };
    assert!(!bound(292, 338).size_limited);
    assert!(bound(291, 338).size_limited);
    assert!(bound(292, 337).size_limited);
    // Independently measured on main: 292 room, 338 work. Projection of the
    // stripped value now needs 165/179, with both exact boundaries enforced.
    assert!(!run(292, 338).size_limited);
    assert!(!run(165, 10000).size_limited);
    assert!(run(164, 10000).size_limited);
    assert!(!run(10000, 178).size_limited);
    assert!(run(10000, 177).size_limited);
}

#[test]
fn undecidable_signature_annotations_do_not_hide_later_props() {
    for first in [
        "$binding<\"a\"b\">",
        "$binding<\"end\\\">",
        "$binding<T",
        "$binding<\"a\"b\"> | $binding<\"c\"d\">",
    ] {
        let signature = format!("Odd(first: {first}, after: $binding<string>)");
        let mut component = JsObject::new();
        component.define("signature", Value::Str(signature));
        let mut components = JsObject::new();
        components.define("Odd", Value::Obj(Arc::new(component)));
        let mut spec =
            match value(r#"{"schema":{"$defs":{"Odd":{"properties":{"first":{},"after":{}}}}}}"#) {
                Value::Obj(ref o) => o.as_ref().clone(),
                _ => panic!("object"),
            };
        spec.define("components", Value::Obj(Arc::new(components)));
        let bindings = BindingProps::from_prompt_spec(&Value::Obj(Arc::new(spec))).unwrap();
        assert!(!bindings.contains("Odd", "first"), "ambiguous {first}");
        assert!(bindings.contains("Odd", "after"));
    }
    let spec = value(
        r#"{"components":{"Odd":{"signature":"Odd(first: \"x, after: $binding<string>\", after: string, last: $binding<string>)"}},"schema":{"$defs":{"Odd":{"properties":{"first":{},"after":{},"last":{}}}}}}"#,
    );
    let bindings = BindingProps::from_prompt_spec(&spec).unwrap();
    assert!(!bindings.contains("Odd", "after"));
    assert!(bindings.contains("Odd", "last"));
    let spec = value(
        r#"{"components":{"Odd":{"signature":"Odd(first: \"x, after: string\", after: $binding<string>, last: $binding<string>)"}},"schema":{"$defs":{"Odd":{"properties":{"first":{},"after":{},"last":{}}}}}}"#,
    );
    let bindings = BindingProps::from_prompt_spec(&spec).unwrap();
    assert!(!bindings.contains("Odd", "after"));
    assert!(bindings.contains("Odd", "last"));
    // Without structural anchors this unescaped signature is undecidable.
    let spec = value(
        r#"{"components":{"Odd":{"signature":"Odd(first: \"a\"b\", after: $binding<string>)"}}}"#,
    );
    let mut bindings = BindingProps::from_prompt_spec(&spec).unwrap();
    assert!(!bindings.contains("Odd", "after"));
    bindings.insert("Odd", "after").unwrap();
    assert!(bindings.contains("Odd", "after"));
}

#[test]
fn repeated_data_markers_fit_the_previous_allowance() {
    let schema = parse_json(
        r#"{"$defs":{"F":{"properties":{"value":{},"display":{},"children":{},"transform":{}}}}}"#,
    )
    .unwrap();
    let library = Library::from_json_schema(&schema);
    free_json(schema);
    let mut parsed = parse("root=F(null)", &library, None);
    let root = parsed.root.as_mut().unwrap();
    root.has_dynamic_props = true;
    root.props = JsObject::new();
    for i in 0..11 {
        root.props.define(
            format!("p{i}"),
            Value::Ast(Arc::new(openui_runtime::Ast::StateRef("$d".into()))),
        );
    }
    let text = format!(
        r#"{{"__reactive":"assign","target":"$missing","expr":{{"k":"Str","v":"{}"}}}}"#,
        "x".repeat(191)
    );
    let json = parse_json(&text).unwrap();
    let marker = Value::from_json(&json);
    free_json(json);
    let mut store = Store::new();
    store.set("$d", Some(marker)).unwrap();
    let run = |room, work| {
        evaluate_tree_with_limits(
            TreeInput::new(&parsed).with_library(&library),
            &store,
            Limits::DEFAULT.with_result_size(room).with_work(work),
        )
    };
    // The same program at db80c86 needed 3030 room and 5946 work.
    assert!(!run(3030, 5946).size_limited);
    assert!(!run(214, 100000).size_limited);
    assert!(run(213, 100000).size_limited);
    assert!(!run(100000, 578).size_limited);
    assert!(run(100000, 577).size_limited);
}

#[test]
fn stripping_does_not_evaluate_the_target_value_again() {
    let (parsed, library, bindings, mut store) = setup("root=Field(null,$e=$value)\n$n=5");
    let element = value(
        r#"{"type":"element","typeName":"Field","props":{"display":{"k":"StateRef","n":"$n"}},"partial":false,"hasDynamicProps":true}"#,
    );
    store.set("$e", Some(element.clone())).unwrap();
    let tree = evaluate_tree(
        TreeInput::new(&parsed)
            .with_library(&library)
            .with_binding_props(&bindings),
        &store,
    );
    assert_eq!(prop(tree.root.as_ref().unwrap(), "display"), &element);
}

#[test]
fn undecided_binding_pairs_are_reported_and_can_be_resolved() {
    let fixture = value(include_str!(
        "../../../conformance/golden/runtime/binding-undecided-spec.json"
    ));
    let spec = prop(&fixture, "spec");
    let mut bindings = BindingProps::from_prompt_spec(spec).unwrap();
    assert_eq!(
        bindings.undecided().collect::<Vec<_>>(),
        vec![
            ("E10", "e10"),
            ("E11", "e11"),
            ("E12", "a, b"),
            ("E12", "b")
        ]
    );
    for component in ["E10", "E11", "E12", "E13"] {
        assert!(bindings.contains(component, "after"));
    }
    assert!(
        bindings.contains("E13", "a, b"),
        "unambiguous delimiter name remains readable"
    );
    let Value::Arr(pairs) = prop(&fixture, "bindings") else {
        panic!("pairs")
    };
    for pair in pairs.iter() {
        let Value::Arr(pair) = pair else {
            panic!("pair")
        };
        let (Value::Str(c), Value::Str(p)) = (&pair[0], &pair[1]) else {
            panic!("names")
        };
        assert!(
            bindings.contains(c, p) || bindings.undecided().any(|(a, b)| a == c && b == p),
            "lost mark {c}.{p}"
        );
    }
    let Value::Obj(defs) = prop(prop(spec, "schema"), "$defs") else {
        panic!("defs")
    };
    for (component, def) in defs.iter() {
        let Value::Obj(props) = prop(def, "properties") else {
            panic!("properties")
        };
        for (name, _) in props.iter() {
            if bindings.contains(component, name) {
                assert!(pairs.iter().any(|p|matches!(p,Value::Arr(p) if p.first()==Some(&Value::Str(component.clone()))&&p.get(1)==Some(&Value::Str(name.clone())))),"wrong mark");
            }
        }
    }
    for signature in [
        "null",
        "123",
        r#""no opening parenthesis""#,
        r#""Bad(x: $binding<string>""#,
    ] {
        let spec = value(&format!(
            r#"{{"components":{{"Bad":{{"signature":{signature}}}}},"schema":{{"$defs":{{"Bad":{{"properties":{{"x":{{}}}}}}}}}}}}"#
        ));
        assert_eq!(
            BindingProps::from_prompt_spec(&spec)
                .unwrap()
                .undecided()
                .collect::<Vec<_>>(),
            vec![("Bad", "x")]
        );
    }
    let plain = value(
        r#"{"components":{"C":{"signature":"C(value: string, trend: {value: number})"}},"schema":{"$defs":{"C":{"properties":{"value":{},"trend":{}}}}}}"#,
    );
    let plain = BindingProps::from_prompt_spec(&plain).unwrap();
    assert!(plain.undecided().next().is_none());
    assert!(!plain.contains("C", "value"));
    let sources = value(include_str!("../../../conformance/bindings.json"));
    let Value::Obj(sources) = &sources else {
        panic!("sources")
    };
    for (schema, path) in sources.iter() {
        let Value::Str(path) = path else {
            panic!("path")
        };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance")
            .join(path);
        let spec = value(&std::fs::read_to_string(path).unwrap());
        let cfg = BindingProps::from_prompt_spec(prop(&spec, "library")).unwrap();
        assert_eq!(
            cfg.undecided().collect::<Vec<_>>(),
            vec![],
            "ordinary spec {schema}"
        );
    }
    bindings.insert("E10", "e10").unwrap();
    assert!(bindings.contains("E10", "e10"));
    assert!(!bindings.undecided().any(|p| p == ("E10", "e10")));
    assert!(bindings.undecided().any(|p| p == ("E11", "e11")));
}

#[test]
fn marker_string_predicates_keep_the_pre_binding_work_prices() {
    let l = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Field":{"properties":{"value":{},"display":{}}}}}"#).unwrap(),
    );
    for (object, expected) in [
        (
            format!(r#"{{"__reactive":"{}"}}"#, "x".repeat(10_000)),
            20_167,
        ),
        (r#"{"__reactive":"no"}"#.to_owned(), 171),
        (
            format!(
                r#"{{"__reactive":"assign","type":"element","typeName":"{}","props":{{}},"partial":false}}"#,
                "x".repeat(10_000)
            ),
            40_377,
        ),
    ] {
        let p = parse("root=Field(null,$o)", &l, None);
        let mut s = Store::new();
        s.set("$o", Some(value(&object))).unwrap();
        let run = |work| {
            evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l),
                &s,
                Limits::DEFAULT.with_work(work),
            )
        };
        let (mut lo, mut hi) = (0, 100_000);
        while hi - lo > 1 {
            let m = (hi + lo) / 2;
            if run(m).size_limited { lo = m } else { hi = m }
        }
        assert_eq!(hi, expected);
        assert!(!run(expected).size_limited);
        assert!(run(expected - 1).size_limited);
    }
}

#[test]
fn ambiguous_signature_only_names_are_reported_and_can_be_excluded() {
    let spec = value(
        r#"{"components":{"C":{"signature":"C(a, b: $binding<string>, b: string, later: $binding<number>)"}}}"#,
    );
    let mut bindings = BindingProps::from_prompt_spec(&spec).unwrap();
    assert!(!bindings.contains("C", "b"));
    assert!(bindings.undecided().any(|p| p == ("C", "a, b")));
    assert!(bindings.undecided().any(|p| p == ("C", "b")));
    assert!(bindings.remove("C", "b"));
    assert!(!bindings.contains("C", "b"));
    assert!(!bindings.undecided().any(|p| p == ("C", "b")));
    assert!(!bindings.remove("C", "b"));
}

#[test]
fn removing_bindings_restores_exact_escaped_name_room_across_repeated_cycles() {
    let mut ten_bytes = BindingProps::with_limits(Limits::DEFAULT.with_result_size(10));
    for prop in ["a", "b"].into_iter().cycle().take(1024) {
        ten_bytes.insert("Box", prop).unwrap();
        assert!(ten_bytes.remove("Box", prop));
    }
    // Independent charge oracle: quoted JSON plus the existing separator byte.
    let charge = |s: &str| openui_lang::JsonValue::String(s.into()).to_string().len() + 1;
    let names = ["", "Box", "quote\"slash\\\n\r\t\u{8}\u{c}\u{1}", "é😀"];
    for component in names {
        for prop in names {
            for shared_component in [false, true] {
                let anchor_component = if shared_component {
                    component
                } else {
                    "anchor"
                };
                let anchor_prop = "kept";
                let starting = charge(anchor_component) + charge(anchor_prop);
                let added = charge(prop)
                    + if shared_component {
                        0
                    } else {
                        charge(component)
                    };
                let mut bindings =
                    BindingProps::with_limits(Limits::DEFAULT.with_result_size(starting + added));
                bindings.insert(anchor_component, anchor_prop).unwrap();
                let before = format!("{bindings:?}");
                for cycle in 0..128 {
                    // Alternate names, and also repeat the exact pair.
                    let name = if cycle % 3 == 0 { "x" } else { prop };
                    // Empty props are smaller than x: use the exact fitting pair
                    // for that row, rather than accidentally exceeding its cap.
                    let name = if charge(name) > charge(prop) {
                        prop
                    } else {
                        name
                    };
                    bindings.insert(component, name).unwrap();
                    bindings.insert(component, name).unwrap();
                    assert!(bindings.contains(component, name));
                    let full = format!("{bindings:?}");
                    let refusal = bindings
                        .insert("too large", "a name that cannot fit")
                        .unwrap_err();
                    assert!(refusal.flags.size_limited && !refusal.flags.depth_limited);
                    assert_eq!(
                        format!("{bindings:?}"),
                        full,
                        "refused insert changed state"
                    );
                    assert!(!bindings.remove(component, "absent"));
                    assert!(bindings.remove(component, name));
                    assert!(!bindings.remove(component, name));
                    assert!(!bindings.contains(component, name));
                    assert!(bindings.contains(anchor_component, anchor_prop));
                    assert_eq!(format!("{bindings:?}"), before, "cycle {cycle}");
                }
                // Exact fit after every cycle; one more name cannot fit.
                bindings.insert(component, prop).unwrap();
                assert!(bindings.insert(component, "extra").is_err());
                assert!(bindings.remove(component, prop));
                assert!(bindings.remove(anchor_component, anchor_prop));
                let empty =
                    BindingProps::with_limits(Limits::DEFAULT.with_result_size(starting + added));
                assert_eq!(format!("{bindings:?}"), format!("{empty:?}"));
            }
        }
    }
}

#[test]
fn removing_undecided_and_resolved_bindings_refunds_each_retained_component_once() {
    let spec = value(
        r#"{"components":{"q\"\\\n":{}},"schema":{"$defs":{"q\"\\\n":{"properties":{"p\"\\\t":{},"other":{}}}}}}"#,
    );
    let limits = Limits::DEFAULT.with_result_size(512);
    for resolve in [false, true] {
        let mut bindings = BindingProps::from_prompt_spec_with_limits(&spec, limits).unwrap();
        assert_eq!(bindings.undecided().count(), 2);
        if resolve {
            bindings.insert("q\"\\\n", "p\"\\\t").unwrap();
            assert_eq!(bindings.undecided().count(), 1);
        }
        assert!(bindings.remove("q\"\\\n", "p\"\\\t"));
        assert!(bindings.remove("q\"\\\n", "other"));
        assert_eq!(bindings.undecided().count(), 0);
        assert_eq!(
            format!("{bindings:?}"),
            format!("{:?}", BindingProps::with_limits(limits))
        );
        // All 512 bytes can be used again, through the public insert interface.
        bindings.insert("C", "p".repeat(505)).unwrap();
        assert!(bindings.insert("C", "x").is_err());
    }
}
