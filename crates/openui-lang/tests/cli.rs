//! The `openui-lang` command-line tool. Each command must print what the
//! library itself gives for the same input; whether that matches the
//! reference is the conformance tests' job.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use openui_lang::{Library, StreamParser, generate_system_prompt, parse, tokenize, tokens_to_json};

fn conformance() -> PathBuf {
    common::conformance_dir()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn library(schema: &Path) -> Arc<Library> {
    Arc::new(Library::from_json_schema(
        &serde_json::from_str(&read(schema)).unwrap(),
    ))
}

/// The library's result after each push of `size` code points, then at the
/// stream's end, one JSON line each.
fn streamed(schema: &Path, program: &Path, size: usize) -> Vec<String> {
    let mut parser = StreamParser::new(library(schema), None);
    let chars: Vec<char> = read(program).chars().collect();
    let mut lines: Vec<String> = chars
        .chunks(size)
        .map(|chunk| parser.push(&chunk.iter().collect::<String>()).to_json())
        .collect();
    lines.push(parser.finish().to_json());
    lines
}

#[test]
fn parse_prints_the_parse_result() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("parse")
        .arg(dir.join("schemas/text.json"))
        .arg(dir.join("cases/hello-text.oui"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected = parse(
        &read(&dir.join("cases/hello-text.oui")),
        &library(&dir.join("schemas/text.json")),
        None,
    )
    .to_json();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{expected}\n")
    );
}

#[test]
fn tokens_prints_the_tokens() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("tokens")
        .arg(dir.join("cases/hand/67-lexer-number-forms.oui"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected = tokens_to_json(&tokenize(&read(
        &dir.join("cases/hand/67-lexer-number-forms.oui"),
    )));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{expected}\n")
    );
}

#[test]
fn stream_prints_the_result_after_every_push() {
    let dir = conformance();
    let case = "hand/71-expression-forms";
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("stream")
        .arg(dir.join("schemas/hand.json"))
        .arg(dir.join(format!("cases/{case}.oui")))
        .arg("7")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected = streamed(
        &dir.join("schemas/hand.json"),
        &dir.join(format!("cases/{case}.oui")),
        7,
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().collect::<Vec<_>>(), expected);
}

/// Without a chunk size, `stream` pushes 4 characters at a time, like the
/// corpus's "4" chunking.
#[test]
fn stream_pushes_four_characters_at_a_time_by_default() {
    let dir = conformance();
    let case = "hand/71-expression-forms";
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("stream")
        .arg(dir.join("schemas/hand.json"))
        .arg(dir.join(format!("cases/{case}.oui")))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected = streamed(
        &dir.join("schemas/hand.json"),
        &dir.join(format!("cases/{case}.oui")),
        4,
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().collect::<Vec<_>>(), expected);
}

/// `-` reads the program from standard input.
#[test]
fn parse_reads_the_program_from_stdin_given_a_dash() {
    use std::io::Write;
    let dir = conformance();
    let program = std::fs::read_to_string(dir.join("cases/hello-text.oui")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("parse")
        .arg(dir.join("schemas/text.json"))
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let expected = parse(&program, &library(&dir.join("schemas/text.json")), None).to_json();
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim_end(), expected);
}

#[test]
fn unknown_command_prints_usage_and_fails() {
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("frobnicate")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: openui-lang parse"));
}

#[test]
fn bench_prints_one_timing_line_per_program() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .env("BENCH_RUNS", "1")
        .arg("bench")
        .arg(dir.join("imported/schemas/bench.json"))
        .arg(dir.join("imported/cases/bench/simple-table.oui"))
        .arg(dir.join("imported/cases/bench/contact-form.oui"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<Vec<&str>> = stdout.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0][0], "simple-table.oui");
    assert_eq!(lines[1][0], "contact-form.oui");
    for line in &lines {
        assert_eq!(line.len(), 4, "name, code points, pushes, ms: {line:?}");
        let code_points: usize = line[1].parse().unwrap();
        let pushes: usize = line[2].parse().unwrap();
        assert_eq!(pushes, code_points.div_ceil(4));
        line[3].parse::<f64>().unwrap();
    }
}

