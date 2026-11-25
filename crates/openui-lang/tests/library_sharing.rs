//! A compiled component library shares each action shape between the params
//! that refer to it, where upstream's `inlineActionRefs` copies it into each
//! (#140): loading a library costs time and memory in step with its size,
//! through a library definition and through a ready-made JSON Schema alike.
//! And compiling, parsing with and dropping a library never recurse without
//! a bound, so all of it works on a 64 KiB thread, unoptimised too (ADR 0002).
//!
//! The shapes are the ones measured on #140 before sharing: a chain of
//! actions each naming the next, an action naming the next twice (which
//! doubled the copies with every action), and many params referring to one
//! large action. Each is made, parsed and dropped on a 64 KiB thread, with
//! its peak heap and allocation traffic measured; doubling the shape's size
//! must no more than about double them. What validation reports is checked
//! too: sharing changes no result (the differential harness compares every
//! result with the copying build; see the ticket).
//!
//! Kept in its own test binary, because a stack overflow aborts the process.
//! Run it unoptimised too, where frames are largest:
//! `CARGO_PROFILE_TEST_OPT_LEVEL=0 cargo test -p openui-lang --test library_sharing`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use openui_lang::{
    Library, LibraryDefinition, StreamParser, create_library, free_json, parse, parse_json,
    stringify_json,
};
use serde_json::{Map, Value as Json, json};

/// Before sharing, the chain of 2,000 took 4.7 GB: a cap far below that
/// makes the copying build fail here by running out, not by timing out.
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const SMALL_STACK: usize = 64 * 1024;

/// Runs `f` on a thread with a 64 KiB stack.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn action(name: &str) -> Json {
    json!({"kind": "action", "name": name})
}

/// A definition whose component `C` takes `go`, an action slot for `first`.
fn definition(first: &str, actions: Vec<Json>) -> Json {
    json!({
        "components": [{"name": "C", "description": "", "props": [
            {"name": "go", "type": action(first)}
        ]}],
        "actions": actions,
        "root": "C"
    })
}

/// A chain of `n` actions, each one's param `p` an action slot for the next;
/// the last one's a string.
fn chain(n: usize) -> Json {
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

/// `n` actions, each naming the next twice (`p` and `q`); the last one's
/// params are strings. Copying doubled with every action.
fn doubling(n: usize) -> Json {
    let actions = (0..n)
        .map(|i| {
            let ty = if i + 1 < n {
                action(&format!("A{}", i + 1))
            } else {
                json!({"kind": "string"})
            };
            json!({"name": format!("A{i}"), "description": "", "params": [
                {"name": "p", "type": ty.clone()},
                {"name": "q", "type": ty}
            ]})
        })
        .collect();
    definition("A0", actions)
}

/// `Many`, with `n` params each an action slot for `Big`, which has `n`
/// string params. Only the first few of each are required, so a program
/// need not write them all.
fn many(n: usize) -> Json {
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

/// A literal action of `name` whose `p` (and `q`, when the shape has it)
/// nests the next action's data `depth` levels down, ending in a number
/// where a string or an action belongs.
fn nested(depth: usize, with_q: bool) -> String {
    let mut value = "1".to_owned();
    for i in (1..=depth).rev() {
        let q = if with_q {
            format!(", q: {value}")
        } else {
            String::new()
        };
        value = format!("{{type: \"A{i}\", params: {{p: {value}{q}}}}}");
    }
    value
}

/// What one load of a library cost on the small stack.
#[derive(Debug, Clone, Copy)]
struct Cost {
    /// Peak heap above the start, in bytes.
    peak: usize,
    /// Bytes allocated in all, freed or not.
    traffic: usize,
    elapsed: Duration,
}

/// Make the library (from the definition, or from its JSON Schema), parse
/// `program` with it one-shot and streamed, and drop everything, on a
/// 64 KiB thread. Returns the cost and the one-shot result.
fn load(definition: &Json, via_schema: bool, program: &str) -> (Cost, String) {
    let definition = definition.clone();
    let program = program.to_owned();
    // The ready-made schema is the host's input, made beforehand.
    let schema = via_schema.then(|| {
        create_library(LibraryDefinition::from_json(&definition).unwrap())
            .unwrap()
            .to_json_schema()
    });
    on_small_stack(move || {
        test_alloc::reset_peak();
        let start_live = test_alloc::peak();
        let start_traffic = test_alloc::allocated_bytes();
        let started = Instant::now();
        let library = match &schema {
            Some(schema) => Library::from_json_schema(schema),
            None => {
                let read = LibraryDefinition::from_json(&definition).unwrap();
                let defined = create_library(read).unwrap();
                let library = defined.library();
                drop(defined);
                library
            }
        };
        let library = Arc::new(library);
        let result = parse(&program, &library, None).to_json();
        let mut stream = StreamParser::new(library.clone(), None);
        for piece in program.as_bytes().chunks(64) {
            stream.push(std::str::from_utf8(piece).unwrap());
        }
        assert_eq!(stream.finish().to_json(), result, "streamed as one-shot");
        drop(stream);
        drop(library);
        let cost = Cost {
            peak: test_alloc::peak() - start_live,
            traffic: test_alloc::allocated_bytes() - start_traffic,
            elapsed: started.elapsed(),
        };
        if let Some(schema) = schema {
            free_json(schema);
        }
        free_json(definition);
        (cost, result)
    })
}

fn errors(result: &str) -> Vec<String> {
    let result = parse_json(result).unwrap();
    let paths = result["meta"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{} {}",
                e["component"].as_str().unwrap(),
                e["path"].as_str().unwrap()
            )
        })
        .collect();
    free_json(result);
    paths
}

