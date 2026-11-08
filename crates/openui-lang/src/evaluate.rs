//! State defaults evaluated while parsing, as 1.1's parser does with the
//! runtime evaluator: built-in functions, operators, member and index access,
//! ternaries and `@Each`, over data, with JavaScript's semantics. Not the
//! renderer's runtime: references to statements and queries read as null, and
//! a default that calls anything but a built-in function is null.
//!
//! Values are JavaScript's: `undefined` apart from `null`, arrays and objects
//! compared by identity, `==` with its coercions, strings indexed by UTF-16
//! unit. Where the core can't follow the reference (a property of a
//! prototype, string order outside ASCII), CONFORMANCE.md lists it.
//!
//! Evaluating keeps to the parse's limits (see [`EvalLimits`]): a default
//! that would build past what's left of the result-size limit, take more
//! steps than the work limit, or nest too deep, is null instead.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ast::{Ast, Field, fields, json_shaped_ast};
use crate::builtins::builtin_function_params;
use crate::js::{self, JsObject, OBJECT_PROTOTYPE_KEYS, array_index, number_to_string};
use crate::result::{Value, json_number_value};
use crate::validation::{element_parts, is_ast_node};

/// A JavaScript value, as the evaluator sees one.
#[derive(Clone, Debug)]
pub(crate) enum Js {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    /// A string, with lone surrogates as the core carries them (see `js`).
    Str(Arc<str>),
    Arr(Arc<Vec<Js>>),
    Obj(Arc<JsObject<Js>>),
    /// A runtime expression read as a value: the plain object its JSON is
    /// (`ast::fields`), its properties read from the node as they're asked
    /// for. It's built whole only if it reaches the result or a literal, so
    /// an `@Each` template assigning a large expression costs nothing per
    /// item to read it.
    Ast(Arc<Ast>),
    /// A built-in method or constructor read off a value (`[].map`). It's
    /// the property of the first prototype in the chain that has it, its
    /// `owner`, so `(1).toString` (`Number.prototype`'s) isn't `{}.toString`
    /// (`Object.prototype`'s), while `(1).hasOwnProperty` is.
    Function {
        name: &'static str,
        constructor: bool,
        owner: Kind,
    },
    /// A prototype, read through `__proto__`.
    Proto(Kind),
}

/// Which built-in prototype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Object,
    Array,
    String,
    Number,
    Boolean,
}

/// What a JavaScript object has from `String.prototype` and so on, as Bun has
/// it (`length` apart).
const STRING_METHODS: [&str; 50] = [
    "anchor",
    "at",
    "big",
    "blink",
    "bold",
    "charAt",
    "charCodeAt",
    "codePointAt",
    "concat",
    "endsWith",
    "fixed",
    "fontcolor",
    "fontsize",
    "includes",
    "indexOf",
    "isWellFormed",
    "italics",
    "lastIndexOf",
    "link",
    "localeCompare",
    "match",
    "matchAll",
    "normalize",
    "padEnd",
    "padStart",
    "repeat",
    "replace",
    "replaceAll",
    "search",
    "slice",
    "small",
    "split",
    "startsWith",
    "strike",
    "sub",
    "substr",
    "substring",
    "sup",
    "toLocaleLowerCase",
    "toLocaleUpperCase",
    "toLowerCase",
    "toString",
    "toUpperCase",
    "toWellFormed",
    "trim",
    "trimEnd",
    "trimLeft",
    "trimRight",
    "trimStart",
    "valueOf",
];
const ARRAY_METHODS: [&str; 38] = [
    "at",
    "concat",
    "copyWithin",
    "entries",
    "every",
    "fill",
    "filter",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "map",
    "pop",
    "push",
    "reduce",
    "reduceRight",
    "reverse",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "toLocaleString",
    "toReversed",
    "toSorted",
    "toSpliced",
    "toString",
    "unshift",
    "values",
    "with",
];
const NUMBER_METHODS: [&str; 6] = [
    "toExponential",
    "toFixed",
    "toLocaleString",
    "toPrecision",
    "toString",
    "valueOf",
];
const BOOLEAN_METHODS: [&str; 2] = ["toString", "valueOf"];

// ─── Limits ─────────────────────────────────────────────────────────────────

/// How many of the evaluator's levels one level of the depth limit allows:
/// as deep as a parse result's JSON nests, so an evaluated default reads back
/// and serializes like any result.
const VALUE_LEVELS_PER_LEVEL: usize = 3;

/// What evaluating defaults may still build and do, and whether it went past
/// either.
///
/// A default can build far more than the program holds. One listing an
/// earlier one twice doubles with every line, and joining that into a string,
/// adding a string to itself or `@Each` copying its items doubles it again;
/// defaults reading defaults also nest without bound. It can also do far more
/// than it builds: `@Each` over a long list whose template reads a large
/// expression, or sorting a long list, builds little but takes long. The
/// reference goes on until the engine runs out of memory or stack, or time.
///
/// So the core keeps all of a parse's evaluating to three limits:
/// - **result size:** what is left of the result-size limit (its room),
///   for strings, literals and the result;
/// - **work:** every step it takes, no more than the work limit
///   ([`crate::Limits::work_for`]). A step is an expression evaluated, an item
///   sorted, filtered, summed or copied into an `@Each` template, a node of a
///   value read, or a byte built;
/// - **depth:** its walks, [`VALUE_LEVELS_PER_LEVEL`] per level of the depth
///   limit.
///
/// A default that goes past any of them is null, and stops there: what it
/// would have done after takes no more work. After room or work runs out,
/// every expression default is null too.
///
/// They reach the evaluator through a thread-local, set for one parse's
/// defaults at a time: evaluating is synchronous, and passing them through
/// every coercion would touch nearly every function here.
struct EvalLimits {
    /// About how many bytes may still be built.
    room: Cell<usize>,
    /// How many steps are left.
    work: Cell<usize>,
    /// Levels open across the walks.
    depth: Cell<usize>,
    /// How many may be open: [`VALUE_LEVELS_PER_LEVEL`] per level of the
    /// parse's depth limit.
    max_depth: Cell<usize>,
    /// The default being evaluated ran out of room or work, or of depth.
    size_limited: Cell<bool>,
    depth_limited: Cell<bool>,
    /// What the default being evaluated ran out of, if anything: nothing more
    /// is drawn for it after.
    cut: Cell<Option<Cut>>,
}

/// What a default ran out of.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cut {
    Room,
    Work,
    Depth,
}

impl EvalLimits {
    fn cut_by(&self, cut: Cut) {
        match cut {
            Cut::Depth => self.depth_limited.set(true),
            Cut::Room | Cut::Work => self.size_limited.set(true),
        }
        if self.cut.get().is_none() {
            self.cut.set(Some(cut));
        }
    }
}

/// Keeps the owning node alive so its address cannot be reused while cached.
struct PropertyRead {
    owner: Arc<Ast>,
    value: Js,
}

thread_local! {
    static LIMITS: EvalLimits = const {
        EvalLimits {
            room: Cell::new(usize::MAX),
            work: Cell::new(usize::MAX),
            depth: Cell::new(0),
            max_depth: Cell::new(0),
            size_limited: Cell::new(false),
            depth_limited: Cell::new(false),
            cut: Cell::new(None),
        }
    };
    /// Stable property identities, including arrays and their entry pairs.
    static PROPERTIES: RefCell<HashMap<(usize, String), PropertyRead>> = RefCell::new(HashMap::new());
    /// The keys put in `PROPERTIES`, in order, so a default's own can be told
    /// apart (see [`Reuse`]).
    static PROPERTIES_LOG: RefCell<Vec<(usize, String)>> = const { RefCell::new(Vec::new()) };
    /// Elements already read as values, by the address of their shared node,
    /// which the entry keeps alive: an element read many times is converted
    /// once, and every read gets the same object, as the reference's do.
    static READ: RefCell<HashMap<usize, (Value, Js)>> = RefCell::new(HashMap::new());
    /// The addresses put in `READ`, in order, so a default's own can be told
    /// apart (see [`Reuse`]).
    static READ_LOG: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// Elements and runtime expressions of a stream's earlier push standing
    /// for this push's, by address (see [`Reuse`]): an object read from one
    /// is the object read from the other, as it would be had the default
    /// been evaluated now.
    static SAME_AS: RefCell<Aliases> = RefCell::new(Aliases::default());
}

/// The address this push knows an element or runtime expression by: its own,
/// or that of the one it stands for (see [`SAME_AS`]).
fn this_push(address: usize) -> usize {
    SAME_AS.with(|s| s.borrow().now.get(&address).copied().unwrap_or(address))
}

/// Take `cost` from room or work: `false`, and the default is cut, once it's
/// spent, or once the default was cut anyhow.
fn draw(l: &EvalLimits, which: Cut, cost: usize) -> bool {
    if l.cut.get().is_some() {
        return false;
    }
    let counter = match which {
        Cut::Room => &l.room,
        Cut::Work | Cut::Depth => &l.work,
    };
    match counter.get().checked_sub(cost) {
        Some(left) => {
            counter.set(left);
            true
        }
        None => {
            counter.set(0);
            l.cut_by(which);
            false
        }
    }
}

/// Build `bytes`: they cost room and work alike.
fn spend(bytes: usize) -> bool {
    LIMITS.with(|l| draw(l, Cut::Work, bytes) && draw(l, Cut::Room, bytes))
}

/// Take `steps` steps of work, building nothing that stays.
fn step(steps: usize) -> bool {
    LIMITS.with(|l| draw(l, Cut::Work, steps))
}

/// Stop once work runs out or a cut occurs; otherwise run the next conversion.
/// The continuation must draw positive work: exact-work reuse relies on every
/// passed check costing work before it can return (asserted in debug builds).
fn out_of_work(next: impl FnOnce() -> Js) -> Js {
    let work = LIMITS.with(|l| l.work.get());
    if LIMITS.with(|l| l.work.get() == 0 || l.cut.get().is_some()) {
        ran_out_of_work();
        return Js::Null;
    }
    let value = next();
    debug_assert!(
        LIMITS.with(|l| l.work.get() < work),
        "out_of_work continuation must draw positive work"
    );
    value
}

