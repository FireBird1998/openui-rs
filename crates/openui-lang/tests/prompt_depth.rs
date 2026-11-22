//! Deep prompt rendering needs the 256 MiB allocation cap: the raised-depth
//! object schema retains nested copies while formatting its type signature.
use openui_lang::{
    Limits, free_json, generate_prompt, generate_prompt_with_limits, generate_system_prompt,
    generate_system_prompt_with_limits,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

/// A prompt spec with one tool whose input schema nests `levels` objects deep.
fn deep_spec(levels: usize) -> serde_json::Value {
    let mut schema = serde_json::json!({"type": "string"});
    for _ in 0..levels {
        schema = serde_json::json!({"type": "object", "properties": {"a": schema}});
    }
    serde_json::json!({"components": {}, "tools": [{"name": "t", "inputSchema": schema}]})
}

#[test]
fn a_prompt_spec_nests_to_the_depth_limit() {
    // 900 schema levels cross the default JSON depth limit of 800. Rendering
    // with a raised limit retains nested schema copies; keep this exercise in
    // a DEFAULT binary rather than weakening the depth shape to fit LIMITS.
    let spec = deep_spec(450);
    let refused = generate_system_prompt(&spec).unwrap_err();
    assert!(
        refused.to_string().contains("nests deeper than 800"),
        "{refused}"
    );
    assert!(generate_prompt_with_limits(&spec, Limits::default()).is_err());

    let deep = Limits::default().with_depth(1_000);
    let prompt = generate_system_prompt_with_limits(&spec, deep).unwrap();
    assert_eq!(prompt, generate_prompt(&spec));
    free_json(spec);

    // A lowered limit refuses shallower specs: 44 JSON levels at depth 4.
    let spec = deep_spec(30);
    assert!(generate_system_prompt(&spec).is_ok());
    let shallow = Limits::default().with_depth(4);
    assert!(generate_system_prompt_with_limits(&spec, shallow).is_err());
}

#[test]
fn prompts_accept_the_exact_json_depth_limit() {
    // At depth=0 the JSON limit is 32 containers. The spec is one, so
    // 31 nested arrays fit; adding a 32nd array must be refused.
    let limits = Limits::default().with_depth(0);
    for (arrays, accepted) in [(31, true), (32, false)] {
        let mut nested = serde_json::Value::Null;
        for _ in 0..arrays {
            nested = serde_json::json!([nested]);
        }
        let spec = serde_json::json!({"components": {}, "extra": nested});
        for result in [
            generate_prompt_with_limits(&spec, limits),
            generate_system_prompt_with_limits(&spec, limits),
        ] {
            if accepted {
                assert_eq!(result.unwrap(), generate_prompt(&spec));
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("nests deeper than 32 levels")
                );
            }
        }
    }
}

/// A spec nested to the JSON depth limit (800 levels by default), its depth
/// in its components, in the library the generator copies or in a cloud
/// request's library, makes its prompt on a 64 KiB thread: the generator
/// copies and drops parts of the spec with serde_json, which recurses once a
/// level, on a segment sized for the spec's depth (#203). Run it unoptimised
/// too (`cargo test --profile dev -p openui-lang --test prompt_depth`).
#[test]
fn a_spec_at_the_depth_limit_generates_on_a_small_stack() {
    for (open, close) in [(r#"{"a":"#, "}"), ("[", "]")] {
        let deep = format!("{}1{}", open.repeat(797), close.repeat(797));
        let texts = [
            format!(r#"{{"components":{{"x":{deep}}}}}"#),
            format!(r#"{{"library":{{"components":{{"x":{deep}}}}}}}"#),
            format!(
                r#"{{"cloud":true,"library":{{"schema":{{"$defs":{{"C":{{}}}}}},"extra":{deep}}}}}"#
            ),
        ];
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                for text in texts {
                    let spec = openui_lang::parse_json(&text).unwrap();
                    assert!(generate_system_prompt(&spec).is_ok(), "{}", &text[..40]);
                    if !text.contains("\"library\"") {
                        assert!(generate_prompt_with_limits(&spec, Limits::default()).is_ok());
                        generate_prompt(&spec);
                    }
                    free_json(spec);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
