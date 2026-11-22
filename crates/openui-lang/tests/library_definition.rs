//! Library definitions: a plain-data library definition goes in, and the
//! component library's JSON Schema and prompt spec come out byte for byte as
//! upstream's `toJSONSchema()` and `toSpec()` give them for the zod
//! declaration it was converted from, or the error upstream's `createLibrary`
//! throws, with its message. The expected output is
//! `conformance/golden/library-definitions.json`, written by
//! `reference/library-definitions.ts` running upstream's code. The real-library
//! importer also checks the pinned react-ui schema, spec, prompt and cloud config.
//!
//! Also the parser guarantee: a defined library reaches the parser only as
//! that JSON Schema, so it parses the corpus programs written for it exactly
//! as the same library supplied as JSON, and the built-in table keeps its 20
//! names.

mod common;

use common::{Case, conformance_dir, golden_dir, load_corpus, read};
use openui_lang::{
    CallKind, Library, LibraryDefinition, LibraryDefinitionError, Limits, StreamParser,
    TypeDescription, create_library, free_json, parse, parse_json, stringify_json,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn golden() -> Vec<Value> {
    let text = read(&golden_dir().join("library-definitions.json"));
    match parse_json(&text).unwrap() {
        Value::Array(cases) => cases,
        other => panic!("expected an array, got {other}"),
    }
}

fn case(name: &str) -> Value {
    golden()
        .into_iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no golden case {name}"))
}

fn define(definition: &Value) -> Result<openui_lang::DefinedLibrary, LibraryDefinitionError> {
    create_library(LibraryDefinition::from_json(definition)?)
}

#[test]
fn every_sample_matches_upstream_byte_for_byte() {
    let cases = golden();
    assert!(cases.len() >= 10, "the golden lost its samples");
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let result = define(&case["definition"]);
        match case.get("error") {
            Some(error) => {
                let err = result
                    .err()
                    .unwrap_or_else(|| panic!("{name}: expected an error"));
                assert_eq!(err.to_string(), error.as_str().unwrap(), "{name}");
            }
            None => {
                let library = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(
                    stringify_json(&library.to_json_schema()),
                    stringify_json(&case["schema"]),
                    "{name}: JSON Schema"
                );
                assert_eq!(
                    stringify_json(&library.to_spec()),
                    stringify_json(&case["spec"]),
                    "{name}: prompt spec"
                );
            }
        }
    }
}

/// If the whole document differs, walk the ordered records to name the first
/// differing entry, including an insertion, deletion or reordering.
fn assert_real_library_bytes(actual: &Value, upstream: &Value, label: &str, records: &[&str]) {
    if stringify_json(actual) == stringify_json(upstream) {
        return;
    }
    let (Value::Object(got), Value::Object(want)) = (actual, upstream) else {
        panic!("{label}: expected an object on both sides");
    };
    let mut got = got.iter();
    let mut want = want.iter();
    loop {
        match (got.next(), want.next()) {
            (None, None) => break,
            (Some((a, value)), Some((b, expected))) => {
                assert_eq!(a, b, "{label}: first different name");
                let path = format!("{label}.{b}");
                if records.contains(&b.as_str()) {
                    assert_real_library_bytes(value, expected, &path, &[]);
                } else {
                    assert_eq!(stringify_json(value), stringify_json(expected), "{path}");
                }
            }
            (None, Some((name, _))) => panic!("{label}: missing {name}"),
            (Some((name, _)), None) => panic!("{label}: unexpected {name}"),
        }
    }
}

fn fixture(path: &str) -> Value {
    parse_json(&read(&conformance_dir().join(path))).unwrap()
}

fn real_library_bytes(
    name: &str,
    library: &openui_lang::DefinedLibrary,
    schema: &Value,
    spec: &Value,
) {
    assert_real_library_bytes(
        &library.to_json_schema(),
        schema,
        &format!("{name} schema"),
        &["properties", "$defs", "functions", "actions"],
    );
    assert_real_library_bytes(
        &library.to_spec(),
        spec,
        &format!("{name} spec"),
        &["components", "functions", "actions"],
    );
}

#[test]
fn intelligent_ui_matches_upstream_byte_for_byte() {
    let f = fixture("real-libraries/intelligent-ui.json");
    real_library_bytes(
        "intelligent-ui",
        &define(&f["definition"]).unwrap(),
        &f["schema"],
        &f["spec"],
    );
    assert_eq!(f["spec"]["components"].as_object().unwrap().len(), 52);
    assert_eq!(f["spec"]["functions"].as_object().unwrap().len(), 59);
    // The prompt the package's own library writes with its exported prompt
    // options: what the model is sent (#158).
    let library = define(&f["definition"]).unwrap();
    let prompt = library.prompt(Some(f["promptOptions"].clone())).unwrap();
    assert_eq!(prompt, f["prompt"].as_str().unwrap());
}

#[test]
fn react_ui_matches_upstream_byte_for_byte() {
    let react_ui = fixture("real-libraries/react-ui.json");
    let definition = &react_ui["definition"];
    let schema = fixture("imported/schemas/react-ui.json");
    let request = fixture("prompts/react-ui-schema.json");
    assert_eq!(definition["components"].as_array().unwrap().len(), 82);
    let library = define(definition).unwrap();
    real_library_bytes("react-ui", &library, &schema, &request["library"]);
    let package_prompt = fixture("golden/prompts/react-ui-schema.json");
    let package_cloud = fixture("golden/prompts/react-ui-cloud.json");
    for (options, prompt, cloud) in [
        (None, &react_ui["prompt"], &react_ui["cloud"]),
        (
            request.get("promptOptions"),
            &package_prompt["prompt"],
            &package_cloud["prompt"],
        ),
    ] {
        assert_eq!(
            library.prompt(options.cloned()).unwrap(),
            *prompt,
            "react-ui prompt"
        );
        let mut system_request = json!({"library": library.to_spec()});
        if let Some(options) = options {
            system_request["promptOptions"] = options.clone();
        }
        assert_eq!(
            openui_lang::generate_system_prompt(&system_request).unwrap(),
            *prompt,
            "react-ui system prompt"
        );
        assert_eq!(
            library.cloud_config(options.cloned(), None).unwrap(),
            *cloud,
            "react-ui cloud config"
        );
    }
}

/// `z.object({b, "10", "2"})` lists "2" and "10" first, as every JavaScript
/// object does; a definition listing them in written order gets the same.
#[test]
fn props_come_in_javascript_order_whatever_order_they_are_listed_in() {
    let case = case("key-order");
    let mut definition = case["definition"].clone();
    let props = definition["components"][0]["props"].as_array_mut().unwrap();
    let names: Vec<&str> = props.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["2", "10", "b", "02", "__proto__"]);
    // As written in the declaration: b, "10", "2", "02", __proto__.
    props.swap(0, 2);
    props.swap(1, 2);
    let names: Vec<&str> = props.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["b", "2", "10", "02", "__proto__"]);
    let library = define(&definition).unwrap();
    assert_eq!(
        stringify_json(&library.to_spec()),
        stringify_json(&case["spec"])
    );
}

