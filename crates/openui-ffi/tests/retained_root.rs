//! Isolated binary: no concurrent test can contaminate live-heap measurements.
use openui_ffi::*;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
#[test]
fn retained_root_heap_is_measured_and_released_on_program_replacement() {
    let library = ComponentLibrary::new(
        r#"{"$defs":{"Box":{"properties":{"value":{},"action":{}}}}}"#.into(),
    )
    .unwrap();
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    let program = parse(
        "$n=0\n$rows=[]\nroot=Box(@Each($rows,x,Box(x,@Set($n,x))))".into(),
        library.clone(),
        None,
    )
    .program;
    s.set_program(program).unwrap();
    s.initialize(None).unwrap();
    s.set(
        "$rows".into(),
        Some(format!(
            "[{}]",
            (0..10_000)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )),
    )
    .unwrap();
    let before = test_alloc::live_bytes();
    let tree = s.evaluate().unwrap();
    assert!(!tree.size_limited);
    let json_bytes = tree.json.len();
    drop(tree);
    let retained = test_alloc::live_bytes().saturating_sub(before);
    eprintln!("retained root: {retained} heap bytes / {json_bytes} JSON bytes");
    // Pin this representative shape, not an unsupported universal heap ratio.
    assert!(retained > json_bytes);
    assert!(retained < json_bytes * 8);
    let empty = parse("".into(), library, None).program;
    s.set_program(empty).unwrap();
    assert!(test_alloc::live_bytes() < before + 64 * 1024);
}
