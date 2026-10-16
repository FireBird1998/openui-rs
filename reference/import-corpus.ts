// Rebuilds conformance/imported/ from upstream files at the pinned commit.
// Run through scripts/import-corpus.sh, which fetches those files first.
//
//   bun import-corpus.ts <dir holding benchmarks/, docs/content/, examples/ and spec/>
//
// Sources, each case recording where it came from:
//   test/      every program the reference test suites parse (recorded live)
//   spec/      the spec's conformance fixtures (spec/fixtures/1.0)
//   bench/     the benchmark samples: real LLM output
//   docs/      programs in the docs and blog (code fences, template strings)
//   examples/  programs in the examples folder (prompts, READMEs)
import { spawnSync } from "child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { basename, dirname, extname, join, relative } from "path";

const upstream = process.argv[2];
if (!upstream) throw new Error("usage: bun import-corpus.ts <upstream dir>");
const HERE = import.meta.dir;
const OUT = join(HERE, "..", "conformance", "imported");

type Case = {
  name: string;
  schema: string;
  file: string;
  source: string;
  origin: string;
  root?: string;
  recorded?: number[];
};
const schemas: Record<string, string> = {};
const cases: Case[] = [];
const seen = new Set<string>();

function write(rel: string, content: string) {
  const path = join(OUT, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
}

function addSchema(name: string, schema: unknown) {
  write(`schemas/${name}.json`, JSON.stringify(schema, null, 1) + "\n");
  schemas[name] = `imported/schemas/${name}.json`;
}

const counters = new Map<string, number>();
/** Adds a case unless the same program, library and root are already in. */
function addCase(prefix: string, schema: string, input: string, source: string, origin: string, root?: string) {
  const key = `${schema}\0${root ?? ""}\0${input}`;
  if (seen.has(key)) return;
  seen.add(key);
  const n = (counters.get(prefix) ?? 0) + 1;
  counters.set(prefix, n);
  const name = `${prefix}-${String(n).padStart(2, "0")}`;
  write(`cases/${name}.oui`, input);
  cases.push({ name, schema, file: `imported/cases/${name}.oui`, source, origin, ...(root ? { root } : {}) });
}

// ── Reference test suites ────────────────────────────────────────────────────
// Run every test file of the package, with capture-tests.ts recording each
// program parsed and the component library it was parsed against. Not the
// conformance runner: it reads the spec's fixtures, imported below instead.
// Nor the query manager's tests: they parse nothing, and use vitest's fake
// timers, which bun test doesn't have.
const SKIPPED_TESTS = new Set(["conformance.test.ts", "queryManager.test.ts"]);
const testDirs = ["src/parser/__tests__", "src/__tests__", "src/runtime/__tests__"];
const testDirOf = new Map<string, string>();
for (const dir of testDirs) {
  if (!existsSync(join(HERE, "lang-core", dir))) continue;
  for (const f of readdirSync(join(HERE, "lang-core", dir))) {
    if (f.endsWith(".test.ts") && !SKIPPED_TESTS.has(f)) testDirOf.set(f, dir);
  }
}
const testFiles = [...testDirOf.keys()].sort();
const origin = (test: string) => `packages/lang-core/${testDirOf.get(test)}/${test}`;
const captureOut = join(OUT, "..", ".capture.json");
rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const run = spawnSync(
  "bun",
  ["test", "--preload", "../capture-tests.ts", ...testFiles.map((f) => join(HERE, "lang-core", testDirOf.get(f)!, f))],
  { cwd: join(HERE, "lang-core"), env: { ...process.env, CAPTURE_OUT: captureOut }, encoding: "utf8" },
);
if (run.status !== 0) throw new Error(`reference tests failed:\n${run.stderr}`);
const capture: {
  parses: { test: string; schema: unknown; input: string; root?: string }[];
  merges: { test: string; existing: string; patch: string; rootId?: string }[];
  serializes: { test: string; element: unknown; schema: unknown; stateDeclarations?: unknown }[];
} = JSON.parse(
  readFileSync(captureOut, "utf8"),
);
rmSync(captureOut);
const testSchemas = new Map<string, string>();
for (const c of capture.parses) {
  const stem = c.test.replace(/\.test\.ts$/, "").replace(/\./g, "-");
  const json = JSON.stringify(c.schema);
  if (!testSchemas.has(json)) {
    const used = [...testSchemas.values()].filter((n) => n.startsWith(`test-${stem}`)).length;
    const name = used ? `test-${stem}-${used + 1}` : `test-${stem}`;
    testSchemas.set(json, name);
    addSchema(name, c.schema);
  }
  addCase(`test/${stem}`, testSchemas.get(json)!, c.input, "reference-test", origin(c.test), c.root);
}

// The tests' mergeStatements and jsonToOpenUI calls, with their arguments as
// JSON carries them, each distinct call once.
const stemOf = (test: string) => test.replace(/\.test\.ts$/, "").replace(/\./g, "-");
const editCases = <T extends { test: string }>(calls: T[], make: (c: T) => object) => {
  const seenCalls = new Set<string>();
  const counts = new Map<string, number>();
  const out: object[] = [];
  for (const c of calls) {
    const { test, ...args } = c;
    const key = JSON.stringify(args);
    if (seenCalls.has(key)) continue;
    seenCalls.add(key);
    const stem = stemOf(test);
    const n = (counts.get(stem) ?? 0) + 1;
    counts.set(stem, n);
    out.push({
      name: `test/${stem}-${String(n).padStart(2, "0")}`,
      ...make(c),
      source: "reference-test",
      origin: origin(test),
    });
  }
  return out;
};
const merges: object[] = editCases(capture.merges, ({ test: _, ...args }) => args);
const editSchemas = new Map<string, string>();
const serializeCases = editCases(capture.serializes, ({ test, schema, ...args }) => {
  const json = JSON.stringify(schema);
  if (!editSchemas.has(json)) editSchemas.set(json, `test-${stemOf(test)}-lib${editSchemas.size ? `-${editSchemas.size + 1}` : ""}`);
  return { schema: editSchemas.get(json), ...args };
});
writeFileSync(
  join(OUT, "serialize.json"),
  JSON.stringify(
    { schemas: Object.fromEntries([...editSchemas].map(([json, name]) => [name, JSON.parse(json)])), cases: serializeCases },
    null,
    1,
  ) + "\n",
);

// ── The spec's conformance fixtures ──────────────────────────────────────────
// spec/fixtures/1.0: the spec's own cases, over its test library. Their
// expected results describe an evaluated tree, which the core doesn't build,
// so only their inputs come in, and the goldens are the reference's parse:
// each program a parse case (one that arrives in chunks keeps them as its
// recorded chunking), each edit a merge case too, and the library a prompt
// spec. Validation-rule cases check the renderer, so they're left out.
const FIXTURES = join(upstream, "spec/fixtures/1.0");
let fixtureCount = 0;
let fixtureEdits = 0;
if (existsSync(FIXTURES)) {
  const library = JSON.parse(readFileSync(join(FIXTURES, "library.json"), "utf8"));
  addSchema("spec-1.0", library.schema);
  const folders = (dir: string) =>
    readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name)
      .sort();
  for (const area of folders(FIXTURES)) {
    for (const fixture of folders(join(FIXTURES, area))) {
      const dir = join(FIXTURES, area, fixture);
      const at = `spec/fixtures/1.0/${area}/${fixture}`;
      const name = `spec/${area}/${fixture}`;
      const file = (f: string) => (existsSync(join(dir, f)) ? readFileSync(join(dir, f), "utf8") : undefined);
      const [input, chunks, patch] = [file("input.oui"), file("chunks.json"), file("patch.oui")];
      let program = input;
      let recorded: number[] | undefined;
      if (chunks !== undefined) {
        const parts: string[] = JSON.parse(chunks);
        program = parts.join("");
        recorded = parts.map((part) => Array.from(part).length);
      }
      if (program === undefined) continue;
      fixtureCount++;
      write(`cases/${name}.oui`, program);
      cases.push({
        name,
        schema: "spec-1.0",
        file: `imported/cases/${name}.oui`,
        source: "spec-fixture",
        origin: at,
        root: library.root,
        ...(recorded ? { recorded } : {}),
      });
      if (patch !== undefined) {
        merges.push({ name, existing: program, patch, source: "spec-fixture", origin: at });
        fixtureEdits++;
      }
    }
  }
  // The fixture library as the system prompt's input, plain and with every option on.
  const prompts = join(HERE, "..", "conformance", "prompts");
  writeFileSync(join(prompts, "spec-1.0-library.json"), JSON.stringify({ library }, null, 1) + "\n");
  writeFileSync(
    join(prompts, "spec-1.0-library-tools.json"),
    JSON.stringify({ library, promptOptions: { toolCalls: true, bindings: true } }, null, 1) + "\n",
  );
}
writeFileSync(join(OUT, "merge.json"), JSON.stringify({ cases: merges }, null, 1) + "\n");