/// Out of work where a step was due: the default is cut.
fn ran_out_of_work() {
    LIMITS.with(|l| l.cut_by(Cut::Work));
}

/// A level open in one of the walks, closed when dropped.
struct Level;

impl Level {
    /// `None`, and the default is cut, past the evaluator's depth limit; or
    /// `None` once it was cut anyhow.
    fn enter() -> Option<Level> {
        LIMITS.with(|l| {
            if l.cut.get().is_some() {
                return None;
            }
            let depth = l.depth.get();
            if depth >= l.max_depth.get() {
                l.cut_by(Cut::Depth);
                return None;
            }
            l.depth.set(depth + 1);
            Some(Level)
        })
    }
}

impl Drop for Level {
    fn drop(&mut self) {
        LIMITS.with(|l| l.depth.set(l.depth.get() - 1));
    }
}

/// About what a value's own JSON costs, its items' and entries' apart.
fn json_cost(js: &Js) -> usize {
    match js {
        Js::Str(s) => s.len() + 2,
        Js::Arr(items) => items.len() + 2,
        Js::Obj(o) => o.iter().map(|(key, _)| key.len() + 4).sum::<usize>() + 2,
        Js::Num(n) => number_cost(*n),
        _ => 4,
    }
}

/// About how long a number's JSON is: an integer's digits, else the longest.
fn number_cost(n: f64) -> usize {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        let digits = if n == 0.0 {
            1
        } else {
            n.abs().log10() as usize + 1
        };
        digits + usize::from(n < 0.0)
    } else {
        24
    }
}

// ─── State defaults ─────────────────────────────────────────────────────────

/// A parse's state defaults, evaluated.
pub(crate) struct Evaluated {
    pub state: JsObject<Value>,
    /// Those that came out `undefined`: left out of the result, as JSON
    /// leaves them, but declared all the same.
    pub undefined: HashSet<String>,
    /// One ran out of room or work, or went too deep, and is null.
    pub size_limited: bool,
    pub depth_limited: bool,
}

/// 1.1 evaluates expression defaults (`$total = 1 + 2`) once, in statement
/// order, each reading the defaults declared above it, building no more than
/// about `room` bytes and taking no more than `work` steps in all, nesting no
/// deeper than [`VALUE_LEVELS_PER_LEVEL`] per level of `depth` (see
/// [`EvalLimits`]).
///
/// A stream passes its [`Reuse`], and gets the same result doing less.
pub(crate) fn state_defaults(
    state: JsObject<Value>,
    room: usize,
    depth: usize,
    work: usize,
    reuse: Option<&mut Reuse>,
) -> Evaluated {
    let mut out = Evaluated {
        state: JsObject::new(),
        undefined: HashSet::new(),
        size_limited: false,
        depth_limited: false,
    };
    if !state.values().any(has_runtime) {
        out.state = state;
        return out;
    }
    let max_depth = depth.saturating_mul(VALUE_LEVELS_PER_LEVEL);
    LIMITS.with(|l| {
        l.room.set(room);
        l.work.set(work);
        l.depth.set(0);
        l.max_depth.set(max_depth);
    });
    let mut scratch = Reuse::default();
    let reuse = reuse.unwrap_or(&mut scratch);
    // What the last push found holds while every default before it is as it
    // was, under the same depth limit.
    let mut in_step = reuse.max_depth == max_depth;
    reuse.max_depth = max_depth;
    let most = Amounts {
        work: reuse.most.work.max(work),
        room: reuse.most.room.max(room),
    };
    let mut earlier = std::mem::take(&mut reuse.records).into_iter();
    // Every earlier value compared at this push lives until its pairs in
    // `SAME_AS` are cleared, at the end: they're keyed by address, so a value
    // freed before then could have its address taken by a new node, which
    // would read as the one it was paired with.
    let mut compared: Vec<Value> = Vec::new();
    let mut declared = Declared::default();
    for (id, value) in state.iter() {
        let before = match earlier.next() {
            Some(record) if in_step && record.id == *id => {
                if same(&record.value, value) {
                    Some(record)
                } else {
                    compared.push(record.value);
                    None
                }
            }
            _ => None,
        };
        if !has_runtime(value) {
            out.state.set(id.clone(), value.clone());
            declared.plain.define(id.clone(), value.clone());
            in_step &= before.is_some();
            if let Some(record) = before {
                compared.push(record.value);
            }
            reuse.records.push(Record {
                id: id.clone(),
                value: value.clone(),
                outcome: Outcome::Plain,
            });
            continue;
        }
        let left = LIMITS.with(|l| Amounts {
            work: l.work.get(),
            room: l.room.get(),
        });
        let decision = match &before {
            Some(record) => record.decide(left, most),
            None => Decision::Evaluate(left),
        };
        // Out of step (an earlier default changed, or the depth limit did),
        // the later records read a different prefix: `in_step` never turns
        // back on, so the guard on `before` uses none of them again at this
        // push. Not even after a default runs out and every expression default
        // after it is null: a record saying so may still hold values computed
        // from the old defaults.
        let record = match (decision, before) {
            (Decision::Take, Some(record)) => {
                record.take(id, &mut declared, &mut out);
                record.moved_to(value, &mut compared)
            }
            (Decision::RunsOut, Some(record)) => {
                ran_out(id, &mut declared, &mut out);
                record.moved_to(value, &mut compared)
            }
            (Decision::Evaluate(given), before) => {
                // Only RanOut records choose Evaluate; every such record declared null.
                let was_null = before.is_some();
                if let Some(record) = before {
                    compared.push(record.value);
                }
                let (record, ended) =
                    evaluate_default((id, value), &mut declared, &mut out, left, given);
                // The defaults after it, as the last push found them, read it
                // as it was: they hold if it's null now as then.
                in_step &= was_null && ended != Ended::Whole;
                record
            }
            (_, None) => unreachable!("a default is taken or found out by its record only"),
        };
        reuse.records.push(record);
    }
    LIMITS.with(|l| {
        l.room.set(usize::MAX);
        l.work.set(usize::MAX);
    });
    READ.with(|r| r.borrow_mut().clear());
    READ_LOG.with(|l| l.borrow_mut().clear());
    PROPERTIES.with(|r| r.borrow_mut().clear());
    PROPERTIES_LOG.with(|l| l.borrow_mut().clear());
    SAME_AS.with(|s| *s.borrow_mut() = Aliases::default());
    drop(compared);
    drop(earlier);
    out
}

/// Work and room together: what a default took, had, or has left.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Amounts {
    work: usize,
    room: usize,
}

impl Amounts {
    /// Neither is less than in `other`.
    fn covers(self, other: Amounts) -> bool {
        self.work >= other.work && self.room >= other.room
    }

    fn less(self, taken: Amounts) -> Amounts {
        Amounts {
            work: self.work - taken.work,
            room: self.room - taken.room,
        }
    }
}

/// How one default came out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ended {
    Whole,
    /// Past the depth limit: null, the rest going on.
    TooDeep,
    /// Out of room or work: null, as is every expression default after it.
    RanOut,
}

/// Default `id` ran out of room or work: null, and no room or work is left.
fn ran_out(id: &str, declared: &mut Declared, out: &mut Evaluated) {
    LIMITS.with(|l| {
        l.work.set(0);
        l.room.set(0);
    });
    out.size_limited = true;
    out.state.set(id.to_owned(), Value::Null);
    declared.evaluated.define(id.to_owned(), Js::Null);
}

/// Default `id` went past the depth limit: null.
fn too_deep(id: &str, declared: &mut Declared, out: &mut Evaluated) {
    out.depth_limited = true;
    out.state.set(id.to_owned(), Value::Null);
    declared.evaluated.define(id.to_owned(), Js::Null);
}

/// The work and room left set to `left`.
fn set_left(left: Amounts) {
    LIMITS.with(|l| {
        l.work.set(left.work);
        l.room.set(left.room);
    });
}

/// What one evaluation of a default found.
struct Run {
    js: Js,
    value: Option<Value>,
    cut: Option<Cut>,
    left: Amounts,
}

/// Evaluate one default with `given` work and room.
fn run(value: &Value, declared: &Declared, given: Amounts) -> Run {
    set_left(given);
    LIMITS.with(|l| {
        l.size_limited.set(false);
        l.depth_limited.set(false);
        l.cut.set(None);
    });
    let js = state_default(value, declared);
    let converted = to_value(&js);
    LIMITS.with(|l| Run {
        js,
        value: converted,
        cut: l.cut.take(),
        left: Amounts {
            work: l.work.get(),
            room: l.room.get(),
        },
    })
}

/// Evaluate default `id` with `left` work and room to go, as a one-shot parse
/// would, into `declared` and `out`, and say what it found and how it came
/// out.
///
/// Given more (`given`: a stream found it out of work or room before, with
/// these inputs), it's evaluated with that instead, to learn more than this
/// push needs. The amounts are all that differs, and a run takes the same
/// course whatever they are until one runs out. So where it went whole or too
/// deep within `left`, it's the run `left` gives; anywhere else it runs out.
fn evaluate_default(
    (id, value): (&String, &Value),
    declared: &mut Declared,
    out: &mut Evaluated,
    left: Amounts,
    given: Amounts,
) -> (Record, Ended) {
    let marks = FirstReads::mark(declared);
    // No exact-work retry is needed: neither out_of_work caller can produce
    // a value without subsequently drawing positive work. Thus extra work
    // cannot turn a zero-work early stop into a successful run of this price.
    let ran = run(value, declared, given);
    let record = Record::of(id, value, &ran, given, FirstReads::since(marks, declared));
    let took = given.less(ran.left);
    let ended = match ran.cut {
        None if left.covers(took) => Ended::Whole,
        Some(Cut::Depth) if left.covers(took) => Ended::TooDeep,
        _ => Ended::RanOut,
    };
    match ended {
        Ended::RanOut => ran_out(id, declared, out),
        Ended::TooDeep => {
            set_left(left.less(took));
            too_deep(id, declared, out);
        }
        Ended::Whole => {
            set_left(left.less(took));
            match ran.value {
                Some(v) => out.state.set(id.clone(), v),
                None => {
                    out.undefined.insert(id.clone());
                }
            }
            declared.evaluated.define(id.clone(), ran.js);
        }
    }
    (record, ended)
}

