//! A component library through the native binding (#140): a library
//! supplied as JSON Schema is host input, held to the session's host input
//! limit in bytes before it is read, and a library costs time and memory in
//! step with its size through either entry point, because the core shares
//! each action shape between the params that refer to it. Making, parsing
//! with and dropping a library work on a 64 KiB thread; run these
//! unoptimised too (`CARGO_PROFILE_TEST_OPT_LEVEL=0`), where frames are
//! largest. Kept in their own test binary: a stack overflow aborts the
//! process.

use std::sync::Arc;
use std::time::{Duration, Instant};

use openui_ffi::{ComponentLibrary, OpenUiError, RuntimeLimits, StreamSession, parse};
use openui_lang::{LibraryDefinition, create_library, stringify_json};
use serde_json::{Value, json};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const SMALL_STACK: usize = 64 * 1024;

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn limit_exceeded(result: Result<Arc<ComponentLibrary>, OpenUiError>) -> String {
    match result {
        Err(OpenUiError::LimitExceeded { reason }) => reason,
        Err(other) => panic!("expected LimitExceeded, got {other:?}"),
        Ok(_) => panic!("expected LimitExceeded, got a library"),
    }
}

fn invalid_schema(result: Result<Arc<ComponentLibrary>, OpenUiError>) -> String {
    match result {
        Err(OpenUiError::InvalidSchema { reason }) => reason,
        Err(other) => panic!("expected InvalidSchema, got {other:?}"),
        Ok(_) => panic!("expected InvalidSchema, got a library"),
    }
}

/// A JSON Schema `bytes` long: one component whose description is `é` (two
/// bytes) repeated, then `a` to make up an odd length.
fn schema_of(bytes: usize) -> String {
    let head = r#"{"$defs":{"A":{"properties":{"x":{}},"description":""#;
    let tail = r#""}}}"#;
    let room = bytes - head.len() - tail.len();
    let text = format!(
        "{head}{}{}{tail}",
        "é".repeat(room / 2),
        "a".repeat(room % 2)
    );
    assert_eq!(text.len(), bytes);
    text
}

/// The schema is the session's host input, counted in bytes: one of exactly
/// `max_json_bytes` is accepted and one byte more is refused with the
/// existing limit error, though it has far fewer characters than the limit;
/// and it is refused before it is read, so malformed text over the limit is
/// a limit error too, while malformed text within it is InvalidSchema as
/// before.
#[test]
fn the_schema_is_held_to_the_host_input_limit_in_bytes() {
    let default = RuntimeLimits::default().max_json_bytes;
    assert_eq!(default, 8 * 1024 * 1024);
    let at = schema_of(default as usize);
    assert!(at.chars().count() < default as usize / 2 + 64);
    let library = ComponentLibrary::new(at).unwrap();
    assert_eq!(library.component_names(), ["A"]);
    let over = schema_of(default as usize + 1);
    assert!(over.chars().count() < default as usize);
    assert_eq!(
        limit_exceeded(ComponentLibrary::new(over)),
        "host input byte limit"
    );
    let junk = "{".repeat(default as usize + 1);
    assert_eq!(
        limit_exceeded(ComponentLibrary::new(junk)),
        "host input byte limit"
    );
    // Malformed text within the limit is refused as it always was.
    let reason = invalid_schema(ComponentLibrary::new("{".into()));
    assert_eq!(reason, "EOF while parsing an object at line 1 column 1");

    // Configurable, as the session's limit is.
    let limits = RuntimeLimits {
        max_json_bytes: 1000,
        ..RuntimeLimits::default()
    };
    let with = |text: String| ComponentLibrary::from_schema_with_limits(text, limits.clone());
    assert_eq!(with(schema_of(1000)).unwrap().component_names(), ["A"]);
    assert_eq!(
        limit_exceeded(with(schema_of(1001))),
        "host input byte limit"
    );
    assert_eq!(
        limit_exceeded(with("[".repeat(1001))),
        "host input byte limit"
    );
    assert!(matches!(
        with("[".repeat(1000)),
        Err(OpenUiError::InvalidSchema { .. })
    ));
}

fn action(name: &str) -> Value {
    json!({"kind": "action", "name": name})
}

fn definition(first: &str, actions: Vec<Value>) -> Value {
    json!({
        "components": [{"name": "C", "description": "", "props": [
            {"name": "go", "type": action(first)}
        ]}],
        "actions": actions,
        "root": "C"
    })
}

/// A chain of `n` actions, each one's param an action slot for the next.
fn chain(n: usize) -> Value {
    let actions = (0..n)
        .map(|i| {
            let ty = if i + 1 < n {
                action(&format!("A{}", i + 1))
            } else {
                json!({"kind": "string"})
            };
            json!({"name": format!("A{i}"), "description": "", "params": [{"name": "p", "type": ty}]})
        })
        .collect();
    definition("A0", actions)
}

