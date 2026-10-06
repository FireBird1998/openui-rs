// Runs the TypeScript reference parser (@openuidev/lang-core source) over a
// batch of conformance cases written by tests/conformance.rs.
//
//   bun conformance/ts-reference.ts <cases.json>                 → one line of step hashes per case
//   bun conformance/ts-reference.ts <cases.json> --full <c> <s>  → full JSON of case c, step s
import { readFileSync } from "fs";
import {
  compileSchema,
  createStreamParser,
  parse,
} from "../../../packages/lang-core/src/parser/parser";

type Case = { schema: string; input: string; chunks?: number[]; root?: string };
type CasesFile = { schemas: Record<string, any>; cases: Case[] };

const [casesPath, flag, caseArg, stepArg] = process.argv.slice(2);
const { schemas, cases } = JSON.parse(readFileSync(casesPath, "utf8")) as CasesFile;
const cats = Object.fromEntries(Object.entries(schemas).map(([k, s]) => [k, compileSchema(s)]));

const encoder = new TextEncoder();
/** FNV-1a (32-bit) over UTF-8 bytes; mirrored in tests/conformance.rs. */
function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (const b of encoder.encode(s)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

function run(c: Case): string[] {
  const cat = cats[c.schema];
  if (!c.chunks) return [JSON.stringify(parse(c.input, cat, c.root))];
  // Chunk by code points so neither side splits a surrogate pair.
  const cps = Array.from(c.input);
  const p = createStreamParser(cat, c.root);
  const out: string[] = [];
  let pos = 0;
  for (const size of c.chunks) {
    out.push(JSON.stringify(p.push(cps.slice(pos, pos + size).join(""))));
    pos += size;
  }
  return out;
}

if (flag === "--full") {
  console.log(run(cases[Number(caseArg)])[Number(stepArg)]);
} else {
  const lines = cases.map((c) => JSON.stringify(run(c).map(fnv1a)));
  process.stdout.write(lines.join("\n") + "\n");
}
