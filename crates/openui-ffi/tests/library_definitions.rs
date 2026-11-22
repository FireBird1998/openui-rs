//! A library definition through the native binding (#137): the plain-data
//! form goes in as JSON text and the component library comes out, with its
//! JSON Schema and prompt spec as text byte for byte as upstream writes
//! them, usable everywhere a library supplied as JSON Schema is. The samples
//! are `conformance/golden/library-definitions.json`, written by the
//! reference harness running upstream's code; no library is declared here
//! beyond the tiny ones the limit and refusal cases need.

#[path = "../../openui-lang/tests/common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{golden_dir, load_corpus, read};
use openui_ffi::{
    ComponentLibrary, OpenUiError, RuntimeLimits, RuntimeSession, StreamSession,
    generate_system_prompt, parse,
};
use openui_lang::{LibraryDefinition, create_library, parse_json, stringify_json};
use serde_json::{Value, json};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn golden() -> Vec<Value> {
    let text = read(&golden_dir().join("library-definitions.json"));
    match parse_json(&text).unwrap() {
        Value::Array(cases) => cases,
        other => panic!("expected an array, got {other}"),
    }
}

fn sample(name: &str) -> Value {
    golden()
        .into_iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no sample {name}"))
}

fn define(definition: &Value) -> Result<Arc<ComponentLibrary>, OpenUiError> {
    ComponentLibrary::from_definition(stringify_json(definition))
}

/// The same library supplied as upstream's JSON Schema.
fn from_upstream(case: &Value) -> Arc<ComponentLibrary> {
    ComponentLibrary::new(stringify_json(&case["schema"])).unwrap()
}

fn invalid_json(result: Result<Arc<ComponentLibrary>, OpenUiError>) -> String {
    match result {
        Err(OpenUiError::InvalidJson { reason }) => reason,
        Err(other) => panic!("expected InvalidJson, got {other:?}"),
        Ok(_) => panic!("expected InvalidJson, got a library"),
    }
}

fn limit_exceeded(result: Result<Arc<ComponentLibrary>, OpenUiError>) -> String {
    match result {
        Err(OpenUiError::LimitExceeded { reason }) => reason,
        Err(other) => panic!("expected LimitExceeded, got {other:?}"),
        Ok(_) => panic!("expected LimitExceeded, got a library"),
    }
}

/// Every sample gives upstream's JSON Schema and prompt spec text, byte for
/// byte, which is also what the core gives written with `stringify_json`,
/// or upstream's error, as InvalidJson.
#[test]
fn every_sample_gives_upstreams_schema_and_spec_text() {
    let cases = golden();
    assert_eq!(cases.len(), 93, "the golden's samples changed");
    let (mut errors, mut libraries) = (0, 0);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let result = define(&case["definition"]);
        if let Some(error) = case.get("error") {
            assert_eq!(invalid_json(result), error.as_str().unwrap(), "{name}");
            errors += 1;
            continue;
        }
        libraries += 1;
        let library = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let core =
            create_library(LibraryDefinition::from_json(&case["definition"]).unwrap()).unwrap();
        let schema = library.json_schema().unwrap();
        let spec = library.prompt_spec().unwrap();
        assert_eq!(
            schema,
            stringify_json(&case["schema"]),
            "{name}: JSON Schema"
        );
        assert_eq!(spec, stringify_json(&case["spec"]), "{name}: prompt spec");
        assert_eq!(schema, stringify_json(&core.to_json_schema()), "{name}");
        assert_eq!(spec, stringify_json(&core.to_spec()), "{name}");
    }
    assert_eq!((errors, libraries), (23, 70), "every sample was checked");
}

