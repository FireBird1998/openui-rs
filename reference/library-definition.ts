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
// catch-all on the props object or anything zod adds later is refused by the
// field's name. Beyond the def it refuses metadata (.describe(), .meta()), a
// binding, a props object shared by two components (zod then writes one as a
// $ref to the other, and the plain data has no identity to carry that), a
// component without a renderer, and a field the plain-data form doesn't have
// on the library, a component or a component group.
import { isReactiveSchema } from "./lang-core/src/reactive";
import * as z from "zod/v4/core";

export type TypeDescription =
  | { kind: "string" }
  | { kind: "number" }
  | { kind: "boolean" }
  | { kind: "any" };
export type PropDefinition = { name: string; type: TypeDescription; optional?: true };
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

function typeOf(schema: z.$ZodType, where: string): TypeDescription {
  const kind = def(schema)?.type;
  const allowed = SCALAR_DEFS[kind];
  if (!Object.hasOwn(SCALAR_DEFS, kind) || allowed === undefined) throw new Error(`${where}: zod kind "${kind}" isn't supported yet`);
  refuseDetails(schema, allowed, where);
  return { kind } as TypeDescription;
}

function propOf(name: string, schema: z.$ZodType, where: string): PropDefinition {
  if (def(schema)?.type === "optional") {
    refuseDetails(schema, OPTIONAL_DEF, where);
    return { name, type: typeOf(def(schema).innerType, where), optional: true };
  }
  return { name, type: typeOf(schema, where) };
}

function propsOf(props: z.$ZodObject, where: string): PropDefinition[] {
  const d = def(props);
  if (d?.type !== "object" || !d.shape) throw new Error(`${where}: props must be z.object(...)`);
  refuseDetails(props, OBJECT_DEF, where);
  if (d.catchall !== undefined) {
    if (def(d.catchall)?.type !== "never") throw new Error(`${where}: a props object with a catch-all (z.looseObject, .catchall()) isn't supported`);
    refuseDetails(d.catchall, NEVER_DEF, `${where} catch-all`);
  }
  return Object.entries(d.shape).map(([name, s]) => propOf(name, s as z.$ZodType, `${where}.${name}`));
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
    return { name: c.name, description: c.description, props: propsOf(c.props, where) };
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