// ── Benchmark samples ────────────────────────────────────────────────────────
addSchema("bench", JSON.parse(readFileSync(join(upstream, "benchmarks/schema.json"), "utf8")));
const samples = join(upstream, "benchmarks/samples");
for (const f of readdirSync(samples).filter((f) => f.endsWith(".oui")).sort()) {
  const name = `bench/${basename(f, ".oui")}`;
  write(`cases/${name}.oui`, readFileSync(join(samples, f), "utf8"));
  cases.push({ name, schema: "bench", file: `imported/cases/${name}.oui`, source: "benchmark", origin: `benchmarks/samples/${f}` });
}

// ── Docs and examples ────────────────────────────────────────────────────────
const STATEMENT = /^[ \t]*\$?[A-Za-z_][A-Za-z0-9_]*[ \t]*=(?!=)/m;
const CALL = /[A-Z][A-Za-z0-9_]*\(/;
const looksLikeProgram = (t: string) => t.length >= 12 && STATEMENT.test(t) && CALL.test(t);
const CODE_LANGS = new Set(["ts", "tsx", "js", "jsx", "mjs", "typescript", "javascript"]);
const FENCE = /^([ \t]*)(`{3,}|~{3,})[ \t]*([\w-]*)[^\n]*\n([\s\S]*?)\n[ \t]*\2[ \t]*$/gm;

/** Program text in a source file's template literals and double-quoted strings. */
function fromCode(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/`((?:[^`\\]|\\[\s\S])*)`/g)) {
    const raw = m[1];
    // Without `${` (and with no bare backtick, by the regex) this is a plain literal.
    if (raw.includes("${") || !looksLikeProgram(raw)) continue;
    try {
      out.push(new Function("return `" + raw + "`")());
    } catch {}
  }
  for (const m of text.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)) {
    let s: string;
    try {
      s = JSON.parse(`"${m[1]}"`);
    } catch {
      continue;
    }
    if (looksLikeProgram(s)) out.push(s);
  }
  return out;
}

/** Program text in Markdown code fences, plus any in JSX/TS outside the fences. */
function fromMarkdown(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(FENCE)) {
    const [, indent, , lang, body] = m;
    const dedented = indent
      ? body.split("\n").map((l) => (l.startsWith(indent) ? l.slice(indent.length) : l)).join("\n")
      : body;
    if (CODE_LANGS.has(lang.toLowerCase())) out.push(...fromCode(dedented));
    else if (looksLikeProgram(dedented)) out.push(dedented);
  }
  out.push(...fromCode(text.replace(FENCE, "")));
  return out;
}

