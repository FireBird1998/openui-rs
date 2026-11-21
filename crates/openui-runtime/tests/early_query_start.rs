//! A measurement for #80, not a test of behaviour: how much earlier a query's
//! first tool call could go out if the query started as soon as its own
//! statement had finished streaming, instead of when the program settles, and
//! how many calls that would repeat or waste. It changes nothing in the
//! libraries: every hook it needs is computed here from public API.
//!
//! It replays every corpus program that has a query under every chunking the
//! corpus records for it, and registers the program as streamed so far with
//! the real query manager at every step, marking each query `complete` only
//! once it has finished under one of four rules:
//!
//! - **watermark**: the statement lies before the streaming parser's
//!   completed-statement watermark (a depth-0 line break whose next
//!   non-blank character has arrived and is not `?`). The scan mirrors the
//!   parser's private `scan_new_completed` over `preprocess` of the text so
//!   far; the text is trimmed, so the next statement must have begun.
//! - **inputs complete**: the watermark has passed the query, and nothing
//!   it reads is still arriving: a one-shot parse of the completed
//!   statements alone gives the same query statement (no pending statement
//!   was inlined into it), it has no unresolved reference, and every state
//!   variable it reads is declared by a completed statement with the same
//!   default.
//! - **line break**: the watermark, or the pending statement needs no
//!   implicit closing and a line break has arrived after it. Earlier by up to
//!   the gap before the next statement starts; it cannot see a `?` line that
//!   would continue the statement.
//! - **naive**: the parse result contains the query, which today is every
//!   query, including one auto-closed mid-argument.
//!
//! Calls are never answered while the stream lasts (no tool latency is
//! modelled), so a key that changes always costs a second call. At settle the
//! whole program is registered, as today; each early call is then answered
//! with a unique probe result, and it counts as useful exactly when the
//! manager delivers it to a query (ADR 0013, rule 3) and today's settle would
//! make the same call. Every other early call, including one the manager let
//! go before settle because its key moved on, is waste, sorted by the query
//! it was issued for: still before the watermark (half-written), waiting on a
//! query at settle, missing from the final program, or else repeated.
//!
//! A waiting query is also given its earliest start if every query it waits
//! on were answered the moment its own early call went out: a zero-latency
//! lower bound.
//!
//! Run it with
//! `cargo test --release -p openui-runtime --test early_query_start -- --ignored --nocapture`.
//! It prints a Markdown report; `EARLY_QUERY_DETAIL=<chunking>` adds one line
//! per query and rule for that chunking. The corpus programs with a query
//! have no timestamps, so positions are in bytes and chunks. The model
//! recordings in `conformance/recordings/` have timestamps but no query; they
//! are read only for their streaming rate, for a labelled illustration.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use openui_lang::{
    Ast, JsonValue, Library, ParseResult, QueryStatement, StreamParser, auto_close, parse,
    parse_json, preprocess, split, tokenize,
};
use openui_runtime::{QueryCommand, QueryManager, Store, ToolOutcome, value_to_json};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn json(path: &Path) -> JsonValue {
    parse_json(&read(path)).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()))
}

// ---------------------------------------------------------------------------
// The corpus

struct Case {
    name: String,
    manifest: &'static str,
    library: Library,
    root: Option<String>,
    text: String,
    chunkings: Vec<Chunking>,
}

enum Chunking {
    Sizes(String, Vec<usize>),
    Resync(String, Vec<(char, usize)>),
}

impl Chunking {
    fn name(&self) -> &str {
        match self {
            Chunking::Sizes(name, _) | Chunking::Resync(name, _) => name,
        }
    }
}

const MANIFESTS: [&str; 6] = [
    "manifest.json",
    "imported.json",
    "fuzz.json",
    "synthetic.json",
    "recorded.json",
    "runtime.json",
];
const FIXED: [usize; 6] = [1, 2, 3, 4, 7, 16];

struct Census {
    programs: usize,
    unique: usize,
    with_queries: usize,
    with_mutations: usize,
    reading_state: usize,
    reading_queries: usize,
    by_manifest: BTreeMap<&'static str, (usize, usize)>,
}

fn load() -> (Vec<Case>, Census) {
    let dir = conformance();
    let mut cases = Vec::new();
    let mut seen = HashSet::new();
    let mut census = Census {
        programs: 0,
        unique: 0,
        with_queries: 0,
        with_mutations: 0,
        reading_state: 0,
        reading_queries: 0,
        by_manifest: BTreeMap::new(),
    };
    // Schema names are shared across manifests, as the conformance tests load them.
    let manifests: Vec<(&str, JsonValue)> =
        MANIFESTS.iter().map(|m| (*m, json(&dir.join(m)))).collect();
    let mut libraries = HashMap::new();
    for (_, m) in &manifests {
        for (name, file) in m["schemas"].as_object().unwrap() {
            libraries.insert(name.clone(), json(&dir.join(file.as_str().unwrap())));
        }
    }
    for (manifest, m) in &manifests {
        let manifest = *manifest;
        let golden = if manifest == "runtime.json" {
            dir.join("golden/runtime")
        } else {
            dir.join("golden")
        };
        for c in m["cases"].as_array().unwrap() {
            census.programs += 1;
            let text = read(&dir.join(c["file"].as_str().unwrap()));
            if !seen.insert(text.clone()) {
                continue;
            }
            census.unique += 1;
            let library = Library::from_json_schema(&libraries[c["schema"].as_str().unwrap()]);
            let root = c["root"].as_str().map(str::to_owned);
            let one_shot = parse(&text, &library, root.as_deref());
            let entry = census.by_manifest.entry(manifest).or_default();
            entry.0 += 1;
            census.with_mutations += usize::from(!one_shot.mutation_statements.is_empty());
            if one_shot.query_statements.is_empty() {
                continue;
            }
            entry.1 += 1;
            census.with_queries += 1;
            census.reading_state += usize::from(
                one_shot
                    .query_statements
                    .iter()
                    .any(|q| q.deps.as_ref().is_some_and(|d| !d.is_empty())),
            );
            census.reading_queries += usize::from(
                one_shot
                    .query_statements
                    .iter()
                    .any(|q| q.wait_for.as_ref().is_some_and(|w| !w.is_empty())),
            );
            let name = c["name"].as_str().unwrap().to_owned();
            let steps_file = golden.join(format!("{name}.steps.json"));
            let steps = steps_file.exists().then(|| json(&steps_file));
            let mut chunkings: Vec<Chunking> = FIXED
                .iter()
                .map(|&size| {
                    let n = text.chars().count();
                    let sizes = (0..n).step_by(size).map(|s| size.min(n - s)).collect();
                    Chunking::Sizes(size.to_string(), sizes)
                })
                .collect();
            for (chunking, golden) in steps.iter().flat_map(|s| s.as_object().unwrap()) {
                if let Some(sizes) = golden.get("sizes").and_then(JsonValue::as_array) {
                    if chunking.parse::<usize>().is_err() {
                        chunkings.push(Chunking::Sizes(
                            chunking.clone(),
                            sizes.iter().map(|s| s.as_u64().unwrap() as usize).collect(),
                        ));
                    }
                } else if let Some(ops) = golden.get("ops").and_then(JsonValue::as_array) {
                    chunkings.push(Chunking::Resync(
                        chunking.clone(),
                        ops.iter()
                            .map(|op| {
                                (
                                    op[0].as_str().unwrap().chars().next().unwrap(),
                                    op[1].as_u64().unwrap_or(0) as usize,
                                )
                            })
                            .collect(),
                    ));
                }
            }
            cases.push(Case {
                name,
                manifest,
                library,
                root,
                text,
                chunkings,
            });
        }
    }
    (cases, census)
}

// ---------------------------------------------------------------------------
// Replaying a stream

struct Step {
    text: String,
    result: ParseResult,
    /// Chunks (or resync operations) taken so far.
    taken: usize,
}

fn char_prefix(text: &str, n: usize) -> &str {
    &text[..text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)]
}

/// The program with its middle character changed, as the conformance runner's
/// resync chunkings edit it.
fn edited(input: &str) -> String {
    let mut chars: Vec<char> = input.chars().collect();
    let m = chars.len() / 2;
    if let Some(c) = chars.get_mut(m) {
        *c = if *c == 'Z' { 'Y' } else { 'Z' };
    }
    chars.into_iter().collect()
}

