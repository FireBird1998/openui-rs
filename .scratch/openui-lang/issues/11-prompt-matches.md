# 11 — System prompt matches the reference

**What to build:** Prompt generation. It takes a prompt spec in the reference parser's JSON shape (components, groups, tools, examples, rules, edit, inline, tool-call and binding modes) and produces the same system prompt string as the reference `generatePrompt` and `generateSystemPrompt`. The test corpus has prompt specs lifted from the reference prompt tests plus specs built from the benchmark schema.

**Blocked by:** 01 — Walking skeleton.

**Status:** done (2026-10-06)

- [x] Every prompt spec in the corpus produces exactly the reference prompt string.
- [x] Every mode combination (tools, no tools, edit mode, inline mode, bindings on and off) has a case.
- [x] `openui-lang prompt <spec.json>` prints the prompt.

## Comments

**2026-10-06, done.**
- `src/prompt.rs`: `generate_system_prompt(&serde_json::Value) -> Result<String, PromptError>` covers the reference's three input forms (`{library, promptOptions}`, a prompt spec directly, `{cloud: true, ...}`), and `generate_prompt` is the deprecated direct form. The spec is taken as JSON and read the way JavaScript reads it: `??` defaults only replace missing or null, `if (x)` is truthiness, `${x}` stringifies, object keys come in property order. Where the reference throws (cloud mode with an invalid library, or options without a library), the error message is the same.
- Cloud mode (`]]>openui:config` block) and its library validator are included: they're part of `generateSystemPrompt`, small, and the config block is plain `JSON.stringify`, which the existing JS-faithful writer handles (key order, numbers, non-finite).
- The builtin registry's prompt docs (signature and one-line description for the 13 builtins and `Each`) are copied into the crate; they're text, not behaviour, and the reference keeps them next to the evaluator this crate doesn't port.
- Corpus: 44 specs in `conformance/prompts/` (the argument to `generateSystemPrompt`, one per file). Lifted from the reference prompt tests (self-hosted, every cloud case, the invalid library), built from the benchmark schema (signatures in the shape `library.toPromptSpec()` produces, with groups), and a matrix over tools (none, names, full specs with input/output schemas), `toolCalls` and `bindings` explicit and defaulted, edit and inline modes, examples and tool examples, rules, preamble (including an empty one, which `??` keeps), falsy and truthy non-booleans, a null root, a null library, groups with notes, unknown and repeated names, and ungrouped components. Goldens are the full text (`conformance/golden/prompts/<name>.json`, `{"prompt"}` or `{"error"}`), written by `reference/prompts.ts`, which `scripts/regen-golden.sh` runs too. `tests/prompt.rs` checks every spec and that the mode matrix is covered.
- All 44 matched on the first run. The only decisions were about inputs the reference would crash on (a non-array `tools`, a non-object `components`): treated as absent rather than panicking.
- Not ported: `library.ts` (Zod component definitions to signatures), per the spec's scope; a Rust user builds the spec's signatures themselves or takes them from the web side's `toPromptSpec()`.
