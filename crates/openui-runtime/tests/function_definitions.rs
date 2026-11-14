//! Original function definition fields, including malformed metadata.
use openui_lang::{Element, JsonValue, ParseResult, free_json, parse_json};
use openui_runtime::{LibraryFunctions, Store, TreeInput, Value, evaluate_tree};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[test]
fn definition_fields_match_the_reference() {
    let fixture = parse_json(include_str!(
        "../../../conformance/golden/function-definitions.json"
    ))
    .unwrap();
    assert_eq!(fixture.as_array().unwrap().len(), 5292);
    assert!(
        include_str!("../../../CONFORMANCE.md")
            .contains("Runtime function definitions: 5292 observations.")
    );
    let mut mismatches = Vec::new();
    for c in fixture.as_array().unwrap() {
        let mut definitions = parse_json("{}").unwrap();
        definitions["Probe"] = c.get("definition").cloned().unwrap_or(JsonValue::Null);
        let mut functions = LibraryFunctions::new(&definitions);
        let outcome = c["outcome"].as_str().unwrap();
        let result = c.get("result").map(Value::from_json);
        functions.register("Probe", move |args| {
            if outcome == "throw" {
                Err("boom".into())
            } else {
                Ok(if outcome == "args" {
                    Some(args)
                } else {
                    result.clone()
                })
            }
        });
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
        let r = evaluate_tree(TreeInput::new(&p).with_functions(&functions), &Store::new());
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
    free_json(fixture);
}
