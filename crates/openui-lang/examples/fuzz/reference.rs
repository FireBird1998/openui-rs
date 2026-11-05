//! A client for `reference/serve.ts`: one long-lived Bun process answering
//! batches of requests.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

use crate::Program;

/// What the reference parser produced, as fingerprints.
pub enum Answer {
    Fingerprints {
        oneshot: String,
        tokens: String,
        steps: Vec<String>,
        /// The result once the stream has ended.
        end: String,
        /// How long the reference took, in milliseconds.
        ms: f64,
    },
    Threw(String),
}

/// The answer to a request the reference died on (Bun says why on stderr,
/// usually that it ran out of memory). A fresh reference answers the rest.
pub const CRASHED: &str = "the reference crashed";

pub struct Reference {
    dir: PathBuf,
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
            dir: reference_dir.to_owned(),
            child,
            stdin,
            stdout,
        }
    }

    fn restart(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        *self = Reference::start(&self.dir.clone());
    }

    /// Answers for a batch, in order; a request the reference died on is
    /// [`Answer::Threw`] with [`CRASHED`].
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
        self.ask_lines(&lines)
            .iter()
            .map(|line| {
                let answer: Value = serde_json::from_str(line).expect("reference answer is JSON");
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
                        end: answer["end"].as_str().unwrap().to_owned(),
                        ms: answer["ms"].as_f64().unwrap_or(0.0),
                    },
                }
            })
            .collect()
    }
}

impl Reference {
    /// Raw answers for a batch of request lines, in order. If the reference
    /// dies, the request it was answering gets a `threw` answer of [`CRASHED`],
    /// and a fresh reference answers the rest.
    pub fn ask_lines(&mut self, lines: &[String]) -> Vec<String> {
        let mut answers = Vec::with_capacity(lines.len());
        while answers.len() < lines.len() {
            let rest = &lines[answers.len()..];
            let got = self.answers_until_eof(rest);
            let died = got.len() < rest.len();
            answers.extend(got);
            if died {
                answers.push(json!({ "threw": CRASHED }).to_string());
                self.restart();
            }
        }
        answers
    }

    /// Answers until the batch is done or the reference stops answering.
    /// Requests are written from another thread so neither side's pipe can
    /// fill up and stall the other; once the reference is gone, writing stops.
    fn answers_until_eof(&mut self, lines: &[String]) -> Vec<String> {
        let stdin = &mut self.stdin;
        let stdout = &mut self.stdout;
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for line in lines {
                    if writeln!(stdin, "{line}").is_err() {
                        return;
                    }
                }
                let _ = stdin.flush();
            });
            let mut answers = Vec::new();
            for _ in lines {
                let mut line = String::new();
                match stdout.read_line(&mut line) {
                    Ok(n) if n > 0 => answers.push(line),
                    _ => break,
                }
            }
            answers
        })
    }
}

impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
