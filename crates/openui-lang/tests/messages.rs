use openui_lang::{
    BuildMessageInput, JsObject, Limits, MessageError, Value, build_message, build_message_json,
    parse_json, parse_message, stringify_json, utf16_unit_string, utf16_units,
};
use serde_json::Value as Json;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

const CASES: &str = include_str!("../../../conformance/messages.json");
const GOLDEN: &str = include_str!("../../../conformance/golden/messages.json");

#[test]
fn message_reads_and_writes_match_the_reference() {
    let cases = parse_json(CASES).unwrap();
    let golden = parse_json(GOLDEN).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expected = &golden[name];
        if let Some(raw) = case["raw"].as_str() {
            check_reads(raw, expected, name);
        } else {
            let actual = build_message_json(&stringify_json(&case["build"]), Limits::DEFAULT);
            if expected["threw"].is_string() {
                assert_eq!(actual, Err(MessageError::InvalidAttribute), "{name}");
            } else {
                let raw = actual.unwrap();
                assert_eq!(Some(raw.as_str()), expected["text"].as_str(), "{name}");
                check_reads(&raw, &expected["read"], name);
            }
        }
    }
}

fn check_reads(raw: &str, expected: &Json, name: &str) {
    let complete = parse_message(raw, false, Limits::DEFAULT).unwrap();
    assert_eq!(
        Some(complete.to_json().as_str()),
        expected["complete"].as_str(),
        "{name} complete"
    );

    if let Some(number) = expected["contextNumber"].as_str() {
        let Value::Num(value) = complete.context else {
            panic!("{name} context should be numeric");
        };
        let actual = if value == 0.0 && value.is_sign_negative() {
            "-0".to_owned()
        } else if value.is_infinite() {
            if value.is_sign_positive() {
                "Infinity"
            } else {
                "-Infinity"
            }
            .to_owned()
        } else {
            stringify_json(&Json::from(value))
        };
        assert_eq!(actual, number, "{name}");
    }

    // Rust callers can also supply the two halves of an astral scalar as
    // adjacent stand-ins. They still describe one JavaScript string.
    let separated: String = utf16_units(raw).map(utf16_unit_string).collect();
    if separated != raw {
        let read = parse_message(&separated, false, Limits::DEFAULT).unwrap();
        assert_eq!(
            Some(read.to_json().as_str()),
            expected["complete"].as_str(),
            "{name} with surrogate halves as separate units"
        );
    }

    let cuts = expected["cuts"].as_array().unwrap();
    let units: Vec<_> = utf16_units(raw).collect();
    assert_eq!(cuts.len(), units.len() + 1, "{name}");
    for (cut, expected) in cuts.iter().enumerate() {
        let prefix: String = char::decode_utf16(units[..cut].iter().copied())
            .map(|decoded| match decoded {
                Ok(ch) => ch.to_string(),
                Err(error) => utf16_unit_string(error.unpaired_surrogate()),
            })
            .collect();
        let actual = parse_message(&prefix, true, Limits::DEFAULT).unwrap();
        assert_eq!(
            Some(actual.to_json().as_str()),
            expected.as_str(),
            "{name} at UTF-16 cut {cut}"
        );
    }
}

#[test]
fn reading_back_reversible_parts_preserves_them() {
    let cases = parse_json(CASES).unwrap();
    for case in cases.as_array().unwrap() {
        if case["roundTrip"] != true {
            continue;
        }
        let input = &case["build"];
        let mut attributes = JsObject::new();
        for (key, value) in input["attributes"].as_object().into_iter().flatten() {
            attributes.define(key.clone(), value.as_str().unwrap().to_owned());
        }
        let context = Value::from_json(&input["context"]);
        assert_round_trip(BuildMessageInput {
            content: input["content"].as_str().unwrap(),
            context: Some(&context),
            attributes: Some(&attributes),
            end: input["end"] == true,
        });
    }

    let fixture = &cases
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "attribute-encoding")
        .unwrap()["build"];
    let separated = |text: &str| utf16_units(text).map(utf16_unit_string).collect::<String>();
    let content = separated(fixture["content"].as_str().unwrap());
    let mut attributes = JsObject::new();
    for (key, value) in fixture["attributes"].as_object().unwrap() {
        attributes.define(separated(key), separated(value.as_str().unwrap()));
    }
    let nested_context = Value::from_json(&fixture["context"]);
    let written = build_message(
        BuildMessageInput {
            content: &content,
            context: Some(&nested_context),
            attributes: Some(&attributes),
            end: true,
        },
        Limits::DEFAULT,
    )
    .unwrap();
    assert_eq!(
        Some(written.as_str()),
        parse_json(GOLDEN).unwrap()["attribute-encoding"]["text"].as_str()
    );

    for i in 0..256 {
        let content = format!("😀 {i}\r\n雪");
        let mut attributes = JsObject::new();
        attributes.define(i.to_string(), format!("a +&={i}"));
        attributes.define("name", format!("🌿 +{i}%"));
        let json = Value::from_json(&serde_json::json!({"i":i,"__proto__":[true,null,"🌿"]}));
        let text = Value::Str(format!("plain context for {i}\r\ntext"));
        let context = match i % 3 {
            0 => None,
            1 => Some(&json),
            _ => Some(&text),
        };
        assert_round_trip(BuildMessageInput {
            content: &content,
            context,
            attributes: Some(&attributes),
            end: i % 2 == 0,
        });
    }
}

