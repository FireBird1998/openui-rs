// Streaming-parse timing for the reference parser: the same workload as
// `openui-lang bench` (chunks of 4 code points, 5 warm-ups, mean of BENCH_RUNS
// runs, default 20). Prints `name<TAB>code points<TAB>chunks<TAB>ms`.
//
//   bun reference/bench.ts <schema.json> <program.oui>...
import { readFileSync } from "fs";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { compileSchema, createStreamParser } = await import("./lang-core/src/parser/parser");

const [schemaPath, ...files] = process.argv.slice(2);
if (!schemaPath || files.length === 0) {
  console.error("usage: bun reference/bench.ts <schema.json> <program.oui>...");
  process.exit(1);
}
const cat = compileSchema(JSON.parse(readFileSync(schemaPath, "utf8")));
const runs = Math.max(1, Number(process.env.BENCH_RUNS ?? 20) || 20);

for (const file of files) {
  const text = readFileSync(file, "utf8");
  const cps = Array.from(text);
  const chunks: string[] = [];
  for (let i = 0; i < cps.length; i += 4) chunks.push(cps.slice(i, i + 4).join(""));
  const once = () => {
    const parser = createStreamParser(cat);
    for (const chunk of chunks) parser.push(chunk);
  };
  for (let w = 0; w < Math.min(5, runs); w++) once();
  const started = performance.now();
  for (let r = 0; r < runs; r++) once();
  const ms = (performance.now() - started) / runs;
  console.log(`${file.split("/").pop()}\t${cps.length}\t${chunks.length}\t${ms.toFixed(2)}`);
}