/// Load `shape(small)` and `shape(2 * small)` both ways, check what each
/// reports, and that doubling the shape at most about doubles its peak heap
/// and allocation traffic (the copying build grew 4 times for a chain, and
/// without bound for the doubling shape). Returns the costs, larger first.
fn grows_in_step(
    name: &str,
    shape: fn(usize) -> Json,
    small: usize,
    program: &str,
    expected_errors: &[String],
) -> Vec<(bool, usize, Cost)> {
    let mut costs = Vec::new();
    for via_schema in [false, true] {
        let mut by_size = Vec::new();
        for n in [small, 2 * small] {
            let definition = shape(n);
            let bytes = stringify_json(&definition).len();
            let (cost, result) = load(&definition, via_schema, program);
            free_json(definition);
            assert_eq!(errors(&result), expected_errors, "{name} {n}: {result}");
            eprintln!(
                "{name} n={n} ({bytes} bytes of definition) via {}: peak {} KB, traffic {} KB, {:?}",
                if via_schema { "schema" } else { "definition" },
                cost.peak / 1024,
                cost.traffic / 1024,
                cost.elapsed
            );
            by_size.push((bytes, cost));
            costs.push((via_schema, n, cost));
        }
        let [(small_bytes, a), (large_bytes, b)] = by_size[..] else {
            unreachable!()
        };
        // The input itself about doubles; allow for allocator rounding and
        // fixed costs, but nothing like the copying build's growth.
        let input = large_bytes as f64 / small_bytes as f64;
        let peak = b.peak as f64 / a.peak as f64;
        let traffic = b.traffic as f64 / a.traffic as f64;
        assert!(
            peak < 1.5 * input,
            "{name}: peak grew {peak:.2}x for {input:.2}x the input"
        );
        assert!(
            traffic < 1.5 * input,
            "{name}: traffic grew {traffic:.2}x for {input:.2}x the input"
        );
    }
    costs
}

/// A chain of 2,000 actions (194 KB of definition) took 4.7 GB and 4.6 s
/// before sharing, and dropping it overflowed a 64 KiB thread past about
/// 150 actions. Measured 2026-11-22 with this test, made, parsed and dropped
/// on a 64 KiB thread, through the definition / through the schema:
/// - 2,000 actions: peak heap 18.4 MB / 6.7 MB; 62 ms / 9 ms in the test
///   profile (opt-level 1), 183 ms / 59 ms at opt-level 0;
/// - 4,000 actions: 36.8 MB / 13.4 MB; 125 ms / 16 ms, 343 ms / 103 ms.
///
/// The build that copied shapes exceeded this binary's 256 MiB cap.
#[test]
fn a_chain_of_2000_actions_loads_in_step_with_its_size() {
    let program = format!("root = C(@A0({}))", nested(40, false));
    // The leaf, 40 actions down, is a number where an action belongs.
    let expected = [format!("A0 /p{}", "/params/p".repeat(40))];
    let costs = grows_in_step("chain", chain, 2000, &program, &expected);
    for (_, n, cost) in costs {
        assert!(
            cost.peak < 32 * 1024 * 1024 * n / 2000,
            "chain {n}: {cost:?}"
        );
    }
}

