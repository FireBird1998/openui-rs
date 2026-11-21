// The converter from upstream's zod declarations to the core's plain-data
// library definition (crates/openui-lang/src/library_definition.rs documents
// the form). It takes what upstream's createLibrary takes, components made by
// defineComponent, component groups, a root and an id, and returns the plain
// data that should give the same JSON Schema and prompt spec in the core.
//
// It refuses, naming it, anything the plain-data form can't carry, so nothing
// is silently approximated and the proof compares like with like. It works
// from allow-lists rather than a list of known-bad cases: each supported zod
// kind names the fields its def may have (and `checks` only when empty), so a
// format (z.int(), z.email(), z.iso.date()), a check (.min()), a coercion, a
// catch-all on an object, a discriminated or exclusive union (z.xor), a
// partial or loose record, or anything zod adds later is refused by the
// field's name, and a kind not listed (z.literal, z.tuple, z.intersection,
// z.map, z.lazy, ...) by its name. Beyond the def it refuses, on every type at
// every depth, metadata (.describe(), .meta()), a binding, and a schema with a
// prompt name (a component's props used as a type, an action slot, a tagged
// type): those are #133's. It refuses symbol keys in an object's shape
// (upstream throws on them), a type nested deeper than the core's default
// depth limit (which also stops a recursive getter schema by name), and
// .optional() anywhere but directly on a
// prop or an object field (inside a union zod makes the whole prop optional,
// which the plain data can't say), an enum whose keys aren't its values (an
// object map or native enum: upstream prints the keys in the signature and
// the values in the schema), a record whose key isn't a bare z.string(), a
// props object shared by two components (zod then writes one as a $ref to the
// other, and the plain data has no identity to carry that), a component
// without a renderer, and a field the plain-data form doesn't have on the
// library, a component or a component group.
import { ACTION_DEFS } from "./lang-core/src/parser/builtins";
import { isReactiveSchema } from "./lang-core/src/reactive";
import { schemaIdTags } from "./lang-core/src/signature";
import * as z from "zod/v4/core";

export type TypeDescription =
  | { kind: "string" }
  | { kind: "number" }
  | { kind: "boolean" }
  | { kind: "any" }
  | { kind: "enum"; values: string[] }
  | { kind: "array"; items: TypeDescription }
  | { kind: "union"; options: TypeDescription[] }
  | { kind: "object"; fields: FieldDefinition[] }
  | { kind: "record"; values: TypeDescription }
  | { kind: "nullable"; type: TypeDescription };
export type FieldDefinition = { name: string; type: TypeDescription; optional?: true };
export type PropDefinition = FieldDefinition;
export type ComponentDefinition = { name: string; description: string; props: PropDefinition[] };
export type ComponentGroup = { name: string; components: string[]; notes?: string[] };
export type LibraryDefinition = {
  components: ComponentDefinition[];
  componentGroups?: ComponentGroup[];
  root?: string;
  id?: string;
};

type Declared = {
  components: { name: string; description: string; props: z.$ZodObject; component: unknown }[];
  componentGroups?: ComponentGroup[];
  root?: string;
  id?: string;
};

// The def fields each supported zod kind may have. `checks` is allowed only
// when empty (z.number() always has one).
const SCALAR_DEFS: Record<string, string[]> = {
  string: ["type", "checks"],
  number: ["type", "checks"],
  boolean: ["type", "checks"],
  any: ["type", "checks"],
};
const OPTIONAL_DEF = ["type", "innerType", "checks"];
const NULLABLE_DEF = ["type", "innerType", "checks"];
const ENUM_DEF = ["type", "entries", "checks"];
const ARRAY_DEF = ["type", "element", "checks"];
const UNION_DEF = ["type", "options", "checks"];
const RECORD_DEF = ["type", "keyType", "valueType", "checks"];
// z.strictObject's catch-all is a bare z.never(), which zod writes as
// z.object writes no catch-all; any other catch-all is refused.
const OBJECT_DEF = ["type", "shape", "catchall", "checks"];
const NEVER_DEF = ["type", "checks"];