/// A group's fields come out as name, components, notes, whatever order the
/// definition's JSON gives them in.
#[test]
fn group_fields_come_out_in_a_fixed_order() {
    let definition = json!({
        "components": [{"name": "A", "description": "a", "props": []}],
        "componentGroups": [{"notes": ["n"], "components": ["A"], "name": "G"}]
    });
    let spec = define(&definition).unwrap().to_spec();
    assert_eq!(
        stringify_json(&spec["componentGroups"]),
        r#"[{"name":"G","components":["A"],"notes":["n"]}]"#
    );
}

#[test]
fn a_root_that_is_not_a_component_is_refused_with_upstreams_message() {
    let definition = json!({
        "components": [
            {"name": "A", "description": "a", "props": []},
            {"name": "B", "description": "b", "props": []}
        ],
        "root": "C"
    });
    let err = define(&definition).err().unwrap();
    assert!(matches!(err, LibraryDefinitionError::RootNotFound { .. }));
    assert_eq!(
        err.to_string(),
        r#"[createLibrary] Root component "C" was not found in components. Available components: A, B"#
    );
}

/// A component named `__proto__` becomes the component record's prototype,
/// and upstream's root check, `!componentsRecord[root]`, then reads a field of
/// the last such component. Confirmed by running upstream's `createLibrary`:
/// `name`, `props`, `component` (a real library always has a renderer there)
/// and `ref` are always truthy; `description` only when it isn't empty.
#[test]
fn a_proto_component_answers_the_root_check_with_its_own_fields() {
    let library = |descriptions: &[&str], root: &str| {
        let mut components: Vec<Value> = descriptions
            .iter()
            .map(|d| json!({"name": "__proto__", "description": d, "props": []}))
            .collect();
        components.push(json!({"name": "A", "description": "a", "props": []}));
        define(&json!({"components": components, "root": root}))
    };
    for root in ["name", "props", "component", "ref", "description"] {
        assert!(library(&["p"], root).is_ok(), "{root}");
    }
    for root in ["name", "props", "component", "ref"] {
        assert!(library(&[""], root).is_ok(), "{root}");
        assert!(library(&[], root).is_err(), "{root} without __proto__");
    }
    for descriptions in [&[""][..], &["p", ""], &[]] {
        let err = library(descriptions, "description").err().unwrap();
        assert_eq!(
            err.to_string(),
            r#"[createLibrary] Root component "description" was not found in components. Available components: A"#,
            "{descriptions:?}"
        );
    }
    assert!(library(&["", "p"], "description").is_ok());
}

#[test]
fn a_kind_the_core_does_not_support_is_refused_naming_the_kind() {
    for kind in ["date", "literal", "tuple", "Component", ""] {
        let definition = json!({"components": [{
            "name": "Card", "description": "",
            "props": [{"name": "ok", "type": {"kind": "string"}}, {"name": "tone", "type": {"kind": kind}}]
        }]});
        let err = LibraryDefinition::from_json(&definition).err().unwrap();
        match &err {
            LibraryDefinitionError::UnsupportedKind { kind: named, path } => {
                assert_eq!(named, kind);
                assert_eq!(path, "components[0].props[1].type");
            }
            other => panic!("{kind}: expected UnsupportedKind, got {other:?}"),
        }
        assert!(err.to_string().contains(&format!("\"{kind}\"")), "{err}");
    }
}

#[test]
fn malformed_definitions_are_refused_with_where_and_why() {
    let cases = [
        (json!([]), "", "expected an object"),
        (json!({}), "components", "missing"),
        (json!({"components": {}}), "components", "expected an array"),
        (
            json!({"components": [{"name": "A", "props": []}]}),
            "components[0].description",
            "missing",
        ),
        (
            json!({"components": [{"name": 1, "description": "", "props": []}]}),
            "components[0].name",
            "expected a string",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [{"name": "x"}]}]}),
            "components[0].props[0].type",
            "missing",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [{"name": "x", "type": {}}]}]}),
            "components[0].props[0].type.kind",
            "missing",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "string"}, "optional": "yes"}
            ]}]}),
            "components[0].props[0].optional",
            "expected a boolean",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "string"}, "optinal": true}
            ]}]}),
            "components[0].props[0].optinal",
            "unknown field",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "string", "values": ["a"]}}
            ]}]}),
            "components[0].props[0].type.values",
            "unknown field",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "string"}},
                {"name": "x", "type": {"kind": "number"}}
            ]}]}),
            "components[0].props[1].name",
            "duplicate prop \"x\"",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "object", "fields": [
                    {"name": "x", "type": {"kind": "string"}},
                    {"name": "x", "type": {"kind": "number"}, "optional": true}
                ]}}
            ]}]}),
            "components[0].props[0].type.fields[1].name",
            "duplicate field \"x\"",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "enum", "values": ["a", 1]}}
            ]}]}),
            "components[0].props[0].type.values[1]",
            "expected a string",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "array", "items": {"kind": "string"}, "minItems": "2"}}
            ]}]}),
            "components[0].props[0].type.minItems",
            "expected a number",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "unknown", "minItems": 2}}
            ]}]}),
            "components[0].props[0].type.minItems",
            "unknown field",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "array"}}
            ]}]}),
            "components[0].props[0].type.items",
            "missing",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "array", "items": {"kind": "string"}, "values": {"kind": "string"}}}
            ]}]}),
            "components[0].props[0].type.values",
            "unknown field",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "union", "options": {"kind": "string"}}}
            ]}]}),
            "components[0].props[0].type.options",
            "expected an array",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "union", "options": [{"kind": "string"}, {"kind": "nullable"}]}}
            ]}]}),
            "components[0].props[0].type.options[1].type",
            "missing",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "record", "values": {"kind": "object", "fields": [
                    {"name": "f", "type": {"kind": "string"}, "nullable": true}
                ]}}}
            ]}]}),
            "components[0].props[0].type.values.fields[0].nullable",
            "unknown field",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "component"}}
            ]}]}),
            "components[0].props[0].type.name",
            "missing",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "action", "name": 1}}
            ]}]}),
            "components[0].props[0].type.name",
            "expected a string",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "action", "only": ["OpenUrl"]}}
            ]}]}),
            "components[0].props[0].type.only",
            "unknown field",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "named", "name": "Color"}}
            ]}]}),
            "components[0].props[0].type.type",
            "missing",
        ),
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "string"}, "binding": "yes"}
            ]}]}),
            "components[0].props[0].binding",
            "expected a boolean",
        ),
        // A binding marks a prop's outermost type only: a field has none.
        (
            json!({"components": [{"name": "A", "description": "", "props": [
                {"name": "x", "type": {"kind": "object", "fields": [
                    {"name": "f", "type": {"kind": "string"}, "binding": true}
                ]}}
            ]}]}),
            "components[0].props[0].type.fields[0].binding",
            "unknown field",
        ),
        (
            json!({"components": [], "root": null}),
            "root",
            "expected a string",
        ),
        (
            json!({"components": [], "componentGroups": [{"name": "G"}]}),
            "componentGroups[0].components",
            "missing",
        ),
        (
            json!({"components": [], "componentGroups": [{"name": "G", "components": [], "hint": "x"}]}),
            "componentGroups[0].hint",
            "unknown field",
        ),
        (
            json!({"components": [], "componentGroups": [{"name": "G", "components": [1]}]}),
            "componentGroups[0].components[0]",
            "expected a string",
        ),
    ];
    for (definition, want_path, want_problem) in cases {
        match LibraryDefinition::from_json(&definition) {
            Err(LibraryDefinitionError::Invalid { path, problem }) => {
                assert_eq!(path, want_path, "{definition}");
                assert_eq!(problem, want_problem, "{definition}");
            }
            other => panic!("{definition}: expected Invalid, got {other:?}"),
        }
    }
}

