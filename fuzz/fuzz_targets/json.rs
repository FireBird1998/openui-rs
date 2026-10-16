//! Reading any text as JSON never panics, and what it reads writes back as
//! JSON that reads to the same: writing it twice gives the same text.
//!
//!   cargo +nightly fuzz run json
#![no_main]

use libfuzzer_sys::fuzz_target;
use openui_lang::{parse_json, stringify_json};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let Ok(value) = parse_json(text) else { return };
    let once = stringify_json(&value);
    let again = parse_json(&once).expect("written JSON reads back");
    assert_eq!(stringify_json(&again), once, "writing JSON settles after one round");
});
