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
    Library, LibraryDefinition, LibraryDefinitionError, Limits, StreamParser, TypeDescription,
    create_library, free_json, parse, parse_json, stringify_json,
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
    for kind in ["component", "action", "named", "date", "literal", ""] {
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
            for wrap in [in_array, in_object, in_union, in_record, in_nullable] {
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
            let at_limit = LibraryDefinition::from_json(&nested(limit, in_array)).unwrap();
            assert_eq!(depth_of(&at_limit.components[0].props[0].ty), limit);
            let err = LibraryDefinition::from_json(&nested(limit + 1, in_array))
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

/// The crate's smallest stack in the depth suite.
const SMALL_STACK: usize = 64 * 1024;

/// On a 64 KiB thread, a definition nested to the default limit in each
/// recursive kind reads, builds its schema, spec and parser library, and
/// drops all of it: compiling the deep schema (whose clones and drops
/// recurse) runs on a stack segment sized for the definition's depth.
///
/// What is handed back is dropped by the caller, through serde_json's
/// recursive drop (the returned `Library` holds each prop's schema). That
/// fits 64 KiB in the test profile (opt-level 1) and in release, as here.
/// Measured at opt-level 0 it does not: the schema of an array chain at the
/// limit overflows 64 KiB, and the `Library` of an object chain needs
/// between 192 and 256 KiB. The returned JSON can be freed with
/// `free_json`; the `Library` cannot be, short of a change to the parser's
/// own type.
#[test]
fn a_definition_at_the_limit_builds_and_drops_on_a_64_kib_stack() {
    let limit = Limits::default().depth();
    for wrap in [in_array, in_object, in_union, in_record, in_nullable] {
        let input = nested(limit, wrap);
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
