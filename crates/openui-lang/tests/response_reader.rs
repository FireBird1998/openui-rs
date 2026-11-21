//! The incremental response reader against the one-shot readers it replaces
//! in the drivers: for every prefix of every text, in many chunkings and
//! after edits that are not appends, the reader's read and parse are byte for
//! byte what reading and parsing the whole text afresh gives, including
//! refusals. And a refused update leaves the reader where it was.

mod common;

#[path = "support/response_bundles.rs"]
#[allow(dead_code)]
mod support;

use std::sync::Arc;

use common::{conformance_dir, read};
use openui_lang::{
    Library, Limits, MessageError, ResponseBundleError, ResponseReader, ResponseReaderError,
    StreamParser, parse_message, parse_response_bundle, response_bundle_json_utf16,
    response_draw_json_utf16,
};
use serde_json::Value;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

fn library() -> Arc<Library> {
    let conformance = conformance_dir();
    let schema = serde_json::from_str(&read(&conformance.join("imported/schemas/bench.json")))
        .expect("bench schema");
    Arc::new(Library::from_json_schema(&schema))
}

/// Steps checked against a full read, across this file's tests.
static STEPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn count_step() {
    STEPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// A small deterministic generator: the same texts and cuts on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Pieces random texts are made of: every marker, marker fragments, line
/// breaks of every kind, quotes and comments the scripts scan follows, JSON
/// for the scripts section, and characters of every UTF-8 length.
const PIECES: &[&str] = &[
    "]]>openui:content",
    "]]>openui:content\n",
    "]]>openui:content?name=Daily%20report&x=1\n",
    "]]>openui:content?",
    "\n]]>openui:context\n",
    "]]>openui:context",
    "\n]]>openui:end",
    "]]>openui:end\n",
    "\n]]>openui:scripts {\"count\":1}\n",
    "]]>openui:scripts",
    "]]>openui:scripts?x",
    "]]>openui:unknown\n",
    "[{\"name\":\"a\",\"code\":\"x()\"}]",
    "[{\"name\":\"b\",\"code\":\"[[\\\"]]>\\\"]]\"}]",
    "[[[[",
    "]]]]",
    "{\"k\":[1,2]}",
    "\n",
    "\r\n",
    "\r",
    "\u{2028}",
    "]",
    "]]",
    "]]>",
    "]]>open",
    "?",
    "#",
    "//",
    "/",
    "\"",
    "'",
    "\\",
    " ",
    "a",
    "root = Stack([t])\n",
    "t = TextContent(\"hi\")\n",
    "$n = 1\n",
    "x = [\n",
    "]\n",
    "é",
    "€",
    "😀",
];

fn random_text(rng: &mut Rng) -> String {
    let pieces = 1 + rng.below(24);
    (0..pieces)
        .map(|_| PIECES[rng.below(PIECES.len())])
        .collect()
}

fn fixed_texts() -> Vec<String> {
    let program = "root = Stack([title, rows])\ntitle = TextContent(\"Report\")\nrows = [\n  TextContent(\"a\"),\n  TextContent(\"b\")\n]\n$count = 3\n";
    let crlf = program.replace('\n', "\r\n");
    let scripts = "\n]]>openui:scripts {\"count\":2}\n[{\"name\":\"a\",\"code\":\"return 1 // ]]> not a marker\"},{\"name\":\"b\",\"code\":\"x = '#'\"}]\n";
    vec![
        program.to_owned(),
        crlf.clone(),
        format!("]]>openui:content?name=Daily\n{program}]]>openui:end\n"),
        format!(
            "]]>openui:content\r\n{crlf}\r\n]]>openui:context\r\n{{\"user\":\"ü\"}}\r\n]]>openui:end"
        ),
        format!(
            "]]>openui:content?name=first\n{program}partial]]>openui:content?name=retry\n{program}]]>openui:end\n"
        ),
        format!("]]>openui:content\n{program}{scripts}]]>openui:end\n"),
        format!(
            "]]>openui:content\r\n{crlf}{}]]>openui:end\r\n",
            scripts.replace('\n', "\r\n")
        ),
        format!("{program}]]>openui:scripts\n[]\n"),
        format!("]]>openui:content\n{program}\n]]>openui:end\n\n]]>openui:end\n\n"),
        format!("x\n]]>openui:end\n\nmore\n]]>openui:content\n{program}"),
        format!(
            "]]>openui:content\n{program}\n]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"a\",\"code\":\"1\"}}] trailing\n"
        ),
        format!(
            "]]>openui:content\n{program}\n]]>openui:scripts {{\"count\":1}}\n{}\n",
            "[".repeat(1000)
        ),
        format!(
            "]]>openui:content\n{program}\n]]>openui:scripts {}\n[]\n",
            "[".repeat(1000)
        ),
        format!("```openui\n{program}```\n"),
        format!(
            "]]>openui:content\n{program}\n]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"a\",\"code\":\"1\"}}]\n]]>openui:other\ntail = 1\n"
        ),
        // The program goes on after the section, past a `]]>` that is no
        // marker, then a CRLF line and a second scripts marker.
        format!(
            "]]>openui:content\n{program}\n]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"a\",\"code\":\"1\"}}]\n]]> no marker\n{program}more = 1\r\n]]>openui:scripts\nx = 2\n"
        ),
        format!("]]>openui:content\n{program}]]>openui:scripts\n[]\n]]>\n{crlf}]]>openui:end\n"),
    ]
}

/// Cloud script code of about `bytes` bytes, the content of one JSON string,
/// so one line: escapes, `]]` (an index), non-ASCII and comments in it.
fn script_code(bytes: usize) -> String {
    let piece =
        "const rows = await query(\\\"sales\\\"); // é\\nlet top = rows[ids[0]] ?? \\u00e9;\\n";
    let mut code = String::with_capacity(bytes + piece.len());
    while code.len() < bytes {
        code.push_str(piece);
    }
    code
}

/// A bundle whose one script's code is `code`, on one line.
fn one_script_line(code: &str) -> String {
    format!(
        "]]>openui:content?name=Long\nroot = TextContent(\"x\")\n]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"refresh\",\"code\":\"{code}\"}}]\n]]>openui:end\n"
    )
}

/// What reading and parsing the whole text gives, as the drivers saw it.
struct Expected {
    read: Result<(String, Option<String>), ResponseReaderError>,
    program: String,
}

fn expected_core(text: &str, streaming: bool) -> Expected {
    expected_core_with(text, streaming, Limits::DEFAULT)
}

fn expected_core_with(text: &str, streaming: bool, limits: Limits) -> Expected {
    let bundle = match parse_response_bundle(Some(text), streaming, limits) {
        Ok(bundle) => bundle,
        Err(error) => {
            return Expected {
                read: Err(ResponseReaderError::Bundle(error)),
                program: String::new(),
            };
        }
    };
    let message = if text.contains("]]>") {
        match parse_message(text, streaming, limits) {
            Ok(message) => Some(without_content(&message.to_json())),
            Err(error) => {
                return Expected {
                    read: Err(ResponseReaderError::Message(error)),
                    program: String::new(),
                };
            }
        }
    } else {
        None
    };
    Expected {
        read: Ok((bundle.to_json(), message)),
        program: bundle.program.clone(),
    }
}

/// `{"content":…,"context":C,"attributes":A,"end":E}` as `{"context":C,"attributes":A}`.
fn without_content(json: &str) -> String {
    let at = json.find(",\"context\":").expect("context field");
    let end = json.rfind(",\"end\":").expect("end field");
    format!("{{{}}}", &json[at + 1..end])
}

/// The WebAssembly transport's read: the draw JSON (program left out, change
/// flag added later) and the bundle JSON with its program.
fn expected_utf16(
    units: &[u16],
    streaming: bool,
    limits: Limits,
) -> Result<(String, String), ResponseBundleError> {
    let draw = response_draw_json_utf16(units, streaming, limits)?;
    let bundle = response_bundle_json_utf16(units, streaming, limits)?;
    let at = draw.find(",\"metadata\"").expect("metadata field");
    let draw = format!("{{\"bundle\":{{{}", &draw[at + 1..]);
    Ok((draw, bundle))
}

/// The bundle's program in the core's representation, which keeps lone
/// surrogates (the driver compares programs as JavaScript strings).
fn program_of(bundle_json: &str) -> String {
    let value = openui_lang::parse_json(bundle_json).expect("bundle JSON");
    value["program"].as_str().unwrap_or_default().to_owned()
}

/// The program as the JavaScript driver handed it to Rust to parse: a
/// string, lone surrogates replaced.
fn lossy(program: &str) -> String {
    String::from_utf16_lossy(&openui_lang::utf16_units(program).collect::<Vec<_>>())
}

/// A fresh parse of `program`, as the drivers' full re-read did it.
fn fresh_parse(library: &Arc<Library>, program: &str, complete: bool) -> String {
    fresh_parse_with(library, program, complete, Limits::DEFAULT)
}

fn fresh_parse_with(
    library: &Arc<Library>,
    program: &str,
    complete: bool,
    limits: Limits,
) -> String {
    let mut parser = StreamParser::with_limits(library.clone(), None, limits);
    let partial = parser.set(program);
    if complete {
        parser.finish().to_json()
    } else {
        partial.to_json()
    }
}

/// Drives one reader through a sequence of texts the way the Swift driver
/// does, checking every step against the one-shot readers.
struct CoreDriver {
    reader: ResponseReader,
    library: Arc<Library>,
    limits: Limits,
    accepted: Option<String>,
}

impl CoreDriver {
    fn new(library: &Arc<Library>) -> Self {
        Self::with_limits(library, Limits::DEFAULT)
    }

    fn with_limits(library: &Arc<Library>, limits: Limits) -> Self {
        Self {
            reader: ResponseReader::new(
                StreamParser::with_limits(library.clone(), None, limits),
                limits,
            ),
            library: library.clone(),
            limits,
            accepted: None,
        }
    }

    /// One step; whether the reader kept the text.
    fn step(&mut self, text: &str, streaming: bool, refuse: bool, what: &str) -> bool {
        count_step();
        let expected = expected_core_with(text, streaming, self.limits);
        let actual = self.reader.read(text, streaming).and_then(|()| {
            let message = self.reader.message_json()?;
            Ok((self.reader.bundle_json(), message))
        });
        assert_eq!(actual, expected.read, "{what}: read of {text:?}");
        if expected.read.is_err() {
            return false;
        }
        let complete = !streaming;
        let parsed = self.reader.try_parse(true, complete, |result| {
            let result = result.expect("a parse").to_json();
            if refuse { Err(result) } else { Ok(result) }
        });
        let parsed = parsed.unwrap_or_else(|refused| refused);
        assert_eq!(
            parsed,
            fresh_parse_with(&self.library, &expected.program, complete, self.limits),
            "{what}: parse of {text:?}"
        );
        if !refuse {
            self.accepted = Some(text.to_owned());
        }
        !refuse
    }
}

/// The same for the WebAssembly driver, which sends only what was appended.
struct Utf16Driver {
    reader: ResponseReader,
    library: Arc<Library>,
    limits: Limits,
    accepted: Vec<u16>,
    program: String,
}

impl Utf16Driver {
    fn new(library: &Arc<Library>) -> Self {
        Self::with_limits(library, Limits::DEFAULT)
    }

    fn with_limits(library: &Arc<Library>, limits: Limits) -> Self {
        Self {
            reader: ResponseReader::new_utf16(
                StreamParser::with_limits(library.clone(), None, limits),
                limits,
            ),
            library: library.clone(),
            limits,
            accepted: Vec::new(),
            program: String::new(),
        }
    }

    /// One step; whether the reader kept the text.
    fn step(&mut self, units: &[u16], streaming: bool, refuse: bool, what: &str) -> bool {
        count_step();
        let expected = expected_utf16(units, streaming, self.limits);
        let read = if units.starts_with(&self.accepted) && !self.accepted.is_empty() {
            self.reader.read_utf16_append(
                self.accepted.len(),
                &units[self.accepted.len()..],
                streaming,
            )
        } else {
            self.reader.read_utf16(units, streaming)
        };
        let actual = read.map(|()| (self.reader.draw_json(), self.reader.bundle_json()));
        let text = String::from_utf16_lossy(units);
        match expected {
            Err(error) => {
                assert_eq!(
                    actual,
                    Err(ResponseReaderError::Bundle(error)),
                    "{what}: {text:?}"
                );
                false
            }
            Ok((draw, bundle)) => {
                let program = program_of(&bundle);
                let changed = program != self.program;
                let draw = format!(
                    "{},\"programChanged\":{changed}}}",
                    draw.strip_suffix('}').expect("object")
                );
                assert_eq!(actual, Ok((draw, bundle)), "{what}: read of {text:?}");
                let complete = !streaming;
                let parse = self.accepted.is_empty() || changed;
                let parsed = self.reader.try_parse(parse, complete, |result| {
                    let result = result.map(|result| result.to_json());
                    if refuse { Err(result) } else { Ok(result) }
                });
                let parsed = parsed.unwrap_or_else(|refused| refused);
                if let Some(parsed) = parsed {
                    assert_eq!(
                        parsed,
                        fresh_parse_with(&self.library, &lossy(&program), complete, self.limits),
                        "{what}: parse of {text:?}"
                    );
                }
                if !refuse {
                    self.accepted = units.to_vec();
                    self.program = program;
                }
                !refuse
            }
        }
    }
}

/// Cuts of `len` units: every one, or random sizes, ending at `len`.
fn cuts(len: usize, rng: &mut Rng, mode: usize) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut at = 0;
    while at < len {
        at += match mode {
            0 => 1,
            1 => 1 + rng.below(4),
            _ => 1 + rng.below(40),
        };
        cuts.push(at.min(len));
    }
    cuts
}