/// `n` actions, each naming the next twice.
fn doubling(n: usize) -> Value {
    let actions = (0..n)
        .map(|i| {
            let ty = if i + 1 < n {
                action(&format!("A{}", i + 1))
            } else {
                json!({"kind": "string"})
            };
            json!({"name": format!("A{i}"), "description": "", "params": [
                {"name": "p", "type": ty.clone()}, {"name": "q", "type": ty}
            ]})
        })
        .collect();
    definition("A0", actions)
}

/// `n` params referring to one action with `n` params.
fn many(n: usize) -> Value {
    let big = (0..n)
        .map(|i| json!({"name": format!("b{i}"), "type": {"kind": "string"}, "optional": i > 0}))
        .collect::<Vec<_>>();
    let refs = (0..n)
        .map(|i| json!({"name": format!("m{i}"), "type": action("Big"), "optional": i > 2}))
        .collect::<Vec<_>>();
    definition(
        "Many",
        vec![
            json!({"name": "Big", "description": "", "params": big}),
            json!({"name": "Many", "description": "", "params": refs}),
        ],
    )
}

/// Make the library from `text` (a definition, or a JSON Schema), parse
/// `program` one-shot and streamed, and drop it, on a 64 KiB thread; the
/// peak heap and the one-shot result.
fn load(text: String, schema: bool, program: &'static str) -> (usize, Duration, String) {
    on_small_stack(move || {
        test_alloc::reset_peak();
        let before = test_alloc::peak();
        let started = Instant::now();
        let library = if schema {
            ComponentLibrary::new(text).unwrap()
        } else {
            ComponentLibrary::from_definition(text).unwrap()
        };
        let stream = StreamSession::new(library.clone(), None);
        for piece in program.as_bytes().chunks(16) {
            stream.push(std::str::from_utf8(piece).unwrap().to_owned());
        }
        let streamed = stream.finish().json;
        drop(stream);
        let parsed = parse(program.into(), library.clone(), None);
        assert_eq!(parsed.json, streamed);
        drop(parsed.program);
        drop(library);
        (test_alloc::peak() - before, started.elapsed(), parsed.json)
    })
}

