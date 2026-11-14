//! Original action definition fields against the pinned evaluator.
use openui_lang::{Element, Library, ParseResult, free_json, parse_json};
use openui_runtime::{Store, TreeInput, Value, evaluate_tree};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[test]
fn definition_fields_match_the_reference() {
    let fixture = parse_json(include_str!(
        "../../../conformance/golden/action-definitions.json"
    ))
    .unwrap();
    let libjson = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&libjson);
    let mut mismatches = Vec::new();
    assert_eq!(fixture.as_array().unwrap().len(), 268);
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CONFORMANCE.md"))
        .unwrap();
    assert!(doc.contains("Runtime action definitions: 268 observations."));
    for c in fixture.as_array().unwrap() {
        let root = Value::from_json(&c["root"]);
        let Value::Obj(root) = &root else {
            panic!("root")
        };
        let Some(Value::Obj(props)) = root.get("props") else {
            panic!("props")
        };
        let p = ParseResult {
            root: Some(Element {
                type_name: "Box".into(),
                props: props.as_ref().clone(),
                partial: false,
                has_dynamic_props: true,
                statement_id: None,
            }),
            ..Default::default()
        };
        let r = evaluate_tree(
            TreeInput::new(&p)
                .with_library(&library)
                .with_actions(&c["actions"]),
            &Store::new(),
        );
        assert!(!r.size_limited && !r.depth_limited, "{}", c["name"]);
        let want = openui_lang::stringify_json(&c["expected"]);
        if r.to_json() != want {
            mismatches.push(format!(
                "{}\nactual: {}\nexpected: {}",
                c["name"],
                r.to_json(),
                want
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    free_json(libjson);
    free_json(fixture);
}

#[test]
fn ordinary_mapped_components_do_not_read_action_metadata() {
    let libjson = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&libjson);
    let p = openui_lang::parse(
        r#"root=Box({k:"Comp",name:"Box",mappedProps:{a:{k:"Str",v:"nested"}}})"#,
        &library,
        None,
    );
    let actions = parse_json(r#"{"Probe":{"params":null}}"#).unwrap();
    let r = evaluate_tree(
        TreeInput::new(&p)
            .with_library(&library)
            .with_actions(&actions),
        &Store::new(),
    );
    assert!(r.errors.is_empty(), "{}", r.to_json());
    assert!(r.to_json().contains(r#""props":{"a":"nested"}"#));
    free_json(actions);
    free_json(libjson);
}