/// A reference to a name that is no component of the library, or an action
/// slot naming an action a slot can't name, is refused with where and what,
/// wherever the reference sits.
#[test]
fn a_reference_to_a_name_that_is_not_a_component_is_refused() {
    let with_type = |ty: Value| {
        let mut prop = json!({"name": "x"});
        prop["type"] = ty;
        json!({"components": [
            {"name": "Item", "description": "", "props": []},
            {"name": "List", "description": "", "props": [
                {"name": "ok", "type": {"kind": "component", "name": "Item"}},
                prop
            ]}
        ]})
    };
    let cases = [
        (
            json!({"kind": "component", "name": "Missing"}),
            "components[1].props[1].type",
        ),
        (
            json!({"kind": "array", "items": {"kind": "union", "options": [
                {"kind": "string"},
                {"kind": "nullable", "type": {"kind": "component", "name": "item"}}
            ]}}),
            "components[1].props[1].type.items.options[1].type",
        ),
        (
            json!({"kind": "object", "fields": [{"name": "f", "type": {"kind": "record", "values":
                {"kind": "named", "name": "N", "type": {"kind": "array", "items": {"kind": "component", "name": "toString"}}}
            }}]}),
            "components[1].props[1].type.fields[0].type.values.type.items",
        ),
    ];
    for (ty, want_path) in cases {
        let err = define(&with_type(ty)).err().unwrap();
        match &err {
            LibraryDefinitionError::UnknownComponent { path, name } => {
                assert_eq!(path, want_path);
                assert!(
                    err.to_string().ends_with(&format!(
                        "{path}: \"{name}\" is not a component of the library"
                    )),
                    "{err}"
                );
            }
            other => panic!("expected UnknownComponent, got {other:?}"),
        }
    }
    assert_eq!(
        define(&with_type(json!({"kind": "component", "name": "Missing"})))
            .err()
            .unwrap()
            .to_string(),
        r#"library definition: components[1].props[1].type: "Missing" is not a component of the library"#
    );

    for name in ["ActionExpression", "Run", "Set", "openUrl", ""] {
        let err = define(&with_type(json!({"kind": "action", "name": name})))
            .err()
            .unwrap();
        assert!(
            matches!(&err, LibraryDefinitionError::UnknownAction { path, name: named }
                if path == "components[1].props[1].type" && named == name),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!(
                "library definition: components[1].props[1].type: \"{name}\" is not an action a slot can name (the built-in ones are OpenUrl and ToAssistant, and the library's own actions)"
            )
        );
    }

    // The component with the empty name exists, but a reference to it is not
    // supported: upstream writes its schema inline at each reference, or
    // never finishes when it refers to itself.
    let mut definition = with_type(json!({"kind": "component", "name": ""}));
    definition["components"][0]["name"] = json!("");
    definition["components"][1]["props"][0]["type"]["name"] = json!("List");
    match define(&definition).err().unwrap() {
        LibraryDefinitionError::Invalid { path, problem } => {
            assert_eq!(path, "components[1].props[1].type");
            assert_eq!(
                problem,
                "a reference to the component with the empty name is not supported"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }

    // A named type names a data type.
    for inner in [
        json!({"kind": "component", "name": "Item"}),
        json!({"kind": "action"}),
        json!({"kind": "named", "name": "M", "type": {"kind": "string"}}),
    ] {
        let mut ty = json!({"kind": "named", "name": "N"});
        ty["type"] = inner;
        match define(&with_type(ty)).err().unwrap() {
            LibraryDefinitionError::Invalid { path, problem } => {
                assert_eq!(path, "components[1].props[1].type.type");
                assert!(
                    problem.starts_with("a named type names a data type"),
                    "{problem}"
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }
}

/// A binding changes the signature only: the schema, and so the parser's
/// library, are the unmarked type's.
#[test]
fn a_binding_changes_the_signature_and_not_the_schema() {
    let marked = case("binding")["definition"].clone();
    let mut unmarked = marked.clone();
    for prop in unmarked["components"][0]["props"].as_array_mut().unwrap() {
        prop.as_object_mut().unwrap().remove("binding");
    }
    assert_ne!(marked, unmarked);
    let (marked, unmarked) = (define(&marked).unwrap(), define(&unmarked).unwrap());
    assert_eq!(marked.to_json_schema(), unmarked.to_json_schema());
    assert_eq!(marked.library(), unmarked.library());
    let signature = |library: &openui_lang::DefinedLibrary| {
        library.to_spec()["components"]["Input"]["signature"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(
        signature(&marked).starts_with("Input(value: $binding<string>, maybe?: $binding<string>,")
    );
    assert!(signature(&unmarked).starts_with("Input(value: string, maybe?: string,"));
}

/// A library whose components refer to each other is a graph: a chain as
/// long as the library, closed into a cycle, is written visiting each
/// component once, with no recursion through the references, on a 64 KiB
/// stack. Its `$defs` come in the depth-first order, so a chain declared
/// backwards comes out forwards.
#[test]
fn a_long_reference_chain_with_a_cycle_is_written_in_one_pass() {
    const COMPONENTS: usize = 2_000;
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(|| {
            // C0 refers to C1, ..., the last back to C0; declared last first.
            let components: Vec<Value> = (0..COMPONENTS)
                .rev()
                .map(|i| {
                    let next = format!("C{}", (i + 1) % COMPONENTS);
                    json!({"name": format!("C{i}"), "description": "", "props": [
                        {"name": "next", "type": {"kind": "component", "name": next}},
                        {"name": "self", "type": {"kind": "array", "items": {"kind": "component", "name": format!("C{i}")}}, "optional": true}
                    ]})
                })
                .collect();
            let mut definition = json!({});
            definition["components"] = Value::Array(components);
            let library = define(&definition).unwrap();
            let schema = library.to_json_schema();
            let defs: Vec<&str> = schema["$defs"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(defs.len(), COMPONENTS);
            // The walk starts at the first declared, C19999, whose `next` is
            // C0, and goes on from there.
            assert_eq!(defs[0], format!("C{}", COMPONENTS - 1));
            for (i, name) in defs.iter().skip(1).enumerate() {
                assert_eq!(*name, format!("C{i}"));
            }
            assert_eq!(
                stringify_json(&schema["$defs"]["C5"]["properties"]),
                r##"{"next":{"$ref":"#/$defs/C6"},"self":{"type":"array","items":{"$ref":"#/$defs/C5"}}}"##
            );
            let spec = library.to_spec();
            assert_eq!(
                spec["components"]["C5"]["signature"],
                "C5(next: C6, self?: C5[])"
            );
            let _ = library.library();
            free_json(schema);
            free_json(spec);
            free_json(definition);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// `z.enum` makes its words an object's keys, so they come in JavaScript
/// property order, each once; a definition listing them as written, with a
/// repeat, gets the same.
#[test]
fn enum_words_come_in_javascript_order_each_once() {
    let case = case("enum-key-order");
    let mut definition = case["definition"].clone();
    definition["components"][0]["props"][0]["type"]["values"] = json!([
        "b",
        "10",
        "2",
        "__proto__",
        "01",
        "-0",
        "4294967295",
        "4294967294",
        "b",
        "2"
    ]);
    definition["components"][0]["props"][1]["type"]["values"] = json!(["a", "b", "a"]);
    let library = define(&definition).unwrap();
    assert_eq!(
        stringify_json(&library.to_spec()),
        stringify_json(&case["spec"])
    );
}

/// An inline object's fields follow JavaScript key order, as props do.
#[test]
fn object_fields_come_in_javascript_order_whatever_order_they_are_listed_in() {
    let case = case("object-key-order");
    let mut definition = case["definition"].clone();
    let fields = definition["components"][0]["props"][0]["type"]["fields"]
        .as_array_mut()
        .unwrap();
    // As written in the declaration: b, "10", "2", __proto__, a"b, "".
    fields.swap(0, 2);
    fields.swap(1, 2);
    let names: Vec<&str> = fields.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["b", "2", "10", "__proto__", "a\"b", ""]);
    let library = define(&definition).unwrap();
    assert_eq!(
        stringify_json(&library.to_spec()),
        stringify_json(&case["spec"])
    );
}

/// Every sample's defined library is the library the parser compiles from
/// upstream's own JSON Schema for it.
#[test]
fn every_sample_compiles_to_the_library_of_upstreams_schema() {
    for case in golden().iter().filter(|c| c.get("schema").is_some()) {
        let name = case["name"].as_str().unwrap();
        let defined = define(&case["definition"]).unwrap().library();
        assert_eq!(
            defined,
            Library::from_json_schema(&case["schema"]),
            "{name}"
        );
    }
}

/// A component whose one prop's type nests `depth` levels: `wrap` puts a
/// type inside another, around a string. Built with a loop, moving each type
/// into the next (`json!` would copy it, recursively).
fn nested(depth: usize, wrap: fn(Value) -> Value) -> Value {
    let mut ty = json!({"kind": "string"});
    for _ in 1..depth {
        ty = wrap(ty);
    }
    let mut prop = json!({"name": "v"});
    prop["type"] = ty;
    let mut component = json!({"name": "Deep", "description": ""});
    component["props"] = Value::Array(vec![prop]);
    let mut definition = json!({});
    definition["components"] = Value::Array(vec![component]);
    definition
}

/// `{"kind": kind, field: inner}`, moving `inner` in.
fn holding(kind: &str, field: &str, inner: Value) -> Value {
    let mut ty = json!({"kind": kind});
    ty[field] = inner;
    ty
}

fn in_array(ty: Value) -> Value {
    holding("array", "items", ty)
}

fn in_object(ty: Value) -> Value {
    let mut field = json!({"name": "f", "optional": true});
    field["type"] = ty;
    holding("object", "fields", Value::Array(vec![field]))
}

fn in_union(ty: Value) -> Value {
    holding(
        "union",
        "options",
        Value::Array(vec![json!({"kind": "number"}), ty]),
    )
}

fn in_record(ty: Value) -> Value {
    holding("record", "values", ty)
}

fn in_nullable(ty: Value) -> Value {
    holding("nullable", "type", ty)
}

/// A named type, or a nullable around one: a named type names no named type.
fn in_named(ty: Value) -> Value {
    if ty["kind"] == "named" {
        return in_nullable(ty);
    }
    let mut named = holding("named", "type", ty);
    named["name"] = json!("N");
    named
}

/// The action slot at the deepest level, under `depth - 1` arrays, with a
/// component that refers to the one holding it: its inline shape (a
/// component has the slot's name) nests further than a string would.
fn deep_action(depth: usize) -> Value {
    let mut definition = nested(depth, in_array);
    let mut ty = &mut definition["components"][0]["props"][0]["type"];
    while ty["kind"] == "array" {
        ty = &mut ty["items"];
    }
    *ty = json!({"kind": "action"});
    definition["components"].as_array_mut().unwrap().push(
        json!({"name": "ActionExpression", "description": "", "props": [
            {"name": "deep", "type": {"kind": "component", "name": "Deep"}}
        ]}),
    );
    definition
}

/// A function returning a type `depth` levels deep (arrays around a
/// string), and a library action whose one param is as deep, referred to by
/// a component so that its `$defs` entry, which nests further than a
/// component's, is written.
fn deep_calls(depth: usize) -> Value {
    // Each type built on its own: serde_json's clone recurses, which a type
    // at the limit outgrows at opt-level 0.
    let deep_type = || nested(depth, in_array)["components"][0]["props"][0]["type"].take();
    let mut definition = json!({
        "components": [{"name": "Deep", "description": "", "props": [
            {"name": "go", "type": {"kind": "action", "name": "Go"}}
        ]}],
        "functions": [{"name": "F", "description": "", "params": []}],
        "actions": [{"name": "Go", "description": "", "params": [{"name": "p"}]}]
    });
    definition["functions"][0]["returns"] = deep_type();
    definition["actions"][0]["params"][0]["type"] = deep_type();
    definition
}

fn depth_of(ty: &TypeDescription) -> usize {
    let mut depth = 1;
    let mut ty = ty;
    while let TypeDescription::Array(inner) = ty {
        depth += 1;
        ty = inner;
    }
    depth
}

/// Types nest as deep as `Limits::depth` (256 by default), a prop's type
/// being the first level, and one level more is refused with the depth
/// error, naming the first type past the limit: reading never recurses past
/// the limit, whatever the input's depth. A definition at the limit goes all
/// the way through (its text reads back with `parse_json`, its schema and
/// spec are written, the parser compiles the schema) on a 512 KiB thread,
/// growing the stack on demand.
#[test]
fn types_nest_to_the_depth_limit_and_no_further() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let limit = Limits::default().depth();
            assert_eq!(limit, 256);
            for wrap in [
                in_array,
                in_object,
                in_union,
                in_record,
                in_nullable,
                in_named,
            ] {
                let at_limit = nested(limit, wrap);
                // The deepest form still reads back from its text.
                let text = stringify_json(&at_limit);
                let reread = parse_json(&text).unwrap();
                let library = define(&reread).unwrap();
                let schema = library.to_json_schema();
                let spec = library.to_spec();
                let signature = spec["components"]["Deep"]["signature"].as_str().unwrap();
                assert!(signature.starts_with("Deep(v"), "{signature}");
                let _ = library.library();
                let schema_text = stringify_json(&schema);
                let schema_reread = parse_json(&schema_text).unwrap();
                assert_eq!(
                    stringify_json(&schema_reread),
                    schema_text,
                    "the schema reads back"
                );
                openui_lang::generate_prompt_with_limits(&spec, Limits::default())
                    .expect("the spec is within the prompt's depth");
                for json in [schema, spec, schema_reread, reread, at_limit] {
                    free_json(json);
                }

                let past = nested(limit + 1, wrap);
                let refused = define(&past).err().unwrap();
                free_json(past);
                match refused {
                    LibraryDefinitionError::DepthLimited { path, limit: named } => {
                        assert_eq!(named, limit);
                        assert!(path.starts_with("components[0].props[0].type."), "{path}");
                    }
                    other => panic!("expected DepthLimited, got {other:?}"),
                }
            }
            // A call's types at the limit, and a library action's entry,
            // fit the prompt's depth, and one level more is refused.
            let library = freeing(deep_calls(limit), define).unwrap();
            let spec = library.to_spec();
            assert!(spec["schema"]["$defs"]["Go"].is_object());
            openui_lang::generate_prompt_with_limits(&spec, Limits::default())
                .expect("the spec is within the prompt's depth");
            let _ = library.library();
            free_json(spec);
            assert!(matches!(
                freeing(deep_calls(limit + 1), define),
                Err(LibraryDefinitionError::DepthLimited { .. })
            ));
            // The action slot written inline at the limit fits the prompt's
            // depth too.
            let library = freeing(deep_action(limit), define).unwrap();
            let spec = library.to_spec();
            openui_lang::generate_prompt_with_limits(&spec, Limits::default())
                .expect("the spec is within the prompt's depth");
            let _ = library.library();
            free_json(spec);
            let at_limit = freeing(nested(limit, in_array), LibraryDefinition::from_json).unwrap();
            assert_eq!(depth_of(&at_limit.components[0].props[0].ty), limit);
            let err = freeing(nested(limit + 1, in_array), LibraryDefinition::from_json)
                .err()
                .unwrap();
            let path = format!("components[0].props[0].type{}", ".items".repeat(limit));
            assert_eq!(
                err.to_string(),
                format!("library definition: {path}: types nest deeper than 256 levels")
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// `read(&json)`, then `json` freed with a loop: serde_json drops a value by
/// recursing, which a deep test input outgrows at opt-level 0.
fn freeing<T>(json: Value, read: impl FnOnce(&Value) -> T) -> T {
    let result = read(&json);
    free_json(json);
    result
}

/// The crate's smallest stack in the depth suite.
const SMALL_STACK: usize = 64 * 1024;

/// On a 64 KiB thread, a definition nested to the default limit in each
/// recursive kind reads, builds its schema, spec and parser library, and
/// drops all of it: compiling the deep schema (whose clones and drops
/// recurse) runs on a stack segment sized for the definition's depth.
///
/// What is handed back is freed here with `free_json`, and the `Library`
/// frees itself with a loop (#140), so this passes at opt-level 0 too, as
/// `scripts/check.sh` runs it.
#[test]
fn a_definition_at_the_limit_builds_and_drops_on_a_64_kib_stack() {
    let limit = Limits::default().depth();
    let wraps: [fn(usize) -> Value; 8] = [
        |depth| nested(depth, in_array),
        |depth| nested(depth, in_object),
        |depth| nested(depth, in_union),
        |depth| nested(depth, in_record),
        |depth| nested(depth, in_nullable),
        |depth| nested(depth, in_named),
        deep_action,
        deep_calls,
    ];
    for make in wraps {
        let input = make(limit);
        std::thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn(move || {
                let definition = LibraryDefinition::from_json(&input).unwrap();
                free_json(input);
                let library = create_library(definition).unwrap();
                let parser_library = library.library();
                assert!(parser_library.component("Deep").is_some());
                drop(parser_library);
                let schema = library.to_json_schema();
                let spec = library.to_spec();
                // serde_json drops a value by recursing into it; a caller on
                // a small stack frees deep ones with `free_json`.
                free_json(schema);
                free_json(spec);
                drop(library);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

/// The limit is the one in `Limits`, raised or lowered like any other.
#[test]
fn the_type_depth_limit_is_configurable() {
    let limits = Limits::default().with_depth(10);
    for wrap in [in_array, in_object, in_union] {
        assert!(LibraryDefinition::from_json_with_limits(&nested(10, wrap), limits).is_ok());
        assert!(matches!(
            LibraryDefinition::from_json_with_limits(&nested(11, wrap), limits),
            Err(LibraryDefinitionError::DepthLimited { limit: 10, .. })
        ));
    }
    let raised = Limits::default().with_depth(300);
    assert!(LibraryDefinition::from_json_with_limits(&nested(300, in_array), raised).is_ok());
    // A depth past a field of an object counts the object's level.
    let flat = json!({"components": [{"name": "A", "description": "", "props": [
        {"name": "a", "type": {"kind": "string"}}
    ]}]});
    assert!(
        LibraryDefinition::from_json_with_limits(&flat, Limits::default().with_depth(1)).is_ok()
    );
    assert!(matches!(
        LibraryDefinition::from_json_with_limits(&flat, Limits::default().with_depth(0)),
        Err(LibraryDefinitionError::DepthLimited { limit: 0, .. })
    ));
}

/// Input far deeper than the limit is refused at the limit, without a panic
/// or a stack overflow, on a small stack.
#[test]
fn a_definition_far_past_the_limit_is_refused_not_a_crash() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            for wrap in [in_array, in_object] {
                let deep = nested(200_000, wrap);
                let err = LibraryDefinition::from_json(&deep).err().unwrap();
                assert!(
                    matches!(err, LibraryDefinitionError::DepthLimited { limit: 256, .. }),
                    "{err:?}"
                );
                // serde_json drops a value by recursing into it.
                free_json(deep);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A default is literal JSON. Both building the schema and freeing the
/// definition must handle that value's own depth on a binding-sized stack.
#[test]
fn nested_json_default_is_copied_and_freed_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut value = Value::Null;
            for _ in 0..2_000 {
                value = Value::Array(vec![value]);
            }
            let mut definition = nested(1, in_array);
            definition["components"][0]["props"][0]["default"] = value;
            let defined = define(&definition).unwrap();
            let schema = defined.to_json_schema();
            let mut default = &schema["$defs"]["Deep"]["properties"]["v"]["default"];
            for _ in 0..2_000 {
                default = default.get(0).unwrap();
            }
            assert!(default.is_null());
            free_json(schema);
            free_json(definition);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// The corpus library for `conformance/cases/library-definition/` is the
/// "scalars" sample's upstream JSON Schema.
#[test]
fn the_corpus_library_is_the_samples_upstream_schema() {
    let file = parse_json(&read(
        &conformance_dir().join("schemas/library-definition.json"),
    ))
    .unwrap();
    assert_eq!(
        stringify_json(&file),
        stringify_json(&case("scalars")["schema"])
    );
}

#[test]
fn a_defined_library_parses_the_corpus_as_the_same_library_as_json() {
    let defined = define(&case("scalars")["definition"]).unwrap();
    let from_json = Library::from_json_schema(
        &parse_json(&read(
            &conformance_dir().join("schemas/library-definition.json"),
        ))
        .unwrap(),
    );
    let corpus = load_corpus();
    let cases: Vec<_> = corpus
        .cases
        .iter()
        .filter(|c| c.name.starts_with("library-definition/"))
        .collect();
    assert_eq!(cases.len(), 3);
    assert_parse_equivalence(defined.library(), from_json, cases);
}

#[test]
fn intelligent_ui_parses_the_whole_corpus_as_json() {
    let f = fixture("real-libraries/intelligent-ui.json");
    let defined = define(&f["definition"]).unwrap().library();
    let from_json = Library::from_json_schema(&f["schema"]);
    assert_parse_equivalence(defined, from_json, &load_corpus().cases);
}

#[test]
fn react_ui_parses_the_whole_corpus_as_json() {
    let defined = define(&fixture("real-libraries/react-ui.json")["definition"])
        .unwrap()
        .library();
    let from_json = Library::from_json_schema(&fixture("imported/schemas/react-ui.json"));
    let cases = load_corpus().cases;
    assert_eq!(cases.len(), 976);
    assert_parse_equivalence(defined, from_json, &cases);
}

fn assert_parse_equivalence<'a>(
    defined: Library,
    from_json: Library,
    cases: impl IntoIterator<Item = &'a Case>,
) {
    assert_eq!(defined, from_json);
    let defined = Arc::new(defined);
    let from_json = Arc::new(from_json);
    for c in cases {
        let root = c.root.as_deref();
        assert_eq!(
            parse(&c.input, &defined, root).to_json(),
            parse(&c.input, &from_json, root).to_json(),
            "{}",
            c.name
        );
        let mut a = StreamParser::new(defined.clone(), root);
        let mut b = StreamParser::new(from_json.clone(), root);
        let chars: Vec<char> = c.input.chars().collect();
        for chunk in chars.chunks(5) {
            let chunk: String = chunk.iter().collect();
            assert_eq!(
                a.push(&chunk).to_json(),
                b.push(&chunk).to_json(),
                "{}",
                c.name
            );
        }
        assert_eq!(a.finish().to_json(), b.finish().to_json(), "{}", c.name);
    }
}

/// A library can't add to the 20 built-ins: they decide what parses as a
/// call, and a library definition must not change how programs parse.
#[test]
fn the_built_in_table_keeps_its_20_names() {
    let defined = define(&case("scalars")["definition"]).unwrap().library();
    for library in [Library::default(), defined] {
        let builtins: Vec<&str> = library
            .calls()
            .filter(|(_, call)| call.builtin)
            .map(|(name, _)| name)
            .collect();
        assert_eq!(builtins.len(), 20, "{builtins:?}");
    }
}

/// The message upstream's `createLibrary` throws for a name collision.
fn collision_message(kind: &str, name: &str) -> String {
    format!(
        "[createLibrary] {kind} \"{name}\" collides with a built-in, component, function, or action of the same name. Rename it."
    )
}

/// A library of one component, `Item`, with functions and actions of these
/// names (each with no params), in order.
fn named_calls(functions: &[&str], actions: &[&str]) -> Value {
    let call = |name: &&str| json!({"name": name, "description": "", "params": []});
    json!({
        "components": [{"name": "Item", "description": "i", "props": []}],
        "functions": functions.iter().map(call).collect::<Vec<_>>(),
        "actions": actions.iter().map(call).collect::<Vec<_>>(),
    })
}

/// The collision refused for this library, checking its kind and name.
fn assert_collision(definition: &Value, kind: CallKind, name: &str) {
    let err = define(definition).err().unwrap();
    assert_eq!(
        err,
        LibraryDefinitionError::NameCollision {
            kind,
            name: name.to_owned()
        }
    );
    let kind = match kind {
        CallKind::Function => "Function",
        CallKind::Action => "Action",
    };
    assert_eq!(err.to_string(), collision_message(kind, name));
}

/// Each collision sample of the golden is upstream's own refusal, and the
/// core gives the same message for its definition.
#[test]
fn every_collision_sample_is_upstreams_message() {
    let collisions: Vec<Value> = golden()
        .into_iter()
        .filter(|c| c["name"].as_str().unwrap().contains("-collides-with-"))
        .collect();
    assert_eq!(collisions.len(), 16);
    for case in collisions {
        let error = case["error"].as_str().unwrap();
        assert!(error.contains("collides with a built-in"), "{error}");
        assert_eq!(
            define(&case["definition"]).err().unwrap().to_string(),
            error
        );
    }
}

/// A function named as one of the 20 built-ins (functions, steps and
/// `Action`) is refused, as is an action.
#[test]
fn a_call_named_as_a_built_in_is_refused() {
    for name in [
        "Count",
        "Each",
        "Run",
        "OpenUrl",
        "ToAssistant",
        "Set",
        "Action",
    ] {
        assert_collision(&named_calls(&[name], &[]), CallKind::Function, name);
        assert_collision(&named_calls(&[], &[name]), CallKind::Action, name);
    }
}

/// `Query` and `Mutation` are reserved, and so, as upstream checks `name in
/// RESERVED_CALLS`, is every name a JavaScript object inherits.
#[test]
fn a_call_named_as_a_reserved_call_is_refused() {
    for name in [
        "Query",
        "Mutation",
        "toString",
        "constructor",
        "__proto__",
        "valueOf",
    ] {
        assert_collision(&named_calls(&[name], &[]), CallKind::Function, name);
        assert_collision(&named_calls(&[], &[name]), CallKind::Action, name);
    }
    // Close to a reserved name is not one.
    assert!(define(&named_calls(&["query", "Queries"], &["mutation"])).is_ok());
}

/// A component's name is taken: a function or action of that name is
/// refused. A `__proto__` component is not in the record, so it takes no
/// name (its name is a reserved call anyway).
#[test]
fn a_call_named_as_a_component_is_refused() {
    assert_collision(&named_calls(&["Item"], &[]), CallKind::Function, "Item");
    assert_collision(&named_calls(&[], &["Item"]), CallKind::Action, "Item");
    let mut definition = named_calls(&["Proto"], &[]);
    definition["components"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "__proto__", "description": "", "props": []}));
    assert!(define(&definition).is_ok());
}

/// A function's or action's name is taken by an earlier function or action:
/// the later one is refused, functions checked before actions.
#[test]
fn a_call_named_as_another_function_or_action_is_refused() {
    assert_collision(&named_calls(&["F", "F"], &[]), CallKind::Function, "F");
    assert_collision(&named_calls(&["F"], &["F"]), CallKind::Action, "F");
    assert_collision(&named_calls(&[], &["F", "F"]), CallKind::Action, "F");
    // The first collision is the one reported, functions first.
    assert_collision(
        &named_calls(&["G", "Count"], &["Item"]),
        CallKind::Function,
        "Count",
    );
    // The root is checked before any name.
    let mut definition = named_calls(&["Item"], &[]);
    definition["root"] = json!("Nope");
    assert!(matches!(
        define(&definition),
        Err(LibraryDefinitionError::RootNotFound { .. })
    ));
}

/// Upstream registers a library action named `ActionExpression` under the
/// built-in action slot's id, and zod throws when a library refers to both;
/// the core refuses the name. A function may have it (no function is
/// registered), and a reference to the library action with the empty name is
/// refused (upstream writes that one inline).
#[test]
fn an_action_named_as_the_action_slot_or_a_reference_to_the_empty_named_action_is_refused() {
    match define(&named_calls(&[], &["ActionExpression"]))
        .err()
        .unwrap()
    {
        LibraryDefinitionError::Invalid { path, problem } => {
            assert_eq!(path, "actions[0].name");
            assert!(problem.contains("ActionExpression"), "{problem}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert!(define(&named_calls(&["ActionExpression"], &[])).is_ok());

    let mut definition = named_calls(&[], &[""]);
    definition["components"][0]["props"] =
        json!([{"name": "a", "type": {"kind": "action", "name": ""}}]);
    match define(&definition).err().unwrap() {
        LibraryDefinitionError::Invalid { path, problem } => {
            assert_eq!(path, "components[0].props[0].type");
            assert_eq!(
                problem,
                "a reference to the action with the empty name is not supported"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    // Unreferenced, the empty name is a name like any other.
    assert!(define(&named_calls(&[""], &[""])).is_err());
    assert!(define(&named_calls(&[], &[""])).is_ok());
}

/// A reference in a call's params or return type, or in an action's params,
/// is checked as a component's: wherever it sits, the path says where.
#[test]
fn references_in_calls_are_checked() {
    let mut definition = named_calls(&["F"], &["Copy"]);
    definition["functions"][0]["returns"] = json!({"kind": "component", "name": "Missing"});
    assert!(matches!(
        define(&definition).err().unwrap(),
        LibraryDefinitionError::UnknownComponent { path, .. } if path == "functions[0].returns"
    ));
    let mut definition = named_calls(&["F"], &["Copy"]);
    definition["functions"][0]["params"] = json!([{"name": "a", "type": {"kind": "array", "items": {"kind": "action", "name": "Paste"}}}]);
    assert!(matches!(
        define(&definition).err().unwrap(),
        LibraryDefinitionError::UnknownAction { path, name }
            if path == "functions[0].params[0].type.items" && name == "Paste"
    ));
    let mut definition = named_calls(&[], &["Copy"]);
    definition["actions"][0]["params"] =
        json!([{"name": "i", "type": {"kind": "component", "name": "Nope"}}]);
    assert!(matches!(
        define(&definition).err().unwrap(),
        LibraryDefinitionError::UnknownComponent { path, .. } if path == "actions[0].params[0].type"
    ));
    // A slot may name a library action wherever it sits.
    definition["actions"][0]["params"] =
        json!([{"name": "next", "type": {"kind": "action", "name": "Copy"}, "optional": true}]);
    assert!(define(&definition).is_ok());
}

/// A call's params are an object's fields: no `binding`, each name once; a
/// function's `returns` is optional; anything else is refused with where.
#[test]
fn malformed_calls_are_refused_with_where_and_why() {
    let cases = [
        (
            json!({"name": "F", "description": "", "params": [], "fn": 1}),
            "functions[0].fn",
            "unknown field",
        ),
        (
            json!({"name": "F", "params": []}),
            "functions[0].description",
            "missing",
        ),
        (
            json!({"name": "F", "description": ""}),
            "functions[0].params",
            "missing",
        ),
        (
            json!({"name": "F", "description": "", "params": [
                {"name": "v", "type": {"kind": "string"}, "binding": true}
            ]}),
            "functions[0].params[0].binding",
            "unknown field",
        ),
        (
            json!({"name": "F", "description": "", "params": [
                {"name": "v", "type": {"kind": "string"}},
                {"name": "v", "type": {"kind": "number"}}
            ]}),
            "functions[0].params[1].name",
            "duplicate param \"v\"",
        ),
        (
            json!({"name": "F", "description": "", "params": [], "returns": "string"}),
            "functions[0].returns",
            "expected an object",
        ),
    ];
    for (function, want_path, want_problem) in cases {
        let mut definition = json!({"components": []});
        definition["functions"] = json!([function]);
        match LibraryDefinition::from_json(&definition).err().unwrap() {
            LibraryDefinitionError::Invalid { path, problem } => {
                assert_eq!(path, want_path);
                assert_eq!(problem, want_problem);
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }
    let action = json!({"components": [], "actions": [
        {"name": "G", "description": "", "params": [], "returns": {"kind": "string"}}
    ]});
    assert!(matches!(
        LibraryDefinition::from_json(&action).err().unwrap(),
        LibraryDefinitionError::Invalid { path, .. } if path == "actions[0].returns"
    ));
    let unsupported = json!({"components": [], "functions": [
        {"name": "F", "description": "", "params": [], "returns": {"kind": "date"}}
    ]});
    assert!(matches!(
        LibraryDefinition::from_json(&unsupported).err().unwrap(),
        LibraryDefinitionError::UnsupportedKind { path, .. } if path == "functions[0].returns"
    ));
}

/// A call's params and return type nest as deep as a prop's type, and no
/// deeper.
#[test]
fn call_types_nest_to_the_depth_limit_and_no_further() {
    let limits = Limits::default().with_depth(10);
    let deep = |depth: usize| {
        let component = nested(depth, in_array);
        component["components"][0]["props"][0]["type"].clone()
    };
    for depth in [10, 11] {
        let mut returns = json!({"components": [], "functions": [
            {"name": "F", "description": "", "params": []}
        ]});
        returns["functions"][0]["returns"] = deep(depth);
        let mut params = json!({"components": [], "actions": [
            {"name": "G", "description": "", "params": [{"name": "p"}]}
        ]});
        params["actions"][0]["params"][0]["type"] = deep(depth);
        for definition in [returns, params] {
            let read = LibraryDefinition::from_json_with_limits(&definition, limits);
            if depth == 10 {
                let library = create_library(read.unwrap()).unwrap();
                let _ = library.library();
            } else {
                assert!(matches!(
                    read,
                    Err(LibraryDefinitionError::DepthLimited { limit: 10, .. })
                ));
            }
        }
    }
}

/// A program calling a defined function parses and validates its arguments
/// exactly as with the same library supplied as upstream's JSON, and an `@`
/// name that is no function is reported as unknown in both. Each sample with
/// functions or actions is checked against programs that call each one
/// right, with an argument missing, with one too many and with a wrong type,
/// and an unknown one.
#[test]
fn a_program_calling_a_defined_function_parses_as_with_the_json() {
    let mut checked = 0;
    for case in golden().iter().filter(|c| {
        c.get("schema")
            .is_some_and(|s| s.get("functions").is_some() || s.get("actions").is_some())
    }) {
        let name = case["name"].as_str().unwrap();
        let defined = define(&case["definition"]).unwrap();
        let schema = &case["schema"];
        let defined = defined.library();
        let from_json = Library::from_json_schema(schema);
        assert_eq!(defined, from_json, "{name}");
        // The calls at the root, where the parser validates them: each with
        // its params' count of strings and of numbers, one fewer and one
        // more.
        let mut programs = vec!["root = @NoSuchFunction(1)".to_owned()];
        for key in ["functions", "actions"] {
            let Some(calls) = schema.get(key).and_then(Value::as_object) else {
                continue;
            };
            for (call, def) in calls {
                let arity = def["params"]["properties"]
                    .as_object()
                    .map_or(0, |p| p.len());
                for n in [arity, arity.saturating_sub(1), arity + 1] {
                    for arg in ["\"x\"", "1", "null"] {
                        let args = vec![arg; n].join(", ");
                        programs.push(format!("root = @{call}({args})"));
                    }
                }
            }
        }
        for program in &programs {
            let a = parse(program, &defined, None).to_json();
            let b = parse(program, &from_json, None).to_json();
            assert_eq!(a, b, "{name}: {program}");
            if program.contains("@NoSuchFunction") {
                assert!(a.contains("unknown-function"), "{name}: {a}");
            }
            checked += 1;
        }
    }
    assert!(checked > 100, "{checked}");
    // A defined function is known; an undefined one is not.
    let percent = define(&case("function-with-params-and-return")["definition"])
        .unwrap()
        .library();
    let known = parse("root = @Percent(1, 2)", &percent, None).to_json();
    assert!(!known.contains("unknown-function"), "{known}");
    let missing = parse("root = @Percent(1)", &percent, None).to_json();
    assert!(missing.contains("missing-required"), "{missing}");
    let unknown = parse("root = @Percnt(1, 2)", &percent, None).to_json();
    assert!(unknown.contains("unknown-function"), "{unknown}");
}

/// A function's or action's implementation never enters the core: a
/// library's calls are declarations, which compile to calls the parser
/// knows, after the 20 built-ins, functions before actions.
#[test]
fn defined_calls_follow_the_built_ins() {
    let library = define(&case("functions-and-actions-with-groups")["definition"])
        .unwrap()
        .library();
    let calls: Vec<(&str, bool, CallKind)> = library
        .calls()
        .map(|(name, call)| (name, call.builtin, call.kind))
        .collect();
    assert_eq!(calls.len(), 23);
    assert!(calls[..20].iter().all(|&(_, builtin, _)| builtin));
    assert_eq!(
        &calls[20..],
        [
            ("F", false, CallKind::Function),
            ("G", false, CallKind::Function),
            ("Z", false, CallKind::Action),
        ]
    );
}

/// `conformance/golden/library-definition-prompts.json`: upstream's
/// `library.prompt(options)` and cloud configs for the samples.
fn prompt_golden() -> Value {
    parse_json(&read(&golden_dir().join("library-definition-prompts.json"))).unwrap()
}

/// The system prompt a defined library generates is upstream's
/// `library.prompt(options)`, byte for byte, for every sample createLibrary
/// accepts: with no options, in the bindings mode for every sample with a
/// binding, an action slot, a function or an action (the `$binding<...>`
/// signatures, the function and action lines, the binding and action
/// text), and with every other option on a few.
#[test]
fn every_prompt_from_a_defined_library_is_upstreams() {
    let golden = prompt_golden();
    let prompts = golden["prompts"].as_array().unwrap();
    assert!(prompts.len() >= 100, "the golden lost its prompts");
    let mut seen = std::collections::HashSet::new();
    for entry in prompts {
        let name = entry["name"].as_str().unwrap();
        let library = define(&case(name)["definition"]).unwrap();
        let options = entry.get("options");
        let prompt = library
            .prompt(options.cloned())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            prompt,
            entry["prompt"].as_str().unwrap(),
            "{name} {options:?}"
        );
        // The same text through generateSystemPrompt with the spec.
        let mut spec = json!({"library": library.to_spec()});
        if let Some(options) = options {
            spec["promptOptions"] = options.clone();
        }
        assert_eq!(
            openui_lang::generate_system_prompt(&spec).unwrap(),
            prompt,
            "{name}"
        );
        seen.insert(name);
    }
    for must in [
        "binding",
        "action-slot",
        "custom-action",
        "functions-in-a-program",
    ] {
        assert!(seen.contains(must), "no prompt for {must}");
    }
    let bound = prompts
        .iter()
        .find(|e| e["name"] == "binding" && e["options"] == json!({"bindings": true}))
        .unwrap()["prompt"]
        .as_str()
        .unwrap();
    assert!(
        bound.contains("$binding<"),
        "the binding sample shows its binding"
    );
}

/// The cloud config a defined library generates is upstream's
/// `generateSystemPrompt({cloud: true, library: library.toSpec(),
/// promptOptions, instructions})`, byte for byte, or its error with its
/// message, for every sample createLibrary accepts.
#[test]
fn every_cloud_config_from_a_defined_library_is_upstreams() {
    let golden = prompt_golden();
    let cloud = golden["cloud"].as_array().unwrap();
    assert!(cloud.len() >= 70, "the golden lost its cloud configs");
    let (mut configs, mut errors) = (0, 0);
    for entry in cloud {
        let name = entry["name"].as_str().unwrap();
        let library = define(&case(name)["definition"]).unwrap();
        let result = library.cloud_config(
            entry.get("promptOptions").cloned(),
            entry.get("instructions").map(|i| i.as_str().unwrap()),
        );
        match (result, entry.get("config"), entry.get("error")) {
            (Ok(config), Some(expected), None) => {
                assert_eq!(config, expected.as_str().unwrap(), "{name}");
                configs += 1;
            }
            (Err(e), None, Some(expected)) => {
                assert_eq!(e.to_string(), expected.as_str().unwrap(), "{name}");
                errors += 1;
            }
            (result, _, _) => panic!("{name}: {result:?} against {entry}"),
        }
    }
    assert!(
        configs > 60 && errors > 5,
        "{configs} configs, {errors} errors"
    );
}

/// A prompt's options and a cloud config's request are held to the prompt's
/// depth limit, as `generate_system_prompt_with_limits` holds a spec.
#[test]
fn prompts_from_a_defined_library_keep_the_prompt_limits() {
    let library = define(&case("scalars")["definition"]).unwrap();
    let mut deep = json!("x");
    // Depth 20 allows 92 levels of JSON (3 per level and 32).
    for _ in 0..100 {
        deep = json!([deep]);
    }
    let options = json!({"examples": deep});
    let limits = Limits::default().with_depth(20);
    let err = library
        .prompt_with_limits(Some(options.clone()), limits)
        .unwrap_err();
    assert!(err.to_string().contains("nests deeper than"), "{err}");
    let err = library
        .cloud_config_with_limits(Some(options), None, limits)
        .unwrap_err();
    assert!(err.to_string().contains("nests deeper than"), "{err}");
    // Options that null the components are refused, not a crash.
    let err = library
        .prompt(Some(json!({"components": null})))
        .unwrap_err();
    assert_eq!(err.to_string(), "[generatePrompt] components is missing.");
    // Options that are not an object add nothing.
    let plain = library.prompt(None).unwrap();
    for options in [json!(null), json!("bindings"), json!([true]), json!(1)] {
        assert_eq!(library.prompt(Some(options)).unwrap(), plain);
    }
}

/// A definition at the type depth limit, in each recursive kind, makes its
/// prompt and its cloud config (which copies the deep JSON Schema) on a
/// 64 KiB thread.
#[test]
fn prompts_from_a_definition_at_the_limit_need_no_stack() {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(|| {
            let limit = Limits::default().depth();
            for wrap in [
                in_array,
                in_object,
                in_union,
                in_record,
                in_nullable,
                in_named,
            ] {
                let mut definition = nested(limit, wrap);
                definition["root"] = json!("Deep");
                let library = define(&definition).unwrap();
                free_json(definition);
                assert!(library.prompt(None).unwrap().contains("Deep(v"));
                let config = library.cloud_config(None, None).unwrap();
                assert!(config.starts_with("]]>openui:config\n"));
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A defined library names its binding props, the ones marked
/// `"binding": true`, for the components the library keeps.
#[test]
fn a_defined_library_names_its_binding_props() {
    let library = define(&json!({
        "components": [
            {"name": "Input", "description": "", "props": [
                {"name": "label", "type": {"kind": "string"}},
                {"name": "value", "type": {"kind": "string"}, "binding": true},
                {"name": "2", "type": {"kind": "number"}, "binding": true}
            ]},
            {"name": "Card", "description": "first", "props": [
                {"name": "open", "type": {"kind": "boolean"}, "binding": true}
            ]},
            {"name": "Card", "description": "second", "props": [
                {"name": "shut", "type": {"kind": "boolean"}, "binding": true}
            ]}
        ]
    }))
    .unwrap();
    let pairs: Vec<_> = library.binding_props().collect();
    assert_eq!(
        pairs,
        [("Input", "2"), ("Input", "value"), ("Card", "shut")]
    );
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
