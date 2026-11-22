//! What a library made from a library definition holds for the bindings
//! (#137): the JSON Schema it was compiled from, as deep as the definition's
//! types, is freed without recursion wherever the last holder drops it. The
//! core's own `Library` and `LibraryDefinition` still free themselves
//! recursively (#140), so each test keeps the compiled `Library` alive and
//! drops it, and makes the definition, on a large stack.

use openui_session::{ParseLimits, RuntimeLimits, define_library};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const SMALL: usize = 64 * 1024;
const LARGE: usize = 1024 * 1024 * 1024;

/// One component whose one prop is `depth` arrays around a string.
fn deep_definition(depth: usize) -> String {
    let mut ty = String::from(r#"{"kind":"string"}"#);
    for _ in 1..depth {
        ty = format!(r#"{{"kind":"array","items":{ty}}}"#);
    }
    format!(
        r#"{{"components":[{{"name":"Deep","description":"","props":[{{"name":"v","type":{ty}}}]}}]}}"#
    )
}

fn on(stack: usize, f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

/// Makes a library at `depth`, then drops everything the binding holds of
/// it on a 64 KiB thread, the core's `Library` excepted.
fn held_schema_drops_on_a_small_stack(depth: usize) {
    let limits = RuntimeLimits {
        evaluation: ParseLimits {
            depth: Some(depth as u64),
            ..ParseLimits::default()
        },
        ..RuntimeLimits::default()
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    on(LARGE, move || {
        let library = define_library(&deep_definition(depth), &limits).unwrap();
        assert!(library.defined.as_ref().unwrap().json_schema.len() > depth * 20);
        sender.send(library).unwrap();
    });
    // The library keeps its definition, whose types drop with a loop too.
    let library = receiver.recv().unwrap();
    let compiled = library.inner.clone();
    // A second holder of the schema, as a session's copy of the library is:
    // whichever drops last frees it.
    let schema = library.schema.clone();
    let session_copy = openui_session::ComponentLibrary {
        inner: compiled.clone(),
        schema,
        defined: None,
    };
    on(SMALL, move || drop(library));
    on(SMALL, move || drop(session_copy));
    on(LARGE, move || drop(compiled));
}

/// At the default type depth limit, 256: the schema nests about 520 levels.
#[test]
fn the_held_schema_of_a_definition_at_the_default_limit_drops_on_a_64_kib_thread() {
    held_schema_drops_on_a_small_stack(256);
}

/// Under a raised depth limit, far past the default.
#[test]
fn the_held_schema_of_a_far_deeper_definition_drops_on_a_64_kib_thread() {
    held_schema_drops_on_a_small_stack(20_000);
}

fn conformance(file: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(file);
    openui_lang::parse_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A session over a defined library given no prompt spec takes its root
/// from the definition (#135), as upstream's renderer does (react-lang's
/// useOpenUIState creates its parser with `library.root`): its stream
/// parser gives upstream's result, byte for byte, for programs with and
/// without a `root` statement. Each case of
/// `conformance/golden/library-definition-roots.json` (the "scalars"
/// sample, root `Page`) replays the parser upstream made: with the library's
/// own root, through a session given no prompt spec; with another root,
/// through a session given a prompt spec naming it, which still wins; and
/// with none, through the same library as a JSON Schema with no prompt spec.
#[test]
fn a_session_over_a_defined_library_takes_its_root_from_the_definition() {
    let samples = conformance("golden/library-definitions.json");
    let sample = samples
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "scalars")
        .unwrap();
    let limits = RuntimeLimits::default();
    let definition = openui_lang::stringify_json(&sample["definition"]);
    let defined = std::sync::Arc::new(define_library(&definition, &limits).unwrap());
    let schema = defined.defined.as_ref().unwrap().json_schema.clone();
    let plain = std::sync::Arc::new(openui_session::library_from_schema(&schema, &limits).unwrap());
    let mut spec = openui_lang::parse_json(&defined.defined.as_ref().unwrap().prompt_spec).unwrap();
    spec["root"] = serde_json::json!("Heading");
    let heading_spec = openui_lang::stringify_json(&spec);
    let cases = conformance("golden/library-definition-roots.json");
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 12);
    let mut picked = 0;
    for case in cases {
        let program = case["program"].as_str().unwrap();
        let (library, spec) = match case.get("root").and_then(|r| r.as_str()) {
            Some("Page") => (&defined, None),
            Some("Heading") => (&defined, Some(heading_spec.clone())),
            None => (&plain, None),
            Some(other) => panic!("unexpected root {other}"),
        };
        let core = openui_session::SessionCore::new(library.clone(), spec, limits.clone()).unwrap();
        let ours = core
            .stream_parser(openui_lang::Limits::default())
            .set(program)
            .to_json();
        let ours = openui_lang::stringify_json(&openui_lang::parse_json(&ours).unwrap());
        assert_eq!(ours, openui_lang::stringify_json(&case["result"]), "{case}");
        if !case["result"]["root"].is_null() && !program.contains("root =") {
            picked += 1;
        }
    }
    // The root decided which statement drew in programs without `root`.
    assert!(picked >= 3, "{picked}");
}

/// `"x"` inside `depth` arrays, or `depth` objects keyed `a`.
fn nest(depth: usize, objects: bool) -> String {
    if objects {
        format!(r#"{}"x"{}"#, r#"{"a":"#.repeat(depth), "}".repeat(depth))
    } else {
        format!(r#"{}"x"{}"#, "[".repeat(depth), "]".repeat(depth))
    }
}

/// Prompt options as deep as the reader allows, nested arrays and nested
/// objects alike, are read, moved into the spec, copied by the cloud config
/// on a segment sized for their depth, and freed without recursion: on a
/// 64 KiB thread a prompt and a cloud config with them are made, or refused
/// as `Prompt` past the prompt's depth, or as `InvalidPromptSpec` past the
/// reader's, never a crash. Under the default limits (JSON 800 levels deep)
/// and a raised one (3,000, JSON 9,032 levels). The cloud config's copies
/// cost most unoptimised: run this with `--profile dev` too (TESTING.md).
#[test]
fn deep_prompt_options_cost_no_stack() {
    for (depth_limit, ok, refused) in [(None, 790, 5000), (Some(3000), 9000, 20_000)] {
        let limits = limits_with_depth(depth_limit);
        let library = define_library(
            r#"{"components":[{"name":"A","description":"","props":[]}],"root":"A"}"#,
            &limits,
        )
        .unwrap();
        on(SMALL, move || {
            let json_limit = depth_limit.unwrap_or(256) as usize * 3 + 32;
            for objects in [false, true] {
                for depth in [ok, json_limit - 2, json_limit, json_limit + 1, refused] {
                    let options = format!(r#"{{"examples":{}}}"#, nest(depth, objects));
                    for result in [
                        library.prompt(Some(&options)),
                        library.cloud_config(Some(&options), Some("i")),
                    ] {
                        match result {
                            Ok(Some(_)) => assert!(depth < json_limit, "{depth}"),
                            Err(openui_session::OpenUiError::Prompt { reason }) => {
                                assert!(depth > ok, "{depth}");
                                assert!(reason.contains("nests deeper than"), "{reason}")
                            }
                            Err(openui_session::OpenUiError::InvalidPromptSpec { .. }) => {
                                assert!(depth >= json_limit - 2, "{depth}")
                            }
                            other => panic!("{depth}: {other:?}"),
                        }
                    }
                    if depth == ok {
                        assert!(library.prompt(Some(&options)).unwrap().is_some());
                        let config = library.cloud_config(Some(&options), None).unwrap();
                        assert!(config.is_some(), "{depth}");
                    }
                }
            }
        });
    }
}

fn limits_with_depth(depth: Option<u64>) -> RuntimeLimits {
    RuntimeLimits {
        evaluation: ParseLimits {
            depth,
            ..ParseLimits::default()
        },
        ..RuntimeLimits::default()
    }
}

/// One component whose one prop is `depth` levels of inline objects (each
/// one field `f`) around a string.
fn deep_object_definition(depth: usize) -> String {
    let mut ty = String::from(r#"{"kind":"string"}"#);
    for _ in 1..depth {
        ty = format!(r#"{{"kind":"object","fields":[{{"name":"f","type":{ty}}}]}}"#);
    }
    format!(
        r#"{{"components":[{{"name":"Deep","description":"","props":[{{"name":"v","type":{ty}}}]}}],"root":"Deep"}}"#
    )
}

/// A definition whose types nest to the depth limit, arrays or objects,
/// makes its prompt and its cloud config (which copies the deep JSON
/// Schema) on a 64 KiB thread, under the default limit (256) and a raised
/// one (3,000). Run with `--profile dev` too (TESTING.md).
#[test]
fn deep_definitions_make_their_cloud_config_on_a_small_stack() {
    for depth in [256usize, 3000] {
        let limits = limits_with_depth(Some(depth as u64));
        for objects in [false, true] {
            let text = if objects {
                deep_object_definition(depth)
            } else {
                let mut text = deep_definition(depth);
                text.pop();
                text + r#","root":"Deep"}"#
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let limits = limits.clone();
            on(LARGE, move || {
                sender
                    .send(define_library(&text, &limits).unwrap())
                    .unwrap();
            });
            let library = receiver.recv().unwrap();
            on(SMALL, move || {
                assert!(library.prompt(None).unwrap().unwrap().contains("Deep(v"));
                let config = library.cloud_config(None, None).unwrap().unwrap();
                assert!(
                    config.starts_with("]]>openui:config\n"),
                    "{depth} {objects}"
                );
                drop(library);
            });
        }
    }
}