// ─── Reuse across a stream's pushes ─────────────────────────────────────────

/// What evaluating a stream's defaults found at its last push, so the next
/// push needn't evaluate again what nothing changed under.
///
/// A default's run takes the same course whatever work and room it's given
/// until one runs out, and it stops where one does. So it comes out whole, or
/// too deep, having taken some work and room, or runs out of room or work
/// (and then so does every expression default after it); and given the same
/// value and reading the same defaults, it comes out the same way given
/// anything that covers what it took, and runs out given anything that
/// doesn't.
///
/// Defaults are taken in order. While each is as it was (the same value, its
/// elements and runtime expressions paired one to one with the earlier
/// push's, see [`same`]), one whole or too deep is taken as it was where this
/// push leaves it what it took, and found out where it doesn't; one that ran
/// out is found out again given no more than it had. Whatever it read first
/// when it was evaluated (elements converted, properties of runtime
/// expressions, plain defaults) is read again by the defaults after it, as
/// the same objects, whether it came out whole or too deep. So the result is
/// the one evaluating them all would give, as a one-shot parse does.
///
/// A default that ran out and is then given more is evaluated with twice
/// what it ran out of last time (no more than any push of this stream can
/// give): a default a stream keeps finding out as its limits grow costs about
/// as much in all as evaluating it once at the end.
#[derive(Default)]
pub(crate) struct Reuse {
    records: Vec<Record>,
    /// The evaluator's depth limit they were found under.
    max_depth: usize,
    /// The most work and room any push of the stream can give.
    most: Amounts,
}

impl Reuse {
    /// For a stream under `limits`: no push can give more work or room than
    /// its whole program allows.
    pub(crate) fn for_limits(limits: &crate::Limits) -> Reuse {
        let program = limits.program_size();
        Reuse {
            most: Amounts {
                work: limits.work_for(program),
                room: limits.result_size_for(program),
            },
            ..Reuse::default()
        }
    }
}

/// One default as the last push found it.
struct Record {
    id: String,
    value: Value,
    outcome: Outcome,
}

enum Outcome {
    /// Nothing to evaluate.
    Plain,
    /// Whole, or too deep (`gave` is `None`), having taken `took`, with what
    /// it read first.
    Ran {
        took: Amounts,
        first: FirstReads,
        gave: Option<Gave>,
        /// Neither what it gave nor what it read first holds an element or a
        /// runtime expression of its value (see [`Record::moved_to`]).
        unpaired: bool,
    },
    /// Out of room or work (`of`), having had `had`.
    RanOut { had: Amounts, of: Cut },
}

/// What a whole default gave.
struct Gave {
    js: Js,
    value: Option<Value>,
}

enum Decision {
    /// As it was.
    Take,
    /// Runs out, without evaluating it.
    RunsOut,
    /// Evaluated given this (see [`evaluate_default`]).
    Evaluate(Amounts),
}

impl Record {
    /// What an unchanged default does with `left` to go, in a stream whose
    /// pushes give no more than `most`.
    fn decide(&self, left: Amounts, most: Amounts) -> Decision {
        match self.outcome {
            Outcome::Plain => Decision::Evaluate(left),
            Outcome::Ran { took, .. } => {
                // With what it took or more, the same run; with less, it
                // runs out. A successful run cannot stop early at zero work:
                // both out_of_work callers subsequently draw positive work.
                if !left.covers(took) {
                    Decision::RunsOut
                } else {
                    Decision::Take
                }
            }
            Outcome::RanOut { had, of: Cut::Work } => {
                if left.work <= had.work {
                    Decision::RunsOut
                } else {
                    Decision::Evaluate(Amounts {
                        work: left.work.max(had.work.saturating_mul(2).min(most.work)),
                        room: left.room,
                    })
                }
            }
            Outcome::RanOut { had, of: Cut::Room } => {
                if left.room <= had.room {
                    Decision::RunsOut
                } else {
                    Decision::Evaluate(Amounts {
                        work: left.work,
                        room: left.room.max(had.room.saturating_mul(2).min(most.room)),
                    })
                }
            }
            Outcome::RanOut { of: Cut::Depth, .. } => Decision::Evaluate(left),
        }
    }

    /// This record, found the same as `value` at this push (see [`same`]).
    /// Where nothing it kept holds an element or runtime expression of the
    /// earlier value, it moves to this push's value, so the next push finds
    /// it the same by its pointer alone; the earlier value goes to `compared`
    /// (see [`state_defaults`]). Otherwise it stays with the earlier value,
    /// whose elements and expressions what it kept stands for.
    fn moved_to(mut self, value: &Value, compared: &mut Vec<Value>) -> Record {
        let unpaired = match &self.outcome {
            Outcome::Ran { unpaired, .. } => *unpaired,
            _ => true,
        };
        if unpaired {
            compared.push(std::mem::replace(&mut self.value, value.clone()));
        }
        self
    }

    /// Take a default as it was, whole or too deep.
    fn take(&self, id: &str, declared: &mut Declared, out: &mut Evaluated) {
        let Outcome::Ran {
            took, first, gave, ..
        } = &self.outcome
        else {
            unreachable!("only a default that ran is taken");
        };
        LIMITS.with(|l| {
            l.work.set(l.work.get() - took.work);
            l.room.set(l.room.get() - took.room);
        });
        first.replay(declared);
        let Some(gave) = gave else {
            too_deep(id, declared, out);
            return;
        };
        match &gave.value {
            Some(v) => out.state.set(id.to_owned(), v.clone()),
            None => {
                out.undefined.insert(id.to_owned());
            }
        }
        declared.evaluated.define(id.to_owned(), gave.js.clone());
    }

    /// What a run of `value` given `had` found.
    fn of(id: &str, value: &Value, ran: &Run, had: Amounts, first: FirstReads) -> Record {
        let ran_whole = |gave: Option<Gave>| {
            let unpaired = first.is_empty_of_shared()
                && gave.as_ref().is_none_or(|gave| !holds_expression(&gave.js));
            Outcome::Ran {
                took: had.less(ran.left),
                first,
                gave,
                unpaired,
            }
        };
        let outcome = match ran.cut {
            None => ran_whole(Some(Gave {
                js: ran.js.clone(),
                value: ran.value.clone(),
            })),
            Some(Cut::Depth) => ran_whole(None),
            Some(of) => Outcome::RanOut { had, of },
        };
        Record {
            id: id.to_owned(),
            value: value.clone(),
            outcome,
        }
    }
}

/// The shared reads a default made first: elements it converted, properties
/// of runtime expressions it read, and plain defaults it read. The defaults
/// after it read the same objects.
#[derive(Default)]
struct FirstReads {
    elements: Vec<(usize, Value, Js)>,
    /// Each property read: its key, the node it was read on, and what it
    /// gave. Replayed under the node's address as this push knows it, as a
    /// later read of that node looks it up.
    properties: Vec<(String, Arc<Ast>, Js)>,
    plain: Vec<(String, Js)>,
}

/// Where the logs of first reads stand.
#[derive(Clone, Copy)]
struct Marks {
    elements: usize,
    properties: usize,
    plain: usize,
}

impl FirstReads {
    /// Where the logs stand before a default is evaluated.
    fn mark(declared: &Declared) -> Marks {
        Marks {
            elements: READ_LOG.with(|l| l.borrow().len()),
            properties: PROPERTIES_LOG.with(|l| l.borrow().len()),
            plain: declared.read_log.borrow().len(),
        }
    }

    /// No element or runtime expression read first.
    fn is_empty_of_shared(&self) -> bool {
        self.elements.is_empty() && self.properties.is_empty()
    }

    /// What was read first since `marks`.
    fn since(marks: Marks, declared: &Declared) -> FirstReads {
        let elements = READ_LOG.with(|l| {
            READ.with(|r| {
                let read = r.borrow();
                l.borrow()[marks.elements..]
                    .iter()
                    .filter_map(|address| {
                        let (value, js) = read.get(address)?;
                        Some((*address, value.clone(), js.clone()))
                    })
                    .collect()
            })
        });
        let properties = PROPERTIES_LOG.with(|l| {
            PROPERTIES.with(|r| {
                let read = r.borrow();
                l.borrow()[marks.properties..]
                    .iter()
                    .filter_map(|key| {
                        let property = read.get(key)?;
                        Some((
                            key.1.clone(),
                            Arc::clone(&property.owner),
                            property.value.clone(),
                        ))
                    })
                    .collect()
            })
        });
        let read = declared.read.borrow();
        let plain = declared.read_log.borrow()[marks.plain..]
            .iter()
            .filter_map(|name| Some((name.clone(), read.get(name)?.clone())))
            .collect();
        FirstReads {
            elements,
            properties,
            plain,
        }
    }

    /// Read again what was read, as the same objects.
    fn replay(&self, declared: &Declared) {
        READ.with(|r| {
            let mut read = r.borrow_mut();
            for (address, value, js) in &self.elements {
                read.insert(this_push(*address), (value.clone(), js.clone()));
            }
        });
        PROPERTIES.with(|r| {
            let mut read = r.borrow_mut();
            for (key, owner, js) in &self.properties {
                read.insert(
                    (this_push(Arc::as_ptr(owner) as usize), key.clone()),
                    PropertyRead {
                        owner: Arc::clone(owner),
                        value: js.clone(),
                    },
                );
            }
        });
        let mut read = declared.read.borrow_mut();
        for (name, js) in &self.plain {
            read.insert(name.clone(), js.clone());
        }
    }
}

/// Whether a value holds a runtime expression read as a value, whose
/// identity it carries.
fn holds_expression(js: &Js) -> bool {
    crate::stack::grow_stack!(holds_expression(js));
    match js {
        Js::Ast(_) => true,
        Js::Arr(items) => items.iter().any(holds_expression),
        Js::Obj(o) => o.values().any(holds_expression),
        _ => false,
    }
}

/// The elements and runtime expressions of an earlier push paired with this
/// push's (see [`same`]), by address: `now` by `then`, and back.
#[derive(Default)]
struct Aliases {
    now: HashMap<usize, usize>,
    then: HashMap<usize, usize>,
}

