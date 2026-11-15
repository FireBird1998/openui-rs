//! Bookkeeping cannot refuse an uncut Rust runtime draw. Exported FFI calls only.
use openui_ffi::{ComponentLibrary, RuntimeSession, parse};
use openui_lang::{Library, Value};
use openui_runtime::{Store, TreeInput, evaluate_tree};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);

const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"a":{}}}}}"#;

#[test]
fn four_thousand_and_ninety_seven_plans_draw_and_the_last_plan_runs() {
    let source = "$n=0\nq=Query(\"q\",{})\nroot=Box(@Each(q.rows,r,Action([@Set($n,r.id)])))";
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session
        .set_program(parse(source.into(), library, None).program)
        .unwrap();
    session.initialize(None).unwrap();
    let rows = (0..4097)
        .map(|i| format!("{{\"id\":{i}}}"))
        .collect::<Vec<_>>()
        .join(",");
    let supplied = format!("{{\"q\":{{\"rows\":[{rows}]}}}}");
    session.set_reference_values(supplied.clone()).unwrap();
    let core_library = Library::from_json_schema(&openui_lang::parse_json(SCHEMA).unwrap());
    let program = openui_lang::parse(source, &core_library, None);
    let Value::Obj(ref values) = Value::from_json(&openui_lang::parse_json(&supplied).unwrap())
    else {
        panic!()
    };
    let expected = evaluate_tree(
        TreeInput::new(&program)
            .with_library(&core_library)
            .with_reference_values(values),
        &Store::new(),
    );
    assert!(!expected.size_limited && !expected.depth_limited);
    let got = session.evaluate().unwrap();
    assert_eq!(got.json, expected.to_json().unwrap());
    assert_eq!((got.size_limited, got.depth_limited), (false, false));
    let action = session
        .action_at(
            got.evaluation,
            vec!["props".into(), "a".into(), "4096".into()],
        )
        .unwrap();
    assert!(
        session
            .run(Some(action), "".into(), None)
            .unwrap()
            .failure
            .is_none()
    );
    assert_eq!(session.get("$n".into()).unwrap().as_deref(), Some("4096"));
    println!("4097 plan positions: byte-identical draw; final plan writes 4096");
}

#[test]
fn two_point_two_million_values_draw_with_the_runtimes_exact_json_and_flags() {
    let source = format!("$padding=\"{}\"\nroot=Box($x)", "p".repeat(140 * 1024));
    let raw = format!("[{}0]", "0,".repeat(2_200_000 - 1));
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session
        .set_program(parse(source.clone(), library, None).program)
        .unwrap();
    let core_library = Library::from_json_schema(&openui_lang::parse_json(SCHEMA).unwrap());
    let program = openui_lang::parse(&source, &core_library, None);
    let mut state = Store::new();
    let parsed = openui_lang::parse_json(&raw).unwrap();
    state.set("$x", Some(Value::from_json(&parsed))).unwrap();
    openui_lang::free_json(parsed);
    session.set("$x".into(), Some(raw)).unwrap();
    let expected = evaluate_tree(TreeInput::new(&program).with_library(&core_library), &state);
    assert!(!expected.size_limited && !expected.depth_limited);
    let got = session.evaluate().unwrap();
    assert_eq!(got.json, expected.to_json().unwrap());
    assert_eq!((got.size_limited, got.depth_limited), (false, false));
    println!(
        "2200000 values: {} JSON bytes; byte-identical, uncut",
        got.json.len()
    );
    // Objects that look like array indices but differ from JSON's spelling fail.
    assert!(
        session
            .action_at(
                got.evaluation,
                vec!["props".into(), "a".into(), "00".into()]
            )
            .is_err()
    );
}
