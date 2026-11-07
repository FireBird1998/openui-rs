//! The binding runs on the caller's thread, whatever its stack: the deepest
//! programs parse, stream and merge on a 64 KB stack, and their results and
//! sessions are dropped there too. Kept in their own test binary, because a
//! stack overflow aborts the process rather than failing one test.

use std::sync::Arc;

use openui_ffi::{
    ComponentLibrary, ParseLimits, StreamSession, merge_statements, parse, parse_with_limits,
};

const STACK: usize = 64 * 1024;

fn library() -> Arc<ComponentLibrary> {
    let schema = r#"{"$defs": {
        "Text": {"properties": {"value": {}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array"}}}
    }}"#;
    ComponentLibrary::new(schema.to_owned()).unwrap()
}

/// The deepest results the core makes: nesting cut at the depth limit, and
/// state defaults whose evaluated values nest three times as deep.
fn deepest_programs() -> Vec<String> {
    let wrap = |inner: &str| format!("{}{inner}{}", "[".repeat(200), "]".repeat(200));
    let mut defaults = String::from("root = Text(\"x\")\n$v0 = [1, \"x\"]\n");
    for i in 1..=30 {
        defaults.push_str(&format!("$v{i} = {}\n", wrap(&format!("$v{}", i - 1))));
    }
    vec![
        format!(
            "root = Stack({}1{})",
            "[".repeat(10_000),
            "]".repeat(10_000)
        ),
        format!(
            "root = {}\"x\"{}",
            "Text(".repeat(10_000),
            ")".repeat(10_000)
        ),
        defaults,
    ]
}

#[test]
fn the_deepest_programs_run_on_a_small_stack() {
    let library = library();
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            for program in deepest_programs() {
                let output = parse(program.clone(), library.clone(), None);
                assert!(output.depth_limited);
                let session = StreamSession::new(library.clone(), None);
                let chars: Vec<char> = program.chars().collect();
                for chunk in chars.chunks(chars.len().div_ceil(16)) {
                    session.push(chunk.iter().collect());
                }
                assert!(session.finish().depth_limited);
                drop(session);
                merge_statements(program.clone(), program, None);
            }
        })
        .unwrap()
        .join()
        .expect("the binding panicked");
}

#[test]
fn a_raised_depth_limit_runs_on_a_small_stack() {
    let library = library();
    let limits = ParseLimits {
        depth: Some(20_000),
        ..ParseLimits::default()
    };
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            for program in &deepest_programs()[..2] {
                // 10,000 levels fit under the raised limit: nothing is cut.
                let output =
                    parse_with_limits(program.clone(), library.clone(), None, limits.clone());
                assert!(!output.depth_limited);
                let session = StreamSession::with_limits(library.clone(), None, limits.clone());
                let chars: Vec<char> = program.chars().collect();
                for chunk in chars.chunks(chars.len().div_ceil(4)) {
                    session.push(chunk.iter().collect());
                }
                assert!(!session.finish().depth_limited);
                drop(session);
            }
        })
        .unwrap()
        .join()
        .expect("the binding panicked");
}
