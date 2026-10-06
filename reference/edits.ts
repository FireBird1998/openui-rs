// Goldens for the rest of the language layer: mergeStatements, jsonToOpenUI
// and enrichErrors. Written to conformance/golden/edits/{merge,serialize,enrich}.json
// as {name: {"text": ...} | {"error": message}} (enrich: {name: JSON}).
//
//   bun reference/edits.ts golden
//   bun reference/edits.ts show merge|serialize|enrich <name>
//
// Inputs:
//   merge      imported/merge.json (the reference tests' calls), merge.json
//              (hand-written), merge-generated.json (reference/merge-cases.ts),
//              merge-fuzz.json (scripts/fuzz.sh --edits)
//   serialize  imported/serialize.json, serialize.json, serialize-fuzz.json, and corpus/<case>: the
//              root of every corpus case's one-shot result, as JSON, with its
//              state declarations
//   enrich     <case>: every corpus case's one-shot errors, with the library's
//              component names; <case>#no-names: the same with none
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "fs";
import { join } from "path";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { mergeStatements } = await import("./lang-core/src/parser/merge");
const { jsonToOpenUI } = await import("./lang-core/src/parser/serialize");
const { enrichErrors } = await import("./lang-core/src/parser/enrich-errors");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");
const GOLDEN = join(CONFORMANCE, "golden", "edits");
const read = (rel: string) => readFileSync(join(CONFORMANCE, rel), "utf8");
const manifest = (rel: string) => (existsSync(join(CONFORMANCE, rel)) ? JSON.parse(read(rel)) : { cases: [] });

// The parse corpus, and every schema by name (a path, or inline).
const corpusFiles = ["manifest.json", "imported.json", "fuzz.json", "synthetic.json"];
const schemaSources: Record<string, unknown> = {};
const corpus: { name: string; schema: string; file: string }[] = [];
for (const f of corpusFiles) {
  const m = manifest(f);
  Object.assign(schemaSources, m.schemas ?? {});
  corpus.push(...m.cases);
}
for (const f of ["imported/serialize.json", "serialize.json"]) Object.assign(schemaSources, manifest(f).schemas ?? {});
const schemaOf = (name: string) => {
  const source = schemaSources[name];
  return typeof source === "string" ? JSON.parse(read(source)) : source;
};
const caseFile = (name: string) => read(corpus.find((c) => c.name === name)!.file);

type Outcome = { text: string } | { error: string };
function attempt(f: () => string): Outcome {
  try {
    return { text: f() };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

// ── merge ────────────────────────────────────────────────────────────────────
type MergeCase = { name: string; existing?: string; existingCase?: string; patch?: string; patchCase?: string; rootId?: string };
const mergeCases: MergeCase[] = ["imported/merge.json", "merge.json", "merge-generated.json", "merge-fuzz.json"].flatMap(
  (f) => manifest(f).cases,
);
const runMerge = (c: MergeCase) =>
  attempt(() =>
    mergeStatements(
      c.existing ?? caseFile(c.existingCase!),
      c.patch ?? caseFile(c.patchCase!),
      c.rootId,
    ),
  );

// ── serialize ────────────────────────────────────────────────────────────────
type SerializeCase = { name: string; schema: string; element: unknown; stateDeclarations?: unknown };
const serializeCases: SerializeCase[] = ["imported/serialize.json", "serialize.json", "serialize-fuzz.json"].flatMap(
  (f) => manifest(f).cases,
);
for (const c of corpus) {
  const result = JSON.parse(read(`golden/${c.name}.oneshot.json`));
  if (result.root) {
    serializeCases.push({ name: `corpus/${c.name}`, schema: c.schema, element: result.root, stateDeclarations: result.stateDeclarations });
  }
}
const runSerialize = (c: SerializeCase) =>
  attempt(() => {
    const schema = schemaOf(c.schema);
    const library = { toJSONSchema: () => schema } as any;
    return jsonToOpenUI(c.element as any, library, "stateDeclarations" in c ? { stateDeclarations: c.stateDeclarations as any } : undefined);
  });

// ── enrich ───────────────────────────────────────────────────────────────────
type EnrichCase = { name: string; schema: string; errors: unknown[]; names: string[] };
const enrichCases: EnrichCase[] = [];
for (const c of corpus) {
  const errors = JSON.parse(read(`golden/${c.name}.oneshot.json`)).meta.errors;
  if (!errors.length) continue;
  const names = Object.keys(schemaOf(c.schema).$defs ?? {});
  enrichCases.push({ name: c.name, schema: c.schema, errors, names });
  if (errors.some((e: { code: string }) => e.code === "unknown-component")) {
    enrichCases.push({ name: `${c.name}#no-names`, schema: c.schema, errors, names: [] });
  }
}
const runEnrich = (c: EnrichCase) => JSON.stringify(enrichErrors(c.errors as any, schemaOf(c.schema), c.names));

const [command, kind, name] = process.argv.slice(2);
if (command === "golden") {
  mkdirSync(GOLDEN, { recursive: true });
  const write = (file: string, entries: [string, unknown][]) =>
    writeFileSync(join(GOLDEN, file), JSON.stringify(Object.fromEntries(entries), null, 0) + "\n");
  write("merge.json", mergeCases.map((c) => [c.name, runMerge(c)]));
  write("serialize.json", serializeCases.map((c) => [c.name, runSerialize(c)]));
  write("enrich.json", enrichCases.map((c) => [c.name, runEnrich(c)]));
  console.log(
    `Wrote edit goldens: ${mergeCases.length} merge, ${serializeCases.length} serialize, ${enrichCases.length} enrich cases`,
  );
} else if (command === "show" && name) {
  const show = (o: Outcome) => ("text" in o ? o.text : `error: ${o.error}`);
  if (kind === "merge") console.log(show(runMerge(mergeCases.find((c) => c.name === name)!)));
  else if (kind === "serialize") console.log(show(runSerialize(serializeCases.find((c) => c.name === name)!)));
  else if (kind === "enrich") console.log(runEnrich(enrichCases.find((c) => c.name === name)!));
} else {
  console.error("usage: bun reference/edits.ts golden | show merge|serialize|enrich <name>");
  process.exit(1);
}
