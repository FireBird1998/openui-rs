//! What a library made from a library definition holds for the bindings
//! (#137): the JSON Schema it was compiled from, as deep as the definition's
//! types, is freed without recursion wherever the last holder drops it. The
//! core's own `Library` and `LibraryDefinition` still free themselves
//! recursively (#140), so each test keeps the compiled `Library` alive and
//! drops it, and makes the definition, on a large stack.

use openui_session::{ParseLimits, RuntimeLimits, define_library};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const SMALL: usize = 64 * 1024;
const LARGE: usize = 1024 * 1024 * 1024;

/// One component whose one prop is `depth` arrays around a string.
fn deep_definition(depth: usize) -> String {
    let mut ty = String::from(r#"{"kind":"string"}"#);
    for _ in 1..depth {
        ty = format!(r#"{{"kind":"array","items":{ty}}}"#);
    }
    format!(
        r#"{{"components":[{{"name":"Deep","description":"","props":[{{"name":"v","type":{ty}}}]}}]}}"#
    )
}

fn on(stack: usize, f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

/// Makes a library at `depth`, then drops everything the binding holds of
/// it on a 64 KiB thread, the core's `Library` excepted.
fn held_schema_drops_on_a_small_stack(depth: usize) {
    let limits = RuntimeLimits {
        evaluation: ParseLimits {
            depth: Some(depth as u64),
            ..ParseLimits::default()
        },
        ..RuntimeLimits::default()
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    on(LARGE, move || {
        let output = define_library(&deep_definition(depth), &limits).unwrap();
        assert!(output.json_schema.len() > depth * 20);
        sender.send(output).unwrap();
    });
    let output = receiver.recv().unwrap();
    let library = output.library;
    let compiled = library.inner.clone();
    // A second holder of the schema, as a session's copy of the library is:
    // whichever drops last frees it.
    let schema = library.schema.clone();
    let session_copy = openui_session::ComponentLibrary {
        inner: compiled.clone(),
        schema,
    };
    on(SMALL, move || drop(library));
    on(SMALL, move || drop(session_copy));
    on(LARGE, move || drop(compiled));
}

/// At the default type depth limit, 256: the schema nests about 520 levels.
#[test]
fn the_held_schema_of_a_definition_at_the_default_limit_drops_on_a_64_kib_thread() {
    held_schema_drops_on_a_small_stack(256);
}

/// Under a raised depth limit, far past the default.
#[test]
fn the_held_schema_of_a_far_deeper_definition_drops_on_a_64_kib_thread() {
    held_schema_drops_on_a_small_stack(20_000);
}
