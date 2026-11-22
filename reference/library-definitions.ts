// Goldens for library definitions: sample libraries declared with upstream's
// own defineComponent, defineFunction, defineAction and createLibrary over
// zod, each converted to the core's
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
const { createLibrary, defineComponent, defineAction, defineFunction } = await import("./lang-core/src/library");
const { toLibraryDefinition } = await import("./library-definition");
const { action, steps } = await import("./lang-core/src/parser/builtins");
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
// reactive(), as react-lang's marks a schema a binding, and tagSchemaId.
const bound = <T extends object>(schema: T): T => {
  markReactive(schema);
  return schema;
};
const named = <T extends object>(schema: T, name: string): T => {
  tagSchemaId(schema, name);
  return schema;
};
// defineFunction and defineAction over a params shape; `fn` is the
// implementation, which never enters the core.
const fn = (name: string, description: string, params: Record<string, z.ZodType>, returns?: z.ZodType) =>
  defineFunction({ name, description, params: z.object(params), ...(returns !== undefined ? { returns } : {}), fn: () => null });
const act = (name: string, description: string, params: Record<string, z.ZodType>) =>
  defineAction({ name, description, params: z.object(params) });
// A library of one component, with these functions and actions.
const calls = (functions: ReturnType<typeof fn>[], actions: ReturnType<typeof act>[] = []): Input => ({
  components: [component("A", "a", {})],
  ...(functions.length ? { functions } : {}),
  ...(actions.length ? { actions } : {}),
});
// A collision: createLibrary refuses the second of two names.
const collision = (kind: "function" | "action", name: string, extra: Partial<Input> = {}): Input => ({
  components: [component("Item", "i", {})],
  ...extra,
  ...(kind === "function"
    ? { functions: [...(extra.functions ?? []), fn(name, "", {})] }
    : { actions: [...(extra.actions ?? []), act(name, "", {})] }),
});

function stringDescription(withDescription: boolean): Input {
  const describe = <T extends z.ZodType>(s: T) => withDescription ? s.describe("field text") : s;
  return one({
    name: describe(z.string()), category: describe(z.string().optional()),
    binding: bound(describe(z.string().optional())), tagged: named(describe(z.string()), "Token"),
    nested: z.array(describe(z.string())),
  });
}

