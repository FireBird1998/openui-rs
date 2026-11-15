//! Directed public-interface rules and safety, independent of pending cases.
use openui_lang::{
    Element, Library, ParseResult, free_json, parse, parse_expression, parse_json, tokenize,
};
use openui_runtime::{
    Ast, JsObject, Limits, Store, TreeResult, Value, evaluate_tree, evaluate_tree_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn library() -> Library {
    let j =
        parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    l
}
fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}
fn initialized(p: &ParseResult) -> Store {
    let mut s = Store::new();
    let r = s.initialize(
        p.state_declarations
            .iter()
            .map(|(k, v)| (k.as_str(), Some(v))),
        [],
    );
    assert!(r.refused.is_empty());
    s
}
fn prop<'a>(r: &'a TreeResult, k: &str) -> &'a Value {
    root(r).props.get(k).unwrap()
}
fn whole(r: TreeResult) -> TreeResult {
    assert!(!r.size_limited && !r.depth_limited, "{r:?}");
    r
}
fn root(r: &openui_runtime::TreeResult) -> &openui_lang::Element {
    match r.root.as_ref().unwrap() {
        Value::Element(e) => e,
        _ => panic!("element"),
    }
}
#[test]
fn scalars_and_null_pass_through() {
    let l = library();
    let mut p = parse("root=Box(null,42,true,\"hello\")", &l, None);
    p.root.as_mut().unwrap().has_dynamic_props = true;
    let r = whole(evaluate_tree((&p).into(), &Store::new()));
    assert_eq!(prop(&r, "a"), &Value::Null);
    assert_eq!(prop(&r, "b"), &Value::Num(42.0));
    assert_eq!(prop(&r, "c"), &Value::Bool(true));
    assert_eq!(prop(&r, "d"), &Value::Str("hello".into()));
    assert_eq!(
        r.to_json().unwrap(),
        r#"{"root":{"type":"element","typeName":"Box","props":{"a":null,"b":42,"c":true,"d":"hello"},"partial":false,"hasDynamicProps":true,"statementId":"root"},"errors":[]}"#
    );
    assert!(r.errors.is_empty());
}
#[test]
fn expressions_read_state_and_resolve_inlined_statements() {
    let l = library();
    let p = parse("root=Box(label,$n+1)\nlabel=\"n=\"+$n\n$n=7", &l, None);
    let mut s = initialized(&p);
    let before = s.version();
    let r = whole(evaluate_tree((&p).into(), &s));
    assert_eq!(prop(&r, "a"), &Value::Str("n=7".into()));
    assert_eq!(prop(&r, "b"), &Value::Num(8.0));
    assert_eq!(s.version(), before);
    assert!(s.is_pristine("$n"));
    s.set("$n", Some(Value::Num(20.0))).unwrap();
    let r = whole(evaluate_tree((&p).into(), &s));
    assert_eq!(prop(&r, "a"), &Value::Str("n=20".into()));
}
#[test]
fn lists_elements_and_nested_objects() {
    let l = library();
    let p = parse(
        "root=Box([$n,Box($n)],{flat:1,nested:{v:$n},list:[$n]})\n$n=9",
        &l,
        None,
    );
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    let j = r.to_json().unwrap();
    assert!(!j.contains("StateRef"), "{j}");
    assert!(j.contains(r#""nested":{"v":9},"list":[9]"#));
    assert!(j.contains(r#""a":[9,{"type":"element","typeName":"Box","props":{"a":9}"#));
}
#[test]
fn static_elements_and_flat_objects_share_bodies() {
    let l = library();
    let p = parse("root=Box(Box(\"static\"),{flat:1},$n)\n$n=2", &l, None);
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    for k in ["a", "b"] {
        match (p.root.as_ref().unwrap().props.get(k).unwrap(), prop(&r, k)) {
            (Value::Element(a), Value::Element(b)) => assert!(Arc::ptr_eq(a, b)),
            (Value::Obj(a), Value::Obj(b)) => assert!(Arc::ptr_eq(a, b)),
            _ => panic!("shape"),
        }
    }
    // hasDynamicProps=false is authoritative, even on a caller-built element.
    let mut p = p;
    let root = p.root.as_mut().unwrap();
    root.has_dynamic_props = false;
    root.props.define("c", Value::Ast(Arc::new(ast("$n"))));
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    assert!(matches!(prop(&r, "c"), Value::Ast(_)));
}
#[test]
fn expression_results_revisit_only_elements() {
    let l = library();
    let child = parse("root=Box($n)", &l, None).root.unwrap();
    let raw = Value::Ast(Arc::new(ast("$n")));
    let values = Value::Arr(Arc::new(vec![raw.clone(), Value::Element(Arc::new(child))]));
    let p = parse("root=Box($items)", &l, None);
    let mut s = Store::new();
    s.set("$items", Some(values)).unwrap();
    s.set("$n", Some(Value::Num(4.0))).unwrap();
    let r = whole(evaluate_tree((&p).into(), &s));
    let Value::Arr(a) = prop(&r, "a") else {
        panic!("list")
    };
    assert_eq!(a.first(), Some(&raw));
    let Some(Value::Element(e)) = a.get(1) else {
        panic!("element")
    };
    assert_eq!(e.props.get("a"), Some(&Value::Num(4.0)));
}
#[test]
fn failed_props_preserve_raw_values_and_report_in_order() {
    let l = library();
    let p = parse(
        "root=Box(\"bad \"+$bad,child,\"good\",false&&(\"skip \"+$bad))\nchild=Box({nested:[\"again \"+$bad]})\n$bad={toString:null}",
        &l,
        None,
    );
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    assert_eq!(r.errors.len(), 2);
    assert_eq!(
        prop(&r, "a"),
        p.root.as_ref().unwrap().props.get("a").unwrap()
    );
    assert_eq!(prop(&r, "c"), &Value::Str("good".into()));
    assert_eq!(prop(&r, "d"), &Value::Bool(false));
    assert_eq!(
        r.errors[0].message,
        "Evaluating prop \"a\" on Box failed: No default value"
    );
    assert_eq!(
        r.errors[0].hint.as_deref(),
        Some("Check the expression used for prop \"a\"")
    );
    assert_eq!(r.errors[0].statement_id.as_deref(), Some("root"));
    assert_eq!(r.errors[1].statement_id.as_deref(), Some("child"));
    assert!(r.to_json().unwrap().contains(r#""source":"runtime","code":"runtime-error","component":"Box","statementId":"root","message":"Evaluating prop \"a\" on Box failed: No default value","hint":"Check the expression used for prop \"a\""#));
}
#[test]
fn failed_sort_and_filter_props_keep_raw_calls() {
    let l = library();
    let p = parse(
        "root=Box(@Sort([$bad,{}],\"\"),@Filter([$bad],\"\",\"contains\",\"x\"),$bad-1,@Abs($bad))\n$bad={toString:1}",
        &l,
        None,
    );
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    assert_eq!(r.errors.len(), 2);
    assert!(matches!(prop(&r, "a"), Value::Ast(_)));
    assert!(matches!(prop(&r, "b"), Value::Ast(_)));
    assert_eq!(prop(&r, "c"), &Value::Num(-1.0));
    assert_eq!(prop(&r, "d"), &Value::Num(0.0));
}
#[test]
fn undefined_is_omitted_in_props_and_null_in_lists() {
    let l = library();
    let p = parse("root=Box($missing,[$missing])", &l, None);
    let r = whole(evaluate_tree((&p).into(), &Store::new()));
    assert!(!root(&r).props.has_own("a"));
    assert_eq!(prop(&r, "b"), &Value::Arr(Arc::new(vec![Value::Null])));
}
#[test]
fn work_and_result_room_are_shared_across_the_tree() {
    let l = library();
    let single = parse("root=Box($text)", &l, None);
    let many = parse(
        &format!("root=Box([{}])", vec!["Box($text)"; 100].join(",")),
        &l,
        None,
    );
    let mut s = Store::new();
    s.set("$text", Some(Value::Str("x".repeat(100)))).unwrap();
    for limits in [
        Limits::DEFAULT.with_work(10_000),
        Limits::DEFAULT.with_result_size(2048),
    ] {
        whole(evaluate_tree_with_limits((&single).into(), &s, limits));
        let r = evaluate_tree_with_limits((&many).into(), &s, limits);
        assert!(r.size_limited && !r.depth_limited);
    }
}
#[test]
fn state_credit_is_read_only() {
    let l = library();
    let p = parse("root=Box($text)", &l, None);
    let limits = Limits::DEFAULT
        .with_result_floor(32)
        .with_amplification(2)
        .with_result_ceiling(256_000);
    let mut s = Store::new();
    s.set("$text", Some(Value::Str("x".repeat(8000)))).unwrap();
    s.set("$unread", Some(Value::Str("u".repeat(128_000))))
        .unwrap();
    let r = whole(evaluate_tree_with_limits((&p).into(), &s, limits));
    assert_eq!(prop(&r, "a"), &Value::Str("x".repeat(8000)));
}
fn chain_tree(depth: usize, expressions: usize) -> ParseResult {
    let mut ast = Ast::Num(7.0);
    for _ in 0..expressions {
        ast = Ast::UnaryOp {
            op: "-",
            operand: Arc::new(ast),
        };
    }
    let mut v = Value::Ast(Arc::new(ast));
    for _ in 0..depth {
        let mut props = JsObject::new();
        props.define("a", v);
        v = Value::Element(Arc::new(Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }));
    }
    ParseResult {
        root: match &v {
            Value::Element(e) => Some((**e).clone()),
            _ => None,
        },
        ..ParseResult::default()
    }
}
#[test]
fn deep_trees_and_expressions_cut_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            for (depth, expressions) in [(200_000, 0), (1, 200_000)] {
                let p = chain_tree(depth, expressions);
                let r = evaluate_tree((&p).into(), &Store::new());
                assert!(r.depth_limited && !r.size_limited);
                assert!(r.root.is_some());
            }
            let p = chain_tree(1, 1000);
            let r = whole(evaluate_tree_with_limits(
                (&p).into(),
                &Store::new(),
                Limits::DEFAULT.with_depth(1500),
            ));
            assert_eq!(prop(&r, "a"), &Value::Num(7.0));
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn tree_nesting_and_expressions_share_depth() {
    let limits = Limits::DEFAULT.with_depth(24);
    for (depth, expressions) in [(11, 0), (1, 12)] {
        let p = chain_tree(depth, expressions);
        whole(evaluate_tree_with_limits(
            (&p).into(),
            &Store::new(),
            limits,
        ));
    }
    let p = chain_tree(11, 12);
    let r = evaluate_tree_with_limits((&p).into(), &Store::new(), limits);
    assert!(r.depth_limited && !r.size_limited);
}
#[test]
fn evaluation_allocation_cost_scales_with_tree_width() {
    let l = library();
    let mut s = Store::new();
    s.set("$n", Some(Value::Num(3.0))).unwrap();
    let measure = |n| {
        let p = parse(
            &format!("root=Box([{}])", vec!["Box($n)"; n].join(",")),
            &l,
            None,
        );
        let before = test_alloc::allocation_calls();
        let r = whole(evaluate_tree((&p).into(), &s));
        let allocations = test_alloc::allocation_calls() - before;
        let Value::Arr(items) = prop(&r, "a") else {
            panic!("list")
        };
        assert_eq!(items.len(), n);
        for v in items.iter() {
            let Value::Element(e) = v else {
                panic!("element")
            };
            assert_eq!(e.props.get("a"), Some(&Value::Num(3.0)));
        }
        allocations
    };
    let a = measure(128);
    let b = measure(256);
    let c = measure(512);
    assert!(b <= a * 22 / 10 && c <= b * 22 / 10, "{a}, {b}, {c}");
}
#[test]
fn a_large_state_container_stays_shared_with_bounded_peak_heap() {
    let l = library();
    let p = parse("root=Box($items)", &l, None);
    let original: Arc<Vec<Value>> =
        Arc::new((0..50_000).map(|i| Value::Num(f64::from(i))).collect());
    let mut s = Store::new();
    s.set("$items", Some(Value::Arr(original.clone()))).unwrap();
    test_alloc::reset_peak();
    let input = test_alloc::peak();
    let r = whole(evaluate_tree((&p).into(), &s));
    let Value::Arr(output) = prop(&r, "a") else {
        panic!("list")
    };
    assert!(Arc::ptr_eq(&original, output));
    assert_eq!(output.len(), 50_000);
    assert!(
        test_alloc::peak() < input * 2,
        "input={input}, peak={}",
        test_alloc::peak()
    );
}
#[test]
fn costly_props_share_work_even_when_the_final_tree_is_small() {
    let l = library();
    let one = parse("root=Box(@Sum($items))", &l, None);
    let many = parse(
        &format!("root=Box([{}])", ["Box(@Sum($items))"; 10].join(",")),
        &l,
        None,
    );
    let mut s = Store::new();
    s.set(
        "$items",
        Some(Value::Arr(Arc::new(vec![Value::Num(1.0); 1000]))),
    )
    .unwrap();
    let limits = Limits::DEFAULT.with_work(20_000);
    let single = whole(evaluate_tree_with_limits((&one).into(), &s, limits));
    assert_eq!(prop(&single, "a"), &Value::Num(1000.0));
    let r = evaluate_tree_with_limits((&many).into(), &s, limits);
    assert!(r.size_limited && !r.depth_limited);
}
#[test]
fn structural_ast_values_follow_the_same_evaluator_and_literal_rules() {
    let l = library();
    let p = parse(
        r#"root=Box({k:"BinOp",op:"+",left:{k:"StateRef",n:"$n"},right:{k:"Num",v:1}},$n,{k:"Num",v:{raw:1}},{k:"Arr",els:[{k:"Str",v:"x"},3,{}]})
$n=4"#,
        &l,
        None,
    );
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    assert_eq!(prop(&r, "a"), &Value::Num(5.0));
    assert_eq!(prop(&r, "b"), &Value::Num(4.0));
    assert!(
        r.to_json()
            .unwrap()
            .contains(r#""c":{"raw":1},"d":["x",null,null]"#)
    );
}
#[test]
fn result_room_is_shared_when_final_tree_is_small() {
    let l = library();
    let p = parse(
        &format!(
            "root=Box([{}])",
            ["Box(@Count([$text+\"\"]))"; 10].join(",")
        ),
        &l,
        None,
    );
    let mut s = Store::new();
    s.set("$text", Some(Value::Str("x".repeat(1000)))).unwrap();
    let complete = whole(evaluate_tree((&p).into(), &s));
    assert!(complete.to_json().unwrap().len() < 2000);
    let r = evaluate_tree_with_limits((&p).into(), &s, Limits::DEFAULT.with_result_size(2000));
    assert!(r.size_limited && !r.depth_limited);
}
#[test]
fn malformed_structural_values_do_not_panic_or_poison_later_props() {
    let l = library();
    for kind in [
        "Comp",
        "Str",
        "Num",
        "Bool",
        "Null",
        "Arr",
        "Obj",
        "Ref",
        "Ph",
        "StateRef",
        "RuntimeRef",
        "BinOp",
        "UnaryOp",
        "Ternary",
        "Member",
        "Index",
        "Assign",
    ] {
        let mut p = parse("root=Box($x,\"later\")", &l, None);
        let mut value = JsObject::new();
        value.define("k", Value::Str(kind.into()));
        p.root
            .as_mut()
            .unwrap()
            .props
            .define("a", Value::Obj(Arc::new(value)));
        let r = whole(evaluate_tree((&p).into(), &Store::new()));
        assert_eq!(prop(&r, "b"), &Value::Str("later".into()));
        assert!(r.errors.len() <= 1);
    }
}
#[test]
fn structural_ast_short_circuits_and_branch_errors_stay_lazy() {
    let l = library();
    let p = parse(
        r#"root=Box({k:"BinOp",op:"&&",left:{k:"Bool",v:false},right:null},$n,{k:"Ternary",cond:{k:"Bool",v:true},then:{k:"Num",v:9},else:null})
$n=4"#,
        &l,
        None,
    );
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(prop(&r, "a"), &Value::Bool(false));
    assert_eq!(prop(&r, "c"), &Value::Num(9.0));
}
#[test]
fn error_bytes_are_bounded_before_formatting_many_large_diagnostics() {
    let mut props = JsObject::new();
    props.define("before", Value::Ast(Arc::new(ast("41+1"))));
    let expression = Arc::new(ast("\"x\"+$bad"));
    for i in 0..256 {
        props.define(format!("p{i}"), Value::Ast(expression.clone()));
    }
    let p = ParseResult {
        root: Some(Element {
            type_name: "T".repeat(32_000),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..ParseResult::default()
    };
    let mut bad = JsObject::new();
    bad.define("toString", Value::Null);
    let mut s = Store::new();
    s.set("$bad", Some(Value::Obj(Arc::new(bad)))).unwrap();
    test_alloc::reset_peak();
    let input = test_alloc::peak();
    let r = evaluate_tree_with_limits((&p).into(), &s, Limits::DEFAULT.with_result_size(40_000));
    assert!(r.size_limited && !r.depth_limited);
    assert_eq!(prop(&r, "before"), &Value::Num(42.0));
    assert!(r.errors.is_empty());
    assert!(
        test_alloc::peak() < input * 3,
        "input={input}, peak={}",
        test_alloc::peak()
    );
}
#[test]
fn failed_inlined_statements_report_each_executed_prop() {
    let l = library();
    let p = parse("root=Box(r,false&&r,r)\nr=\"x\"+$bad\n$bad={}", &l, None);
    let mut s = Store::new();
    let mut bad = JsObject::new();
    bad.define("toString", Value::Null);
    s.set("$bad", Some(Value::Obj(Arc::new(bad)))).unwrap();
    let r = whole(evaluate_tree((&p).into(), &s));
    assert_eq!(r.errors.len(), 2);
    assert!(r.errors[0].message.contains("prop \"a\""));
    assert!(r.errors[1].message.contains("prop \"c\""));
    assert!(matches!(prop(&r, "a"), Value::Ast(_)));
    assert!(matches!(prop(&r, "c"), Value::Ast(_)));
    assert_eq!(prop(&r, "b"), &Value::Bool(false));
}

#[test]
fn rebuilt_element_props_use_javascript_assignment_semantics() {
    let j = parse_json(r#"{"$defs":{"Proto":{"properties":{"__proto__":{},"a":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    let p = parse("root=Proto(null,$n)\n$n=4", &l, None);
    assert!(p.root.as_ref().unwrap().props.has_own("__proto__"));
    let r = whole(evaluate_tree((&p).into(), &initialized(&p)));
    assert!(!root(&r).props.has_own("__proto__"));
    assert_eq!(prop(&r, "a"), &Value::Num(4.0));
}
