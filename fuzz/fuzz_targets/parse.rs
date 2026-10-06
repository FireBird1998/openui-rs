//! One-shot parsing of any UTF-8 never panics, and its result serializes.
//!
//!   cargo +nightly fuzz run parse -- -dict=fuzz/openui.dict
#![no_main]

mod library;

use libfuzzer_sys::fuzz_target;
use openui_lang::parse;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    parse(text, &library::LIBRARY, None).to_json();
    parse(text, &library::LIBRARY, Some("Card")).to_json();
});