/// Every step of the stream, then the settled result (the stream's end).
fn replay(case: &Case, chunking: &Chunking) -> (Vec<Step>, Step) {
    let library = std::sync::Arc::new(case.library.clone());
    let mut parser = StreamParser::new(library, case.root.as_deref());
    let mut steps = Vec::new();
    let mut text = String::new();
    match chunking {
        Chunking::Sizes(_, sizes) => {
            let mut rest = case.text.as_str();
            for (taken, &size) in sizes.iter().enumerate() {
                let part = char_prefix(rest, size);
                rest = &rest[part.len()..];
                text.push_str(part);
                let result = parser.push(part);
                steps.push(Step {
                    text: text.clone(),
                    result,
                    taken: taken + 1,
                });
            }
        }
        Chunking::Resync(_, ops) => {
            let other = edited(&case.text);
            for (taken, &(kind, n)) in ops.iter().enumerate() {
                let result = match kind {
                    'p' | 'x' => {
                        let source = if kind == 'p' { &case.text } else { &other };
                        text = char_prefix(source, n).to_owned();
                        parser.set(&text)
                    }
                    'r' => parser.result(),
                    _ => break, // 'e', the stream's end, is the settle step
                };
                steps.push(Step {
                    text: text.clone(),
                    result,
                    taken: taken + 1,
                });
            }
        }
    }
    let taken = steps.last().map_or(0, |s| s.taken);
    let settled = Step {
        result: parser.finish(),
        text,
        taken,
    };
    (steps, settled)
}

/// The streaming parser's completed-statement watermark, recomputed here from
/// the text so far (a copy of its private scan, which the public API does not
/// expose). Kept incrementally, as the parser keeps it.
#[derive(Default)]
struct Watermark {
    text: String,
    cleaned: String,
    end: usize,
    /// Completed statement ids, with the cleaned-text index of the line break
    /// that ended each and whether a next non-blank character has arrived
    /// (and was `?`).
    done: Vec<(String, usize, Option<u8>)>,
}

