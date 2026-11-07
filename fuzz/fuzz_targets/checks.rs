// What the targets check of every result besides not panicking (ADR 0001): a
// result past a limit says so with its flag, and a cut result stays bounded.
// Amplification is a finding, not a pass. Shared by the parse and stream
// targets.

use openui_lang::{Limits, ParseResult};

/// Small limits, so the targets hit them often: a 1 KiB floor and 4 times
/// the program for the result, a little work, and 16 levels.
pub const SMALL: Limits = Limits::DEFAULT
    .with_result_floor(1024)
    .with_amplification(4)
    .with_work(100_000)
    .with_depth(16);

/// Check `result`, parsed from `program_bytes` of program under `limits`.
///
/// The result-size limit is priced as the result is built, not by writing it
/// out, so it's approximate: an unflagged result may run past it by half
/// again plus 4 KiB (over 23,754 corpus inputs, under six sets of limits, the
/// most was 16%). Past that, it was over the limit and should have been cut
/// and flagged. A cut result keeps what fit plus a placeholder for each node
/// left (`{"k":"Null"}`, a dozen bytes for a node of a few bytes of program),
/// so it may run past by 16 bytes a byte of program more, never with the
/// amplification. And an unflagged result nests no deeper than the JSON
/// entry points read: three levels a level of the depth limit, plus 32.
pub fn check(program_bytes: usize, limits: Limits, result: &ParseResult) {
    let json = result.to_json();
    let limit = limits.result_size_for(program_bytes);
    let slack = limit / 2 + 4096;
    if !result.size_limited {
        assert!(
            json.len() <= limit + slack,
            "past the result-size limit, unflagged: {} bytes of JSON from {program_bytes} of program, limit {limit}",
            json.len()
        );
    }
    assert!(
        json.len() <= limit + slack + 16 * program_bytes,
        "a cut result kept growing: {} bytes of JSON from {program_bytes} of program, limit {limit}",
        json.len()
    );
    if !result.depth_limited {
        let deepest = nesting(&json);
        let most = 3 * limits.depth() + 32;
        assert!(
            deepest <= most,
            "past the depth limit, unflagged: JSON nests {deepest} deep, at most {most}"
        );
    }
}

/// How deep `json` nests.
fn nesting(json: &str) -> usize {
    let (mut depth, mut deepest, mut in_string, mut escaped) = (0usize, 0, false, false);
    for b in json.bytes() {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}
