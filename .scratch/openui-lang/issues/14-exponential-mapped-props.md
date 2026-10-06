# 14 — Components nested in a runtime expression cost exponential memory

**Status:** proposed (2026-10-06), needs your decision. Found while measuring stack depth in ticket 12.

**Problem:** materializing a runtime expression copies each library component's args into its `mappedProps`, at every level. A component nested `n` deep inside an expression is copied `2^n` times. The reference does the same assignment but shares the objects, so its parse stays cheap; only its `JSON.stringify` of the result blows up.

```
root = Text($x ? Text(Text(Text( … 21 deep … "a"))) : null)
```

| Nesting | Core: parse + JSON | Core: peak memory | Reference: parse | Reference: JSON.stringify |
|---:|---:|---:|---:|---:|
| 16 | 28 ms | 58 MB | 2 ms | 5 ms (5 MB of JSON) |
| 18 | 106 ms | 219 MB | 0 ms | 18 ms (21 MB) |
| 20 | 421 ms | 863 MB | 2 ms | 71 ms (84 MB) |
| 21 | 989 ms | 1.7 GB | – | – |

About 25 levels would exhaust a phone's memory and kill the app. Real model output nests two or three levels inside an expression, but a garbled stream can nest far deeper, and "never crash on any input" is the core's promise (ticket 09). The depth limit doesn't help: 25 is far below 256.

**What to build:** share instead of copy. `mappedProps` only ever maps param `i` to arg `i`, so the parse result can keep it as a mapping onto `args` and expand it when writing JSON. Parsing then costs linear memory, like the reference.

**Open question for you:** the JSON form is exponential by contract (it must equal `JSON.stringify`), so `to_json` of such a result stays huge. Options: (a) leave it, since a host that needs JSON gets what the reference would give; (b) add a size guard that refuses to write past some limit; (c) the Kotlin binding avoids JSON for the tree anyway (Dynamic News ticket 06). The parse-time fix is worth doing either way.

**Blocked by:** nothing.

- [ ] A component nested 30 deep inside a runtime expression parses in linear time and memory.
- [ ] All conformance, fuzzing and robustness checks stay green.
- [ ] A test pins the cost, as `tests/streaming_cost.rs` does for streaming.
