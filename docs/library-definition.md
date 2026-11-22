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

## Define and draw a component in Swift

`OpenUINative` pairs each component declaration with a SwiftUI view. The
view receives the evaluated element and its path. Props are positional in
declaration order. For example, this program draws “Ready”.

```swift
import OpenUINative
import SwiftUI

@MainActor func badgeView() async throws -> ElementView {
    let badge = defineComponent(name: "Badge", description: "A short label.",
                                props: [.init("label", .string)]) { element, _ in
        if case let .scalar(.string(label))? = element.props.first?.value {
            Text(label)
        }
    }
    let library = try await Core().createLibrary(components: [badge], root: badge.name)
    let live = try LiveProgram(library: library)
    try await live.show(response: #"root = Badge("Ready")"#, streaming: false)
    return ElementView(live: live)
}
```

Use the optional `groups` argument of `createLibrary` to supply an ordered
array of `ComponentGroup` values for the prompt. The declared root also supplies the
parser's default root component. An explicit root argument overrides it.
`try await library.promptSpec()` returns the core's generated spec.

An app's `ElementDrawing.component` can override a declared view. A
declaration without a view uses the existing fallback, including when it
shares a built-in component's name. Use `ElementView(element: child)` in
a defined view to draw children in the same live program.

Kotlin authors can supply the plain data through the binding.

## Entry points

Swift (`import OpenUIFFI`), Kotlin (`import uniffi.openui_ffi.*`, companion
object functions with `@Throws(OpenUiException::class)`) and JavaScript (the
WebAssembly package):

```swift
static func fromDefinition(definition: String) throws -> ComponentLibrary
static func fromDefinitionWithLimits(definition: String, limits: RuntimeLimits) throws -> ComponentLibrary
func jsonSchema() -> String?
func promptSpec() -> String?
func prompt(options: String?) throws -> String?
func cloudConfig(options: String?, instructions: String?) throws -> String?
```

```kotlin
fun fromDefinition(definition: String): ComponentLibrary
fun fromDefinitionWithLimits(definition: String, limits: RuntimeLimits): ComponentLibrary
fun jsonSchema(): String?
fun promptSpec(): String?
fun prompt(options: String?): String?
fun cloudConfig(options: String?, instructions: String?): String?
```

```typescript
ComponentLibrary.fromDefinition(definition: string): ComponentLibrary
ComponentLibrary.fromDefinitionWithLimits(definition: string, limits: RuntimeLimits): ComponentLibrary
library.jsonSchema(): string | undefined
library.promptSpec(): string | undefined
library.prompt(options?: string): string | undefined
library.cloudConfig(options?: string, instructions?: string): string | undefined
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
- **Runtime sessions:** pass the library; no prompt spec is needed. A
  session over a defined library given no prompt spec takes its binding
  props (the `"binding": true` props) and its root from the definition, so an
  input bound to one writes state. A prompt spec, when given, decides both,
  as for any library.
- **Prompts:** `prompt(options)` is the system prompt, byte for byte as
  upstream's `library.prompt(options)`; `options` is upstream's prompt
  options as JSON text, such as `{"bindings": true}`, `{"tools": [...]}` or
  `{"preamble": "...", "additionalRules": [...]}`, and may be left out.
  `cloudConfig(options, instructions)` is the OpenUI Cloud config block, byte
  for byte as upstream's `generateSystemPrompt({cloud: true, library:
  library.toSpec(), promptOptions, instructions})`: only `examples`,
  `preamble` and `additionalRules` of the options are sent, and non-empty
  instructions follow the block. Both are absent for a library made from a
  JSON Schema. The native `generateSystemPrompt` with
  `{"library": <promptSpec()>, "promptOptions": ...}` gives the same prompt.
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
`maxJsonBytes` and `evaluation.depth` apply to a definition. A library
supplied as JSON Schema is held to the same limit, in the same way, by the
plain constructor (the default) and `fromSchemaWithLimits` (#140).

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

### What a library costs past the byte limit

A library's cost grows in step with its size, through a definition and
through a ready-made JSON Schema alike (#140). Upstream's `inlineActionRefs`
copies each action's shape into every param that refers to it, with each
copy's own references copied in turn; the compiled library holds each shape
once instead and shares it, so library actions that refer to each other
cost no more than any other declaration. Validation reads a param's schema
as upstream's copy has it, so nothing it reports changes. Making, using and
freeing a library recurse nowhere without a bound: the drops and copies use
loops, so they work on a 64 KiB thread, unoptimised too.

Measured on 2026-11-22 (Apple silicon), made, parsed and freed through
`fromDefinition` / through the same library's JSON Schema; before #140 in
brackets:

| Shape | Native, test build: peak heap, time | WebAssembly (Bun 1.4): time |
|---|---|---|
| A chain of 2,000 actions, each one's param an action slot for the next (194 KB) | 32 MB / 20 MB, 0.16 s / 0.02 s (4.7 GB and 4.6 s) | 29 ms / 10 ms (freeing overflowed Bun's stack past 1,750 actions) |
| The same chain of 4,000 actions | 37 MB / 13 MB in the core alone, 0.13 s / 0.02 s | 54 ms / 19 ms |
| 30 actions, each naming the next twice (4.4 KB) | 0.5 MB / 0.4 MB, 5 ms / 0.5 ms (18 actions took 2 GB; 30 would need terabytes) | 0.8 ms / 0.3 ms |
| 2,000 params referring to one action with 2,000 params (256 KB) | 11 MB / 8 MB, 0.24 s / 0.02 s (1.7 GB and 0.7 s for 188 KB) | 22 ms / 12 ms |
| A chain of 71,000 components referring to each other (8.4 MB, just under the limit) | unchanged: components were never copied (0.6 s and 415 MB before) | |

Reading needs nothing of the caller's stack either: a definition is read
with the core's reader, which grows its stack, and a JSON Schema with
serde_json's, run on a stack segment of its own sized for serde_json's
127-level limit, so a schema nested to that limit (or refused one level past
it, with the message serde_json gives) reads on a 64 KiB thread, unoptimised
too, and in WebAssembly. A definition, like a schema, is trusted input from
the app's own author.

#### After a failure in WebAssembly

None of the shapes above runs a WebAssembly instance out of memory or the
engine out of stack any more. Should a call still run out:

- A `Worker` error with the message "unreachable" (memory ran out while
  making a library) leaves the WebAssembly instance unusable for every
  later call: the host must load a new instance.
- After a `Worker` error "Maximum call stack size exceeded" the instance
  keeps working, but the library whose call was interrupted is leaked, and
  the `RuntimeLimits` object passed to the failed call can no longer be used.

### Limits and errors of prompts

A prompt's options and a cloud config's instructions are host input, held to
the limits the library was made with (the defaults for `fromDefinition`):
longer than `maxJsonBytes` in UTF-8 bytes is `LimitExceeded` (`host input
byte limit`); options that are not JSON, or nest past `evaluation.depth`, are
`InvalidPromptSpec`; and a prompt the core refuses, as upstream's
`generatePrompt` and `generateSystemPrompt` refuse it, is `Prompt` with
upstream's message: options that set `components` to null, a request nesting
past the prompt's depth limit, and a cloud config for a library Cloud does
not accept (no component, or a root its JSON Schema has no entry for, such as
a root that only passes `createLibrary`'s check because every object has it).

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