/// Pair the element or runtime expression at `then` (an earlier push's) with
/// the one at `now` (this push's): `false` if either is paired with another
/// already.
fn pair(then: usize, now: usize) -> bool {
    SAME_AS.with(|s| {
        let mut s = s.borrow_mut();
        match (s.now.get(&then), s.then.get(&now)) {
            (None, None) => {
                s.now.insert(then, now);
                s.then.insert(now, then);
                true
            }
            (Some(&n), Some(&t)) => n == now && t == then,
            _ => false,
        }
    })
}

/// Whether a default's value at an earlier push is its value at this one:
/// equal, with every element and runtime expression in it, whose identity
/// the evaluator can see, paired one to one with this push's. Evaluated now,
/// it would give what it gave then.
///
/// A comparison that fails may leave pairs behind: harmless, as both values
/// live until the pairs are cleared (see [`state_defaults`]), and a pair
/// only ever stands for an equal element or expression at the same place.
fn same(then: &Value, now: &Value) -> bool {
    crate::stack::grow_stack!(same(then, now));
    match (then, now) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Num(a), Value::Num(b)) => a.to_bits() == b.to_bits(),
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Arr(a), Value::Arr(b)) => {
            Arc::ptr_eq(a, b)
                || (a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| same(x, y)))
        }
        (Value::Obj(a), Value::Obj(b)) => Arc::ptr_eq(a, b) || same_entries(a, b),
        (Value::Element(a), Value::Element(b)) => {
            pair(Arc::as_ptr(a) as usize, Arc::as_ptr(b) as usize)
                && (Arc::ptr_eq(a, b)
                    || (a.type_name == b.type_name
                        && a.partial == b.partial
                        && a.has_dynamic_props == b.has_dynamic_props
                        && a.statement_id == b.statement_id
                        && same_entries(&a.props, &b.props)))
        }
        (Value::Ast(a), Value::Ast(b)) => same_expression(a, b),
        _ => false,
    }
}

fn same_entries(a: &JsObject<Value>, b: &JsObject<Value>) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((ka, va), (kb, vb))| ka == kb && same(va, vb))
}

/// A shared runtime expression: paired, and the same node.
fn same_expression(a: &Arc<Ast>, b: &Arc<Ast>) -> bool {
    pair(Arc::as_ptr(a) as usize, Arc::as_ptr(b) as usize) && (Arc::ptr_eq(a, b) || same_ast(a, b))
}

/// The same node, read field by field as the evaluator reads one.
fn same_ast(a: &Ast, b: &Ast) -> bool {
    crate::stack::grow_stack!(same_ast(a, b));
    let (fa, fb) = (fields(a), fields(b));
    fa.len() == fb.len()
        && fa
            .iter()
            .zip(&fb)
            .all(|((ka, x), (kb, y))| ka == kb && same_field(x, y))
}

fn same_field(x: &Field, y: &Field) -> bool {
    match (x, y) {
        (Field::Text(a), Field::Text(b)) => a == b,
        (Field::Num(a), Field::Num(b)) => a.to_bits() == b.to_bits(),
        (Field::Bool(a), Field::Bool(b)) => a == b,
        (Field::Node(a), Field::Node(b)) => same_expression(a, b),
        (Field::Nodes(a), Field::Nodes(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| same_ast(x, y))
        }
        (Field::Entries(a), Field::Entries(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|((ka, x), (kb, y))| ka == kb && same_ast(x, y))
        }
        (Field::Props(a), Field::Props(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|((ka, x), (kb, y))| ka == kb && same_ast(x, y))
        }
        _ => false,
    }
}

/// The defaults declared so far, as the ones after them read them: those
/// evaluated, as their values; the rest as they were materialized, read as a
/// value only when something reads them, and then once.
#[derive(Default)]
struct Declared {
    evaluated: JsObject<Js>,
    plain: JsObject<Value>,
    read: RefCell<HashMap<String, Js>>,
    /// The names put in `read`, in order (see [`Reuse`]).
    read_log: RefCell<Vec<String>>,
}

impl Declared {
    fn get(&self, name: &str) -> Option<Js> {
        if let Some(js) = self.evaluated.get(name) {
            return Some(js.clone());
        }
        let value = self.plain.get(name)?;
        if let Some(js) = self.read.borrow().get(name) {
            return Some(js.clone());
        }
        let js = from_value(value);
        self.read.borrow_mut().insert(name.to_owned(), js.clone());
        self.read_log.borrow_mut().push(name.to_owned());
        Some(js)
    }
}

/// Whether a materialized default holds anything to evaluate: a runtime
/// expression, in it or in its lists and plain objects.
pub(crate) fn has_runtime(value: &Value) -> bool {
    crate::stack::grow_stack!(has_runtime(value));
    match value {
        Value::Ast(_) => true,
        Value::Arr(items) => items.iter().any(has_runtime),
        Value::Obj(o) if element_parts(value).is_none() && !is_ast_node(value) => {
            o.values().any(has_runtime)
        }
        _ => false,
    }
}

/// `stateDefault`: the materialized default with its runtime expressions
/// evaluated, reading `declared`, the defaults declared before it. An object
/// literal shaped like an AST node isn't evaluated (a divergence).
fn state_default(value: &Value, declared: &Declared) -> Js {
    crate::stack::grow_stack!(state_default(value, declared));
    match value {
        Value::Ast(ast) => {
            if only_builtin_functions(ast) {
                evaluate(ast, &Scope::Root(declared))
            } else {
                Js::Null
            }
        }
        Value::Arr(items) => Js::Arr(Arc::new(
            items.iter().map(|v| state_default(v, declared)).collect(),
        )),
        Value::Obj(o) if element_parts(value).is_none() && !is_ast_node(value) => {
            let mut out = JsObject::new();
            for (key, v) in o.iter() {
                out.define(key.clone(), state_default(v, declared));
            }
            Js::Obj(Arc::new(out))
        }
        _ => from_value(value),
    }
}

/// Whether every call in the expression is a built-in function: anything else
/// exists only in the client, so the default is null.
fn only_builtin_functions(node: &Ast) -> bool {
    let mut ok = true;
    node.walk(&mut |n| {
        if let Ast::Comp { name, .. } = n
            && builtin_function_params(name).is_none()
        {
            ok = false;
        }
    });
    ok
}

/// A parse result's value as a JavaScript value: a step for each of its
/// nodes, and one for each byte of its strings and keys.
fn from_value(value: &Value) -> Js {
    crate::stack::grow_stack!(from_value(value));
    if !step(1) {
        return Js::Null;
    }
    match value {
        Value::Null => Js::Null,
        Value::Bool(b) => Js::Bool(*b),
        Value::Num(n) => Js::Num(*n),
        Value::Str(s) => {
            if !step(s.len()) {
                return Js::Null;
            }
            Js::Str(s.as_str().into())
        }
        Value::Arr(items) => Js::Arr(Arc::new(items.iter().map(from_value).collect())),
        Value::Obj(o) => from_object(o),
        Value::Element(element) => {
            let address = this_push(Arc::as_ptr(element) as usize);
            if let Some(js) = READ.with(|r| r.borrow().get(&address).map(|(_, js)| js.clone())) {
                return js;
            }
            // The object its JSON reads back as, built directly.
            out_of_work(|| {
                let mut out = JsObject::new();
                for (key, value) in [
                    ("type", from_value(&Value::Str("element".into()))),
                    (
                        "typeName",
                        from_value(&Value::Str(element.type_name.clone())),
                    ),
                    (
                        "props",
                        if step(1) {
                            from_object(&element.props)
                        } else {
                            Js::Null
                        },
                    ),
                    ("partial", from_value(&Value::Bool(element.partial))),
                    (
                        "hasDynamicProps",
                        from_value(&Value::Bool(element.has_dynamic_props)),
                    ),
                ] {
                    if !step(key.len()) {
                        return Js::Null;
                    }
                    out.define(key.to_owned(), value);
                }
                if let Some(id) = &element.statement_id {
                    if !step("statementId".len()) {
                        return Js::Null;
                    }
                    out.define("statementId", from_value(&Value::Str(id.clone())));
                }
                let js = Js::Obj(Arc::new(out));
                READ.with(|r| r.borrow_mut().insert(address, (value.clone(), js.clone())));
                READ_LOG.with(|l| l.borrow_mut().push(address));
                js
            })
        }
        Value::Ast(node) => Js::Ast(Arc::clone(node)),
    }
}

/// Convert an object's entries as they are priced, without copying it first.
fn from_object(object: &JsObject<Value>) -> Js {
    let mut out = JsObject::new();
    for (key, value) in object.iter() {
        if !step(key.len()) {
            break;
        }
        out.define(key.clone(), from_value(value));
    }
    Js::Obj(Arc::new(out))
}

/// A runtime expression read as a value, built whole: the object its JSON
/// reads back as, for the result or a literal.
fn whole_ast(node: &Ast) -> Js {
    out_of_work(|| from_value(&json_shaped_ast(node)))
}

/// `node[key]`, the node read as the plain object its JSON is: a step, and
/// one per item of a list it reads.
fn ast_property(node: &Arc<Ast>, key: &str) -> Option<Js> {
    let cache_key = (this_push(Arc::as_ptr(node) as usize), key.to_owned());
    if let Some(value) =
        PROPERTIES.with(|r| r.borrow().get(&cache_key).map(|read| read.value.clone()))
    {
        return Some(value);
    }
    let (_, field) = fields(node).into_iter().find(|(k, _)| *k == key)?;
    let read = |node: &Ast| Js::Ast(Arc::new(node.clone()));
    let value = match field {
        Field::Text(s) => Js::Str(s.into()),
        Field::Num(n) => match json_number_value(n) {
            Value::Num(n) => Js::Num(n),
            _ => Js::Null,
        },
        Field::Bool(b) => Js::Bool(b),
        Field::Node(node) => Js::Ast(Arc::clone(node)),
        Field::Nodes(nodes) => {
            if !step(nodes.len()) {
                return Some(Js::Null);
            }
            Js::Arr(Arc::new(nodes.iter().map(read).collect()))
        }
        Field::Entries(entries) => {
            if !step(entries.len()) {
                return Some(Js::Null);
            }
            Js::Arr(Arc::new(
                entries
                    .iter()
                    .map(|(k, v)| Js::Arr(Arc::new(vec![Js::Str(k.as_str().into()), read(v)])))
                    .collect(),
            ))
        }
        Field::Props(props) => {
            if !step(props.len()) {
                return Some(Js::Null);
            }
            let mut out = JsObject::new();
            for (k, v) in props.iter() {
                out.define(k.clone(), read(v));
            }
            Js::Obj(Arc::new(out))
        }
    };
    PROPERTIES_LOG.with(|l| l.borrow_mut().push(cache_key.clone()));
    PROPERTIES.with(|r| {
        r.borrow_mut().insert(
            cache_key,
            PropertyRead {
                owner: Arc::clone(node),
                value: value.clone(),
            },
        )
    });
    Some(value)
}

