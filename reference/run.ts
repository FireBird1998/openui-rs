// Runs the reference parser (vendored @openuidev/lang-core) over the conformance corpus.
//
//   bun reference/run.ts golden                          write conformance/golden/ for every case
//   bun reference/run.ts show <case> <chunking> <step>   print the reference JSON for one step
//   bun reference/run.ts show-tokens <case> <length>     print tokenize() of the first <length> chars
//   bun reference/run.ts show-statements <case> <length> print the statement stages of the first <length> chars
//   bun reference/run.ts show-preprocess <case> <length> print stripFences and preprocess of the first <length> chars
//
// The corpus is conformance/manifest.json (hand-written), plus
// conformance/imported.json (from scripts/import-corpus.sh) and
// conformance/fuzz.json (shrunk mismatches from the fuzzer). A chunking is
// "oneshot", a chunk size in code points, "random-<k>" (push sizes 1–16 drawn
// from a generator seeded with the case name and k), "recorded" (the sizes in
// the case's `recorded`, for cases that came with their own chunks), or
// "resync-<k>": set() calls with the text so far that mostly grow, sometimes
// rewind (reset), and sometimes switch to an edited copy of the program (reset
// once they differ), then one getResult(). Every stream then ends: its last
// step is getResult(true), the result once the stream is complete. Steps count
// from 1.
import { plugin } from "bun";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { dirname, join } from "path";

// parser.ts keeps stripComments and preprocess to itself; export them as the
// module loads so their output can be checked stage by stage. The vendored
// file itself stays untouched.
plugin({
  name: "expose-parser-internals",
  setup(build) {
    build.onLoad({ filter: /lang-core\/src\/parser\/parser\.ts$/ }, ({ path }) => ({
      contents: readFileSync(path, "utf8") + "\nexport { stripComments, preprocess };\n",
      loader: "ts",
    }));
  },
});

// Set before the parser module loads; telemetry must never run here.
process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { compileSchema, createStreamParser, parse, preprocess, stripFences } = await import(
  "./lang-core/src/parser/parser"
);
const { tokenize } = await import("./lang-core/src/parser/lexer");
const { autoClose, split } = await import("./lang-core/src/parser/statements");
const { parseExpression } = await import("./lang-core/src/parser/expressions");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const GOLDEN = join(CONFORMANCE, "golden");

// Large programs cost the square of their length to golden at every prefix
// and every step: `stages: false` skips the per-prefix stage goldens, and
// `chunkings` limits a case to some of the manifest's chunkings.
type Case = {
  name: string;
  schema: string;
  file: string;
  source: string;
  root?: string;
  stages?: boolean;
  chunkings?: string[];
  /** Chunk sizes in code points, for a case that came with its own chunks. */
  recorded?: number[];
};
type Manifest = { chunkings?: string[]; schemas: Record<string, string>; cases: Case[] };

function readManifest(file: string): Manifest {
  const path = join(CONFORMANCE, file);
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : { schemas: {}, cases: [] };
}
const hand = readManifest("manifest.json");
const imported = readManifest("imported.json");
const fuzz = readManifest("fuzz.json");
const synthetic = readManifest("synthetic.json");
const recorded = readManifest("recorded.json");
const chunkings = hand.chunkings ?? [];
const schemaFiles = { ...hand.schemas, ...imported.schemas, ...fuzz.schemas, ...synthetic.schemas, ...recorded.schemas };
const cases = [...hand.cases, ...imported.cases, ...fuzz.cases, ...synthetic.cases, ...recorded.cases];

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
  if (chunking === "recorded") return { sizes: c.recorded!, random: false };
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

/** The program with its middle code point changed, so its prefixes past the
 *  middle aren't continuations of the program's own. */
function edited(cps: string[]): string[] {
  const copy = [...cps];
  const m = Math.floor(copy.length / 2);
  if (copy.length) copy[m] = copy[m] === "Z" ? "Y" : "Z";
  return copy;
}

type ResyncOp = ["p" | "x", number] | ["r", 0] | ["e", 0];

/** A seeded sequence of set() calls: ["p", n] sets the program's first n code
 *  points, ["x", n] the edited copy's, then ["r", 0] is getResult() and a
 *  final ["e", 0] getResult(true), the stream's end. */
