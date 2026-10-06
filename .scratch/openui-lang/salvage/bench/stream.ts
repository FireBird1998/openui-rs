// Streaming-parse timing for the TS parser; mirrors `openui-lang bench`.
//   bun bench/stream.ts <schema.json> <file.oui>...
import { readFileSync } from "fs";
import { compileSchema, createStreamParser } from "../../../packages/lang-core/src/parser/parser";

const [schemaPath, ...files] = process.argv.slice(2);
const cat = compileSchema(JSON.parse(readFileSync(schemaPath, "utf8")));
for (const f of files) {
  const text = readFileSync(f, "utf8");
  const cps = Array.from(text);
  const chunks: string[] = [];
  for (let i = 0; i < cps.length; i += 4) chunks.push(cps.slice(i, i + 4).join(""));
  const once = () => {
    const p = createStreamParser(cat);
    for (const c of chunks) p.push(c);
  };
  for (let w = 0; w < 5; w++) once();
  const runs = 20;
  const t0 = performance.now();
  for (let r = 0; r < runs; r++) once();
  const ms = (performance.now() - t0) / runs;
  console.log(`${f.split("/").pop()}\t${text.length}\t${chunks.length}\t${ms.toFixed(2)}`);
}
