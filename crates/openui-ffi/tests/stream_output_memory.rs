//! Isolated allocation measurement of retained outputs, through the exported API.
use openui_ffi::{ComponentLibrary, ParseLimits, StreamSession};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn fifty_one_unchanged_stream_outputs_share_one_program_and_only_retain_their_json() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#.into()).unwrap();
    let stream = StreamSession::new(library, None);
    let mut outputs = Vec::with_capacity(51);
    let source = format!("root=Box(\"{}\")", "x".repeat(50_000));
    outputs.push(stream.push(source.clone()));
    let before = test_alloc::live_bytes();
    for i in 0..50 {
        outputs.push(match i % 3 {
            0 => stream.result(),
            1 => stream.push(String::new()),
            _ => stream.push_all(Vec::new()),
        });
    }
    let retained = test_alloc::live_bytes() - before;
    let json: usize = outputs.iter().skip(1).map(|o| o.json.len()).sum();
    println!(
        "51 unchanged 50KB outputs: additional live={retained}, JSON={json}, overhead={}",
        retained.saturating_sub(json)
    );
    assert!(
        retained <= json + 16 * 1024,
        "retained parse-result copies: live={retained}, JSON={json}"
    );
    assert!(
        outputs
            .iter()
            .all(|o| Arc::ptr_eq(&o.program, &outputs[0].program))
    );
    // No-op set and repeated finish must share too; completion semantics remain
    // the parser's responsibility, and changed outputs have new program identity.
    let finished = stream.finish();
    let again = stream.finish();
    assert!(Arc::ptr_eq(&finished.program, &again.program));
    let same = stream.set(format!("root=Box(\"{}\")", "x".repeat(50_000)));
    assert!(Arc::ptr_eq(&same.program, &stream.result().program));
    let changed = stream.set("root=Box(1)".into());
    assert!(!Arc::ptr_eq(&changed.program, &again.program));

    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#.into()).unwrap();
    let cut = StreamSession::with_limits(
        library,
        None,
        ParseLimits {
            program_size: Some(10),
            ..Default::default()
        },
    );
    let first = cut.push("root=Box(\"too long\")".into());
    assert!(first.size_limited);
    assert!(Arc::ptr_eq(
        &first.program,
        &cut.push("ignored".into()).program
    ));
    assert!(Arc::ptr_eq(
        &first.program,
        &cut.push_all(vec!["also ignored".into()]).program
    ));
}
