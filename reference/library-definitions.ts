// Goldens for library definitions: sample libraries declared with upstream's
// own defineComponent and createLibrary over zod, each converted to the core's
// plain-data form (library-definition.ts) and run through upstream for its
// toJSONSchema() and toSpec(), or the message createLibrary throws.
//
//   bun --no-install library-definitions.ts
//
// writes conformance/golden/library-definitions.json, one entry per sample:
// {name, definition, schema, spec} or {name, definition, error}. The core must
// give the same output for `definition`, byte for byte (tests/library_definition.rs).
// It also writes the "scalars" sample's schema to
// conformance/schemas/library-definition.json, the corpus library for
// conformance/cases/library-definition/; that test checks the two agree.
import { writeFileSync } from "fs";
import { join } from "path";
import { z } from "zod/v4";

process.env.OPENUI_TELEMETRY_DISABLED = "1";
process.env.DO_NOT_TRACK = "1";
const { createLibrary, defineComponent } = await import("./lang-core/src/library");
const { toLibraryDefinition } = await import("./library-definition");
const { action } = await import("./lang-core/src/parser/builtins");
const { markReactive } = await import("./lang-core/src/reactive");
const { tagSchemaId } = await import("./lang-core/src/signature");

const CONFORMANCE = join(import.meta.dir, "..", "conformance");

type Input = Parameters<typeof createLibrary>[0];
// Every component has a renderer, as in a real library: the plain-data form
// has none and the core takes one as present (upstream's root check can read
// it, through a component named __proto__).
const renderer = () => null;
const component = (name: string, description: string, props: Record<string, z.ZodType> | z.ZodObject) =>
  defineComponent({ name, description, props: props instanceof z.ZodObject ? props : z.object(props), component: renderer });

// The main sample: every scalar kind, optional and required, descriptions
// (one empty), groups with and without notes, and a root.
const scalars: Input = {
  components: [
    component("Page", "A page with a title and any children.", {
      title: z.string(),
      children: z.any(),
      subtitle: z.string().optional(),
      dense: z.boolean().optional(),
    }),
    component("Heading", "A heading of a given level.", {
      text: z.string(),
      level: z.number().optional(),
    }),
    component("Text", "A paragraph.", { value: z.string(), muted: z.boolean().optional() }),
    component("Metric", "A labelled number with an optional change and raw data.", {
      label: z.string(),
      value: z.number(),
      delta: z.number().optional(),
      data: z.any().optional(),
    }),
    component("Toggle", "An on/off switch.", { label: z.string(), on: z.boolean() }),
    component("Divider", "", {}),
  ],
  componentGroups: [
    { name: "Layout", components: ["Page", "Divider"], notes: ["Page is the root.", "Divider takes nothing."] },
    { name: "Content", components: ["Heading", "Text", "Metric", "Toggle"] },
  ],
  root: "Page",
};

const one = (props: Record<string, z.ZodType> | z.ZodObject): Input => ({ components: [component("A", "a", props)] });