/// Each action naming the next twice doubled the copies with each action:
/// 18 actions (2.6 KB) took 2 GB, 30 would need terabytes. Shared, 30 and
/// 60 actions load like any small library. Measured 2026-11-22 as above,
/// through the definition / the schema:
/// - 30 actions (4.4 KB): peak 381 KB / 381 KB; 8 ms / 7 ms at opt-level 1,
///   52 ms / 49 ms at opt-level 0 (mostly the parse, the same at 60);
/// - 60 actions (8.8 KB): 646 KB / 503 KB; 10 ms / 7 ms, 55 ms / 50 ms.
#[test]
fn the_doubling_shape_loads_in_step_with_its_size_at_30_actions_and_beyond() {
    // Every leaf, 6 actions down both ways, is a number where an action
    // belongs: one error per leaf, 64 in all, depth first as upstream
    // validates, then `q`, which the step leaves out.
    let program = format!("root = C(@A0({}))", nested(6, true));
    let mut paths = vec![String::new()];
    for _ in 0..6 {
        paths = paths
            .iter()
            .flat_map(|path| [format!("{path}/params/p"), format!("{path}/params/q")])
            .collect();
    }
    let mut expected: Vec<String> = paths.iter().map(|path| format!("A0 /p{path}")).collect();
    expected.push("A0 /q".to_owned());
    let costs = grows_in_step("doubling", doubling, 30, &program, &expected);
    for (_, n, cost) in costs {
        assert!(cost.peak < 2 * 1024 * 1024, "doubling {n}: {cost:?}");
    }
}

/// 2,000 params referring to one action with 2,000 params (188 KB) took
/// 1.7 GB and 0.74 s before sharing. Measured 2026-11-22 as above, through
/// the definition / the schema:
/// - 1,000 of each (127 KB): peak 3.4 MB / 1.6 MB; 47 ms / 4 ms at
///   opt-level 1, 87 ms / 28 ms at opt-level 0;
/// - 2,000 of each (256 KB): 6.8 MB / 3.1 MB; 94 ms / 8 ms, 175 ms / 55 ms.
#[test]
fn many_params_referring_to_one_large_action_load_in_step_with_their_size() {
    let program = "root = C(@Many({type: \"Big\", params: {b0: \"x\", b1: 2}}, null, {type: \"Big\", params: {}}))";
    let expected = ["Many /m0/params/b1", "Many /m1", "Many /m2/params/b0"].map(String::from);
    let costs = grows_in_step("many", many, 1000, program, &expected);
    for (_, n, cost) in costs {
        assert!(
            cost.peak < 16 * 1024 * 1024 * n / 2000,
            "many {n}: {cost:?}"
        );
    }
}

#[cfg(unix)]
fn thread_cpu_time() -> Duration {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `time` is writable storage for the returned timespec.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) },
        0
    );
    Duration::new(
        time.tv_sec.try_into().unwrap(),
        time.tv_nsec.try_into().unwrap(),
    )
}

/// The `required` lookup is a set lookup, not a scan of the list for every
/// prop: compiling 40,000 required props costs about 8 times what 5,000
/// cost, where the scan made it about 64 times. The best of three runs of
/// each, using thread CPU time so descheduling cannot inflate the ratio.
/// Measured 2026-11-22: 1.3 ms and 10.7 ms (8.3
/// times) at opt-level 1, 9.6 ms and 77.8 ms (8.1 times) at opt-level 0;
/// with the scan, 26 ms and 1.48 s (57.6 times) at opt-level 1.
#[test]
#[cfg(unix)]
fn compiling_is_linear_in_the_number_of_props() {
    let schema = |n: usize| {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for i in 0..n {
            properties.insert(format!("p{i}"), json!({"type": "string"}));
            required.push(Json::String(format!("p{i}")));
        }
        json!({"$defs": {"Wide": {"properties": properties, "required": required}}})
    };
    let best = |schema: &Json| {
        (0..3)
            .map(|_| {
                let started = thread_cpu_time();
                let library = Library::from_json_schema(schema);
                let elapsed = thread_cpu_time() - started;
                assert!(
                    library
                        .component("Wide")
                        .unwrap()
                        .params
                        .iter()
                        .all(|p| p.required)
                );
                elapsed
            })
            .min()
            .unwrap()
    };
    let small = schema(5_000);
    let large = schema(40_000);
    let (a, b) = (best(&small), best(&large));
    let ratio = b.as_secs_f64() / a.as_secs_f64();
    eprintln!("5,000 props {a:?}, 40,000 props {b:?}: {ratio:.1}x");
    assert!(ratio < 24.0, "8x the props took {ratio:.1}x as long");
    free_json(small);
    free_json(large);
}

fn library_with_default(default: Json) -> Library {
    let mut schema = json!({"$defs": {"A": {"properties": {"value": {}}, "required": ["value"]}}});
    schema["$defs"]["A"]["properties"]["value"]["default"] = default;
    let library = Library::from_json_schema(&schema);
    free_json(schema);
    library
}

