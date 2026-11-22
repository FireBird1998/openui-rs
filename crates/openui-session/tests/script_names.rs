//! The Cloud script names of a response state cost memory per name, not only
//! per byte: each is charged four bytes beside its text, and nothing the
//! session builds from them grows past what the byte limit admits (#143).
use openui_lang::{JsObject, Library, Limits};
use openui_runtime::PreparedReferenceValues;
use openui_session::*;
use std::sync::Arc;

// The inputs below hold millions of names, so this binary gets the larger cap.
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const LIMIT: u64 = 8 * 1024 * 1024;
/// The most empty names whose JSON array, `[""` then `,""` each, fits 8 MiB.
const EMPTY_NAMES_IN_THE_JSON_LIMIT: usize = 2_796_202;
/// The most empty names the session admits at 8 MiB: four bytes each.
const EMPTY_NAMES_ADMITTED: usize = 2 * 1024 * 1024;

fn session() -> (SessionCore, PreparedReferenceValues<'static>) {
    let schema = serde_json::json!({"$defs":{"Box":{"properties":{"a":{}}}}});
    let library = Arc::new(ComponentLibrary {
        inner: Arc::new(Library::from_json_schema(&schema)),
        schema: Arc::new(schema),
        defined: None,
    });
    let refs = PreparedReferenceValues::owned_with_limits(JsObject::new(), Limits::DEFAULT);
    let mut session = SessionCore::new(library.clone(), None, RuntimeLimits::default()).unwrap();
    let inner = Arc::new(openui_lang::parse("root=Box(1)", &library.inner, None));
    let program = Arc::new(ParsedProgram {
        json_bytes: inner.to_json().len() as u64,
        inner,
    });
    session
        .replace_program(&refs, program, None, None, true)
        .unwrap();
    (session, refs)
}

/// Heap bytes the call adds above what was live when it started.
fn extra_peak<T>(call: impl FnOnce() -> T) -> (T, usize) {
    test_alloc::reset_peak();
    let before = test_alloc::peak();
    let result = call();
    (result, test_alloc::peak() - before)
}

fn empty_names_json(count: usize) -> String {
    let mut json = String::with_capacity(3 * count + 1);
    json.push('[');
    for i in 0..count {
        json.push_str(if i == 0 { "\"\"" } else { ",\"\"" });
    }
    json.push(']');
    json
}

/// The native binding hands the names as a list. 2.8 Mi empty names, as many
/// as 8 MiB of scripts JSON holds, are refused; 2 Mi, the most admitted, are
/// drawn without the session reserving room for each of them.
#[test]
fn many_empty_names_as_a_list_are_refused_or_built_within_the_limit() {
    let (mut session, refs) = session();
    let refused = ResponseState {
        scripts: vec![String::new(); EMPTY_NAMES_IN_THE_JSON_LIMIT],
        ..Default::default()
    };
    let (result, peak) = extra_peak(|| session.evaluate_with_response(&refs, refused));
    assert!(
        matches!(result, Err(OpenUiError::LimitExceeded { ref reason }) if reason == "response state byte limit"),
        "{:?} (extra peak {peak} bytes)",
        result.map(|draw| draw.blocked)
    );
    assert!(peak < 1024 * 1024, "extra peak {peak} bytes");

    let admitted = ResponseState {
        scripts: vec![String::new(); EMPTY_NAMES_ADMITTED],
        ..Default::default()
    };
    let (result, peak) = extra_peak(|| session.evaluate_with_response(&refs, admitted));
    assert!(result.is_ok(), "{:?}", result.err());
    assert!(peak < LIMIT as usize, "extra peak {peak} bytes");
    let one_more = ResponseState {
        scripts: vec![String::new(); EMPTY_NAMES_ADMITTED + 1],
        ..Default::default()
    };
    assert!(matches!(
        session.evaluate_with_response(&refs, one_more),
        Err(OpenUiError::LimitExceeded { .. })
    ));
}

/// The WebAssembly binding hands the names as JSON text. Text over the limit
/// is refused before it is read, so malformed text over it is a limit
/// refusal, not a JSON one; within it, the names are counted before any is
/// built, so 2.8 Mi empty names build nothing.
#[test]
fn scripts_json_is_held_to_the_limit_before_it_is_read() {
    let (session, _) = session();
    let over = "!".repeat(LIMIT as usize + 1);
    assert!(matches!(
        session.script_names(&over),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    assert!(matches!(
        session.script_names(&over[1..]),
        Err(OpenUiError::InvalidJson { .. })
    ));
    drop(over);
    for malformed in [
        "",
        "[",
        "]",
        "[\"a\",]",
        "[,\"a\"]",
        "[\"a\" \"b\"]",
        "[1]",
        "[[]]",
        "{}",
        "[\"\\x\"]",
        "[\"\u{1}\"]",
        "[\"a\\\"]",
        "[]x",
        "[\"\\ud800\"]",
    ] {
        assert!(
            matches!(
                session.script_names(malformed),
                Err(OpenUiError::InvalidJson { .. })
            ),
            "{malformed}"
        );
    }
    assert_eq!(
        session
            .script_names(" [ \"a\\\"]\" ,\n\"\\u00e9\\ud83d\\ude00\\\\\",\"\" ]\t")
            .unwrap(),
        ["a\"]", "é😀\\", ""]
    );
    assert!(session.script_names("[]").unwrap().is_empty());

    let json = empty_names_json(EMPTY_NAMES_IN_THE_JSON_LIMIT);
    assert!(json.len() as u64 <= LIMIT);
    let (result, peak) = extra_peak(|| session.script_names(&json));
    assert!(
        matches!(result, Err(OpenUiError::LimitExceeded { ref reason }) if reason == "response state byte limit"),
        "{:?} (extra peak {peak} bytes)",
        result.map(|names| names.len())
    );
    assert!(peak < 1024 * 1024, "extra peak {peak} bytes");
    drop(json);

    // The most admitted: the list of names is all it builds, one `String`
    // (24 bytes here, 12 in WebAssembly) per four bytes of the limit.
    let json = empty_names_json(EMPTY_NAMES_ADMITTED);
    let (result, peak) = extra_peak(|| session.script_names(&json));
    assert_eq!(
        result.map(|names| names.len()).ok(),
        Some(EMPTY_NAMES_ADMITTED)
    );
    let list = EMPTY_NAMES_ADMITTED * std::mem::size_of::<String>();
    assert!(peak <= list + 1024 * 1024, "extra peak {peak} bytes");
    drop(json);
    // A name's text counts beside its four bytes.
    let mut json = empty_names_json(EMPTY_NAMES_ADMITTED);
    json.insert(2, 'a');
    assert!(matches!(
        session.script_names(&json),
        Err(OpenUiError::LimitExceeded { .. })
    ));
}
