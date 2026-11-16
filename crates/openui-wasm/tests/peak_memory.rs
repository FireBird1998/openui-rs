//! Optional memory configuration through the native-test wasm session seam.
mod support;
use support::*;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#;

#[test]
fn optional_memory_field_reaches_runtime_evaluation_without_changing_defaults() {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let program = parse(
        "root=Box(@Each([1,2,3],i,[i,i]))".into(),
        library.clone(),
        None,
    )
    .program;
    let mut completed = None;
    for limit in [None, Some(1_000_000), Some(0)] {
        let session = RuntimeSession::with_limits(
            library.clone(),
            None,
            RuntimeLimits {
                evaluation: ParseLimits {
                    peak_memory: limit,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        session.set_program(program.clone()).unwrap();
        let before = session.snapshot().unwrap();
        let version = session.version().unwrap();
        let result = session.evaluate().unwrap();
        if limit == Some(0) {
            assert!(result.size_limited && !result.depth_limited);
        } else {
            assert!(!result.size_limited && !result.depth_limited);
            if let Some(bytes) = &completed {
                assert_eq!(&result.json, bytes);
            }
            completed = Some(result.json);
        }
        assert_eq!(session.snapshot().unwrap(), before);
        assert_eq!(session.version().unwrap(), version);
        session.dispose();
    }
}

#[test]
fn wasm_memory_limit_accessor_keeps_its_optional_default() {
    let mut limits = openui_wasm::ParseLimits::new();
    assert_eq!(limits.peak_memory(), None);
    limits.set_peak_memory(Some(123));
    assert_eq!(limits.peak_memory(), Some(123));
    limits.set_peak_memory(None);
    assert_eq!(limits.peak_memory(), None);
}
