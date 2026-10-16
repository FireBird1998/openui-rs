// A long-lived reference parser for differential fuzzing. Reads one JSON
// request per line on stdin and writes one JSON response per line:
//
//   {"lib": "<schema name>", "input": "...", "root"?: "...", "sizes": [n, ...]}
//   → {"oneshot": fp, "tokens": fp, "steps": [fp, ...], "end": fp, "ms": n}   or   {"threw": "<error>"}
//   {"op": "merge", "existing": "...", "patch": "...", "rootId"?: "..."}
//   → {"text": "..."}   or   {"threw": "<error>"}
//   {"op": "serialize", "lib": "<schema name>", "element": {...}, "state"?: {...}}
//   → {"text": "..."}   or   {"threw": "<error>"}
//
// fp is the FNV-1a fingerprint the goldens use, end the stream's result once it
// has ended (getResult(true)), and ms how long the reference took. Schema names are those in the conformance manifests. Sizes are push
// chunk sizes in code points.
import { existsSync, readFileSync } from "fs";
import { join } from "path";
import { createInterface } from "readline";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { compileSchema, createStreamParser, parse } = await import("./lang-core/src/parser/parser");
const { tokenize } = await import("./lang-core/src/parser/lexer");
const { mergeStatements } = await import("./lang-core/src/parser/merge");
const { jsonToOpenUI } = await import("./lang-core/src/parser/serialize");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const schemaFiles: Record<string, string> = {};
for (const file of ["manifest.json", "imported.json", "fuzz.json"]) {
  const path = join(CONFORMANCE, file);
  if (existsSync(path)) Object.assign(schemaFiles, JSON.parse(readFileSync(path, "utf8")).schemas);
}
const schemas = new Map<string, unknown>();
function schema(name: string) {
  if (!schemas.has(name)) schemas.set(name, JSON.parse(readFileSync(join(CONFORMANCE, schemaFiles[name]), "utf8")));
  return schemas.get(name);
}
const libraries = new Map<string, unknown>();
function library(name: string) {
  if (!libraries.has(name)) {
    // Compiled fresh per name and cached, as run.ts does.
    libraries.set(name, compileSchema(JSON.parse(readFileSync(join(CONFORMANCE, schemaFiles[name]), "utf8"))));
  }
  return libraries.get(name);
}

const encoder = new TextEncoder();
function fingerprint(s: string): string {
  let h = 0x811c9dc5;
  for (const b of encoder.encode(s)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

type Request = { lib: string; input: string; root?: string; sizes: number[] };

function answer(req: Request) {
  try {
    const started = performance.now();
    const lib = library(req.lib);
    const oneshot = fingerprint(JSON.stringify(parse(req.input, lib, req.root)));
    const tokens = fingerprint(JSON.stringify(tokenize(req.input)));
    const cps = Array.from(req.input);
    const parser = createStreamParser(lib, req.root);
    const steps: string[] = [];
    let pos = 0;
    for (const size of req.sizes) {
      steps.push(fingerprint(JSON.stringify(parser.push(cps.slice(pos, pos + size).join("")))));
      pos += size;
    }
    const end = fingerprint(JSON.stringify(parser.getResult(true)));
    return { oneshot, tokens, steps, end, ms: performance.now() - started };
  } catch (e) {
    return { threw: String(e) };
  }
}

type EditRequest =
  | { op: "merge"; existing: string; patch: string; rootId?: string }
  | { op: "serialize"; lib: string; element: unknown; state?: unknown };

function answerEdit(req: EditRequest) {
  try {
    if (req.op === "merge") return { text: mergeStatements(req.existing, req.patch, req.rootId) };
    const s = schema(req.lib);
    const lib = { toJSONSchema: () => s } as any;
    return { text: jsonToOpenUI(req.element as any, lib, "state" in req ? { stateDeclarations: req.state as any } : undefined) };
  } catch (e) {
    return { threw: String(e) };
  }
}

for await (const line of createInterface({ input: process.stdin })) {
  if (!line.trim()) continue;
  const req = JSON.parse(line);
  process.stdout.write(JSON.stringify(req.op ? answerEdit(req) : answer(req)) + "\n");
}
