//! The binding hands Swift exactly what the core produces, for every case.

#[path = "../../openui-lang/tests/common/mod.rs"]
mod common;

use std::fmt::Write;

use openui_ffi::{ComponentLibrary, OpenUiError, ParseOutput, parse};

#[test]
fn one_shot_parse_matches_the_core_on_every_case() {
    let corpus = common::load_corpus();
    let mut report = String::new();
    for case in &corpus.cases {
        let library = ComponentLibrary::new(case.schema.to_string()).unwrap();
        let got = parse(case.input.clone(), library, case.root.clone());
        let want: ParseOutput =
            openui_lang::parse(&case.input, &case.library, case.root.as_deref()).into();
        if got != want {
            let _ = write!(report, "\n✗ {}", case.name);
        }
    }
    assert!(
        report.is_empty(),
        "the binding differs from the core:{report}"
    );
}

#[test]
fn a_schema_that_is_not_json_is_an_error() {
    let Err(error) = ComponentLibrary::new("{\"$defs\":".to_owned()) else {
        panic!("loaded a truncated schema");
    };
    assert!(matches!(error, OpenUiError::InvalidSchema { .. }));
}

#[test]
fn component_names_keep_the_schema_order() {
    let schema = r#"{"$defs": {"Stack": {"properties": {}}, "Card": {"properties": {}}, "Text": {"properties": {}}}}"#;
    let library = ComponentLibrary::new(schema.to_owned()).unwrap();
    assert_eq!(library.component_names(), ["Stack", "Card", "Text"]);
}
