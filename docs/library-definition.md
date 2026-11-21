# Library definitions: the plain-data form

A host can define a component library without zod or upstream's JavaScript
tools: it writes the library's declarations as JSON text in the plain-data
form below and hands it to the binding, which returns a component library
usable everywhere one supplied as JSON Schema is (parsing, streaming,
runtime sessions, prompts). The core turns the declarations into the JSON
Schema and prompt spec upstream's lang-core 1.1.0 (`createLibrary`,
`toJSONSchema()`, `toSpec()`, with zod 4.6.7) produces for the equivalent zod
declaration, byte for byte (ADR 0017). The authoritative reference is the
`openui_lang::library_definition` module documentation; this page is the
host's view of it.

Swift and Kotlin get typed wrappers later (Swift in OpenUINative); the
binding itself takes only the plain data.

## Entry points

Swift (`import OpenUIFFI`), Kotlin (`import uniffi.openui_ffi.*`, companion
object functions with `@Throws(OpenUiException::class)`) and JavaScript (the
WebAssembly package):

```swift
static func fromDefinition(definition: String) throws -> ComponentLibrary
static func fromDefinitionWithLimits(definition: String, limits: RuntimeLimits) throws -> ComponentLibrary
func jsonSchema() -> String?
func promptSpec() -> String?
```

```kotlin
fun fromDefinition(definition: String): ComponentLibrary
fun fromDefinitionWithLimits(definition: String, limits: RuntimeLimits): ComponentLibrary
fun jsonSchema(): String?
fun promptSpec(): String?
```

```typescript
ComponentLibrary.fromDefinition(definition: string): ComponentLibrary
ComponentLibrary.fromDefinitionWithLimits(definition: string, limits: RuntimeLimits): ComponentLibrary
library.jsonSchema(): string | undefined
library.promptSpec(): string | undefined
```

`jsonSchema()` and `promptSpec()` give a defined library's JSON Schema and
prompt spec as JSON text, exactly the bytes upstream's
`JSON.stringify(library.toJSONSchema())` and `JSON.stringify(library.toSpec())`
write. They are absent for a library made from a JSON Schema. Read the output
through them: the core keeps `$defs` in the order zod meets them and puts
names that look like array indices ("7", "12") first only when it writes the
text, as JavaScript does, so text re-serialized by another JSON writer may
differ from upstream's.

Using a defined library:

- **Parsing and streaming:** pass it to `parse`, `parseProgram` or a stream
  session like any other library. A program parses exactly as with the same
  library supplied as upstream's JSON Schema.
- **Runtime sessions:** pass the library, and `promptSpec()` as the session's
  prompt spec, so the session knows which props are bindings. A session given
  no prompt spec does not read bindings from the definition.
- **Prompts:** `generateSystemPrompt` (native) takes
  `{"library": <promptSpec()>, "promptOptions": ...}`; in JavaScript, hand
  the parsed `promptSpec()` to upstream's prompt tooling.
- **Functions:** a function's implementation never enters the core. Register
  it with the session's `registerFunction` under the function's name.

## Limits and errors

A definition is host input. `fromDefinition` holds it to the default
`RuntimeLimits.maxJsonBytes` (8 MiB) and `fromDefinitionWithLimits` to the
one given, the same limit a session puts on other host text. The length is
counted in UTF-8 bytes (a JavaScript string's bytes as it crosses, not its
UTF-16 length), before the text is read: one byte over is refused with
`LimitExceeded` and the message `host input byte limit`, and a definition of
exactly the limit is accepted. It is then read as JSON under
`limits.evaluation`, whose `depth` (256 by default) is also how deep types
may nest, a prop's own type being the first level. The other `RuntimeLimits`
fields are accepted and ignored by `fromDefinitionWithLimits`: only
`maxJsonBytes` and `evaluation.depth` apply to a definition.

Every other refusal is `InvalidJson`, with the core's message. That
includes a definition that is well-formed JSON but is refused (not in the
form, an unsupported kind, a root that is not a component, a colliding
name), not only malformed JSON: the kind does not tell them apart, the
message does, and it is upstream's own text where upstream has one. A
dedicated kind for a refused definition is tracked on #146.

