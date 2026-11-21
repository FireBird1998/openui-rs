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
  ["root-missing", () => ({ components: [component("A", "a", {}), component("B", "b", {})], root: "C" })],
  ["root-missing-no-components", () => ({ components: [], root: "Page" })],
  ["root-missing-proto-component", () => ({ components: [component("__proto__", "p", {})], root: "Nope" })],
];

// Declarations the converter must refuse: each one's upstream output differs
// from what the core gives for the plain data the converter could make of it.
// The run stops if one converts, or is refused for another reason.
const shared = () => z.object({ a: z.string() });
const one = (props: Record<string, z.ZodType> | z.ZodObject): Input => ({ components: [component("A", "a", props)] });
const refusals: [string, () => unknown, string][] = [
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
for (const [name, make, reason] of refusals) {
  let message: string | undefined;
  try {
    toLibraryDefinition(make() as any);
  } catch (e) {
    message = (e as Error).message;
  }
  if (message === undefined) throw new Error(`refusal ${name}: the converter accepted it`);
  if (!message.includes(reason)) throw new Error(`refusal ${name}: refused for another reason: ${message}`);
}

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
