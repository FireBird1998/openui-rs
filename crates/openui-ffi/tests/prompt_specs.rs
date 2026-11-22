//! A prompt spec through the native binding (#203): `generate_system_prompt`
//! reads its spec with serde_json, which reads 127 nested levels and refuses
//! the 128th. A spec at that bound is read, generated from and freed on a
//! 64 KiB thread, and one level more is refused with exactly the error it
//! always had: the spec is read on a stack segment of its own, as a library
//! schema is (#140), and the prompt is generated on one sized for the spec's
//! depth. A library's prompt options are read by the core's reader, under the
//! evaluation depth limit: options at that limit make a prompt and a cloud
//! config on a 64 KiB thread, and one level more is refused.
//!
//! The test profile builds at opt-level 1, which hides overflows: run these
//! unoptimised too (`cargo test --profile dev -p openui-ffi --test
//! prompt_specs`), where frames are largest. `DEPTH_STACK_KB=<n>` overrides
//! the thread's stack, for measuring. Kept in their own test binary: a stack
//! overflow aborts the process.

use openui_ffi::{ComponentLibrary, OpenUiError, ParseLimits, generate_system_prompt_with_limits};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn stack() -> usize {
    std::env::var("DEPTH_STACK_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(64 * 1024, |kb| kb * 1024)
}

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(stack())
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// JSON nested `levels` deep, of objects (`{"a": ...}`) or of arrays, around
/// the number 1; zero levels is the number alone.
fn nested(shape: &str, levels: usize) -> String {
    let (open, close) = match shape {
        "object" => (r#"{"a":"#, "}"),
        _ => ("[", "]"),
    };
    format!("{}1{}", open.repeat(levels), close.repeat(levels))
}

/// The three ways a spec reaches the core's generator, each `levels` deep in
/// all (the spec itself is level 1): a prompt spec directly, `{library,
/// promptOptions}` (whose library the generator copies), and a cloud request
/// (whose library it copies into the config block).
fn specs(shape: &str, levels: usize) -> [String; 3] {
    [
        format!(
            r#"{{"components":{{}},"extra":{}}}"#,
            nested(shape, levels - 1)
        ),
        format!(
            r#"{{"library":{{"components":{{}},"extra":{}}},"promptOptions":{{"preamble":"p"}}}}"#,
            nested(shape, levels - 2)
        ),
        format!(
            r#"{{"cloud":true,"library":{{"schema":{{"$defs":{{"C":{{}}}}}},"extra":{}}}}}"#,
            nested(shape, levels - 2)
        ),
    ]
}

/// The prompt the core makes from `text` read with serde_json, on a thread
/// with room to spare: what the binding gave before #203.
fn core_prompt(text: &str) -> String {
    let text = text.to_owned();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            openui_lang::generate_system_prompt(&json).unwrap()
        })
        .unwrap()
        .join()
        .unwrap()
}

fn invalid_spec(result: Result<String, OpenUiError>) -> String {
    match result {
        Err(OpenUiError::InvalidPromptSpec { reason }) => reason,
        other => panic!("expected InvalidPromptSpec, got {other:?}"),
    }
}

/// Where serde_json stops reading each of `specs(_, 128)`, objects then
/// arrays, as the binding reported before #203 (captured from e05fbec9).
const OVER_COLUMN: [[usize; 3]; 2] = [[656, 662, 687], [152, 162, 187]];

/// A spec nested to serde_json's bound (127 levels), of objects and of
/// arrays, in each of the three forms, gives the core's prompt on a 64 KiB
/// thread; one level more is refused with exactly the error it always had.
#[test]
fn a_spec_nested_to_the_readers_limit_generates_on_a_small_stack() {
    for (s, shape) in ["object", "array"].into_iter().enumerate() {
        for (form, (at, over)) in specs(shape, 127)
            .into_iter()
            .zip(specs(shape, 128))
            .enumerate()
        {
            let want = core_prompt(&at);
            let expected = format!(
                "recursion limit exceeded at line 1 column {}",
                OVER_COLUMN[s][form]
            );
            let (got, refused) = on_small_stack(move || {
                let limits = ParseLimits::default();
                (
                    generate_system_prompt_with_limits(at, limits.clone()),
                    invalid_spec(generate_system_prompt_with_limits(over, limits)),
                )
            });
            assert_eq!(got.unwrap(), want, "{shape} form {form}");
            assert_eq!(refused, expected, "{shape} form {form}");
        }
    }
}

