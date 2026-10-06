//! Whatever text arrives, parsing finishes without panicking: every stage,
//! one-shot, streamed at any chunk boundaries, and resynced with `set`. So do
//! merging, serializing any JSON tree, reading any text as JSON, and enriching
//! errors against any schema. Seeds are fixed, so a run is the same everywhere.

use std::sync::{Arc, LazyLock};

use openui_lang::{
    Library, StreamParser, Token, ValidationError, ValidationErrorCode, enrich_errors,
    enriched_errors_to_json, json_to_openui, merge_statements, parse, parse_json, preprocess,
    strip_fences, tokenize, tokens_to_json,
};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, RngSeed};
use serde_json::Value as Json;

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

proptest! {
    #![proptest_config(config(1500))]

    #[test]
    fn merging_any_texts_finishes(existing in any::<String>(), patch in any::<String>(), root in prop::option::of("[a-z$]{0,4}")) {
        merge_statements(&existing, &patch, root.as_deref());
    }

    #[test]
    fn merging_program_like_texts_finishes(existing in lexer_heavy_text(), patch in lexer_heavy_text(), root in prop::option::of(prop::sample::select(vec!["root", "x", "$s", ""]))) {
        merge_statements(&existing, &patch, root);
        merge_statements(&existing, &format!("```openui\n{patch}\n```"), root);
    }
}

/// Text dense in JSON's syntax, escapes and lone surrogates.
fn json_heavy_text() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "[", "]", "{", "}", "\"", "\\", "\\u", "D800", "DC00", "dfff", "\\n", ",", ":", " ", "\n",
        "0", "1", "-", "+", ".", "e", "E", "true", "false", "null", "\"k\"", "\u{1}", "é",
    ]);
    prop::collection::vec(pieces, 0..48).prop_map(|p| p.concat())
}

proptest! {
    #![proptest_config(config(2000))]

    #[test]
    fn reading_any_text_as_json_finishes(text in any::<String>()) {
        let _ = parse_json(&text);
    }

    #[test]
    fn reading_json_like_text_finishes(text in json_heavy_text()) {
        let _ = parse_json(&text);
    }
}

/// JSON trees dense in the keys and values the serializer and enrichment look
/// at: element and AST shapes, prototype names, odd types in every field.
fn tree() -> impl Strategy<Value = Json> {
    let leaf = prop_oneof![
        Just(Json::Null),
        any::<bool>().prop_map(Json::Bool),
        prop::sample::select(vec![0.0, 1.0, -1.5, 1e21, 3.0]).prop_map(|n| serde_json::json!(n)),
        prop::sample::select(vec![
            "element",
            "Text",
            "Stack",
            "Card",
            "Count",
            "Action",
            "Str",
            "Num",
            "Bool",
            "Null",
            "Ref",
            "Ph",
            "StateRef",
            "RuntimeRef",
            "Arr",
            "Obj",
            "BinOp",
            "UnaryOp",
            "Ternary",
            "Member",
            "Index",
            "Assign",
            "Comp",
            "+",
            "*",
            "||",
            "toString",
            "__proto__",
            "null",
            "",
            "0",
            "ab",
        ])
        .prop_map(|s| Json::String(s.to_owned())),
    ];
    let keys = prop::sample::select(vec![
        "type",
        "typeName",
        "props",
        "partial",
        "statementId",
        "k",
        "v",
        "n",
        "els",
        "entries",
        "op",
        "left",
        "right",
        "operand",
        "cond",
        "then",
        "else",
        "obj",
        "field",
        "index",
        "target",
        "value",
        "name",
        "args",
        "toString",
        "__proto__",
        "length",
        "0",
        "children",
        "properties",
        "required",
        "$defs",
    ]);
    leaf.prop_recursive(6, 96, 6, move |inner| {
        let name = prop::sample::select(vec!["Count", "Action", "Run", "Text", "Query", "Nope"]);
        let op = prop::sample::select(vec!["+", "-", "*", "&&", "||", "==", "toString", "??"]);
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Json::Array),
            prop::collection::vec((keys.clone(), inner.clone()), 0..6)
                .prop_map(|entries| Json::Object(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())),
            // Well-formed elements and AST nodes, with anything inside.
            (prop::sample::select(vec!["Text", "Stack", "Card", "Nope"]), prop::collection::vec((keys.clone(), inner.clone()), 0..4), inner.clone())
                .prop_map(|(t, props, id)| {
                    let props: serde_json::Map<String, Json> = props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
                    serde_json::json!({"type": "element", "typeName": t, "props": props, "partial": false, "statementId": id})
                }),
            (name, prop::collection::vec(inner.clone(), 0..4)).prop_map(|(n, args)| serde_json::json!({"k": "Comp", "name": n, "args": args})),
            (op, inner.clone(), inner.clone()).prop_map(|(op, l, r)| serde_json::json!({"k": "BinOp", "op": op, "left": l, "right": r})),
            prop::collection::vec(inner.clone(), 0..4).prop_map(|els| serde_json::json!({"k": "Arr", "els": els})),
            prop::collection::vec(("[a-z]{0,2}", inner.clone()), 0..4).prop_map(|entries| {
                let entries: Vec<Json> = entries.into_iter().map(|(k, v)| serde_json::json!([k, v])).collect();
                serde_json::json!({"k": "Obj", "entries": entries})
            }),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(c, t, e)| serde_json::json!({"k": "Ternary", "cond": c, "then": t, "else": e})),
            (prop::sample::select(vec!["Ref", "StateRef", "Ph", "RuntimeRef", "Str", "Num", "Bool"]), inner)
                .prop_map(|(k, v)| serde_json::json!({"k": k, "n": v, "v": v})),
        ]
    })
}

proptest! {
    #![proptest_config(config(1500))]

    #[test]
    fn serializing_any_tree_finishes(props in tree(), state in tree(), type_name in prop::sample::select(vec!["Text", "Stack", "Card", "Nope"]), id in tree()) {
        let element = serde_json::json!({"type": "element", "typeName": type_name, "props": props, "partial": false, "statementId": id});
        let _ = json_to_openui(&element, &LIBRARY, Some(&state));
        let _ = json_to_openui(&props, &LIBRARY, None);
    }

    #[test]
    fn enriching_against_any_schema_finishes(defs in tree(), component in prop::sample::select(vec!["Text", "Card", "0", "toString", "__proto__", "length"]), path in "(/[a-z0-9]{0,3}){0,2}") {
        let schema = serde_json::json!({"$defs": defs});
        let errors: Vec<ValidationError> = [
            ValidationErrorCode::MissingRequired,
            ValidationErrorCode::NullRequired,
            ValidationErrorCode::UnknownComponent,
            ValidationErrorCode::InlineReserved,
            ValidationErrorCode::ExcessArgs,
            ValidationErrorCode::TypeMismatch,
        ]
        .into_iter()
        .map(|code| ValidationError {
            code,
            component: component.to_owned(),
            path: path.clone(),
            message: "m".to_owned(),
            statement_id: None,
        })
        .collect();
        enriched_errors_to_json(&enrich_errors(&errors, &schema, &[]));
        enriched_errors_to_json(&enrich_errors(&errors, &defs, &["A".to_owned()]));
        let library = Library::from_json_schema(&schema);
        parse("root = Text(1)\nx = Card()", &library, None).to_json();
    }
}
