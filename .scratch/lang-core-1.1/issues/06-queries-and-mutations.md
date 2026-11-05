# 06 — Queries and mutations: arguments, waitFor and query-cycle

**What to build:** Query and mutation statements match 1.1:
- their arguments are materialized like other expressions;
- `deps` come from the materialized arguments;
- `waitFor` lists the queries whose results the tool name or arguments read;
- queries reading each other around a loop get `query-cycle`, once per loop, on its first query in statement order, with the loop in the message.

Error hints carry the new code.

**Blocked by:** 05 — Materializing

**Status:** done (2026-11-06)

- [x] The parse cases pending for queries and mutations match, and upstream's `toolCompatibility` and `toolReferences` cases match.
- [x] Cases cover a self-reading query, a loop of three, two separate loops, a query reading a loop, and a tool name read from another statement.
- [x] Error hints for `query-cycle` match.
- [x] `scripts/check.sh` passes.

## Comments

**2026-11-06, done.**
- **Arguments:** a query's or mutation's arguments go through `materialize_expr` under the statement's id, as anywhere else, replacing 1.0's `name_raw_call_args`. References are inlined, and the statements reached aren't orphans. `deps` comes from the materialized arguments.
- **New fields:** `waitFor` lists the queries the tool name and arguments read, in the order first met. `query-cycle` is found by a depth-first walk in statement order, done with an explicit stack where the reference recurses, same order. It's reported once per loop, on the loop's first query, with the members sorted into statement order.
- **Result:** 75 pending parse cases match, among them all 63 of upstream's `toolCompatibility` and `toolReferences` cases. The 2 pending error hints match too. The 2 parse cases left are state defaults (ticket 07).
- **New cases:** `hand/130` to `hand/134`:
  - two queries reading each other;
  - a query reading itself;
  - a loop of three plus a reader outside it;
  - two separate loops, one through a tool name;
  - a tool, its arguments and a mutation's arguments read from other statements, through an alias and a built-in.