/// The UTF-8 cut nearest at or after `at`, for the core's strings.
fn char_cut(text: &str, mut at: usize) -> usize {
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at.min(text.len())
}

fn run(text: &str, rng: &mut Rng, library: &Arc<Library>, refusals: bool) {
    let units: Vec<u16> = text.encode_utf16().collect();
    for mode in 0..3 {
        let mut core = CoreDriver::new(library);
        let mut web = Utf16Driver::new(library);
        for &cut in &cuts(units.len(), rng, mode) {
            let refuse = refusals && rng.below(5) == 0;
            web.step(
                &units[..cut],
                true,
                refuse,
                &format!("mode {mode}, unit cut {cut}"),
            );
        }
        // Byte cuts, moved to the next character: the core takes Rust strings.
        for &cut in &cuts(text.len(), rng, mode) {
            let refuse = refusals && rng.below(5) == 0;
            let at = char_cut(text, cut);
            core.step(
                &text[..at],
                true,
                refuse,
                &format!("mode {mode}, byte cut {at}"),
            );
        }
        web.step(&units, false, false, "settled");
        core.step(text, false, false, "settled");
    }
}

#[test]
fn every_prefix_of_the_fixed_texts_reads_as_a_full_read() {
    let library = library();
    let mut rng = Rng(0x0116_2026);
    for text in fixed_texts() {
        run(&text, &mut rng, &library, false);
    }
}

