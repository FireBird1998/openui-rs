//! The `openui-lang` command-line tool.

use std::path::{Path, PathBuf};
use std::process::Command;

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

#[test]
fn parse_prints_the_reference_parse_result() {
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
    let expected = std::fs::read_to_string(dir.join("golden/hello-text.oneshot.json")).unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{expected}\n")
    );
}

/// FNV-1a (32-bit) as 8 hex digits, as the goldens store it.
fn fingerprint(text: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for &b in text.as_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

#[test]
fn tokens_prints_the_reference_tokens() {
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
    let stdout = String::from_utf8(out.stdout).unwrap();
    let golden =
        std::fs::read_to_string(dir.join("golden/hand/67-lexer-number-forms.tokens.json")).unwrap();
    let prefixes: Vec<String> = serde_json::from_str(&golden).unwrap();
    // The last prefix is the whole program.
    assert_eq!(
        fingerprint(stdout.trim_end_matches('\n')),
        *prefixes.last().unwrap()
    );
}

#[test]
fn stream_prints_the_reference_result_after_every_push() {
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
    let golden = std::fs::read_to_string(dir.join(format!("golden/{case}.steps.json"))).unwrap();
    let golden: serde_json::Value = serde_json::from_str(&golden).unwrap();
    let expected: Vec<&str> = golden["7"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    let actual: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(fingerprint)
        .collect();
    assert_eq!(actual, expected);
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
fn prompt_prints_the_reference_system_prompt() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("prompt")
        .arg(dir.join("prompts/library-tools-strings.json"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let golden: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("golden/prompts/library-tools-strings.json")).unwrap(),
    )
    .unwrap();
    let expected = format!("{}\n", golden["prompt"].as_str().unwrap());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

#[test]
fn prompt_reports_the_reference_error_and_fails() {
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
