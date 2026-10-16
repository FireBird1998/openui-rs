//! Merging any program with any patch never panics. The input is the
//! program, a zero byte, the patch, and optionally another zero byte and the
//! root's name.
//!
//!   cargo +nightly fuzz run merge -- -dict=fuzz/openui.dict
#![no_main]

use libfuzzer_sys::fuzz_target;
use openui_lang::merge_statements;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let mut parts = text.splitn(3, '\0');
    let existing = parts.next().unwrap_or_default();
    let patch = parts.next().unwrap_or_default();
    merge_statements(existing, patch, parts.next());
});