/// Components named like array indices come first in the text, as
/// JavaScript writes them, though the core holds `$defs` in walk order.
#[test]
fn integer_like_component_names_come_first_as_javascript_writes_them() {
    let case = sample("key-order");
    let library = define(&case["definition"]).unwrap();
    let schema = library.json_schema().unwrap();
    assert!(
        schema.contains(r##""properties":{"7":{"$ref":"#/$defs/7"},"Zeta""##),
        "{schema}"
    );
    assert!(schema.contains(r#""$defs":{"7":{"#), "{schema}");
    let spec = library.prompt_spec().unwrap();
    assert!(
        spec.starts_with(r#"{"root":"7","components":{"7":"#),
        "{spec}"
    );
    assert_eq!(schema, stringify_json(&case["schema"]));
}

/// A library loaded from a JSON Schema has no definition texts.
#[test]
fn a_library_from_a_json_schema_has_no_definition_texts() {
    let library = from_upstream(&sample("scalars"));
    assert_eq!(library.json_schema(), None);
    assert_eq!(library.prompt_spec(), None);
}

/// Programs to parse with each sample's library: the corpus programs written
/// for the "scalars" sample, and a call of each component with no
/// arguments and with a few.
fn programs(library: &ComponentLibrary) -> Vec<String> {
    let mut programs: Vec<String> = load_corpus()
        .cases
        .iter()
        .filter(|c| c.name.starts_with("library-definition/"))
        .map(|c| c.input.clone())
        .collect();
    assert_eq!(programs.len(), 3);
    for name in library.component_names() {
        programs.push(format!("root = {name}()"));
        programs.push(format!(
            "x = Action([@ToAssistant(\"hi\")])\nroot = {name}(\"a\", 1, true, [x], {{k: null}})"
        ));
    }
    programs
}

/// Each sample's defined library parses, one shot and streamed, exactly as
/// the same library supplied as upstream's JSON Schema.
#[test]
fn a_defined_library_parses_as_the_same_library_supplied_as_json() {
    for case in golden().iter().filter(|c| c.get("schema").is_some()) {
        let name = case["name"].as_str().unwrap();
        let defined = define(&case["definition"]).unwrap();
        let upstream = from_upstream(case);
        assert_eq!(
            defined.component_names(),
            upstream.component_names(),
            "{name}"
        );
        let root = case["definition"]["root"].as_str().map(str::to_owned);
        for program in programs(&defined) {
            let a = parse(program.clone(), defined.clone(), root.clone());
            let b = parse(program.clone(), upstream.clone(), root.clone());
            assert_eq!(a.json, b.json, "{name}: {program}");
            let a = StreamSession::new(defined.clone(), root.clone());
            let b = StreamSession::new(upstream.clone(), root.clone());
            for c in program.chars() {
                let chunk = c.to_string();
                assert_eq!(a.push(chunk.clone()).json, b.push(chunk).json, "{name}");
            }
            assert_eq!(a.finish().json, b.finish().json, "{name}: {program}");
        }
    }
}

/// The prompt spec text makes the system prompt upstream's spec makes.
#[test]
fn the_prompt_spec_text_makes_the_same_system_prompt() {
    for case in golden().iter().filter(|c| c.get("spec").is_some()) {
        let name = case["name"].as_str().unwrap();
        let spec = define(&case["definition"]).unwrap().prompt_spec().unwrap();
        let ours = generate_system_prompt(format!(r#"{{"library":{spec}}}"#));
        let theirs = generate_system_prompt(format!(
            r#"{{"library":{}}}"#,
            stringify_json(&case["spec"])
        ));
        match (ours, theirs) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{name}"),
            (a, b) => assert_eq!(format!("{a:?}"), format!("{b:?}"), "{name}"),
        }
    }
}

/// A session over a defined library, given its prompt spec text, draws and
/// decides bindings as one over the same library supplied as upstream's
/// JSON Schema and spec.
#[test]
fn a_session_over_a_defined_library_draws_as_one_over_the_json() {
    let cases = [
        (
            "binding",
            "$v = \"a\"\n$n = null\nroot = Input($v, $v, $n, [\"x\"], \"a\")",
        ),
        (
            "functions-in-a-program",
            "root = Box(@Percent(1, 4), @Label(\"x\"), @Unimplemented(2))",
        ),
        ("custom-action", "root = Root(Action([@Copy(\"t\")]))"),
    ];
    for (name, program) in cases {
        let case = sample(name);
        let defined = define(&case["definition"]).unwrap();
        let upstream = from_upstream(&case);
        let mut draws = Vec::new();
        for (library, spec) in [
            (defined.clone(), defined.prompt_spec().unwrap()),
            (upstream, stringify_json(&case["spec"])),
        ] {
            let session = RuntimeSession::new(library.clone(), Some(spec)).unwrap();
            let parsed = parse(program.into(), library, None).program;
            session.set_program(parsed).unwrap();
            let draw = session.evaluate().unwrap();
            let undecided = session.undecided_bindings().unwrap().len();
            draws.push((draw.json, undecided, session.snapshot().unwrap()));
            session.dispose();
        }
        assert_eq!(draws[0], draws[1], "{name}");
    }
}

/// A definition `bytes` long: one component whose description is `é`
/// (two bytes) repeated, then `a` to make up an odd length.
fn definition_of(bytes: usize) -> String {
    let head = r#"{"components":[{"name":"A","description":""#;
    let tail = r#"","props":[]}]}"#;
    let room = bytes - head.len() - tail.len();
    let text = format!(
        "{head}{}{}{tail}",
        "é".repeat(room / 2),
        "a".repeat(room % 2)
    );
    assert_eq!(text.len(), bytes);
    text
}

/// The limit is the session's host input limit, counted in bytes: a
/// definition of exactly `max_json_bytes` is accepted and one byte more is
/// refused with the existing limit error, though it has far fewer
/// characters than the limit; and it is refused before it is read, so
/// malformed text over the limit is a limit error too.
#[test]
fn the_definition_is_held_to_the_host_input_limit_in_bytes() {
    let default = RuntimeLimits::default().max_json_bytes;
    assert_eq!(default, 8 * 1024 * 1024);
    let at = definition_of(default as usize);
    assert!(at.chars().count() < default as usize / 2 + 64);
    assert!(define_text(at).is_ok());
    let over = definition_of(default as usize + 1);
    assert!(over.chars().count() < default as usize);
    assert_eq!(limit_exceeded(define_text(over)), "host input byte limit");
    let junk = "{".repeat(default as usize + 1);
    assert_eq!(limit_exceeded(define_text(junk)), "host input byte limit");

    // Configurable, as the session's limit is.
    let limits = RuntimeLimits {
        max_json_bytes: 1000,
        ..RuntimeLimits::default()
    };
    let with = |text: String| ComponentLibrary::from_definition_with_limits(text, limits.clone());
    assert!(with(definition_of(1000)).is_ok());
    assert_eq!(
        limit_exceeded(with(definition_of(1001))),
        "host input byte limit"
    );
    assert_eq!(
        limit_exceeded(with("[".repeat(1001))),
        "host input byte limit"
    );
    assert!(matches!(
        with("[".repeat(1000)),
        Err(OpenUiError::InvalidJson { .. })
    ));
}

fn define_text(text: String) -> Result<Arc<ComponentLibrary>, OpenUiError> {
    ComponentLibrary::from_definition(text)
}

/// Each refusal reaches the host as InvalidJson with the core's message.
#[test]
fn refusals_reach_the_host_as_invalid_json_with_the_cores_message() {
    let malformed = invalid_json(define_text(r#"{"components": ["#.into()));
    assert!(!malformed.is_empty());
    assert_eq!(
        invalid_json(define_text("[]".into())),
        "library definition: expected an object"
    );
    let root = json!({"components": [{"name": "A", "description": "", "props": []}], "root": "B"});
    assert_eq!(
        invalid_json(define(&root)),
        "[createLibrary] Root component \"B\" was not found in components. Available components: A"
    );
    let collision = json!({
        "components": [{"name": "A", "description": "", "props": []}],
        "functions": [{"name": "A", "description": "", "params": []}]
    });
    assert_eq!(
        invalid_json(define(&collision)),
        "[createLibrary] Function \"A\" collides with a built-in, component, function, or action of the same name. Rename it."
    );
    let kind = json!({"components": [{"name": "A", "description": "", "props": [
        {"name": "when", "type": {"kind": "date"}}
    ]}]});
    assert_eq!(
        invalid_json(define(&kind)),
        "library definition: components[0].props[0].type: type kind \"date\" is not supported"
    );
    // Types nested past the session's depth limit.
    let limits = RuntimeLimits {
        evaluation: openui_ffi::ParseLimits {
            depth: Some(1),
            ..Default::default()
        },
        ..RuntimeLimits::default()
    };
    let nested = json!({"components": [{"name": "A", "description": "", "props": [
        {"name": "v", "type": {"kind": "array", "items": {"kind": "string"}}}
    ]}]});
    let refused = ComponentLibrary::from_definition_with_limits(stringify_json(&nested), limits);
    assert!(invalid_json(refused).contains("types nest deeper than 1 levels"));
}

/// A chain of library actions, each one's param an action slot for the
/// next. Upstream copies each action's shape into every param that refers
/// to it, quadratic in `n`; the compiled library shares the shapes instead,
/// so this costs time and memory in step with `n` (#140; the 2,000-action
/// chain is in `library_schemas.rs`).
fn action_chain(n: usize) -> String {
    let mut actions = Vec::with_capacity(n);
    for i in 0..n {
        let ty = if i + 1 < n {
            json!({"kind": "action", "name": format!("A{}", i + 1)})
        } else {
            json!({"kind": "string"})
        };
        actions.push(
            json!({"name": format!("A{i}"), "description": "", "params": [
                {"name": "p", "type": ty}
            ]}),
        );
    }
    stringify_json(&json!({
        "components": [{"name": "C", "description": "", "props": [
            {"name": "go", "type": {"kind": "action", "name": "A0"}}
        ]}],
        "actions": actions,
        "root": "C"
    }))
}

/// A chain of 100 actions, under 10 KB of definition, is made, parsed,
/// streamed, drawn by a session and dropped, all on 64 KiB threads, within
/// 32 MiB of heap and 10 seconds. Before #140 shared the action shapes, 100
/// actions took about 12 MB and dropping the library overflowed a 64 KiB
/// thread past about 150 actions.
#[test]
fn a_long_action_chain_of_a_handled_size_is_handled_on_a_small_stack() {
    const SMALL: usize = 64 * 1024;
    let text = action_chain(100);
    assert!(text.len() < 10_000, "{}", text.len());
    let started = Instant::now();
    let (library, peak) = std::thread::Builder::new()
        .stack_size(SMALL)
        .spawn(move || {
            test_alloc::reset_peak();
            let before = test_alloc::peak();
            let library = ComponentLibrary::from_definition(text).unwrap();
            (library, test_alloc::peak() - before)
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(peak < 32 * 1024 * 1024, "peak heap {peak} bytes");
    let program = "root = C(Action([@A0(Action([@A1(\"x\")]))]))";
    let parsed = {
        let library = library.clone();
        std::thread::Builder::new()
            .stack_size(SMALL)
            .spawn(move || {
                let stream = StreamSession::new(library.clone(), None);
                for c in program.chars() {
                    stream.push(c.to_string());
                }
                let streamed = stream.finish().json;
                let parsed = parse(program.into(), library, None);
                assert_eq!(parsed.json, streamed);
                parsed.program
            })
            .unwrap()
            .join()
            .unwrap()
    };
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session.set_program(parsed).unwrap();
    let draw = session.evaluate().unwrap();
    assert!(draw.json.contains(r#""name":"A0""#), "{}", draw.json);
    session.dispose();
    drop(session);
    std::thread::Builder::new()
        .stack_size(SMALL)
        .spawn(move || drop(library))
        .unwrap()
        .join()
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

/// An input whose `value` is a binding, as a small definition.
fn bound_input() -> Value {
    json!({
        "components": [{"name": "Input", "description": "", "props": [
            {"name": "label", "type": {"kind": "string"}},
            {"name": "value", "type": {"kind": "string"}, "binding": true}
        ]}],
        "root": "Input"
    })
}

const BOUND_PROGRAM: &str = "$v = \"a\"\nroot = Input($v, $v)";
const MARKER: &str =
    r#"{"__reactive":"assign","target":"$v","expr":{"k":"StateRef","n":"$value"}}"#;

fn session_over(library: &Arc<ComponentLibrary>, spec: Option<String>) -> Arc<RuntimeSession> {
    let session = RuntimeSession::new(library.clone(), spec).unwrap();
    let parsed = parse(BOUND_PROGRAM.into(), library.clone(), None).program;
    session.set_program(parsed).unwrap();
    session
}

/// Spec #123 story 24 (#135): a session created with a defined library and
/// no prompt spec knows the definition's binding props, so an input bound
/// to one draws a binding marker and writes state through it, exactly as a
/// session over the same library given its prompt spec does today. The same
/// library supplied as a JSON Schema with no prompt spec has no binding.
#[test]
fn a_session_over_a_defined_library_knows_its_bindings_without_a_prompt_spec() {
    let defined = define(&bound_input()).unwrap();
    let plain = ComponentLibrary::new(defined.json_schema().unwrap()).unwrap();
    let spec = defined.prompt_spec().unwrap();
    let sessions = [
        session_over(&defined, None),
        session_over(&defined, Some(spec.clone())),
        session_over(&plain, Some(spec)),
    ];
    let mut draws = Vec::new();
    for session in &sessions {
        let draw = session.evaluate().unwrap().json;
        assert!(draw.contains(MARKER), "{draw}");
        assert!(session.undecided_bindings().unwrap().is_empty());
        assert!(
            session
                .write_binding(MARKER.into(), r#""typed""#.into())
                .unwrap()
        );
        assert_eq!(
            session.get("$v".into()).unwrap().as_deref(),
            Some(r#""typed""#)
        );
        draws.push((
            draw,
            session.snapshot().unwrap(),
            session.evaluate().unwrap().json,
        ));
        session.dispose();
    }
    assert_eq!(draws[0], draws[1]);
    assert_eq!(draws[0], draws[2]);
    let unbound = session_over(&plain, None);
    let draw = unbound.evaluate().unwrap().json;
    assert!(!draw.contains("__reactive"), "{draw}");
    unbound.dispose();
}

/// A prompt spec given with a defined library still decides the bindings,
/// as before: one whose signature marks no binding leaves the prop unbound.
#[test]
fn an_explicit_prompt_spec_decides_the_bindings_of_a_defined_library() {
    let defined = define(&bound_input()).unwrap();
    let mut unbound = bound_input();
    unbound["components"][0]["props"][1]["binding"] = json!(false);
    let spec = define(&unbound).unwrap().prompt_spec().unwrap();
    let session = session_over(&defined, Some(spec));
    let draw = session.evaluate().unwrap().json;
    assert!(!draw.contains("__reactive"), "{draw}");
    session.dispose();
}

fn prompt_golden() -> Value {
    parse_json(&read(&golden_dir().join("library-definition-prompts.json"))).unwrap()
}

/// Through the binding, a defined library's prompt and cloud config are
/// upstream's `library.prompt(options)` and `generateSystemPrompt({cloud:
/// true, library: library.toSpec(), promptOptions, instructions})`, byte for
/// byte, or its error with its message, for every golden entry.
#[test]
fn a_defined_librarys_prompt_and_cloud_config_are_upstreams() {
    let golden = prompt_golden();
    for entry in golden["prompts"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let library = define(&sample(name)["definition"]).unwrap();
        let options = entry.get("options").map(stringify_json);
        let prompt = library.prompt(options).unwrap().unwrap();
        assert_eq!(prompt, entry["prompt"].as_str().unwrap(), "{name}");
    }
    for entry in golden["cloud"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let library = define(&sample(name)["definition"]).unwrap();
        let options = entry.get("promptOptions").map(stringify_json);
        let instructions = entry
            .get("instructions")
            .map(|i| i.as_str().unwrap().to_owned());
        match (
            library.cloud_config(options, instructions),
            entry.get("error"),
        ) {
            (Ok(config), None) => {
                assert_eq!(config.unwrap(), entry["config"].as_str().unwrap(), "{name}")
            }
            (Err(OpenUiError::Prompt { reason }), Some(error)) => {
                assert_eq!(reason, error.as_str().unwrap(), "{name}")
            }
            (other, _) => panic!("{name}: {other:?}"),
        }
    }
}

/// A prompt's options and a cloud config's instructions are host input,
/// held to the limits the library was made with; options that are not JSON
/// are refused as InvalidPromptSpec; a library from a JSON Schema has no
/// prompt of its own.
#[test]
fn prompt_options_are_held_to_the_librarys_limits() {
    let limits = RuntimeLimits {
        max_json_bytes: 1000,
        ..RuntimeLimits::default()
    };
    let text = stringify_json(&bound_input());
    let library = ComponentLibrary::from_definition_with_limits(text, limits).unwrap();
    let at = format!(r#"{{"preamble":"{}"}}"#, "a".repeat(1000 - 15));
    assert_eq!(at.len(), 1000);
    assert!(
        library
            .prompt(Some(at.clone()))
            .unwrap()
            .unwrap()
            .starts_with("aaa")
    );
    let over = format!(r#"{{"preamble":"{}"}}"#, "a".repeat(1000 - 14));
    for result in [
        library.prompt(Some(over.clone())),
        library.cloud_config(Some(over), None),
        library.cloud_config(None, Some("i".repeat(1001))),
    ] {
        assert!(
            matches!(&result, Err(OpenUiError::LimitExceeded { reason }) if reason == "host input byte limit"),
            "{result:?}"
        );
    }
    assert!(library.cloud_config(None, Some("i".repeat(1000))).is_ok());
    for bad in ["{", "[".repeat(300).as_str()] {
        assert!(
            matches!(
                library.prompt(Some(bad.into())),
                Err(OpenUiError::InvalidPromptSpec { .. })
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        library.prompt(Some(r#"{"components":null}"#.into())),
        Err(OpenUiError::Prompt { reason }) if reason == "[generatePrompt] components is missing."
    ));
    let plain = ComponentLibrary::new(library.json_schema().unwrap()).unwrap();
    assert_eq!(plain.prompt(None).unwrap(), None);
    assert_eq!(plain.cloud_config(None, None).unwrap(), None);
}
