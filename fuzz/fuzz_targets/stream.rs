//! Streaming any UTF-8, cut wherever the input's first bytes say and resynced
//! with `set`, never panics, and every push's result past a limit is flagged
//! and stays bounded (`checks.rs`), the limits scaling with the text so far.
//!
//! A stream of at most 256 pushes: a long input's chunks grow instead. A
//! statement that never closes is read again in full on every push, so in
//! one-character pushes an 8 KB one would cost thousands of whole-statement
//! parses, quadratic in its length (openui-rs #22, a known limit). Capping
//! the pushes keeps that within the 2-second timeout, so a timeout still means
//! a push costing more than it should.
//!
//!   cargo +nightly fuzz run stream -- -dict=fuzz/openui.dict
#![no_main]

mod checks;
mod library;

use libfuzzer_sys::fuzz_target;
use openui_lang::{Limits, StreamParser};

const MOST_PUSHES: usize = 256;

fuzz_target!(|data: &[u8]| {
    let Some((&plan, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let size = (usize::from(plan % 16) + 1).max(chars.len().div_ceil(MOST_PUSHES));
    let limits = if plan & 0x40 != 0 {
        checks::SMALL
    } else {
        Limits::DEFAULT
    };
    let mut stream = StreamParser::with_limits(library::LIBRARY.clone(), None, limits);
    for group in chars.chunks(size) {
        let start = group[0].0;
        let end = group.last().map_or(start, |&(i, c)| i + c.len_utf8());
        checks::check(end, limits, &stream.push(&text[start..end]));
    }
    if plan & 0x80 != 0 {
        let half = chars.get(chars.len() / 2).map_or(text.len(), |&(i, _)| i);
        checks::check(half, limits, &stream.set(&text[..half]));
        checks::check(text.len(), limits, &stream.set(text));
    }
    checks::check(text.len(), limits, &stream.result());
});
