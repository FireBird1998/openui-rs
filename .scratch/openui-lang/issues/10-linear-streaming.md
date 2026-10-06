# 10 — Linear-time streaming, still identical

**What to build:** Rework the streaming parser so it stops reprocessing the whole text on every chunk, making total streaming work grow linearly with program length. Output stays byte-identical. Add benchmarks that run the same workload through the core and the reference parser, and record the results.

**Blocked by:** 08 — Differential fuzzing; 09 — Never panic.

**Status:** ready-for-agent

- [ ] All conformance, fuzzing and robustness checks stay green.
- [ ] Doubling a program costs no more than about 2.2× the streaming time, measured from 3.5 KB up to at least 28 KB.
- [ ] Benchmarks report Rust vs reference timings for the 7 benchmark samples and the synthetic large programs, and the results are written to a benchmarks document.
- [ ] `openui-lang bench` runs the same workload as the reference bench script.
