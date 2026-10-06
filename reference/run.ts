// Runs the reference parser (vendored @openuidev/lang-core) over the conformance corpus.
//
//   bun reference/run.ts golden                          write conformance/golden/ for every case
//   bun reference/run.ts show <case> <chunking> <step>   print the reference JSON for one step
//   bun reference/run.ts show-tokens <case> <length>     print tokenize() of the first <length> chars
//   bun reference/run.ts show-statements <case> <length> print the statement stages of the first <length> chars
//
// The corpus is conformance/manifest.json (hand-written) plus
// conformance/imported.json (from scripts/import-corpus.sh). A chunking is
// "oneshot", a chunk size in code points, or "random-<k>": sizes 1–16 drawn
// from a generator seeded with the case name and k. Steps count from 1.
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { dirname, join } from "path";

// Set before the parser module loads; telemetry must never run here.
process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { compileSchema, createStreamParser, parse } = await import(
  "./lang-core/src/parser/parser"
);
const { tokenize } = await import("./lang-core/src/parser/lexer");
const { autoClose, split } = await import("./lang-core/src/parser/statements");
const { parseExpression } = await import("./lang-core/src/parser/expressions");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const GOLDEN = join(CONFORMANCE, "golden");

type Case = { name: string; schema: string; file: string; source: string; root?: string };
type Manifest = { chunkings?: string[]; schemas: Record<string, string>; cases: Case[] };

function readManifest(file: string): Manifest {
  const path = join(CONFORMANCE, file);
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : { schemas: {}, cases: [] };
}
const hand = readManifest("manifest.json");
const imported = readManifest("imported.json");
const chunkings = hand.chunkings ?? [];
const schemaFiles = { ...hand.schemas, ...imported.schemas };
const cases = [...hand.cases, ...imported.cases];

const libraries = new Map<string, unknown>();
function library(name: string) {
  if (!libraries.has(name)) {
    const schema = JSON.parse(readFileSync(join(CONFORMANCE, schemaFiles[name]), "utf8"));
    libraries.set(name, compileSchema(schema));
  }
  return libraries.get(name);
}

const input = (c: Case) => readFileSync(join(CONFORMANCE, c.file), "utf8");
const oneshot = (c: Case) => JSON.stringify(parse(input(c), library(c.schema), c.root));

const encoder = new TextEncoder();
/** FNV-1a (32-bit) over UTF-8 bytes. */
function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (const b of encoder.encode(s)) {
    h ^= b;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h;
}
/** FNV-1a as 8 hex digits; mirrored in the Rust conformance test. */
const fingerprint = (s: string) => fnv1a(s).toString(16).padStart(8, "0");

/** Mulberry32: small, seedable, deterministic. */
function generator(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Chunk sizes in code points; random chunkings return them for the golden. */
function sizes(c: Case, chunking: string, length: number): { sizes: number[]; random: boolean } {
  const random = chunking.startsWith("random-");
  const next = random ? generator(fnv1a(`${c.name}:${chunking}`)) : null;
  const fixed = random ? 0 : Number(chunking);
  const out: number[] = [];
  for (let pos = 0; pos < length; ) {
    const size = Math.min(next ? 1 + Math.floor(next() * 16) : fixed, length - pos);
    out.push(size);
    pos += size;
  }
  return { sizes: out, random };
}

/** One JSON string per push, plus the chunk sizes used. */
function stream(c: Case, chunking: string) {
  const cps = Array.from(input(c));
  const { sizes: plan, random } = sizes(c, chunking, cps.length);
  const parser = createStreamParser(library(c.schema), c.root);
  const steps: string[] = [];
  let pos = 0;
  for (const size of plan) {
    steps.push(JSON.stringify(parser.push(cps.slice(pos, pos + size).join(""))));
    pos += size;
  }
  return { steps, sizes: random ? plan : undefined };
}

/** tokenize() of the first `length` code points, as JSON. */
function tokensOfPrefix(c: Case, length: number): string {
  return JSON.stringify(tokenize(Array.from(input(c)).slice(0, length).join("")));
}

/**
 * The statement stages of a prefix, as one JSON document: autoClose, then
 * split(tokenize(closed text)), each statement with parseExpression of its tokens.
 */
function statementsOf(text: string): string {
  const closed = autoClose(text);
  const statements = split(tokenize(closed.text)).map((s) => ({ ...s, expr: parseExpression(s.tokens) }));
  return JSON.stringify({ autoClose: closed, statements });
}

function writeGolden(rel: string, content: string) {
  const path = join(GOLDEN, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
}

const [command, ...args] = process.argv.slice(2);
if (command === "golden") {
  rmSync(GOLDEN, { recursive: true, force: true });
  for (const c of cases) {
    writeGolden(`${c.name}.oneshot.json`, oneshot(c));
    const streams: Record<string, { sizes?: number[]; steps: string[] }> = {};
    for (const chunking of chunkings) {
      const { steps, sizes } = stream(c, chunking);
      streams[chunking] = { ...(sizes ? { sizes } : {}), steps: steps.map(fingerprint) };
    }
    writeGolden(`${c.name}.steps.json`, JSON.stringify(streams) + "\n");
    // Stage golden: tokens of every prefix (index k is the first k+1 code points).
    const cps = Array.from(input(c));
    const prefixes = cps.map((_, k) => fingerprint(JSON.stringify(tokenize(cps.slice(0, k + 1).join("")))));
    writeGolden(`${c.name}.tokens.json`, JSON.stringify(prefixes) + "\n");
    const statements = cps.map((_, k) => fingerprint(statementsOf(cps.slice(0, k + 1).join(""))));
    writeGolden(`${c.name}.statements.json`, JSON.stringify(statements) + "\n");
  }
  console.log(`Wrote goldens for ${cases.length} cases × ${chunkings.length} chunkings to conformance/golden/`);
} else if (command === "show" && args.length === 3) {
  const [name, chunking, step] = args;
  const c = cases.find((x) => x.name === name);
  if (!c) throw new Error(`no case named ${name}`);
  process.stdout.write(chunking === "oneshot" ? oneshot(c) : stream(c, chunking).steps[Number(step) - 1]);
} else if (command === "show-tokens" && args.length === 2) {
  const [name, length] = args;
  const c = cases.find((x) => x.name === name);
  if (!c) throw new Error(`no case named ${name}`);
  process.stdout.write(tokensOfPrefix(c, Number(length)));
} else if (command === "show-statements" && args.length === 2) {
  const [name, length] = args;
  const c = cases.find((x) => x.name === name);
  if (!c) throw new Error(`no case named ${name}`);
  process.stdout.write(statementsOf(Array.from(input(c)).slice(0, Number(length)).join("")));
} else {
  console.error(
    "usage: bun reference/run.ts golden | show <case> <chunking> <step> | show-tokens <case> <length> | show-statements <case> <length>",
  );
  process.exit(1);
}