#[test]
fn every_prefix_of_the_response_bundle_corpus_reads_as_a_full_read() {
    let library = library();
    let mut rng = Rng(0x0116_0098);
    let corpus: Value = serde_json::from_str(support::CASES).expect("corpus");
    let mut checked = 0;
    for case in corpus["cases"].as_array().expect("cases") {
        let Some(units) = support::units(case.get("units").unwrap_or(&case["raw"])) else {
            continue;
        };
        let mut web = Utf16Driver::new(&library);
        for &cut in &cuts(units.len(), &mut rng, 0) {
            web.step(
                &units[..cut],
                true,
                false,
                case["name"].as_str().unwrap_or("?"),
            );
        }
        web.step(&units, false, false, "settled");
        // Both drivers, on the cases a Rust string can hold.
        if let Ok(text) = String::from_utf16(&units) {
            run(&text, &mut rng, &library, false);
        }
        checked += 1;
    }
    assert!(checked > 90);
}

#[test]
fn random_texts_in_random_chunks_read_as_a_full_read_with_refusals() {
    let library = library();
    let mut rng = Rng(0x5eed_0116);
    for _ in 0..400 {
        let text = random_text(&mut rng);
        run(&text, &mut rng, &library, true);
    }
    println!(
        "{} steps checked against a full read so far",
        STEPS.load(std::sync::atomic::Ordering::Relaxed)
    );
}

#[test]
fn lone_surrogates_split_across_chunks_read_as_a_full_read() {
    let library = library();
    let mut rng = Rng(0x0d83_d800);
    let mut texts: Vec<Vec<u16>> = vec![
        "]]>openui:content?name=a\nroot = TextContent(\"x"
            .encode_utf16()
            .chain([0xd83d, 0xde00, 0xd800])
            .chain("\")\n".encode_utf16())
            .collect(),
        "a".encode_utf16().chain([0xd83d]).collect(),
        [0xdc00, 0xd83d, 0xde00, 0xd800, 0x000a, 0xdbff].to_vec(),
    ];
    for _ in 0..200 {
        let mut units: Vec<u16> = random_text(&mut rng).encode_utf16().collect();
        for _ in 0..rng.below(4) {
            let at = rng.below(units.len() + 1);
            units.insert(at, [0xd800, 0xdbff, 0xdc00, 0xdfff][rng.below(4)]);
        }
        texts.push(units);
    }
    for units in texts {
        for mode in 0..3 {
            let mut web = Utf16Driver::new(&library);
            for &cut in &cuts(units.len(), &mut rng, mode) {
                let refuse = rng.below(6) == 0;
                web.step(&units[..cut], true, refuse, "lone surrogates");
            }
            web.step(&units, false, false, "settled");
        }
    }
}

/// Edits that are not appends take the full re-read and still match: a
/// shrink, a changed middle byte, the same length with other content, and
/// the same prefix then something else.
#[test]
fn edits_that_are_not_appends_read_as_a_full_read() {
    let library = library();
    let mut rng = Rng(0xed17_0116);
    let mut texts = fixed_texts();
    for _ in 0..150 {
        texts.push(random_text(&mut rng));
    }
    for text in texts {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        if chars.len() < 4 {
            continue;
        }
        let mut core = CoreDriver::new(&library);
        let mut web = Utf16Driver::new(&library);
        let mut feed = |next: &str, what: &str| {
            let units: Vec<u16> = next.encode_utf16().collect();
            core.step(next, true, false, what);
            web.step(&units, true, false, what);
        };
        let middle = chars[chars.len() / 2].0;
        feed(&text, "whole");
        feed(&text[..middle], "shrunk");
        feed(&text, "grown back");
        let mut replaced = text.clone();
        let (at, ch) = chars[chars.len() / 3];
        replaced.replace_range(at..at + ch.len_utf8(), if ch == 'x' { "y" } else { "x" });
        feed(&replaced, "middle replaced");
        let mut same_length = text.clone();
        let (last, ch) = chars[chars.len() - 1];
        same_length.replace_range(
            last..last + ch.len_utf8(),
            if ch == 'z' { "q" } else { "z" },
        );
        feed(&text, "original");
        feed(&same_length, "same length, other end");
        let mut diverged = text[..middle].to_owned();
        diverged.push_str("]]>openui:content?name=edit\nroot = TextContent(\"other\")\n");
        feed(&diverged, "same prefix, then other");
        feed(&format!("{diverged}more"), "append after the edit");
    }
}

/// A refused update leaves the reader where it was: the next text, an append
/// to the last accepted one, reads and parses as if the refused one never
/// came, including the program-change flag.
#[test]
fn a_refused_update_leaves_the_reader_where_it_was() {
    let library = library();
    let text = fixed_texts()[5].clone();
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut control = Utf16Driver::new(&library);
    let mut staged = Utf16Driver::new(&library);
    for cut in (7..units.len()).step_by(7) {
        // The refused text is a longer append, a shrink or a different text.
        let refused: Vec<u16> = match cut % 3 {
            0 => units[..(cut + 11).min(units.len())].to_vec(),
            1 => units[..cut / 2].to_vec(),
            _ => "]]>openui:content\nroot = TextContent(\"refused\")\n"
                .encode_utf16()
                .collect(),
        };
        staged.step(&refused, true, true, "refused");
        staged.step(&units[..cut], true, false, "after a refusal");
        control.step(&units[..cut], true, false, "control");
        assert_eq!(staged.reader.draw_json(), control.reader.draw_json());
        assert_eq!(
            staged.reader.finish().to_json(),
            control.reader.finish().to_json()
        );
    }
    // A refused read is refused before anything is staged, too.
    let too_long = vec![u16::from(b'a'); Limits::DEFAULT.program_size() + 1];
    assert_eq!(
        staged.reader.read_utf16(&too_long, true),
        Err(ResponseReaderError::Bundle(
            ResponseBundleError::ProgramSize
        ))
    );
    assert_eq!(
        staged.reader.read_utf16_append(3, &[], true),
        Err(ResponseReaderError::NotAnAppend)
    );
    staged.step(&units, false, false, "settled after refusals");
}

