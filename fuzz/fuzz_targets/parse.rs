//! One-shot parsing of any UTF-8 never panics, its result serializes, and a
//! result past a limit is flagged and stays bounded (`checks.rs`): under the
//! default limits and under small ones the target hits often.
//!
//!   cargo +nightly fuzz run parse -- -dict=fuzz/openui.dict
#![no_main]

mod checks;
mod library;

use libfuzzer_sys::fuzz_target;
use openui_lang::{Limits, parse_with_limits};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    for (limits, root) in [
        (Limits::DEFAULT, None),
        (Limits::DEFAULT, Some("Card")),
        (checks::SMALL, None),
    ] {
        let result = parse_with_limits(text, &library::LIBRARY, root, limits);
        checks::check(text.len(), limits, &result);
    }
});
