use openui_ffi::{OpenUiError, parse_response_bundle};
use serde_json::Value;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn raw(cases: &Value, name: &str) -> String {
    cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()["raw"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn the_native_binding_reads_stored_and_streamed_bundles() {
    let cases: Value =
        serde_json::from_str(include_str!("../../../conformance/response-bundles.json")).unwrap();
    let golden: Value = serde_json::from_str(include_str!(
        "../../../conformance/golden/response-bundles.json"
    ))
    .unwrap();
    let name = "crlf";
    let raw = raw(&cases, name);
    let read = |text, streaming| {
        let json = parse_response_bundle(text, streaming).unwrap();
        let (prefix, revision) = json.rsplit_once(",\"scriptRevision\":").unwrap();
        assert!(serde_json::from_str::<String>(revision.strip_suffix('}').unwrap()).is_ok());
        format!("{prefix},\"scriptRevision\":0}}")
    };
    assert_eq!(
        read(Some(raw.clone()), false),
        golden["cases"][name]["complete"].as_str().unwrap()
    );
    let (prefix, _) = raw.rsplit_once('\n').unwrap();
    assert_eq!(
        read(Some(prefix.into()), true),
        golden["cases"][name]["cuts"][prefix.encode_utf16().count()]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        parse_response_bundle(None, true).unwrap(),
        golden["cases"]["null"]["cuts"][0].as_str().unwrap()
    );
}

#[test]
fn the_native_binding_preserves_revision_equality() {
    let cases: Value =
        serde_json::from_str(include_str!("../../../conformance/response-bundles.json")).unwrap();
    let revision = |name| {
        let json = parse_response_bundle(Some(raw(&cases, name)), false).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        parsed["scriptRevision"].as_str().unwrap().to_owned()
    };
    assert_eq!(
        revision("revision-original-order"),
        revision("revision-reordered")
    );
    assert_ne!(
        revision("revision-original-order"),
        revision("revision-changed-code")
    );
}

#[test]
fn the_native_binding_returns_typed_bundle_limit_errors() {
    let oversized = "x".repeat(openui_lang::MAX_PROGRAM_SIZE + 1);
    let too_deep = format!("]]>openui:scripts {{\"count\":0}}\n{}", "[".repeat(801));
    for (raw, suffix) in [(oversized, "program-size limit"), (too_deep, "depth limit")] {
        let error = parse_response_bundle(Some(raw), true).unwrap_err();
        assert!(matches!(error, OpenUiError::LimitExceeded { reason } if reason.ends_with(suffix)));
    }
}

/// The native response reader, fed a growing response in pieces and then
/// edits, gives what the three calls it replaces in the Swift driver give
/// for each text: the bundle, the message's context and attributes, and a
/// stream session's parse of the program.
#[test]
fn the_native_response_reader_reads_as_the_one_shot_calls_do() {
    use openui_ffi::{ComponentLibrary, StreamSession, parse_message};
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let text = "]]>openui:content?name=Daily%20report\r\n$n=1\r\nrows=[\r\n  Box(\"a\")\r\n]\r\nroot=Box(rows)\r\n]]>openui:context\r\n{\"user\":1}\r\n]]>openui:end\r\n";
    let reader = StreamSession::new(library.clone(), None);
    let mut texts: Vec<String> = (1..=text.len())
        .filter(|&cut| text.is_char_boundary(cut))
        .map(|cut| text[..cut].to_owned())
        .collect();
    texts.push(text[..20].to_owned());
    texts.push(text.replace("Daily", "Other"));
    for (index, piece) in texts.iter().enumerate() {
        let streaming = index + 1 != text.len();
        let read = reader.read_response(piece.clone(), streaming).unwrap();
        let bundle = parse_response_bundle(Some(piece.clone()), streaming).unwrap();
        assert_eq!(read.bundle, bundle, "{piece:?}");
        let message = piece.contains("]]>").then(|| {
            let json: Value =
                serde_json::from_str(&parse_message(piece.clone(), streaming).unwrap()).unwrap();
            serde_json::json!({"context": json["context"], "attributes": json["attributes"]})
        });
        let read_message = read
            .message
            .map(|json| serde_json::from_str::<Value>(&json).unwrap());
        assert_eq!(read_message, message, "{piece:?}");
        let program: Value = serde_json::from_str(&bundle).unwrap();
        let session = StreamSession::new(library.clone(), None);
        let partial = session.set(program["program"].as_str().unwrap().to_owned());
        let expected = if streaming { partial } else { session.finish() };
        assert_eq!(read.parse.json, expected.json, "{piece:?}");
    }
    let deep = format!(
        "]]>openui:content\nx\n]]>openui:context\n{}",
        "[".repeat(1000)
    );
    assert!(matches!(
        reader.read_response(deep, true),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    // One parser: `finish`, `push` and `set` go on from the response's
    // program, as they did when the driver handed the program to `set`.
    let mixed = StreamSession::new(library.clone(), None);
    let control = StreamSession::new(library.clone(), None);
    let raw = "]]>openui:content\nroot=Box(\"a\")\n$n=1\n";
    let read = mixed.read_response(raw.into(), true).unwrap();
    let program: Value = serde_json::from_str(&read.bundle).unwrap();
    let program = program["program"].as_str().unwrap().to_owned();
    assert_eq!(read.parse.json, control.set(program.clone()).json);
    assert_eq!(mixed.finish().json, control.finish().json);
    assert_eq!(
        mixed.push("x=1\n".into()).json,
        control.push("x=1\n".into()).json
    );
    assert_eq!(mixed.set(program.clone()).json, control.set(program).json);
    // And a read after pushes compares its program with what was pushed.
    let next = format!("{raw}y=2\n");
    let read = mixed.read_response(next.clone(), true).unwrap();
    let fresh = StreamSession::new(library.clone(), None);
    let program: Value = serde_json::from_str(&read.bundle).unwrap();
    assert_eq!(
        read.parse.json,
        fresh.set(program["program"].as_str().unwrap().into()).json
    );
}

/// A session whose parser takes less of a program than a response may hold
/// reads responses under the core's limits, as `parse_response_bundle` did
/// for the Swift driver, and its parser cuts the program, as `set` did. The
/// reader must accept a parser that cut its text (found by a review: a debug
/// assertion fired here).
#[test]
fn a_session_with_a_small_program_size_reads_long_responses_as_set_did() {
    use openui_ffi::{ComponentLibrary, ParseLimits, StreamSession};
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let limits = || ParseLimits {
        program_size: Some(10),
        ..Default::default()
    };
    let session = StreamSession::with_limits(library.clone(), None, limits());
    session
        .read_response("a = 1\nb = 2\n".into(), true)
        .unwrap();
    session
        .read_response("a = 1\nb = 2\nc".into(), true)
        .unwrap();

    // Many steps at a small size, every one against what the driver did
    // before: read the bundle, then `set` its program on a fresh session.
    let pieces = [
        "root = Box(a)\n",
        "a = Box(\"x\")\n",
        "]]>openui:content\n",
        "\n]]>openui:end",
        "]",
        "\r\n",
        "$n = 1\n",
        "]]>openui:context\n{\"k\":1}\n",
        "b",
    ];
    let mut seed = 0x0116_0040_u64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut steps = 0;
    for _ in 0..150 {
        let session = StreamSession::with_limits(
            library.clone(),
            None,
            ParseLimits {
                program_size: Some(40),
                ..Default::default()
            },
        );
        let mut text = String::new();
        for _ in 0..30 {
            match next(6) {
                0 => text.truncate(text.len() / 2),
                1 => {
                    session.push("x".into());
                }
                _ => text.push_str(pieces[next(pieces.len())]),
            }
            let streaming = next(5) != 0;
            let read = session.read_response(text.clone(), streaming).unwrap();
            let bundle = parse_response_bundle(Some(text.clone()), streaming).unwrap();
            assert_eq!(read.bundle, bundle, "{text:?}");
            let program: Value = serde_json::from_str(&bundle).unwrap();
            let control = StreamSession::with_limits(
                library.clone(),
                None,
                ParseLimits {
                    program_size: Some(40),
                    ..Default::default()
                },
            );
            let partial = control.set(program["program"].as_str().unwrap().to_owned());
            let expected = if streaming { partial } else { control.finish() };
            assert_eq!(read.parse.json, expected.json, "{text:?}");
            steps += 1;
        }
    }
    assert_eq!(steps, 4500);
}

/// A Cloud script's code is one JSON string, so its line is as long as its
/// code: the native session resumes that line's scan between reads (#122)
/// and still gives what the one-shot call gives at every chunk, through the
/// end of the line and the section.
#[test]
fn the_native_response_reader_reads_a_long_script_line_as_the_one_shot_call_does() {
    use openui_ffi::{ComponentLibrary, StreamSession};
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let code = "let top = rows[ids[0]] ?? \\\"é\\\"; // ]] \\uD83D\\uDE00\\n".repeat(1400);
    let text = format!(
        "]]>openui:content?name=Long\nroot=Box(\"a\")\n]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"refresh\",\"code\":\"{code}\"}}]\n]]>openui:end\n"
    );
    assert!(code.len() >= 65_536);
    let session = StreamSession::new(library, None);
    let mut at = 0;
    while at < text.len() {
        at = (at + 509).min(text.len());
        while !text.is_char_boundary(at) {
            at += 1;
        }
        let piece = &text[..at];
        let streaming = at != text.len();
        let read = session.read_response(piece.to_owned(), streaming).unwrap();
        let bundle = parse_response_bundle(Some(piece.to_owned()), streaming).unwrap();
        assert_eq!(read.bundle, bundle, "cut {at}");
    }
}

/// A driver that hands the session each appended piece and draws less often
/// (the chat app, #206) gets, at every draw, byte for byte what a driver
/// that read every whole prefix gets: the bundle, the message and the
/// parse, streaming or settled, refusals included, and under a parser that
/// cuts the program. A refused piece leaves the session as it was, and
/// `push`, `set` and `finish` go on from an appended response as from a read
/// one.
#[test]
fn appended_pieces_read_as_the_whole_response_does() {
    use openui_ffi::{ComponentLibrary, ParseLimits, StreamSession};
    let library =
        ComponentLibrary::new(r#"{"$defs":{"Box":{"properties":{"value":{}}}}}"#.into()).unwrap();
    let pieces = [
        "root = Box(a)\n",
        "a = Box(\"x\")\n",
        "]]>openui:content?name=Daily%20report\r\n",
        "\n]]>openui:end",
        "]",
        "]]",
        "\r\n",
        "$n = 1\n",
        "]]>openui:context\n{\"k\":1}\n",
        "]]>openui:context\n[[[",
        "é😀",
        "b",
        "",
    ];
    let mut seed = 0x0206_0001_u64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut draws = 0;
    for round in 0..120 {
        let limits = || ParseLimits {
            program_size: (round % 2 == 0).then_some(40),
            ..Default::default()
        };
        let appending = StreamSession::with_limits(library.clone(), None, limits());
        let whole = StreamSession::with_limits(library.clone(), None, limits());
        let mut text = String::new();
        for _ in 0..40 {
            let piece = pieces[next(pieces.len())];
            let candidate = format!("{text}{piece}");
            let expected = whole.read_response(candidate.clone(), true);
            let appended = appending.append_response(piece.into(), true);
            match (&appended, &expected) {
                (Ok(()), Ok(_)) => {}
                (Err(error), Err(expected)) => assert_eq!(
                    std::mem::discriminant(error),
                    std::mem::discriminant(expected),
                    "{candidate:?}"
                ),
                _ => panic!("{candidate:?}: {appended:?}"),
            }
            if appended.is_err() {
                // A refused read leaves both sessions where they were.
                continue;
            }
            text = candidate;
            if next(3) == 0 {
                let streaming = next(4) != 0;
                let read = appending.reread_response(streaming).unwrap();
                let expected = whole.read_response(text.clone(), streaming).unwrap();
                assert_eq!(read.bundle, expected.bundle, "{text:?}");
                assert_eq!(read.message, expected.message, "{text:?}");
                assert_eq!(read.parse.json, expected.parse.json, "{text:?}");
                assert_eq!(read.parse.size_limited, expected.parse.size_limited);
                draws += 1;
            }
        }
        // Settled, then the parser's own calls go on from the response.
        let read = appending.reread_response(false).unwrap();
        let expected = whole.read_response(text.clone(), false).unwrap();
        assert_eq!(read.bundle, expected.bundle, "{text:?}");
        assert_eq!(read.parse.json, expected.parse.json, "{text:?}");
        // A streaming draw, then a piece: the next result is the piece's.
        appending.reread_response(true).unwrap();
        whole.read_response(text.clone(), true).unwrap();
        appending.append_response("c = 1\n".into(), true).unwrap();
        whole.read_response(format!("{text}c = 1\n"), true).unwrap();
        assert_eq!(appending.result().json, whole.result().json);
        assert_eq!(appending.finish().json, whole.finish().json);
        assert_eq!(appending.push("x".into()).json, whole.push("x".into()).json);
        let program: Value = serde_json::from_str(&expected.bundle).unwrap();
        let program = program["program"].as_str().unwrap().to_owned();
        assert_eq!(appending.set(program.clone()).json, whole.set(program).json);
    }
    assert!(draws > 1000, "{draws}");

    // Past the core's program-size limit the piece is refused, and the
    // session stays at the text it accepted.
    let session = StreamSession::new(library.clone(), None);
    let head = "root = Box(\"a\")\n".to_owned();
    session.append_response(head.clone(), true).unwrap();
    let filler = "x".repeat(openui_lang::Limits::DEFAULT.program_size() - head.len());
    assert!(matches!(
        session.append_response(format!("{filler}y"), true),
        Err(OpenUiError::LimitExceeded { .. })
    ));
    // So is a context nested past the depth limit, as `read_response`
    // refuses it. The bundle reader refuses it before the message reader
    // sees it, with or without a bare scripts marker after it (which ends
    // the message reader's context but not the bundle reader's): the
    // message reader's refusal in `append` is reached by no text today.
    for deep in [
        format!("]]>openui:context\n{}", "[".repeat(1000)),
        format!(
            "]]>openui:context\n{}\n]]>openui:scripts\n",
            "[".repeat(1000)
        ),
    ] {
        assert!(matches!(
            session.append_response(deep, true),
            Err(OpenUiError::LimitExceeded { reason })
                if reason == "response bundle context JSON exceeds the depth limit"
        ));
    }
    let read = session.reread_response(false).unwrap();
    assert_eq!(
        read.bundle,
        parse_response_bundle(Some(head), false).unwrap()
    );
}
