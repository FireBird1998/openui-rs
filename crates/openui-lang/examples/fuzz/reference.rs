//! A client for `reference/serve.ts`: one long-lived Bun process answering
//! batches of requests.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

use crate::Program;

/// What the reference parser produced, as fingerprints.
pub enum Answer {
    Fingerprints {
        oneshot: String,
        tokens: String,
        steps: Vec<String>,
    },
    Threw(String),
}

pub struct Reference {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Reference {
    pub fn start(reference_dir: &Path) -> Reference {
        let mut child = Command::new("bun")
            .arg("serve.ts")
            .current_dir(reference_dir)
            .env("OPENUI_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start `bun serve.ts` (is Bun installed?)");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Reference {
            child,
            stdin,
            stdout,
        }
    }

    /// Answers for a batch, in order. Requests are written from another thread
    /// so neither side's pipe can fill up and stall the other.
    pub fn ask(&mut self, programs: &[&Program]) -> Vec<Answer> {
        let lines: Vec<String> = programs
            .iter()
            .map(|p| {
                let mut request = json!({ "lib": p.lib, "input": p.input, "sizes": p.sizes });
                if let Some(root) = &p.root {
                    request["root"] = json!(root);
                }
                request.to_string()
            })
            .collect();
        let stdin = &mut self.stdin;
        let stdout = &mut self.stdout;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for line in &lines {
                    writeln!(stdin, "{line}").expect("write to reference");
                }
                stdin.flush().expect("flush reference");
            });
            (0..programs.len())
                .map(|_| {
                    let mut line = String::new();
                    stdout.read_line(&mut line).expect("read from reference");
                    let answer: Value =
                        serde_json::from_str(&line).expect("reference answer is JSON");
                    match answer.get("threw") {
                        Some(error) => Answer::Threw(error.as_str().unwrap_or_default().to_owned()),
                        None => Answer::Fingerprints {
                            oneshot: answer["oneshot"].as_str().unwrap().to_owned(),
                            tokens: answer["tokens"].as_str().unwrap().to_owned(),
                            steps: answer["steps"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|s| s.as_str().unwrap().to_owned())
                                .collect(),
                        },
                    }
                })
                .collect()
        })
    }
}

impl Reference {
    /// Raw answers for a batch of request lines, in order.
    pub fn ask_lines(&mut self, lines: &[String]) -> Vec<String> {
        let stdin = &mut self.stdin;
        let stdout = &mut self.stdout;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for line in lines {
                    writeln!(stdin, "{line}").expect("write to reference");
                }
                stdin.flush().expect("flush reference");
            });
            (0..lines.len())
                .map(|_| {
                    let mut line = String::new();
                    stdout.read_line(&mut line).expect("read from reference");
                    line
                })
                .collect()
        })
    }
}

impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