const samples: [string, () => Input][] = [
  ["scalars", () => scalars],
  ["default", () => one({
    name: z.string().default(""), nullable: z.any().default(null),
    optional: bound(z.boolean().default(false).optional()),
    required: z.string().optional().default("x"),
    rows: z.array(z.object({ key: z.number().default(0) })).default([{ key: 7 }]),
    object: z.object({ text: z.string() }).default({ text: "value" }),
  })],
  ["array-min", () => one({
    items: z.array(z.string()).min(2).min(1), zero: z.array(z.number()).min(0),
    nonempty: z.array(z.boolean()).nonempty(),
  })],
  ["string-description", () => stringDescription(true)],
  ["string-description-control", () => stringDescription(false)],
  ["refined-props", () => one(z.object({ v: z.string() }).refine(() => false, { path: ["v"] }))],
  ["unknown", () => one({ required: z.unknown(), bound: bound(z.unknown().optional()), list: z.array(z.unknown()) })],
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
  // The kinds that link a library together (#133). A component reference is
  // a $ref to the component's $defs entry and its name in the signature,
  // inside any kind.
  [
    "component-reference",
    () => {
      const item = component("Item", "An item.", { text: z.string() });
      const other = component("Other", "", {});
      return {
        components: [
          component("List", "A list.", {
            items: z.array(item.ref),
            either: z.union([item.ref, other.ref]),
            maybe: item.ref.nullable(),
            optional: item.ref.optional(),
            field: z.object({ item: item.ref, more: other.ref.optional() }),
            byKey: z.record(z.string(), item.ref),
            mixed: z.array(z.union([item.ref, other.ref])),
            withText: z.union([item.ref, z.string()]),
          }),
          item,
          other,
        ],
        root: "List",
      };
    },
  ],
  // $defs come in the order zod first meets each schema walking the
  // components in order, depth first: a component referred to before its own
  // turn comes at the reference, with what it refers to after it.
  [
    "reference-order-forwards",
    () => {
      const d = component("D", "d", {});
      const c = component("C", "c", { d: d.ref });
      const b = component("B", "b", {});
      return { components: [component("A", "a", { c: c.ref, b: b.ref }), b, c, d] };
    },
  ],
  [
    "reference-order-backwards",
    () => {
      const b = component("B", "b", { v: z.string() });
      return { components: [b, component("A", "a", { b: b.ref, again: z.array(b.ref) })] };
    },
  ],
  [
    "reference-cycle",
    () => {
      const a: any = component("A", "a", {
        get b() {
          return b.ref;
        },
        n: z.number().optional(),
      });
      const b: any = component("B", "b", { a: a.ref.nullable(), list: z.array(a.ref) });
      return { components: [component("Root", "r", { b: b.ref }), a, b], root: "Root" };
    },
  ],
  [
    "reference-self",
    () => {
      const node: any = component("Node", "A tree node.", {
        label: z.string(),
        get children() {
          return z.array(node.ref).optional();
        },
      });
      return { components: [node], root: "Node" };
    },
  ],
  // Refers to a component through a $ref escaped as a JSON Pointer, and
  // into a component named __proto__ (registered, so it gets a $defs entry,
  // but not in the record, so no description and no property).
  [
    "reference-escaping-and-proto",
    () => {
      const slash = component("a/b", "slash", {});
      const tilde = component("a~b", "tilde", {});
      const proto = component("__proto__", "p", { v: z.string() });
      return { components: [component("A", "a", { s: slash.ref, t: z.array(tilde.ref), p: proto.ref }), slash, tilde, proto] };
    },
  ],
  // Of two components of one name the later wins, and a reference means it.
  [
    "reference-to-the-later-duplicate",
    () => {
      const first = component("K", "first", { v: z.string() });
      const later = component("K", "later", { w: z.number() });
      return { components: [first, component("A", "a", { k: later.ref }), later] };
    },
  ],
  // A component outside the library is no reference: upstream prints its
  // name and writes its props inline, a named type.
  [
    "component-outside-library",
    () => one({ out: component("Out", "o", { v: z.string() }).ref, list: z.array(component("Out2", "", {}).ref) }),
  ],
  // The action slot: $defs.ActionExpression, printed ActionExpression (in
  // parentheses as an array's items: its schema is a union).
  [
    "action-slot",
    () => ({
      components: [
        component("Button", "b", { label: z.string(), action: action().optional() }),
        component("Menu", "m", { actions: z.array(action()), maybe: action().nullable() }),
      ],
    }),
  ],
  // A slot restricted to named actions: a union of their refs, printed @Name.
  [
    "action-restricted",
    () => one({
      both: z.union([steps.OpenUrl.ref, steps.ToAssistant.ref]).optional(),
      one: steps.ToAssistant.ref,
      single: z.union([steps.OpenUrl.ref]),
      maybe: steps.OpenUrl.ref.nullable(),
      list: z.array(z.union([steps.ToAssistant.ref, steps.OpenUrl.ref])),
    }),
  ],
  // Action $defs come where they are first met, among the components'.
  [
    "action-definition-order",
    () => {
      const later = component("Later", "l", { go: steps.OpenUrl.ref });
      return {
        components: [component("First", "f", { tell: steps.ToAssistant.ref, later: later.ref, act: action() }), later],
      };
    },
  ],
  // A component of an action's name wins its $defs entry; the action is then
  // written inline wherever it is used, and prints as before.
  [
    "action-shadowed-by-component",
    () => ({
      components: [
        component("A", "a", { act: action(), maybe: action().optional(), go: steps.OpenUrl.ref, tell: steps.ToAssistant.ref }),
        component("ActionExpression", "x", { v: z.string() }),
        component("OpenUrl", "", {}),
      ],
    }),
  ],
  // A binding on a prop's outermost type (the .optional() wrapper included)
  // prints $binding<...>; the schema is the unmarked type's.
  [
    "binding",
    () => {
      const item = component("Item", "", {});
      return {
        components: [
          component("Input", "i", {
            value: bound(z.string()),
            maybe: bound(z.string().optional()),
            nullable: bound(z.string().nullable()),
            list: bound(z.array(z.string())),
            words: bound(z.enum(["a", "b"])),
            none: bound(z.enum([] as any)),
            noneList: bound(z.array(z.enum([] as any)).optional()),
            map: bound(z.record(z.string(), z.number())),
            anything: bound(z.any()),
            either: bound(z.union([z.string(), z.number()]).nullable().optional()),
            shape: bound(z.object({ x: z.string() })),
            item: bound(item.ref.optional()),
            act: bound(action().optional()),
            go: bound(steps.OpenUrl.ref.nullable()),
            plain: z.string(),
          }),
          item,
        ],
      };
    },
  ],
  // A named type prints its name and writes its structure. The name sits on
  // the schema it tags: lost under .nullable() (upstream unwraps it first),
  // empty means none, and an array of a named union keeps the parentheses.
  [
    "named-type",
    () => one({
      color: named(z.string(), "Color"),
      optional: named(z.string(), "Color").optional(),
      nullable: named(z.string(), "Color").nullable(),
      tagNullable: named(z.string().nullable(), "Color"),
      shape: named(z.object({ r: z.number() }), "Rgb"),
      list: named(z.array(z.string()), "List"),
      nothing: named(z.enum([] as any), "Nothing"),
      empty: named(z.string(), ""),
      emptyList: named(z.array(z.enum([] as any)), ""),
      noUnion: named(z.union([] as any), "U"),
      boundLength: bound(named(z.number(), "Length")),
    }),
  ],
  [
    "named-type-in-containers",
    () => one({
      colors: z.array(named(z.string(), "Color")),
      either: z.union([named(z.string(), "Color"), z.number()]),
      lengths: z.array(named(z.union([z.string(), z.number()]), "Length")),
      byKey: z.record(z.string(), named(z.number(), "Length")),
      nullableItems: z.array(named(z.string().nullable(), "Color")),
      field: z.object({ c: named(z.string(), "Color").optional(), n: named(z.string().nullable(), "Color") }),
    }),
  ],
  // One name over two structures, and a name that is also a component's.
  [
    "named-type-collisions",
    () => ({
      components: [
        component("A", "a", { text: named(z.string(), "Color"), rgb: named(z.object({ r: z.number() }), "Color"), b: named(z.boolean(), "B") }),
        component("B", "b", {}),
      ],
    }),
  ],
  // Functions and actions (#134). A function's signature is its params' as
  // a component's, with " → T" when it declares a return type; its JSON
  // Schema entry is {description, params, returns?}, after $defs.
  [
    "function-with-params-and-return",
    () => calls([fn("Percent", "Percent of a total.", { part: z.number(), total: z.number() }, z.number())]),
  ],
  [
    "function-returning-array-of-objects",
    () => calls([
      fn("Rows", "Rows matching a query.", { query: z.string(), limit: z.number().optional() }, z.array(z.object({ id: z.string(), score: z.number().nullable(), tags: z.array(z.string()).optional() }))),
    ]),
  ],
  ["function-without-return-type", () => calls([fn("Log", "Logs a value.", { value: z.any(), label: z.string().optional() })])],
  // The signature unwraps a nullable return type; the schema keeps the null.
  [
    "function-with-nullable-return",
    () => calls([
      fn("Find", "Finds a name.", { id: z.string() }, z.string().nullable()),
      fn("Shape", "", {}, z.object({ a: z.string() }).nullable()),
      fn("Either", "", {}, z.union([z.string(), z.number()]).nullable()),
    ]),
  ],
  // Params as an object's fields: optional, nullable, both, named, with no
  // annotation, in JavaScript key order; none at all.
  [
    "function-params",
    () => calls([
      fn("Zero", "No params.", {}, z.boolean()),
      fn("Kinds", "Every kind of param.", {
        text: z.string(),
        maybe: z.number().optional(),
        nullable: z.string().nullable(),
        both: z.boolean().nullish(),
        color: named(z.string(), "Color"),
        words: z.enum(["a", "b"]),
        mixed: z.array(z.union([z.string(), z.number()])),
        map: z.record(z.string(), z.any()),
        shape: z.object({ x: z.number(), y: z.number().optional() }),
        none: z.array(z.enum([] as any)),
        nothing: z.enum([] as any).optional(),
      }),
      fn("Order", "", { b: z.string(), "10": z.number(), "2": z.boolean() }, z.string()),
    ]),
  ],
  // Every kind as a return type; an empty enum names none, so no arrow.
  [
    "function-return-kinds",
    () => calls([
      fn("Union", "", {}, z.union([z.string(), z.number()])),
      fn("Unions", "", {}, z.array(z.union([z.string(), z.number()]))),
      fn("Map", "", {}, z.record(z.string(), z.number())),
      fn("Any", "", {}, z.any()),
      fn("Words", "", {}, z.enum(["x", "y"])),
      fn("Never", "", {}, z.enum([] as any)),
      fn("Nevers", "", {}, z.array(z.enum([] as any))),
      fn("Named", "", {}, named(z.number(), "Length")),
      fn("Grid", "", {}, z.array(z.array(z.boolean()))),
    ]),
  ],
  // Functions are keyed by name: array-index names first; the empty name
  // and ActionExpression (no function is registered) are names like any other.
  [
    "function-names",
    () => calls(["b", "10", "2", "", "ActionExpression"].map((name) => fn(name, name, { v: z.string() }))),
  ],
  // A call's params and return type refer to the library's components and
  // actions with $refs, as a component's props do, but its $defs are left
  // out: a $ref to a schema no component refers to has no $defs entry.
  [
    "function-references",
    () => {
      const item = component("Item", "An item.", { text: z.string() });
      const unused = component("Unused", "", {});
      const copy = act("Copy", "Copies text.", { text: z.string() });
      return {
        components: [component("List", "A list.", { items: z.array(item.ref) }), item, unused],
        functions: [
          fn("Pick", "Picks an item.", { items: z.array(item.ref), other: unused.ref.optional() }, item.ref),
          fn("Act", "", { a: action(), go: steps.OpenUrl.ref, copy: copy.ref }, z.array(copy.ref)),
        ],
        actions: [copy],
        root: "List",
      };
    },
  ],
  // A component of the action slot's name wins its id: the slot is written
  // inline in a call's params too.
  [
    "function-action-slot-shadowed-by-component",
    () => ({
      components: [component("ActionExpression", "x", {}), component("A", "a", {})],
      functions: [fn("F", "", { a: action(), maybe: action().optional() }, action())],
    }),
  ],
  // A library a program can call functions in, for the runtime's tests
  // (crates/openui-runtime/tests/library_functions.rs): a holder of any
  // values, and functions with checked params and results.
  [
    "functions-in-a-program",
    () => ({
      components: [component("Box", "Holds values.", { a: z.any(), b: z.any().optional(), c: z.any().optional() })],
      functions: [
        fn("Percent", "Percent of a total.", { part: z.number(), total: z.number() }, z.number()),
        fn("Label", "A label for a value.", { text: z.string(), suffix: z.string().optional() }, z.string()),
        fn("Unimplemented", "Has no implementation.", { v: z.any() }),
      ],
      root: "Box",
    }),
  ],
  ["functions-without-components", () => ({ components: [], functions: [fn("F", "f", { x: z.number() }, z.number())] })],
  // A custom action: its signature, its params in the schema's actions, and
  // a $defs entry {type: const name, params} when a component's slot refers
  // to it, printed @Name.
  [
    "custom-action",
    () => {
      const copy = act("Copy", "Copies text to the clipboard.", { text: z.string(), label: z.string().optional() });
      return {
        components: [
          component("Button", "A button.", { label: z.string(), action: z.union([copy.ref, steps.OpenUrl.ref]).optional() }),
          component("Share", "", { copy: copy.ref }),
        ],
        actions: [copy],
      };
    },
  ],
  // Not referred to by any component: no $defs entry.
  [
    "custom-action-unreferenced",
    () => calls([], [act("Copy", "Copies text.", { text: z.string() }), act("Ping", "", {})]),
  ],
  // In an array (not a union, so no parentheses), beside the action slot,
  // nullable, through an escaped $ref; names in JavaScript key order.
  [
    "custom-action-kinds",
    () => {
      const slash = act("a/b~c", "escaped", {});
      const copy = act("Copy", "", { text: z.string() });
      const nine = act("9", "nine", { n: z.number().nullable() });
      return {
        components: [
          component("A", "a", {
            list: z.array(copy.ref),
            either: z.union([copy.ref, action()]),
            maybe: copy.ref.nullable(),
            escaped: slash.ref,
            many: z.array(z.union([slash.ref, nine.ref])).optional(),
          }),
        ],
        actions: [copy, slash, nine],
      };
    },
  ],
  // An action's params refer to components and other actions: their $defs
  // come where zod first meets them, inside the action's entry.
  [
    "custom-action-references",
    () => {
      const item = component("Item", "i", { t: z.string() });
      const other = component("Other", "o", {});
      const b = act("B", "b", { o: other.ref, act: action().optional() });
      const c = act("C", "c", { item: item.ref, b: b.ref, list: z.array(b.ref).optional() });
      return { components: [component("Root", "r", { c: c.ref, item: item.ref }), item, other], actions: [c, b], root: "Root" };
    },
  ],
  // Spec keys: root, components, componentGroups, functions, actions,
  // schema, id.
  [
    "functions-and-actions-with-groups",
    () => ({
      components: [component("A", "a", {})],
      componentGroups: [{ name: "G", components: ["A"], notes: ["n"] }],
      functions: [fn("F", "f", {}), fn("G", "g", { x: z.string() }, z.string())],
      actions: [act("Z", "z", { y: z.boolean() })],
      root: "A",
      id: "calls",
    }),
  ],
  // Object identity (#134 review): a schema shared by identity that zod
  // writes inline, as the core does, still converts: two actions sharing a
  // params object with no path back to it, a function whose params or return
  // type is an action's params object (the root of a call's conversion is
  // written in place), and an inline object referring to the component that
  // meets it first, then shared by a later one.
  [
    "actions-sharing-params",
    () => {
      const params = z.object({ t: z.string() });
      const copy = defineAction({ name: "Copy", description: "", params });
      const h = defineAction({ name: "H", description: "", params });
      return { components: [component("A", "a", { h: h.ref, c: copy.ref })], actions: [h, copy] };
    },
  ],
  [
    "function-params-are-action-params",
    () => {
      const params: any = z.object({
        get next() {
          return copy.ref;
        },
      });
      const copy = defineAction({ name: "Copy", description: "", params });
      return {
        components: [component("A", "a", { c: copy.ref })],
        functions: [defineFunction({ name: "F", description: "", params, fn: () => null })],
        actions: [copy],
      };
    },
  ],
  [
    "function-returns-action-params",
    () => {
      const params: any = z.object({
        get next() {
          return copy.ref;
        },
      });
      const copy = defineAction({ name: "Copy", description: "", params });
      return {
        components: [component("A", "a", {})],
        functions: [defineFunction({ name: "F", description: "", params: z.object({}), returns: params, fn: () => null })],
        actions: [copy],
      };
    },
  ],
  [
    "inline-object-shared-after-its-cycle",
    () => {
      const q: any = z.object({
        get c() {
          return c.ref;
        },
      });
      const c: any = component("C", "c", { x: q });
      return { components: [c, component("D", "d", { y: q })] };
    },
  ],
  // Name collisions: upstream's message, for the first name that collides.
  ["function-collides-with-builtin-function", () => collision("function", "Count")],
  ["function-collides-with-builtin-step", () => collision("function", "OpenUrl")],
  ["function-collides-with-action-builtin", () => collision("function", "Action")],
  ["function-collides-with-reserved-query", () => collision("function", "Query")],
  ["function-collides-with-reserved-mutation", () => collision("function", "Mutation")],
  // `name in RESERVED_CALLS` sees the names every object inherits.
  ["function-collides-with-inherited-name", () => collision("function", "toString")],
  ["function-collides-with-proto", () => collision("function", "__proto__")],
  ["function-collides-with-component", () => collision("function", "Item")],
  ["function-collides-with-function", () => collision("function", "F", { functions: [fn("F", "", {})] })],
  ["action-collides-with-builtin-function", () => collision("action", "Sum")],
  ["action-collides-with-builtin-step", () => collision("action", "ToAssistant")],
  ["action-collides-with-reserved-call", () => collision("action", "Mutation")],
  ["action-collides-with-inherited-name", () => collision("action", "constructor")],
  ["action-collides-with-component", () => collision("action", "Item")],
  ["action-collides-with-function", () => collision("action", "F", { functions: [fn("F", "", {})] })],
  ["action-collides-with-action", () => collision("action", "F", { actions: [act("F", "", {})] })],
  // A __proto__ component is not in the record, so its name is not taken
  // (the name is a reserved call anyway); the root is checked first.
  ["function-beside-proto-component", () => ({ components: [component("__proto__", "p", {}), component("A", "a", {})], functions: [fn("Proto", "", {})] })],
  ["root-checked-before-collisions", () => collision("function", "Item", { root: "Nope" })],
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
  ["meta-description-only", () => one({ s: z.string().meta({ description: "d" }) }), ".meta()"],
  ["meta-empty", () => one({ s: z.string().meta({}) }), ".meta()"],
  ["describe-number", () => one({ n: z.number().describe("d") }), "description on number"],
  ["describe-optional-number", () => one({ n: z.number().optional().describe("d") }), "description on optional number"],
  ["describe-enum", () => one({ e: z.enum(["a"]).describe("d") }), "description on enum"],
  ["describe-object", () => one({ o: z.object({}).describe("d") }), "description on object"],
  ["describe-array", () => one({ a: z.array(z.string()).describe("d") }), "description on array"],
  ["describe-extra-argument", () => one({ s: (z.string().describe as any)("d", "extra") }), "only a single string-valued .describe()"],
  ["describe-non-string", () => one({ s: z.string().describe(42 as any) }), "non-string description"],
  ["describe-plus-metadata", () => one({ s: z.string().meta({ title: "t" }).describe("d") }), "description plus other metadata"],
  ["meta-plus-description", () => one({ s: z.string().meta({ description: "d", title: "t" }) }), "description plus other metadata"],
  ["meta-after-describe", () => one({ s: z.string().describe("d").meta({ title: "t" }) }), "description plus other metadata"],
  ["describe-literal", () => one({ l: z.literal("x").describe("d") }), "description on literal"],
  ["describe-twice", () => one({ s: z.string().describe("a").describe("b") }), "repeated description"],
  ["describe-inner-and-outer", () => one({ s: z.string().describe("a").optional().describe("b") }), "repeated description"],
  ["default-undefined", () => one({ s: z.any().default(undefined) }), "JSON default"],
  ["default-bigint", () => one({ s: z.any().default(2n) }), "JSON default", "BigInt defaults"],
  ["default-inner-binding", () => one({ s: bound(z.string()).default("v") }), "reactive"],
  ["default-on-checked-type", () => one({ s: z.string().min(1).default("x") }), "checks"],
  ["loose-object", () => one(z.looseObject({ v: z.string() })), "catch-all"],
  ["catchall", () => one(z.object({ v: z.string() }).catchall(z.number())), "catch-all"],
  ["refined-registered-props", () => {
    const base = z.object({ v: z.string() });
    return { components: [component("Base", "", base), component("Derived", "", base.refine(() => true))] };
  }, "registered parent"],
  ["refined-inline-object", () => one({ v: z.object({}).refine(() => false) }), "checks"],
  ["refined-props-catchall", () => one(z.looseObject({}).refine(() => false)), "catch-all"],
  ["refined-props-metadata", () => one(z.object({}).refine(() => false).describe("d")), "metadata"],
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
  ["array-min-max", () => one({ a: z.array(z.string()).min(1).max(2) }), "checks"],
  ["array-length", () => one({ a: z.array(z.string()).length(2) }), "checks"],
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
  ["describe-nested-number", () => one({ a: z.array(z.number().describe("d")) }), "description on number"],
  ["meta-object", () => one({ o: z.object({}).meta({ title: "t" }) }), "metadata"],
  // A binding anywhere but on a prop's outermost type (#133): upstream
  // prints $binding<...> there but never binds it.
  ["reactive-nested", () => one({ a: z.array(bound(z.string())) }), "reactive"],
  ["reactive-in-union", () => one({ u: z.union([bound(z.string()), z.number()]) }), "reactive"],
  ["reactive-in-field", () => one({ o: z.object({ x: bound(z.string()) }) }), "reactive"],
  ["reactive-in-record", () => one({ r: bound(z.record(z.string(), bound(z.number()))) }), "reactive"],
  // Under .optional() or .nullable() upstream ignores it altogether.
  ["reactive-inside-optional", () => one({ o: bound(z.string()).optional() }), "reactive"],
  ["reactive-inside-nullable", () => one({ n: bound(z.string()).nullable() }), "reactive"],
  ["reactive-field", () => one({ o: z.object({ x: bound(z.string().optional()) }) }), "reactive"],
  // Marking a component's props object marks every reference to it.
  [
    "reactive-component-props",
    () => {
      const item = component("Item", "", {});
      return { components: [component("A", "a", { i: bound(item.ref) }), item] };
    },
    "reactive",
  ],
  // Upstream never reads a prompt name on .optional().
  ["tag-on-optional", () => one({ o: named(z.string().optional(), "Opt") }), "prompt name on .optional()"],
  // A reference the plain data's names can't say.
  [
    "reference-to-empty-name",
    () => {
      const empty = component("", "e", {});
      return { components: [component("A", "a", { e: empty.ref }), empty] };
    },
    "empty name",
  ],
  [
    "reference-to-shadowed-duplicate",
    () => {
      const first = component("K", "first", {});
      return { components: [first, component("A", "a", { k: first.ref }), component("K", "later", {})] };
    },
    "shadowed",
    'Duplicate schema id "K"',
  ],
  [
    "reference-to-first-of-two-proto",
    () => {
      const first = component("__proto__", "p", { v: z.string() });
      return { components: [first, component("__proto__", "q", {}), component("A", "a", { p: first.ref })] };
    },
    "shadowed",
  ],
  // An action outside the library is no reference: its ref holds a
  // z.literal, which upstream prints and writes inline.
  [
    "custom-action-outside-library",
    () => one({ a: defineAction({ name: "Copy", description: "c", params: z.object({ text: z.string() }) }).ref as any }),
    'kind "literal"',
  ],
  // Functions and actions (#134): what the plain data can't say.
  [
    "action-named-action-expression",
    () => {
      const ae = act("ActionExpression", "", { x: z.string() });
      return { components: [component("A", "a", { mine: ae.ref, slot: action() })], actions: [ae] };
    },
    "named ActionExpression",
    'Duplicate schema id "ActionExpression"',
  ],
  [
    "reference-to-empty-named-action",
    () => {
      const empty = act("", "", {});
      return { components: [component("A", "a", { e: empty.ref })], actions: [empty] };
    },
    "empty name",
  ],
  ["reactive-function-param", () => calls([fn("F", "", { v: bound(z.string()) })]), "reactive"],
  ["reactive-action-param", () => calls([], [act("G", "", { v: bound(z.string().optional()) })]), "reactive"],
  ["reactive-return", () => calls([fn("F", "", {}, bound(z.string()))]), "reactive"],
  ["optional-return", () => calls([fn("F", "", {}, z.string().optional())]), "optional inside"],
  ["return-with-check", () => calls([fn("F", "", {}, z.string().min(1))]), "checks"],
  ["function-param-int", () => calls([fn("F", "", { n: z.int() })]), '"check"'],
  [
    "function-params-shared-with-component",
    () => {
      const a = component("A", "a", { v: z.string() });
      return { components: [a], functions: [defineFunction({ name: "F", description: "", params: a.props, fn: () => null })] };
    },
    "shared with a component",
  ],
  [
    "action-params-shared-with-component",
    () => {
      const a = component("A", "a", { v: z.string() });
      return { components: [a], actions: [defineAction({ name: "G", description: "", params: a.props })] };
    },
    "shared with a component",
  ],
  ["function-extra-field", () => ({ ...one({}), functions: [{ ...fn("F", "", {}), lazy: true }] }), 'field "lazy"'],
  ["action-extra-field", () => ({ ...one({}), actions: [{ ...act("G", "", {}), returns: z.string() }] }), 'field "returns"'],
  [
    "action-ref-shared",
    () => {
      const g = act("G", "", {});
      return { ...one({}), actions: [g, { ...act("H", "", {}), ref: g.ref }] };
    },
    "shared with another action",
  ],
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
  ["library-extra-field", () => ({ ...one({}), tools: [] }), 'field "tools"'],
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
// Declarations where zod writes a schema as a "__schemaN" $defs entry (or a
// call's $ref to one): a schema met again inside itself through a reference,
// of each kind. The plain data has no object identity, so the converter
// refuses them; the run checks that upstream's output still has the entry.
// `shared(t)` is a type met first under D and again under C, which D's
// type refers to: zod meets it again while still writing it.
const cyclic = (make: (back: () => z.ZodType) => z.ZodType) => () => {
  const c: any = component("C", "c", {
    get x() {
      return shared;
    },
  });
  const shared: any = make(() => c.ref);
  return { components: [component("D", "d", { y: shared }), c] };
};
const loop = (back: () => z.ZodType) =>
  z.object({
    get c() {
      return back();
    },
  });
const extractedRefusals: [string, () => unknown][] = [
  // V1: two actions share a params object that refers to one of them.
  [
    "two-actions-share-params-on-a-cycle",
    () => {
      const params: any = z.object({
        get next() {
          return copy.ref;
        },
      });
      const copy = defineAction({ name: "Copy", description: "", params });
      const h = defineAction({ name: "H", description: "", params });
      return { components: [component("A", "a", { h: h.ref })], actions: [h, copy] };
    },
  ],
  // V2: an action's params object, referring to the action, as a prop's type.
  [
    "action-params-as-prop-type-on-a-cycle",
    () => {
      const params: any = z.object({
        get next() {
          return copy.ref;
        },
      });
      const copy = defineAction({ name: "Copy", description: "", params });
      return { components: [component("A", "a", { p: params })], actions: [copy] };
    },
  ],
  // V5: an inline object referring to a component, met first under another.
  ["inline-object-on-a-cycle", cyclic(loop)],
  // The same for each kind that holds another type.
  ["array-on-a-cycle", cyclic((back) => z.array(loop(back)))],
  ["union-on-a-cycle", cyclic((back) => z.union([z.string(), loop(back)]))],
  ["record-on-a-cycle", cyclic((back) => z.record(z.string(), loop(back)))],
  ["nullable-on-a-cycle", cyclic((back) => loop(back).nullable())],
  ["optional-on-a-cycle", cyclic((back) => loop(back).optional())],
  ["refinement-parent-on-a-cycle", () => {
    const base = z.object({ get child(): z.ZodType { return child.ref; } });
    const child = component("Child", "", base.refine(() => true));
    return { components: [component("Page", "", { base }), child] };
  }],
  ["default-on-a-cycle", cyclic((back) => loop(back).default({}))],
  // V6: a call's param type met again inside itself through an action: the
  // call's $ref to its entry is kept though the entry is dropped.
  [
    "function-param-on-a-cycle",
    () => {
      const q: any = z.object({
        get a() {
          return copy.ref;
        },
      });
      const copy = defineAction({ name: "Copy", description: "", params: z.object({ q }) });
      return {
        components: [component("A", "a", { c: copy.ref })],
        functions: [defineFunction({ name: "F", description: "", params: z.object({ q }), fn: () => null })],
        actions: [copy],
      };
    },
  ],
  [
    "action-param-on-a-cycle",
    () => {
      const q: any = z.object({
        get a() {
          return copy.ref;
        },
      });
      const copy = defineAction({ name: "Copy", description: "", params: z.object({ q }) });
      return { components: [component("A", "a", {})], actions: [copy, defineAction({ name: "G", description: "", params: z.object({ q }) })] };
    },
  ],
];
for (const [name, make] of extractedRefusals) {
  let message: string | undefined;
  try {
    toLibraryDefinition(make() as any);
  } catch (e) {
    message = (e as Error).message;
  }
  if (!message?.includes("__schemaN")) throw new Error(`refusal ${name}: not refused as __schemaN: ${message}`);
  const output = JSON.stringify(createLibrary(make() as Input).toSpec());
  if (!output.includes("__schema")) throw new Error(`refusal ${name}: upstream no longer writes a __schemaN entry`);
}

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

// Upstream keys marks by identity. Applying the mark to the described clone
// works; a mark applied before Zod clones the string stays on its parent.
const original = named(bound(z.string()), "Token");
const clone = original.describe("field text");
const markedInput = one({ original, clone, markedClone: named(bound(z.string().describe("field text")), "Token") });
const marked = toLibraryDefinition(markedInput as any).components[0].props;
const signature = createLibrary(markedInput).toSpec().components.A.signature;
if (signature !== "A(original: $binding<Token>, clone: string, markedClone: $binding<Token>)" ||
    JSON.stringify(marked.map(p => [p.type, !!p.binding])) !== JSON.stringify([
      [{ kind: "named", name: "Token", type: { kind: "string" } }, true],
      [{ kind: "string" }, false],
      [{ kind: "named", name: "Token", type: { kind: "string" } }, true],
    ])) {
  throw new Error(`described clone marks: ${signature}, ${JSON.stringify(marked)}`);
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
console.log(`${out.length} library-definition observations, ${refusals.length + extractedRefusals.length} converter refusals`);

// Prompts from a defined library (#135): upstream's library.prompt(options)
// and the cloud config generateSystemPrompt({cloud: true, library:
// library.toSpec(), promptOptions, instructions}) makes, for every sample
// createLibrary accepts, written to conformance/golden/library-definition-prompts.json
// as {prompts: [{name, options?, prompt}], cloud: [{name, promptOptions?,
// instructions?, config | error}]}. Every sample's prompt and cloud config
// with no options; the bindings mode for every sample with a binding, an
// action slot, a function or an action (the $binding and action text); and
// the other options on a few.
const { generateSystemPrompt } = await import("./lang-core/src/parser/prompt");
const tool = {
  name: "list_items",
  description: "Lists the items.",
  inputSchema: { type: "object", properties: { q: { type: "string" } } },
  outputSchema: { type: "array", items: { type: "string" } },
  annotations: { readOnlyHint: true },
};
const linked = (definition: unknown) => {
  const d = definition as { functions?: unknown[]; actions?: unknown[] };
  const text = JSON.stringify(definition);
  return text.includes('"binding":true') || text.includes('"kind":"action"') || !!d.functions?.length || !!d.actions?.length;
};
const everyOption = ["scalars", "binding", "action-slot", "functions-and-actions-with-groups"];
const promptOptions: object[] = [
  { tools: ["search", tool], toolExamples: ['root = Page("t", [])'] },
  { toolCalls: true, bindings: false, builtinFunctions: false },
  { bindings: true, builtinFunctions: false },
  { preamble: "You draw pages.", additionalRules: ["Be brief.", "No tables."], examples: ['root = Page("x", [])'] },
  { editMode: true, inlineMode: true },
  { root: "Elsewhere", componentGroups: null },
];
const cloudOptions: { promptOptions?: object; instructions?: string }[] = [
  { promptOptions: { examples: ['root = Page("x", [])'], preamble: "P", additionalRules: ["R"], tools: ["dropped"] } },
  { promptOptions: { editMode: true }, instructions: "Answer briefly." },
  { promptOptions: {}, instructions: "" },
];
const prompts: object[] = [];
const cloud: object[] = [];
for (const [name, make] of samples) {
  const input = make();
  let library: ReturnType<typeof createLibrary>;
  try {
    library = createLibrary(input);
  } catch {
    continue;
  }
  const definition = toLibraryDefinition(input as any);
  const variants: (object | undefined)[] = [undefined];
  if (everyOption.includes(name)) variants.push({ bindings: true }, ...promptOptions);
  else if (linked(definition)) variants.push({ bindings: true });
  for (const options of variants) {
    prompts.push({ name, ...(options ? { options } : {}), prompt: library.prompt(options as any) });
  }
  const cloudVariants = everyOption.includes(name) ? [{}, ...cloudOptions] : [{}];
  for (const variant of cloudVariants) {
    try {
      cloud.push({ name, ...variant, config: generateSystemPrompt({ cloud: true, library: library.toSpec(), ...variant } as any) });
    } catch (e) {
      cloud.push({ name, ...variant, error: (e as Error).message });
    }
  }
}
writeFileSync(
  join(CONFORMANCE, "golden", "library-definition-prompts.json"),
  JSON.stringify({ prompts, cloud }, null, 1) + "\n",
);
console.log(`${prompts.length} prompts and ${cloud.length} cloud configs from defined libraries`);

// The root a defined library gives its parser (#135): react-lang's
// useOpenUIState creates its streaming parser with library.root
// (createStreamingParser(schema, library.root)), so a program with no
// `root` statement draws from the root component's statement. Written to
// conformance/golden/library-definition-roots.json as [{name, root,
// program, result}]: the parser with the library's own root, and with a
// root given in its place (as a prompt spec's root is).
const { createStreamingParser } = await import("./lang-core/src/parser/parser");
const rootPrograms = [
  'h = Heading("Hi")\np = Page("Title", [h])',
  'p = Page("Title", [t])\nt = Text("x")',
  'h = Heading("Hi")\nroot = Text("y")',
  'h = Heading("Hi", 2)',
];
const roots: object[] = [];
{
  const library = createLibrary(scalars);
  for (const program of rootPrograms) {
    for (const root of [library.root, "Heading", undefined]) {
      const parser = createStreamingParser(library.toJSONSchema(), root);
      roots.push({ name: "scalars", ...(root !== undefined ? { root } : {}), program, result: parser.set(program) });
    }
  }
}
writeFileSync(join(CONFORMANCE, "golden", "library-definition-roots.json"), JSON.stringify(roots, null, 1) + "\n");
console.log(`${roots.length} parses with a defined library's root`);
