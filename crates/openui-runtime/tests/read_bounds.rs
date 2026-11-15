//! Reads inspect bounded state and reject impossible names before hashing them.
use openui_runtime::{JsObject, Limits, Store, Value, get_field_value};
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
#[test]
fn impossible_lookup_names_allocate_nothing_and_cannot_read_retained_state() {
    let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(1024));
    store.set("$n", Some(Value::Num(1.))).unwrap();
    let name = "n".repeat(1_000_000);
    let snapshot = store.snapshot();
    let version = store.version();
    let before = test_alloc::allocation_calls();
    for _ in 0..1000 {
        assert!(store.get(&name).is_none());
        assert!(!store.is_pristine(&name));
        assert!(get_field_value(&store, Some(&name), "n").is_none());
        assert!(get_field_value(&store, Some("$n"), &name).is_none());
        assert_eq!(store.version(), version);
    }
    assert_eq!(test_alloc::allocation_calls(), before);
    assert!(Arc::ptr_eq(&snapshot, &store.snapshot()));
}
#[test]
fn field_reads_only_copy_one_admitted_shallow_object() {
    for n in [16, 32, 64, 128] {
        let mut fields = JsObject::new();
        for i in 0..n {
            fields.define(format!("f{i}"), Value::Str("v".repeat(8)));
        }
        let mut store = Store::new();
        store
            .set("form", Some(Value::Obj(Arc::new(fields))))
            .unwrap();
        let before = test_alloc::allocation_calls();
        assert_eq!(
            get_field_value(&store, Some("form"), "f0"),
            Some(Value::Str("v".repeat(8)))
        );
        let allocated = test_alloc::allocation_calls() - before;
        assert!(allocated <= 4 * n + 16, "n={n} allocations={allocated}");
    }
}

#[test]
fn inherited_form_prototypes_survive_a_cap_smaller_than_the_fixed_property_name() {
    let mut store = Store::with_limits(Limits::DEFAULT.with_result_size(8));
    store
        .set("f", Some(Value::Obj(Arc::new(JsObject::new()))))
        .unwrap();
    assert!(
        matches!(get_field_value(&store,Some("f"),"__proto__").as_ref(),Some(Value::Obj(o)) if o.is_empty())
    );
}
