//! CLI for the Rust OpenUI Lang parser.
//!
//!   openui-lang parse  <schema.json> <input>            one-shot parse, prints JSON
//!   openui-lang stream <schema.json> <input> [chunk]    prints one JSON line per push
//!   openui-lang bench  <schema.json> <file>...          streaming timing (see bench/stream.ts)
//!
//! `<input>` may be `-` for stdin.

use std::io::Read;
use std::process::ExitCode;

use openui_lang::{parse, Library, StreamParser};

fn read_input(path: &str) -> String {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("read stdin");
        s
    } else {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }
}

fn load_lib(path: &str) -> Library {
    let schema: serde_json::Value =
        serde_json::from_str(&read_input(path)).expect("schema is valid JSON");
    Library::from_json_schema(&schema)
}

/// Split into chunks of `size` chars (not bytes), like slicing a JS string.
pub fn chunks(text: &str, size: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (i, _) in text.char_indices() {
        if count == size {
            out.push(&text[start..i]);
            start = i;
            count = 0;
        }
        count += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("parse") if args.len() >= 3 => {
            let lib = load_lib(&args[1]);
            println!("{}", parse(&read_input(&args[2]), &lib, None).to_json());
        }
        Some("stream") if args.len() >= 3 => {
            let lib = load_lib(&args[1]);
            let text = read_input(&args[2]);
            let size = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);
            let mut p = StreamParser::new(&lib, None);
            for c in chunks(&text, size) {
                println!("{}", p.push(c).to_json());
            }
        }
        Some("bench") if args.len() >= 3 => {
            // Same workload as bench/stream.ts: 4-char chunks, 5 warmups, mean of 20 runs.
            let lib = load_lib(&args[1]);
            for path in &args[2..] {
                let text = read_input(path);
                let parts = chunks(&text, 4);
                let once = || {
                    let mut p = StreamParser::new(&lib, None);
                    for c in &parts {
                        std::hint::black_box(p.push(c));
                    }
                };
                for _ in 0..5 {
                    once();
                }
                let runs = 20;
                let t0 = std::time::Instant::now();
                for _ in 0..runs {
                    once();
                }
                let ms = t0.elapsed().as_secs_f64() * 1000.0 / runs as f64;
                let name = path.rsplit('/').next().unwrap_or(path);
                println!("{name}\t{}\t{}\t{ms:.2}", text.len(), parts.len());
            }
        }
        _ => {
            eprintln!("usage: openui-lang parse|stream <schema.json> <input|-> [chunk]");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