#[test]
fn prompt_prints_the_system_prompt() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("prompt")
        .arg(dir.join("prompts/library-tools-strings.json"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let spec =
        serde_json::from_str(&read(&dir.join("prompts/library-tools-strings.json"))).unwrap();
    let expected = format!("{}\n", generate_system_prompt(&spec).unwrap());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

#[test]
fn limits_apply_to_parse_and_stream() {
    let dir = conformance();
    let schema = dir.join("schemas/text.json");
    let program = dir.join("cases/hello-text.oui");
    let limits = openui_lang::Limits::default()
        .with_program_size(10)
        .with_depth(3);
    let expected = openui_lang::parse_with_limits(&read(&program), &library(&schema), None, limits);
    assert!(expected.size_limited);
    // `parse` prints the one result; `stream` ends with the result at the end.
    for command in ["parse", "stream"] {
        let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
            .args(["--limit", "program-size=10", command])
            .arg(&schema)
            .arg(&program)
            .args(["--limit", "depth=3"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        let last = stdout.lines().last().unwrap();
        assert_eq!(last, expected.to_json(), "{command}");
    }
}

#[test]
fn the_scaling_result_size_limit_can_be_set() {
    let dir = conformance();
    let schema = dir.join("schemas/text.json");
    let program = dir.join("cases/hello-text.oui");
    let limits = openui_lang::Limits::default()
        .with_amplification(0)
        .with_result_floor(10)
        .with_result_ceiling(1_000_000);
    let expected = openui_lang::parse_with_limits(&read(&program), &library(&schema), None, limits);
    assert!(expected.size_limited);
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .args([
            "parse",
            "--limit",
            "amplification=0",
            "--limit",
            "result-floor=10",
        ])
        .args(["--limit", "result-ceiling=1000000"])
        .arg(&schema)
        .arg(&program)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().last().unwrap(), expected.to_json());
}

#[test]
fn an_unknown_limit_names_the_valid_ones_and_fails() {
    let dir = conformance();
    for bad in ["frobs=3", "depth", "depth=lots"] {
        let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
            .args(["parse", "--limit", bad])
            .arg(dir.join("schemas/text.json"))
            .arg(dir.join("cases/hello-text.oui"))
            .output()
            .unwrap();
        assert!(!out.status.success(), "{bad}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains(
                "program-size, result-size, result-floor, amplification, result-ceiling, depth, work"
            ),
            "{bad}: {stderr}"
        );
    }
}

#[test]
fn the_work_limit_applies_to_parse() {
    let dir = conformance();
    let schema = dir.join("schemas/hand.json");
    let program = dir.join("cases/hand/136-state-defaults-builtin-functions.oui");
    let limits = openui_lang::Limits::default().with_work(5);
    let expected = openui_lang::parse_with_limits(&read(&program), &library(&schema), None, limits);
    assert!(expected.size_limited);
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .args(["parse", "--limit", "work=5"])
        .arg(&schema)
        .arg(&program)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim_end(),
        expected.to_json()
    );
}

#[test]
fn limits_apply_to_tokens_and_prompt() {
    let dir = conformance();
    // `tokens` reads the program up to the program-size limit, as parsing does.
    let program = dir.join("cases/hand/67-lexer-number-forms.oui");
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .args(["tokens", "--limit", "program-size=40"])
        .arg(&program)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = read(&program);
    let expected = tokens_to_json(&tokenize(&text[..40]));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{expected}\n")
    );

    // `prompt` refuses a spec nesting past the depth limit's 44 JSON levels
    // at depth 4, and takes it under the default.
    let mut schema = serde_json::json!({"type": "string"});
    for _ in 0..30 {
        schema = serde_json::json!({"type": "object", "properties": {"a": schema}});
    }
    let spec = serde_json::json!({"tools": [{"name": "t", "inputSchema": schema}]}).to_string();
    for (limit, succeeds) in [("depth=4", false), ("depth=256", true)] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
            .args(["prompt", "-", "--limit", limit])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(spec.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.success(), succeeds, "{limit}");
        if !succeeds {
            let stderr = String::from_utf8(out.stderr).unwrap();
            assert!(stderr.contains("nests deeper than 44"), "{stderr}");
        }
    }
}

#[test]
fn prompt_reports_the_error_and_fails() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("prompt")
        .arg(dir.join("prompts/cloud-options-without-library.json"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("promptOptions requires a library"),
        "{stderr}"
    );
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
