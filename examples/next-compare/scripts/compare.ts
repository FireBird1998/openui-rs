// The headless check: both engines, through the same seam the app uses
// (lib/lang-core-shim.ts), over every bundled program at every chunk size,
// driven the way react-lang's Renderer drives them: set(text so far) per
// chunk, then set(whole text, true) when the stream ends. Asserts the two
// results are JSON-equal at every update and prints the parser timings.
//
//   npm run compare        (copies the wasm build and programs first)
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";

const here = dirname(fileURLToPath(import.meta.url));
const { openuiLibrary } = await import("@openuidev/react-ui/genui-lib");
const { createStreamingParser } = await import("../lib/lang-core-shim.ts");
const { loadWasmSync, parserCalls, tagLibrary } = await import("../lib/engines.ts");
const { chunk, CHUNK_SIZES } = await import("../lib/chunks.ts");
const { summarize } = await import("../lib/stats.ts");

loadWasmSync(readFileSync(join(here, "../wasm/openui_wasm_bg.wasm")));
const programs: { name: string; text: string }[] = JSON.parse(
  readFileSync(join(here, "../lib/programs.generated.json"), "utf8"),
);
const RUNS = Math.max(1, Number(process.env.COMPARE_RUNS ?? 5) || 5);

type Times = number[];
const sinks = { js: [] as Times, rust: [] as Times };
const libraries = {
  js: tagLibrary(openuiLibrary, "js", { parse: (ms) => sinks.js.push(ms) }),
  rust: tagLibrary(openuiLibrary, "rust", { parse: (ms) => sinks.rust.push(ms) }),
};

// One stream through one engine, as the Renderer would: a fresh parser per stream.
function stream(engine: "js" | "rust", chunks: string[]) {
  const library = libraries[engine];
  const parser = createStreamingParser(library.toJSONSchema(), library.root);
  const results: unknown[] = [];
  let text = "";
  for (const piece of chunks) results.push(parser.set((text += piece), false));
  results.push(parser.set(text, true));
  return results;
}

// Keys either side has that the other lacks (array indices folded to []):
// JSON.stringify drops undefined-valued keys, so equal JSON can hide these.
const keyDiffs = new Set<string>();
function compareKeys(a: unknown, b: unknown, path: string) {
  if (Array.isArray(a) && Array.isArray(b)) {
    for (let i = 0; i < Math.min(a.length, b.length); i++) compareKeys(a[i], b[i], `${path}[]`);
    return;
  }
  if (a && b && typeof a === "object" && typeof b === "object") {
    const ak = Object.keys(a), bk = Object.keys(b);
    for (const k of ak) if (!(k in b)) keyDiffs.add(`${path}.${k}: only lang-core (value ${JSON.stringify((a as any)[k]) ?? "undefined"})`);
    for (const k of bk) if (!(k in a)) keyDiffs.add(`${path}.${k}: only openui-rs`);
    for (const k of ak) if (k in b) compareKeys((a as any)[k], (b as any)[k], `${path}.${k}`);
  }
}

let failures = 0;
const rows: string[] = [];
const totals = { js: 0, rust: 0 };
for (const program of programs) {
  for (const size of CHUNK_SIZES) {
    const chunks = chunk(program.text, size);
    // Equality, update by update.
    const js = stream("js", chunks), rust = stream("rust", chunks);
    let firstDiff = -1;
    for (let i = 0; i < js.length; i++) {
      compareKeys(js[i], rust[i], "result");
      if (firstDiff < 0 && JSON.stringify(js[i]) !== JSON.stringify(rust[i])) firstDiff = i;
    }
    if (firstDiff >= 0) {
      failures++;
      console.error(`DIFFERENT: ${program.name}, chunks of ${size}, update ${firstDiff + 1} of ${js.length}`);
    }
    const last = js[js.length - 1] as { root: unknown; meta: { errors: unknown[]; unresolved: string[] } };
    // Timings: warm up, then RUNS streams per engine, alternating.
    for (let w = 0; w < 3; w++) (stream("js", chunks), stream("rust", chunks));
    sinks.js.length = sinks.rust.length = 0;
    for (let r = 0; r < RUNS; r++) (stream("js", chunks), stream("rust", chunks));
    const j = summarize(sinks.js), s = summarize(sinks.rust);
    totals.js += j.total / RUNS;
    totals.rust += s.total / RUNS;
    rows.push(
      `| ${program.name} | ${size} | ${js.length} | ${(j.total / RUNS).toFixed(2)} | ${(s.total / RUNS).toFixed(2)} | ` +
        `${j.mean.toFixed(3)} / ${j.p95.toFixed(3)} | ${s.mean.toFixed(3)} / ${s.p95.toFixed(3)} | ` +
        `${(j.total / s.total).toFixed(1)}× | ${last.root ? "yes" : "NO"} | ${last.meta.errors.length} | ${firstDiff < 0 ? "yes" : "NO"} |`,
    );
  }
}

console.log(`Parser time per stream, mean of ${RUNS} runs after 3 warm-ups (Node ${process.version}); updates = set() calls incl. the final set(text, true).\n`);
console.log("| Program | Chunk | Updates | lang-core total (ms) | openui-rs total (ms) | lang-core mean / p95 (ms) | openui-rs mean / p95 (ms) | Speedup | Root | Errors | Equal at every update |");
console.log("|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---|");
for (const row of rows) console.log(row);
console.log(`\nAll programs, all chunk sizes: lang-core ${totals.js.toFixed(1)} ms, openui-rs ${totals.rust.toFixed(1)} ms (${(totals.js / totals.rust).toFixed(1)}×).`);
console.log(`Parser calls: JS ${parserCalls.js}, Rust ${parserCalls.rust}.`);
if (keyDiffs.size) {
  console.log("\nKey differences JSON.stringify hides:");
  for (const d of keyDiffs) console.log(`  ${d}`);
} else console.log("No key differences between the two engines' result objects.");
if (failures) {
  console.error(`\n${failures} program/chunk-size pairs gave different results.`);
  process.exit(1);
}
console.log("\nOK: both engines gave JSON-equal results at every update.");
