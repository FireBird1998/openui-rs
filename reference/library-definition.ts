// The converter from upstream's zod declarations to the core's plain-data
// library definition (crates/openui-lang/src/library_definition.rs documents
// the form). It takes what upstream's createLibrary takes, components made by
// defineComponent, component groups, functions made by defineFunction,
// actions made by defineAction, a root and an id, and returns the plain data
// that should give the same JSON Schema and prompt spec in the core.
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
// every depth, metadata (.describe(), .meta()).
//
// The kinds that link a library together (#133) are found by identity: a
// component's props object used as a type is a reference to that component
// (refused when the name is empty, or when a later component of the same name
// shadows it: upstream then throws or registers another schema under that
// name, and the plain data refers by name); action() is the action slot and
// steps.OpenUrl.ref and steps.ToAssistant.ref the named built-in actions,
// and the ref of one of the library's own actions (#134) names that action
// (an action outside the library is no reference: its ref holds a z.literal,
// refused by that name; a reference to the action with the empty name is
// refused, as upstream writes that one inline);
// any other schema with a prompt name (tagSchemaId, or the props of a
// component outside the library) is a named type. A binding (reactive()) is
// carried on a prop's outermost schema, the .optional() wrapper included,
// and refused anywhere else: upstream prints $binding<...> inside a type too,
// but its runtime binds only a prop's outermost schema, so the plain data
// has nowhere to say it. A prompt name on an .optional() wrapper is refused
// too (upstream never reads it). It refuses symbol keys in an object's shape
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
// library, a component, a component group, a function or an action.
//
// Functions and actions (#134): their params are an object's fields (no
// binding: upstream prints $binding<...> in a call's signature too, but
// nothing binds a call's argument), a function's return type is a type
// (.optional() or a binding there is refused like anywhere inside a type), a
// params object may not be a component's props (zod then writes the call's
// params as a $ref), and an action may not be named ActionExpression
// (upstream registers it under the built-in action slot's id, and zod throws
// when a library refers to both).
//
// Object identity (#134 review): zod writes a schema with no registered id as
// a "__schemaN" $defs entry when, in one conversion, it is met again while
// it is still being written, which needs a path from it back to itself
// through a component's props or an action's ref (zod writes a registered
// schema's body where it first meets it, so a path can run through one).
// The plain data has no object identity, so the core writes such a schema
// inline; the converter refuses the declaration instead. It replays zod's
// walk for each conversion upstream makes (the library's schema, each
// function's params and return type, each action's params), in zod's order,
// so it refuses exactly those declarations whose output has such an entry
// or a $ref to one (a call's $defs are dropped, so there only a $ref written
// outside them counts): a schema met again after it was written (shared,
// with no path back to itself) or the root of a call's conversion met again
// (zod writes it in place, with a "#" $ref inside the dropped $defs) still
// converts.
import { ACTION_DEFS, action, steps } from "./lang-core/src/parser/builtins";
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
  | { kind: "nullable"; type: TypeDescription }
  | { kind: "component"; name: string }
  | { kind: "action"; name?: string }
  | { kind: "named"; name: string; type: TypeDescription };
export type FieldDefinition = { name: string; type: TypeDescription; optional?: true };
export type PropDefinition = FieldDefinition & { binding?: true };
export type ComponentDefinition = { name: string; description: string; props: PropDefinition[] };
export type ComponentGroup = { name: string; components: string[]; notes?: string[] };
export type FunctionDefinition = { name: string; description: string; params: FieldDefinition[]; returns?: TypeDescription };
export type ActionDefinition = { name: string; description: string; params: FieldDefinition[] };
export type LibraryDefinition = {
  components: ComponentDefinition[];
  componentGroups?: ComponentGroup[];
  functions?: FunctionDefinition[];
  actions?: ActionDefinition[];
  root?: string;
  id?: string;
};