/// The whole response counts toward the byte limit, however it arrives: a
/// response one byte over is refused on the append that crosses it, and a
/// pair split across chunks is charged as the pair (four bytes), not as two
/// lone surrogates (six).
#[test]
fn the_whole_response_counts_toward_the_byte_limit() {
    let library = library();
    let limits = Limits::DEFAULT.with_program_size(64);
    let mut reader = ResponseReader::new_utf16(
        StreamParser::with_limits(library.clone(), None, limits),
        limits,
    );
    let mut sent = 0;
    let text: Vec<u16> = "a".repeat(64).encode_utf16().collect();
    for chunk in text.chunks(16) {
        reader
            .read_utf16_append(sent, chunk, true)
            .expect("within the limit");
        reader
            .try_parse(true, false, Ok::<_, ()>)
            .expect("accepted");
        sent += chunk.len();
    }
    assert_eq!(
        reader.read_utf16_append(sent, &[u16::from(b'b')], true),
        Err(ResponseReaderError::Bundle(
            ResponseBundleError::ProgramSize
        ))
    );
    // 60 bytes, then a pair split across two chunks: 64 in all.
    let mut reader = ResponseReader::new_utf16(
        StreamParser::with_limits(library.clone(), None, limits),
        limits,
    );
    let head: Vec<u16> = "a".repeat(60).encode_utf16().chain([0xd83d]).collect();
    reader.read_utf16(&head, true).expect("63 bytes");
    reader
        .try_parse(true, false, Ok::<_, ()>)
        .expect("accepted");
    reader
        .read_utf16_append(head.len(), &[0xde00], true)
        .expect("the pair is four bytes");
    reader
        .try_parse(true, false, Ok::<_, ()>)
        .expect("accepted");
    assert_eq!(
        reader.read_utf16_append(head.len() + 1, &[u16::from(b'b')], true),
        Err(ResponseReaderError::Bundle(
            ResponseBundleError::ProgramSize
        ))
    );
    // The core's strings, the same.
    let mut core = ResponseReader::new(StreamParser::with_limits(library, None, limits), limits);
    let text = "a".repeat(64);
    for cut in (16..=64).step_by(16) {
        core.read(&text[..cut], true).expect("within the limit");
        core.try_parse(true, false, Ok::<_, ()>).expect("accepted");
    }
    assert_eq!(
        core.read(&format!("{text}b"), true),
        Err(ResponseReaderError::Bundle(
            ResponseBundleError::ProgramSize
        ))
    );
    // A context nested past the depth limit is refused as the message reader refuses it.
    let deep = format!(
        "]]>openui:content\nx\n]]>openui:context\n{}",
        "[".repeat(1000)
    );
    let mut core = CoreDriver::new(&library_for_depth());
    core.step(&deep, true, false, "deep context");
    assert!(matches!(
        expected_core(&deep, true).read,
        Err(ResponseReaderError::Bundle(
            ResponseBundleError::ContextDepth
        )) | Err(ResponseReaderError::Message(MessageError::ContextDepth))
    ));
}

fn library_for_depth() -> Arc<Library> {
    library()
}

// ─── Cost ───────────────────────────────────────────────────────────────────

/// 64 KiB of a real program, cut at a character boundary.
fn program_64k() -> String {
    program_of_size(65_536)
}

/// `bytes` of a real program, cut at a line break.
fn program_of_size(bytes: usize) -> String {
    let conformance = conformance_dir();
    let one = read(&conformance.join("cases/synthetic/dashboard-x8.oui"));
    let mut text = String::new();
    while text.len() < bytes {
        text.push_str(&one);
        text.push('\n');
    }
    let at = text[..bytes].rfind('\n').map_or(bytes, |at| at + 1);
    text.truncate(at);
    text
}

/// The shapes a response takes, around a program.
fn shapes(program: &str) -> Vec<(&'static str, String)> {
    let script = "const rows = await query();\\n".repeat(16);
    let (head, tail) = program.split_at(
        program[..program.len() / 2]
            .rfind('\n')
            .map_or(0, |at| at + 1),
    );
    vec![
        ("bare", program.to_owned()),
        (
            "message",
            format!("]]>openui:content?name=Dashboard\n{program}\n]]>openui:end\n"),
        ),
        (
            "CRLF",
            format!(
                "]]>openui:content\r\n{}\r\n]]>openui:end\r\n",
                program.replace('\n', "\r\n")
            ),
        ),
        (
            "bundle",
            format!(
                "]]>openui:content\n{program}\n]]>openui:scripts {{\"count\":1}}\n[\n{{\"name\":\"refresh\",\"code\":\"{script}\"}}\n]\n]]>openui:end\n"
            ),
        ),
        // The program goes on after the scripts section and a `]]>` that is
        // no marker: `extract_scripts` keeps that part after the program.
        (
            "after",
            format!(
                "]]>openui:content\n{head}\n]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"a\",\"code\":\"1\"}}]\n]]> not a marker\n{tail}]]>openui:end\n"
            ),
        ),
        // A Cloud script is one JSON string: as much script code as the
        // program has bytes, on one line (#122).
        ("script line", one_script_line(&script_code(program.len()))),
    ]
}

/// Streaming twice the text costs about twice as much in every shape and
/// both string forms: quadratic work in the counted code cannot hide behind
/// a constant bound. The count covers what the reader looks at and what its
/// parser takes in and looks at again; it leaves out the core form's
/// comparison of the whole text with the last one (the Swift driver hands
/// over and copies the whole text anyway), `bundle_json` writing the program
/// on every call and the Swift side reading it back, and the JavaScript
/// package's `startsWith`.
#[test]
fn streaming_twice_the_text_costs_about_twice_as_much() {
    for utf16 in [false, true] {
        let mut previous: Option<Vec<u64>> = None;
        for kib in [32, 64, 128, 256] {
            let program = program_of_size(kib * 1024);
            let costs: Vec<u64> = shapes(&program)
                .iter()
                .map(|(_, text)| stream_costs(text, &program, utf16).0)
                .collect();
            if let Some(previous) = &previous {
                for ((name, _), (now, before)) in
                    shapes(&program).iter().zip(costs.iter().zip(previous))
                {
                    let ratio = *now as f64 / *before as f64;
                    println!(
                        "{name} ({}), {kib} KiB: {ratio:.2}× the half",
                        if utf16 { "WebAssembly" } else { "core" }
                    );
                    assert!(
                        ratio < 2.4,
                        "{name} at {kib} KiB costs {ratio:.2}× the text half its size"
                    );
                }
            }
            previous = Some(costs);
        }
    }
}

