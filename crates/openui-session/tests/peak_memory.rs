//! Reference preparation cannot accept an evaluation-only allowance.
use openui_lang::{JsObject, Library};
use openui_runtime::PreparedReferenceValues;
use openui_session::{ComponentLibrary, OpenUiError, ParseLimits, RuntimeLimits, SessionCore};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn reference_preparation_refuses_memory_configuration_before_session_creation() {
    let schema = serde_json::json!({});
    let library = Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
        defined: None,
    });
    let mut existing = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let values = JsObject::new();
    let refs = PreparedReferenceValues::new(&values);
    existing.set(&refs, "$n".into(), Some("7".into())).unwrap();
    let before = existing.snapshot(&refs).unwrap();
    let version = existing.version(&refs).unwrap();
    for allowance in [0, 1, u64::MAX] {
        let result = SessionCore::new(
            library.clone(),
            None,
            RuntimeLimits {
                reference_preparation: ParseLimits {
                    peak_memory: Some(allowance),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert!(
            matches!(result, Err(OpenUiError::LimitExceeded { reason }) if reason.contains("referencePreparation.peakMemory"))
        );
        assert_eq!(existing.snapshot(&refs).unwrap(), before);
        assert_eq!(existing.version(&refs).unwrap(), version);
    }
    let mut fresh = SessionCore::new(library, None, RuntimeLimits::default()).unwrap();
    assert_eq!(fresh.snapshot(&refs).unwrap(), "{}");
}