/// The value as `JSON.stringify` writes it, or `None` where it leaves it
/// out (`undefined`, a function): a property is dropped, an array item is
/// null. Null where the limits cut it.
fn to_value(js: &Js) -> Option<Value> {
    crate::stack::grow_stack!(to_value(js));
    if matches!(js, Js::Undefined | Js::Function { .. }) {
        return None;
    }
    if let Js::Ast(node) = js {
        return to_value(&whole_ast(node));
    }
    let Some(_level) = Level::enter() else {
        return Some(Value::Null);
    };
    if !spend(json_cost(js)) {
        return Some(Value::Null);
    }
    Some(match js {
        Js::Undefined | Js::Function { .. } | Js::Ast(_) => return None,
        Js::Null => Value::Null,
        Js::Bool(b) => Value::Bool(*b),
        Js::Num(n) => Value::Num(*n),
        Js::Str(s) => Value::Str(s.to_string()),
        Js::Arr(items) => Value::Arr(Arc::new(
            items
                .iter()
                .map(|v| to_value(v).unwrap_or(Value::Null))
                .collect(),
        )),
        Js::Obj(o) => {
            let mut out = JsObject::new();
            for (key, v) in o.iter() {
                if let Some(v) = to_value(v) {
                    out.define(key.clone(), v);
                }
            }
            Value::Obj(Arc::new(out))
        }
        // The boxed values `String.prototype` and the rest are.
        Js::Proto(kind) => match kind {
            Kind::Object => Value::Obj(Arc::new(JsObject::new())),
            Kind::Array => Value::Arr(Arc::new(Vec::new())),
            Kind::String => Value::Str(String::new()),
            Kind::Number => Value::Num(0.0),
            Kind::Boolean => Value::Bool(false),
        },
    })
}

// ─── Evaluating ─────────────────────────────────────────────────────────────

/// What names read: state from the defaults declared so far, and `@Each`'s
/// iterator variables. The reference's `Each` both substitutes the item for
/// `Ref` nodes named after its variable (`substituteRef`, here [`substitute`])
/// and evaluates the template in a child context whose `resolveRef` gives the
/// item itself for that name. A `RuntimeRef` isn't substituted, and one named
/// after the variable reaches the template through a statement it inlines
/// (`y = q` read inside `@Each(…, "q", y)`), so the lookup is needed too. Other
/// references are null here.
enum Scope<'a> {
    Root(&'a Declared),
    Each {
        name: &'a str,
        item: &'a Js,
        parent: &'a Scope<'a>,
    },
}

impl Scope<'_> {
    /// `getState(name)`: `declared[name] ?? null`.
    fn state(&self, name: &str) -> Js {
        match self {
            Scope::Root(declared) => match declared.get(name) {
                Some(Js::Undefined | Js::Null) | None => Js::Null,
                Some(v) => v,
            },
            Scope::Each { parent, .. } => parent.state(name),
        }
    }

    /// `resolveRef(name)`: the innermost `@Each` item of that name, the item
    /// itself (not a copy), else null.
    fn reference(&self, name: &str) -> Js {
        match self {
            Scope::Root(_) => Js::Null,
            Scope::Each {
                name: var, item, ..
            } if *var == name => (*item).clone(),
            Scope::Each { parent, .. } => parent.reference(name),
        }
    }
}

fn evaluate(node: &Ast, scope: &Scope) -> Js {
    crate::stack::grow_stack!(evaluate(node, scope));
    let Some(_level) = Level::enter() else {
        return Js::Null;
    };
    if !step(1) {
        return Js::Null;
    }
    match node {
        Ast::Str(s) => Js::Str(s.as_str().into()),
        Ast::Num(n) => Js::Num(*n),
        Ast::Bool(b) => Js::Bool(*b),
        Ast::Null | Ast::Ph(_) => Js::Null,
        Ast::StateRef(name) => scope.state(name),
        Ast::Ref(name) | Ast::RuntimeRef { n: name, .. } => scope.reference(name),
        Ast::Arr(els) => Js::Arr(Arc::new(els.iter().map(|e| evaluate(e, scope)).collect())),
        Ast::Obj(entries) => {
            let mut out = JsObject::new();
            for (key, v) in entries.iter() {
                out.define(key.clone(), evaluate(v, scope));
            }
            Js::Obj(Arc::new(out))
        }
        Ast::Comp {
            name,
            args,
            mapped_props,
            ..
        } => {
            if name == "Each" {
                return each(args, scope);
            }
            let Some(params) = builtin_function_params(name) else {
                return Js::Null;
            };
            // `node.mappedProps ?? nameArgs(node.args, params)`, evaluated.
            let mut named = JsObject::new();
            match mapped_props {
                Some(props) => {
                    for (key, v) in props.iter() {
                        named.define(key.clone(), evaluate(v, scope));
                    }
                }
                None => {
                    for (param, arg) in params.iter().zip(args.iter()) {
                        named.define(*param, evaluate(arg, scope));
                    }
                }
            }
            call(name, &named)
        }
        Ast::BinOp { op, left, right } => binary(op, left, right, scope),
        Ast::UnaryOp { op, operand } => match *op {
            "!" => Js::Bool(!truthy(&evaluate(operand, scope))),
            "-" => Js::Num(-to_number(&evaluate(operand, scope))),
            _ => Js::Null,
        },
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => {
            if truthy(&evaluate(cond, scope)) {
                evaluate(then, scope)
            } else {
                evaluate(otherwise, scope)
            }
        }
        Ast::Member { obj, field } => {
            let obj = evaluate(obj, scope);
            match &obj {
                Js::Undefined | Js::Null => Js::Null,
                // A field plucked from every item.
                Js::Arr(items) if field == "length" => Js::Num(items.len() as f64),
                // A step per item plucked from.
                Js::Arr(items) if !step(items.len()) => Js::Null,
                Js::Arr(items) => Js::Arr(Arc::new(
                    items
                        .iter()
                        .map(|item| match property(item, field) {
                            Js::Undefined | Js::Null => Js::Null,
                            v => v,
                        })
                        .collect(),
                )),
                _ => property(&obj, field),
            }
        }
        Ast::Index { obj, index } => {
            let obj = evaluate(obj, scope);
            let index = evaluate(index, scope);
            if is_nullish(&obj) || is_nullish(&index) {
                return Js::Null;
            }
            match &obj {
                Js::Arr(items) => {
                    let n = to_number(&index);
                    if n.fract() == 0.0 && n >= 0.0 && n < items.len() as f64 {
                        items[n as usize].clone()
                    } else {
                        Js::Undefined
                    }
                }
                _ => property(&obj, &to_string(&index)),
            }
        }
        // A reactive binding, as data: `{__reactive: "assign", target, expr}`.
        Ast::Assign { target, value } => {
            let mut out = JsObject::new();
            out.define("__reactive", Js::Str("assign".into()));
            out.define("target", Js::Str(target.as_str().into()));
            out.define("expr", from_value(&Value::Ast(value.clone())));
            Js::Obj(Arc::new(out))
        }
    }
}

fn binary(op: &str, left: &Ast, right: &Ast, scope: &Scope) -> Js {
    let left = evaluate(left, scope);
    match op {
        "&&" => {
            return if truthy(&left) {
                evaluate(right, scope)
            } else {
                left
            };
        }
        "||" => {
            return if truthy(&left) {
                left
            } else {
                evaluate(right, scope)
            };
        }
        _ => {}
    }
    let right = evaluate(right, scope);
    match op {
        "+" if matches!(left, Js::Str(_)) || matches!(right, Js::Str(_)) => {
            let text = |v: &Js| {
                if is_nullish(v) {
                    String::new()
                } else {
                    to_string(v)
                }
            };
            let (left, right) = (text(&left), text(&right));
            if !spend(left.len() + right.len()) {
                return Js::Str("".into());
            }
            Js::Str(format!("{left}{right}").into())
        }
        "+" => Js::Num(to_number(&left) + to_number(&right)),
        "-" => Js::Num(to_number(&left) - to_number(&right)),
        "*" => Js::Num(to_number(&left) * to_number(&right)),
        // Division by zero is 0 by design.
        "/" | "%" => {
            let (a, b) = (to_number(&left), to_number(&right));
            Js::Num(match (b == 0.0, op) {
                (true, _) => 0.0,
                (false, "/") => a / b,
                (false, _) => a % b,
            })
        }
        "==" => Js::Bool(loose_eq(&left, &right)),
        "!=" => Js::Bool(!loose_eq(&left, &right)),
        ">" => Js::Bool(to_number(&left) > to_number(&right)),
        "<" => Js::Bool(to_number(&left) < to_number(&right)),
        ">=" => Js::Bool(to_number(&left) >= to_number(&right)),
        "<=" => Js::Bool(to_number(&left) <= to_number(&right)),
        _ => Js::Null,
    }
}

/// `@Each(array, varName, template)`: the template once per item, its
/// iterator variable replaced by the item's literal first.
fn each(args: &[Ast], scope: &Scope) -> Js {
    if args.len() < 3 {
        return Js::Arr(Arc::new(Vec::new()));
    }
    let array = evaluate(&args[0], scope);
    let Js::Arr(items) = &array else {
        return Js::Arr(Arc::new(Vec::new()));
    };
    let var = match &args[1] {
        Ast::Ref(name) | Ast::Str(name) if !name.is_empty() => name.as_str(),
        _ => return Js::Arr(Arc::new(Vec::new())),
    };
    Js::Arr(Arc::new(
        items
            .iter()
            .map(|item| {
                if !step(1) {
                    return Js::Null;
                }
                // A step per node of the template (see `substitute`).
                let copy = substitute(&args[2], var, item);
                let child = Scope::Each {
                    name: var,
                    item,
                    parent: scope,
                };
                evaluate(copy.as_ref().unwrap_or(&args[2]), &child)
            })
            .collect(),
    ))
}