/// What streaming `text` in 16-byte chunks costs a reader, in bytes looked
/// at by the reader and taken in by its parser, and what pushing its program
/// costs a stream parser.
fn stream_costs(text: &str, program: &str, utf16: bool) -> (u64, u64) {
    let library = library();
    let mut reader = if utf16 {
        ResponseReader::new_utf16(StreamParser::new(library.clone(), None), Limits::DEFAULT)
    } else {
        ResponseReader::new(StreamParser::new(library.clone(), None), Limits::DEFAULT)
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut sent = 0;
    let mut at = 0;
    while at < text.len() {
        at = char_cut(text, at + 16);
        if utf16 {
            let end = text[..at].encode_utf16().count();
            if sent == 0 {
                reader.read_utf16(&units[..end], true).expect("read");
            } else {
                reader
                    .read_utf16_append(sent, &units[sent..end], true)
                    .expect("read");
            }
            sent = end;
        } else {
            reader.read(&text[..at], true).expect("read");
        }
        reader
            .try_parse(false, false, |result| {
                Ok::<_, ()>(result.map(|r| r.root.is_some()))
            })
            .expect("accepted");
    }
    let mut push = StreamParser::new(library, None);
    let mut at = 0;
    while at < program.len() {
        let next = char_cut(program, at + 16);
        push.push(&program[at..next]);
        at = next;
    }
    (
        reader.bytes_examined() + reader.parser_bytes_examined(),
        push.bytes_examined(),
    )
}

#[test]
fn streaming_64_kib_in_16_byte_chunks_costs_within_three_times_pushing_it() {
    let program = program_64k();
    let message = format!("]]>openui:content?name=Dashboard\n{program}\n]]>openui:end\n");
    // A Cloud script is one JSON string, so its line is as long as its code.
    let bundle = format!(
        "]]>openui:content\n{program}\n]]>openui:scripts {{\"count\":2}}\n[\n{{\"name\":\"refresh\",\"code\":\"{}\"}},\n{{\"name\":\"total\",\"code\":\"return rows.length // ]]> in a string\"}}\n]\n]]>openui:end\n",
        "const rows = await query();\\n".repeat(16)
    );
    for (name, text) in [
        ("bare", &program),
        ("message", &message),
        ("bundle", &bundle),
    ] {
        for utf16 in [false, true] {
            let (reader, push) = stream_costs(text, &program, utf16);
            let ratio = reader as f64 / push as f64;
            println!(
                "{name} ({}): {reader} against {push}: {ratio:.2}×",
                if utf16 { "WebAssembly" } else { "core" }
            );
            assert!(ratio < 3.0, "{name}: {ratio:.2}× the push path");
        }
    }
}

/// 64 KiB of script code on one line, in 16-byte chunks, costs within a small
/// constant of the same code arriving as many lines (one short script per
/// line): the partial line's scans resume instead of starting again (#122).
#[test]
fn a_long_script_line_costs_about_what_the_same_code_on_many_lines_costs() {
    let code = script_code(65_536);
    let one_line = one_script_line(&code);
    // The same code cut between its pieces into scripts of about 64 bytes,
    // one per line.
    let piece = script_code(1);
    let pieces: Vec<&str> = code
        .as_bytes()
        .chunks(piece.len())
        .map(|chunk| std::str::from_utf8(chunk).expect("whole pieces"))
        .collect();
    let scripts: Vec<String> = pieces
        .iter()
        .enumerate()
        .map(|(index, code)| format!("{{\"name\":\"s{index}\",\"code\":\"{code}\"}}"))
        .collect();
    let many_lines = format!(
        "]]>openui:content?name=Long\nroot = TextContent(\"x\")\n]]>openui:scripts {{\"count\":{}}}\n[\n{}\n]\n]]>openui:end\n",
        scripts.len(),
        scripts.join(",\n")
    );
    let program = "root = TextContent(\"x\")";
    for utf16 in [false, true] {
        let (long, _) = stream_costs(&one_line, program, utf16);
        let (many, _) = stream_costs(&many_lines, program, utf16);
        let ratio = long as f64 / many as f64;
        println!(
            "64 KiB of script ({}): one line {long}, many lines {many}: {ratio:.2}×",
            if utf16 { "WebAssembly" } else { "core" }
        );
        assert!(ratio < 2.0, "one line costs {ratio:.2}× many lines");
    }
}

/// A refused update that took back the program's trailing line break (a
/// last line that may be a marker) must put it back: the next append and
/// then an edit read as a fresh parse does. Found by an independent review.
#[test]
fn a_refused_update_that_took_back_a_line_break_puts_it_back() {
    let library = library();
    let mut reader = ResponseReader::new(StreamParser::new(library.clone(), None), Limits::DEFAULT);
    let mut step = |text: &str, refuse: bool| {
        reader.read(text, true).expect("read");
        reader
            .try_parse(true, false, |parsed| {
                let json = parsed.expect("a parse").to_json();
                if refuse { Err(json) } else { Ok(json) }
            })
            .unwrap_or_else(|json| json)
    };
    let head = "root = TextContent(b)\nb = \"two\"\n";
    step(head, false);
    step(&format!("{head}]"), true);
    step(&format!("{head}c = \"x\"\n"), false);
    let text = format!("{head}c = \"y\"\n");
    assert_eq!(step(&text, false), fresh_parse(&library, &text, false));
}

/// Random sequences in which about a quarter of the updates are refused,
/// and a refusal is often followed by a different append to the accepted
/// text, an edit that is not an append, or a shrink, rather than by the
/// refused text again: the reader must be at the accepted text, not the
/// refused one. Both drivers' forms, every step against a full read.
#[test]
fn random_sequences_of_refusals_then_other_texts_read_as_a_full_read() {
    let library = library();
    let mut rng = Rng(0x0b1b_0116);
    let mut steps = 0;
    for _ in 0..600 {
        let target = random_text(&mut rng);
        let mut core = CoreDriver::new(&library);
        let mut web = Utf16Driver::new(&library);
        let mut accepted = String::new();
        let mut after_refusal = false;
        for _ in 0..40 {
            let text = if after_refusal && rng.below(4) != 0 {
                match rng.below(3) {
                    // A different append to the accepted text.
                    0 => format!("{accepted}{}", PIECES[rng.below(PIECES.len())]),
                    // An edit: one character of the accepted text replaced.
                    1 => {
                        let mut edited: Vec<char> = accepted.chars().collect();
                        if !edited.is_empty() {
                            let at = rng.below(edited.len());
                            edited[at] = ['x', '\n', ']', '\r', 'é'][rng.below(5)];
                        }
                        edited.into_iter().collect()
                    }
                    // A shrink.
                    _ => {
                        let at = char_cut(&accepted, rng.below(accepted.len() + 1));
                        accepted[..at].to_owned()
                    }
                }
            } else if accepted.len() < target.len() && target.starts_with(&accepted) {
                let at = char_cut(&target, accepted.len() + 1 + rng.below(12));
                target[..at].to_owned()
            } else {
                format!("{accepted}{}", PIECES[rng.below(PIECES.len())])
            };
            let refuse = rng.below(4) == 0;
            let streaming = rng.below(8) != 0;
            let units: Vec<u16> = text.encode_utf16().collect();
            core.step(&text, streaming, refuse, "sequence");
            web.step(&units, streaming, refuse, "sequence");
            steps += 2;
            if !refuse {
                accepted = text;
            }
            after_refusal = refuse;
        }
    }
    assert!(steps >= 48_000, "{steps} steps");
    println!(
        "{} steps checked against a full read so far",
        STEPS.load(std::sync::atomic::Ordering::Relaxed)
    );
}

// ─── A long line in the scripts section (#122) ──────────────────────────────

/// Pieces of script code inside a JSON string: mostly plain code, with every
/// escape (`\"`, `\\`, `\n`, `\uXXXX`, an escaped surrogate pair), `]` and
/// `]]` (an index), comments and quotes the scripts scan follows, and
/// characters of every UTF-8 length.
const CODE_PIECES: &[&str] = &[
    "const total = rows.length;",
    "let x = 1;",
    "abcdefgh",
    "\\\"",
    "\\\\",
    "\\n",
    "\\r\\n",
    "\\u00e9",
    "\\uD83D\\uDE00",
    "\\ud800",
    "rows[ids[0]]",
    "]]",
    "]",
    "//",
    "/",
    "#",
    "'",
    "\\'",
    " ",
    "é",
    "€",
    "😀",
];

/// What else a long line may hold in some texts: raw CRs, `]]>` and markers
/// inside the string, a retry starting mid-line, and (core strings only) the
/// core's lone surrogates, adjacent halves that pair when they meet.
#[derive(Clone, Copy, PartialEq)]
enum Extra {
    None,
    RawCr,
    Markers,
    LoneSurrogates,
}

/// About `bytes` of script code on one line, with a rare piece from `extra`
/// one time in thirty.
fn long_code(rng: &mut Rng, bytes: usize, extra: Extra) -> String {
    let mut code = String::with_capacity(bytes + 64);
    while code.len() < bytes {
        let rare = rng.below(30) == 0;
        let piece: &str = match extra {
            Extra::RawCr if rare => ["\r", "\r\r", "x\r"][rng.below(3)],
            Extra::Markers if rare => [
                "]]>",
                "]]",
                "a]]>b",
                "]]>openui:end",
                "]]>openui:scripts",
                "]]>openui:",
            ][rng.below(6)],
            Extra::LoneSurrogates if rare => [
                "\u{10f83d}\u{10fe00}",
                "\u{10f83d}",
                "\u{10fe00}",
                "\u{10fbff}\u{10ffff}",
                "\u{10f800}\u{10f800}\u{10fc00}",
            ][rng.below(5)],
            _ => CODE_PIECES[rng.below(CODE_PIECES.len())],
        };
        code.push_str(piece);
    }
    code
}

/// Responses with a long line in or near a scripts section: one that stays
/// quiet to its end, ones that later turn out not to be quiet, and ones that
/// are never resumable (in a context section, after the section, nested past
/// the depth limit, a marker line too deep to read).
fn long_line_texts(rng: &mut Rng, bytes: usize) -> Vec<(&'static str, String, Limits)> {
    let head = "]]>openui:content?name=Long\nroot = Stack([t])\nt = TextContent(\"hi\")\n";
    let defaults = Limits::DEFAULT;
    let mut texts = vec![
        (
            "quiet to its end",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"big\",\"code\":\"{}\"}}]\n]]>openui:end\n",
                long_code(rng, bytes, Extra::None)
            ),
            defaults,
        ),
        (
            "array opened on its own line, CRLF",
            format!(
                "{}]]>openui:scripts {{\"count\":2}}\r\n[\r\n{{\"name\":\"big\",\"code\":\"{}\"}},\r\n{{\"name\":\"small\",\"code\":\"1\"}}\r\n]\r\n]]>openui:end\r\n",
                head.replace('\n', "\r\n"),
                long_code(rng, bytes, Extra::RawCr)
            ),
            defaults,
        ),
        (
            "markers inside the string, then a retry mid-line",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"big\",\"code\":\"{}]]>openui:content?name=retry\nroot = TextContent(\\\"{}\\\")\n",
                long_code(rng, bytes, Extra::Markers),
                long_code(rng, bytes / 8, Extra::None)
            ),
            defaults,
        ),
        (
            "quiet until a retry starts mid-line",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"big\",\"code\":\"{}]]>openui:content?name=retry\nroot = TextContent(\\\"x\\\")\n",
                long_code(rng, bytes, Extra::None)
            ),
            defaults,
        ),
        (
            "the array closes mid-line, then junk",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"big\",\"code\":\"{}\"}}]   \t {}\n]]>openui:end\n",
                long_code(rng, bytes, Extra::None),
                long_code(rng, bytes / 8, Extra::None)
            ),
            defaults,
        ),
        (
            "core lone surrogates",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"big\",\"code\":\"{}\"}}]\n",
                long_code(rng, bytes, Extra::LoneSurrogates)
            ),
            defaults,
        ),
        (
            "nested past the depth limit",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n{}{{\"code\":\"{}\"}}\n",
                "[".repeat(300),
                long_code(rng, bytes / 4, Extra::None)
            ),
            defaults,
        ),
        (
            "a marker line too deep to read",
            format!(
                "{head}]]>openui:scripts {{\"count\":{}}}\n[{{\"name\":\"big\",\"code\":\"{}\"}}]\n",
                "[".repeat(1000),
                long_code(rng, bytes / 8, Extra::None)
            ),
            defaults,
        ),
        (
            "in a context section",
            format!(
                "{head}]]>openui:context\n{{\"k\":\"{}\"}}\n]]>openui:end\n",
                long_code(rng, bytes / 4, Extra::None)
            ),
            defaults,
        ),
        (
            "after the section",
            format!(
                "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"a\",\"code\":\"1\"}}]\n]]> no marker\nx = \"{}\"\n",
                long_code(rng, bytes / 4, Extra::None)
            ),
            defaults,
        ),
    ];
    // The program-size limit falls inside the long line.
    let text = format!(
        "{head}]]>openui:scripts {{\"count\":1}}\n[{{\"name\":\"big\",\"code\":\"{}\"}}]\n",
        long_code(rng, bytes + bytes / 4, Extra::None)
    );
    texts.push((
        "over the size limit",
        text,
        defaults.with_program_size(bytes),
    ));
    texts
}

