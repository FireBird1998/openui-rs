//! Owned and borrowed preparations grant the same credit and preserve lazy cuts.
use openui_runtime::*;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
#[test]
fn owned_preparation_matches_borrowed_bytes_flags_and_repeated_reads() {
    let library = openui_lang::Library::from_json_schema(
        &openui_lang::parse_json(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#).unwrap(),
    );
    let program = openui_lang::parse("q=Query(\"q\",{})\nroot=Box(q)", &library, None);
    let mut values = JsObject::new();
    values.define(
        "q",
        Value::Arr(std::sync::Arc::new(vec![Value::Str("x".into()); 200])),
    );
    for limits in [
        Limits::DEFAULT,
        Limits::DEFAULT.with_work(4),
        Limits::DEFAULT.with_depth(1),
        Limits::DEFAULT.with_result_size(8),
    ] {
        let borrowed = PreparedReferenceValues::with_limits(&values, limits);
        let owned = PreparedReferenceValues::owned_with_limits(values.clone(), limits);
        let store = Store::new();
        let expected = evaluate_tree(
            TreeInput::new(&program)
                .with_library(&library)
                .with_prepared_reference_values(&borrowed),
            &store,
        );
        for _ in 0..3 {
            let actual = evaluate_tree(
                TreeInput::new(&program)
                    .with_library(&library)
                    .with_prepared_reference_values(&owned),
                &store,
            );
            assert_eq!(actual.to_json().unwrap(), expected.to_json().unwrap());
            assert_eq!(
                (actual.size_limited, actual.depth_limited),
                (expected.size_limited, expected.depth_limited)
            );
        }
    }
}
