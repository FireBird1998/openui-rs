//! Running against a clock and leaving findings for a hunt.
//!
//! `--seconds S` starts no batch in the last 5 s of S (a batch takes about one)
//! and stops shrinking at S, so a run ends within its time. `--findings DIR` saves shrunk cases into
//! DIR instead of the corpus, appends one JSON line per finding to
//! `DIR/findings.jsonl`, and before each batch writes the command that replays
//! it to `DIR/progress-<check>`, so a hunt can say where a run that died was.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;

pub struct Run {
    started: Instant,
    seconds: Option<f64>,
    /// The core taking longer than this on one input is a blow-up (`--slow-ms`).
    pub slow: Duration,
    /// Where findings go instead of the corpus (`--findings`).
    pub dir: Option<PathBuf>,
}

static RUN: OnceLock<Run> = OnceLock::new();

pub fn init(seconds: Option<u64>, slow_ms: u64, dir: Option<PathBuf>) {
    if let Some(dir) = &dir {
        std::fs::create_dir_all(dir).expect("create the findings folder");
    }
    let run = Run {
        started: Instant::now(),
        seconds: seconds.map(|s| s as f64),
        slow: Duration::from_millis(slow_ms),
        dir,
    };
    assert!(RUN.set(run).is_ok(), "findings::init runs once");
}

pub fn run() -> &'static Run {
    RUN.get().expect("findings::init runs first")
}

impl Run {
    fn may_start_batch(&self) -> bool {
        self.seconds
            .is_none_or(|s| self.started.elapsed().as_secs_f64() < s - (s / 10.0).min(5.0))
    }

    /// True once the run's time is up: shrinking stops and keeps what it has.
    pub fn out_of_time(&self) -> bool {
        self.seconds
            .is_some_and(|s| self.started.elapsed().as_secs_f64() >= s)
    }

    /// Where shrunk cases are saved: the findings folder, or the corpus.
    pub fn save_dir<'a>(&'a self, conformance: &'a Path) -> &'a Path {
        self.dir.as_deref().unwrap_or(conformance)
    }

    pub fn progress(&self, check: &str, replay: &str) {
        if let Some(dir) = &self.dir {
            std::fs::write(dir.join(format!("progress-{check}")), format!("{replay}\n"))
                .expect("write progress");
        }
    }

    /// A check's tallies, as `DIR/summary-<check>.json`.
    pub fn summary(&self, check: &str, summary: &Value) {
        if let Some(dir) = &self.dir {
            std::fs::write(
                dir.join(format!("summary-{check}.json")),
                format!("{summary}\n"),
            )
            .expect("write summary");
        }
    }

    pub fn record(&self, finding: &Value) {
        let Some(dir) = &self.dir else {
            return;
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("findings.jsonl"))
            .expect("open findings.jsonl");
        writeln!(file, "{finding}").expect("write findings.jsonl");
    }
}

/// The batches of input numbers to run: `--only K`; `--count N` of them from
/// `--from K`; or with `--seconds`, from `--from` on until time is up (and
/// `--count`, if given, still caps them).
pub fn batches(
    only: Option<u64>,
    from: u64,
    count: Option<u64>,
    size: usize,
) -> impl Iterator<Item = Vec<u64>> {
    let end = match (only, count, run().seconds) {
        (Some(k), _, _) => k + 1,
        (None, Some(n), _) => from + n,
        (None, None, Some(_)) => u64::MAX,
        (None, None, None) => from + 1000,
    };
    let mut next = only.unwrap_or(from);
    std::iter::from_fn(move || {
        if next >= end || !run().may_start_batch() {
            return None;
        }
        let batch: Vec<u64> = (next..end.min(next + size as u64)).collect();
        next += batch.len() as u64;
        Some(batch)
    })
}

/// What a differential run makes of one input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Outcome {
    Match,
    Mismatch,
    ReferenceThrew,
    CorePanicked,
    /// The core took longer than `--slow-ms`.
    BlowUp,
}

impl Outcome {
    /// The kind of finding, as far as the fuzzer can tell. The reference
    /// throwing where the core doesn't is a candidate quirk or divergence.
    pub fn kind(self) -> &'static str {
        match self {
            Outcome::Match => "none",
            Outcome::Mismatch => "mismatch",
            Outcome::CorePanicked => "crash",
            Outcome::BlowUp => "blow-up",
            Outcome::ReferenceThrew => "quirk?",
        }
    }
}
