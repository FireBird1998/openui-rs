//! Rust port of the OpenUI Lang parser from `@openuidev/lang-core`.
//!
//! Output is meant to be byte-identical to `JSON.stringify` of the TS
//! `ParseResult`; `conformance/` checks that against the TS implementation.
//!
//! ```
//! use openui_lang::{parse, Library};
//! let schema = serde_json::json!({"$defs": {"Text": {
//!     "properties": {"value": {"type": "string"}}, "required": ["value"]}}});
//! let lib = Library::from_json_schema(&schema);
//! let result = parse(r#"root = Text("hi")"#, &lib, None);
//! assert!(result.root.is_some());
//! ```

pub mod ast;
pub mod builtins;
pub mod expr;
pub mod lexer;
pub mod library;
pub mod materialize;
pub mod parser;
pub mod statements;
pub mod validation;
pub mod value;

pub use ast::Ast;
pub use library::Library;
pub use parser::{parse, ParseResult, StreamParser};
pub use value::Value;