impl Watermark {
    fn step(&mut self, text: &str) {
        if !text.starts_with(self.text.as_str()) {
            *self = Watermark::default();
        }
        text.clone_into(&mut self.text);
        let cleaned = preprocess(text);
        if !cleaned.starts_with(&self.cleaned[..self.end]) {
            self.end = 0;
            self.done.clear();
        }
        self.cleaned = cleaned;
        let s = self.cleaned.as_bytes();
        let (mut depth, mut ternary) = (0i32, 0i32);
        let mut in_str: Option<u8> = None;
        let mut esc = false;
        let mut start = self.end;
        let mut i = self.end;
        while i < s.len() {
            let c = s[i];
            if esc {
                esc = false;
            } else if c == b'\\' && in_str.is_some() {
                esc = true;
            } else if let Some(q) = in_str {
                if c == q {
                    in_str = None;
                }
            } else {
                match c {
                    b'"' | b'\'' => in_str = Some(c),
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => depth = (depth - 1).max(0),
                    b'?' if depth == 0 => ternary += 1,
                    b':' if depth == 0 && ternary > 0 => ternary -= 1,
                    b'\n' if depth <= 0 && ternary <= 0 => {
                        let next = s[i + 1..]
                            .iter()
                            .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
                        if !next.is_some_and(|n| s[i + 1 + n] == b'?') {
                            let body = self.cleaned[start..i].trim();
                            if !body.is_empty() {
                                for raw in split(&tokenize(body)) {
                                    self.done.push((raw.id, i, None));
                                }
                            }
                            self.end = i + 1;
                            start = self.end;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        for (_, nl, next) in &mut self.done {
            if next.is_none() {
                *next = s[*nl + 1..]
                    .iter()
                    .copied()
                    .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'));
            }
        }
    }

    fn watermark(&self) -> HashSet<String> {
        self.done.iter().map(|(id, _, _)| id.clone()).collect()
    }

    /// Statements whose next statement has begun (the spec's rule 4.1).
    fn begun(&self) -> HashSet<String> {
        self.done
            .iter()
            .filter(|(_, _, next)| next.is_some_and(|b| b != b'?'))
            .map(|(id, _, _)| id.clone())
            .collect()
    }

    /// The watermark plus the pending statement when it needs no implicit
    /// closing and the raw text after it already holds a line break.
    fn line_break(&self) -> HashSet<String> {
        let mut ids = self.watermark();
        let pending = self.cleaned[self.end..].trim();
        let tail = &self.text[self.text.trim_end().len()..];
        if !pending.is_empty() && tail.contains('\n') && !auto_close(pending).1 {
            ids.extend(split(&tokenize(pending)).into_iter().map(|raw| raw.id));
        }
        ids
    }
}

// ---------------------------------------------------------------------------
// Keys

/// What one query, registered alone, would call: the cache key the manager
/// builds is the tool name, the arguments and the state the arguments read.
#[derive(Clone, Debug, PartialEq)]
enum Iso {
    Key(String),
    Waits,
    Refused(String),
    NoCall,
}

fn deps_json(stmt: &QueryStatement, store: &Store) -> String {
    let mut out = String::new();
    for name in stmt.deps.iter().flatten() {
        let value = store
            .get(name)
            .map_or_else(|| "undefined".to_owned(), |v| value_to_json(&v).unwrap());
        let _ = write!(out, "{name}={value};");
    }
    out
}

fn isolated(result: &ParseResult, stmt: &QueryStatement, store: &Store) -> Iso {
    if stmt.wait_for.as_ref().is_some_and(|w| !w.is_empty()) {
        return Iso::Waits;
    }
    let mut program = result.clone();
    let mut only = stmt.clone();
    only.complete = true;
    program.query_statements = vec![only];
    program.mutation_statements.clear();
    let mut manager = QueryManager::default();
    match manager.apply(QueryCommand::Evaluate {
        program: &program,
        store,
    }) {
        Ok(update) => match update.calls.as_slice() {
            [call] => Iso::Key(format!(
                "{}::{}::{}",
                call.tool_name,
                call.args
                    .as_ref()
                    .map_or_else(|| "undefined".to_owned(), |a| value_to_json(a).unwrap()),
                deps_json(stmt, store)
            )),
            _ => Iso::NoCall,
        },
        Err(e) => Iso::Refused(e.to_string()),
    }
}

fn has_placeholder(ast: &Ast) -> bool {
    match ast {
        Ast::Ph(_) | Ast::Ref(_) => true,
        Ast::Comp { args, .. } | Ast::Arr(args) => args.iter().any(has_placeholder),
        Ast::Obj(entries) => entries.iter().any(|(_, v)| has_placeholder(v)),
        Ast::BinOp { left, right, .. } => has_placeholder(left) || has_placeholder(right),
        Ast::UnaryOp { operand, .. } => has_placeholder(operand),
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => has_placeholder(cond) || has_placeholder(then) || has_placeholder(otherwise),
        Ast::Member { obj, .. } => has_placeholder(obj),
        Ast::Index { obj, index } => has_placeholder(obj) || has_placeholder(index),
        Ast::Assign { value, .. } => has_placeholder(value),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// One stream under one rule

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
enum Rule {
    InputsComplete,
    Watermark,
    LineBreak,
    Naive,
}

const RULES: [Rule; 4] = [
    Rule::InputsComplete,
    Rule::Watermark,
    Rule::LineBreak,
    Rule::Naive,
];

impl Rule {
    fn label(self) -> &'static str {
        match self {
            Rule::InputsComplete => "inputs complete",
            Rule::Watermark => "watermark",
            Rule::LineBreak => "line break",
            Rule::Naive => "naive",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Saved {
    bytes: usize,
    chunks: usize,
    fraction: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
enum Class {
    SameKey,
    DifferentKey,
    Waits,
    WaitsCalledEarly,
    OnlyAtSettle,
    Refused,
    NotInFinal,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::SameKey => "same key",
            Class::DifferentKey => "different key",
            Class::Waits => "waits on a query",
            Class::WaitsCalledEarly => "waits at settle, called early",
            Class::OnlyAtSettle => "finishes only at settle",
            Class::Refused => "refused or no call",
            Class::NotInFinal => "not in the final program",
        }
    }
}

struct QueryOutcome {
    id: String,
    class: Class,
    saved: Option<Saved>,
    /// Times its key changed after it first finished, before settle.
    changes: usize,
    early: Option<Iso>,
    final_key: Iso,
    tags: Vec<&'static str>,
    /// The text so far when it first finished.
    at: Option<String>,
    /// The step at which it first finished.
    first: Option<usize>,
}

#[derive(Default)]
struct RuleOutcome {
    baseline: usize,
    early: usize,
    useful: usize,
    at_settle: usize,
    refused_steps: usize,
    saved: Vec<Saved>,
    first: Option<Saved>,
    queries: Vec<QueryOutcome>,
    /// Early calls that were not useful, by why they went out.
    waste: BTreeMap<Waste, usize>,
    /// Waiting queries: where they could first start if every query they
    /// wait on were answered the moment its own call went out (zero tool
    /// latency), or `None` when only at settle or never.
    waits: Vec<WaitStart>,
}

struct WaitStart {
    id: String,
    /// Bytes still to come at the earliest start, if before settle.
    saved: Option<Saved>,
    /// It waits, directly or through others, on itself: no call ever.
    cycle: bool,
}

struct HalfWritten {
    id: String,
    tail: String,
    key: String,
    wrong: bool,
}

struct StreamOutcome {
    case: usize,
    chunking: String,
    total_bytes: usize,
    total_chunks: usize,
    rules: BTreeMap<Rule, RuleOutcome>,
    half_written: Vec<HalfWritten>,
    /// Steps where the watermark and the next-statement-begun rule disagree.
    begun_differs: usize,
}

#[derive(Default)]
struct Track {
    first: Option<usize>,
    first_key: Option<Iso>,
    last_key: Option<Iso>,
    changes: usize,
    first_deps: Vec<(String, String, bool)>,
    first_placeholder: bool,
    first_pending_input: bool,
}

/// An issued call: its identity, the step it went out at, and what it calls.
struct Issued {
    id: openui_runtime::ToolCallId,
    step: usize,
    call: String,
    /// The registered query whose key the call is for, at the step it went out.
    query: Option<String>,
    /// That query's statement had not passed the watermark: it was still
    /// being written (and, unless it needed no closing, auto-closed).
    half_written: bool,
}

/// Why an early call that was not useful went out.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
enum Waste {
    /// For a query the final program has, with a key that is not its final key.
    Repeated,
    /// For a query whose statement was still being written.
    HalfWritten,
    /// For a query that waits on another query at settle: it went out before
    /// the query it waits on had arrived.
    BeforeItsWait,
    /// For a query the final program does not have.
    NotInFinal,
}

impl Waste {
    fn label(self) -> &'static str {
        match self {
            Waste::Repeated => "repeated (query kept, key changed or call lost)",
            Waste::HalfWritten => "wasted: statement not past the watermark",
            Waste::BeforeItsWait => "wasted: before the query it waits on",
            Waste::NotInFinal => "wasted: query not in the final program",
        }
    }
}

struct Sim {
    manager: QueryManager,
    issued: Vec<Issued>,
    last: Option<(Vec<QueryStatement>, u64)>,
    refused: usize,
    tracks: BTreeMap<String, Track>,
}

struct StepView<'a> {
    result: &'a ParseResult,
    store: &'a Store,
    keys: &'a HashMap<&'a str, Iso>,
    marked: &'a HashSet<String>,
    index: usize,
}

fn register(sim: &mut Sim, view: &StepView<'_>, finished: &HashSet<String>) {
    let StepView {
        result,
        store,
        keys,
        marked,
        index: step,
    } = *view;
    let mut program = result.clone();
    program.mutation_statements.clear();
    for q in &mut program.query_statements {
        q.complete = finished.contains(&q.statement_id);
    }
    let registered: Vec<QueryStatement> = program
        .query_statements
        .iter()
        .filter(|q| q.complete)
        .cloned()
        .collect();
    let signature = (registered, store.version());
    if sim.last.as_ref() == Some(&signature) {
        return;
    }
    sim.last = Some(signature);
    match sim.manager.apply(QueryCommand::Evaluate {
        program: &program,
        store,
    }) {
        Ok(update) => sim.issued.extend(update.calls.into_iter().map(|call| {
            let text = call_text(&call);
            let prefix = format!("{text}::");
            let query = program
                .query_statements
                .iter()
                .filter(|q| q.complete)
                .find(|q| {
                    matches!(keys.get(q.statement_id.as_str()), Some(Iso::Key(k)) if k.starts_with(&prefix))
                })
                .map(|q| q.statement_id.clone());
            Issued {
                half_written: query.as_ref().is_some_and(|q| !marked.contains(q)),
                query,
                call: text,
                id: call.id,
                step,
            }
        })),
        Err(_) => sim.refused += 1,
    }
}

fn call_text(call: &openui_runtime::ToolCall) -> String {
    format!(
        "{}::{}",
        call.tool_name,
        call.args
            .as_ref()
            .map_or_else(|| "undefined".to_owned(), |a| value_to_json(a).unwrap())
    )
}

fn initialize(store: &mut Store, result: &ParseResult) {
    // A refused default leaves that name as it was; the counts below come
    // from the calls the manager asks for either way.
    let _ = store.initialize(
        result
            .state_declarations
            .iter()
            .map(|(name, value)| (name.as_str(), Some(value))),
        [],
    );
}

fn run(case_index: usize, case: &Case, chunking: &Chunking) -> StreamOutcome {
    let (steps, settled) = replay(case, chunking);
    let total_bytes = settled.text.len();
    let total_chunks = settled.taken;
    let saved_at = |step: &Step| {
        let bytes = total_bytes.saturating_sub(step.text.len());
        Saved {
            bytes,
            chunks: total_chunks - step.taken,
            fraction: if total_bytes == 0 {
                0.0
            } else {
                bytes as f64 / total_bytes as f64
            },
        }
    };
    let mut store = Store::new();
    let mut watermark = Watermark::default();
    let mut sims: BTreeMap<Rule, Sim> = RULES
        .iter()
        .map(|&rule| {
            (
                rule,
                Sim {
                    manager: QueryManager::default(),
                    issued: Vec::new(),
                    last: None,
                    refused: 0,
                    tracks: BTreeMap::new(),
                },
            )
        })
        .collect();
    let mut memo: HashMap<(String, String), (QueryStatement, Iso)> = HashMap::new();
    // A query seen before its statement finished: (id, key) and the last line
    // of the first text that produced that key.
    let mut half: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut begun_differs = 0;
    // A one-shot parse of the completed statements alone, at each watermark.
    let mut completed: (usize, ParseResult) = (usize::MAX, ParseResult::default());
    for (index, step) in steps.iter().enumerate() {
        initialize(&mut store, &step.result);
        watermark.step(&step.text);
        let marked = watermark.watermark();
        if marked != watermark.begun() {
            begun_differs += 1;
        }
        if completed.0 != watermark.end {
            completed = (
                watermark.end,
                parse(
                    &watermark.cleaned[..watermark.end],
                    &case.library,
                    case.root.as_deref(),
                ),
            );
        }
        // A query reads something still arriving when the completed
        // statements alone give it a different statement (a pending
        // statement was inlined into it) or a state it reads a different
        // default.
        let pending_input: HashMap<&str, bool> = step
            .result
            .query_statements
            .iter()
            .map(|q| {
                let alone = completed
                    .1
                    .query_statements
                    .iter()
                    .find(|c| c.statement_id == q.statement_id);
                let pending = alone != Some(q)
                    || q.tool.as_ref().is_some_and(has_placeholder)
                    || q.args.as_ref().is_some_and(has_placeholder)
                    || q.deps.iter().flatten().any(|d| {
                        !marked.contains(d)
                            || completed.1.state_declarations.get(d)
                                != step.result.state_declarations.get(d)
                    });
                (q.statement_id.as_str(), pending)
            })
            .collect();
        let inputs_complete: HashSet<String> = marked
            .iter()
            .filter(|id| pending_input.get(id.as_str()) == Some(&false))
            .cloned()
            .collect();
        let finished: BTreeMap<Rule, HashSet<String>> = [
            (Rule::InputsComplete, inputs_complete),
            (Rule::Watermark, marked.clone()),
            (Rule::LineBreak, watermark.line_break()),
            (
                Rule::Naive,
                step.result
                    .query_statements
                    .iter()
                    .map(|q| q.statement_id.clone())
                    .collect(),
            ),
        ]
        .into_iter()
        .collect();
        // Each query's key as streamed so far, shared by the rules.
        let mut keys: HashMap<&str, Iso> = HashMap::new();
        for q in &step.result.query_statements {
            let memo_key = (q.statement_id.clone(), deps_json(q, &store));
            let key = match memo.get(&memo_key) {
                Some((stmt, key)) if stmt == q => key.clone(),
                _ => {
                    let key = isolated(&step.result, q, &store);
                    memo.insert(memo_key, (q.clone(), key.clone()));
                    key
                }
            };
            if !marked.contains(&q.statement_id) {
                half.entry((q.statement_id.clone(), format!("{key:?}")))
                    .or_insert_with(|| step.text.lines().last().unwrap_or("").to_owned());
            }
            keys.insert(q.statement_id.as_str(), key);
        }
        let view = StepView {
            result: &step.result,
            store: &store,
            keys: &keys,
            marked: &marked,
            index,
        };
        for (rule, sim) in &mut sims {
            let finished = &finished[rule];
            register(sim, &view, finished);
            for q in &step.result.query_statements {
                if !finished.contains(&q.statement_id) {
                    continue;
                }
                let key = keys[q.statement_id.as_str()].clone();
                let track = sim.tracks.entry(q.statement_id.clone()).or_default();
                if track.first.is_none() {
                    track.first = Some(index);
                    track.first_key = Some(key.clone());
                    track.first_placeholder = q.tool.as_ref().is_some_and(has_placeholder)
                        || q.args.as_ref().is_some_and(has_placeholder);
                    track.first_pending_input = pending_input[q.statement_id.as_str()];
                    track.first_deps = q
                        .deps
                        .iter()
                        .flatten()
                        .map(|d| {
                            let value = store
                                .get(d)
                                .map_or_else(|| "undefined".into(), |v| value_to_json(&v).unwrap());
                            (d.clone(), value, marked.contains(d))
                        })
                        .collect();
                } else if track.last_key.as_ref() != Some(&key) {
                    track.changes += 1;
                }
                track.last_key = Some(key);
            }
        }
    }

    // Settle: the whole program registers, as today.
    initialize(&mut store, &settled.result);
    let mut baseline_manager = QueryManager::default();
    let mut baseline_store = Store::new();
    initialize(&mut baseline_store, &settled.result);
    let mut final_program = settled.result.clone();
    final_program.mutation_statements.clear();
    for q in &mut final_program.query_statements {
        q.complete = true;
    }
    let baseline_calls: Vec<String> = baseline_manager
        .apply(QueryCommand::Evaluate {
            program: &final_program,
            store: &baseline_store,
        })
        .map_or_else(|_| Vec::new(), |u| u.calls.iter().map(call_text).collect());
    let baseline = baseline_calls.len();
    let final_keys: HashMap<&str, Iso> = settled
        .result
        .query_statements
        .iter()
        .map(|q| {
            (
                q.statement_id.as_str(),
                isolated(&settled.result, q, &store),
            )
        })
        .collect();

    let mut rules = BTreeMap::new();
    for (rule, mut sim) in sims {
        let at_settle = sim
            .manager
            .apply(QueryCommand::Evaluate {
                program: &final_program,
                store: &store,
            })
            .map_or(0, |u| u.calls.len());
        let mut outcome = RuleOutcome {
            baseline,
            early: sim.issued.len(),
            at_settle,
            refused_steps: sim.refused,
            ..RuleOutcome::default()
        };
        // An early call is useful when the manager delivers it to a query at
        // settle and today's settle would make the same call.
        let mut wanted = baseline_calls.clone();
        for (n, issued) in sim.issued.iter().enumerate() {
            // A call the manager has already let go (its key moved on before
            // settle) can deliver nothing: it is counted as waste below.
            let probe = parse_json(&format!("{{\"probe\":{n}}}")).unwrap();
            let delivered = issued.id.is_pending()
                && sim
                    .manager
                    .apply(QueryCommand::Complete {
                        id: &issued.id,
                        outcome: ToolOutcome::Result(Some(&probe)),
                    })
                    .is_ok_and(|u| u.changed);
            let matching = wanted.iter().position(|c| *c == issued.call);
            if let (true, Some(i)) = (delivered, matching) {
                wanted.swap_remove(i);
                outcome.useful += 1;
                let saved = saved_at(&steps[issued.step]);
                outcome.saved.push(saved);
                if outcome.first.is_none_or(|f| saved.bytes > f.bytes) {
                    outcome.first = Some(saved);
                }
            } else {
                let waste = if issued.half_written {
                    Waste::HalfWritten
                } else {
                    match issued.query.as_deref().and_then(|q| final_keys.get(q)) {
                        None => Waste::NotInFinal,
                        Some(Iso::Waits) => Waste::BeforeItsWait,
                        Some(_) => Waste::Repeated,
                    }
                };
                *outcome.waste.entry(waste).or_default() += 1;
            }
        }
        for q in &settled.result.query_statements {
            let final_key = final_keys[q.statement_id.as_str()].clone();
            let track = sim.tracks.remove(&q.statement_id).unwrap_or_default();
            let early = track.first_key.clone();
            let class = match (&final_key, &early) {
                (Iso::Waits, Some(Iso::Key(_))) => Class::WaitsCalledEarly,
                (Iso::Waits, _) => Class::Waits,
                (_, None) => Class::OnlyAtSettle,
                (Iso::Refused(_) | Iso::NoCall, _) => Class::Refused,
                (key, Some(first)) if key == first => Class::SameKey,
                _ => Class::DifferentKey,
            };
            let mut tags = Vec::new();
            if q.deps.as_ref().is_some_and(|d| !d.is_empty()) {
                tags.push("reads state");
            }
            if track.first.is_some() {
                if track.first_placeholder {
                    tags.push("read a statement not yet arrived");
                } else if track.first_pending_input {
                    tags.push("read a statement still arriving");
                }
                for (name, value, declared) in &track.first_deps {
                    let now = store
                        .get(name)
                        .map_or_else(|| "undefined".into(), |v| value_to_json(&v).unwrap());
                    if &now != value {
                        tags.push(if *declared {
                            "state default changed later"
                        } else {
                            "state declared later"
                        });
                    }
                }
                if class == Class::SameKey && tags.iter().all(|t| *t == "reads state") {
                    tags.push("nothing it read changed");
                }
            }
            outcome.queries.push(QueryOutcome {
                id: q.statement_id.clone(),
                class,
                saved: (class == Class::SameKey)
                    .then(|| track.first.map(|i| saved_at(&steps[i])))
                    .flatten(),
                changes: track.changes,
                early,
                final_key,
                tags,
                at: track.first.map(|i| steps[i].text.clone()),
                first: track.first,
            });
        }
        // A waiting query could start once it has finished and every query
        // it waits on has been answered. With zero tool latency that is when
        // the last of them could start, if each of those is itself right.
        let wait_for: HashMap<&str, Vec<String>> = settled
            .result
            .query_statements
            .iter()
            .map(|q| {
                (
                    q.statement_id.as_str(),
                    q.wait_for.clone().unwrap_or_default(),
                )
            })
            .collect();
        let by_id: HashMap<&str, &QueryOutcome> =
            outcome.queries.iter().map(|q| (q.id.as_str(), q)).collect();
        let mut waits = Vec::new();
        for q in outcome.queries.iter().filter(|q| q.final_key == Iso::Waits) {
            let start = earliest_start(&q.id, &by_id, &wait_for, &mut Vec::new());
            waits.push(WaitStart {
                id: q.id.clone(),
                saved: match start {
                    Start::At(i) => Some(saved_at(&steps[i])),
                    _ => None,
                },
                cycle: start == Start::Never,
            });
        }
        outcome.waits = waits;
        for (id, track) in sim.tracks {
            outcome.queries.push(QueryOutcome {
                id,
                class: Class::NotInFinal,
                saved: None,
                changes: track.changes,
                early: track.first_key,
                final_key: Iso::NoCall,
                tags: Vec::new(),
                at: track.first.map(|i| steps[i].text.clone()),
                first: track.first,
            });
        }
        rules.insert(rule, outcome);
    }
    let half_written = half
        .into_iter()
        .map(|((id, key), tail)| {
            let wrong = final_keys
                .get(id.as_str())
                .is_none_or(|f| format!("{f:?}") != key);
            HalfWritten {
                id,
                tail,
                key,
                wrong,
            }
        })
        .collect();
    StreamOutcome {
        case: case_index,
        chunking: chunking.name().to_owned(),
        total_bytes,
        total_chunks,
        rules,
        half_written,
        begun_differs,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Start {
    /// At this step of the stream.
    At(usize),
    /// Only once the program settles.
    Settle,
    /// Never: it waits, directly or not, on a loop of queries.
    Never,
}

fn earliest_start(
    id: &str,
    by_id: &HashMap<&str, &QueryOutcome>,
    wait_for: &HashMap<&str, Vec<String>>,
    seen: &mut Vec<String>,
) -> Start {
    if seen.iter().any(|s| s == id) {
        return Start::Never;
    }
    let Some(q) = by_id.get(id) else {
        return Start::Settle;
    };
    if q.final_key != Iso::Waits {
        return match (q.class, q.first) {
            (Class::SameKey, Some(i)) => Start::At(i),
            _ => Start::Settle,
        };
    }
    seen.push(id.to_owned());
    let mut start = q.first.map_or(Start::Settle, Start::At);
    for w in wait_for.get(id).into_iter().flatten() {
        start = match (start, earliest_start(w, by_id, wait_for, seen)) {
            (Start::Never, _) | (_, Start::Never) => Start::Never,
            (Start::Settle, _) | (_, Start::Settle) => Start::Settle,
            (Start::At(a), Start::At(b)) => Start::At(a.max(b)),
        };
    }
    seen.pop();
    start
}

// ---------------------------------------------------------------------------
// Statistics and the report

/// Min, q1, median, q3 and max as five table cells, without outer pipes.
fn cells(mut values: Vec<f64>) -> String {
    if values.is_empty() {
        return "– | – | – | – | –".to_owned();
    }
    values.sort_by(f64::total_cmp);
    let at = |p: f64| {
        let i = p * (values.len() - 1) as f64;
        let (lo, hi) = (i.floor() as usize, i.ceil() as usize);
        values[lo] + (values[hi] - values[lo]) * (i - lo as f64)
    };
    format!(
        "{} | {} | {} | {} | {}",
        fmt(at(0.0)),
        fmt(at(0.25)),
        fmt(at(0.5)),
        fmt(at(0.75)),
        fmt(at(1.0))
    )
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

fn fmt(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{v:.0}")
    } else if v.abs() < 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.1}")
    }
}

fn group(name: &str) -> &'static str {
    match name.split('/').next().unwrap_or("") {
        "docs" => "upstream docs",
        "test" => "upstream tests",
        "spec" => "upstream spec fixtures",
        "examples" => "upstream examples",
        "hand" => "hand-written",
        "runtime" => "runtime cases",
        _ => "other",
    }
}

struct Rate {
    model: String,
    bytes: usize,
    seconds: f64,
    bytes_per_s: f64,
    delta_chars: Vec<f64>,
}

fn recording_rates() -> Vec<Rate> {
    let dir = conformance().join("recordings");
    let mut rates = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let r = json(&path);
        let chunks = r["chunks"].as_array().unwrap();
        if chunks.len() < 2 || r.get("patchOf").is_some() {
            continue;
        }
        let t = |c: &JsonValue| c["t"].as_f64().unwrap();
        let bytes: usize = chunks
            .iter()
            .map(|c| c["text"].as_str().unwrap().len())
            .sum();
        let span = t(&chunks[chunks.len() - 1]) - t(&chunks[0]);
        if span <= 0.0 {
            continue;
        }
        rates.push(Rate {
            model: r["servedBy"]
                .as_str()
                .or(r["model"].as_str())
                .unwrap()
                .to_owned(),
            bytes,
            seconds: span,
            bytes_per_s: bytes as f64 / span,
            delta_chars: chunks
                .iter()
                .map(|c| c["text"].as_str().unwrap().chars().count() as f64)
                .collect(),
        });
    }
    rates
}

/// A query's class in one chunking, with the fraction still to come if right.
type ClassSeen = (Class, Option<f64>);

/// Chunkings that replay the program as a stream; `resync-*` also `set` an
/// edited program, so they measure edits as well as streaming.
fn is_stream(chunking: &str) -> bool {
    !chunking.starts_with("resync")
}

/// The headline tables: every rule at code-point resolution (chunking 1, so
/// a call can go out after any character), the waste by reason, waiting
/// queries, whether the answer depends on chunking, and the wall-clock
/// illustration.
fn summary(w: &mut String, cases: &[Case], outcomes: &[StreamOutcome], rates: &[Rate]) {
    let at_1: Vec<&StreamOutcome> = outcomes.iter().filter(|o| o.chunking == "1").collect();
    let _ = writeln!(
        w,
        "\n## Summary at code-point resolution (chunking 1, {} streams)\n",
        at_1.len()
    );
    let _ = writeln!(
        w,
        "| Rule | Queries | Same key | Different key | Waits | Waits, called early | Only at settle | Not in final | Calls at settle today | Early calls | Useful | {} | {} | {} | {} |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
        Waste::Repeated.label(),
        Waste::HalfWritten.label(),
        Waste::BeforeItsWait.label(),
        Waste::NotInFinal.label()
    );
    for rule in RULES {
        let mut classes: BTreeMap<Class, usize> = BTreeMap::new();
        let mut waste: BTreeMap<Waste, usize> = BTreeMap::new();
        let (mut base, mut early, mut useful) = (0, 0, 0);
        for o in &at_1 {
            let r = &o.rules[&rule];
            for q in &r.queries {
                *classes.entry(q.class).or_default() += 1;
            }
            for (k, n) in &r.waste {
                *waste.entry(*k).or_default() += n;
            }
            base += r.baseline;
            early += r.early;
            useful += r.useful;
        }
        let c = |k| classes.get(&k).copied().unwrap_or(0);
        let x = |k| waste.get(&k).copied().unwrap_or(0);
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {base} | {early} | {useful} | {} | {} | {} | {} |",
            rule.label(),
            classes.values().sum::<usize>(),
            c(Class::SameKey),
            c(Class::DifferentKey),
            c(Class::Waits),
            c(Class::WaitsCalledEarly),
            c(Class::OnlyAtSettle),
            c(Class::NotInFinal),
            x(Waste::Repeated),
            x(Waste::HalfWritten),
            x(Waste::BeforeItsWait),
            x(Waste::NotInFinal),
        );
    }

    let _ = writeln!(
        w,
        "\nHow much of the stream was still to come when a useful early call could go out (chunking 1, one row per useful call; bytes equal code points here only for ASCII):\n\n| Rule | Useful calls | Fraction: min | q1 | median | q3 | max | Bytes: min | q1 | median | q3 | max |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    for rule in RULES {
        let saved: Vec<Saved> = at_1
            .iter()
            .flat_map(|o| o.rules[&rule].saved.iter().copied())
            .collect();
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} |",
            rule.label(),
            saved.len(),
            cells(saved.iter().map(|s| s.fraction).collect()),
            cells(saved.iter().map(|s| s.bytes as f64).collect())
        );
    }

    // Waiting queries.
    let _ = writeln!(
        w,
        "\n## Queries that wait on another query (chunking 1)\n\nEarliest start if every query waited on were answered the moment its own early call went out (zero tool latency: a lower bound).\n\n| Rule | Waiting queries | Could start before settle | Only at settle | Never (waits on a loop) | Fraction still to come: min | q1 | median | q3 | max |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    for rule in [Rule::InputsComplete, Rule::Watermark] {
        let waits: Vec<&WaitStart> = at_1.iter().flat_map(|o| &o.rules[&rule].waits).collect();
        let early: Vec<f64> = waits
            .iter()
            .filter_map(|s| s.saved.map(|s| s.fraction))
            .collect();
        let never = waits.iter().filter(|s| s.cycle).count();
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} | {never} | {} |",
            rule.label(),
            waits.len(),
            early.len(),
            waits.len() - early.len() - never,
            cells(early)
        );
    }
    let _ = writeln!(w, "\nEach waiting query, inputs-complete rule:\n");
    for o in &at_1 {
        for s in &o.rules[&Rule::InputsComplete].waits {
            let when = match (s.cycle, s.saved) {
                (true, _) => "never (a loop)".to_owned(),
                (false, None) => "only at settle".to_owned(),
                (false, Some(saved)) => {
                    format!("{} of {} bytes still to come", saved.bytes, o.total_bytes)
                }
            };
            let _ = writeln!(w, "- `{}`, `{}`: {when}", cases[o.case].name, s.id);
        }
    }

    // Chunking dependence.
    let _ = writeln!(
        w,
        "\n## Does the answer depend on chunking? (every streaming chunking a program has; resync excluded)\n\n| Rule | Queries | Same class in every chunking | Differs | Same key everywhere: spread of fraction still to come, median | max | Early calls per chunking 1 / 4 / 16 / random-1 |\n|---|---:|---:|---:|---:|---:|---|"
    );
    for rule in RULES {
        let mut per_query: BTreeMap<(usize, &str), Vec<ClassSeen>> = BTreeMap::new();
        for o in outcomes.iter().filter(|o| is_stream(&o.chunking)) {
            for q in &o.rules[&rule].queries {
                per_query
                    .entry((o.case, q.id.as_str()))
                    .or_default()
                    .push((q.class, q.saved.map(|s| s.fraction)));
            }
        }
        let mut differs = Vec::new();
        let mut spreads = Vec::new();
        for (key, seen) in &per_query {
            if seen.iter().any(|(c, _)| *c != seen[0].0) {
                differs.push(*key);
            } else if seen[0].0 == Class::SameKey {
                let f: Vec<f64> = seen.iter().filter_map(|(_, f)| *f).collect();
                let lo = f.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = f.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                spreads.push(hi - lo);
            }
        }
        let calls = ["1", "4", "16", "random-1"]
            .iter()
            .map(|c| {
                outcomes
                    .iter()
                    .filter(|o| o.chunking == *c)
                    .map(|o| o.rules[&rule].early)
                    .sum::<usize>()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(" / ");
        let max_spread = spreads.iter().copied().fold(0.0, f64::max);
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {calls} |",
            rule.label(),
            per_query.len(),
            per_query.len() - differs.len(),
            differs.len(),
            fmt(median(spreads)),
            fmt(max_spread)
        );
        if rule != Rule::Naive {
            for (case, id) in differs.iter().take(6) {
                let classes: Vec<String> = outcomes
                    .iter()
                    .filter(|o| o.case == *case && is_stream(&o.chunking))
                    .filter_map(|o| {
                        o.rules[&rule]
                            .queries
                            .iter()
                            .find(|q| q.id == *id)
                            .map(|q| format!("{}: {}", o.chunking, q.class.label()))
                    })
                    .collect();
                let _ = writeln!(
                    w,
                    "  - {}: `{}` `{id}`: {}",
                    rule.label(),
                    cases[*case].name,
                    classes.join(", ")
                );
            }
        }
    }

    // Wall clock, as an illustration only.
    let mut bps: Vec<f64> = rates.iter().map(|r| r.bytes_per_s).collect();
    bps.sort_by(f64::total_cmp);
    let pick = |p: f64| bps[((bps.len() - 1) as f64 * p).round() as usize];
    let saved: Vec<f64> = at_1
        .iter()
        .flat_map(|o| o.rules[&Rule::InputsComplete].saved.iter())
        .map(|s| s.bytes as f64)
        .collect();
    let firsts: Vec<f64> = at_1
        .iter()
        .filter(|o| o.rules[&Rule::InputsComplete].baseline > 0)
        .map(|o| {
            o.rules[&Rule::InputsComplete]
                .first
                .unwrap_or_default()
                .bytes as f64
        })
        .collect();
    let _ = writeln!(
        w,
        "\n## Illustration only: bytes still to come as seconds (inputs-complete rule, chunking 1)\n\nSeconds = bytes still to come / streaming rate. The rates are the model recordings' (none of which has a query), first to last delta. Real savings depend on the model, the network and the tool's own latency.\n\n| Rate | Bytes/s | Per useful call: median s | q3 s | max s | First call per stream: median s | q3 s | max s |\n|---|---:|---:|---:|---:|---:|---:|---:|"
    );
    for (label, p) in [("slow (q1)", 0.25), ("median", 0.5), ("fast (q3)", 0.75)] {
        let rate = pick(p);
        let secs = |v: &[f64], q: f64| {
            let mut v = v.to_vec();
            v.sort_by(f64::total_cmp);
            if v.is_empty() {
                return 0.0;
            }
            v[((v.len() - 1) as f64 * q).round() as usize] / rate
        };
        let _ = writeln!(
            w,
            "| {label} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
            fmt(rate),
            secs(&saved, 0.5),
            secs(&saved, 0.75),
            secs(&saved, 1.0),
            secs(&firsts, 0.5),
            secs(&firsts, 0.75),
            secs(&firsts, 1.0)
        );
    }
    let mut durations: Vec<f64> = rates.iter().map(|r| r.seconds).collect();
    durations.sort_by(f64::total_cmp);
    let typical = durations[durations.len() / 2];
    let fractions: Vec<f64> = at_1
        .iter()
        .flat_map(|o| o.rules[&Rule::InputsComplete].saved.iter())
        .map(|s| s.fraction)
        .collect();
    let mut f = fractions.clone();
    f.sort_by(f64::total_cmp);
    let _ = writeln!(
        w,
        "\nThe corpus programs are short (see the corpus table). Scaled instead to a typical recorded response ({typical:.1} s first to last delta), the useful calls' fractions still to come would be: median {:.1} s, q3 {:.1} s, max {:.1} s. This assumes a model-written program puts its queries where the corpus does, which nothing here shows.",
        f[f.len() / 2] * typical,
        f[(f.len() - 1) * 3 / 4] * typical,
        f[f.len() - 1] * typical
    );

    // Per-query detail on request.
    if let Ok(chunking) = std::env::var("EARLY_QUERY_DETAIL") {
        let _ = writeln!(w, "\n## Every query, chunking {chunking}\n");
        for o in outcomes.iter().filter(|o| o.chunking == chunking) {
            for rule in RULES {
                for q in &o.rules[&rule].queries {
                    let _ = writeln!(
                        w,
                        "- {} | `{}` `{}` | {} | still to come {} | changes {} | {:?} | early `{}` | final `{:?}`",
                        rule.label(),
                        cases[o.case].name,
                        q.id,
                        q.class.label(),
                        q.saved
                            .map_or("–".into(), |s| format!("{}/{}", s.bytes, o.total_bytes)),
                        q.changes,
                        q.tags,
                        q.early.as_ref().map_or("–".into(), |k| format!("{k:?}")),
                        q.final_key
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "measurement for #80; prints a report"]
fn early_query_start_report() {
    let (cases, census) = load();
    let mut outcomes = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        for chunking in &case.chunkings {
            outcomes.push(run(i, case, chunking));
        }
    }
    let mut out = String::new();
    let w = &mut out;
    // The corpus.
    let _ = writeln!(w, "## Corpus\n");
    let _ = writeln!(
        w,
        "{} parse cases, {} distinct programs; {} with a query, {} with a mutation, {} with a query that reads state, {} with a query that reads another query.\n",
        census.programs,
        census.unique,
        census.with_queries,
        census.with_mutations,
        census.reading_state,
        census.reading_queries
    );
    let _ = writeln!(
        w,
        "| Manifest | Distinct programs | With a query |\n|---|---:|---:|"
    );
    for (m, (n, q)) in &census.by_manifest {
        let _ = writeln!(w, "| {m} | {n} | {q} |");
    }
    let mut groups: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for case in &cases {
        let g = groups.entry(group(&case.name)).or_default();
        g.0 += 1;
        g.2 += case.text.len();
        g.1 += parse(&case.text, &case.library, case.root.as_deref())
            .query_statements
            .len();
    }
    let _ = writeln!(
        w,
        "\n| Source of the programs with a query | Programs | Queries | Median size (bytes) |\n|---|---:|---:|---:|"
    );
    for (g, (n, q, _)) in &groups {
        let mut sizes: Vec<f64> = cases
            .iter()
            .filter(|c| group(&c.name) == *g)
            .map(|c| c.text.len() as f64)
            .collect();
        sizes.sort_by(f64::total_cmp);
        let _ = writeln!(w, "| {g} | {n} | {q} | {} |", fmt(sizes[sizes.len() / 2]));
    }
    let chunking_count: usize = cases.iter().map(|c| c.chunkings.len()).sum();
    let _ = writeln!(
        w,
        "\n{} streams replayed ({} programs × their chunkings: 1, 2, 3, 4, 7 and 16 code points, the seeded random ones, recorded chunks and resync sequences where the goldens hold them).",
        chunking_count,
        cases.len()
    );
    let recorded_queries = cases
        .iter()
        .filter(|c| c.manifest == "recorded.json")
        .count();
    let _ = writeln!(
        w,
        "Recorded model streams with a query: {recorded_queries}."
    );

    // The recordings' streaming rate.
    let rates = recording_rates();
    let _ = writeln!(
        w,
        "\n## Streaming rate of the model recordings (fresh requests, first to last delta)\n"
    );
    let _ = writeln!(
        w,
        "| Model | Recordings | Bytes/s: min | q1 | median | q3 | max | Median delta (code points) | Median program (bytes) | Median first-to-last delta (s) |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    let mut models: BTreeMap<&str, Vec<&Rate>> = BTreeMap::new();
    for r in &rates {
        models.entry(r.model.as_str()).or_default().push(r);
        models.entry("all").or_default().push(r);
    }
    for (model, rs) in &models {
        let mut deltas: Vec<f64> = rs
            .iter()
            .flat_map(|r| r.delta_chars.iter().copied())
            .collect();
        deltas.sort_by(f64::total_cmp);
        let _ = writeln!(
            w,
            "| {model} | {} | {} | {} | {} | {} |",
            rs.len(),
            cells(rs.iter().map(|r| r.bytes_per_s).collect()),
            fmt(deltas[deltas.len() / 2]),
            fmt(median(rs.iter().map(|r| r.bytes as f64).collect())),
            fmt(median(rs.iter().map(|r| r.seconds).collect()))
        );
    }

    // Statement order.
    let mut positions = Vec::new();
    let mut buckets: BTreeMap<&str, usize> = BTreeMap::new();
    let mut state_order: BTreeMap<&str, usize> = BTreeMap::new();
    for case in &cases {
        let cleaned = preprocess(&case.text);
        let ids: Vec<String> = split(&tokenize(&cleaned))
            .into_iter()
            .map(|r| r.id)
            .collect();
        let result = parse(&case.text, &case.library, case.root.as_deref());
        for q in &result.query_statements {
            let Some(index) = ids.iter().rposition(|id| *id == q.statement_id) else {
                continue;
            };
            let n = ids.len();
            let f = if n > 1 {
                index as f64 / (n - 1) as f64
            } else {
                1.0
            };
            positions.push(f);
            let bucket = if n == 1 {
                "the only statement"
            } else if index + 1 == n {
                "e) the last statement"
            } else if index == 0 {
                "a) the first statement"
            } else if f < 1.0 / 3.0 {
                "b) first third"
            } else if f < 2.0 / 3.0 {
                "c) middle third"
            } else {
                "d) last third (not last)"
            };
            *buckets.entry(bucket).or_default() += 1;
            for dep in q.deps.iter().flatten() {
                let order = match ids.iter().position(|id| id == dep) {
                    None => "never declared (reads null)",
                    Some(d) if d < index => "declared before the query",
                    Some(_) => "declared after the query",
                };
                *state_order.entry(order).or_default() += 1;
            }
        }
    }
    let _ = writeln!(w, "\n## Where queries sit in their programs\n");
    let _ = writeln!(
        w,
        "| Position of the query's statement | Queries |\n|---|---:|"
    );
    for (b, n) in &buckets {
        let _ = writeln!(w, "| {b} | {n} |");
    }
    let _ = writeln!(
        w,
        "\n| A state variable a query reads (query, variable pairs) | Pairs |\n|---|---:|"
    );
    for (o, n) in &state_order {
        let _ = writeln!(w, "| {o} | {n} |");
    }
    let _ = writeln!(
        w,
        "\nPosition as a fraction of the statements (0 first, 1 last): min, q1, median, q3, max\n\n| min | q1 | median | q3 | max |\n|---:|---:|---:|---:|---:|\n| {} |",
        cells(positions)
    );

    summary(w, &cases, &outcomes, &rates);

    // Per rule and chunking.
    let chunking_order = [
        "1", "2", "3", "4", "7", "16", "random-1", "random-2", "recorded", "resync-1", "resync-2",
    ];
    for rule in RULES {
        let _ = writeln!(w, "\n## Rule: {}\n", rule.label());
        let _ = writeln!(
            w,
            "Per query, at the step its statement first finished, compared with its key at settle.\n"
        );
        let _ = writeln!(
            w,
            "| Chunking | Streams | Queries | Same key | Different key | Waits on a query | Waits, called early | Finishes only at settle | Refused | Not in final | Calls at settle today | Early calls | Useful | Repeated or wasted | Still needed at settle | Refused registrations |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
        );
        for chunking in chunking_order {
            let rows: Vec<&StreamOutcome> =
                outcomes.iter().filter(|o| o.chunking == chunking).collect();
            if rows.is_empty() {
                continue;
            }
            let mut classes: BTreeMap<Class, usize> = BTreeMap::new();
            let (mut base, mut early, mut useful, mut settle, mut refused) = (0, 0, 0, 0, 0);
            for o in &rows {
                let r = &o.rules[&rule];
                for q in &r.queries {
                    *classes.entry(q.class).or_default() += 1;
                }
                base += r.baseline;
                early += r.early;
                useful += r.useful;
                settle += r.at_settle;
                refused += r.refused_steps;
            }
            let c = |k| classes.get(&k).copied().unwrap_or(0);
            let total: usize = classes.values().sum();
            let _ = writeln!(
                w,
                "| {chunking} | {} | {total} | {} | {} | {} | {} | {} | {} | {} | {base} | {early} | {useful} | {} | {settle} | {refused} |",
                rows.len(),
                c(Class::SameKey),
                c(Class::DifferentKey),
                c(Class::Waits),
                c(Class::WaitsCalledEarly),
                c(Class::OnlyAtSettle),
                c(Class::Refused),
                c(Class::NotInFinal),
                early - useful,
            );
        }
        let _ = writeln!(
            w,
            "\nHow much of the stream was still to come when a useful early call went out (per useful call):\n"
        );
        let _ = writeln!(
            w,
            "| Chunking | Useful calls | Fraction: min | q1 | median | q3 | max | Bytes: min | q1 | median | q3 | max | Chunks: min | q1 | median | q3 | max |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
        );
        for chunking in chunking_order {
            let saved: Vec<Saved> = outcomes
                .iter()
                .filter(|o| o.chunking == chunking)
                .flat_map(|o| o.rules[&rule].saved.iter().copied())
                .collect();
            if outcomes.iter().all(|o| o.chunking != chunking) {
                continue;
            }
            let _ = writeln!(
                w,
                "| {chunking} | {} | {} | {} | {} |",
                saved.len(),
                cells(saved.iter().map(|s| s.fraction).collect()),
                cells(saved.iter().map(|s| s.bytes as f64).collect()),
                cells(saved.iter().map(|s| s.chunks as f64).collect())
            );
        }
        let _ = writeln!(
            w,
            "\nThe first tool call of each stream: how much earlier (zero where no early call was useful):\n"
        );
        let _ = writeln!(
            w,
            "| Chunking | Streams | With an earlier first call | Fraction: min | q1 | median | q3 | max | Bytes: min | q1 | median | q3 | max |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
        );
        for chunking in chunking_order {
            let rows: Vec<&StreamOutcome> = outcomes
                .iter()
                .filter(|o| o.chunking == chunking && o.rules[&rule].baseline > 0)
                .collect();
            if rows.is_empty() {
                continue;
            }
            let firsts: Vec<Saved> = rows
                .iter()
                .map(|o| o.rules[&rule].first.unwrap_or_default())
                .collect();
            let _ = writeln!(
                w,
                "| {chunking} | {} | {} | {} | {} |",
                rows.len(),
                firsts.iter().filter(|s| s.bytes > 0).count(),
                cells(firsts.iter().map(|s| s.fraction).collect()),
                cells(firsts.iter().map(|s| s.bytes as f64).collect())
            );
        }
        // Reasons for different keys, every chunking.
        let mut tags: BTreeMap<(Class, &str), usize> = BTreeMap::new();
        let mut changed_back = 0;
        let mut changes: BTreeMap<Class, usize> = BTreeMap::new();
        for o in &outcomes {
            for q in &o.rules[&rule].queries {
                if q.class == Class::SameKey && q.changes > 0 {
                    changed_back += 1;
                }
                *changes.entry(q.class).or_default() += q.changes;
                if matches!(q.class, Class::SameKey | Class::DifferentKey) {
                    for t in &q.tags {
                        *tags.entry((q.class, t)).or_default() += 1;
                    }
                }
            }
        }
        let _ = writeln!(
            w,
            "\nWhat the early-startable queries read (all chunkings; a query can have several):\n\n| Class | Reads | Queries |\n|---|---|---:|"
        );
        for ((class, tag), n) in &tags {
            let _ = writeln!(w, "| {} | {tag} | {n} |", class.label());
        }
        let _ = writeln!(
            w,
            "\nSame key at settle but changed in between (each change is a call): {changed_back}."
        );
        let _ = writeln!(
            w,
            "Key changes after a query first finished, before settle (each is one more call while the earlier is unanswered), by class: {}.",
            changes
                .iter()
                .map(|(c, n)| format!("{} {n}", c.label()))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Per source group, inputs-complete rule, chunking 16 and random-1.
    let _ = writeln!(
        w,
        "\n## Inputs-complete rule by source, chunkings 16 and random-1\n"
    );
    let _ = writeln!(
        w,
        "| Source | Chunking | Queries | Same key | Different key | Waits | Only at settle | Median fraction saved (same key) | Median bytes saved (same key) |\n|---|---|---:|---:|---:|---:|---:|---:|---:|"
    );
    for g in groups.keys() {
        for chunking in ["16", "random-1"] {
            let mut classes: BTreeMap<Class, usize> = BTreeMap::new();
            let mut fractions = Vec::new();
            let mut bytes = Vec::new();
            for o in outcomes
                .iter()
                .filter(|o| o.chunking == chunking && group(&cases[o.case].name) == *g)
            {
                for q in &o.rules[&Rule::InputsComplete].queries {
                    *classes.entry(q.class).or_default() += 1;
                    if let Some(s) = q.saved {
                        fractions.push(s.fraction);
                        bytes.push(s.bytes as f64);
                    }
                }
            }
            let c = |k| classes.get(&k).copied().unwrap_or(0);
            let shown = |v: Vec<f64>| {
                if v.is_empty() {
                    "–".to_owned()
                } else {
                    fmt(median(v))
                }
            };
            let _ = writeln!(
                w,
                "| {g} | {chunking} | {} | {} | {} | {} | {} | {} | {} |",
                classes.values().sum::<usize>(),
                c(Class::SameKey),
                c(Class::DifferentKey),
                c(Class::Waits) + c(Class::WaitsCalledEarly),
                c(Class::OnlyAtSettle),
                shown(fractions),
                shown(bytes)
            );
        }
    }

    // Half-written queries under the naive rule.
    let mut half_streams = 0;
    let mut half_keys = 0;
    let mut half_wrong = 0;
    let mut examples = Vec::new();
    for o in &outcomes {
        if !o.half_written.is_empty() {
            half_streams += 1;
        }
        for h in &o.half_written {
            half_keys += 1;
            if h.wrong {
                half_wrong += 1;
                if examples.len() < 12 && (o.chunking == "16" || o.chunking == "random-1") {
                    examples.push(format!(
                        "- `{}` ({}), query `{}`: last line so far `{}` gives `{}`",
                        cases[o.case].name,
                        o.chunking,
                        h.id,
                        h.tail.replace('`', "'"),
                        h.key.replace('`', "'")
                    ));
                }
            }
        }
    }
    let _ = writeln!(
        w,
        "\n## Half-written queries (visible in the parse result, not past the watermark)\n"
    );
    let _ = writeln!(
        w,
        "{half_streams} of {} streams show a query before its statement has finished; {half_keys} distinct (query, key) pairs are seen that way, of which {half_wrong} differ from the query's key at settle (a wrong call under the naive rule).\n",
        outcomes.len()
    );
    for e in &examples {
        let _ = writeln!(w, "{e}");
    }
    let differs: usize = outcomes.iter().map(|o| o.begun_differs).sum();
    let _ = writeln!(
        w,
        "\nSteps where the watermark and \"the next statement has begun\" disagree: {differs}."
    );

    // Examples for the report.
    let _ = writeln!(w, "\n## Examples (watermark rule, chunking 16)\n");
    let mut best: Option<(&StreamOutcome, &QueryOutcome)> = None;
    let mut repeated: Vec<(&StreamOutcome, &QueryOutcome)> = Vec::new();
    let mut late: Vec<(&StreamOutcome, &QueryOutcome)> = Vec::new();
    for o in outcomes.iter().filter(|o| o.chunking == "16") {
        for q in &o.rules[&Rule::Watermark].queries {
            match q.class {
                Class::SameKey
                    if best.is_none_or(|(bo, bq)| {
                        q.saved.unwrap().bytes > bq.saved.unwrap().bytes
                            || (q.saved.unwrap().bytes == bq.saved.unwrap().bytes
                                && o.total_bytes < bo.total_bytes)
                    }) =>
                {
                    best = Some((o, q));
                }
                Class::DifferentKey | Class::WaitsCalledEarly => repeated.push((o, q)),
                Class::Waits | Class::OnlyAtSettle => late.push((o, q)),
                _ => {}
            }
        }
    }
    let show = |w: &mut String, title: &str, (o, q): (&StreamOutcome, &QueryOutcome)| {
        let case = &cases[o.case];
        let _ = writeln!(
            w,
            "### {title}: `{}`, query `{}` ({})\n\n```\n{}\n```\n",
            case.name,
            q.id,
            q.class.label(),
            case.text.trim_end()
        );
        if let Some(at) = &q.at {
            let _ = writeln!(
                w,
                "Finished after {} of {} bytes; text so far ends `{}`.",
                at.len(),
                o.total_bytes,
                at.lines().last().unwrap_or("").replace('`', "'")
            );
        }
        let _ = writeln!(
            w,
            "Key when it finished: `{}`; at settle: `{}`; tags: {:?}; key changes before settle: {}.\n",
            q.early
                .as_ref()
                .map_or("–".into(), |k| format!("{k:?}").replace('`', "'")),
            format!("{:?}", q.final_key).replace('`', "'"),
            q.tags,
            q.changes
        );
    };
    if let Some(b) = best {
        show(w, "Gains most", b);
    }
    repeated.sort_by_key(|(o, _)| o.total_bytes);
    for r in repeated
        .iter()
        .filter(|(_, q)| q.class == Class::DifferentKey)
        .take(2)
    {
        show(w, "Repeated", *r);
    }
    for r in repeated
        .iter()
        .filter(|(_, q)| q.class == Class::WaitsCalledEarly)
        .take(2)
    {
        show(w, "Wasted: called before the query it waits on arrived", *r);
    }
    late.sort_by_key(|(o, _)| o.total_bytes);
    for l in late.iter().filter(|(_, q)| q.class == Class::Waits).take(1) {
        show(w, "Cannot start early (waits)", *l);
    }
    for l in late
        .iter()
        .filter(|(_, q)| q.class == Class::OnlyAtSettle)
        .take(1)
    {
        show(w, "Cannot start early (last statement)", *l);
    }
    let _ = writeln!(
        w,
        "\nStreams measured: {}; mean stream length {} bytes, {} chunks.",
        outcomes.len(),
        outcomes.iter().map(|o| o.total_bytes).sum::<usize>() / outcomes.len().max(1),
        outcomes.iter().map(|o| o.total_chunks).sum::<usize>() / outcomes.len().max(1)
    );
    println!("{out}");
}
