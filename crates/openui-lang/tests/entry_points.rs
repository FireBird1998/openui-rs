//! The one `Limits` value on the entry points besides parsing (ADR 0001):
//! merging, converting back to source, reading JSON, parsing one expression
//! and generating a system prompt. A hostile input to each is bounded: cut
//! and flagged where the entry point has flags, refused with its error where
//! it has an error, and a raised limit lets it through.

use std::time::{Duration, Instant};

use openui_lang::{
    Library, Limits, free_json, generate_prompt, json_to_openui, json_to_openui_with_limits,
    merge_statements, merge_statements_with_limits, parse_expression, parse_expression_with_limits,
    parse_json, parse_json_with_limits, parse_with_limits, tokenize,
};

/// A prompt spec of `n` components, each listed in a group of its own.
fn grouped_spec(n: usize) -> serde_json::Value {
    let components: serde_json::Map<String, serde_json::Value> = (0..n)
        .map(|i| {
            (
                format!("C{i}"),
                serde_json::json!({"signature": format!("C{i}()")}),
            )
        })
        .collect();
    let groups: Vec<serde_json::Value> = (0..n)
        .map(|i| serde_json::json!({"name": format!("g{i}"), "components": [format!("C{i}")]}))
        .collect();
    serde_json::json!({"components": components, "componentGroups": groups})
}

