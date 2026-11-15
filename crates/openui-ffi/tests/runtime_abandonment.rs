//! Single-test binary so process-wide live-byte samples include the worker only.
use openui_ffi::{ComponentLibrary, RuntimeSession, parse};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);

#[test]
fn ten_thousand_abandoned_mutations_release_slots_and_old_programs() {
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    let load = |i| {
        let source = format!(
            "$padding=\"{}{:05}\"\nm=Mutation(\"m\",{{}})\nroot=Box(@Run(m))",
            "p".repeat(800_000),
            i
        );
        session
            .set_program(parse(source, library.clone(), None).program)
            .unwrap();
        let tree = session.evaluate().unwrap();
        session
            .action_at(tree.evaluation, vec!["props".into(), "a".into()])
            .unwrap()
    };
    let mut action = load(0);
    for _ in 0..32 {
        let pending = session
            .run(Some(action.clone()), "".into(), None)
            .unwrap()
            .continuation
            .unwrap();
        drop(pending);
        assert_eq!(
            session.pending_continuations().unwrap(),
            0,
            "dropped outcome still occupies a slot"
        );
    }
    let baseline = test_alloc::live_bytes();
    let mut maximum = baseline;
    for i in 0..10_000 {
        if i % 100 == 0 {
            action = load(i / 100 + 1);
        }
        let pending = session
            .run(Some(action.clone()), "".into(), None)
            .unwrap()
            .continuation
            .unwrap();
        if i % 100 == 0 {
            std::thread::spawn(move || drop(pending)).join().unwrap();
        } else {
            drop(pending);
        }
        assert_eq!(session.pending_continuations().unwrap(), 0);
        if i % 1000 == 999 {
            let live = test_alloc::live_bytes();
            maximum = maximum.max(live);
            assert!(
                live <= baseline + 32 * 1024,
                "abandoned runs retain old programs: baseline={baseline}, live={live}"
            );
        }
    }
    println!(
        "10000 abandoned runs, 100 program replacements (~800KB each), baseline live={baseline}, maximum checkpoint live={maximum}, pending=0"
    );
    let pending = session
        .run(Some(action), "".into(), None)
        .unwrap()
        .continuation
        .unwrap();
    session.dispose();
    std::thread::spawn(move || drop(pending)).join().unwrap();
}
