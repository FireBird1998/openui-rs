//! Whatever text arrives, the stages finish without panicking.

use openui_lang::{Token, tokenize, tokens_to_json};
use proptest::prelude::*;

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
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn tokenizing_any_text_ends_in_eof(text in any::<String>()) {
        assert_tokenizes(&text);
    }

    #[test]
    fn tokenizing_lexer_heavy_text_ends_in_eof(text in lexer_heavy_text()) {
        assert_tokenizes(&text);
    }
}
