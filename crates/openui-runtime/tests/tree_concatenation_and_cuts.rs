//! Review regressions, exercised through public interfaces.
use openui_lang::{
    Element, Library, ParseResult, free_json, parse, parse_expression, parse_json, tokenize,
};
use openui_runtime::{
    Ast, JsObject, Limits, Store, TreeResult, Value, evaluate_tree, evaluate_tree_with_limits,
};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn library() -> Library {
    let j =
        parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    free_json(j);
    l
}
fn root(r: &TreeResult) -> &Element {
    match r.root.as_ref().unwrap() {
        Value::Element(e) => e,
        _ => panic!("element"),
    }
}
fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    match v {
        Value::Obj(o) => o.get(k).unwrap(),
        _ => panic!("object"),
    }
}
fn text(v: &Value) -> &str {
    match v {
        Value::Str(s) => s,
        _ => panic!("text"),
    }
}
fn json(v: &Value) -> String {
    let mut props = JsObject::new();
    props.define("v", v.clone());
    let p = ParseResult {
        root: Some(Element {
            type_name: String::new(),
            props,
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }),
        ..ParseResult::default()
    }
    .to_json();
    let s = p
        .strip_prefix("{\"root\":{\"type\":\"element\",\"typeName\":\"\",\"props\":{\"v\":")
        .unwrap();
    s[..s
        .rfind("},\"partial\":false,\"hasDynamicProps\":false},\"meta\":")
        .unwrap()]
        .to_owned()
}
#[test]
fn supplied_concatenations_throw_sites_and_descriptors_match_upstream() {
    let j = parse_json(include_str!(
        "../../../conformance/golden/runtime-interface.json"
    ))
    .unwrap();
    let cases = Value::from_json(&j);
    free_json(j);
    let Value::Arr(cases) = &cases else {
        panic!("cases")
    };
    let l = library();
    let mut mismatches = Vec::new();
    for c in cases.iter() {
        let name = text(get(c, "name"));
        let p = parse(text(get(c, "program")), &l, None);
        let mut s = Store::new();
        let Value::Obj(saved) = get(c, "saved") else {
            panic!("saved")
        };
        let initialized = s.initialize(
            p.state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            saved.iter().map(|(k, v)| (k.as_str(), Some(v))),
        );
        assert!(initialized.refused.is_empty());
        let r = evaluate_tree((&p).into(), &s);
        assert!(!r.size_limited && !r.depth_limited, "{name} cut");
        let want = json(get(c, "expected"));
        let actual = r.to_json();
        if want != actual {
            let byte = want
                .bytes()
                .zip(actual.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(want.len().min(actual.len()));
            mismatches.push(format!(
                "{name}: first differing byte {byte}; expected errors {}; actual errors {:?}",
                json(get(get(c, "expected"), "errors")),
                r.errors
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
#[test]
fn concatenation_work_grows_linearly_with_term_count() {
    let l = library();
    let mut s = Store::new();
    s.set("$a", Some(Value::Str("x".repeat(40)))).unwrap();
    for terms in [32, 64, 128, 200] {
        let p = parse(
            &format!("root=Box({})\n$a=\"\"", vec!["$a"; terms].join("+")),
            &l,
            None,
        );
        let r = evaluate_tree_with_limits(
            (&p).into(),
            &s,
            Limits::DEFAULT.with_work(2000 + terms * 200),
        );
        assert!(!r.size_limited && !r.depth_limited, "{terms} terms cut");
        assert_eq!(
            root(&r).props.get("a"),
            Some(&Value::Str("x".repeat(40 * terms)))
        );
    }
}
fn ast(s: &str) -> Ast {
    parse_expression(&tokenize(s))
}
#[test]
fn cuts_keep_fitted_props_raw_remainder_and_prior_errors() {
    let l = library();
    let p = parse(
        "root=Box($n+1,\"\"+$bad,@Sum($items),$n+2)\n$n=7\n$bad={}\n$items=[]",
        &l,
        None,
    );
    let mut s = Store::new();
    let mut bad = JsObject::new();
    bad.define("toString", Value::Null);
    s.set("$n", Some(Value::Num(7.0))).unwrap();
    s.set("$bad", Some(Value::Obj(Arc::new(bad)))).unwrap();
    s.set(
        "$items",
        Some(Value::Arr(Arc::new(vec![Value::Num(1.0); 5000]))),
    )
    .unwrap();
    let raw = &p.root.as_ref().unwrap().props;
    for limits in [
        Limits::DEFAULT.with_work(1500),
        Limits::DEFAULT.with_result_size(500),
    ] {
        let mut p = p.clone();
        if limits.result_size() == 500 {
            p.root
                .as_mut()
                .unwrap()
                .props
                .define("c", Value::Ast(Arc::new(ast("\"x\"+$items"))));
        }
        let r = evaluate_tree_with_limits((&p).into(), &s, limits);
        assert!(r.size_limited && !r.depth_limited);
        assert_eq!(root(&r).props.get("a"), Some(&Value::Num(8.0)));
        assert_eq!(r.errors.len(), 1);
        assert_eq!(root(&r).props.get("b"), raw.get("b"));
        assert_eq!(
            root(&r).props.get("c"),
            p.root.as_ref().unwrap().props.get("c")
        );
        assert_eq!(root(&r).props.get("d"), raw.get("d"));
    }
    // Keep the parse shallow, but make a state result exceed value depth.
    let mut deep = Value::Num(1.0);
    for _ in 0..100 {
        deep = Value::Arr(Arc::new(vec![deep]));
    }
    s.set("$deep", Some(deep)).unwrap();
    let p = parse(
        "root=Box($n+1,\"\"+$bad,$deep,$n+2)\n$n=7\n$bad={}\n$deep=null",
        &l,
        None,
    );
    let r = evaluate_tree_with_limits((&p).into(), &s, Limits::DEFAULT.with_depth(24));
    assert!(r.depth_limited && !r.size_limited);
    assert_eq!(root(&r).props.get("a"), Some(&Value::Num(8.0)));
    assert_eq!(r.errors.len(), 1);
    assert_eq!(
        root(&r).props.get("c"),
        p.root.as_ref().unwrap().props.get("c")
    );
    assert_eq!(
        root(&r).props.get("d"),
        p.root.as_ref().unwrap().props.get("d")
    );
}
