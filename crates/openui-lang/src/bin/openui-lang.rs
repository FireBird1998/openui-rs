//! Command-line tool for the OpenUI Lang parser.
//!
//!   openui-lang parse <schema.json> <program|->    one-shot parse, prints the result JSON
//!   openui-lang stream <schema.json> <program|-> [chunk]
//!                                                   streaming parse in chunks of `chunk` chars
//!                                                   (default 4), one result JSON per line,
//!                                                   then the result at the stream's end
//!   openui-lang tokens <program|->                  prints the tokens as JSON
//!   openui-lang prompt <spec.json|->                 prints the system prompt for a prompt
//!                                                   spec ({library, promptOptions}, a prompt
//!                                                   spec, or {cloud: true, ...})
//!   openui-lang bench <schema.json> <program>...     streaming timing: chunks of 4 code
//!                                                   points, 5 warm-ups, mean of BENCH_RUNS
//!                                                   (default 20) runs; same workload as
//!                                                   reference/bench.ts
//!
//! `--limit name=value`, anywhere and as often as needed, sets one of the
//! core's limits for every command: `program-size` and `result-size` in
//! bytes, `depth` in levels, `work` in steps.
//! `prompt` reads the spec with serde_json, which refuses nesting past 128
//! levels regardless of `--limit depth`. `tokens` reads
//! to the program-size limit, and `prompt` refuses a spec nesting past the
//! depth limit. Fuzz finds and blow-ups replay under any limit this way.

use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use openui_lang::{
    Library, Limits, StreamParser, generate_system_prompt_with_limits, parse_with_limits, tokenize,
    tokens_to_json,
};

const USAGE: &str = "usage: openui-lang parse <schema.json> <program|->
       openui-lang stream <schema.json> <program|-> [chunk]
       openui-lang tokens <program|->
       openui-lang prompt <spec.json|->
       openui-lang bench <schema.json> <program>...
options: --limit name=value";

/// The limits `--limit name=value` can set.
const LIMIT_NAMES: &str =
    "program-size, result-size, result-floor, amplification, result-ceiling, depth, work";

/// The arguments with every `--limit name=value` taken out, and the limits
/// they set.
fn take_limits(args: &[String]) -> Result<(Vec<String>, Limits), String> {
    let mut rest = Vec::new();
    let mut limits = Limits::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg != "--limit" {
            rest.push(arg.clone());
            continue;
        }
        let setting = args.next().map_or("", String::as_str);
        let bad =
            || format!("--limit takes name=value, a whole number; the limits are {LIMIT_NAMES}");
        let (name, value) = setting.split_once('=').ok_or_else(bad)?;
        let value: usize = value.parse().map_err(|_| bad())?;
        limits = match name {
            "program-size" => limits.with_program_size(value),
            "result-size" => limits.with_result_size(value),
            "result-floor" => limits.with_result_floor(value),
            "amplification" => limits.with_amplification(value),
            "result-ceiling" => limits.with_result_ceiling(value),
            "depth" => limits.with_depth(value),
            "work" => limits.with_work(value),
            _ => {
                return Err(format!(
                    "unknown limit `{name}`; the limits are {LIMIT_NAMES}"
                ));
            }
        };
    }
    Ok((rest, limits))
}

/// The part of `text` within `bytes`, never splitting a character: what the
/// program-size limit lets parsing read.
// `Limits::fit` is crate-private, so the CLI keeps this character-safe copy.
fn within(text: &str, bytes: usize) -> &str {
    let end = (0..=bytes.min(text.len()))
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    &text[..end]
}

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
    let (args, limits) = take_limits(args)?;
    match args.as_slice() {
        [command, schema, program] if command == "parse" => {
            let library = load_library(schema)?;
            let result = parse_with_limits(&read_input(program)?, &library, None, limits);
            println!("{}", result.to_json());
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
            let mut parser = StreamParser::with_limits(library, None, limits);
            for chunk in chunks(&text, size) {
                println!("{}", parser.push(chunk).to_json());
            }
            println!("{}", parser.finish().to_json());
            Ok(())
        }
        [command, schema, programs @ ..] if command == "bench" && !programs.is_empty() => {
            let library = Arc::new(load_library(schema)?);
            let runs: usize = benchmark_runs(std::env::var("BENCH_RUNS").ok().as_deref());
            for path in programs {
                let text = read_input(path)?;
                let pieces = chunks(&text, 4);
                let once = || {
                    let mut parser = StreamParser::with_limits(library.clone(), None, limits);
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
                let ms = mean_milliseconds(started.elapsed(), runs);
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
            let text = read_input(program)?;
            let text = within(&text, limits.program_size());
            println!("{}", tokens_to_json(&tokenize(text)));
            Ok(())
        }
        [command, spec] if command == "prompt" => {
            let spec: serde_json::Value =
                serde_json::from_str(&read_input(spec)?).map_err(|e| format!("{spec}: {e}"))?;
            let prompt =
                generate_system_prompt_with_limits(&spec, limits).map_err(|e| e.to_string())?;
            println!("{prompt}");
            Ok(())
        }
        _ => Err(format!("{USAGE} ({LIMIT_NAMES})")),
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

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

/// Mean wall-clock duration of one benchmark run, in milliseconds.
fn mean_milliseconds(elapsed: std::time::Duration, runs: usize) -> f64 {
    elapsed.as_secs_f64() * 1000.0 / runs as f64
}

/// The documented benchmark repetition count; absent, invalid or zero uses 20.
fn benchmark_runs(setting: Option<&str>) -> usize {
    setting
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(20)
}

#[cfg(test)]
mod tests {
    #[test]
    fn benchmark_repetitions_accept_positive_counts_and_default_otherwise() {
        for (setting, expected) in [
            (None, 20),
            (Some("0"), 20),
            (Some("1"), 1),
            (Some("3"), 3),
            (Some("-1"), 20),
            (Some("abc"), 20),
        ] {
            assert_eq!(super::benchmark_runs(setting), expected, "{setting:?}");
        }
    }

    #[test]
    fn benchmark_reports_mean_milliseconds() {
        assert_eq!(
            super::mean_milliseconds(std::time::Duration::from_millis(750), 3),
            250.0
        );
    }
}