/// The fastest of three runs.
fn fastest(f: impl Fn()) -> Duration {
    (0..3)
        .map(|_| {
            let started = Instant::now();
            f();
            started.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn a_prompt_costs_time_linear_in_its_component_groups() {
    let small = grouped_spec(4_000);
    let large = grouped_spec(16_000);
    let ratio = fastest(|| drop(generate_prompt(&large))).as_secs_f64()
        / fastest(|| drop(generate_prompt(&small))).as_secs_f64();
    // Four times the components: about 4 when linear, 16 when quadratic.
    assert!(ratio < 8.0, "16,000 components cost {ratio:.1} times 4,000");
}

fn library() -> Library {
    Library::from_json_schema(&serde_json::json!({"$defs": {
        "Stack": {"properties": {"children": {"type": "array"}}}
    }}))
}

/// The root of `root = Stack([[...[1]...]])`, `levels` lists deep, parsed and
/// read back under `limits`.
fn nested_root(levels: usize, limits: Limits) -> serde_json::Value {
    let program = format!(
        "root = Stack({}1{})",
        "[".repeat(levels),
        "]".repeat(levels)
    );
    let result = parse_with_limits(&program, &library(), None, limits);
    assert!(!result.depth_limited && !result.size_limited);
    let mut json = parse_json_with_limits(&result.to_json(), limits).unwrap();
    json["root"].take()
}

#[test]
fn converting_back_to_source_nests_to_the_depth_limit() {
    let deep = Limits::default().with_depth(2_000);
    let root = nested_root(900, deep);
    // Past the default's 800 JSON levels: refused, as before.
    assert!(json_to_openui(&root, &library(), None).is_err());
    assert!(json_to_openui_with_limits(&root, &library(), None, Limits::default()).is_err());
    // Under the raised limit it converts back, and parses to the same tree.
    let source = json_to_openui_with_limits(&root, &library(), None, deep).unwrap();
    assert_eq!(nested_root(900, deep), {
        let again = parse_with_limits(&source, &library(), None, deep);
        parse_json_with_limits(&again.to_json(), deep).unwrap()["root"].take()
    });
    free_json(root);

    // And a lowered limit refuses shallower trees: 44 JSON levels at depth 4.
    let root = nested_root(60, Limits::default());
    assert!(json_to_openui(&root, &library(), None).is_ok());
    let shallow = Limits::default().with_depth(4);
    let refused = json_to_openui_with_limits(&root, &library(), None, shallow).unwrap_err();
    assert!(
        refused.to_string().contains("nests too deeply"),
        "{refused}"
    );
}

/// JSON arrays nested `levels` deep around a 1.
fn nested_json(levels: usize) -> String {
    format!("{}1{}", "[".repeat(levels), "]".repeat(levels))
}

#[test]
fn reading_json_nests_three_levels_per_depth_level_and_32_more() {
    // 800 at the default depth of 256, as before.
    assert!(parse_json(&nested_json(800)).is_ok());
    assert!(parse_json(&nested_json(801)).is_err());
    let limits = Limits::default();
    assert!(parse_json_with_limits(&nested_json(800), limits).is_ok());
    assert!(parse_json_with_limits(&nested_json(801), limits).is_err());

    let shallow = Limits::default().with_depth(4);
    assert!(parse_json_with_limits(&nested_json(44), shallow).is_ok());
    let refused = parse_json_with_limits(&nested_json(45), shallow).unwrap_err();
    assert!(refused.to_string().contains("deeper than 44"), "{refused}");

    let deep = Limits::default().with_depth(10_000);
    let json = parse_json_with_limits(&nested_json(20_000), deep).unwrap();
    free_json(json);
}

#[test]
fn a_deep_json_value_is_freed_on_a_small_stack() {
    let json =
        parse_json_with_limits(&nested_json(30_000), Limits::default().with_depth(10_000)).unwrap();
    std::thread::Builder::new()
        .name(format!(
            "{}::worker",
            std::thread::current().name().unwrap_or("test")
        ))
        .stack_size(64 * 1024)
        .spawn(move || free_json(json))
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn parsing_one_expression_stops_at_the_depth_limit() {
    let tokens = tokenize(&format!("{}1{}", "[".repeat(10), "]".repeat(10)));

    let (cut, limited) = parse_expression_with_limits(&tokens, Limits::default().with_depth(4));
    assert!(limited);
    assert_ne!(cut, parse_expression(&tokens));

    let (whole, limited) = parse_expression_with_limits(&tokens, Limits::default());
    assert!(!limited);
    assert_eq!(whole, parse_expression(&tokens));
}

#[test]
fn merging_reads_each_input_up_to_the_program_size_limit() {
    let existing = "root = Stack([a, b])\na = Text(\"A\")\nb = Text(\"B\")";
    let patch = "a = Text(\"patched\")\nc = Text(\"C\")";
    // Room for the existing program up to `a`'s line, and the patch's first line.
    let limits = Limits::default().with_program_size(34);
    let cut = merge_statements_with_limits(existing, patch, None, limits);
    assert!(cut.size_limited);
    assert_eq!(
        cut.text,
        merge_statements(&existing[..34], &patch[..34.min(patch.len())], None)
    );

    let whole = merge_statements_with_limits(existing, patch, None, Limits::default());
    assert!(!whole.size_limited && !whole.depth_limited);
    assert_eq!(whole.text, merge_statements(existing, patch, None));
}

#[test]
fn merging_reads_expressions_to_the_depth_limit() {
    // `b` is reached only from the bottom of `root`'s nesting, 10 levels down.
    let existing = format!(
        "root = Stack({}b{})\nb = Text(\"B\")",
        "[".repeat(10),
        "]".repeat(10)
    );
    let patch = "c = Text(\"C\")";

    let shallow =
        merge_statements_with_limits(&existing, patch, None, Limits::default().with_depth(4));
    assert!(shallow.depth_limited);
    assert!(!shallow.text.contains("b = Text"), "{}", shallow.text);

    let deep = merge_statements_with_limits(&existing, patch, None, Limits::default());
    assert!(!deep.depth_limited);
    assert!(deep.text.contains("b = Text(\"B\")"), "{}", deep.text);
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn serializer_accepts_exactly_the_configured_nesting_boundary() {
    // At depth zero, serialization allows 32 nested value visits. An empty
    // array adds one visit without increasing the JSON input's nesting.
    let limits = Limits::default().with_depth(0);
    let mut value = serde_json::json!([]);
    for _ in 0..31 {
        value = serde_json::json!([value]);
    }
    let root =
        serde_json::json!({"type":"element", "typeName":"Stack", "props":{"children":value}});
    assert!(json_to_openui_with_limits(&root, &library(), None, limits).is_ok());
    let mut root = root;
    root["props"]["children"] = serde_json::json!([root["props"]["children"].take()]);
    let error = json_to_openui_with_limits(&root, &library(), None, limits).unwrap_err();
    assert_eq!(
        error.to_string(),
        "[jsonToOpenUI] the element tree nests too deeply"
    );
}

#[test]
fn evaluated_defaults_reuse_the_room_their_expressions_occupied() {
    let program = format!(
        "root = Stack([])\n{}",
        (0..50)
            .map(|i| format!("$x{i} = 1 + 2\n"))
            .collect::<String>()
    );
    let result = parse_with_limits(
        &program,
        &library(),
        None,
        // The materialized expressions fill this limit. Evaluation must
        // reclaim their room before replacing them with the numeric defaults.
        Limits::default().with_result_size(6320),
    );
    assert!(!result.size_limited);
    let json = parse_json(&result.to_json()).unwrap();
    for i in 0..50 {
        assert_eq!(json["stateDeclarations"][format!("$x{i}")], 3);
    }
}

#[test]
fn expression_depth_counts_chains_and_releases_siblings() {
    let limits = Limits::default().with_depth(4);
    for (text, limited) in [
        ("1 + 2 + 3 + 4", false),
        ("1 + 2 + 3 + 4 + 5", true),
        ("[[1], [2], [3], [4]]", false),
    ] {
        let (_, got) = parse_expression_with_limits(&tokenize(text), limits);
        assert_eq!(got, limited, "{text}");
    }
}
