//! Cleaning up model output before parsing, as the reference parser's
//! `stripFences`, `stripComments` and `preprocess`.
//!
//! Byte positions stand in for the reference's UTF-16 positions: every
//! position that's sliced at is a backtick, a newline, a quote or a comment
//! marker, all ASCII.

use crate::js;

/// Model output ready to parse: trimmed, the code fences' contents (if any),
/// `//` and `#` comments removed, trimmed again.
pub fn preprocess(input: &str) -> String {
    let stripped = strip_comments(&strip_fences(js::trim(input)));
    js::trim(&stripped).to_owned()
}

/// Where a `"..."` string starting at `start` ends (just past its closing
/// quote, or at or past the end if unclosed); `start` itself if no string starts there.
fn skip_string(s: &[u8], start: usize) -> usize {
    if s.get(start) != Some(&b'"') {
        return start;
    }
    let mut i = start + 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// The first ``` at or after `from` that isn't inside a `"..."` string.
fn find_fence(s: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < s.len() {
        let next = skip_string(s, i);
        if next > i {
            i = next;
            continue;
        }
        if s[i..].starts_with(b"```") {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The contents of every markdown code fence, joined by newlines; the input
/// as-is when there are none. An unclosed fence (mid-stream) runs to the end.
/// ``` inside a `"..."` string doesn't count.
pub fn strip_fences(input: &str) -> String {
    let s = input.as_bytes();
    let mut blocks: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let Some(open) = find_fence(s, i) else { break };
        // The rest of the opening line is the language tag.
        let Some(newline) = s[open + 3..].iter().position(|&b| b == b'\n') else {
            // No newline yet after the fence: nothing of the block has arrived.
            blocks.push("");
            break;
        };
        let body = open + 3 + newline + 1;
        match find_fence(s, body) {
            Some(close) => {
                blocks.push(&input[body..close]);
                i = close + 3;
            }
            None => {
                blocks.push(&input[body..]);
                break;
            }
        }
    }
    if !blocks.is_empty() {
        return blocks.join("\n");
    }

    // The reference keeps this fallback for input starting with ``` that wasn't
    // matched above, though the scan always matches it first.
    if let Some(rest) = input.strip_prefix("```") {
        let body = match rest.find('\n') {
            Some(newline) => &rest[newline + 1..],
            None => rest,
        };
        return match body.rfind("```") {
            Some(fence) => body[..fence].to_owned(),
            None => body.to_owned(),
        };
    }
    input.to_owned()
}

/// Remove `//` and `#` comments outside strings, then trailing whitespace on
/// lines that had one. String state carries over from line to line, so an
/// apostrophe in prose opens a `'...'` string that can hide later comments.
fn strip_comments(input: &str) -> String {
    let mut in_str: Option<u8> = None;
    let lines: Vec<&str> = input
        .split('\n')
        .map(|line| {
            let s = line.as_bytes();
            let mut i = 0;
            while i < s.len() {
                let c = s[i];
                if let Some(quote) = in_str {
                    if c == b'\\' && i + 1 < s.len() {
                        i += 2;
                        continue;
                    }
                    if c == quote {
                        in_str = None;
                    }
                } else if c == b'"' || c == b'\'' {
                    in_str = Some(c);
                } else if (c == b'/' && s.get(i + 1) == Some(&b'/')) || c == b'#' {
                    return js::trim_end(&line[..i]);
                }
                i += 1;
            }
            line
        })
        .collect();
    lines.join("\n")
}
