//! Whatever text arrives, parsing finishes without panicking: every stage,
//! one-shot, streamed at any chunk boundaries, and resynced with `set`. Seeds
//! are fixed, so a run is the same everywhere.

use std::sync::{Arc, LazyLock};

use openui_lang::{
    Library, StreamParser, Token, parse, preprocess, strip_fences, tokenize, tokens_to_json,
};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, RngSeed};

/// Fixed seed and case count; failures are reported, not persisted to files.
fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        rng_seed: RngSeed::Fixed(0x0915_2026),
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

/// A library with every schema shape validation handles.
static LIBRARY: LazyLock<Arc<Library>> = LazyLock::new(|| {
    let schema = serde_json::json!({"$defs": {
        "Text": {"properties": {"value": {"type": "string"}, "tone": {"enum": ["info", 2, null], "default": "info"}}, "required": ["value"]},
        "Stack": {"properties": {"children": {"type": "array", "items": {"anyOf": [{"$ref": "#/$defs/Text"}]}}}},
        "Card": {"properties": {"meta": {"type": "object", "properties": {"id": {"type": "string"}, "0": {"const": 1}}, "required": ["id"]}, "rows": {"type": "array", "items": {"type": "number"}}}, "required": ["meta"]},
        "10": {"properties": {"b": {}, "0": {"type": "boolean", "default": true}}, "required": ["0"]}
    }});
    Arc::new(Library::from_json_schema(&schema))
});

/// Text dense in the characters the lexer treats specially, plus multi-byte
/// characters, so odd combinations come up often.
fn lexer_heavy_text() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "\"",
        "'",
        "\\",
        "\\u",
        "\\uD83D",
        "\\uDE00",
        "u00",
        "$",
        "@",
        "-",
        ".",
        "e",
        "E",
        "+",
        "0",
        "7",
        "1e",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        ",",
        ":",
        "=",
        "==",
        "!",
        "<",
        ">",
        "&",
        "|",
        "?",
        "*",
        "/",
        "%",
        "#",
        "\n",
        "\t",
        "\r",
        " ",
        "a",
        "Z",
        "_",
        "true",
        "null",
        "é",
        "😀",
        "\u{10F800}",
        "\u{0}",
        "\u{1F}",
        "`",
        "```",
        "```openui\n",
        "//",
        "\u{FEFF}",
        "root = ",
        "x = ",
        "$s = ",
        "Text(",
        "Stack([",
        "Card({id: ",
        "Query(",
        "@Each(",
        "Action([",
        "@Set(",
    ]);
    prop::collection::vec(pieces, 0..64).prop_map(|p| p.concat())
}

fn assert_tokenizes(text: &str) {
    let tokens = tokenize(text);
    assert_eq!(tokens.last(), Some(&Token::Eof));
    // Every token serializes, so a stray code point can't break the writer either.
    tokens_to_json(&tokens);
}

proptest! {
    #![proptest_config(config(2000))]

    #[test]
    fn tokenizing_any_text_ends_in_eof(text in any::<String>()) {
        assert_tokenizes(&text);
    }

    #[test]
    fn tokenizing_lexer_heavy_text_ends_in_eof(text in lexer_heavy_text()) {
        assert_tokenizes(&text);
    }
}

proptest! {
    #![proptest_config(config(2000))]

    #[test]
    fn preprocessing_any_text_finishes(text in any::<String>()) {
        strip_fences(&text);
        preprocess(&text);
    }

    #[test]
    fn preprocessing_fence_and_comment_heavy_text_finishes(text in lexer_heavy_text()) {
        strip_fences(&text);
        preprocess(&text);
    }
}

/// Parse one-shot, push at the given boundaries, then resync through a
/// rewind and an edit; serialize every result along the way.
fn assert_parses(text: &str, cuts: &[usize]) {
    parse(text, &LIBRARY, None).to_json();
    parse(text, &LIBRARY, Some("Card")).to_json();
    let chars: Vec<char> = text.chars().collect();
    let mut bounds: Vec<usize> = cuts.iter().map(|c| c % (chars.len() + 1)).collect();
    bounds.sort_unstable();
    bounds.push(chars.len());
    let mut stream = StreamParser::new(LIBRARY.clone(), None);
    let mut start = 0;
    for end in bounds {
        let chunk: String = chars[start..end.max(start)].iter().collect();
        stream.push(&chunk).to_json();
        start = end.max(start);
    }
    let half: String = chars[..chars.len() / 2].iter().collect();
    stream.set(&half).to_json(); // a rewind: starts over
    let edited = format!(
        "{half}Z{}",
        chars[chars.len() / 2..].iter().collect::<String>()
    );
    stream.set(&edited).to_json(); // continues the rewound text
    stream.set(text).to_json(); // not a continuation: starts over
    stream.result().to_json();
}

proptest! {
    #![proptest_config(config(1500))]

    #[test]
    fn parsing_any_text_finishes(text in any::<String>(), cuts in prop::collection::vec(any::<usize>(), 0..8)) {
        assert_parses(&text, &cuts);
    }

    #[test]
    fn parsing_program_like_text_finishes(text in lexer_heavy_text(), cuts in prop::collection::vec(any::<usize>(), 0..8)) {
        assert_parses(&text, &cuts);
    }
}