| Refusal | Message |
|---|---|
| Malformed JSON | the JSON reader's, such as `unexpected end of JSON at byte 16` |
| Not in the form (a missing or unknown field, a wrong type, a duplicate prop or field name) | `library definition: components[0].props[1].name: duplicate prop "a"` |
| An unsupported kind | `library definition: components[0].props[0].type: type kind "date" is not supported` |
| Types nested past the depth limit | `library definition: <path>: types nest deeper than 256 levels` |
| A root that is not a component | upstream's: `[createLibrary] Root component "B" was not found in components. Available components: A` |
| A function or action name that collides | upstream's: `[createLibrary] Function "A" collides with a built-in, component, function, or action of the same name. Rename it.` |
| A reference to a name that is no component | `library definition: <path>: "X" is not a component of the library` |
| An action slot naming an action it can't name | `library definition: <path>: "X" is not an action a slot can name (the built-in ones are OpenUrl and ToAssistant, and the library's own actions)` |

### What the byte limit does not bound

The parser's library copies each library action's shape into every param
that refers to it, as upstream's `inlineActionRefs` does, with each copy's
own references copied in turn. So what a definition costs to compile, hold
and free is not bounded by its size. Measured on 2026-11-22 (Apple silicon,
release build, through `fromDefinition`):

- **A chain of actions**, each one's param an action slot for the next:
  memory and time are quadratic in its length, and the copies nest about four
  JSON levels per action. 1,000 actions (96 KB of definition) take 1.2 GB and
  1 s; 2,000 (194 KB) take 4.7 GB and 4.6 s. The longest chain within 8 MiB,
  about 86,000 actions, would need terabytes.
- **A doubling chain**, each action with two params naming the next: memory
  and time double with each action. 18 actions (2.6 KB) take 2 GB and 1 s.
- **Many references to one large action**: 2,000 params referring to one
  action with 2,000 params (188 KB) take 1.7 GB and 0.7 s; quadratic.
- **Freeing the library** recurses as deep as the copies nest. Natively it
  overflows a 64 KiB thread between 150 and 175 chained actions and a
  512 KiB thread between 1,000 and 1,250 (about 400 bytes of stack per
  action); a session's worker thread (2 MiB) dropped a 2,000-action library
  without trouble, and would overflow at about 5,000, past what memory
  allows. In WebAssembly the free runs on the JavaScript engine's stack and
  throws `Worker` (Maximum call stack size exceeded) between 600 and 1,000
  actions in Node 22 and between 1,750 and 2,000 in Bun 1.4; making 2,600
  exhausts WebAssembly's 4 GiB.
- **Components referring to each other** are not copied: a chain of 71,000
  components (8.4 MB, just under the limit) takes 0.6 s and 415 MB, and
  parses and frees on a 64 KiB thread.

The same holds for a library supplied as JSON Schema, which has no size limit
at all (#140 tracks both). Until then, a definition is trusted input from the
app's own author, and a host that accepts library definitions from anyone
else should not. The expensive shapes are library actions that refer to each
other: a chain of actions, each naming the next, costs memory and time
quadratic in its length, and an action that names the next one twice
doubles the cost with each action (#140).

#### After a failure in WebAssembly

- A `Worker` error with the message "unreachable" (memory ran out while
  making a library) leaves the WebAssembly instance unusable for every
  later call: the host must load a new instance.
- After a `Worker` error "Maximum call stack size exceeded" the instance
  keeps working, but the library whose free was interrupted is leaked, and
  the `RuntimeLimits` object passed to the failed call can no longer be used.

## The form

```json
{
  "components": [
    {
      "name": "Card",
      "description": "A card.",
      "props": [
        {"name": "title", "type": {"kind": "string"}},
        {"name": "count", "type": {"kind": "number"}, "optional": true},
        {"name": "text", "type": {"kind": "string"}, "binding": true}
      ]
    },
    {"name": "Item", "description": "One item.", "props": [
      {"name": "label", "type": {"kind": "string"}}
    ]}
  ],
  "componentGroups": [
    {"name": "Content", "components": ["Card", "Item"], "notes": ["Cards hold items."]}
  ],
  "functions": [
    {"name": "Percent", "description": "Percent of a total.",
     "params": [{"name": "part", "type": {"kind": "number"}},
                {"name": "total", "type": {"kind": "number"}}],
     "returns": {"kind": "number"}}
  ],
  "actions": [
    {"name": "Copy", "description": "Copies text.",
     "params": [{"name": "text", "type": {"kind": "string"}}]}
  ],
  "root": "Card",
  "id": "example"
}
```

- `components` (required): in order. A component has a `name`, a
  `description` (may be empty) and `props`, an ordered array, never a map:
  the order is the order of the component's positional arguments. As in a
  JavaScript object, names that are array indices ("0", "12") come first,
  ascending.
- A prop has a `name`, a `type`, `optional` (default `false`) and `binding`
  (default `false`). An inline object's field has a `name`, a `type` and
  `optional`. Each name appears once per component or object.
- `componentGroups` (optional): copied into the prompt spec. A group has a
  `name`, `components` (names) and optional `notes`, and no other field.
- `functions` (optional): upstream's `defineFunction` without the
  implementation: a `name`, a `description`, `params` (fields, as an inline
  object's) and an optional `returns` type. Its signature is
  `Percent(part: number, total: number) → number`.
- `actions` (optional): upstream's `defineAction`: a `name`, a
  `description` and `params`. Its signature is `Copy(text: string)`; a
  program calls it as an action step, `@Copy("t")`.
- `root` (optional): must name a component. `id` (optional): copied into the
  prompt spec.

A field the form doesn't have is refused, so a misspelt `"optinal"` is never
silently ignored.

## Every supported kind

Each row is a prop's `type`, what zod declaration it stands for, the JSON
Schema written for it and how the signature prints it (for a prop named
`p`). Examples are taken from the core's output.

| Kind | Example | zod | JSON Schema | Signature |
|---|---|---|---|---|
| string | `{"kind": "string"}` | `z.string()` | `{"type":"string"}` | `p: string` |
| number | `{"kind": "number"}` | `z.number()` | `{"type":"number"}` | `p: number` |
| boolean | `{"kind": "boolean"}` | `z.boolean()` | `{"type":"boolean"}` | `p: boolean` |
| any | `{"kind": "any"}` | `z.any()` | `{}` | `p: any` |
| list of words | `{"kind": "enum", "values": ["sm", "lg"]}` | `z.enum(["sm", "lg"])` | `{"type":"string","enum":["sm","lg"]}` | `p: "sm" \| "lg"` |
| array | `{"kind": "array", "items": {"kind": "string"}}` | `z.array(z.string())` | `{"type":"array","items":{"type":"string"}}` | `p: string[]` |
| union | `{"kind": "union", "options": [{"kind": "string"}, {"kind": "number"}]}` | `z.union([...])` | `{"type":["string","number"]}` (an `anyOf` unless every option is a bare type) | `p: string \| number` |
| inline object | `{"kind": "object", "fields": [{"name": "x", "type": {"kind": "number"}}, {"name": "y", "type": {"kind": "number"}, "optional": true}]}` | `z.object({x, y: ....optional()})` | `{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x"],"additionalProperties":false}` | `p: {x: number, y?: number}` |
| record | `{"kind": "record", "values": {"kind": "string"}}` | `z.record(z.string(), z.string())` | `{"type":"object","propertyNames":{"type":"string"},"additionalProperties":{"type":"string"}}` | `p: Record<string, string>` |
| optional | `"optional": true` on the prop or field | `.optional()` | left out of `required` | `p?: ...` |
| nullable | `{"kind": "nullable", "type": {"kind": "string"}}` | `.nullable()` | `{"type":["string","null"]}`, still in `required` | `p?: string` |
| component reference | `{"kind": "array", "items": {"kind": "component", "name": "Item"}}` | `z.array(Item.ref)` | `{"type":"array","items":{"$ref":"#/$defs/Item"}}` | `p: Item[]` |
| action slot | `{"kind": "action"}` | `action()` | `{"$ref":"#/$defs/ActionExpression"}` | `p: ActionExpression` |
| one built-in action | `{"kind": "action", "name": "OpenUrl"}` (or `ToAssistant`) | `steps.OpenUrl.ref` | `{"$ref":"#/$defs/OpenUrl"}` | `p: @OpenUrl` |
| one library action | `{"kind": "action", "name": "Copy"}` | `copy.ref` | `{"$ref":"#/$defs/Copy"}`, whose entry is `{"type":"object","properties":{"type":{"type":"string","const":"Copy"},"params":...},"required":["type","params"],"additionalProperties":false}` | `p: @Copy` |
| binding | `"binding": true` on the prop | `reactive(...)` | unchanged | `p: $binding<string>` |
| named type | `{"kind": "named", "name": "Color", "type": {"kind": "string"}}` | `tagSchemaId(z.string(), "Color")` | the inner type's: `{"type":"string"}` | `p: Color` |

Optional and nullable are separate facts: a prop may be both (zod's
`.nullable().optional()`). A referenced component or action gets its `$defs`
entry where zod first meets it, walking the components in order and each
one's props depth first; a built-in action's entry is not written when a
component has its name (the shape is then written inline).

## What is refused or cannot be written

- Any kind not in the table, refused with the kind's name. A new kind is
  added when a real library needs it.
- **No object identity.** A type used in two places is two equal types. A
  zod declaration where upstream writes one shared object as a `__schemaN`
  `$defs` entry (an object met again inside itself through a component or
  action reference) cannot be written in plain data.
- **A binding marks only a prop's outermost type.** `binding` is a prop
  field; a field of an object, an array's items and a function's or action's
  params and return type have none (upstream prints `$binding<...>` there but
  never binds it). Conversely the core can bind one use of a component
  reference or action slot while another use stays unbound, which no zod
  declaration can say (zod marks the shared schema); this is harmless.
- **An optional return type** has nowhere to go: write the inner type, which
  upstream writes identically.
- **A reference to the component, or the library action, with the empty
  name** is refused (upstream writes those inline, or never finishes when
  one refers to itself).
- **A library action named `ActionExpression`** is refused (upstream
  registers it under the action slot's own id and zod throws when a library
  uses both).
- **A named type around a component reference, an action slot or another
  named type** is refused.
- A root that names no component, and a function or action whose name is a
  built-in's, a reserved call's (`Query`, `Mutation`, or a name every
  JavaScript object inherits), a component's or an earlier function's or
  action's, are refused with upstream's messages.
- Upstream's `library.extend` is not ported.
