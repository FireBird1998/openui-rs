// The core as WebAssembly against the reference parser, in one JavaScript
// engine: the same workload as bench.ts (chunks of 4 code points, 5 warm-ups,
// mean of BENCH_RUNS runs, default 20), every program warmed up before any is
// timed. The core is timed three ways, since a result has to cross into
// JavaScript:
//   objects   push, then JSON.parse the result (what a renderer needs each push)
//   text      push, the result left as JSON text
//   feed      push without writing the result, then one result at the end
// Prints a markdown table, and checks the last result equals the reference's.
//
//   bun reference/bench-wasm.ts <schema.json> <program.oui>...
// Needs the Node build of crates/openui-wasm in target/wasm-pkg/node
// (scripts/wasm.sh).
import { readFileSync } from "fs";
import { join } from "path";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { compileSchema, createStreamParser } = await import("./lang-core/src/parser/parser");
const wasm = require(join(import.meta.dir, "../target/wasm-pkg/node/openui_wasm.js"));

const [schemaPath, ...files] = process.argv.slice(2);
if (!schemaPath || files.length === 0) {
  console.error("usage: bun reference/bench-wasm.ts <schema.json> <program.oui>...");
  process.exit(1);
}
const schemaText = readFileSync(schemaPath, "utf8");
const cat = compileSchema(JSON.parse(schemaText));
const library = new wasm.ComponentLibrary(schemaText);
const runs = Math.max(1, Number(process.env.BENCH_RUNS ?? 20) || 20);

const programs = files.map((file) => {
  const cps = Array.from(readFileSync(file, "utf8"));
  const chunks: string[] = [];
  for (let i = 0; i < cps.length; i += 4) chunks.push(cps.slice(i, i + 4).join(""));
  return { name: file.split("/").pop()!.replace(/\.oui$/, ""), cps: cps.length, chunks };
});

type Way = (chunks: string[]) => unknown;
const ways: Record<string, Way> = {
  reference: (chunks) => {
    const parser = createStreamParser(cat);
    let last: unknown;
    for (const chunk of chunks) last = parser.push(chunk);
    return last;
  },
  objects: (chunks) => {
    const session = new wasm.StreamSession(library);
    let last: unknown;
    for (const chunk of chunks) last = JSON.parse(session.push(chunk));
    session.free();
    return last;
  },
  text: (chunks) => {
    const session = new wasm.StreamSession(library);
    let last = "";
    for (const chunk of chunks) last = session.push(chunk);
    session.free();
    return last;
  },
  feed: (chunks) => {
    const session = new wasm.StreamSession(library);
    for (const chunk of chunks) session.feed(chunk);
    const last = JSON.parse(session.result());
    session.free();
    return last;
  },
};

// Warm every way on every program before timing any: whichever runs first
// otherwise pays for the engine warming up.
for (const way of Object.values(ways)) for (const p of programs) for (let w = 0; w < Math.min(5, runs); w++) way(p.chunks);

// The reference's result has functions and undefineds JSON drops; compare what both write.
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

console.log("| Program | Code points | Pushes | Reference (ms) | Core, objects (ms) | Core, text (ms) | Core, feed (ms) | Speedup, objects | Same result |");
console.log("|---|---:|---:|---:|---:|---:|---:|---:|---|");
for (const p of programs) {
  const ms: Record<string, number> = {};
  for (const [name, way] of Object.entries(ways)) {
    const started = performance.now();
    for (let r = 0; r < runs; r++) way(p.chunks);
    ms[name] = (performance.now() - started) / runs;
  }
  const equal = same(ways.reference(p.chunks), ways.objects(p.chunks)) ? "yes" : "NO";
  console.log(
    `| ${p.name} | ${p.cps} | ${p.chunks.length} | ${ms.reference.toFixed(2)} | ${ms.objects.toFixed(2)} | ${ms.text.toFixed(2)} | ${ms.feed.toFixed(2)} | ${(ms.reference / ms.objects).toFixed(1)}× | ${equal} |`,
  );
}
