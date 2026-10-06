//! OpenUI Lang parser, conformant with the reference parser in
//! `@openuidev/lang-core`: for the same program, component library and
//! chunking, [`ParseResult::to_json`] matches the reference parser's
//! `JSON.stringify` output byte for byte.
//!
//! ```
//! use openui_lang::{Library, parse};
//!
//! let schema = serde_json::json!({"$defs": {"Text": {
//!     "properties": {"value": {"type": "string"}},
//!     "required": ["value"]
//! }}});
//! let library = Library::from_json_schema(&schema);
//! let result = parse(r#"root = Text("hi")"#, &library, None);
//! assert_eq!(result.root.unwrap().type_name, "Text");
//! ```

mod ast;
mod builtins;
mod expr;
mod js;
mod lexer;
mod library;
mod materialize;
mod parser;
mod preprocess;
mod result;
mod statements;
mod stream;
mod validation;

pub use ast::Ast;
pub use builtins::{is_builtin, is_reserved_call};
pub use expr::parse_expression;
pub use lexer::{Token, tokenize, tokens_to_json};
pub use library::{Component, Library, Param};
pub use parser::parse;
pub use preprocess::{preprocess, strip_fences};
pub use result::{Element, Meta, ParseResult, ValidationError, ValidationErrorCode, Value};
pub use statements::{IdKind, RawStatement, auto_close, split};
pub use stream::StreamParser;
