//! The OpenUI Lang language layer in Rust: parsing (one-shot and streaming),
//! prop validation, system prompt generation, edit-mode merging, and turning
//! element trees back into source.
//!
//! It's conformant with the reference implementation, `@openuidev/lang-core`
//! at a pinned upstream commit: for the same program, component library and
//! chunking, [`ParseResult::to_json`] matches the reference's `JSON.stringify`
//! output byte for byte, after every streamed chunk. The repository's
//! `CONFORMANCE.md` says how that's checked.
//!
//! # Parsing
//!
//! A [`Library`] comes from the JSON Schema that `library.toJSONSchema()`
//! produces on the web side. Positional arguments map to its props in order.
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! use openui_lang::{Library, parse};
//!
//! let schema = serde_json::json!({"$defs": {
//!     "Stack": {"properties": {"children": {"type": "array"}}},
//!     "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]}
//! }});
//! let library = Library::from_json_schema(&schema);
//! let result = parse("root = Stack([title])\ntitle = Text(\"Hello\")", &library, None);
//!
//! let root = result.root.unwrap();
//! assert_eq!(root.type_name, "Stack");
//! assert!(result.meta.errors.is_empty());
//! ```
//!
//! Parsing never fails: problems are data in the result. Here a number is
//! given where the schema wants a string, so the prop is dropped, and since
//! it's required, the component is too:
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! # use openui_lang::{Library, parse};
//! # let schema = serde_json::json!({"$defs": {
//! #     "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]}
//! # }});
//! # let library = Library::from_json_schema(&schema);
//! let result = parse("root = Text(5)", &library, None);
//!
//! assert!(result.root.is_none());
//! assert_eq!(result.meta.errors[0].message, r#"field "/value" expects string but got number"#);
//! ```
//!
//! Calls with `@` (`@Count(rows)`, `@ToAssistant("Hi")`) are built-ins, or the
//! library's own functions and actions from the schema's `functions` and
//! `actions`. They stay expressions for the runtime to evaluate, with their
//! args also by param name; a library call's literal args are checked like
//! props. An `@` name that's neither is reported as `unknown-function` and is
//! null.
//!
//! # Streaming
//!
//! A [`StreamParser`] takes the model's output chunk by chunk and returns a
//! full result after each one. What's still arriving is auto-closed and
//! marked `partial`; references that haven't arrived yet are left out until
//! they do. A push costs the new text plus the statements above it, not the
//! whole program.
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! # use openui_lang::Library;
//! use std::sync::Arc;
//! use openui_lang::StreamParser;
//! # let schema = serde_json::json!({"$defs": {
//! #     "Stack": {"properties": {"children": {"type": "array"}}},
//! #     "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]}
//! # }});
//! let library = Arc::new(Library::from_json_schema(&schema));
//! let mut stream = StreamParser::new(library, None);
//!
//! let first = stream.push("root = Stack([tit");
//! assert!(first.meta.incomplete);
//! assert!(first.root.unwrap().partial);
//!
//! stream.push("le])\ntitle = Text(\"Hello\")");
//! // Once the stream has ended:
//! let last = stream.finish();
//! assert!(!last.meta.incomplete);
//! assert!(!last.root.unwrap().partial);
//! ```
//!
//! After a reconnect, [`StreamParser::set`] takes the full text so far and
//! parses only what's new.
//!
//! The tree grows from the `root` statement. Without one, the root argument
//! names the library's root component, and the first statement calling it
//! stands in, with a `no-root` warning; with neither, there's no tree and a
//! `no-root` error, which a streaming client reads once the stream ends. A
//! `root` that picks a component at runtime (`root = $tab ? A : B`) comes
//! back as `root_expression`.
//!
//! # System prompts
//!
//! [`generate_system_prompt`] writes the instructions that teach a model the
//! language for a component library, as the reference's
//! `generateSystemPrompt` does, from the same JSON. A library's own
//! `functions` and `actions` are taught beside the built-ins.
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! let spec = serde_json::json!({"library": {
//!     "root": "Stack",
//!     "components": {"Stack": {"signature": "Stack(children: Component[])"}}
//! }});
//! let prompt = openui_lang::generate_system_prompt(&spec).unwrap();
//! assert!(prompt.contains("Stack(children: Component[])"));
//! ```
//!
//! # Edit mode
//!
//! In edit mode the model writes a patch: only the statements that change.
//! [`merge_statements`] applies it by statement name; `name = null` deletes,
//! and statements nothing reaches any more are dropped.
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! let existing = "root = Stack([a, b])\na = Text(\"A\")\nb = Text(\"B\")";
//! let patch = "root = Stack([a])\na = Text(\"New\")";
//! let merged = openui_lang::merge_statements(existing, patch, None);
//! assert_eq!(merged, "root = Stack([a])\na = Text(\"New\")");
//! ```
//!
//! # Back to source
//!
//! [`json_to_openui`] turns an element tree into source again. It takes the
//! tree's JSON form, the shape every result crosses a boundary in; read a
//! result back with [`parse_json`], which keeps it exact.
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! # use openui_lang::{Library, parse};
//! use openui_lang::{json_to_openui, parse_json};
//! # let schema = serde_json::json!({"$defs": {
//! #     "Stack": {"properties": {"children": {"type": "array"}}},
//! #     "Text": {"properties": {"value": {"type": "string"}}, "required": ["value"]}
//! # }});
//! # let library = Library::from_json_schema(&schema);
//! let result = parse("root = Stack([title])\ntitle = Text(\"Hello\")", &library, None);
//! let json = parse_json(&result.to_json()).unwrap();
//!
//! let source = json_to_openui(&json["root"], &library, Some(&json["stateDeclarations"])).unwrap();
//! assert_eq!(source, "root = Stack([title])\ntitle = Text(\"Hello\")");
//! ```
//!
//! # Limits
//!
//! No input can panic the core, overflow its stack, exhaust memory or keep it
//! busy for long, so a bad model response can't crash or freeze the app.
//! Four limits and a stall guard make that so, and each cuts the result and
//! sets a flag on it instead of failing. The limits are one [`Limits`] value, given to
//! [`parse_with_limits`] or [`StreamParser::with_limits`]; [`parse`] and
//! [`StreamParser::new`] use the defaults. Each can be raised or lowered:
//!
//! - **Program size**, 1 MiB by default ([`MAX_PROGRAM_SIZE`]): past it, the
//!   rest of the text is ignored as if the stream had stopped there
//!   ([`ParseResult::size_limited`]). A stream cut there stays cut: later
//!   pushes are ignored until [`StreamParser::set`].
//! - **Result size**, in bytes of JSON, scaling with the program: by default
//!   the larger of 64 KiB ([`MIN_RESULT_SIZE`]) and 64 times the program's
//!   size ([`MAX_AMPLIFICATION`]), never above 8 MiB ([`MAX_RESULT_SIZE`]); see
//!   [`Limits::result_size_for`]. A result that would grow past it is cut, the
//!   rest null ([`ParseResult::size_limited`]). That bounds amplification, so
//!   a caller serializing every push of a stream knows what it can cost. A
//!   stream's limit is that of the text so far, so an early push can be cut
//!   and a later one not; every push's result is still the one-shot parse of
//!   the text so far. Repeated subtrees are shared, not copied.
//! - **Depth**, 256 levels by default ([`MAX_DEPTH`]): nesting past it becomes
//!   null ([`ParseResult::depth_limited`]); evaluated state defaults may
//!   nest three levels per level.
//! - **Work**, by default the larger of a million steps and 1,000 steps per
//!   byte of program, never above 64 million ([`Limits::work_for`]):
//!   evaluating state defaults counts every step it takes, an expression
//!   evaluated, an item sorted, filtered or copied into an `@Each` template,
//!   a node of a value read, a byte built. Past it, the default being
//!   evaluated is null, as is every expression default after it
//!   ([`ParseResult::size_limited`]). A step takes nanoseconds, so a parse
//!   stays within a fraction of a second.
//! - A stall: every loop that reads input either moves forward or stops. A
//!   loop that stopped moving forward would be a bug in the core, never the
//!   input's fault. In debug builds it panics; in release builds the parse
//!   ends there with a cut result ([`ParseResult::size_limited`]), and a
//!   [`StreamParser`] it cut gives that result again, ignoring pushes, until
//!   [`StreamParser::set`]. [`parse_json`] refuses its input instead, and
//!   [`merge_statements`] stops where the stall was.
//!
//! ```
//! # #[global_allocator]
//! # static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
//! # use openui_lang::{Library, Limits, parse_with_limits};
//! # let library = Library::from_json_schema(&serde_json::json!({"$defs": {
//! #     "Stack": {"properties": {"children": {"type": "array"}}}
//! # }}));
//! let program = "root = Stack([[[[[1]]]]])";
//! let shallow = Limits::default().with_depth(3);
//! assert!(parse_with_limits(program, &library, None, shallow).depth_limited);
//! ```
//!
//! The limits count bytes, levels and steps, never time, so a result is the
//! same on every machine. No default cuts a program in the conformance
//! corpus. The result-size and work limits scale with the program as
//! streamed so far, so a stream's result is always the one-shot parse of its
//! text so far: an early push may be cut where a later one isn't.
//!
//! Every other entry point takes the same value, beside a form that uses the
//! defaults. Where it has flags, hitting a limit cuts and flags the result;
//! where it has an error, a limit refuses the input with it:
//!
//! - [`merge_statements_with_limits`]: the program-size limit applies to each
//!   input, the depth limit to each statement's expression, with
//!   `size_limited` and `depth_limited` on the [`Merged`] result.
//! - [`parse_expression_with_limits`]: the depth limit, and whether it cut.
//! - [`parse_json_with_limits`], [`json_to_openui_with_limits`],
//!   [`generate_system_prompt_with_limits`] and
//!   [`generate_prompt_with_limits`]: JSON nesting deeper than three levels
//!   per level of the depth limit and 32 more (800 at the default) is refused
//!   with their error, so whatever a parse under the same limits produces
//!   still reads back, converts back and nests. Their time and output grow
//!   with the JSON, nothing more, and it's already in the caller's memory, so
//!   the size limits don't apply.
//!
//! The core grows its stack on demand: when a call runs low, it goes on in a
//! segment allocated on the heap. And a result frees its tree with a loop,
//! not recursion. So the core needs no minimum thread stack: it parses, and
//! its results drop, on a 64 KB one, at the default depth limit or a raised
//! one, optimized or not. The exception is [`parse_json`]'s result: a
//! `serde_json::Value`, which drops by recursion as serde_json's own type
//! does. At the default's 800 levels that takes under 64 KB optimized; free a
//! value read under a raised depth limit with [`free_json`], which loops.
//!
//! Because they free themselves, [`Value`] and [`Ast`] implement [`Drop`],
//! so a pattern can't move a field out of one: borrow it instead, and clone
//! the [`Arc`](std::sync::Arc) that holds the field's tree, which is cheap.
//! [`Element`] and [`ParseResult`] destructure as before.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// The repository README's Rust example runs as a doc test, so it can't drift.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeDoctests;

/// How deep expressions and reference chains may nest, by default, before the
/// core stops descending and sets [`ParseResult::depth_limited`]: the
/// default of [`Limits::depth`]. Real UIs nest a few dozen levels at most;
/// this bound keeps a result's depth, and so the work it takes, bounded. See
/// `tests/depth.rs`.
pub const MAX_DEPTH: usize = 256;

/// About how large a parse result may grow by default, in bytes of its JSON,
/// however large its program: the default ceiling, [`Limits::result_size`],
/// on a limit that scales with the program ([`Limits::result_size_for`]). A
/// statement
/// referenced from two places is inlined in both, so a few lines that each
/// reference the one before twice double the result at every line; a
/// component inside a runtime expression is written twice (`args` and
/// `mappedProps`), with the same effect when nested; and a state default,
/// evaluated, can list the one above it twice, or add a string to itself.
/// Past this, the rest of the result becomes null and
/// [`ParseResult::size_limited`] is set. Real results are about twice the
/// program's size, so this is far out of reach of any program a model writes.
pub const MAX_RESULT_SIZE: usize = 8 * 1024 * 1024;

/// The default floor of the result-size limit, [`Limits::result_floor`]: a
/// short program's result may always grow to 64 KiB of JSON.
pub const MIN_RESULT_SIZE: usize = 64 * 1024;

/// The default [`Limits::amplification`]: how many times its program's size a
/// result may grow to, above [`MIN_RESULT_SIZE`] and up to
/// [`MAX_RESULT_SIZE`]. Real results are about twice their program; the
/// most any conformance case reaches is about 42, on a program of 22 bytes,
/// well inside the floor.
pub const MAX_AMPLIFICATION: usize = 64;

/// How many bytes of a program a parse reads by default, the default of
/// [`Limits::program_size`]: past it, the rest
/// is ignored as if the stream had stopped there, and
/// [`ParseResult::size_limited`] is set. A model's reply is a few KB; this
/// keeps a runaway stream from growing without end.
pub const MAX_PROGRAM_SIZE: usize = 1024 * 1024;

mod ast;
mod builtins;
mod enrich;
mod evaluate;
mod expr;
mod free;
mod js;
mod lexer;
mod library;
mod limits;
mod materialize;
mod merge;
mod parser;
mod preprocess;
mod progress;
mod prompt;
mod result;
mod serialize;
mod stack;
mod statements;
mod stream;
mod table;
mod validation;

/// The conformance folder, for the unit tests that read it: the repository's,
/// or wherever `OPENUI_CONFORMANCE_DIR` names, as the integration tests read
/// it, so they also run where the source tree isn't (an Android device).
#[cfg(test)]
fn conformance_dir() -> std::path::PathBuf {
    match std::env::var_os("OPENUI_CONFORMANCE_DIR") {
        Some(dir) if !dir.is_empty() => dir.into(),
        _ => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance"),
    }
}

pub use ast::{Ast, RefType};
pub use builtins::{is_builtin, is_reserved_call};
pub use enrich::{EnrichedError, enrich_errors, enriched_errors_to_json};
pub use expr::{parse_expression, parse_expression_with_limits};
pub use free::free_json;
pub use js::{JsObject, JsonError, parse_json, parse_json_with_limits, stringify_json};
pub use lexer::{Token, tokenize, tokens_to_json};
pub use library::{Call, CallKind, Component, Library, Param};
pub use limits::Limits;
pub use merge::{Merged, merge_statements, merge_statements_with_limits};
pub use parser::{parse, parse_with_limits};
pub use preprocess::{preprocess, strip_fences};
pub use prompt::{
    PromptError, generate_prompt, generate_prompt_with_limits, generate_system_prompt,
    generate_system_prompt_with_limits,
};
pub use result::{
    Element, Meta, MutationStatement, ParseResult, QueryStatement, Severity, ValidationError,
    ValidationErrorCode, Value,
};
pub use serialize::{SerializeError, json_to_openui, json_to_openui_with_limits};
pub use statements::{IdKind, RawStatement, auto_close, split};
pub use stream::StreamParser;

// Embedders parse off the UI thread and share one library between parsers, so
// these stay shareable across threads; this stops compiling if one stops.
const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<StreamParser>();
    shareable::<ParseResult>();
    shareable::<Library>();
};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