/// A spec far past the bound is refused at it, on a 64 KiB thread.
#[test]
fn a_spec_far_past_the_readers_limit_is_refused_on_a_small_stack() {
    for shape in ["object", "array"] {
        let deep = nested(shape, 1_000_000);
        let reason = on_small_stack(move || {
            invalid_spec(generate_system_prompt_with_limits(
                deep,
                ParseLimits::default(),
            ))
        });
        assert!(
            reason.starts_with("recursion limit exceeded at line 1 column "),
            "{reason}"
        );
    }
}

/// Malformed specs are refused with the kind and message they always had
/// (captured from e05fbec9, before the read moved onto its own segment), and
/// a spec the core refuses is still a `Prompt` error with its message.
#[test]
fn malformed_specs_are_refused_as_before() {
    for (text, message) in MALFORMED {
        let owned = text.to_owned();
        let reason = on_small_stack(move || {
            invalid_spec(generate_system_prompt_with_limits(
                owned,
                ParseLimits::default(),
            ))
        });
        assert_eq!(reason, message, "{text:?}");
    }
    for (text, message) in REFUSED {
        let owned = text.to_owned();
        let result = on_small_stack(move || {
            generate_system_prompt_with_limits(owned, ParseLimits::default())
        });
        match result {
            Err(OpenUiError::Prompt { reason }) => assert_eq!(reason, message, "{text:?}"),
            other => panic!("expected Prompt for {text:?}, got {other:?}"),
        }
    }
    // A lowered depth limit refuses a shallower spec as a Prompt error, as
    // before: 44 JSON levels at depth 4.
    let limits = ParseLimits {
        depth: Some(4),
        ..ParseLimits::default()
    };
    let text = specs("array", 45)[0].clone();
    let result = on_small_stack(move || generate_system_prompt_with_limits(text, limits));
    match result {
        Err(OpenUiError::Prompt { reason }) => assert_eq!(
            reason,
            "[generateSystemPrompt] the spec nests deeper than 44 levels."
        ),
        other => panic!("expected Prompt, got {other:?}"),
    }
}

const MALFORMED: [(&str, &str); 7] = [
    ("", "EOF while parsing a value at line 1 column 0"),
    ("{", "EOF while parsing an object at line 1 column 1"),
    ("[1,]", "trailing comma at line 1 column 4"),
    ("nul", "EOF while parsing a value at line 1 column 3"),
    (
        "{\"components\": {}} x",
        "trailing characters at line 1 column 20",
    ),
    (
        "{\"a\": \"\\ud800\"}",
        "unexpected end of hex escape at line 1 column 14",
    ),
    ("{\"a\": 1e400}", "number out of range at line 1 column 11"),
];

const REFUSED: [(&str, &str); 2] = [
    ("[]", "[generateSystemPrompt] spec must be an object."),
    ("{}", "[generatePrompt] components is missing."),
];

/// A defined library with one component, for its prompt and cloud config.
fn defined() -> std::sync::Arc<ComponentLibrary> {
    ComponentLibrary::from_definition(
        r#"{"components":[{"name":"C","description":"","props":[]}],"root":"C"}"#.into(),
    )
    .unwrap()
}