const samples: [string, () => Input][] = [
  ["scalars", () => scalars],
  // JavaScript lists array-index keys first, for component and prop names alike.
  [
    "key-order",
    () => ({
      components: [
        component("Zeta", "z", { b: z.string(), "10": z.number(), "2": z.boolean(), "02": z.any(), ["__proto__"]: z.string().optional() }),
        component("7", "seven", { x: z.string().optional() }),
        component("Alpha", "a", { "-1": z.number() }),
      ],
      root: "7",
      id: "key-order-library",
    }),
  ],
  // The later of two components of one name wins, in the first one's place.
  [
    "duplicate-components",
    () => ({
      components: [
        component("Card", "first", { a: z.string() }),
        component("Other", "other", {}),
        component("Card", "second", { b: z.number().optional() }),
      ],
      root: "Card",
    }),
  ],
  // A $ref escapes "~" and "/" as a JSON Pointer does, and nothing else.
  [
    "ref-escaping",
    () => ({
      components: [
        component("a/b", "slash", { "x/y": z.string() }),
        component("a~b", "tilde", {}),
        component("a b#%\"", "others", {}),
        component("", "empty name", { "": z.number().optional(), v: z.string() }),
      ],
    }),
  ],
  // Names of the action shapes upstream registers are components like any other.
  [
    "action-shape-names",
    () => ({ components: [component("ToAssistant", "t", { message: z.string() }), component("OpenUrl", "", {})], root: "OpenUrl" }),
  ],
  ["no-components", () => ({ components: [] })],
  ["empty-groups", () => ({ components: [component("A", "a", {})], componentGroups: [], id: "" })],
  // createLibrary reads the root off a plain object, so inherited names pass.
  ["root-inherited", () => ({ components: [component("A", "a", { v: z.string() })], root: "toString" })],
  // An empty root isn't checked.
  ["root-empty", () => ({ components: [component("A", "a", {})], root: "" })],
  // A component named __proto__ becomes the record's prototype: it's left out,
  // and the root check, `!record[root]`, then reads the fields of the last
  // such component: name, props, component and ref are always truthy, its
  // description only when not empty.
  [
    "proto-component",
    () => ({ components: [component("__proto__", "p", { v: z.string() }), component("A", "a", {})], root: "props" }),
  ],
  ...(["name", "component", "ref", "description"] as const).map((root): [string, () => Input] => [
    `proto-component-root-${root}`,
    () => ({ components: [component("__proto__", "p", {}), component("A", "a", {})], root }),
  ]),
  [
    "proto-component-empty-description-root-description",
    () => ({ components: [component("__proto__", "", {}), component("A", "a", {})], root: "description" }),
  ],
  [
    "proto-component-empty-description-root-props",
    () => ({ components: [component("__proto__", "", {}), component("A", "a", {})], root: "props" }),
  ],
  [
    "two-proto-components-last-empty-description",
    () => ({ components: [component("__proto__", "p", {}), component("__proto__", "", {}), component("A", "a", {})], root: "description" }),
  ],
  [
    "two-proto-components-first-empty-description",
    () => ({ components: [component("__proto__", "", {}), component("__proto__", "p", {}), component("A", "a", {})], root: "description" }),
  ],
  // Without a __proto__ component those names are not components.
  ["root-description-no-proto-component", () => ({ components: [component("A", "a", {})], root: "description" })],
  // z.strictObject's catch-all is a bare z.never(): zod writes it as z.object.
  ["strict-object", () => ({ components: [component("A", "a", z.strictObject({ v: z.string(), w: z.number().optional() }))] })],
  // The data kinds (#132), one sample per kind so a failure names it. Each is
  // written as upstream's zod writes it: zod 4.6.7 folds an anyOf of bare
  // types into a type list, and the signature unwraps optional and nullable.
  [
    "enum",
    () => one({
      tone: z.enum(["info", "warn", "error"]),
      level: z.enum(["low", "high"]).optional(),
      single: z.enum(["only"]),
      keys: z.object({ a: z.string(), b: z.number() }).keyof(),
    }),
  ],
  // Object.keys order: array-index words first, ascending; a repeated word once.
  [
    "enum-key-order",
    () => one({
      order: z.enum(["b", "10", "2", "__proto__", "01", "-0", "4294967295", "4294967294"]),
      repeated: z.enum(["a", "b", "a"]),
    }),
  ],
  // Upstream writes the words unescaped in the signature.
  ["enum-quotes", () => one({ q: z.enum(['say "hi"', "back\\slash", ""]) })],
  // An empty enum accepts nothing ({"not": {}}) and prints no type: the bare
  // name, [name] when it is an array's items, any as a record's values, left
  // out of a union.
  [
    "enum-empty",
    () => one({
      none: z.enum([] as any),
      maybe: z.enum([] as any).optional(),
      list: z.array(z.enum([] as any)),
      nullable: z.enum([] as any).nullable(),
      nullableList: z.array(z.enum([] as any)).nullable().optional(),
      field: z.object({ a: z.enum([] as any), b: z.enum([] as any).optional(), c: z.array(z.enum([] as any)) }),
      map: z.record(z.string(), z.enum([] as any)),
      onlyEmpty: z.union([z.enum([] as any), z.enum([] as any)]),
      withString: z.union([z.enum([] as any), z.string()]),
    }),
  ],
  [
    "array",
    () => one({
      tags: z.array(z.string()),
      scores: z.array(z.number()).optional(),
      grid: z.array(z.array(z.boolean())),
      rows: z.array(z.object({ id: z.string(), note: z.string().optional() })),
      tones: z.array(z.enum(["a", "b"])),
      maps: z.array(z.record(z.string(), z.any())),
      anything: z.array(z.any()),
    }),
  ],
  // A union's items are written in parentheses in the signature.
  [
    "array-of-union",
    () => one({
      mixed: z.array(z.union([z.string(), z.number()])),
      shapes: z.array(z.union([z.string(), z.object({ a: z.number() })])),
      one: z.array(z.union([z.string()])),
      none: z.array(z.union([] as any)),
      nullableUnion: z.array(z.union([z.string(), z.number()]).nullable()),
      nullableItems: z.array(z.string().nullable()),
      nested: z.array(z.array(z.union([z.boolean(), z.string()]))),
    }),
  ],
  [
    "union-of-scalars",
    () => one({
      text: z.union([z.string(), z.number()]),
      order: z.union([z.boolean(), z.string(), z.number()]),
      repeated: z.union([z.string(), z.string()]),
      nested: z.union([z.union([z.string(), z.boolean()]), z.number()]),
      nullableMember: z.union([z.string().nullable(), z.number()]),
      single: z.union([z.number()]),
      withAny: z.union([z.any(), z.string()]),
      optional: z.union([z.string(), z.boolean()]).optional(),
    }),
  ],
  [
    "union-with-object",
    () => one({
      value: z.union([z.string(), z.object({ a: z.number(), b: z.string().optional() })]),
      single: z.union([z.object({ a: z.string() })]),
      nested: z.union([z.union([z.string(), z.object({})]), z.number()]),
      arrays: z.union([z.array(z.string()), z.string()]),
      words: z.union([z.enum(["x", "y"]), z.string()]),
      maps: z.union([z.record(z.string(), z.number()), z.boolean()]),
    }),
  ],
  [
    "union-empty",
    () => one({ none: z.union([] as any), nullable: z.union([] as any).nullable(), list: z.array(z.union([] as any)).optional() }),
  ],
  [
    "object-with-optional-fields",
    () => one({
      point: z.object({ x: z.number(), y: z.number(), label: z.string().optional() }),
      allOptional: z.object({ a: z.string().optional(), b: z.boolean().optional() }).optional(),
      nested: z.object({ inner: z.object({ v: z.string(), w: z.array(z.number()).optional() }) }),
      nullableFields: z.object({ a: z.string().nullable(), b: z.string().nullable().optional() }),
      strict: z.strictObject({ s: z.string() }),
    }),
  ],
  ["object-empty", () => one({ empty: z.object({}), maybe: z.object({}).optional(), list: z.array(z.object({})) })],
  // JavaScript key order applies to an inline object's fields too.
  [
    "object-key-order",
    () => one({
      fields: z.object({ b: z.string(), "10": z.number(), "2": z.boolean(), ["__proto__"]: z.string().optional(), 'a"b': z.any(), "": z.number() }),
    }),
  ],
  [
    "record",
    () => one({
      counts: z.record(z.string(), z.number()),
      anything: z.record(z.string(), z.any()),
      shapes: z.record(z.string(), z.object({ a: z.string() })),
      nullable: z.record(z.string(), z.string().nullable()),
      mixed: z.record(z.string(), z.union([z.string(), z.number()])),
      lists: z.record(z.string(), z.array(z.string())).optional(),
      nested: z.record(z.string(), z.record(z.string(), z.boolean())),
    }),
  ],
  // A nullable prop stays in `required` but prints with `?`, as optional does.
  [
    "nullable",
    () => one({
      text: z.string().nullable(),
      anything: z.any().nullable(),
      union: z.union([z.string(), z.number()]).nullable(),
      list: z.array(z.string()).nullable(),
      shape: z.object({ a: z.string() }).nullable(),
      word: z.enum(["a", "b"]).nullable(),
      map: z.record(z.string(), z.string()).nullable(),
      twice: z.string().nullable().nullable(),
      emptyObject: z.object({}).nullable(),
    }),
  ],
  // Optional and nullable together: out of `required`, null in the type.
  [
    "optional-and-nullable",
    () => one({
      both: z.string().nullable().optional(),
      nullish: z.number().nullish(),
      list: z.array(z.string()).nullable().optional(),
      shape: z.object({ a: z.string().nullable().optional() }).nullable().optional(),
      required: z.boolean(),
    }),
  ],
  // Kinds inside kinds, with a root and groups like a real library.
  [
    "nested-kinds",
    () => ({
      components: [
        component("Table", "A table of rows.", {
          columns: z.array(z.object({ key: z.string(), label: z.string(), align: z.enum(["left", "right"]).optional() })),
          rows: z.array(z.record(z.string(), z.union([z.string(), z.number(), z.boolean()]).nullable())),
          footer: z.union([z.string(), z.object({ total: z.number(), note: z.string().nullable() })]).optional(),
        }),
        component("Chart", "A chart.", {
          kind: z.enum(["bar", "line"]),
          series: z.array(z.object({ name: z.string(), points: z.array(z.union([z.number(), z.array(z.number())])) })),
        }),
      ],
      componentGroups: [{ name: "Data", components: ["Table", "Chart"] }],
      root: "Table",
    }),
  ],
  ["root-missing", () => ({ components: [component("A", "a", {}), component("B", "b", {})], root: "C" })],
  ["root-missing-no-components", () => ({ components: [], root: "Page" })],
  ["root-missing-proto-component", () => ({ components: [component("__proto__", "p", {})], root: "Nope" })],
];