// The fields the library, a component (what defineComponent returns) and a
// component group may have; a group's in the order the core writes them.
const LIBRARY_FIELDS = ["components", "componentGroups", "root", "id"];
const COMPONENT_FIELDS = ["name", "props", "description", "component", "ref"];
const GROUP_ORDERS = [
  ["name", "components"],
  ["name", "components", "notes"],
];

function def(schema: unknown): any {
  return (schema as any)?._zod?.def;
}

function refuseUnknownFields(value: object, allowed: string[], where: string) {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) throw new Error(`${where}: field "${key}" isn't supported`);
  }
}

// The def holds only the allowed fields, no checks, and nothing outside the
// def changes what zod writes.
function refuseDetails(schema: z.$ZodType, allowed: string[], where: string) {
  const d = def(schema);
  if (d === undefined || d === null || typeof d !== "object") throw new Error(`${where}: not a zod schema`);
  for (const key of Object.keys(d)) {
    if (!allowed.includes(key)) throw new Error(`${where}: zod ${d.type} with "${key}" (a format, coercion or other detail) isn't supported yet`);
  }
  if (d.checks !== undefined && (!Array.isArray(d.checks) || d.checks.length > 0)) {
    throw new Error(`${where}: checks (${d.type} with .min() and the like) aren't supported yet`);
  }
  if (z.globalRegistry.get(schema) !== undefined) throw new Error(`${where}: metadata (.describe(), .meta()) isn't supported yet`);
  if (isReactiveSchema(schema)) throw new Error(`${where}: reactive (binding) isn't supported yet`);
}

const ACTION_SCHEMAS = new Set<unknown>(Object.values(ACTION_DEFS));

// A type, at any depth: a scalar, or one of the kinds that hold other types.
// How deep a type may nest, a prop's own type being level 1: the core's
// default limit (Limits::depth). Counting also stops a recursive schema (a
// getter returning its own object) by name instead of by the stack.
export const MAX_TYPE_DEPTH = 256;

function typeOf(schema: z.$ZodType, where: string, depth: number): TypeDescription {
  if (depth > MAX_TYPE_DEPTH) {
    throw new Error(`${where}: types nest deeper than ${MAX_TYPE_DEPTH} levels (a recursive schema?), which the core refuses`);
  }
  const d = def(schema);
  const kind = d?.type;
  if (ACTION_SCHEMAS.has(schema)) throw new Error(`${where}: an action slot isn't supported yet`);
  if (typeof schema === "object" && schema !== null && schemaIdTags.has(schema)) {
    throw new Error(`${where}: a named schema (a component reference, action slot or tagged type) isn't supported yet`);
  }
  switch (kind) {
    case "optional":
      throw new Error(`${where}: optional inside a type (anywhere but on a prop or an object field) isn't supported`);
    case "enum": {
      refuseDetails(schema, ENUM_DEF, where);
      const entries = Object.entries(d.entries as Record<string, unknown>);
      if (!entries.every(([key, value]) => typeof value === "string" && key === value)) {
        throw new Error(`${where}: an enum whose keys aren't its string values (an object map or native enum) isn't supported`);
      }
      // Object.keys order: array-index words first, a repeated word once.
      return { kind, values: entries.map(([key]) => key) };
    }
    case "array":
      refuseDetails(schema, ARRAY_DEF, where);
      return { kind, items: typeOf(d.element, `${where}[]`, depth + 1) };
    case "union":
      refuseDetails(schema, UNION_DEF, where);
      return { kind, options: (d.options as z.$ZodType[]).map((o, i) => typeOf(o, `${where}|${i}`, depth + 1)) };
    case "object":
      return { kind, fields: fieldsOf(schema as z.$ZodObject, where, depth + 1) };
    case "record": {
      refuseDetails(schema, RECORD_DEF, where);
      const key = d.keyType;
      if (def(key)?.type !== "string") throw new Error(`${where}: a record key other than z.string() isn't supported`);
      refuseDetails(key, SCALAR_DEFS.string, `${where} record key`);
      return { kind, values: typeOf(d.valueType, `${where}{}`, depth + 1) };
    }
    case "nullable":
      refuseDetails(schema, NULLABLE_DEF, where);
      return { kind, type: typeOf(d.innerType, `${where}?`, depth + 1) };
  }
  const allowed = SCALAR_DEFS[kind];
  if (!Object.hasOwn(SCALAR_DEFS, kind) || allowed === undefined) throw new Error(`${where}: zod kind "${kind}" isn't supported yet`);
  refuseDetails(schema, allowed, where);
  return { kind } as TypeDescription;
}

