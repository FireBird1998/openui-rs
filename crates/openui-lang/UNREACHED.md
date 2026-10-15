# Lines no test runs, and why

`scripts/coverage.sh` measures line coverage over the offline suite. It fails if a line no test runs isn't listed here: under its file, as a code span holding the line's text exactly, trimmed. Each line below is a guard that mirrors the reference, a check of an invariant, or code that runs only at compile time. A test would have to break the invariant to reach it.

Measured 2026-10-15 (testing ticket 03): 99.12% of lines. The 30 source lines not run are all listed here. The summary's "44 missed lines" counts some generic code once per compiled copy; the per-line listing the check uses merges those copies.

## crates/openui-ffi/src/lib.rs

Nothing: every line runs.

## crates/openui-lang/src/enrich.rs

- `_ => Vec::new(),` — the signature hint reads `properties` only for an error about a missing or null required prop. A component has such a prop only when its `properties` is an object, array or string, so the fallback for other values never runs.

## crates/openui-lang/src/expr.rs

- `_ => Arc::unwrap_or_clone(left),` — only tokens with an infix precedence reach this match (commented in place).

## crates/openui-lang/src/js.rs

- `return None;` — `json_parse_string` is only called on text that starts with a double quote: the lexer's strings and the JSON reader's.
- `unreachable!()` — twice, in the iterative JSON reader: the stack of open arrays and objects always holds the kind just closed.

## crates/openui-lang/src/lib.rs

- `const fn shareable<T: Send + Sync>() {}` — the thread-sharing check runs at compile time, never at run time.

## crates/openui-lang/src/materialize.rs

- `Ast::RuntimeRef { n, .. } => 44 + n.len(),` — value mode only sizes nodes from the parser, and the parser never makes a `RuntimeRef`. Only resolving a ref in expression mode does.
- `Resolved::Ast(a) => Value::Ast(Arc::new(a)),` — in value mode, `resolve_ref` always answers with a value; an expression is wrapped in one.
- `continue; // unresolved placeholders drop out of arrays` — placeholders (`Ph`) are only made in expression mode, never in the parser's arrays that value mode walks. It mirrors the reference's guard.
- `ctx.unresolved(name);` and `return placeholder(mode);` — the table gives a slot to every name a statement mentions, defined or not (`Table::intern`), so a ref always finds one.
- `_ => {}` — `tag_statement` returns early unless the value is an element or an object shaped like one, and those are the two arms above it.
- `Resolved::Value(_) => unreachable!("expr mode resolves to AST"),` — the invariant its message states.

## crates/openui-lang/src/parser.rs

- `_ => &[],` — a statement classified as a Query or Mutation is always a call to one, so its expression is always a component call.
- `return ParseResult::empty(was_incomplete);` — twice: the entry statement is `root`, the root name's statement, a statement calling that component, or the first statement, and all of those are defined.

## crates/openui-lang/src/preprocess.rs

- `let body = match rest.find('\n') {`, `Some(newline) => &rest[newline + 1..],`, `None => rest,`, `return match body.rfind("```") {`, `Some(fence) => body[..fence].to_owned(),`, `None => body.to_owned(),` — the reference keeps this fallback for input that starts with a fence the scan didn't match, but the scan always matches it first (commented in place).

## crates/openui-lang/src/prompt.rs

- `}` — the end of copying a cloud library's entries when the library isn't an object. Validation has already rejected such a library with "chatLibrary must be an object." (corpus: `cloud-library-not-an-object`).

## crates/openui-lang/src/serialize.rs

- `_ => unreachable!("only scalars"),` — its callers only pass strings, numbers and booleans.
- `_ => Ok(Out::Text("null".to_owned())),` — `ast_kind` is only called for nodes already recognized as one of the kinds listed above it. An object with an unknown `k` is serialized as a plain object instead.

## crates/openui-lang/src/stream.rs

- `}`, `let count = completed.count;`, `return (self.build_from_table(None, false, count), false);` — the case where nothing is pending after completed statements. None of the inputs tried reached it, including trailing blank lines and trailing comments: while streaming, the cleaned text is never seen to end right after a completed statement. It mirrors the reference's structure; it's listed instead of deleted because that is unproven.

## crates/openui-lang/src/table.rs

- `_ => unreachable!("specials out of sync"),` — the invariant its message states.

## crates/openui-lang/src/validation.rs

- `_ => "null".to_owned(),` — `scalar_json`'s only caller checks that the value is a string, number or boolean first.
