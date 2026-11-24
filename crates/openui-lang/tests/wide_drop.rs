use std::sync::Arc;

use openui_lang::{JsObject, ParseResult, Value};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn refusing_wide_values_uses_bounded_heap() {
    let mut samples = Vec::with_capacity(4);
    for width in [65_536, 1_000_000] {
        samples.push(("array", width, measure_refusal(|| wide_array(width))));
        samples.push(("object", width, measure_refusal(|| wide_object(width))));
    }

    for (shape, width, (peak, allocated)) in &samples {
        eprintln!(
            "{shape} ({width} children): {peak} bytes peak above input, {allocated} bytes allocated"
        );
    }
    for (shape, width, (peak, allocated)) in samples {
        const AUXILIARY_BOUND: usize = 16 * 1024;
        assert!(
            peak <= AUXILIARY_BOUND && allocated <= AUXILIARY_BOUND,
            "{shape} ({width} children): {peak} bytes peak above input, {allocated} bytes allocated"
        );
    }
}

fn wide_array(width: usize) -> Value {
    Value::Arr(Arc::new((0..width).map(child).collect()))
}

fn wide_object(width: usize) -> Value {
    let mut object = JsObject::with_capacity(width);
    for key in 0..width {
        object.define(key.to_string(), child(key));
    }
    Value::Obj(Arc::new(object))
}

fn child(index: usize) -> Value {
    let leaf = Value::Str("child".into());
    if index.is_multiple_of(4) {
        Value::Arr(Arc::new(vec![leaf]))
    } else {
        leaf
    }
}

fn measure_refusal(build: impl FnOnce() -> Value) -> (usize, usize) {
    test_alloc::reset_peak();
    let initial_live = test_alloc::peak();
    let mut candidate = ParseResult::default();
    candidate.state_declarations.define("wide", build());

    // A refusal discards its unpublished result. Build it directly so that the
    // measured drop has unique ownership and excludes parsing and rollback.
    test_alloc::reset_peak();
    let input_live = test_alloc::peak();
    let before = test_alloc::allocated_bytes();
    drop(candidate);
    let allocated = test_alloc::allocated_bytes() - before;
    let peak = test_alloc::peak() - input_live;

    test_alloc::reset_peak();
    assert!(input_live > initial_live);
    assert_eq!(test_alloc::peak(), initial_live, "refused result leaked");
    (peak, allocated)
}
