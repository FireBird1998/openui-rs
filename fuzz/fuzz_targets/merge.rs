//! Merging any program with any patch never panics, and the merge is never
//! longer than its two inputs (give or take the newlines that join them): it
//! has no result-size limit because it can't amplify. The input is the
//! program, a zero byte, the patch, and optionally another zero byte and the
//! root's name.
//!
//!   cargo +nightly fuzz run merge -- -dict=fuzz/openui.dict
#![no_main]

use libfuzzer_sys::fuzz_target;
use openui_lang::{Limits, merge_statements_with_limits};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut parts = text.splitn(3, '\0');
    let existing = parts.next().unwrap_or_default();
    let patch = parts.next().unwrap_or_default();
    let merged = merge_statements_with_limits(existing, patch, parts.next(), Limits::DEFAULT);
    let inputs = existing.len() + patch.len();
    assert!(
        merged.text.len() <= inputs + 64,
        "a merge grew past its inputs: {} bytes from {inputs}",
        merged.text.len()
    );
});
