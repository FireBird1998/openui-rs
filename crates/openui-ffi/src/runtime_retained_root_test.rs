//! Sample retained heap on its owning worker, between commands.
use super::*;
use crate::parse;

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
    s.set_program(program.clone()).unwrap();
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
    // Warm evaluator/TLS/cache setup, then clear the first root. Each sample
    // executes on the worker after the previous command and its captures drop.
    // reset_peak reads this worker's live account, never process-wide traffic
    // from libtest, the caller, or callback threads.
    drop(s.evaluate().unwrap());
    s.set_program(program).unwrap();
    let live = || {
        s.call(|_, _| {
            test_alloc::reset_peak();
            Ok(test_alloc::peak())
        })
        .unwrap()
    };
    let before = live();
    let tree = s.evaluate().unwrap();
    assert!(!tree.size_limited);
    let json_bytes = tree.json.len();
    drop(tree);
    let retained = live().saturating_sub(before);
    eprintln!("retained root: {retained} heap bytes / {json_bytes} JSON bytes");
    // Representative shape only. Allow 5% (at least 4 KiB) layout tolerance
    // over the original 8x envelope; a second retained root still exceeds it.
    assert!(retained > json_bytes);
    let envelope = json_bytes * 8;
    assert!(retained < envelope + (envelope / 20).max(4 * 1024));
    let empty = parse("".into(), library, None).program;
    s.set_program(empty).unwrap();
    assert!(live() < before + 64 * 1024);
}
