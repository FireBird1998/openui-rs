//! Converting any JSON back to source never panics: an element tree, with or
//! without state declarations, or anything else, which is an error.
//!
//!   cargo +nightly fuzz run serialize -- -dict=fuzz/openui.dict
#![no_main]

mod library;

use libfuzzer_sys::fuzz_target;
use openui_lang::{json_to_openui, parse_json};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let Ok(json) = parse_json(text) else { return };
    // A whole parse result: its root, with its state declarations.
    if let Some(root) = json.get("root") {
        let _ = json_to_openui(root, &library::LIBRARY, json.get("stateDeclarations"));
    }
    let _ = json_to_openui(&json, &library::LIBRARY, None);
});