function resyncOps(c: Case, chunking: string, n: number): ResyncOp[] {
  const next = generator(fnv1a(`${c.name}:${chunking}`));
  const ops: ResyncOp[] = [];
  let kind: "p" | "x" = "p";
  let length = 0;
  while (!(kind === "p" && length === n) && ops.length < 2 * n + 64) {
    const r = next();
    if (r < 0.15 && length > 0) length = Math.max(0, length - 1 - Math.floor(next() * 20));
    else if (r < 0.3) {
      kind = kind === "p" ? "x" : "p";
      length = Math.min(n, length + Math.floor(next() * 4));
    } else length = Math.min(n, length + 1 + Math.floor(next() * 16));
    ops.push([kind, length]);
  }
  if (!(kind === "p" && length === n)) ops.push(["p", n]);
  ops.push(["r", 0], ["e", 0]);
  return ops;
}

/** One JSON string per push, plus the chunk sizes used. */
function stream(c: Case, chunking: string) {
  if (chunking.startsWith("resync-")) {
    const cps = Array.from(input(c));
    const other = edited(cps);
    const ops = resyncOps(c, chunking, cps.length);
    const parser = createStreamParser(library(c.schema), c.root);
    const steps = ops.map(([kind, n]) =>
      JSON.stringify(
        kind === "e"
          ? parser.getResult(true)
          : kind === "r"
            ? parser.getResult()
            : parser.set((kind === "p" ? cps : other).slice(0, n).join("")),
      ),
    );
    return { steps, sizes: undefined, ops };
  }
  const cps = Array.from(input(c));
  const { sizes: plan, random } = sizes(c, chunking, cps.length);
  const parser = createStreamParser(library(c.schema), c.root);
  const steps: string[] = [];
  let pos = 0;
  for (const size of plan) {
    steps.push(JSON.stringify(parser.push(cps.slice(pos, pos + size).join(""))));
    pos += size;
  }
  steps.push(JSON.stringify(parser.getResult(true)));
  return { steps, sizes: random || chunking === "recorded" ? plan : undefined, ops: undefined };
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

/** stripFences and the whole preprocess (trim, fences, comments, trim) of a prefix. */
function preprocessOf(text: string): string {
  return JSON.stringify({ stripFences: stripFences(text), preprocess: preprocess(text) });
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
    const streams: Record<string, { sizes?: number[]; ops?: ResyncOp[]; steps: string[] }> = {};
    for (const chunking of [...(c.chunkings ?? chunkings), ...(c.recorded ? ["recorded"] : [])]) {
      const { steps, sizes, ops } = stream(c, chunking);
      streams[chunking] = { ...(sizes ? { sizes } : {}), ...(ops ? { ops } : {}), steps: steps.map(fingerprint) };
    }
    writeGolden(`${c.name}.steps.json`, JSON.stringify(streams) + "\n");
    if (c.stages === false) continue;
    // Stage golden: tokens of every prefix (index k is the first k+1 code points).
    const cps = Array.from(input(c));
    const prefixes = cps.map((_, k) => fingerprint(JSON.stringify(tokenize(cps.slice(0, k + 1).join("")))));
    writeGolden(`${c.name}.tokens.json`, JSON.stringify(prefixes) + "\n");
    const statements = cps.map((_, k) => fingerprint(statementsOf(cps.slice(0, k + 1).join(""))));
    writeGolden(`${c.name}.statements.json`, JSON.stringify(statements) + "\n");
    const preprocessed = cps.map((_, k) => fingerprint(preprocessOf(cps.slice(0, k + 1).join(""))));
    writeGolden(`${c.name}.preprocess.json`, JSON.stringify(preprocessed) + "\n");
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
} else if (command === "show-preprocess" && args.length === 2) {
  const [name, length] = args;
  const c = cases.find((x) => x.name === name);
  if (!c) throw new Error(`no case named ${name}`);
  process.stdout.write(preprocessOf(Array.from(input(c)).slice(0, Number(length)).join("")));
} else {
  console.error(
    "usage: bun reference/run.ts golden | show <case> <chunking> <step> | show-tokens <case> <length> | show-statements <case> <length> | show-preprocess <case> <length>",
  );
  process.exit(1);
}
