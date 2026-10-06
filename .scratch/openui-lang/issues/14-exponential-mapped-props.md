# 14 — Results that double at every level

**Status:** done (2026-10-07). Proposed 2026-10-06 after measuring stack depth in ticket 12; you approved it on 2026-10-07.

**Problem (as proposed):** materializing a runtime expression copies each library component's args into its `mappedProps`, at every level, so a component nested `n` deep inside an expression is copied `2^n` times. Nesting 21 deep took 1.7 GB; about 25 would kill a phone app. The reference shares the objects, so its parse stays cheap.

**Wider than proposed.** Measuring before fixing found three sibling shapes, all from inlining a statement wherever it's referenced:

| Shape (20 levels) | Core before | Core after | Reference |
|---|---:|---:|---:|
| Component nested in a runtime expression | 863 MB | 2 MB | 0 ms, shared |
| Data value referenced twice per level | 1.8 GB | 3 MB | 211 ms, 525 MB |
| Expression referenced twice per level | 1.8 GB | 12 MB | 138 ms, exponential time |
| Element referenced twice per level | 254 MB (mostly the JSON) | 12 MB | 2.2 s, 1.2 GB |

The JSON of all four, and the error list when the repeated statement has an error, is exponential by contract: it must equal `JSON.stringify` of the reference's result. So sharing alone can't make every consumer safe.

**Decisions (made while implementing; your open question was which way to bound the JSON):**
- **Share, don't copy.** Elements were already reference-counted (ticket 10). Now arrays, objects and expressions in a result are too, and so are the children of expression nodes: `Value::Arr(Arc<Vec<Value>>)`, `Value::Obj(Arc<JsObject<Value>>)`, `Value::Ast(Arc<Ast>)`, `Ast` children as `Arc<Ast>` and `Arc<Vec<…>>`, `mapped_props` as `Option<Arc<JsObject<Ast>>>`. Cloning any node is constant time; validation copies on write. Memory is proportional to the program.
- **Cap the result, not the JSON writer.** I took option (b) from the proposal, applied at parse time instead of in `to_json`: a running estimate of the result's JSON size, counting every inlining (a cached statement adds its full size each time it's reused) and the `mappedProps` duplicate. Past `MAX_RESULT_SIZE` (8 MiB), what doesn't fit becomes null and `ParseResult::size_limited` is set. Every consumer then gets a bounded result, JSON or not, and the flag says it was cut, as `depth_limited` does for nesting. Results over the cap aren't cached.
- **The estimate errs high.** Each node kind is charged its JSON overhead, plus strings, keys, prop names, statement ids, errors and unresolved names. Checked against real output: the doubling shapes' JSON stays at or under the estimate (0.18–0.97 of it); corpus results over 3 KB are within 8% either way. A 487 KB synthetic program, far beyond any model output, estimates at 1.4 MB.

**Checks:**
- `tests/size.rs`: five doubling shapes at 60 levels (the four above plus one doubling its errors) are cut and flagged, with JSON under 1.25 × the cap, on a 512 KB stack, and streaming in eight pieces ends at the same cut. At 8 levels they're complete. No corpus program is cut.
- All conformance, stage, edit, prompt, fuzz and robustness checks unchanged. 40,000 more fuzz inputs (parse and edits): no new mismatches.
- Streaming benchmark unchanged on an idle machine: 8–12 µs per push, about 2.1× per doubling.
- Stack: the 10,000-statement reference chain now needs 256 KB in release (was 384 KB).

**Draft upstream report** (the reference's half of this; not filed):

> **A few lines of model output can make `parse()` take exponential time and memory.** Each reference is materialized afresh (`resolveRef` has no cache), so `s1 = [s0, s0]`, `s2 = [s1, s1]`, … doubles the work and the result with every line: 20 lines take 0.2–2 s and 0.5–1.2 GB on Bun, 30 would take hours. The same goes for expressions (`s1 = s0 + s0`) and elements (`s1 = Stack([s0, s0])`). `mappedProps` is fine in memory (it shares the args), but `JSON.stringify` of a result with components nested ~25 deep inside an expression is hundreds of MB. A model can't be trusted not to produce this, especially under prompt injection. Possible fixes: memoize materialized statements per parse (shared results, with a size cap for serialization), or cap the number of materialized nodes.