#[test]
fn a_prop_default_keeps_its_serde_byte_charge() {
    use openui_lang::{Limits, parse_with_limits};

    let default = json!({"quoted\"key": [1.0, -0.0, u64::MAX, null, true, false, "leaf", "\u{100000}\n", {}, []]});
    let library = library_with_default(default);
    let parse_at = |bytes| {
        parse_with_limits(
            "root = A()",
            &library,
            None,
            Limits::default().with_result_size(bytes),
        )
    };
    // Measured before #215. JS renders several of these scalars with different lengths.
    assert!(parse_at(120).size_limited);
    assert!(!parse_at(121).size_limited);
}

#[test]
fn a_deep_prop_default_is_applied_on_a_small_stack() {
    on_small_stack(|| {
        let depth = 2_000;
        let default = (0..depth).fold(Json::from("leaf"), |inner, _| Json::Array(vec![inner]));
        let library = library_with_default(default);
        let result = parse("root = A()", &library, None);
        assert!(!result.size_limited && !result.depth_limited);
        assert!(result.meta.errors.is_empty());

        let mut value = result.root.as_ref().unwrap().props.get("value").unwrap();
        for _ in 0..depth {
            let openui_lang::Value::Arr(items) = value else {
                panic!("expected another array in the default");
            };
            let [inner] = items.as_slice() else {
                panic!("expected a single array item");
            };
            value = inner;
        }
        assert!(matches!(value, openui_lang::Value::Str(s) if s == "leaf"));
    });
}

/// A prop schema nested to the core JSON reader's limit (800 levels at the
/// default depth limit), of objects and of arrays: read, compiled, parsed
/// with a value as deep, and dropped on a 64 KiB thread. Before #140 the
/// compiled library of a 256-deep object overflowed 64 KiB at opt-level 0.
#[test]
fn a_schema_nested_to_the_readers_limit_compiles_parses_and_drops_on_a_small_stack() {
    for shape in ["object", "array"] {
        on_small_stack(move || {
            // The document's own levels: root, $defs, Box, properties.
            let levels = 800 - 4;
            let (open, close, value) = match shape {
                "object" => (
                    r#"{"type":"object","properties":{"a":"#,
                    "}}",
                    ("{a: ", "}"),
                ),
                _ => (r#"{"type":"array","items":"#, "}", ("[", "]")),
            };
            // The prop's schema is at level 5, under the document's own four
            // (root, $defs, Box, properties), and may hold 796 levels with
            // its leaf: an object level is two JSON levels (it and its
            // `properties`), an array level one.
            let count = if shape == "object" { 397 } else { 795 };
            let _ = levels;
            let text = format!(
                r#"{{"$defs":{{"Box":{{"properties":{{"v":{}{{"type":"string"}}{}}}}}}}}}"#,
                open.repeat(count),
                close.repeat(count)
            );
            let schema = parse_json(&text).unwrap();
            let library = Arc::new(Library::from_json_schema(&schema));
            let program = format!(
                "root = Box({}1{})",
                value.0.repeat(200),
                value.1.repeat(200)
            );
            let result = parse(&program, &library, None);
            assert!(
                !result.meta.errors.is_empty(),
                "the leaf is a number, not a string"
            );
            drop(result);
            drop(library);
            free_json(schema);
        });
    }
}

/// A library definition at the type depth limit (256), of objects and of
/// arrays: read, made, compiled, parsed and dropped, the definition, the
/// defined library and the compiled library, on a 64 KiB thread. Before #140
/// the definition's drop overflowed 64 KiB at opt-level 0 at about 230
/// nested objects.
#[test]
fn a_definition_at_the_depth_limit_compiles_parses_and_drops_on_a_small_stack() {
    for kind in ["object", "array"] {
        on_small_stack(move || {
            let mut ty = json!({"kind": "string"});
            for _ in 1..256 {
                let mut next = Map::new();
                next.insert("kind".into(), kind.into());
                if kind == "object" {
                    let mut field = Map::new();
                    field.insert("name".into(), "a".into());
                    field.insert("type".into(), ty);
                    next.insert("fields".into(), Json::Array(vec![Json::Object(field)]));
                } else {
                    next.insert("items".into(), ty);
                }
                ty = Json::Object(next);
            }
            let mut prop = Map::new();
            prop.insert("name".into(), "v".into());
            prop.insert("type".into(), ty);
            // serde_json's own drop recurses: free the built JSON with a loop.
            let prop = Json::Object(prop);
            let text = format!(
                r#"{{"components":[{{"name":"Box","description":"","props":[{}]}}]}}"#,
                stringify_json(&prop)
            );
            free_json(prop);
            let json = parse_json(&text).unwrap();
            let read = LibraryDefinition::from_json(&json).unwrap();
            free_json(json);
            let defined = create_library(read).unwrap();
            let library = defined.library();
            let result = parse("root = Box(1)", &library, None);
            assert_eq!(result.meta.errors.len(), 1, "{}", result.to_json());
            drop(result);
            drop(library);
            drop(defined);
        });
    }
}
