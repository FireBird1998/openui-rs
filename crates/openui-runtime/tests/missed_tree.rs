//! Small public tree cases pin bytes, flags and admission edges for #40.
//! The table records unchanged-runtime observations, including cut recovery.
#[path = "support/reuse.rs"]
#[allow(dead_code)]
mod reuse;
use openui_lang::{Element, Library, ParseResult, parse, parse_json};
use openui_runtime::*;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn value(json: &str) -> Value {
    Value::from_json(&parse_json(json).unwrap())
}
fn source(case: usize) -> &'static str {
    [
        "root=Box(@Each([1,2],i,Box($n,$n=$value)))",
        "root=Box(@Id(@Each([1,2],i,Box($n,$n=$value))))",
        "root=Box(@Each([1,2],i,$elements))",
        "root=Box(@Each([1,2],i,$shaped_elements))",
        "root=Box([$elements,$shaped_elements,$marker])",
        "root=Box(@Id($elements),@Id($shaped_elements),@Id($marker))",
        "root=Box(@Each([1,2],i,Box(@Each([3],j,Box($n)))))",
        "root=Box([@Each([1],i,Box(1)),@Each([1],i,Box($n))])",
        "root=Box(@Throw(),@Each([1],i,Box(@Throw())),@Throw())",
        "root=Box(@Id($n),@OpenUrl($n),@Reset($n))",
        "root=Box(@Probe($n),@Action([@Set($n,$n+1),@OpenUrl(\"ok\")]))",
        "root=Box($marker,$n=$value*2,$n)",
        "root=Box(@Each([1],i,Box($n,7,$n=$value)))",
        "root=Box(@Each([1],i,Box(@Id($marker),7,$n=$value)))",
        "root=Box(($n>2?[$elements[0],\"t\"]:0))",
        "root=Box(@Id($elements[0]))",
        "root=Box(@Id($shaped_elements[0]))",
        "root=Box(@Each([1],i,Box(1)))",
        r#"root=Box(@Each([1,2],i,Box($n,7,{__reactive:"assign",target:"$n"})))"#,
        "root=Box(@Each([1],i,Box($n,$n=$value,Box($n))))",
        "root=Box(@Warn(@Each([1],i,Box($n))))",
        "root=Box(@Warn(Box(@Warn(7))),@Warn(8))",
        "root=Box(($n>2?[$shaped_elements[0],7]:0))",
        "root=Box(@Each([1,2],i,Box($n,$n=$value)),@Each([1],i,Box($n)))",
        "root=Box(@Each($elements,e,e))",
        "root=Box(@Each($shaped_elements,e,e))",
        "root=Box(@Each($elements,e,Box(e)))",
        "root=Box(@Each($shaped_elements,e,Box(e)))",
        "root=Box(@Each($rows,i,Box(@Each($elements,e,e))))",
        "root=Box(@Each($rows,i,Box(@Each($shaped_elements,e,e))))",
        r#"root=Box({k:"Comp",name:"Action",args:"x"})"#,
        "root=Box([Box($n),@Warn(7)])",
        "root=Box(@Id($static))",
        "root=Box(@Each([1],i,Box($static)))",
        "root=Box(@Each([1],i,Box($static,$n=$value)))",
        "root=Box(@Each([1],i,Box($static,$static)))",
        "root=Box(($n>2?[$elements[0],7]:0))",
        "root=Box($shaped_elements[0])",
        "root=Box(@Each([1],i,Box($n,7,$n=$value)))",
        "root=Box(@Each([1],i,Box($n,7,$marker)))",
        "root=Box([Box($n),@Warn(7)])",
        "root=Box(@Each([1],i,Box([$n=$value])))",
        "root=Box(@Each([1],i,Box([[$n=$value]])))",
        "root=Box(@Each($boundElements,e,e))",
        "root=Box(@Each($boundShaped,e,e))",
        "root=Box(@Id($boundElements))",
        "root=Box(@Id($boundShaped))",
        "root=Box(($n>2?[$elements[0],@Each([1],i,Box(7))[0]]:[]))",
        "root=Box(@Each([1],i,Box(@Id($marker))))",
        "root=Box(@Each([1],i,Box($n=$value)))",
        "root=Box(($n>2?[$shaped_elements[0],@Each([1],i,Box(7))[0]]:[]))",
        "root=Box(@Each([1],i,Box(@Id($marker),7,$n=$value)))",
        "root=Box(@Each([1],i,Box(@Id($boundElements))))",
        "root=Box(@Each([1],i,Box(@Id($boundShaped))))",
        "root=Box(@Each([1],i,Box(0,0,$static)))",
        "root=Box(@Each([1],i,Box(0,0,$boundElements[0])))",
        "root=Box(@Each([1],i,Box(0,0,@Id($boundElements))))",
        "root=Box(($n>2?[$boundElements[0],@Each([1],i,Box($n))[0]]:[]))",
        "root=Box(0)",
        "root=Box(@Each([1],i,Box($n=$value,0,[$n=$value])))",
        r#"root=Box(@Each([1],i,Box({__reactive:"assign",target:"$n"},0,[$n=$value])))"#,
        "root=Box(@Each([0,1],i,i==0 ? $boundElements[0] : Box($n)))",
        r#"root=Box(@Each([1],i,Box({__reactive:"assign",target:($n>2?"$n":"x")},0,[$n=$value])))"#,
    ][case]
}
fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
fn observation(case: usize, data: bool, dimension: &str, allowance: usize) -> String {
    let mut schema = reuse::schema();
    schema["functions"]["Warn"] =
        parse_json(r#"{"params":{"type":"object","properties":{"value":{"type":"string"}}}}"#)
            .unwrap();
    let library = Library::from_json_schema(&schema);
    let mut functions = reuse::functions(&schema);
    functions
        .register("Warn", |v| {
            Ok(if let Value::Obj(o) = &v {
                o.get("value").cloned()
            } else {
                None
            })
        })
        .unwrap();
    let actions = parse_json(r#"{"Probe":{"params":{"properties":{"v":{}}}}}"#).unwrap();
    let (mut store, mut refs) = reuse::supplied(8);
    let static_element = Value::Element(Arc::new(Element {
        type_name: "Static".into(),
        props: JsObject::new(),
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    }));
    store.set("$static", Some(static_element)).unwrap();
    let marker =
        value(r#"{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}"#);
    let mut boundprops = JsObject::new();
    boundprops.define("a", Value::Arr(Arc::new(vec![marker])));
    let bound = Value::Element(Arc::new(Element {
        type_name: "Box".into(),
        props: boundprops,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    }));
    store
        .set(
            "$boundElements",
            Some(Value::Arr(Arc::new(vec![bound.clone()]))),
        )
        .unwrap();
    let bound = value(&value_to_json(&bound).unwrap());
    store
        .set("$boundShaped", Some(Value::Arr(Arc::new(vec![bound]))))
        .unwrap();
    // Undeclared supplies must not turn into references, even with receipts.
    refs.define("undeclared", Value::Num(123.));
    let prepared = PreparedReferenceValues::new(&refs);
    let mut bindings = BindingProps::new();
    bindings.insert("Box", "a").unwrap();
    bindings.insert("Box", "b").unwrap();
    let p = parse(source(case), &library, None);
    let mut p = if data { reuse::shaped(p) } else { p };
    if let Some(root) = p.root.as_mut() {
        root.statement_id = Some("root\u{1}".into());
        root.props.define("tail\u{2}", Value::Num(9.));
        if case == 30 {
            root.has_dynamic_props = true;
        }
        if case == 58 {
            let mut mapped = JsObject::new();
            mapped.define("c", Value::Ast(Arc::new(Ast::StateRef("$marker".into()))));
            let mut call = JsObject::new();
            call.define("k", Value::Str("Comp".into()));
            call.define("name", Value::Str("Other".into()));
            call.define("mappedProps", Value::Obj(Arc::new(mapped)));
            root.props.define("a", Value::Obj(Arc::new(call)));
            root.props.define("b", store.get("$marker").unwrap());
            root.has_dynamic_props = true;
        }
        if case == 40 {
            let mut childprops = JsObject::new();
            childprops.define("a-very-long-inner-property", Value::Num(1.));
            let child = Value::Element(Arc::new(Element {
                type_name: "Inner".into(),
                props: childprops,
                partial: false,
                has_dynamic_props: true,
                statement_id: None,
            }));
            let warning = value(r#"{"k":"Comp","name":"Warn","args":[{"k":"Num","v":7}]}"#);
            root.props
                .define("a", Value::Arr(Arc::new(vec![child, warning])));
            root.has_dynamic_props = true;
        }
    }
    let input = TreeInput::new(&p)
        .with_library(&library)
        .with_functions(&functions)
        .with_actions(&actions)
        .with_binding_props(&bindings)
        .with_prepared_reference_values(&prepared);
    let limits = match dimension {
        "work" => Limits::DEFAULT.with_work(allowance),
        "room" => Limits::DEFAULT.with_result_size(allowance),
        "depth" => Limits::DEFAULT.with_depth(allowance),
        "memory" => Limits::DEFAULT.with_peak_memory(allowance),
        _ => panic!("unknown allowance"),
    };
    let r = evaluate_tree_with_limits(input, &store, limits);
    format!(
        "{:016x} {} {}",
        digest(&r.to_json().unwrap()),
        r.size_limited,
        r.depth_limited
    )
}
fn check_table(dimension: &str) {
    for line in include_str!("support/missed-tree.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        if fields[2] != dimension {
            continue;
        }
        let case = fields[0].parse().unwrap();
        let data = fields[1].parse().unwrap();
        for allowance in [fields[3].parse().unwrap(), fields[4].parse().unwrap()] {
            assert_eq!(
                observation(case, data, dimension, allowance),
                fields[5],
                "case={case} data={data} {dimension}={allowance}: {}",
                source(case)
            );
        }
    }
}
#[test]
fn loop_elements_and_bindings_keep_their_work_edges_and_cut_bytes() {
    check_table("work");
}
#[test]
fn loop_elements_and_bindings_keep_their_room_edges_and_cut_bytes() {
    check_table("room");
}
#[test]
fn reused_elements_and_markers_keep_their_use_depth_edges() {
    check_table("depth");
}
#[test]
fn refused_element_rebuilds_keep_memory_flags_and_recovery_bytes() {
    check_table("memory");
}

#[test]
fn descriptor_mapped_props_use_javascript_truthiness_and_keep_numeric_keys() {
    let mut p = ParseResult::default();
    let mut props = JsObject::new();
    let inputs = [
        Value::Null,
        Value::Bool(false),
        Value::Bool(true),
        Value::Num(0.),
        Value::Num(1.),
        Value::Num(f64::NAN),
        Value::Str("".into()),
        Value::Str("x".into()),
        value(r#"{"a":{"k":"Num","v":7}}"#),
        value(r#"[{"k":"Num","v":8}]"#),
    ];
    for (i, mapped) in inputs.iter().enumerate() {
        let mut call = JsObject::new();
        call.define("k", Value::Str("Comp".into()));
        call.define("name", Value::Str("Other".into()));
        call.define("mappedProps", mapped.clone());
        props.define(i.to_string(), Value::Obj(Arc::new(call)));
    }
    p.root = Some(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    });
    let r = evaluate_tree(TreeInput::new(&p), &Store::new());
    assert!(!r.size_limited && !r.depth_limited && r.errors.is_empty());
    let Some(Value::Element(ref root)) = r.root else {
        panic!("root")
    };
    for i in [0, 1, 3, 5, 6] {
        assert_eq!(root.props.get(&i.to_string()), Some(&Value::Null), "{i}");
    }
    for i in [2, 4, 7] {
        assert!(
            matches!(root.props.get(&i.to_string()),Some(Value::Element(e)) if e.props.is_empty()),
            "{i}"
        );
    }
    for (i, key, n) in [(8, "a", 7.), (9, "0", 8.)] {
        assert!(
            matches!(root.props.get(&i.to_string()),Some(Value::Element(e)) if e.props.get(key)==Some(&Value::Num(n))),
            "{i}"
        );
    }
}

#[test]
fn tree_export_admits_empty_roots_and_diagnostic_envelopes_at_exact_room() {
    let library = Library::from_json_schema(&reuse::schema());
    let schema = reuse::schema();
    let functions = reuse::functions(&schema);
    let p = parse("root=Box(@Throw(),@Throw())", &library, None);
    let tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
    assert_eq!(tree.errors.len(), 2);
    for count in 0..=2 {
        let mut tree = tree.clone();
        tree.errors.truncate(count);
        for root in [None, tree.root.clone()] {
            tree.root = root;
            let json = tree.to_json().unwrap();
            assert_eq!(
                tree.to_json_with_limits(Limits::DEFAULT.with_result_size(json.len()))
                    .unwrap(),
                json
            );
            assert!(
                tree.to_json_with_limits(Limits::DEFAULT.with_result_size(json.len() - 1))
                    .unwrap_err()
                    .flags
                    .size_limited
            );
        }
    }
}

#[test]
fn nested_diagnostics_restore_the_following_props_component_context() {
    let schema = reuse::schema();
    let library = Library::from_json_schema(&schema);
    let functions = reuse::functions(&schema);
    let mut p = parse("root=Box(Box(@Throw()),@Throw())", &library, None);
    let root = p.root.as_mut().unwrap();
    root.type_name = "Outer\u{1}".into();
    root.statement_id = Some("outer\u{2}".into());
    let tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
    assert_eq!(tree.errors.len(), 2);
    assert_eq!(tree.errors[0].component.as_deref(), Some("Box"));
    assert_eq!(tree.errors[1].component.as_deref(), Some("Outer\u{1}"));
    assert_eq!(tree.errors[1].statement_id.as_deref(), Some("outer\u{2}"));
    let json = tree.to_json().unwrap();
    assert!(json.contains("Outer\\u0001"));
}

#[test]
fn undeclared_prepared_supplies_cannot_be_read_by_runtime_descriptors() {
    let mut p = ParseResult::default();
    let mut props = JsObject::new();
    props.define(
        "a",
        value(r#"{"k":"RuntimeRef","n":"not-declared","refType":"query"}"#),
    );
    p.root = Some(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    });
    let mut values = JsObject::new();
    values.define("not-declared", Value::Num(123.));
    let prepared = PreparedReferenceValues::new(&values);
    let tree = evaluate_tree(
        TreeInput::new(&p).with_prepared_reference_values(&prepared),
        &Store::new(),
    );
    assert!(!tree.size_limited && !tree.depth_limited && tree.errors.is_empty());
    let Some(Value::Element(root)) = tree.root.as_ref() else {
        panic!("root")
    };
    assert_eq!(root.props.get("a"), Some(&Value::Null));
}

#[test]
fn typed_state_references_in_descriptor_props_keep_binding_authority() {
    let mut p = ParseResult::default();
    let mut props = JsObject::new();
    let mut mapped = JsObject::new();
    mapped.define("a", Value::Ast(Arc::new(Ast::StateRef("$n".into()))));
    let mut call = JsObject::new();
    call.define("k", Value::Str("Comp".into()));
    call.define("name", Value::Str("Other".into()));
    call.define("mappedProps", Value::Obj(Arc::new(mapped)));
    props.define("a", Value::Obj(Arc::new(call)));
    p.root = Some(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    });
    let mut bindings = BindingProps::new();
    bindings.insert("Other", "a").unwrap();
    let mut store = Store::new();
    store.set("$n", Some(Value::Num(7.))).unwrap();
    let tree = evaluate_tree(TreeInput::new(&p).with_binding_props(&bindings), &store);
    let Some(Value::Element(root)) = tree.root.as_ref() else {
        panic!("root")
    };
    let Some(Value::Element(child)) = root.props.get("a") else {
        panic!("child")
    };
    let marker = child.props.get("a").unwrap();
    assert!(value_to_json(marker).unwrap().contains("\"target\":\"$n\""));
    assert!(write_binding(TreeInput::new(&p), &mut store, marker, Value::Num(9.)).unwrap());
    assert_eq!(store.get("$n"), Some(Value::Num(9.)));
}

#[test]
fn array_like_action_arguments_keep_their_first_expression_and_ordered_steps() {
    let mut p = ParseResult::default();
    let mut props = JsObject::new();
    props.define("a",value(r#"{"k":"Comp","name":"Action","args":{"length":1,"0":{"k":"Arr","els":[{"k":"Comp","name":"OpenUrl","args":[{"k":"Str","v":"first"}]},{"k":"Comp","name":"OpenUrl","args":[{"k":"Str","v":"second"}]}]}}}"#));
    p.root = Some(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    });
    let mut store = Store::new();
    let tree = evaluate_tree(TreeInput::new(&p), &store);
    assert!(
        !tree.size_limited && !tree.depth_limited && tree.errors.is_empty(),
        "{tree:?}"
    );
    let Some(Value::Element(root)) = tree.root.as_ref() else {
        panic!("root")
    };
    let run = run_action(
        TreeInput::new(&p),
        &mut store,
        root.props.get("a"),
        "",
        None,
    );
    assert!(run.failure.is_none() && !run.size_limited && !run.depth_limited);
    assert_eq!(run.events.len(), 2);
    assert_eq!(
        run.events
            .iter()
            .map(|event| event.to_json().unwrap())
            .collect::<Vec<_>>(),
        [
            r#"{"type":"open_url","params":{"url":"first"},"humanFriendlyMessage":"","formState":{}}"#,
            r#"{"type":"open_url","params":{"url":"second"},"humanFriendlyMessage":"","formState":{}}"#,
        ]
    );
    assert_eq!(
        value_to_json(root.props.get("a").unwrap()).unwrap(),
        r#"{"steps":[{"type":"open_url","url":"first"},{"type":"open_url","url":"second"}]}"#
    );
}
#[test]
fn array_mapped_props_do_not_copy_a_temporary_property_map() {
    fn calls(array: bool, n: usize) -> usize {
        let mut fields = JsObject::new();
        let mut items = Vec::new();
        for i in 0..n {
            let v = value(r#"{"k":"Num","v":7}"#);
            fields.define(i.to_string(), v.clone());
            items.push(v);
        }
        let mut call = JsObject::new();
        call.define("k", Value::Str("Comp".into()));
        call.define("name", Value::Str("Other".into()));
        call.define(
            "mappedProps",
            if array {
                Value::Arr(Arc::new(items))
            } else {
                Value::Obj(Arc::new(fields))
            },
        );
        let mut props = JsObject::new();
        props.define("a", Value::Obj(Arc::new(call)));
        let p = ParseResult {
            root: Some(Element {
                type_name: "Box".into(),
                props,
                partial: false,
                has_dynamic_props: true,
                statement_id: None,
            }),
            ..Default::default()
        };
        let store = Store::new();
        let start = test_alloc::allocation_calls();
        let tree = evaluate_tree(TreeInput::new(&p), &store);
        let count = test_alloc::allocation_calls() - start;
        assert!(!tree.size_limited && !tree.depth_limited && tree.errors.is_empty());
        count
    }
    for n in [8, 64] {
        let object = calls(false, n);
        let array = calls(true, n);
        assert!(array <= object + 8, "n={n}: array={array}, object={object}");
    }
}
#[test]
fn shaped_element_diagnostics_retain_their_statement_id() {
    let schema = reuse::schema();
    let functions = reuse::functions(&schema);
    let mut props = JsObject::new();
    props.define("a",value(r#"{"type":"element","typeName":"Other","props":{"bad":{"k":"Comp","name":"Throw","args":[]}},"partial":false,"hasDynamicProps":true,"statementId":"shaped-id"}"#));
    let p = ParseResult {
        root: Some(Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..Default::default()
    };
    let tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
    assert!(!tree.size_limited && !tree.depth_limited);
    assert_eq!(tree.errors.len(), 1);
    assert_eq!(tree.errors[0].component.as_deref(), Some("Other"));
    assert_eq!(tree.errors[0].statement_id.as_deref(), Some("shaped-id"));
}

#[test]
fn diagnostic_export_charges_commas_to_its_shared_work_allowance() {
    let schema = reuse::schema();
    let library = Library::from_json_schema(&schema);
    let functions = reuse::functions(&schema);
    let p = parse("root=Box(@Throw(),@Throw())", &library, None);
    let tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
    // Observed on unchanged main, rather than inferred from serializer internals.
    for (n, work) in [(0, 65), (1, 809), (2, 1555)] {
        let mut tree = tree.clone();
        tree.errors.truncate(n);
        tree.root = None;
        let json = tree.to_json().unwrap();
        assert!(
            tree.to_json_with_limits(Limits::DEFAULT.with_work(work - 1))
                .unwrap_err()
                .flags
                .size_limited
        );
        for allowance in [work, work + 1] {
            assert_eq!(
                tree.to_json_with_limits(Limits::DEFAULT.with_work(allowance))
                    .unwrap(),
                json
            );
        }
    }
}
#[test]
fn refused_diagnostic_export_checks_text_before_copying_the_message() {
    let schema = reuse::schema();
    let library = Library::from_json_schema(&schema);
    let functions = reuse::functions(&schema);
    let p = parse("root=Box(@Throw())", &library, None);
    let mut tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
    tree.root = None;
    tree.errors[0].message = "x".repeat(128 * 1024);
    let peak = |limits| {
        test_alloc::reset_peak();
        let start = test_alloc::peak();
        let result = tree.to_json_with_limits(limits);
        (result, test_alloc::peak().saturating_sub(start))
    };
    let (whole, control) = peak(Limits::DEFAULT);
    assert!(whole.is_ok());
    drop(whole);
    for limits in [
        Limits::DEFAULT.with_work(100),
        Limits::DEFAULT.with_result_size(100),
    ] {
        let (refused, used) = peak(limits);
        assert!(refused.unwrap_err().flags.size_limited);
        assert!(
            used < control / 8 + 16 * 1024,
            "refused={used}, completed={control}"
        );
    }
}

#[test]
fn deferred_set_syntax_keeps_positional_and_mapped_bodies_independent() {
    let body = Arc::new(vec![Ast::Ref("i".into())]);
    let mut mapped = JsObject::new();
    mapped.define("v", Ast::Arr(body.clone()));
    let component = Ast::Comp {
        name: "Other".into(),
        args: Arc::new(vec![Ast::Arr(body)]),
        function: false,
        mapped_props: Some(Arc::new(mapped)),
    };
    // Public typed descriptors can carry both representations on a deferred
    // call too. Its unexecuted valueAST remains inspectable by the host.
    let set = Ast::Comp {
        name: "Set".into(),
        args: Arc::new(vec![Ast::StateRef("$n".into()), component]),
        function: true,
        mapped_props: Some(Arc::new(JsObject::new())),
    };
    let each = Ast::Comp {
        name: "Each".into(),
        args: Arc::new(vec![
            Ast::Arr(Arc::new(vec![Ast::Num(1.)])),
            Ast::Ref("i".into()),
            set,
        ]),
        function: true,
        mapped_props: None,
    };
    let mut props = JsObject::new();
    props.define("a", Value::Ast(Arc::new(each)));
    let p = ParseResult {
        root: Some(Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..Default::default()
    };
    let tree = evaluate_tree(TreeInput::new(&p), &Store::new());
    assert!(
        !tree.size_limited && !tree.depth_limited && tree.errors.is_empty(),
        "{tree:?}"
    );
    let Some(Value::Element(root)) = tree.root.as_ref() else {
        panic!("root")
    };
    let Some(Value::Arr(plans)) = root.props.get("a") else {
        panic!("plans")
    };
    let Value::Obj(plan) = &plans[0] else {
        panic!("plan")
    };
    let Some(Value::Arr(steps)) = plan.get("steps") else {
        panic!("steps")
    };
    let Value::Obj(step) = &steps[0] else {
        panic!("step")
    };
    let Some(Value::Ast(ast)) = step.get("valueAST") else {
        panic!("deferred AST")
    };
    let Ast::Comp {
        args,
        mapped_props: Some(mapped),
        ..
    } = ast.as_ref()
    else {
        panic!("component syntax")
    };
    let Ast::Arr(positional) = &args[0] else {
        panic!("positional")
    };
    let Some(Ast::Arr(named)) = mapped.get("v") else {
        panic!("mapped")
    };
    assert!(!Arc::ptr_eq(positional, named));
    assert_eq!(positional.as_ref(), named.as_ref());
}

#[test]
fn diagnostic_export_keeps_depth_refusal_when_text_fills_the_size_ceiling() {
    let schema = reuse::schema();
    let library = Library::from_json_schema(&schema);
    let functions = reuse::functions(&schema);
    let p = parse("root=Box(@Throw())", &library, None);
    let mut tree = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
    tree.root = None;
    tree.errors[0].component = None;
    tree.errors[0].statement_id = None;
    tree.errors[0].hint = None;
    tree.errors[0].message = "x".repeat(100);
    let error = tree
        .to_json_with_limits(Limits::DEFAULT.with_depth(0).with_result_size(100))
        .unwrap_err();
    assert!(error.flags.depth_limited);
    assert!(!error.flags.size_limited);
    assert_eq!(error.to_string(), "depth limit exceeded");
}

/// Re-record the existing probe coordinates, splitting a row if its endpoints
/// now differ. This deliberately preserves coverage coordinates across cost changes.
#[test]
#[ignore = "rewrites the checked-in observation fixture"]
fn regenerate_missed_tree_observations() {
    let mut output = String::from(
        "# case\tdata\tallowance\tfirst\tlast\tJSON FNV-1a64 size_limited depth_limited\n",
    );
    for line in include_str!("support/missed-tree.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let f: Vec<_> = line.split('\t').collect();
        let first: usize = f[3].parse().unwrap();
        let last: usize = f[4].parse().unwrap();
        let a = observation(f[0].parse().unwrap(), f[1].parse().unwrap(), f[2], first);
        let b = observation(f[0].parse().unwrap(), f[1].parse().unwrap(), f[2], last);
        if a == b {
            output.push_str(&format!(
                "{}\t{}\t{}\t{first}\t{last}\t{a}\n",
                f[0], f[1], f[2]
            ));
        } else {
            for (at, value) in [(first, a), (last, b)] {
                output.push_str(&format!(
                    "{}\t{}\t{}\t{at}\t{at}\t{value}\n",
                    f[0], f[1], f[2]
                ));
            }
        }
    }
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/missed-tree.tsv"),
        output,
    )
    .unwrap();
}
