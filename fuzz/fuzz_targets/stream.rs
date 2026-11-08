//! Streaming any UTF-8, cut wherever the input's first bytes say and resynced
//! with `set`, never panics, and every push's result past a limit is flagged
//! and stays bounded (`checks.rs`), the limits scaling with the text so far.
//!
//! A statement that never closes is read again in full on every push, and a
//! state default in it evaluated again, up to the work limit each time
//! (openui-rs #22, a known limit). So the pushes are capped by what that
//! costs at most: the work limit for the whole text times the pushes stays
//! within `MOST_WORK` steps, with at most 128 pushes; a long input's chunks
//! grow instead. That keeps the known worst case well within the 2-second
//! timeout, so a timeout still means a push costing more than it should. The
//! hardening run (#19) found the cap too loose twice: at 256 pushes a 6 KB
//! input whose stray quotes left a statement open, and at 128 an 884-byte
//! default left open and evaluated to the 1-million-step limit on each of
//! 127 pushes (0.9 s idle in the fuzz build).
//!
//!   cargo +nightly fuzz run stream -- -dict=fuzz/openui.dict
#![no_main]

mod checks;
mod library;

use libfuzzer_sys::fuzz_target;
use openui_lang::{Limits, StreamParser};

const MOST_PUSHES: usize = 128;
/// Work steps a whole stream may take at most, at the work limit on every
/// push: about 0.5 s in the fuzz build.
const MOST_WORK: usize = 64_000_000;

fuzz_target!(|data: &[u8]| {
    let Some((&plan, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let limits = if plan & 0x40 != 0 {
        checks::SMALL
    } else {
        Limits::DEFAULT
    };
    let pushes = (MOST_WORK / limits.work_for(text.len())).clamp(1, MOST_PUSHES);
    let size = (usize::from(plan % 16) + 1).max(chars.len().div_ceil(pushes));
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
