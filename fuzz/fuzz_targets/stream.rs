//! Streaming any UTF-8, cut wherever the input's first bytes say and resynced
//! with `set`, never panics.
//!
//!   cargo +nightly fuzz run stream -- -dict=fuzz/openui.dict
#![no_main]

mod library;

use libfuzzer_sys::fuzz_target;
use openui_lang::StreamParser;

fuzz_target!(|data: &[u8]| {
    let Some((&plan, rest)) = data.split_first() else { return };
    let Ok(text) = std::str::from_utf8(rest) else { return };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let size = usize::from(plan % 16) + 1;
    let mut stream = StreamParser::new(library::LIBRARY.clone(), None);
    for group in chars.chunks(size) {
        let start = group[0].0;
        let end = group.last().map_or(start, |&(i, c)| i + c.len_utf8());
        stream.push(&text[start..end]).to_json();
    }
    if plan & 0x80 != 0 {
        let half = chars.get(chars.len() / 2).map_or(text.len(), |&(i, _)| i);
        stream.set(&text[..half]).to_json();
        stream.set(text).to_json();
    }
    stream.result().to_json();
});
