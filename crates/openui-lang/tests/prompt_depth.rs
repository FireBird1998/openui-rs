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
    serde_json::json!({"tools": [{"name": "t", "inputSchema": schema}]})
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