// Declarations the converter must refuse: each one's upstream output differs
// from what the core gives for the plain data the converter could make of it.
// The run stops if one converts, or is refused for another reason.
const shared = () => z.object({ a: z.string() });
// z.object over a shape that also has a symbol key.
const withSymbol = (shape: Record<string, z.ZodType>) => {
  const withKey: Record<string | symbol, z.ZodType> = { ...shape };
  withKey[Symbol("s")] = z.number();
  return z.object(withKey as Record<string, z.ZodType>);
};
// A prop type `levels` arrays around a string: `levels + 1` levels deep.
const deepArray = (levels: number): z.ZodType => {
  let type: z.ZodType = z.string();
  for (let i = 0; i < levels; i++) type = z.array(type);
  return type;
};
const tagged = () => {
  const schema = z.string();
  tagSchemaId(schema, "Color");
  return schema;
};
const refusals: [string, () => unknown, string, string?][] = [
  ["int", () => one({ n: z.int() }), '"check"'],
  ["int32", () => one({ n: z.int32() }), '"check"'],
  ["float64", () => one({ n: z.float64() }), '"check"'],
  ["optional-int", () => one({ n: z.int().optional() }), '"check"'],
  ["email", () => one({ s: z.email() }), '"format"'],
  ["uuid", () => one({ s: z.uuid() }), '"format"'],
  ["url", () => one({ s: z.url() }), '"format"'],
  ["ipv4", () => one({ s: z.ipv4() }), '"format"'],
  ["cuid", () => one({ s: z.cuid() }), '"format"'],
  ["base64", () => one({ s: z.base64() }), '"format"'],
  ["iso-date", () => one({ s: z.iso.date() }), '"format"'],
  ["iso-datetime", () => one({ s: z.iso.datetime() }), '"format"'],
  ["min", () => one({ s: z.string().min(1) }), "checks"],
  ["trim", () => one({ s: z.string().trim() }), "checks"],
  ["coerce", () => one({ n: z.coerce.number() }), '"coerce"'],
  ["describe", () => one({ s: z.string().describe("d") }), "metadata"],
  ["default", () => one({ s: z.string().default("d") }), 'kind "default"'],
  ["unknown-kind", () => one({ s: z.unknown() }), 'kind "unknown"'],
  ["loose-object", () => one(z.looseObject({ v: z.string() })), "catch-all"],
  ["catchall", () => one(z.object({ v: z.string() }).catchall(z.number())), "catch-all"],
  ["props-checked", () => one(z.object({ v: z.string() }).refine(() => true) as any), "checks"],
  // The data kinds (#132): what zod can say that the plain data can't, or
  // says differently.
  ["literal", () => one({ s: z.literal("x") }), 'kind "literal"'],
  ["tuple", () => one({ t: z.tuple([z.string(), z.number()]) }), 'kind "tuple"'],
  ["intersection", () => one({ t: z.intersection(z.object({ a: z.string() }), z.object({ b: z.string() })) }), 'kind "intersection"'],
  ["map", () => one({ m: z.map(z.string(), z.number()) }), 'kind "map"'],
  ["lazy", () => one({ l: z.lazy(() => z.string()) }), 'kind "lazy"'],
  ["readonly", () => one({ r: z.array(z.string()).readonly() }), 'kind "readonly"'],
  ["null", () => one({ n: z.union([z.string(), z.null()]) }), 'kind "null"'],
  [
    "discriminated-union",
    () => one({ d: z.discriminatedUnion("k", [z.object({ k: z.literal("a") }), z.object({ k: z.literal("b") })]) }),
    '"discriminator"',
  ],
  ["xor", () => one({ x: z.xor([z.string(), z.number()]) }), '"inclusive"'],
  ["enum-object-map", () => one({ e: z.enum({ A: "a", B: "b" }) }), "keys aren't its string values"],
  ["enum-numeric", () => one({ e: z.nativeEnum({ A: 1, B: 2 } as any) }), "keys aren't its string values"],
  ["record-enum-key", () => one({ r: z.record(z.enum(["a", "b"]), z.string()) }), "record key"],
  ["record-key-check", () => one({ r: z.record(z.string().min(1), z.string()) }), "checks"],
  ["partial-record", () => one({ r: z.partialRecord(z.enum(["a"]), z.string()) }), '"partial"'],
  ["loose-record", () => one({ r: z.looseRecord(z.string(), z.string()) }), '"mode"'],
  ["array-min", () => one({ a: z.array(z.string()).min(1) }), "checks"],
  ["array-nonempty", () => one({ a: z.array(z.string()).nonempty() }), "checks"],
  ["optional-in-array", () => one({ a: z.array(z.string().optional()) }), "optional inside"],
  // Inside a union zod drops the prop from `required` though the signature
  // shows no `?`.
  ["optional-in-union", () => one({ u: z.union([z.string().optional(), z.number()]) }), "optional inside"],
  ["optional-in-record", () => one({ r: z.record(z.string(), z.string().optional()) }), "optional inside"],
  ["nullable-optional", () => one({ n: z.string().optional().nullable() }), "optional inside"],
  ["optional-optional", () => one({ o: z.string().optional().optional() }), "optional inside"],
  ["default-in-array", () => one({ a: z.array(z.string().default("x")) }), 'kind "default"'],
  ["loose-inner-object", () => one({ o: z.looseObject({ a: z.string() }) }), "catch-all"],
  ["inner-catchall", () => one({ o: z.object({ a: z.string() }).catchall(z.string()) }), "catch-all"],
  ["describe-nested", () => one({ a: z.array(z.string().describe("d")) }), "metadata"],
  ["meta-object", () => one({ o: z.object({}).meta({ title: "t" }) }), "metadata"],
  [
    "reactive-nested",
    () => {
      const inner = z.string();
      markReactive(inner);
      return one({ a: z.array(inner) });
    },
    "reactive",
  ],
  ["component-reference", () => one({ items: z.array(component("Item", "i", {}).ref as any) }), "named schema"],
  ["action-slot", () => one({ action: action().optional() }), "action slot"],
  ["named-type", () => one({ c: tagged() }), "named schema"],
  // Object.entries skips symbol keys; upstream throws on them, at the props
  // object and inside it (checked below).
  ["symbol-key-props", () => one(withSymbol({ a: z.string() })), "symbol keys", "Symbol keys cannot be represented in JSON Schema"],
  [
    "symbol-key-nested",
    () => one({ o: withSymbol({ a: z.string() }) }),
    "symbol keys",
    "Symbol keys cannot be represented in JSON Schema",
  ],
  // A getter that returns its own object nests without end; refused at the
  // core's depth limit by name, not by the stack.
  [
    "recursive-getter",
    () => {
      const node = z.object({
        name: z.string(),
        get child() {
          return node;
        },
      });
      return one({ tree: node });
    },
    "nest deeper than 256",
  ],
  ["deeper-than-the-limit", () => one({ deep: deepArray(256) }), "nest deeper than 256"],
  [
    "shared-props-object",
    () => {
      const props = shared();
      return { components: [component("A", "first", props), component("B", "second", props)] };
    },
    "shared",
  ],
  [
    "no-renderer",
    () => ({ components: [defineComponent({ name: "A", description: "a", props: z.object({}), component: null })] }),
    "renderer",
  ],
  [
    "component-extra-field",
    () => ({ components: [{ ...component("A", "a", {}), extra: 1 }] }),
    'field "extra"',
  ],
  ["library-extra-field", () => ({ ...one({}), functions: [] }), 'field "functions"'],
  [
    "group-key-order",
    () => ({ ...one({}), componentGroups: [{ components: ["A"], name: "G" }] }),
    "in that order",
  ],
  [
    "group-notes-first",
    () => ({ ...one({}), componentGroups: [{ notes: ["n"], name: "G", components: ["A"] }] }),
    "in that order",
  ],
  [
    "group-extra-field",
    () => ({ ...one({}), componentGroups: [{ name: "G", components: ["A"], hint: "x" }] }),
    "in that order",
  ],
];
for (const [name, make, reason, upstreamThrows] of refusals) {
  let message: string | undefined;
  try {
    toLibraryDefinition(make() as any);
  } catch (e) {
    message = (e as Error).message;
  }
  if (message === undefined) throw new Error(`refusal ${name}: the converter accepted it`);
  if (!message.includes(reason)) throw new Error(`refusal ${name}: refused for another reason: ${message}`);
  // Where upstream itself throws, check it still does, with that message.
  if (upstreamThrows !== undefined) {
    let upstream: string | undefined;
    try {
      createLibrary(make() as Input).toJSONSchema();
    } catch (e) {
      upstream = (e as Error).message;
    }
    if (!upstream?.includes(upstreamThrows)) throw new Error(`refusal ${name}: upstream no longer throws: ${upstream}`);
  }
}

// The converter's depth limit is the core's: 256 levels convert.
toLibraryDefinition(one({ deep: deepArray(255) }) as any);

const out = samples.map(([name, make]) => {
  const input = make();
  const definition = toLibraryDefinition(input as any);
  try {
    const library = createLibrary(input);
    return { name, definition, schema: library.toJSONSchema(), spec: library.toSpec() };
  } catch (e) {
    return { name, definition, error: (e as Error).message };
  }
});
writeFileSync(join(CONFORMANCE, "golden", "library-definitions.json"), JSON.stringify(out, null, 1) + "\n");
writeFileSync(
  join(CONFORMANCE, "schemas", "library-definition.json"),
  JSON.stringify(out[0].schema, null, 2) + "\n",
);
console.log(`${out.length} library-definition observations, ${refusals.length} converter refusals`);