/// The three shapes measured on #140 before sharing, through the definition
/// and through its JSON Schema: a chain of 2,000 actions (194 KB; 4.7 GB and
/// 4.6 s before), the doubling shape at 30 actions (terabytes before) and
/// 2,000 params referring to an action with 2,000 params (256 KB here; 1.7
/// GB before), each made, parsed and dropped on a 64 KiB thread. Measured
/// 2026-11-22, peak heap and time through the definition / the schema, test
/// profile (opt-level 1), then opt-level 0:
/// - chain: 31.8 MB / 20.5 MB; 156 ms / 15 ms; 464 ms / 115 ms;
/// - doubling: 549 KB / 385 KB; 4.5 ms / 0.5 ms; 11 ms / 3 ms;
/// - many: 10.9 MB / 8.2 MB; 236 ms / 16 ms; 447 ms / 124 ms.
#[test]
fn the_shapes_that_copied_load_in_step_with_their_size_through_both_entry_points() {
    let cases: [(&str, Value, &'static str, &str, usize); 3] = [
        (
            "chain",
            chain(2000),
            "root = C(@A0({type: \"A1\", params: {p: {type: \"A2\", params: {p: 1}}}}))",
            "/p/params/p/params/p",
            48,
        ),
        (
            "doubling",
            doubling(30),
            "root = C(@A0({type: \"A1\", params: {p: 1, q: {type: \"A2\", params: {p: \"x\"}}}}, null))",
            "/p/params/p",
            4,
        ),
        (
            "many",
            many(2000),
            "root = C(@Many({type: \"Big\", params: {b0: \"x\", b1: 2}}, null, {type: \"Big\", params: {b0: \"y\"}}))",
            "/m0/params/b1",
            24,
        ),
    ];
    for (name, definition, program, first_error, megabytes) in cases {
        let text = stringify_json(&definition);
        let schema = stringify_json(
            &create_library(LibraryDefinition::from_json(&definition).unwrap())
                .unwrap()
                .to_json_schema(),
        );
        let mut results = Vec::new();
        for (via_schema, text) in [(false, text.clone()), (true, schema)] {
            let (peak, elapsed, result) = load(text, via_schema, program);
            eprintln!(
                "{name} via {}: peak {} KB, {elapsed:?}",
                if via_schema { "schema" } else { "definition" },
                peak / 1024
            );
            assert!(peak < megabytes * 1024 * 1024, "{name}: peak {peak} bytes");
            assert!(
                result.contains(&format!("\"path\":\"{first_error}\"")),
                "{name}: {result}"
            );
            results.push(result);
        }
        assert_eq!(
            results[0], results[1],
            "{name}: definition and schema parse alike"
        );
    }
}

/// A library schema's text nested `levels` deep in all (the root object is
/// level 1), of objects or of arrays, holding one prop `v` of component
/// `Box`: the prop's schema starts at level 5, under root, $defs, Box and
/// properties. An object level is two JSON levels (it and its
/// `properties`); an odd remainder is made up with an `enum` list in the
/// leaf.
fn nested_schema(shape: &str, levels: usize) -> String {
    let (open, close) = match shape {
        "object" => (r#"{"type":"object","properties":{"a":"#, "}}"),
        _ => (r#"{"type":"array","items":"#, "}"),
    };
    let (count, leaf) = match shape {
        "object" if (levels - 5) % 2 == 1 => {
            ((levels - 6) / 2, r#"{"type":"string","enum":["a"]}"#)
        }
        "object" => ((levels - 5) / 2, r#"{"type":"string"}"#),
        _ => (levels - 5, r#"{"type":"string"}"#),
    };
    format!(
        r#"{{"$defs":{{"Box":{{"properties":{{"v":{}{leaf}{}}}}}}}}}"#,
        open.repeat(count),
        close.repeat(count)
    )
}

/// A schema nested to the binding's JSON reader's limit (serde_json's: 127
/// levels read, the 128th refused), of objects and of arrays, is read,
/// compiled, parsed with and dropped on a 64 KiB thread, unoptimised too;
/// one level more is refused with exactly the error it always had. The read
/// runs on a stack segment sized for the reader's bound (#140), so a schema
/// at the limit cannot overflow the caller's thread.
#[test]
fn a_schema_nested_to_the_readers_limit_reads_parses_and_drops_on_a_small_stack() {
    for shape in ["object", "array"] {
        let (at, over) = (nested_schema(shape, 127), nested_schema(shape, 128));
        let value = match shape {
            "object" => format!("{}1{}", "{a: ".repeat(60), "}".repeat(60)),
            _ => format!("{}1{}", "[".repeat(120), "]".repeat(120)),
        };
        let expected = format!(
            "recursion limit exceeded at line 1 column {}",
            OVER_COLUMN[usize::from(shape == "array")]
        );
        on_small_stack(move || {
            assert_eq!(invalid_schema(ComponentLibrary::new(over)), expected);
            let library = ComponentLibrary::new(at).unwrap();
            let parsed = parse(format!("root = Box({value})"), library.clone(), None);
            assert!(parsed.json.contains("type-mismatch"), "{}", parsed.json);
            drop(parsed.program);
            drop(library);
        });
    }
}

/// Where serde_json stops reading `nested_schema(_, 128)`, objects then
/// arrays, as the binding reported before #140 (captured from 8765b48e).
const OVER_COLUMN: [usize; 2] = [2195, 2988];

/// Malformed schemas are refused with the kind and message they always had
/// (captured from 8765b48e, before the read moved onto its own segment).
#[test]
fn malformed_schemas_are_refused_as_before() {
    for (text, message) in MALFORMED {
        let owned = text.to_owned();
        let reason = on_small_stack(move || invalid_schema(ComponentLibrary::new(owned)));
        assert_eq!(reason, message, "{text:?}");
    }
}

const MALFORMED: [(&str, &str); 7] = [
    ("", "EOF while parsing a value at line 1 column 0"),
    ("{", "EOF while parsing an object at line 1 column 1"),
    ("[1,]", "trailing comma at line 1 column 4"),
    ("nul", "EOF while parsing a value at line 1 column 3"),
    (
        "{\"$defs\": {}} x",
        "trailing characters at line 1 column 15",
    ),
    (
        "{\"a\": \"\\ud800\"}",
        "unexpected end of hex escape at line 1 column 14",
    ),
    ("{\"a\": 1e400}", "number out of range at line 1 column 11"),
];

/// A library definition with types nested to the depth limit (256), of
/// objects and of arrays, is read, made, parsed with and dropped on a
/// 64 KiB thread: the binding reads a definition with the core's reader,
/// which grows its stack.
#[test]
fn a_definition_at_the_depth_limit_loads_parses_and_drops_on_a_small_stack() {
    for kind in ["object", "array"] {
        let mut ty = r#"{"kind":"string"}"#.to_owned();
        for _ in 1..256 {
            ty = match kind {
                "object" => format!(r#"{{"kind":"object","fields":[{{"name":"a","type":{ty}}}]}}"#),
                _ => format!(r#"{{"kind":"array","items":{ty}}}"#),
            };
        }
        let text = format!(
            r#"{{"components":[{{"name":"Box","description":"","props":[{{"name":"v","type":{ty}}}]}}]}}"#
        );
        on_small_stack(move || {
            let library = ComponentLibrary::from_definition(text).unwrap();
            let parsed = parse("root = Box(1)".into(), library.clone(), None);
            assert!(parsed.json.contains("type-mismatch"), "{}", parsed.json);
            drop(parsed.program);
            drop(library);
        });
    }
}