function walk(dir: string): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name))
    .flatMap((e) => {
      if (["node_modules", ".next", "dist", "build", "generated"].includes(e.name)) return [];
      const path = join(dir, e.name);
      return e.isDirectory() ? walk(path) : [path];
    });
}

function importPrograms(root: string, source: string, slugOf: (rel: string) => string) {
  for (const path of walk(join(upstream, root))) {
    const ext = extname(path);
    const markdown = ext === ".md" || ext === ".mdx";
    if (!markdown && ![".ts", ".tsx", ".js", ".mjs"].includes(ext)) continue;
    const text = readFileSync(path, "utf8");
    const rel = relative(upstream, path);
    for (const program of markdown ? fromMarkdown(text) : fromCode(text)) {
      addCase(slugOf(rel), "bench", program, source, rel);
    }
  }
}

const slug = (parts: string[]) => parts.join("-").replace(/\.[^.]+$/, "").replace(/[^A-Za-z0-9-]+/g, "-");
// docs/content/docs/openui-lang/syntax.mdx → docs/openui-lang-syntax
importPrograms("docs/content", "docs", (rel) => {
  const parts = rel.split("/").slice(2);
  return `docs/${slug(parts[0] === "docs" ? parts.slice(1) : parts)}`;
});
// examples/cookbooks/booking-assistant/src/lib/prompt.ts → examples/booking-assistant-prompt
importPrograms("examples", "examples", (rel) => {
  const parts = rel.split("/");
  return `examples/${slug([parts[2], basename(rel)])}`;
});

writeFileSync(join(OUT, "..", "imported.json"), JSON.stringify({ schemas, cases }, null, 1) + "\n");
const bySource = Object.entries(Object.groupBy(cases, (c) => c.source)).map(([s, cs]) => `${cs!.length} ${s}`);
console.log(`Imported ${cases.length} cases (${bySource.join(", ")}) into conformance/imported/`);
console.log(`Captured ${capture.merges.length} merge and ${capture.serializes.length} serialize calls from the reference tests (${serializeCases.length} distinct serialize calls)`);
console.log(`Imported ${fixtureCount} of the spec's fixtures, ${fixtureEdits} of them edits`);
