//! One interface over both parsers, so that an invariant is written once and
//! checked on the core and the reference alike: a full one-shot result, a
//! stream over given chunks, merging, and converting back to source.
//!
//! Calls come in batches, because the reference answers over a pipe; results
//! and trees cross the interface as JSON text, exactly as each parser wrote
//! them. An `Err` is the reference throwing or the core panicking.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use openui_lang::{Library, StreamParser, json_to_openui, merge_statements, parse, parse_json};
use serde_json::{Value, json};

use crate::reference::Reference;

pub struct ParseCall<'a> {
    pub lib: &'a str,
    pub input: &'a str,
    pub root: Option<&'a str>,
}

pub struct StreamCall<'a> {
    pub lib: &'a str,
    pub input: &'a str,
    pub root: Option<&'a str>,
    /// Push sizes in code points.
    pub sizes: &'a [usize],
    /// Whether to keep each push's result, or only the end's.
    pub steps: bool,
}

pub struct MergeCall<'a> {
    pub existing: &'a str,
    pub patch: &'a str,
    pub root_id: Option<&'a str>,
}

pub struct SerializeCall<'a> {
    pub lib: &'a str,
    /// The element tree's JSON.
    pub element: &'a str,
    /// A result's `stateDeclarations` JSON, if any.
    pub state: Option<&'a str>,
}

/// A stream's result after each push, and once it has ended.
pub struct Streamed {
    pub steps: Vec<String>,
    pub end: String,
}

pub type Answer<T> = Result<T, String>;

pub trait Engine {
    fn parse(&mut self, calls: &[ParseCall]) -> Vec<Answer<String>>;
    fn stream(&mut self, calls: &[StreamCall]) -> Vec<Answer<Streamed>>;
    fn merge(&mut self, calls: &[MergeCall]) -> Vec<Answer<String>>;
    fn serialize(&mut self, calls: &[SerializeCall]) -> Vec<Answer<String>>;
}

/// The core, in this process.
pub struct Core<'a> {
    pub libraries: &'a BTreeMap<String, Arc<Library>>,
}

fn caught<T>(f: impl FnOnce() -> T) -> Answer<T> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| "the core panicked".to_owned())
}

impl Engine for Core<'_> {
    fn parse(&mut self, calls: &[ParseCall]) -> Vec<Answer<String>> {
        calls
            .iter()
            .map(|c| caught(|| parse(c.input, &self.libraries[c.lib], c.root).to_json()))
            .collect()
    }

    fn stream(&mut self, calls: &[StreamCall]) -> Vec<Answer<Streamed>> {
        calls
            .iter()
            .map(|c| {
                caught(|| {
                    let mut parser = StreamParser::new(self.libraries[c.lib].clone(), c.root);
                    let chars: Vec<char> = c.input.chars().collect();
                    let mut at = 0;
                    let mut steps = Vec::new();
                    for &n in c.sizes {
                        let chunk: String = chars[at..at + n].iter().collect();
                        at += n;
                        let result = parser.push(&chunk);
                        if c.steps {
                            steps.push(result.to_json());
                        }
                    }
                    Streamed {
                        steps,
                        end: parser.finish().to_json(),
                    }
                })
            })
            .collect()
    }

    fn merge(&mut self, calls: &[MergeCall]) -> Vec<Answer<String>> {
        calls
            .iter()
            .map(|c| caught(|| merge_statements(c.existing, c.patch, c.root_id)))
            .collect()
    }

    fn serialize(&mut self, calls: &[SerializeCall]) -> Vec<Answer<String>> {
        calls
            .iter()
            .map(|c| {
                let element = parse_json(c.element).map_err(|e| e.to_string())?;
                let state = c
                    .state
                    .map(parse_json)
                    .transpose()
                    .map_err(|e| e.to_string())?;
                caught(|| json_to_openui(&element, &self.libraries[c.lib], state.as_ref()))?
                    .map_err(|e| e.to_string())
            })
            .collect()
    }
}

/// The reference, through `reference/serve.ts`.
pub struct Ref<'a> {
    pub reference: &'a mut Reference,
}

impl Ref<'_> {
    fn ask(&mut self, lines: Vec<String>) -> Vec<Value> {
        self.reference
            .ask_lines(&lines)
            .iter()
            // As `JSON.parse` reads it: a result may hold a lone surrogate.
            .map(|line| parse_json(line).expect("reference answer is JSON"))
            .collect()
    }
}

fn text(answer: &Value, key: &str) -> Answer<String> {
    match answer.get("threw") {
        Some(error) => Err(error.as_str().unwrap_or_default().to_owned()),
        None => Ok(answer[key]
            .as_str()
            .expect("reference answer field")
            .to_owned()),
    }
}

impl Engine for Ref<'_> {
    fn parse(&mut self, calls: &[ParseCall]) -> Vec<Answer<String>> {
        let lines = calls
            .iter()
            .map(|c| {
                json!({"op": "parse", "lib": c.lib, "input": c.input, "root": c.root}).to_string()
            })
            .collect();
        self.ask(lines).iter().map(|a| text(a, "json")).collect()
    }

    fn stream(&mut self, calls: &[StreamCall]) -> Vec<Answer<Streamed>> {
        let lines = calls
            .iter()
            .map(|c| {
                json!({"op": "stream", "lib": c.lib, "input": c.input, "root": c.root, "sizes": c.sizes, "steps": c.steps})
                    .to_string()
            })
            .collect();
        self.ask(lines)
            .iter()
            .map(|a| {
                let end = text(a, "end")?;
                let steps = a["steps"]
                    .as_array()
                    .expect("reference steps")
                    .iter()
                    .map(|s| s.as_str().expect("a step's JSON").to_owned())
                    .collect();
                Ok(Streamed { steps, end })
            })
            .collect()
    }

    fn merge(&mut self, calls: &[MergeCall]) -> Vec<Answer<String>> {
        let lines = calls
            .iter()
            .map(|c| {
                let mut request = json!({"op": "merge", "existing": c.existing, "patch": c.patch});
                if let Some(id) = c.root_id {
                    request["rootId"] = json!(id);
                }
                request.to_string()
            })
            .collect();
        self.ask(lines).iter().map(|a| text(a, "text")).collect()
    }

    fn serialize(&mut self, calls: &[SerializeCall]) -> Vec<Answer<String>> {
        // The tree goes as the parser wrote it, not re-encoded: a parser's
        // JSON has no raw newline, so it can sit in the request line as is.
        let lines = calls
            .iter()
            .map(|c| {
                let mut line = json!({"op": "serialize", "lib": c.lib}).to_string();
                line.pop();
                line.push_str(",\"element\":");
                line.push_str(c.element);
                if let Some(state) = c.state {
                    line.push_str(",\"state\":");
                    line.push_str(state);
                }
                line.push('}');
                line
            })
            .collect();
        self.ask(lines).iter().map(|a| text(a, "text")).collect()
    }
}
