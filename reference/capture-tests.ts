// Preload for `bun test`: wraps the reference parser so every program the
// reference test suites parse is recorded, with the component library it was
// parsed against; and likewise every mergeStatements and jsonToOpenUI call,
// with its arguments. Run through `bun import-corpus.ts`, not directly.
import { afterAll, mock } from "bun:test";
import { writeFileSync } from "fs";
import { join } from "path";

const PARSER = join(import.meta.dir, "lang-core/src/parser/parser.ts");
const MERGE = join(import.meta.dir, "lang-core/src/parser/merge.ts");
const SERIALIZE = join(import.meta.dir, "lang-core/src/parser/serialize.ts");
// Copy the exports first: mock.module rewrites the live module namespace too.
const real = { ...(await import(PARSER)) };
const realMerge = { ...(await import(MERGE)) };
const realSerialize = { ...(await import(SERIALIZE)) };
const out = process.env.CAPTURE_OUT!;

type Captured = { test: string; schema: unknown; input: string; root?: string };
const captured: Captured[] = [];
type MergeCall = { test: string; existing: string; patch: string; rootId?: string };
const merges: MergeCall[] = [];
type SerializeCall = { test: string; element: unknown; schema: unknown; stateDeclarations?: unknown };
const serializes: SerializeCall[] = [];
/** A value as JSON carries it: what the Rust side gets. */
const asJson = (v: unknown) => JSON.parse(JSON.stringify(v));

/** The test that's running, from the call stack (`<file>.test.ts`). */
function currentTestFile(): string {
  const stack = new Error().stack ?? "";
  const m = stack.match(/([\w.-]+\.test\.ts)/);
  return m ? m[1] : "unknown";
}

/** A ParamMap (what parse() takes) as the JSON Schema compileSchema would read back. */
function paramMapToSchema(cat: Map<string, { params: any[] }> | undefined): unknown {
  const $defs: Record<string, unknown> = {};
  for (const [name, { params }] of cat ?? []) {
    const properties: Record<string, unknown> = {};
    for (const p of params) {
      const schema = p.schema && typeof p.schema === "object" ? { ...p.schema } : {};
      if (p.defaultValue !== undefined) schema.default = p.defaultValue;
      properties[p.name] = schema;
    }
    $defs[name] = { properties, required: params.filter((p) => p.required).map((p) => p.name) };
  }
  return { $defs };
}

function record(schema: unknown, input: string, root?: string) {
  if (typeof input !== "string") return;
  captured.push({ test: currentTestFile(), schema, input, ...(root ? { root } : {}) });
}

/** Records the full text a stream parser saw once it's been pushed or set. */
function recordingStream(stream: any, schema: unknown, root?: string) {
  let text = "";
  let entry: Captured | undefined;
  const sync = () => {
    if (!entry) {
      record(schema, text, root);
      entry = captured[captured.length - 1];
    }
    entry.input = text;
  };
  return {
    ...stream,
    push(chunk: string) {
      text += chunk;
      sync();
      return stream.push(chunk);
    },
    set(full: string) {
      text = full;
      sync();
      return stream.set(full);
    },
    getResult: () => stream.getResult(),
  };
}

mock.module(PARSER, () => ({
  ...real,
  parse(input: string, cat: any, root?: string) {
    record(paramMapToSchema(cat), input, root);
    return real.parse(input, cat, root);
  },
  createStreamParser(cat: any, root?: string) {
    return recordingStream(real.createStreamParser(cat, root), paramMapToSchema(cat), root);
  },
  createParser(schema: unknown, root?: string) {
    const parser = real.createParser(schema, root);
    return {
      parse(input: string) {
        record(schema, input, root);
        return parser.parse(input);
      },
    };
  },
  createStreamingParser(schema: unknown, root?: string) {
    return recordingStream(real.createStreamingParser(schema, root), schema, root);
  },
}));

mock.module(MERGE, () => ({
  ...realMerge,
  mergeStatements(existing: string, patch: string, rootId?: string) {
    merges.push({ test: currentTestFile(), existing, patch, ...(rootId !== undefined ? { rootId } : {}) });
    return realMerge.mergeStatements(existing, patch, rootId);
  },
}));

mock.module(SERIALIZE, () => ({
  ...realSerialize,
  jsonToOpenUI(json: unknown, library: any, options?: { stateDeclarations?: unknown }) {
    serializes.push({
      test: currentTestFile(),
      element: asJson(json),
      schema: asJson(library.toJSONSchema()),
      ...(options?.stateDeclarations !== undefined ? { stateDeclarations: asJson(options.stateDeclarations) } : {}),
    });
    return realSerialize.jsonToOpenUI(json, library, options);
  },
}));

// In a preload, afterAll runs once after every test file. (`exit` never fires under bun test.)
afterAll(() => writeFileSync(out, JSON.stringify({ parses: captured, merges, serializes })));