/// Appends that end or break a long line, or carry on with something else.
const DIVERGING: &[&str] = &[
    "\n",
    "\r",
    "\r\n",
    "]",
    "]]",
    "]]>",
    "]]>openui:content\n",
    "\"",
    "\"}]",
    "\"}]\n]]>openui:end\n",
    "\\",
    "\\u",
    "\\uD8",
    "x",
    "\n]]>openui:end\n",
    "é",
    "😀",
    "\u{10f83d}",
    "\u{10fe00}",
    "/",
];

/// Cuts through `len` positions: big and small steps at random, cuts just
/// after about 300 of the features (an escape, a non-ASCII character or
/// surrogate, a bracket, a quote, a CR), and a cut at every position around
/// each turning point (a line break, a `]]>`), where a line stops being
/// resumable.
fn long_line_cuts(len: usize, features: &[usize], turns: &[usize], rng: &mut Rng) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut at = 0;
    while at < len {
        at += match rng.below(3) {
            0 => 1 + rng.below(8),
            1 => 8 + rng.below(56),
            _ => 64 + rng.below(4096),
        };
        cuts.push(at.min(len));
    }
    let one_in = (features.len() / 300).max(1);
    for &feature in features {
        if rng.below(one_in) == 0 {
            for offset in 1..=3 {
                cuts.push((feature + offset).min(len));
            }
        }
    }
    for &turn in turns {
        for at in turn.saturating_sub(4)..turn + 32 {
            cuts.push(at.min(len));
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}

/// Where a text turns: its line breaks and the starts of its `]]>`s.
fn turns<T: Copy + PartialEq>(items: &[T], newline: T, bracket: T, close: T) -> Vec<usize> {
    (0..items.len())
        .filter(|&at| {
            items[at] == newline || items.get(at..at + 3) == Some(&[bracket, bracket, close][..])
        })
        .collect()
}

/// One sequence against one target: walk its cuts, refuse about a quarter
/// of the updates, and after a refusal usually send something else: a
/// different append, an edit that is not an append, or a shrink (often into
/// the middle of the long line). A text that has left the target goes back
/// to the target's longest cut it still covers, a shrink too. `units` turns
/// a text into what the driver takes; `cut_of` moves a cut to a position the
/// driver can take.
struct Walk<'a> {
    rng: &'a mut Rng,
    cuts: Vec<usize>,
}

