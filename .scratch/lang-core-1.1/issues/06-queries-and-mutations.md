# 06 — Queries and mutations: arguments, waitFor and query-cycle

**What to build:** Query and mutation statements match 1.1:
- their arguments are materialized like other expressions;
- `deps` come from the materialized arguments;
- `waitFor` lists the queries whose results the tool name or arguments read;
- queries reading each other around a loop get `query-cycle`, once per loop, on its first query in statement order, with the loop in the message.

Error hints carry the new code.

**Blocked by:** 05 — Materializing

**Status:** ready-for-agent

- [ ] The parse cases pending for queries and mutations match, and upstream's `toolCompatibility` and `toolReferences` cases match.
- [ ] Cases cover a self-reading query, a loop of three, two separate loops, a query reading a loop, and a tool name read from another statement.
- [ ] Error hints for `query-cycle` match.
- [ ] `scripts/check.sh` passes.