type Declared = {
  components: { name: string; description: string; props: z.$ZodObject; component: unknown }[];
  componentGroups?: ComponentGroup[];
  functions?: { name: string; description: string; params: z.$ZodObject; returns?: z.$ZodType }[];
  actions?: { name: string; description: string; params: z.$ZodObject; ref: unknown }[];
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
const LIBRARY_FIELDS = ["components", "componentGroups", "functions", "actions", "root", "id"];
const COMPONENT_FIELDS = ["name", "props", "description", "component", "ref"];
// What defineFunction and defineAction return; a function's implementation
// (`fn`) never enters the core.
const FUNCTION_FIELDS = ["name", "description", "params", "returns", "fn"];
const ACTION_FIELDS = ["name", "description", "params", "ref"];
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
// def changes what zod writes. A binding is refused unless `binding` says the
// schema is a prop's outermost one.
function refuseDetails(schema: z.$ZodType, allowed: string[], where: string, binding = false) {
  const d = def(schema);
  if (d === undefined || d === null || typeof d !== "object") throw new Error(`${where}: not a zod schema`);
  for (const key of Object.keys(d)) {
    if (!allowed.includes(key)) throw new Error(`${where}: zod ${d.type} with "${key}" (a format, coercion or other detail) isn't supported yet`);
  }
  if (d.checks !== undefined && (!Array.isArray(d.checks) || d.checks.length > 0)) {
    throw new Error(`${where}: checks (${d.type} with .min() and the like) aren't supported yet`);
  }
  if (z.globalRegistry.get(schema) !== undefined) throw new Error(`${where}: metadata (.describe(), .meta()) isn't supported yet`);
  refuseBinding(schema, where, binding);
}

function refuseBinding(schema: unknown, where: string, binding: boolean) {
  if (!binding && isReactiveSchema(schema)) {
    throw new Error(`${where}: reactive (binding) anywhere but on a prop's outermost type isn't supported`);
  }
}

// The action slot and the built-in actions a restricted slot names, by
// identity, as upstream's ACTION_DEFS registers them.
const ACTION_SCHEMAS = new Map<unknown, TypeDescription>([
  [action(), { kind: "action" }],
  [steps.OpenUrl.ref, { kind: "action", name: "OpenUrl" }],
  [steps.ToAssistant.ref, { kind: "action", name: "ToAssistant" }],
]);

// A library component's props object, by identity: its name, and whether it
// is the last component of that name (the one a reference by name means).
type Owner = { name: string; last: boolean };
let owners = new Map<unknown, Owner>();
// A library action's ref, by identity: the action's name.
let actionOwners = new Map<unknown, string>();

// A type, at any depth: a scalar, or one of the kinds that hold other types.
// How deep a type may nest, a prop's own type being level 1: the core's
// default limit (Limits::depth). Counting also stops a recursive schema (a
// getter returning its own object) by name instead of by the stack.
export const MAX_TYPE_DEPTH = 256;

function refuseDepth(where: string, depth: number) {
  if (depth > MAX_TYPE_DEPTH) {
    throw new Error(`${where}: types nest deeper than ${MAX_TYPE_DEPTH} levels (a recursive schema?), which the core refuses`);
  }
}

// `binding`: the schema is a prop's outermost one, where a binding is carried.
function typeOf(schema: z.$ZodType, where: string, depth: number, binding = false): TypeDescription {
  refuseDepth(where, depth);
  const owner = owners.get(schema);
  if (owner !== undefined) {
    if (owner.name === "") throw new Error(`${where}: a reference to the component with the empty name isn't supported`);
    if (!owner.last) throw new Error(`${where}: a reference to a component shadowed by a later one of the same name isn't supported`);
    // Its props were checked as that component's; a binding is only ever
    // the prop's (a reactive props object is refused there).
    refuseBinding(schema, where, binding);
    return { kind: "component", name: owner.name };
  }
  const slot = ACTION_SCHEMAS.get(schema);
  if (slot !== undefined) {
    if (z.globalRegistry.get(schema as z.$ZodType) !== undefined) throw new Error(`${where}: metadata (.describe(), .meta()) isn't supported yet`);
    if (slot.kind === "action" && slot.name === undefined && schemaIdTags.get(schema as object) !== "ActionExpression") {
      throw new Error(`${where}: an action slot whose prompt name was changed isn't supported`);
    }
    refuseBinding(schema, where, binding);
    return { ...slot };
  }
  const custom = actionOwners.get(schema);
  if (custom !== undefined) {
    if (custom === "") throw new Error(`${where}: a reference to the action with the empty name isn't supported`);
    if (z.globalRegistry.get(schema as z.$ZodType) !== undefined) throw new Error(`${where}: metadata (.describe(), .meta()) isn't supported yet`);
    refuseBinding(schema, where, binding);
    return { kind: "action", name: custom };
  }
  if (typeof schema === "object" && schema !== null && schemaIdTags.has(schema)) {
    const name = schemaIdTags.get(schema) as string;
    // The tag names the schema object; its structure is one level in.
    return { kind: "named", name, type: structureOf(schema, where, depth + 1, binding) };
  }
  return structureOf(schema, where, depth, binding);
}

function structureOf(schema: z.$ZodType, where: string, depth: number, binding: boolean): TypeDescription {
  refuseDepth(where, depth);
  const d = def(schema);
  const kind = d?.type;
  switch (kind) {
    case "optional":
      throw new Error(`${where}: optional inside a type (anywhere but on a prop or an object field) isn't supported`);
    case "enum": {
      refuseDetails(schema, ENUM_DEF, where, binding);
      const entries = Object.entries(d.entries as Record<string, unknown>);
      if (!entries.every(([key, value]) => typeof value === "string" && key === value)) {
        throw new Error(`${where}: an enum whose keys aren't its string values (an object map or native enum) isn't supported`);
      }
      // Object.keys order: array-index words first, a repeated word once.
      return { kind, values: entries.map(([key]) => key) };
    }
    case "array":
      refuseDetails(schema, ARRAY_DEF, where, binding);
      return { kind, items: typeOf(d.element, `${where}[]`, depth + 1) };
    case "union":
      refuseDetails(schema, UNION_DEF, where, binding);
      return { kind, options: (d.options as z.$ZodType[]).map((o, i) => typeOf(o, `${where}|${i}`, depth + 1)) };
    case "object":
      return { kind, fields: fieldsOf(schema as z.$ZodObject, where, depth + 1, binding) };
    case "record": {
      refuseDetails(schema, RECORD_DEF, where, binding);
      const key = d.keyType;
      if (def(key)?.type !== "string") throw new Error(`${where}: a record key other than z.string() isn't supported`);
      refuseDetails(key, SCALAR_DEFS.string, `${where} record key`);
      return { kind, values: typeOf(d.valueType, `${where}{}`, depth + 1) };
    }
    case "nullable":
      refuseDetails(schema, NULLABLE_DEF, where, binding);
      return { kind, type: typeOf(d.innerType, `${where}?`, depth + 1) };
  }
  const allowed = SCALAR_DEFS[kind];
  if (!Object.hasOwn(SCALAR_DEFS, kind) || allowed === undefined) throw new Error(`${where}: zod kind "${kind}" isn't supported yet`);
  refuseDetails(schema, allowed, where, binding);
  return { kind } as TypeDescription;
}

// A prop or an object field: .optional() is allowed here and only here. A
// prop (`prop`) carries a binding on its outermost schema: the .optional()
// wrapper when there is one, as upstream reads it.
function fieldOf(name: string, schema: z.$ZodType, where: string, depth: number, prop = false): PropDefinition {
  const binding = prop && isReactiveSchema(schema);
  const marks = binding ? { binding: true as const } : {};
  if (def(schema)?.type === "optional") {
    refuseDetails(schema, OPTIONAL_DEF, where, prop);
    if (schemaIdTags.has(schema)) throw new Error(`${where}: a prompt name on .optional() (which upstream never reads) isn't supported`);
    return { name, type: typeOf(def(schema).innerType, where, depth), optional: true, ...marks };
  }
  return { name, type: typeOf(schema, where, depth, prop), ...marks };
}

// The fields of a z.object (a component's props, `props`, or an inline
// object), in Object.entries order; `depth` is its fields' types' level.
// Object.entries skips symbol keys, which upstream refuses ("Symbol keys
// cannot be represented in JSON Schema"), so a shape with one is refused
// here. `binding`: an inline object that is a prop's outermost type.
function fieldsOf(object: z.$ZodObject, where: string, depth: number, binding = false, props = false): PropDefinition[] {
  const d = def(object);
  if (d?.type !== "object" || !d.shape) throw new Error(`${where}: props must be z.object(...)`);
  refuseDetails(object, OBJECT_DEF, where, binding);
  if (d.catchall !== undefined) {
    if (def(d.catchall)?.type !== "never") throw new Error(`${where}: an object with a catch-all (z.looseObject, .catchall()) isn't supported`);
    refuseDetails(d.catchall, NEVER_DEF, `${where} catch-all`);
  }
  if (Object.getOwnPropertySymbols(d.shape).length > 0) {
    throw new Error(`${where}: an object with symbol keys isn't supported (upstream can't write it as JSON Schema)`);
  }
  return Object.entries(d.shape).map(([name, s]) => fieldOf(name, s as z.$ZodType, `${where}.${name}`, depth, props));
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

// The schemas zod meets inside a schema, in the order its processors meet
// them: an object's fields (and a catch-all other than z.never()), an
// array's element, a union's options, a record's values then key, the
// schema inside .nullable() or .optional().
function children(schema: unknown): unknown[] {
  const d = def(schema);
  switch (d?.type) {
    case "object": {
      const fields = Object.keys(d.shape).map((key) => d.shape[key]);
      return d.catchall !== undefined && def(d.catchall)?.type !== "never" ? [...fields, d.catchall] : fields;
    }
    case "array":
      return [d.element];
    case "union":
      return [...d.options];
    case "record":
      return [d.valueType, d.keyType];
    case "nullable":
    case "optional":
      return [d.innerType];
    default:
      return [];
  }
}

// Replays one zod conversion from `root` (or, when it is undefined, from a
// fresh object holding `roots`) with an explicit stack, and returns the
// schemas zod writes as "__schemaN" $defs entries: those with no registered
// id met again while they are still being written (`seen.cycle`). The root
// met again is written in place, with a "#" $ref, so it is not among them.
function extracted(roots: unknown[], root: unknown, registered: (schema: unknown) => boolean): Set<unknown> {
  const found = new Set<unknown>();
  const seen = new Set<unknown>([root]);
  const writing = new Set<unknown>([root]);
  const stack: { schema: unknown; next: unknown[]; i: number }[] = [{ schema: root, next: roots, i: 0 }];
  while (stack.length > 0) {
    const top = stack[stack.length - 1];
    if (top.i >= top.next.length) {
      writing.delete(top.schema);
      stack.pop();
      continue;
    }
    const schema = top.next[top.i++];
    if (seen.has(schema)) {
      if (writing.has(schema) && schema !== root && !registered(schema)) found.add(schema);
      continue;
    }
    seen.add(schema);
    writing.add(schema);
    stack.push({ schema, next: children(schema), i: 0 });
  }
  return found;
}

const EXTRACTED =
  'a schema met again inside itself through a reference isn\'t supported (zod writes it as a "__schemaN" $defs entry, and the plain data has no object identity)';

// A call's conversion keeps only its root's body: its $defs (the registered
// schemas' and the "__schemaN" entries) are dropped. So a "__schemaN" entry
// shows only where a $ref to it is written outside them: walk what is kept,
// stopping at each $ref, each body written once.
// A registered root is written as a $ref alone.
function refuseCallExtracted(root: unknown, registered: (schema: unknown) => boolean, where: string) {
  if (registered(root)) return;
  const found = extracted(children(root), root, registered);
  if (found.size === 0) return;
  const written = new Set<unknown>([root]);
  const open = [root];
  while (open.length > 0) {
    for (const schema of children(open.pop())) {
      if (found.has(schema)) throw new Error(`${where}: ${EXTRACTED}`);
      if (registered(schema) || written.has(schema)) continue;
      written.add(schema);
      open.push(schema);
    }
  }
}

// Every conversion upstream's createLibrary makes, replayed: the library's
// schema (its record's components' props, in record order; every $defs
// entry is kept), each function's params and return type, and each action's
// params.
function refuseExtractedSchemas(input: Declared) {
  const record: Record<string, Declared["components"][number]> = {};
  for (const c of input.components) record[c.name] = c;
  const ids = new Set<unknown>();
  for (const c of input.components) if (c.name) ids.add(c.props);
  for (const [id, schema] of Object.entries(ACTION_DEFS)) if (!record[id]) ids.add(schema);
  for (const a of input.actions ?? []) if (a.name) ids.add(a.ref);
  const registered = (schema: unknown) => ids.has(schema);
  if (extracted(Object.values(record).map((c) => c.props), undefined, registered).size > 0) {
    throw new Error(`library: ${EXTRACTED}`);
  }
  for (const f of input.functions ?? []) {
    refuseCallExtracted(f.params, registered, `function ${f.name} params`);
    if (f.returns) refuseCallExtracted(f.returns, registered, `function ${f.name} returns`);
  }
  for (const a of input.actions ?? []) refuseCallExtracted(a.params, registered, `action ${a.name} params`);
}

/** The plain-data form of what `createLibrary(input)` is given. */
export function toLibraryDefinition(input: Declared): LibraryDefinition {
  refuseUnknownFields(input, LIBRARY_FIELDS, "library");
  const seen = new Set<unknown>();
  // Every component's props object, for references; the last of each name
  // is the one a reference by name means.
  const lastOfName = new Map<unknown, number>();
  input.components.forEach((c, i) => lastOfName.set(c.name, i));
  owners = new Map();
  input.components.forEach((c, i) => {
    if (typeof c.name === "string" && !owners.has(c.props)) owners.set(c.props, { name: c.name, last: lastOfName.get(c.name) === i });
  });
  actionOwners = new Map();
  for (const [i, a] of (input.actions ?? []).entries()) {
    if (typeof a?.name !== "string") throw new Error(`actions[${i}]: an action's name is a string`);
    if (actionOwners.has(a.ref)) throw new Error(`${a.name}: an action ref shared with another action isn't supported`);
    actionOwners.set(a.ref, a.name);
  }
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
    return { name: c.name, description: c.description, props: fieldsOf(c.props, where, 1, false, true) };
  });
  // A call's params object: a z.object that is no component's props.
  const paramsOf = (params: z.$ZodObject, where: string) => {
    if (owners.has(params)) throw new Error(`${where}: a params object shared with a component isn't supported`);
    return fieldsOf(params, where, 1).map(({ binding: _, ...field }) => field);
  };
  const functions = input.functions?.map((f, i) => {
    const where = typeof f?.name === "string" ? `function ${f.name}` : `functions[${i}]`;
    refuseUnknownFields(f, FUNCTION_FIELDS, where);
    if (typeof f.name !== "string" || typeof f.description !== "string") {
      throw new Error(`${where}: a function's name and description are strings`);
    }
    const params = paramsOf(f.params, where);
    const returns = f.returns === undefined ? undefined : typeOf(f.returns, `${where} returns`, 1);
    return { name: f.name, description: f.description, params, ...(returns !== undefined ? { returns } : {}) };
  });
  const actions = input.actions?.map((a) => {
    const where = `action ${a.name}`;
    refuseUnknownFields(a, ACTION_FIELDS, where);
    if (typeof a.description !== "string") throw new Error(`${where}: an action's description is a string`);
    if (a.name === "ActionExpression") {
      throw new Error(`${where}: an action named ActionExpression isn't supported (upstream registers it under the action slot's id)`);
    }
    return { name: a.name, description: a.description, params: paramsOf(a.params, where) };
  });
  refuseExtractedSchemas(input);
  if (input.root !== undefined && typeof input.root !== "string") throw new Error("library: root is a string");
  if (input.id !== undefined && typeof input.id !== "string") throw new Error("library: id is a string");
  return {
    components,
    ...(input.componentGroups !== undefined
      ? { componentGroups: input.componentGroups.map((g, i) => groupOf(g, `componentGroups[${i}]`)) }
      : {}),
    ...(functions !== undefined ? { functions } : {}),
    ...(actions !== undefined ? { actions } : {}),
    ...(input.root !== undefined ? { root: input.root } : {}),
    ...(input.id !== undefined ? { id: input.id } : {}),
  };
}