impl Walk<'_> {
    fn core(&mut self, target: &str, driver: &mut CoreDriver, what: &str) -> usize {
        let mut accepted = String::new();
        let mut after_refusal = false;
        let mut next = 0;
        let mut steps = 0;
        while steps < 3 * self.cuts.len() + 8 {
            let rng = &mut *self.rng;
            let text = if after_refusal && rng.below(4) != 0 {
                match rng.below(3) {
                    0 => format!("{accepted}{}", DIVERGING[rng.below(DIVERGING.len())]),
                    1 => {
                        let mut edited: Vec<char> = accepted.chars().collect();
                        if !edited.is_empty() {
                            let at = rng.below(edited.len());
                            edited[at] = ['x', '\n', ']', '\r', 'é', '"'][rng.below(6)];
                        }
                        edited.into_iter().collect()
                    }
                    _ => accepted[..char_cut(&accepted, rng.below(accepted.len() + 1))].to_owned(),
                }
            } else if target.starts_with(&accepted) {
                // Forward through the cuts, past refused ones too.
                while next < self.cuts.len() && self.cuts[next] <= accepted.len() {
                    next += 1;
                }
                let Some(&cut) = self.cuts.get(next) else {
                    break;
                };
                next += 1;
                target[..char_cut(target, cut)].to_owned()
            } else {
                let back = self
                    .cuts
                    .iter()
                    .rev()
                    .find(|&&cut| cut < accepted.len() && target.is_char_boundary(cut))
                    .copied()
                    .unwrap_or(0);
                target[..back].to_owned()
            };
            let refuse = rng.below(4) == 0;
            // A settled read sees a retry still arriving mid-line.
            let streaming = rng.below(6) != 0;
            if driver.step(&text, streaming, refuse, what) {
                accepted = text;
                after_refusal = false;
            } else {
                after_refusal = true;
            }
            steps += 1;
        }
        driver.step(target, false, false, what);
        steps + 1
    }

    fn utf16(&mut self, target: &[u16], driver: &mut Utf16Driver, what: &str) -> usize {
        let mut accepted: Vec<u16> = Vec::new();
        let mut after_refusal = false;
        let mut next = 0;
        let mut steps = 0;
        while steps < 3 * self.cuts.len() + 8 {
            let rng = &mut *self.rng;
            let units: Vec<u16> = if after_refusal && rng.below(4) != 0 {
                match rng.below(4) {
                    0 => accepted
                        .iter()
                        .copied()
                        // The core's lone surrogates as lone units.
                        .chain(openui_lang::utf16_units(
                            DIVERGING[rng.below(DIVERGING.len())],
                        ))
                        .collect(),
                    // A lone half: the low one pairs with a high half the
                    // accepted text ends with.
                    1 => accepted
                        .iter()
                        .copied()
                        .chain([[0xdc00, 0xde00, 0xd83d][rng.below(3)]])
                        .collect(),
                    2 => {
                        let mut edited = accepted.clone();
                        if !edited.is_empty() {
                            let at = rng.below(edited.len());
                            edited[at] = [0x78, 0x0a, 0x5d, 0x0d, 0xd800, 0xdc00][rng.below(6)];
                        }
                        edited
                    }
                    _ => accepted[..rng.below(accepted.len() + 1)].to_vec(),
                }
            } else if target.starts_with(&accepted) {
                // Forward through the cuts, past refused ones too.
                while next < self.cuts.len() && self.cuts[next] <= accepted.len() {
                    next += 1;
                }
                let Some(&cut) = self.cuts.get(next) else {
                    break;
                };
                next += 1;
                target[..cut].to_vec()
            } else {
                let back = self
                    .cuts
                    .iter()
                    .rev()
                    .find(|&&cut| cut < accepted.len())
                    .copied()
                    .unwrap_or(0);
                target[..back].to_vec()
            };
            let refuse = rng.below(4) == 0;
            // A settled read sees a retry still arriving mid-line.
            let streaming = rng.below(6) != 0;
            if driver.step(&units, streaming, refuse, what) {
                accepted = units;
                after_refusal = false;
            } else {
                after_refusal = true;
            }
            steps += 1;
        }
        driver.step(target, false, false, what);
        steps + 1
    }
}

/// Where a text's features are: every byte (or unit) that is not plain
/// ASCII code (a backslash, CR, LF, quote, `]`, `/` or anything non-ASCII),
/// so cuts land inside escapes, `\uXXXX` sequences, multi-byte characters,
/// surrogate pairs, around CRs and brackets.
fn features<T: Copy>(items: &[T], plain: impl Fn(T) -> bool) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|&(_, &item)| !plain(item))
        .map(|(at, _)| at)
        .collect()
}