/// `substituteRef`: the template with its iterator variable replaced by the
/// item's literal, at a step per node. `None` where nothing changed: an AST
/// never changes once built, so the parts without the variable are shared,
/// not copied, and only the paths down to it are new.
fn substitute(node: &Ast, var: &str, value: &Js) -> Option<Ast> {
    crate::stack::grow_stack!(substitute(node, var, value));
    if !step(1) {
        return Some(Ast::Null);
    }
    let shared = |n: &Arc<Ast>| substitute(n, var, value).map(Arc::new);
    // Each part substituted, or as it was where the other parts changed.
    let either = |new: Option<Arc<Ast>>, old: &Arc<Ast>| new.unwrap_or_else(|| Arc::clone(old));
    match node {
        Ast::Ref(name) if name == var => Some(literal(value)),
        Ast::Member { obj, field } => {
            let new = substitute(obj, var, value);
            // A field of an object literal is that entry's value.
            if let Ast::Obj(entries) = new.as_ref().unwrap_or(obj)
                && let Some((_, v)) = entries.iter().find(|(k, _)| k == field)
            {
                return Some(v.clone());
            }
            Some(Ast::Member {
                obj: Arc::new(new?),
                field: field.clone(),
            })
        }
        Ast::Index { obj, index } => match (shared(obj), shared(index)) {
            (None, None) => None,
            (o, i) => Some(Ast::Index {
                obj: either(o, obj),
                index: either(i, index),
            }),
        },
        Ast::BinOp { op, left, right } => match (shared(left), shared(right)) {
            (None, None) => None,
            (l, r) => Some(Ast::BinOp {
                op,
                left: either(l, left),
                right: either(r, right),
            }),
        },
        Ast::UnaryOp { op, operand } => Some(Ast::UnaryOp {
            op,
            operand: shared(operand)?,
        }),
        Ast::Ternary {
            cond,
            then,
            otherwise,
        } => match (shared(cond), shared(then), shared(otherwise)) {
            (None, None, None) => None,
            (c, t, o) => Some(Ast::Ternary {
                cond: either(c, cond),
                then: either(t, then),
                otherwise: either(o, otherwise),
            }),
        },
        Ast::Arr(els) => Some(Ast::Arr(Arc::new(substitute_all(els, var, value)?))),
        Ast::Obj(entries) => {
            let mut out: Option<Vec<(String, Ast)>> = None;
            for (i, (k, v)) in entries.iter().enumerate() {
                match substitute(v, var, value) {
                    Some(new) => out
                        .get_or_insert_with(|| entries[..i].to_vec())
                        .push((k.clone(), new)),
                    None => {
                        if let Some(out) = &mut out {
                            out.push((k.clone(), v.clone()));
                        }
                    }
                }
            }
            Some(Ast::Obj(Arc::new(out?)))
        }
        Ast::Comp {
            name,
            args,
            function,
            mapped_props,
        } => {
            let new_args = substitute_all(args, var, value);
            let new_props = mapped_props.as_ref().and_then(|props| {
                let mut changed = false;
                let mut out = JsObject::new();
                for (key, v) in props.iter() {
                    let new = substitute(v, var, value);
                    changed |= new.is_some();
                    out.define(key.clone(), new.unwrap_or_else(|| v.clone()));
                }
                changed.then(|| Arc::new(out))
            });
            if new_args.is_none() && new_props.is_none() {
                return None;
            }
            Some(Ast::Comp {
                name: name.clone(),
                args: new_args.map_or_else(|| Arc::clone(args), Arc::new),
                function: *function,
                mapped_props: new_props.or_else(|| mapped_props.clone()),
            })
        }
        Ast::Assign { target, value: v } => Some(Ast::Assign {
            target: target.clone(),
            value: shared(v)?,
        }),
        _ => None,
    }
}

/// [`substitute`] over a list: `None` if no item changed.
fn substitute_all(nodes: &[Ast], var: &str, value: &Js) -> Option<Vec<Ast>> {
    let mut out: Option<Vec<Ast>> = None;
    for (i, node) in nodes.iter().enumerate() {
        match substitute(node, var, value) {
            Some(new) => out.get_or_insert_with(|| nodes[..i].to_vec()).push(new),
            None => {
                if let Some(out) = &mut out {
                    out.push(node.clone());
                }
            }
        }
    }
    out
}

/// `toLiteralAST`; null where the limits cut it.
fn literal(value: &Js) -> Ast {
    crate::stack::grow_stack!(literal(value));
    if let Js::Ast(node) = value {
        return literal(&whole_ast(node));
    }
    let Some(_level) = Level::enter() else {
        return Ast::Null;
    };
    if !spend(json_cost(value)) {
        return Ast::Null;
    }
    match value {
        Js::Undefined | Js::Null | Js::Function { .. } | Js::Ast(_) => Ast::Null,
        Js::Str(s) => Ast::Str(s.to_string()),
        Js::Num(n) => Ast::Num(*n),
        Js::Bool(b) => Ast::Bool(*b),
        Js::Arr(items) => Ast::Arr(Arc::new(items.iter().map(literal).collect())),
        Js::Obj(o) => Ast::Obj(Arc::new(
            o.iter().map(|(k, v)| (k.clone(), literal(v))).collect(),
        )),
        // `Array.prototype` is an array; the others have no own entries.
        Js::Proto(Kind::Array) => Ast::Arr(Arc::new(Vec::new())),
        Js::Proto(_) => Ast::Obj(Arc::new(Vec::new())),
    }
}

// ─── The built-in functions ─────────────────────────────────────────────────

fn call(name: &str, args: &JsObject<Js>) -> Js {
    let arg = |key: &str| args.get(key).cloned().unwrap_or(Js::Undefined);
    let array = arg("array");
    let numbers = arg("numbers");
    // A step for each number an aggregate reads.
    if matches!(name, "Sum" | "Avg" | "Min" | "Max")
        && let Js::Arr(items) = &numbers
        && !step(items.len())
    {
        return Js::Null;
    }
    match name {
        "Count" => Js::Num(match &array {
            Js::Arr(items) => items.len() as f64,
            _ => 0.0,
        }),
        "First" | "Last" => match &array {
            Js::Arr(items) => {
                let item = if name == "First" {
                    items.first()
                } else {
                    items.last()
                };
                match item {
                    None | Some(Js::Undefined | Js::Null) => Js::Null,
                    Some(v) => v.clone(),
                }
            }
            _ => Js::Null,
        },
        "Sum" => Js::Num(match &numbers {
            Js::Arr(items) => sum(items),
            _ => 0.0,
        }),
        "Avg" => Js::Num(match &numbers {
            Js::Arr(items) if !items.is_empty() => sum(items) / items.len() as f64,
            _ => 0.0,
        }),
        "Min" | "Max" => Js::Num(match &numbers {
            Js::Arr(items) if !items.is_empty() => {
                let pick = if name == "Min" { js_min } else { js_max };
                items
                    .iter()
                    .fold(to_number(&items[0]), |acc, v| pick(acc, to_number(v)))
            }
            _ => 0.0,
        }),
        "Sort" => sort(&array, &arg("field"), &arg("direction")),
        "Filter" => filter(&array, &arg("field"), &arg("operator"), &arg("value")),
        "Round" => {
            let decimals = arg("decimals");
            let d = if is_nullish(&decimals) {
                0.0
            } else {
                to_number(&decimals)
            };
            let factor = 10f64.powf(d);
            elementwise(&arg("number"), &|n| js_round(n * factor) / factor)
        }
        "Abs" => elementwise(&arg("number"), &f64::abs),
        "Floor" => elementwise(&arg("number"), &f64::floor),
        "Ceil" => elementwise(&arg("number"), &f64::ceil),
        _ => Js::Null,
    }
}

fn sum(items: &[Js]) -> f64 {
    items.iter().fold(0.0, |acc, v| acc + to_number(v))
}

// The sign of zero is never observable here: text and JSON write -0 as "0"
// (`number_to_string`), `==` and sorting treat it as 0, and division by zero
// is 0 by design. So `Math.min`, `Math.max` and `Math.round` keep only their
// visible behaviour, without choosing between -0 and +0.

/// `Math.min`: NaN wins.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// `Math.max`: NaN wins.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `Math.round`: to the nearest integer, halves up. NaN and the infinities
/// come out as they went in, since their floor is themselves.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// A number function on a number, or on each item of a list, nested lists too.
fn elementwise(value: &Js, f: &dyn Fn(f64) -> f64) -> Js {
    crate::stack::grow_stack!(elementwise(value, f));
    if !step(1) {
        return Js::Null;
    }
    match value {
        Js::Arr(items) => Js::Arr(Arc::new(items.iter().map(|v| elementwise(v, f)).collect())),
        v => Js::Num(f(to_number(v))),
    }
}

/// `resolveField`: a property, or a dot path of them. The path isn't empty
/// (see [`field_of`]), and a nullish object has no properties, so neither
/// needs a check of its own.
fn resolve_field(obj: &Js, path: &str) -> Js {
    if !path.contains('.') {
        return property(obj, path);
    }
    let mut current = obj.clone();
    for part in path.split('.') {
        if is_nullish(&current) {
            return Js::Undefined;
        }
        current = property(&current, part);
    }
    current
}

fn field_of(item: &Js, field: &str) -> Js {
    if field.is_empty() {
        item.clone()
    } else {
        resolve_field(item, field)
    }
}

fn text_or(value: &Js, otherwise: &str) -> String {
    if is_nullish(value) {
        otherwise.to_owned()
    } else {
        to_string(value)
    }
}

