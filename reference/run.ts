// Runs the reference parser (vendored @openuidev/lang-core) over the conformance corpus.
//
//   bun reference/run.ts golden                          write conformance/golden/ for every case
//   bun reference/run.ts show <case> <chunking> <step>   print the reference JSON for one step
//
// A chunking is "oneshot" or a chunk size in code points. Steps count from 1.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { join } from "path";

// Set before the parser module loads; telemetry must never run here.
process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { compileSchema, createStreamParser, parse } = await import(
  "./lang-core/src/parser/parser"
);

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const GOLDEN = join(CONFORMANCE, "golden");

type Case = { name: string; schema: string; file: string; source: string; root?: string };
type Manifest = { chunkings: number[]; schemas: Record<string, string>; cases: Case[] };

const manifest: Manifest = JSON.parse(readFileSync(join(CONFORMANCE, "manifest.json"), "utf8"));
const libraries = new Map<string, unknown>();
function library(name: string) {
  if (!libraries.has(name)) {
    const schema = JSON.parse(readFileSync(join(CONFORMANCE, manifest.schemas[name]), "utf8"));
    libraries.set(name, compileSchema(schema));
  }
  return libraries.get(name);
}

function input(c: Case): string {
  return readFileSync(join(CONFORMANCE, c.file), "utf8");
}

function oneshot(c: Case): string {
  return JSON.stringify(parse(input(c), library(c.schema), c.root));
}

/** Push the input in chunks of `size` code points; one JSON string per push. */
function stream(c: Case, size: number): string[] {
  const cps = Array.from(input(c));
  const parser = createStreamParser(library(c.schema), c.root);
  const out: string[] = [];
  for (let pos = 0; pos < cps.length; pos += size) {
    out.push(JSON.stringify(parser.push(cps.slice(pos, pos + size).join(""))));
  }
  return out;
}

const encoder = new TextEncoder();
/** "<fnv1a-32 hex>:<byte length>" over UTF-8 bytes; mirrored in the Rust conformance test. */
function fingerprint(s: string): string {
  const bytes = encoder.encode(s);
  let h = 0x811c9dc5;
  for (const b of bytes) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return `${h.toString(16).padStart(8, "0")}:${bytes.length}`;
}

const [command, ...args] = process.argv.slice(2);
if (command === "golden") {
  rmSync(GOLDEN, { recursive: true, force: true });
  mkdirSync(GOLDEN, { recursive: true });
  for (const c of manifest.cases) {
    writeFileSync(join(GOLDEN, `${c.name}.oneshot.json`), oneshot(c));
    const streams: Record<string, string[]> = {};
    for (const size of manifest.chunkings) streams[String(size)] = stream(c, size).map(fingerprint);
    writeFileSync(join(GOLDEN, `${c.name}.steps.json`), JSON.stringify(streams, null, 1) + "\n");
  }
  console.log(`Wrote goldens for ${manifest.cases.length} case(s) to conformance/golden/`);
} else if (command === "show" && args.length === 3) {
  const [name, chunking, step] = args;
  const c = manifest.cases.find((x) => x.name === name);
  if (!c) throw new Error(`no case named ${name}`);
  process.stdout.write(chunking === "oneshot" ? oneshot(c) : stream(c, Number(chunking))[Number(step) - 1]);
} else {
  console.error("usage: bun reference/run.ts golden | show <case> <chunking> <step>");
  process.exit(1);
}