/// Script code of at least 64 KiB on one line, streamed in both forms with
/// chunk edges inside escapes, `\uXXXX` sequences, surrogate pairs (UTF-16
/// form) and multi-byte characters (string form), around CR and CRLF, with
/// `]]` and `]]>` inside the string, lines that later turn out not to be
/// quiet, and refusals followed by other appends, edits and shrinks: every
/// step reads and parses as a full read does (#122).
#[test]
fn a_long_script_line_reads_as_a_full_read_however_it_arrives() {
    let library = library();
    let mut rng = Rng(0x0122_2026);
    let mut steps = 0;
    for (what, text, limits) in long_line_texts(&mut rng, 65_536) {
        let plain = |b: u8| b.is_ascii() && !b"\\\r\n\"]/".contains(&b);
        let bytes = text.as_bytes();
        let cuts = long_line_cuts(
            text.len(),
            &features(bytes, plain),
            &turns(bytes, b'\n', b']', b'>'),
            &mut rng,
        );
        let mut driver = CoreDriver::with_limits(&library, limits);
        steps += Walk {
            rng: &mut rng,
            cuts,
        }
        .core(&text, &mut driver, what);
        // The UTF-16 form: the core's lone surrogates become lone units.
        let units: Vec<u16> = openui_lang::utf16_units(&text).collect();
        let plain = |u: u16| u < 0x80 && plain(u as u8);
        let cuts = long_line_cuts(
            units.len(),
            &features(&units, plain),
            &turns(&units, 0x0a, 0x5d, 0x3e),
            &mut rng,
        );
        let mut driver = Utf16Driver::with_limits(&library, limits);
        steps += Walk {
            rng: &mut rng,
            cuts,
        }
        .utf16(&units, &mut driver, what);
    }
    println!("{steps} steps on long script lines");
    assert!(steps > 5_000, "{steps} steps");
}

/// A refused update leaves the resumed scan of a long script line where the
/// last accepted read left it: after refused appends (a longer one, one that
/// ends the line, one that pairs a trailing lone surrogate), the next
/// accepted append reads as a reader that never saw them reads it, at the
/// same cost, and that cost is the new bytes only, not the line again. A
/// read that is staged and then discarded leaves it there too.
#[test]
fn a_refused_update_leaves_the_resumed_scan_where_it_was() {
    let library = library();
    let code = script_code(65_536);
    let text = one_script_line(&code);
    let middle = char_cut(&text, text.len() / 2);
    for utf16 in [false, true] {
        let make = || {
            let parser = StreamParser::new(library.clone(), None);
            if utf16 {
                ResponseReader::new_utf16(parser, Limits::DEFAULT)
            } else {
                ResponseReader::new(parser, Limits::DEFAULT)
            }
        };
        let (mut control, mut staged) = (make(), make());
        // Both readers: the text up to the middle of the long line, in
        // 16-byte chunks, and then a lone high surrogate.
        let head = format!("{}\u{10f83d}", &text[..middle]);
        let feed = |reader: &mut ResponseReader, from: &str, to: &str, accept: bool| {
            let read = if utf16 {
                let old: Vec<u16> = openui_lang::utf16_units(from).collect();
                let new: Vec<u16> = openui_lang::utf16_units(to).collect();
                reader.read_utf16_append(old.len(), &new[old.len()..], true)
            } else {
                reader.read(to, true)
            };
            read.expect("read");
            let draw = (reader.bundle_json(), reader.draw_json());
            let kept = reader.try_parse(true, false, |parsed| {
                let parsed = parsed.map(|parsed| parsed.to_json());
                if accept { Ok(parsed) } else { Err(parsed) }
            });
            (draw, kept.is_ok())
        };
        for reader in [&mut control, &mut staged] {
            let mut at = 0;
            let mut sent = String::new();
            while at < head.len() {
                at = char_cut(&head, at + 16);
                let (_, kept) = feed(reader, &sent, &head[..at], true);
                assert!(kept);
                sent = head[..at].to_owned();
            }
        }
        // Refused: more of the line, the line's end, and the low half.
        for refused in [
            format!("{head}{}", &code[..4096]),
            format!("{head}\"}}]\n]]>openui:end\n"),
            format!("{head}\u{10fe00}more code"),
        ] {
            let (_, kept) = feed(&mut staged, &head, &refused, false);
            assert!(!kept);
        }
        // A read staged and then discarded, as the native binding does when
        // the message's context is refused after the read: the same.
        let discarded = format!("{head}\u{10fe00}{}", &code[..2048]);
        if utf16 {
            let old = openui_lang::utf16_units(&head).count();
            let new: Vec<u16> = openui_lang::utf16_units(&discarded).collect();
            staged
                .read_utf16_append(old, &new[old..], true)
                .expect("read");
        } else {
            staged.read(&discarded, true).expect("read");
        }
        staged.discard();
        let next = format!("{head}\u{10fe00}abcdef");
        let before = (control.bytes_examined(), staged.bytes_examined());
        let read = (
            feed(&mut control, &head, &next, true),
            feed(&mut staged, &head, &next, true),
        );
        assert_eq!(read.0, read.1);
        let cost = (
            control.bytes_examined() - before.0,
            staged.bytes_examined() - before.1,
        );
        assert_eq!(cost.0, cost.1, "the refused reads moved the scan");
        assert!(cost.1 < 1_000, "the line was read again: {} bytes", cost.1);
    }
}

/// A scripts marker line whose JSON is nested past the depth limit is not
/// read while the line after it is a held-back `]` or `]]` (the full read
/// has no line break after the marker line yet). The next append makes the
/// full read refuse it, and the resumed read must refuse it too: the scan
/// is not built for that line (found by an independent review).
#[test]
fn a_marker_line_too_deep_is_refused_after_a_held_back_bracket() {
    let library = library();
    let limits = Limits::DEFAULT.with_depth(1);
    let head = format!(
        "]]>openui:content\nroot = TextContent(\"x\")\n]]>openui:scripts {}\n",
        "[".repeat(36)
    );
    for held in ["]", "]]"] {
        let first = format!("{head}{held}");
        let next = format!("{first}abc");
        assert_eq!(
            expected_core_with(&next, true, limits).read,
            Err(ResponseReaderError::Bundle(
                ResponseBundleError::ScriptsDepth
            ))
        );
        let mut core = CoreDriver::with_limits(&library, limits);
        assert!(core.step(&first, true, false, "held back"));
        assert!(!core.step(&next, true, false, "too deep"));
        let mut web = Utf16Driver::with_limits(&library, limits);
        let units: Vec<u16> = first.encode_utf16().collect();
        assert!(web.step(&units, true, false, "held back"));
        let units: Vec<u16> = next.encode_utf16().collect();
        assert!(!web.step(&units, true, false, "too deep"));
    }
}