fn sort(array: &Js, field: &Js, direction: &Js) -> Js {
    let Js::Arr(items) = array else {
        return array.clone();
    };
    let field = text_or(field, "");
    let desc = text_or(direction, "asc") == "desc";
    let numeric = |v: &Js| match v {
        Js::Num(_) => true,
        Js::Str(s) => !s.is_empty() && !js_number(s).is_nan(),
        _ => false,
    };
    // A step per comparison, and one per byte of the text it compares. Once
    // the work runs out the rest compare equal, so the sort ends quickly; the
    // default is cut anyway.
    let compare = |a: &Js, b: &Js| -> Ordering {
        if !step(1) {
            return Ordering::Equal;
        }
        let (av, bv) = (field_of(a, &field), field_of(b, &field));
        let ordering = if numeric(&av) && numeric(&bv) {
            // A NaN difference counts as equal.
            (to_number(&av) - to_number(&bv))
                .partial_cmp(&0.0)
                .unwrap_or(Ordering::Equal)
        } else {
            let (at, bt) = (text_or(&av, ""), text_or(&bv, ""));
            if !step(at.len() + bt.len()) {
                return Ordering::Equal;
            }
            locale_compare(&at, &bt)
        };
        if desc { ordering.reverse() } else { ordering }
    };
    if !step(items.len()) {
        return Js::Null;
    }
    // `undefined` items go last without being compared.
    let (defined, undefined): (Vec<Js>, Vec<Js>) = items
        .iter()
        .cloned()
        .partition(|v| !matches!(v, Js::Undefined));
    let mut sorted = js_sort(defined, &|a, b| compare(a, b) == Ordering::Less);
    sorted.extend(undefined);
    Js::Arr(Arc::new(sorted))
}

/// `Array.prototype.sort` after JavaScriptCore (Bun), so that a comparison
/// that isn't a total order (numbers and text mixed so that it loops) mostly
/// gives the reference's order too; ECMAScript leaves that order to the
/// engine, an engine-dependent divergence where they differ (a 65-value list
/// whose first run descends, say). Traced in Bun: binary insertion under 8
/// items; from 8 to 65, the first run (reversed if it descends strictly) and
/// binary insertion after it; past 65, runs merged. That last is a stable
/// merge sort here, which agrees whenever the comparison is consistent. It
/// never panics, as `slice::sort_by` may.
fn js_sort(mut items: Vec<Js>, less: &dyn Fn(&Js, &Js) -> bool) -> Vec<Js> {
    let n = items.len();
    if n > 65 {
        return merge_sort(items, less);
    }
    if n < 2 {
        return items;
    }
    // The first run ends at the first item that breaks it, compared in order
    // from the third item, as Bun does.
    let start = if n < 8 {
        1
    } else if less(&items[1], &items[0]) {
        let run = (2..n)
            .find(|&i| !less(&items[i], &items[i - 1]))
            .unwrap_or(n);
        items[..run].reverse();
        run
    } else {
        (2..n)
            .find(|&i| less(&items[i], &items[i - 1]))
            .unwrap_or(n)
    };
    for i in start..n {
        let (mut lo, mut hi) = (0, i);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if less(&items[i], &items[mid]) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let item = items.remove(i);
        items.insert(lo, item);
    }
    items
}

/// A bottom-up merge sort that takes the right item only when it's strictly
/// less, so it's stable. It never panics, whatever the comparison does.
fn merge_sort(items: Vec<Js>, less: &dyn Fn(&Js, &Js) -> bool) -> Vec<Js> {
    let n = items.len();
    let mut src = items;
    let mut width = 1;
    while runs_to_merge(width, n) {
        let mut dst = Vec::with_capacity(n);
        for start in (0..n).step_by(2 * width) {
            let (mut left, left_end) = (start, (start + width).min(n));
            let (mut right, right_end) = (left_end, (left_end + width).min(n));
            while left < left_end || right < right_end {
                if right < right_end && (left >= left_end || less(&src[right], &src[left])) {
                    dst.push(src[right].clone());
                    right += 1;
                } else {
                    dst.push(src[left].clone());
                    left += 1;
                }
            }
        }
        src = dst;
        width *= 2;
    }
    src
}

/// Whether runs of `width` items are still shorter than the `n` being
/// sorted, so another merging pass is needed. (A pass at `width == n` would
/// only copy the one sorted run.)
fn runs_to_merge(width: usize, n: usize) -> bool {
    width < n
}

fn filter(array: &Js, field: &Js, operator: &Js, value: &Js) -> Js {
    crate::stack::grow_stack!(filter(array, field, operator, value));
    let Js::Arr(items) = array else {
        return Js::Arr(Arc::new(Vec::new()));
    };
    let field = text_or(field, "");
    let operator = text_or(operator, "==");
    // A step per item, and per byte of the text `contains` searches.
    let keep = |item: &Js| {
        if !step(1) {
            return false;
        }
        let v = field_of(item, &field);
        match operator.as_str() {
            "==" => loose_eq(&v, value),
            "!=" => !loose_eq(&v, value),
            ">" => to_number(&v) > to_number(value),
            "<" => to_number(&v) < to_number(value),
            ">=" => to_number(&v) >= to_number(value),
            "<=" => to_number(&v) <= to_number(value),
            "contains" => {
                let (text, part) = (text_or(&v, ""), text_or(value, ""));
                step(text.len() + part.len()) && text.contains(&part)
            }
            _ => false,
        }
    };
    Js::Arr(Arc::new(
        items.iter().filter(|v| keep(v)).cloned().collect(),
    ))
}

// ─── JavaScript's semantics ─────────────────────────────────────────────────

fn is_nullish(v: &Js) -> bool {
    matches!(v, Js::Undefined | Js::Null)
}

fn truthy(v: &Js) -> bool {
    match v {
        Js::Undefined | Js::Null => false,
        Js::Bool(b) => *b,
        Js::Num(n) => *n != 0.0 && !n.is_nan(),
        Js::Str(s) => !s.is_empty(),
        _ => true,
    }
}