/// A library's prompt options are read by the core's reader, under the
/// evaluation depth limit (800 JSON levels by default): options at the limit,
/// of objects and of arrays, make a prompt on a 64 KiB thread, and one level
/// more is refused as `InvalidPromptSpec`. A cloud request holds the options
/// one level down, under the same limit: options one level short of it make
/// a cloud config, and options at it are refused as `Prompt`.
#[test]
fn prompt_options_at_the_depth_limit_make_prompts_on_a_small_stack() {
    for shape in ["object", "array"] {
        let options = |levels: usize| format!(r#"{{"extra":{}}}"#, nested(shape, levels - 1));
        let (at, over, cloud_at) = (options(800), options(801), options(799));
        on_small_stack(move || {
            let library = defined();
            let plain = library.prompt(None).unwrap().unwrap();
            assert_eq!(library.prompt(Some(at.clone())).unwrap().unwrap(), plain);
            let result = library.prompt(Some(over.clone()));
            assert!(
                matches!(result, Err(OpenUiError::InvalidPromptSpec { .. })),
                "{result:?}"
            );
            let config = library.cloud_config(None, None).unwrap().unwrap();
            assert_eq!(
                library.cloud_config(Some(cloud_at), None).unwrap().unwrap(),
                config
            );
            match library.cloud_config(Some(at), None) {
                Err(OpenUiError::Prompt { reason }) => assert_eq!(
                    reason,
                    "[generateSystemPrompt] the spec nests deeper than 800 levels."
                ),
                other => panic!("expected Prompt, got {other:?}"),
            }
            let result = library.cloud_config(Some(over), None);
            assert!(
                matches!(result, Err(OpenUiError::InvalidPromptSpec { .. })),
                "{result:?}"
            );
            drop(library);
        });
    }
}

/// Host text the core's reader refuses, or whose repeated key replaces a
/// value, after a value nested to the depth limit: prompt options and a
/// library definition, both read by the core's reader, free what it read
/// with a loop on a 64 KiB thread, and are refused or used as before (#203).
#[test]
fn what_the_reader_lets_go_of_is_freed_on_a_small_stack() {
    for shape in ["object", "array"] {
        let deep = nested(shape, 798);
        let texts = [
            format!(r#"{{"extra":{deep},"extra":1}}"#),
            format!(r#"{{"extra":{deep}}} x"#),
            format!(r#"{{"extra":{deep},"b":!}}"#),
            format!(r#"{{"extra":[{deep},{}]}}"#, nested(shape, 900)),
        ];
        on_small_stack(move || {
            let library = defined();
            let plain = library.prompt(None).unwrap();
            let [repeated, refused @ ..] = texts;
            assert_eq!(library.prompt(Some(repeated.clone())).unwrap(), plain);
            let definition = repeated.replace(
                r#""extra":1"#,
                r#""extra":1,"components":[{"name":"C","description":"","props":[]}]"#,
            );
            assert!(matches!(
                ComponentLibrary::from_definition(definition),
                Err(OpenUiError::InvalidJson { .. })
            ));
            for text in refused {
                let result = library.prompt(Some(text.clone()));
                assert!(
                    matches!(result, Err(OpenUiError::InvalidPromptSpec { .. })),
                    "{result:?}"
                );
                assert!(matches!(
                    ComponentLibrary::from_definition(text),
                    Err(OpenUiError::InvalidJson { .. })
                ));
            }
            drop(library);
        });
    }
}

/// The prompt and cloud config of `options`, on a thread of `stack` bytes.
fn prompts(options: String, stack: usize) -> [Result<Option<String>, OpenUiError>; 2] {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let library = defined();
            [
                library.prompt(Some(options.clone())),
                library.cloud_config(Some(options), None),
            ]
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Options may replace any field the generator reads, and it copies some of
/// them (components, functions, actions) with serde_json: options putting a
/// value nested near the depth limit in each field give the same prompt and
/// cloud config, or the same error, on a 64 KiB thread as with room to spare
/// (#203). Before, a deep `components` overflowed the caller's stack.
#[test]
fn options_deep_in_every_field_the_generator_reads_fit_a_small_stack() {
    const FIELDS: [&str; 9] = [
        "components",
        "functions",
        "actions",
        "tools",
        "componentGroups",
        "examples",
        "additionalRules",
        "root",
        "preamble",
    ];
    for shape in ["object", "array"] {
        for key in FIELDS {
            let deep = nested(shape, 797);
            for options in [
                format!(r#"{{"{key}":{{"x":{deep}}}}}"#),
                format!(r#"{{"{key}":[{deep}]}}"#),
                format!(r#"{{"{key}":[{{"name":"t","inputSchema":{deep}}}]}}"#),
            ] {
                let roomy = prompts(options.clone(), 64 * 1024 * 1024);
                let small = prompts(options, stack());
                assert_eq!(format!("{small:?}"), format!("{roomy:?}"), "{key}");
            }
        }
    }
}
