//! Command-line tool for the OpenUI Lang parser.
//!
//!   openui-lang parse <schema.json> <program|->    one-shot parse, prints the result JSON
//!   openui-lang stream <schema.json> <program|-> [chunk]
//!                                                   streaming parse in chunks of `chunk` chars
//!                                                   (default 4), one result JSON per line
//!   openui-lang tokens <program|->                  prints the tokens as JSON
//!   openui-lang prompt <spec.json|->                 prints the system prompt for a prompt
//!                                                   spec ({library, promptOptions}, a prompt
//!                                                   spec, or {cloud: true, ...})
//!   openui-lang bench <schema.json> <program>...     streaming timing: chunks of 4 code
//!                                                   points, 5 warm-ups, mean of BENCH_RUNS
//!                                                   (default 20) runs; same workload as
//!                                                   reference/bench.ts

use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use openui_lang::{Library, StreamParser, generate_system_prompt, parse, tokenize, tokens_to_json};

const USAGE: &str = "usage: openui-lang parse <schema.json> <program|->
       openui-lang stream <schema.json> <program|-> [chunk]
       openui-lang tokens <program|->
       openui-lang prompt <spec.json|->
       openui-lang bench <schema.json> <program>...";

/// `text` cut into pieces of `size` code points.
fn chunks(text: &str, size: usize) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    chars
        .chunks(size)
        .map(|group| {
            let start = group[0].0;
            let end = group.last().map_or(start, |&(i, c)| i + c.len_utf8());
            &text[start..end]
        })
        .collect()
}

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("read stdin: {e}"))?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))
    }
}

fn load_library(path: &str) -> Result<Library, String> {
    let schema: serde_json::Value =
        serde_json::from_str(&read_input(path)?).map_err(|e| format!("{path}: {e}"))?;
    Ok(Library::from_json_schema(&schema))
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [command, schema, program] if command == "parse" => {
            let library = load_library(schema)?;
            println!("{}", parse(&read_input(program)?, &library, None).to_json());
            Ok(())
        }
        [command, schema, program, rest @ ..] if command == "stream" && rest.len() <= 1 => {
            let size: usize = match rest.first() {
                Some(n) => n
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or("chunk must be a positive number")?,
                None => 4,
            };
            let library = Arc::new(load_library(schema)?);
            let text = read_input(program)?;
            let mut parser = StreamParser::new(library, None);
            for chunk in chunks(&text, size) {
                println!("{}", parser.push(chunk).to_json());
            }
            Ok(())
        }
        [command, schema, programs @ ..] if command == "bench" && !programs.is_empty() => {
            let library = Arc::new(load_library(schema)?);
            let runs: usize = std::env::var("BENCH_RUNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(20);
            for path in programs {
                let text = read_input(path)?;
                let pieces = chunks(&text, 4);
                let once = || {
                    let mut parser = StreamParser::new(library.clone(), None);
                    for chunk in &pieces {
                        std::hint::black_box(parser.push(chunk));
                    }
                };
                for _ in 0..runs.min(5) {
                    once();
                }
                let started = Instant::now();
                for _ in 0..runs {
                    once();
                }
                let ms = started.elapsed().as_secs_f64() * 1000.0 / runs as f64;
                let name = path.rsplit('/').next().unwrap_or(path);
                println!(
                    "{name}\t{}\t{}\t{ms:.2}",
                    text.chars().count(),
                    pieces.len()
                );
            }
            Ok(())
        }
        [command, program] if command == "tokens" => {
            println!("{}", tokens_to_json(&tokenize(&read_input(program)?)));
            Ok(())
        }
        [command, spec] if command == "prompt" => {
            let spec: serde_json::Value =
                serde_json::from_str(&read_input(spec)?).map_err(|e| format!("{spec}: {e}"))?;
            let prompt = generate_system_prompt(&spec).map_err(|e| e.to_string())?;
            println!("{prompt}");
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