/// The built-ins' `toNumber`: numbers as they are, strings by `Number()` with
/// NaN as 0, booleans as 1 and 0, anything else 0.
fn to_number(v: &Js) -> f64 {
    match v {
        Js::Num(n) => *n,
        Js::Str(s) => {
            let n = js_number(s);
            if n.is_nan() { 0.0 } else { n }
        }
        Js::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

/// `Number(text)`.
pub(crate) fn js_number(text: &str) -> f64 {
    let t = js::trim(text);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let bytes = t.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            return radix_number(&t[2..], radix);
        }
    }
    // StrDecimalLiteral: a sign, digits with a point, an exponent. Rust's
    // parser takes the same decimal syntax, and refuses a malformed exponent
    // (`1e`, `1e+`, `1ex`) as `Number` does, but it also reads `inf` and
    // `nan`, so the digits before the exponent are checked here.
    let rest = t.strip_prefix(['+', '-']).unwrap_or(t);
    let mantissa = rest.split(['e', 'E']).next().unwrap_or_default();
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mantissa_ok = (!whole.is_empty() || !fraction.is_empty())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.bytes().all(|b| b.is_ascii_digit());
    if mantissa_ok {
        t.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// Digits in a radix, as `Number("0x…")` reads them: NaN if any isn't one.
fn radix_number(digits: &str, radix: u32) -> f64 {
    let mut exact: u128 = 0;
    let mut approx = 0.0f64;
    let mut overflowed = false;
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else {
            return f64::NAN;
        };
        approx = approx * f64::from(radix) + f64::from(d);
        match exact
            .checked_mul(u128::from(radix))
            .and_then(|v| v.checked_add(u128::from(d)))
        {
            Some(v) => exact = v,
            None => overflowed = true,
        }
    }
    if overflowed { approx } else { exact as f64 }
}

/// `String(value)`.
fn to_string(v: &Js) -> String {
    crate::stack::grow_stack!(to_string(v));
    match v {
        Js::Undefined => "undefined".to_owned(),
        Js::Null => "null".to_owned(),
        Js::Bool(b) => b.to_string(),
        Js::Num(n) => number_to_string(*n),
        Js::Str(s) => s.to_string(),
        Js::Arr(items) => {
            let mut out = String::new();
            join(items, &mut out);
            out
        }
        Js::Obj(_) | Js::Ast(_) | Js::Proto(Kind::Object) => "[object Object]".to_owned(),
        // The text is the name's, wherever the function is from.
        Js::Function {
            name,
            constructor: true,
            ..
        } => format!("function {name}() {{ [native code] }}"),
        Js::Function { name, .. } => format!("function {name}() {{\n    [native code]\n}}"),
        Js::Proto(Kind::Array | Kind::String) => String::new(),
        Js::Proto(Kind::Number) => "0".to_owned(),
        Js::Proto(Kind::Boolean) => "false".to_owned(),
    }
}

/// `Array.prototype.join(",")` into `out`, nested arrays and all: a string
/// built from an array costs its length, and stops where the limits cut it.
fn join(items: &[Js], out: &mut String) {
    crate::stack::grow_stack!(join(items, out));
    let Some(_level) = Level::enter() else {
        return;
    };
    // A step per item, as well as the bytes it adds.
    if !step(1 + items.len()) {
        return;
    }
    for (i, item) in items.iter().enumerate() {
        if i > 0 && !push(out, ",") {
            return;
        }
        match item {
            Js::Undefined | Js::Null => {}
            Js::Arr(inner) => join(inner, out),
            _ => {
                if !push(out, &to_string(item)) {
                    return;
                }
            }
        }
    }
}

fn push(out: &mut String, text: &str) -> bool {
    if !spend(text.len()) {
        return false;
    }
    out.push_str(text);
    true
}

/// `ToPrimitive` of an object, as `==` takes it.
fn to_primitive(v: &Js) -> Js {
    match v {
        Js::Proto(Kind::Number) => Js::Num(0.0),
        Js::Proto(Kind::Boolean) => Js::Bool(false),
        Js::Arr(_) | Js::Obj(_) | Js::Ast(_) | Js::Function { .. } | Js::Proto(_) => {
            Js::Str(to_string(v).into())
        }
        _ => v.clone(),
    }
}

fn is_object(v: &Js) -> bool {
    matches!(
        v,
        Js::Arr(_) | Js::Obj(_) | Js::Ast(_) | Js::Function { .. } | Js::Proto(_)
    )
}

/// `a == b`.
fn loose_eq(a: &Js, b: &Js) -> bool {
    crate::stack::grow_stack!(loose_eq(a, b));
    match (a, b) {
        (Js::Undefined | Js::Null, Js::Undefined | Js::Null) => true,
        (Js::Undefined | Js::Null, _) | (_, Js::Undefined | Js::Null) => false,
        (Js::Num(x), Js::Num(y)) => x == y,
        (Js::Str(x), Js::Str(y)) => x == y,
        (Js::Bool(x), Js::Bool(y)) => x == y,
        (Js::Num(x), Js::Str(s)) | (Js::Str(s), Js::Num(x)) => *x == js_number(s),
        (Js::Bool(x), other) | (other, Js::Bool(x)) => {
            loose_eq(&Js::Num(f64::from(u8::from(*x))), other)
        }
        (Js::Arr(x), Js::Arr(y)) => Arc::ptr_eq(x, y),
        (Js::Obj(x), Js::Obj(y)) => Arc::ptr_eq(x, y),
        (Js::Ast(x), Js::Ast(y)) => {
            this_push(Arc::as_ptr(x) as usize) == this_push(Arc::as_ptr(y) as usize)
        }
        // The same built-in: the same name on the same prototype.
        (
            Js::Function {
                name: x,
                constructor: cx,
                owner: ox,
            },
            Js::Function {
                name: y,
                constructor: cy,
                owner: oy,
            },
        ) => x == y && cx == cy && ox == oy,
        (Js::Proto(x), Js::Proto(y)) => x == y,
        (x, y) if is_object(x) && is_object(y) => false,
        (x, y) if is_object(x) => loose_eq(&to_primitive(x), y),
        (x, y) => loose_eq(x, &to_primitive(y)),
    }
}

/// The string's UTF-16 units, lone surrogates included.
fn utf16(s: &str) -> Vec<u16> {
    let mut units = Vec::with_capacity(s.len());
    for c in s.chars() {
        match js::lone_surrogate_unit(c) {
            Some(unit) => units.push(unit),
            None => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
        }
    }
    units
}

/// One UTF-16 unit as a string: a lone surrogate if it's half of a pair.
fn unit_string(unit: u16) -> String {
    match char::from_u32(u32::from(unit)) {
        Some(c) => c.to_string(),
        None => js::lone_surrogate(unit).to_string(),
    }
}

/// `value[key]`, the key a string.
fn property(value: &Js, key: &str) -> Js {
    let inherited = |kind: Kind, methods: &[&'static str]| -> Js {
        if key == "__proto__" {
            return Js::Proto(kind);
        }
        if key == "constructor" {
            let name = match kind {
                Kind::Object => "Object",
                Kind::Array => "Array",
                Kind::String => "String",
                Kind::Number => "Number",
                Kind::Boolean => "Boolean",
            };
            return Js::Function {
                name,
                constructor: true,
                owner: kind,
            };
        }
        // The value's own prototype's method, else `Object.prototype`'s.
        let method = |names: &[&'static str], owner: Kind| {
            names.iter().find(|&&m| m == key).map(|&name| Js::Function {
                name,
                constructor: false,
                owner,
            })
        };
        method(methods, kind)
            .or_else(|| method(&OBJECT_PROTOTYPE_KEYS, Kind::Object))
            .unwrap_or(Js::Undefined)
    };
    match value {
        Js::Obj(o) => match o.get(key) {
            Some(v) => v.clone(),
            None => inherited(Kind::Object, &[]),
        },
        Js::Ast(node) => match ast_property(node, key) {
            Some(v) => v,
            None => inherited(Kind::Object, &[]),
        },
        Js::Arr(items) => {
            if key == "length" {
                return Js::Num(items.len() as f64);
            }
            match array_index(key) {
                Some(i) => items.get(i as usize).cloned().unwrap_or(Js::Undefined),
                None => inherited(Kind::Array, &ARRAY_METHODS),
            }
        }
        Js::Str(s) => {
            // Reading its UTF-16 units costs a step per byte.
            if !step(s.len()) {
                return Js::Undefined;
            }
            let units = utf16(s);
            if key == "length" {
                return Js::Num(units.len() as f64);
            }
            match array_index(key) {
                Some(i) => match units.get(i as usize) {
                    Some(&unit) => Js::Str(unit_string(unit).into()),
                    None => Js::Undefined,
                },
                None => inherited(Kind::String, &STRING_METHODS),
            }
        }
        Js::Num(_) => inherited(Kind::Number, &NUMBER_METHODS),
        Js::Bool(_) => inherited(Kind::Boolean, &BOOLEAN_METHODS),
        // Properties of functions and prototypes: a divergence.
        Js::Undefined | Js::Null | Js::Function { .. } | Js::Proto(_) => Js::Undefined,
    }
}

/// Weights for ASCII in ICU's root collation, which `localeCompare` uses:
/// the controls ignorable (0), then whitespace, punctuation and symbols in
/// ICU's order, digits, and letters with either case alike. Case is the
/// tie-breaker, lowercase first.
fn ascii_weight(c: u8) -> u16 {
    const ORDER: &[u8] =
        b"\t\n\x0b\x0c\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";
    let folded = c.to_ascii_lowercase();
    match ORDER.iter().position(|&o| o == folded) {
        Some(i) => i as u16 + 1,
        None => 0,
    }
}

/// `a.localeCompare(b)` for ASCII text, exactly; text with other characters
/// compares by code point after the ASCII weights, which ICU doesn't (a
/// divergence).
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    let key = |s: &str| -> (Vec<u32>, Vec<bool>) {
        let mut primary = Vec::new();
        let mut upper = Vec::new();
        for c in s.chars() {
            if c.is_ascii() {
                let w = ascii_weight(c as u8);
                if w == 0 {
                    continue;
                }
                primary.push(u32::from(w));
                upper.push(c.is_ascii_uppercase());
            } else {
                primary.push(0x1_0000 + c as u32);
                upper.push(false);
            }
        }
        (primary, upper)
    };
    let (pa, ua) = key(a);
    let (pb, ub) = key(b);
    pa.cmp(&pb).then_with(|| ua.cmp(&ub))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "out_of_work continuation must draw positive work")]
    fn a_work_check_without_a_draw_is_rejected() {
        LIMITS.with(|l| {
            l.work.set(1);
            l.cut.set(None);
        });
        out_of_work(|| Js::Null);
    }

    /// Text outside ASCII compares by code point, after all of ASCII: the
    /// documented divergence from ICU's collation (CONFORMANCE.md). It holds
    /// past U+FFFF too, where the weights must not overflow.
    #[test]
    fn text_outside_ascii_compares_by_code_point_after_ascii() {
        assert_eq!(locale_compare("é", "ü"), Ordering::Less);
        assert_eq!(locale_compare("z", "é"), Ordering::Less);
        assert_eq!(locale_compare("é", "😀"), Ordering::Less);
        assert_eq!(locale_compare("😀", "😁"), Ordering::Less);
        assert_eq!(locale_compare("😁", "😀"), Ordering::Greater);
    }

    /// `$s`, as a parse of `program` gives it.
    fn evaluated(program: &str) -> serde_json::Value {
        let schema = serde_json::json!({"$defs": {
            "Root": {"properties": {"children": {"type": "array"}}, "required": ["children"]},
            "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]}
        }});
        let library = crate::Library::from_json_schema(&schema);
        let result = crate::parse(program, &library, None);
        let json: serde_json::Value = serde_json::from_str(&result.to_json()).unwrap();
        json["stateDeclarations"]["$s"].clone()
    }

    /// `@Each` reads its loop variable two ways, as the reference does: a
    /// `Ref` named after it is substituted with a copy of the item before
    /// evaluating (`substitute`), and a `RuntimeRef` named after it (a query
    /// of that name, read through a statement the template inlines) resolves
    /// through the scope to the item itself. Each half shows: the copy isn't
    /// the item, the resolved one is, and both read its value.
    #[test]
    fn each_substitutes_refs_and_resolves_runtime_refs_to_its_item() {
        let program = "root = Root([Text(\"x\")])\nq = Query(\"tool\", {}, {rows: []})\ny = q\n$o = {a: 1}\n$s = @Each([$o], \"q\", [q == $o, y == $o, q.a, y.a])";
        assert_eq!(evaluated(program), serde_json::json!([[false, true, 1, 1]]));
    }

    /// A built-in's text is engine-defined, an engine-dependent divergence
    /// (CONFORMANCE.md): JavaScriptCore writes `Number`, `String` and
    /// `Boolean`'s constructors across several lines. The core writes every
    /// constructor on one line, and a method across three.
    #[test]
    fn a_constructor_turned_into_text_is_one_line() {
        let program = "root = Root([Text(\"x\")])\n$s = [(1).constructor + \"\", \"\".constructor + \"\", true.constructor + \"\", {}.constructor + \"\", (1).toFixed + \"\"]";
        assert_eq!(
            evaluated(program),
            serde_json::json!([
                "function Number() { [native code] }",
                "function String() { [native code] }",
                "function Boolean() { [native code] }",
                "function Object() { [native code] }",
                "function toFixed() {\n    [native code]\n}"
            ])
        );
    }

    /// Sorting values whose comparison loops gives an engine-defined order,
    /// an engine-dependent divergence (CONFORMANCE.md). The core follows
    /// JavaScriptCore's algorithm, which agrees for most lists but not this
    /// one: 65 values whose first run descends. Pinned as the core gives it.
    #[test]
    fn a_looping_sort_whose_first_run_descends_keeps_the_cores_order() {
        let twelve = r#""9", "8", "1a", "7", "10", 2, "x", "2", "b", "A", 3, "03""#;
        let items = format!(
            "{}, \"9\", \"8\", \"1a\", \"7\", \"10\"",
            [twelve; 5].join(", ")
        );
        let program = format!("root = Root([Text(\"x\")])\n$s = @Sort([{items}], \"\", \"desc\")");
        let expected = serde_json::json!([
            "x", "x", "x", "x", "x", "b", "b", "b", "b", "b", "A", "A", "A", "A", "A", "10", "10",
            "10", "10", "10", "10", "9", "9", "9", "9", "9", "9", "8", "8", "8", "8", "8", "8",
            "7", "7", "7", "7", "7", "7", 3, "03", 3, "03", 3, "03", 3, "03", 3, "03", 2, "2", 2,
            "2", 2, "2", 2, "2", 2, "2", "1a", "1a", "1a", "1a", "1a", "1a"
        ]);
        assert_eq!(evaluated(&program), expected);
    }
}
