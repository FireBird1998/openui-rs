//! Optional memory configuration through the exported native session.
use openui_ffi::*;

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
fn optional_memory_field_reaches_parse_time_evaluated_defaults() {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let source = "$s=@Each([1,2,3],i,[i,i])\nroot=Box(1)";
    let original = parse(source.into(), library.clone(), None);
    for allowance in [0, 1_000_000] {
        let result = parse_with_limits(
            source.into(),
            library.clone(),
            None,
            ParseLimits {
                peak_memory: Some(allowance),
                ..Default::default()
            },
        );
        assert_eq!(result.size_limited, allowance == 0);
        if allowance > 0 {
            assert_eq!(result.json, original.json);
        }
    }
}