// A prop or an object field: .optional() is allowed here and only here.
function fieldOf(name: string, schema: z.$ZodType, where: string, depth: number): FieldDefinition {
  if (def(schema)?.type === "optional") {
    refuseDetails(schema, OPTIONAL_DEF, where);
    return { name, type: typeOf(def(schema).innerType, where, depth), optional: true };
  }
  return { name, type: typeOf(schema, where, depth) };
}

// The fields of a z.object (a component's props or an inline object), in
// Object.entries order; `depth` is its fields' types' level. Object.entries
// skips symbol keys, which upstream refuses ("Symbol keys cannot be
// represented in JSON Schema"), so a shape with one is refused here.
function fieldsOf(object: z.$ZodObject, where: string, depth: number): FieldDefinition[] {
  const d = def(object);
  if (d?.type !== "object" || !d.shape) throw new Error(`${where}: props must be z.object(...)`);
  refuseDetails(object, OBJECT_DEF, where);
  if (d.catchall !== undefined) {
    if (def(d.catchall)?.type !== "never") throw new Error(`${where}: an object with a catch-all (z.looseObject, .catchall()) isn't supported`);
    refuseDetails(d.catchall, NEVER_DEF, `${where} catch-all`);
  }
  if (Object.getOwnPropertySymbols(d.shape).length > 0) {
    throw new Error(`${where}: an object with symbol keys isn't supported (upstream can't write it as JSON Schema)`);
  }
  return Object.entries(d.shape).map(([name, s]) => fieldOf(name, s as z.$ZodType, `${where}.${name}`, depth));
}

function isStrings(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function groupOf(g: ComponentGroup, where: string): ComponentGroup {
  const keys = Object.keys(g);
  // upstream's toSpec writes a group as given, so its fields and their order
  // must be the ones the core writes.
  if (!GROUP_ORDERS.some((order) => order.length === keys.length && order.every((k, i) => keys[i] === k))) {
    throw new Error(`${where}: a component group's fields must be name, components and optional notes, in that order (got ${keys.join(", ")})`);
  }
  if (typeof g.name !== "string" || !isStrings(g.components) || (g.notes !== undefined && !isStrings(g.notes))) {
    throw new Error(`${where}: a component group's name is a string and its components and notes are lists of strings`);
  }
  return {
    name: g.name,
    components: [...g.components],
    ...(g.notes !== undefined ? { notes: [...g.notes] } : {}),
  };
}

/** The plain-data form of what `createLibrary(input)` is given. */
export function toLibraryDefinition(input: Declared): LibraryDefinition {
  refuseUnknownFields(input, LIBRARY_FIELDS, "library");
  const seen = new Set<unknown>();
  const components = input.components.map((c, i) => {
    const where = typeof c.name === "string" ? c.name : `components[${i}]`;
    refuseUnknownFields(c, COMPONENT_FIELDS, where);
    if (typeof c.name !== "string" || typeof c.description !== "string") {
      throw new Error(`${where}: a component's name and description are strings`);
    }
    // The core takes every component as having a renderer, as every real
    // library's does; upstream's root check can read it (a component named
    // __proto__), so a falsy one would not compare like with like.
    if (!c.component) throw new Error(`${where}: a component without a renderer isn't supported`);
    if (seen.has(c.props)) throw new Error(`${where}: a props object shared with another component isn't supported`);
    seen.add(c.props);
    return { name: c.name, description: c.description, props: fieldsOf(c.props, where, 1) };
  });
  if (input.root !== undefined && typeof input.root !== "string") throw new Error("library: root is a string");
  if (input.id !== undefined && typeof input.id !== "string") throw new Error("library: id is a string");
  return {
    components,
    ...(input.componentGroups !== undefined
      ? { componentGroups: input.componentGroups.map((g, i) => groupOf(g, `componentGroups[${i}]`)) }
      : {}),
    ...(input.root !== undefined ? { root: input.root } : {}),
    ...(input.id !== undefined ? { id: input.id } : {}),
  };
}