fn assert_round_trip(input: BuildMessageInput<'_>) {
    let written = build_message(input, Limits::DEFAULT).unwrap();
    let parsed = parse_message(&written, false, Limits::DEFAULT).unwrap();
    assert_eq!(parsed.content, input.content);
    assert_eq!(&parsed.context, input.context.unwrap_or(&Value::Null));
    assert_eq!(Some(&parsed.attributes), input.attributes);
    assert_eq!(parsed.end, input.end);
}

#[test]
fn whole_message_size_is_checked_first_in_utf8_bytes() {
    let raw = format!(
        "]]>openui:context\n{}\n]]>openui:content\n雪",
        "x".repeat(30)
    );
    for streaming in [false, true] {
        let limits = Limits::DEFAULT.with_program_size(raw.len() - 1);
        assert!(matches!(
            parse_message(&raw, streaming, limits),
            Err(MessageError::ProgramSize)
        ));
        let limits = limits.with_program_size(raw.len());
        assert_eq!(
            parse_message(&raw, streaming, limits).unwrap().content,
            "雪"
        );
    }
    assert!(matches!(
        parse_message("é", true, Limits::DEFAULT.with_program_size(1)),
        Err(MessageError::ProgramSize)
    ));
}

#[test]
fn only_the_winning_context_is_read_under_the_json_depth_limit() {
    let raw = |depth| {
        format!(
            "]]>openui:context\n{}0{}",
            "[".repeat(depth),
            "]".repeat(depth)
        )
    };
    let limits = Limits::DEFAULT.with_depth(0);
    let at_limit = raw(32);
    let over_limit = raw(33);
    for streaming in [false, true] {
        assert!(matches!(
            parse_message(&at_limit, streaming, limits).unwrap().context,
            Value::Arr(_)
        ));
        assert!(matches!(
            parse_message(&over_limit, streaming, limits),
            Err(MessageError::ContextDepth)
        ));
        assert!(matches!(
            parse_message(&over_limit, streaming, limits.with_program_size(0)),
            Err(MessageError::ProgramSize)
        ));
    }
    let abandoned = format!("{over_limit}\n]]>openui:content\nbody");
    assert_eq!(
        parse_message(&abandoned, false, limits).unwrap().context,
        Value::Null
    );
    let replaced = format!("{over_limit}\n]]>openui:context\ntrue");
    assert_eq!(
        parse_message(&replaced, false, limits).unwrap().context,
        Value::Bool(true)
    );
}

#[test]
fn raised_depth_is_safe_on_a_small_stack_on_success_and_refusal() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let depth = 12_000;
            let until_closing = format!(
                "]]>openui:context\n{}0{}",
                "[".repeat(depth),
                "]".repeat(depth)
            );
            let raw = format!("{until_closing}\n]]>openui:end");
            let limits = Limits::DEFAULT.with_depth(depth);
            let parsed = parse_message(&raw, false, limits).unwrap();
            let parts = BuildMessageInput {
                context: Some(&parsed.context),
                ..Default::default()
            };
            let written = build_message(parts, limits).unwrap();
            assert_eq!(
                parse_message(&written, false, limits).unwrap().to_json(),
                parsed_without_end(&until_closing, limits)
            );
            assert!(matches!(
                build_message(parts, Limits::DEFAULT),
                Err(MessageError::ContextDepth)
            ));
            let bad = format!(
                "]]>openui:context\n[{}!",
                &until_closing["]]>openui:context\n".len()..]
            );
            assert!(matches!(
                parse_message(&bad, false, limits).unwrap().context,
                Value::Str(_)
            ));
        })
        .unwrap()
        .join()
        .unwrap();
}

fn parsed_without_end(raw: &str, limits: Limits) -> String {
    parse_message(raw, false, limits).unwrap().to_json()
}

#[test]
fn writer_bounds_emitted_bytes_including_attribute_expansion() {
    let mut attributes = JsObject::new();
    attributes.define("a", "雪".to_owned());
    let parts = BuildMessageInput {
        content: "é",
        attributes: Some(&attributes),
        end: true,
        ..Default::default()
    };
    let text = build_message(parts, Limits::DEFAULT).unwrap();
    assert_eq!(
        build_message(parts, Limits::DEFAULT.with_program_size(text.len())),
        Ok(text.clone())
    );
    assert_eq!(
        build_message(parts, Limits::DEFAULT.with_program_size(text.len() - 1)),
        Err(MessageError::ProgramSize)
    );
    assert_eq!(
        build_message_json(r#"{"content":"é"}"#, Limits::DEFAULT.with_program_size(1)),
        Err(MessageError::ProgramSize)
    );
}
