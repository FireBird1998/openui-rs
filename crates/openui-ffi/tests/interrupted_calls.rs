//! The native counterpart of #208. A host function that panics or throws
//! cannot leave the core's thread-local state wrong: a panic unwinds through
//! Rust, running every destructor, and is caught at the callback boundary;
//! a throw is an ordinary error return. Neither the interrupted session nor
//! any other gives different output afterwards, after one or after 1,000.
//! (Each native session also evaluates on its own worker thread, ADR 0006,
//! so its thread-locals are not another session's.)
use openui_ffi::*;
use std::sync::Arc;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}},"returns":{}}}}"#;

struct Function<F>(F);
impl<F: Fn(String) -> Result<Option<String>, FunctionError> + Send + Sync> LibraryFunction
    for Function<F>
{
    fn call(&self, json: String) -> Result<Option<String>, FunctionError> {
        (self.0)(json)
    }
}

fn session(library: &Arc<ComponentLibrary>, program: &str) -> Arc<RuntimeSession> {
    let s = RuntimeSession::new(library.clone(), None).unwrap();
    s.set_program(parse(program.into(), library.clone(), None).program)
        .unwrap();
    s.initialize(None).unwrap();
    s
}

/// The issue's cases, each in a new session: `[$a]`, and whether nesting at
/// depths around the limit is depth-limited.
fn outputs(library: &Arc<ComponentLibrary>) -> Vec<String> {
    let mut out = Vec::new();
    for depth in [1, 100, 252, 253, 254, 300] {
        let program = format!(
            "$a=1\nroot=Box({}$a{})",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        let s = session(library, &program);
        let tree = s.evaluate().unwrap();
        // Deeper than serde_json's own recursion limit.
        let limits = openui_lang::Limits::DEFAULT.with_depth(4096);
        let json = openui_lang::parse_json_with_limits(&tree.json, limits).unwrap();
        let mut value = &json["root"]["props"]["value"];
        let mut levels = 0;
        while let Some(first) = value.as_array().and_then(|items| items.first()) {
            value = first;
            levels += 1;
        }
        let leaf = value.to_string();
        openui_lang::free_json(json);
        out.push(format!(
            "{depth}: {levels} around {leaf}, depth {}",
            tree.depth_limited
        ));
    }
    let s = session(library, "$a=1\nroot=Box([$a])");
    let json: serde_json::Value = serde_json::from_str(&s.evaluate().unwrap().json).unwrap();
    out.push(json["root"]["props"]["value"].to_string());
    out
}

#[test]
fn a_panicking_or_throwing_host_function_changes_no_later_output() {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let other = session(&library, "root=Box(1)");
    let fresh = outputs(&library);
    assert_eq!(fresh.last().map(String::as_str), Some("[1]"));
    assert!(fresh.iter().any(|line| line.ends_with("depth true")));
    assert!(fresh.iter().any(|line| line.ends_with("depth false")));
    let interrupted = session(&library, "root=Box([[[@Echo(7)]]])");
    let panics = || {
        interrupted
            .register_function("Echo".into(), Box::new(Function(|_| panic!("host panic"))))
            .unwrap();
        let tree = interrupted.evaluate().unwrap();
        assert!(tree.json.contains("@Echo threw: host function panicked"));
    };
    let throws = || {
        interrupted
            .register_function(
                "Echo".into(),
                Box::new(Function(|_| {
                    Err(FunctionError::Thrown {
                        reason: "host throw".into(),
                    })
                })),
            )
            .unwrap();
        assert!(
            interrupted
                .evaluate()
                .unwrap()
                .json
                .contains("@Echo threw: host throw")
        );
    };
    panics();
    throws();
    assert_eq!(outputs(&library), fresh);
    for _ in 1..1000 {
        panics();
        throws();
    }
    assert_eq!(outputs(&library), fresh);
    // The interrupted session itself, against a new one with its program.
    let echo = || Box::new(Function(|json: String| Ok(Some(json))));
    interrupted
        .register_function("Echo".into(), echo())
        .unwrap();
    let new = session(&library, "root=Box([[[@Echo(7)]]])");
    new.register_function("Echo".into(), echo()).unwrap();
    assert_eq!(
        interrupted.evaluate().unwrap().json,
        new.evaluate().unwrap().json
    );
    assert_eq!(
        other.evaluate().unwrap().json,
        session(&library, "root=Box(1)").evaluate().unwrap().json
    );
}
