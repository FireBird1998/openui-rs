//! Library definitions: a plain-data library definition goes in, and the
//! component library's JSON Schema and prompt spec come out byte for byte as
//! upstream's `toJSONSchema()` and `toSpec()` give them for the zod
//! declaration it was converted from, or the error upstream's `createLibrary`
//! throws, with its message. The expected output is
//! `conformance/golden/library-definitions.json`, written by
//! `reference/library-definitions.ts` running upstream's code.
//!
//! Also the parser guarantee: a defined library reaches the parser only as
//! that JSON Schema, so it parses the corpus programs written for it exactly
//! as the same library supplied as JSON, and the built-in table keeps its 20
//! names.

mod common;

use common::{conformance_dir, golden_dir, load_corpus, read};
use openui_lang::{
    Library, LibraryDefinition, LibraryDefinitionError, StreamParser, create_library, parse,
    parse_json, stringify_json,
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
    for kind in ["enum", "array", "union", "date", ""] {
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
    let defined = defined.library();
    assert_eq!(defined, from_json);

    let corpus = load_corpus();
    let cases: Vec<_> = corpus
        .cases
        .iter()
        .filter(|c| c.name.starts_with("library-definition/"))
        .collect();
    assert_eq!(cases.len(), 3);
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

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
