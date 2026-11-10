//! The `openui-lang` command-line tool. Each command must print what the
//! library itself gives for the same input; whether that matches the
//! reference is the conformance tests' job.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use openui_lang::{Library, StreamParser, generate_system_prompt, parse, tokenize, tokens_to_json};

/// All CLI children are drained concurrently so a full pipe cannot prevent
/// the watchdog from polling them.
trait BoundedOutput {
    fn bounded_output(&mut self) -> std::io::Result<std::process::Output>;
}
impl BoundedOutput for Command {
    fn bounded_output(&mut self) -> std::io::Result<std::process::Output> {
        self.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        bounded_wait(self.spawn()?)
    }
}

fn footprint(pid: u32, deadline: std::time::Instant) -> Option<u64> {
    physical_footprint(pid).or_else(|| rss(pid, deadline))
}

fn physical_footprint(pid: u32) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
        // SAFETY: the buffer has exactly the layout and size for V4; the
        // syscall initializes it on success. Like the clock_gettime calls
        // in the timing tests, this unsafe boundary exists only in tests.
        let status = unsafe {
            libc::proc_pid_rusage(
                pid as libc::pid_t,
                libc::RUSAGE_INFO_V4,
                usage.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            return None;
        }
        // SAFETY: proc_pid_rusage succeeded with a V4 buffer.
        let usage = unsafe { usage.assume_init() };
        // The same physical footprint as `footprint`, including compressed
        // memory. The lifetime peak also catches spikes between polls.
        Some(
            usage
                .ri_lifetime_max_phys_footprint
                .max(usage.ri_phys_footprint),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        Some(
            status
                .lines()
                .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmSwap:"))
                .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                .sum::<u64>()
                * 1024,
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn rss(pid: u32, deadline: std::time::Instant) -> Option<u64> {
    let mut child = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let output = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kb| kb * 1024)
}

fn bounded_wait(child: std::process::Child) -> std::io::Result<std::process::Output> {
    bounded_wait_with(child, 512 * 1024 * 1024, std::time::Duration::from_secs(60))
}

fn bounded_wait_with(
    mut child: std::process::Child,
    memory: u64,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut bytes).unwrap();
            }
            bytes
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = std::time::Instant::now();
    let mut peak = 0;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        match footprint(child.id(), start + timeout) {
            Some(bytes) => peak = peak.max(bytes),
            None => {
                if child.try_wait()?.is_none() {
                    child.kill()?;
                    child.wait()?;
                    panic!("cannot measure CLI child physical footprint or ps RSS");
                }
            }
        }
        if peak > memory || start.elapsed() >= timeout {
            child.kill()?;
            child.wait()?;
            stdout.join().unwrap();
            stderr.join().unwrap();
            panic!(
                "CLI child {} killed: footprint peak {peak} bytes; elapsed {:?}; limits {memory} bytes / {timeout:?}",
                child.id(),
                start.elapsed()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    Ok(std::process::Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    })
}

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
        .bounded_output()
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
        .bounded_output()
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
        .bounded_output()
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
        .bounded_output()
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
    let out = bounded_wait(child).unwrap();
    assert!(out.status.success());
    let expected = parse(&program, &library(&dir.join("schemas/text.json")), None).to_json();
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim_end(), expected);
}

#[test]
fn unknown_command_prints_usage_and_fails() {
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("frobnicate")
        .bounded_output()
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
        .bounded_output()
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
        .bounded_output()
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
            .bounded_output()
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
        .bounded_output()
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
            .bounded_output()
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
        .bounded_output()
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
        .bounded_output()
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
    let spec =
        serde_json::json!({"components": {}, "tools": [{"name": "t", "inputSchema": schema}]})
            .to_string();
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
        let out = bounded_wait(child).unwrap();
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
        .bounded_output()
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

#[test]
fn invalid_command_shapes_and_chunks_fail() {
    let dir = conformance();
    let schema = dir.join("schemas/text.json");
    let program = dir.join("cases/hello-text.oui");
    for args in [
        vec!["unknown", "-"],
        vec!["bench"],
        vec!["bench", "missing"],
        vec!["stream", "a", "b", "4", "extra"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
            .args(args)
            .bounded_output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8(out.stderr).unwrap().contains("usage:"));
    }
    for chunk in ["0", "-1", "abc"] {
        let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
            .arg("stream")
            .arg(&schema)
            .arg(&program)
            .arg(chunk)
            .bounded_output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8(out.stderr)
                .unwrap()
                .contains("chunk must be a positive number")
        );
    }
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("stream")
        .arg(&schema)
        .arg(&program)
        .arg("1")
        .bounded_output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        streamed(&schema, &program, 1)
    );
}

#[test]
fn the_fixed_result_size_limit_can_be_set() {
    let dir = conformance();
    let schema = dir.join("schemas/text.json");
    let program = dir.join("cases/hello-text.oui");
    let expected = openui_lang::parse_with_limits(
        &read(&program),
        &library(&schema),
        None,
        openui_lang::Limits::default().with_result_size(10),
    );
    assert!(expected.size_limited);
    assert_ne!(
        expected.to_json(),
        parse(&read(&program), &library(&schema), None).to_json()
    );
    for command in ["parse", "stream"] {
        let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
            .args([command, "--limit", "result-size=10"])
            .arg(&schema)
            .arg(&program)
            .bounded_output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
            expected.to_json(),
            "{command}"
        );
    }
}

// A child mode in the test binary, never in the published CLI. Normal runs
// return immediately; the parent uses small bounds to exercise both kills.
#[test]
fn bounded_child_fixture() {
    if std::env::var_os("OPENUI_CLI_BOUND_FIXTURE").is_some() {
        let bytes = vec![42u8; 8 * 1024 * 1024];
        std::hint::black_box(&bytes);
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

#[test]
fn child_memory_and_time_bounds_kill_and_reap() {
    for (memory, timeout) in [
        (1024 * 1024, std::time::Duration::from_secs(2)),
        (u64::MAX, std::time::Duration::from_millis(50)),
    ] {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "bounded_child_fixture"])
            .env("OPENUI_CLI_BOUND_FIXTURE", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let panic =
            std::panic::catch_unwind(|| bounded_wait_with(child, memory, timeout)).unwrap_err();
        let message = panic.downcast::<String>().unwrap();
        assert!(
            message.contains("CLI child") && message.contains("killed: footprint peak"),
            "{message}"
        );
    }
}
