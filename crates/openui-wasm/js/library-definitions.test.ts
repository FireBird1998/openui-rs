// A library definition through the WebAssembly package (#137): the
// plain-data form goes in as JSON text and the component library comes out,
// with its JSON Schema and prompt spec as text byte for byte as upstream
// writes them, usable everywhere a library supplied as JSON Schema is. The
// samples are conformance/golden/library-definitions.json, written by the
// reference harness running upstream's code.
import { test, expect } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import golden from '../../../conformance/golden/library-definitions.json';
import promptGolden from '../../../conformance/golden/library-definition-prompts.json';
import { wasm, kind, loadWith } from './support';

type Sample = { name: string; definition: any; schema?: any; spec?: any; error?: string };
const samples = golden as Sample[];
const sample = (name: string) => {
  const found = samples.find(s => s.name === name);
  if (!found) throw new Error(`no sample ${name}`);
  return found;
};
const define = (definition: unknown) => wasm.ComponentLibrary.fromDefinition(JSON.stringify(definition));
const fromUpstream = (s: Sample) => new wasm.ComponentLibrary(JSON.stringify(s.schema));

test.each(['fromDefinition', 'fromDefinitionWithLimits'] as const)(
  '%s checks the host input byte limit before copying the definition', method => {
    // Use a fresh instance so previous allocations cannot hide memory growth.
    const meter = { bytes: () => 0, copies: 0 };
    const pkg = loadWith(`
      host.bytes = () => wasm.memory.buffer.byteLength;
      const passString = passStringToWasm0;
      passStringToWasm0 = (text, malloc, realloc) => {
        host.copies++;
        return passString(text, malloc, realloc);
      };`, meter);
    using limits = new pkg.RuntimeLimits();
    if (method === 'fromDefinitionWithLimits') limits.maxJsonBytes = 1000n;
    const limit = Number(limits.maxJsonBytes);
    const make = (text: string) => method === 'fromDefinition'
      ? pkg.ComponentLibrary.fromDefinition(text)
      : pkg.ComponentLibrary.fromDefinitionWithLimits(text, limits);
    const definition = '{"components":[{"name":"C","description":"é😀","props":[]}],"root":"C"}';
    const bytes = new TextEncoder().encode(definition).length;

    const before = meter.bytes();
    const oversized = definition.padEnd(16 * 1024 * 1024);
    expect(kind(() => make(oversized), 'LimitExceeded').message).toBe('host input byte limit');
    expect(meter.bytes() - before).toBe(0);
    expect(meter.copies).toBe(0);

    const atLimit = definition.padEnd(definition.length + limit - bytes);
    const oneByteOver = atLimit + ' ';
    expect(oneByteOver.length).toBeLessThan(limit);
    expect(kind(() => make(oneByteOver), 'LimitExceeded').message).toBe('host input byte limit');
    expect(meter.copies).toBe(0);

    using library = make(atLimit);
    expect(library.jsonSchema()).toContain('"C"');
  },
);

test('every sample gives upstream\'s schema and spec text, or its error as InvalidJson', () => {
  expect(samples.length).toBe(99);
  let errors = 0, libraries = 0;
  for (const s of samples) {
    if (s.error !== undefined) {
      const thrown = kind(() => define(s.definition), 'InvalidJson');
      expect(thrown.message).toBe(s.error);
      errors++;
      continue;
    }
    libraries++;
    const library = define(s.definition);
    // JSON.stringify of the parsed golden is upstream's own text.
    expect(library.jsonSchema()).toBe(JSON.stringify(s.schema));
    expect(library.promptSpec()).toBe(JSON.stringify(s.spec));
    library.free();
  }
  expect([errors, libraries]).toEqual([23, 76]);
});

test('integer-like component names come first, as JavaScript writes them', () => {
  const library = define(sample('key-order').definition);
  expect(library.jsonSchema()).toContain('"properties":{"7":{"$ref":"#/$defs/7"},"Zeta"');
  expect(library.jsonSchema()).toContain('"$defs":{"7":{');
  expect(library.promptSpec()!.startsWith('{"root":"7","components":{"7":')).toBe(true);
  library.free();
});

test('a library from a JSON Schema has no definition texts', () => {
  const library = fromUpstream(sample('scalars'));
  expect(library.jsonSchema()).toBeUndefined();
  expect(library.promptSpec()).toBeUndefined();
  library.free();
});

const corpusDir = new URL('../../../conformance/cases/library-definition/', import.meta.url);
const corpus = readdirSync(corpusDir).filter(f => f.endsWith('.oui')).map(f => readFileSync(new URL(f, corpusDir), 'utf8'));

test('a defined library parses, one shot and streamed, as the same library supplied as JSON', () => {
  expect(corpus.length).toBe(3);
  for (const s of samples.filter(s => s.schema !== undefined)) {
    const defined = define(s.definition), upstream = fromUpstream(s);
    const root = typeof s.definition.root === 'string' ? s.definition.root : undefined;
    const names = s.definition.components.map((c: any) => c.name);
    const programs = [...corpus, ...names.flatMap((n: string) =>
      [`root = ${n}()`, `x = Action([@ToAssistant("hi")])\nroot = ${n}("a", 1, true, [x], {k: null})`])];
    for (const program of programs) {
      expect(wasm.parse(program, defined, root)).toBe(wasm.parse(program, upstream, root));
      const a = new wasm.StreamSession(defined, root), b = new wasm.StreamSession(upstream, root);
      for (const c of program) expect(a.push(c)).toBe(b.push(c));
      expect(a.finish()).toBe(b.finish());
      a.free(); b.free();
    }
    defined.free(); upstream.free();
  }
});

test('a session over a defined library draws as one over the JSON, given the spec text', () => {
  const cases: [string, string][] = [
    ['binding', '$v = "a"\n$n = null\nroot = Input($v, $v, $n, ["x"], "a")'],
    ['functions-in-a-program', 'root = Box(@Percent(1, 4), @Label("x"), @Unimplemented(2))'],
    ['custom-action', 'root = Root(Action([@Copy("t")]))'],
  ];
  for (const [name, program] of cases) {
    const s = sample(name), defined = define(s.definition), upstream = fromUpstream(s);
    const draws = [[defined, defined.promptSpec()], [upstream, JSON.stringify(s.spec)]].map(([library, spec]) => {
      const session = new wasm.RuntimeSession(library, spec);
      const parsed = wasm.parseProgram(program, library);
      session.setProgram(parsed); parsed.free();
      const draw = session.evaluate();
      const result = [draw.json, session.undecidedBindings(), session.snapshot()];
      draw.free(); session.free();
      return JSON.stringify(result);
    });
    expect(draws[0]).toBe(draws[1]);
    defined.free(); upstream.free();
  }
});

// A definition `bytes` long in UTF-8: a description of `é` (two bytes, one
// UTF-16 unit), then `a` to make up an odd length.
function definitionOf(bytes: number) {
  const head = '{"components":[{"name":"A","description":"', tail = '","props":[]}]}';
  const room = bytes - head.length - tail.length;
  const text = head + 'é'.repeat(Math.floor(room / 2)) + 'a'.repeat(room % 2) + tail;
  expect(new TextEncoder().encode(text).length).toBe(bytes);
  return text;
}

test('the definition is held to the host input limit in bytes, before it is read', () => {
  const limit = 8 * 1024 * 1024;
  expect(new wasm.RuntimeLimits().maxJsonBytes).toBe(BigInt(limit));
  wasm.ComponentLibrary.fromDefinition(definitionOf(limit)).free();
  const over = definitionOf(limit + 1);
  expect(over.length).toBeLessThan(limit);
  expect(kind(() => wasm.ComponentLibrary.fromDefinition(over), 'LimitExceeded').message).toBe('host input byte limit');
  expect(kind(() => wasm.ComponentLibrary.fromDefinition('{'.repeat(limit + 1)), 'LimitExceeded').message).toBe('host input byte limit');

  const limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 1000n;
  wasm.ComponentLibrary.fromDefinitionWithLimits(definitionOf(1000), limits).free();
  expect(kind(() => wasm.ComponentLibrary.fromDefinitionWithLimits(definitionOf(1001), limits), 'LimitExceeded').message)
    .toBe('host input byte limit');
  expect(kind(() => wasm.ComponentLibrary.fromDefinitionWithLimits('['.repeat(1001), limits), 'LimitExceeded').message)
    .toBe('host input byte limit');
  kind(() => wasm.ComponentLibrary.fromDefinitionWithLimits('['.repeat(1000), limits), 'InvalidJson');
  limits.free();
});

test('refusals reach the host as InvalidJson with the core\'s message', () => {
  kind(() => wasm.ComponentLibrary.fromDefinition('{"components": ['), 'InvalidJson');
  expect(kind(() => wasm.ComponentLibrary.fromDefinition('[]'), 'InvalidJson').message).toBe('library definition: expected an object');
  expect(kind(() => define({ components: [{ name: 'A', description: '', props: [] }], root: 'B' }), 'InvalidJson').message)
    .toBe('[createLibrary] Root component "B" was not found in components. Available components: A');
  expect(kind(() => define({ components: [{ name: 'A', description: '', props: [] }],
    functions: [{ name: 'A', description: '', params: [] }] }), 'InvalidJson').message)
    .toBe('[createLibrary] Function "A" collides with a built-in, component, function, or action of the same name. Rename it.');
  expect(kind(() => define({ components: [{ name: 'A', description: '', props: [{ name: 'when', type: { kind: 'date' } }] }] }), 'InvalidJson').message)
    .toBe('library definition: components[0].props[0].type: type kind "date" is not supported');
  // The arguments are checked by the package guard before Rust sees them.
  kind(() => wasm.ComponentLibrary.fromDefinition(42), 'InvalidJson');
  kind(() => wasm.ComponentLibrary.fromDefinitionWithLimits('{}', {}), 'InvalidHandle');
});

test('a freed defined library refuses its accessors', () => {
  const library = define(sample('scalars').definition);
  library.free();
  expect(kind(() => library.jsonSchema(), 'Disposed').message).toBe('object freed');
  expect(kind(() => library.promptSpec(), 'Disposed').message).toBe('object freed');
  expect(kind(() => library.prompt(), 'Disposed').message).toBe('object freed');
  expect(kind(() => library.cloudConfig(), 'Disposed').message).toBe('object freed');
});

// #135: upstream's library.prompt(options) and generateSystemPrompt({cloud:
// true, library: library.toSpec(), promptOptions, instructions}).
type PromptEntry = { name: string; options?: unknown; prompt: string };
type CloudEntry = { name: string; promptOptions?: unknown; instructions?: string; config?: string; error?: string };
const { prompts, cloud } = promptGolden as { prompts: PromptEntry[]; cloud: CloudEntry[] };

test('a defined library\'s prompt and cloud config are upstream\'s, byte for byte', () => {
  expect(prompts.length).toBe(131);
  for (const e of prompts) {
    const library = define(sample(e.name).definition);
    expect(library.prompt(e.options === undefined ? undefined : JSON.stringify(e.options))).toBe(e.prompt);
    library.free();
  }
  expect(cloud.length).toBe(88);
  for (const e of cloud) {
    const library = define(sample(e.name).definition);
    const options = e.promptOptions === undefined ? undefined : JSON.stringify(e.promptOptions);
    if (e.error === undefined) expect(library.cloudConfig(options, e.instructions)).toBe(e.config!);
    else expect(kind(() => library.cloudConfig(options, e.instructions), 'Prompt').message).toBe(e.error);
    library.free();
  }
});

test('prompt options are host input, held to the library\'s limits', () => {
  const limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 1000n;
  const small = { components: [{ name: 'A', description: '', props: [] }] };
  const library = wasm.ComponentLibrary.fromDefinitionWithLimits(JSON.stringify(small), limits);
  limits.free();
  const at = JSON.stringify({ preamble: 'é'.repeat((1000 - 15) / 2) + 'a' });
  expect(new TextEncoder().encode(at).length).toBe(1000);
  expect(library.prompt(at)!.startsWith('é')).toBe(true);
  const over = at.replace('a"', 'aa"');
  expect(kind(() => library.prompt(over), 'LimitExceeded').message).toBe('host input byte limit');
  expect(kind(() => library.cloudConfig(undefined, 'i'.repeat(1001)), 'LimitExceeded').message).toBe('host input byte limit');
  kind(() => library.prompt('{'), 'InvalidPromptSpec');
  expect(kind(() => library.prompt('{"components":null}'), 'Prompt').message).toBe('[generatePrompt] components is missing.');
  // Strings only: the guard refuses anything else before Rust sees it.
  kind(() => library.prompt({ bindings: true }), 'InvalidJson');
  library.free();
  const plain = fromUpstream(sample('scalars'));
  expect(plain.prompt()).toBeUndefined();
  expect(plain.cloudConfig()).toBeUndefined();
  plain.free();
});

test('a session over a defined library knows its bindings with no prompt spec (#135)', () => {
  const definition = { components: [{ name: 'Input', description: '', props: [
    { name: 'label', type: { kind: 'string' } }, { name: 'value', type: { kind: 'string' }, binding: true }] }], root: 'Input' };
  const marker = '{"__reactive":"assign","target":"$v","expr":{"k":"StateRef","n":"$value"}}';
  const defined = define(definition), plain = new wasm.ComponentLibrary(defined.jsonSchema()!);
  const run = (library: any, spec: string | undefined) => {
    const session = new wasm.RuntimeSession(library, spec);
    const parsed = wasm.parseProgram('$v = "a"\nroot = Input($v, $v)', library);
    session.setProgram(parsed); parsed.free();
    const draw = session.evaluate();
    const json = draw.json; draw.free();
    const bound = json.includes(marker);
    if (bound) expect(session.writeBinding(marker, '"typed"')).toBe(true);
    const snapshot = session.snapshot();
    const result = JSON.stringify([json, session.undecidedBindings(), snapshot]);
    session.free();
    return { bound, result, snapshot };
  };
  const withoutSpec = run(defined, undefined), withSpec = run(defined, defined.promptSpec());
  expect(withoutSpec.bound).toBe(true);
  expect(withoutSpec).toEqual(withSpec);
  expect(withoutSpec.snapshot).toBe('{"$v":"typed"}');
  expect(run(plain, undefined).bound).toBe(false);
  // A prompt spec, when given, still decides: one marking no binding.
  const unmarked = define({ ...definition, components: [{ ...definition.components[0], props: [
    definition.components[0].props[0], { name: 'value', type: { kind: 'string' } }] }] });
  expect(run(defined, unmarked.promptSpec()).bound).toBe(false);
  unmarked.free();
  defined.free(); plain.free();
});

// A chain of library actions, each one's param an action slot for the next.
// Upstream copies each action's shape into every param that refers to it;
// the compiled library shares the shapes, so this costs time and memory in
// step with its length (#140).
function actionChain(n: number) {
  const actions = [];
  for (let i = 0; i < n; i++) {
    actions.push({ name: `A${i}`, description: '', params: [
      { name: 'p', type: i + 1 < n ? { kind: 'action', name: `A${i + 1}` } : { kind: 'string' } }] });
  }
  return JSON.stringify({ components: [{ name: 'C', description: '', props: [
    { name: 'go', type: { kind: 'action', name: 'A0' } }] }], actions, root: 'C' });
}

function packageWithMemory() {
  const meter = {bytes: () => 0};
  const pkg = loadWith('host.bytes = () => wasm.memory.buffer.byteLength;', meter);
  return {pkg, bytes: meter.bytes};
}

// Before #140 shared the shapes, freeing the library recursed on the
// JavaScript engine's stack and overflowed it between 1,750 and 2,000
// actions in Bun 1.4, and making 2,600 exhausted WebAssembly's 4 GiB; the
// shapes measured then are below.
test('a long action chain of a handled size is made, used and freed', () => {
  const {pkg, bytes} = packageWithMemory();
  const before = bytes();
  const library = pkg.ComponentLibrary.fromDefinition(actionChain(100));
  const program = 'root = C(Action([@A0(Action([@A1("x")]))]))';
  const stream = new pkg.StreamSession(library);
  for (const c of program) stream.push(c);
  expect(stream.finish()).toBe(pkg.parse(program, library));
  stream.free();
  const session = new pkg.RuntimeSession(library);
  const parsed = pkg.parseProgram(program, library);
  session.setProgram(parsed); parsed.free();
  const draw = session.evaluate();
  expect(draw.json).toContain('"name":"A0"');
  draw.free(); session.free(); library.free();
  expect(bytes() - before).toBeLessThan(32 * 1024 * 1024);
});

// A library's JSON Schema `bytes` long in UTF-8: a description of `é` (two
// bytes, one UTF-16 unit), then `a` to make up an odd length.
function schemaOf(bytes: number) {
  const head = '{"$defs":{"A":{"properties":{"x":{}},"description":"', tail = '"}}}';
  const room = bytes - head.length - tail.length;
  const text = head + 'é'.repeat(Math.floor(room / 2)) + 'a'.repeat(room % 2) + tail;
  expect(new TextEncoder().encode(text).length).toBe(bytes);
  return text;
}

test('a library\'s JSON Schema is held to the host input limit in bytes, before it is read (#140)', () => {
  const limit = 8 * 1024 * 1024;
  new wasm.ComponentLibrary(schemaOf(limit)).free();
  const over = schemaOf(limit + 1);
  expect(over.length).toBeLessThan(limit);
  expect(kind(() => new wasm.ComponentLibrary(over), 'LimitExceeded').message).toBe('host input byte limit');
  expect(kind(() => new wasm.ComponentLibrary('{'.repeat(limit + 1)), 'LimitExceeded').message).toBe('host input byte limit');
  // Malformed text within the limit throws as it always did.
  expect(kind(() => new wasm.ComponentLibrary('{'), 'InvalidSchema').message)
    .toBe('EOF while parsing an object at line 1 column 1');

  const limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 1000n;
  wasm.ComponentLibrary.fromSchemaWithLimits(schemaOf(1000), limits).free();
  expect(kind(() => wasm.ComponentLibrary.fromSchemaWithLimits(schemaOf(1001), limits), 'LimitExceeded').message)
    .toBe('host input byte limit');
  expect(kind(() => wasm.ComponentLibrary.fromSchemaWithLimits('['.repeat(1001), limits), 'LimitExceeded').message)
    .toBe('host input byte limit');
  kind(() => wasm.ComponentLibrary.fromSchemaWithLimits('['.repeat(1000), limits), 'InvalidSchema');
  limits.free();
  // The arguments are checked by the package guard before Rust sees them.
  kind(() => wasm.ComponentLibrary.fromSchemaWithLimits(42, new wasm.RuntimeLimits()), 'InvalidJson');
  kind(() => wasm.ComponentLibrary.fromSchemaWithLimits('{}', {}), 'InvalidHandle');
});

const action = (name: string) => ({ kind: 'action', name });
const shapeDefinition = (first: string, actions: unknown[]) => ({ components: [{ name: 'C', description: '',
  props: [{ name: 'go', type: action(first) }] }], actions, root: 'C' });
function chainOf(n: number) {
  return shapeDefinition('A0', Array.from({ length: n }, (_, i) => ({ name: `A${i}`, description: '',
    params: [{ name: 'p', type: i + 1 < n ? action(`A${i + 1}`) : { kind: 'string' } }] })));
}
function doublingOf(n: number) {
  return shapeDefinition('A0', Array.from({ length: n }, (_, i) => {
    const type = i + 1 < n ? action(`A${i + 1}`) : { kind: 'string' };
    return { name: `A${i}`, description: '', params: [{ name: 'p', type }, { name: 'q', type }] };
  }));
}
function manyOf(n: number) {
  return shapeDefinition('Many', [
    { name: 'Big', description: '', params: Array.from({ length: n }, (_, i) =>
      ({ name: `b${i}`, type: { kind: 'string' }, optional: i > 0 })) },
    { name: 'Many', description: '', params: Array.from({ length: n }, (_, i) =>
      ({ name: `m${i}`, type: action('Big'), optional: i > 2 })) },
  ]);
}

// The shapes measured on #140 before sharing, through the definition and
// through its JSON Schema: a chain of 2,000 actions (4.7 GB natively, and
// freeing it overflowed the engine's stack in Bun past 1,750), the doubling
// shape at 30 actions (terabytes) and 2,000 params referring to an action
// with 2,000 params (1.7 GB). The many-reference case uses 1,500 of each
// here. Each is made, parsed one-shot and streamed, and freed, and the
// instance keeps working after every one. Measured 2026-11-22
// in Bun 1.4, made to freed (definition / schema): chain 2,000 29 ms / 10 ms,
// 4,000 54 ms / 19 ms; doubling 30 0.8 ms / 0.3 ms, 60 1.2 ms / 0.5 ms; many
// 2,000 22 ms / 12 ms.
test.each<[string, unknown, string, string]>([
    ['chain', chainOf(2000), 'root = C(@A0({type: "A1", params: {p: {type: "A2", params: {p: 1}}}}))', '/p/params/p/params/p'],
    ['chain 4,000', chainOf(4000), 'root = C(@A0({type: "A1", params: {p: 1}}))', '/p/params/p'],
    ['doubling', doublingOf(30), 'root = C(@A0({type: "A1", params: {p: 1, q: {type: "A2", params: {p: "x"}}}}, null))', '/p/params/p'],
    ['doubling 60', doublingOf(60), 'root = C(@A0({type: "A1", params: {p: 1, q: 1}}, null))', '/p/params/q'],
    ['many', manyOf(1500), 'root = C(@Many({type: "Big", params: {b0: "x", b1: 2}}, null, {type: "Big", params: {b0: "y"}}))', '/m0/params/b1'],
])('the %s shape loads, parses and frees with bounded memory and leaves the instance usable',
  (name, definition, program, firstError) => {
    const results: string[] = [];
    let schema = '';
    for (const viaSchema of [false, true]) {
      const {pkg, bytes} = packageWithMemory();
      const before = bytes();
      const library = viaSchema ? new pkg.ComponentLibrary(schema)
        : pkg.ComponentLibrary.fromDefinition(JSON.stringify(definition));
      const result = pkg.parse(program, library);
      const stream = new pkg.StreamSession(library);
      for (let i = 0; i < program.length; i += 16) stream.push(program.slice(i, i + 16));
      expect(stream.finish()).toBe(result);
      stream.free();
      if (!viaSchema) schema = library.jsonSchema()!;
      library.free();
      expect(bytes() - before, `${name} via ${viaSchema ? 'schema' : 'definition'}`)
        .toBeLessThan(128 * 1024 * 1024);
      expect(result).toContain(`"path":"${firstError}"`);
      results.push(result);
      // The instance is still usable.
      const small = new pkg.ComponentLibrary('{"$defs":{"T":{"properties":{"v":{}}}}}');
      expect(JSON.parse(pkg.parse('root = T(1)', small)).root.props.v).toBe(1);
      small.free();
    }
    expect(results[0]).toBe(results[1]);
    // And so is a schema read again after its library is gone.
    const again = new wasm.ComponentLibrary(schema);
    again.free();
  }, 30_000);

// A library schema's text nested `levels` deep in all, of objects or arrays
// (as crates/openui-ffi/tests/library_schemas.rs builds it).
function nestedSchema(shape: 'object' | 'array', levels: number) {
  const [open, close] = shape === 'object'
    ? ['{"type":"object","properties":{"a":', '}}'] : ['{"type":"array","items":', '}'];
  const odd = shape === 'object' && (levels - 5) % 2 === 1;
  const count = shape === 'object' ? Math.floor((levels - (odd ? 6 : 5)) / 2) : levels - 5;
  const leaf = odd ? '{"type":"string","enum":["a"]}' : '{"type":"string"}';
  return `{"$defs":{"Box":{"properties":{"v":${open.repeat(count)}${leaf}${close.repeat(count)}}}}}`;
}

// serde_json reads 127 levels and refuses the 128th. The read runs on a stack
// segment of its own (#140), here as natively: a schema at the limit is read,
// compiled, parsed with and freed, one past it is refused with the message
// it always had, and the instance keeps working.
test('a schema nested to the reader\'s limit is read; one level more is refused as before (#140)', () => {
  const columns = { object: 2195, array: 2988 };
  for (const shape of ['object', 'array'] as const) {
    expect(kind(() => new wasm.ComponentLibrary(nestedSchema(shape, 128)), 'InvalidSchema').message)
      .toBe(`recursion limit exceeded at line 1 column ${columns[shape]}`);
    const library = new wasm.ComponentLibrary(nestedSchema(shape, 127));
    const value = shape === 'object' ? '{a: '.repeat(60) + '1' + '}'.repeat(60) : '['.repeat(120) + '1' + ']'.repeat(120);
    expect(wasm.parse(`root = Box(${value})`, library)).toContain('type-mismatch');
    library.free();
  }
  const small = new wasm.ComponentLibrary('{"$defs":{"T":{"properties":{"v":{}}}}}');
  expect(JSON.parse(wasm.parse('root = T(1)', small)).root.props.v).toBe(1);
  small.free();
});

test('malformed schemas are refused with the messages they always had', () => {
  const cases: [string, string][] = [
    ['', 'EOF while parsing a value at line 1 column 0'],
    ['{', 'EOF while parsing an object at line 1 column 1'],
    ['[1,]', 'trailing comma at line 1 column 4'],
    ['nul', 'EOF while parsing a value at line 1 column 3'],
    ['{"$defs": {}} x', 'trailing characters at line 1 column 15'],
    ['{"a": "\\ud800"}', 'unexpected end of hex escape at line 1 column 14'],
    ['{"a": 1e400}', 'number out of range at line 1 column 11'],
  ];
  for (const [text, message] of cases) {
    expect(kind(() => new wasm.ComponentLibrary(text), 'InvalidSchema').message).toBe(message);
  }
});

// JSON nested `levels` deep, of objects or arrays, around the number 1.
function nested(shape: 'object' | 'array', levels: number) {
  const [open, close] = shape === 'object' ? ['{"a":', '}'] : ['[', ']'];
  return open.repeat(levels) + '1' + close.repeat(levels);
}

// #203: a library's prompt options are read by the core's reader under the
// evaluation depth limit (800 JSON levels by default). Options at the limit
// make a prompt and one level more is refused; a cloud request holds them
// one level down, under the same limit. Text the reader refuses, or whose
// repeated key replaces a value, after a value nested to the limit frees
// what it read with a loop, and the instance keeps working.
test('prompt options nested to the depth limit make prompts; the reader frees what it lets go of (#203)', () => {
  const library = define({ components: [{ name: 'C', description: '', props: [] }], root: 'C' });
  const plain = library.prompt(), config = library.cloudConfig();
  for (const shape of ['object', 'array'] as const) {
    const options = (levels: number) => `{"extra":${nested(shape, levels - 1)}}`;
    expect(library.prompt(options(800))).toBe(plain);
    kind(() => library.prompt(options(801)), 'InvalidPromptSpec');
    expect(library.cloudConfig(options(799))).toBe(config);
    expect(kind(() => library.cloudConfig(options(800)), 'Prompt').message)
      .toBe('[generateSystemPrompt] the spec nests deeper than 800 levels.');
    kind(() => library.cloudConfig(options(801)), 'InvalidPromptSpec');
    const deep = nested(shape, 798);
    expect(library.prompt(`{"extra":${deep},"extra":1}`)).toBe(plain);
    for (const text of [`{"extra":${deep}} x`, `{"extra":${deep},"b":!}`, `{"extra":[${deep},${nested(shape, 900)}]}`]) {
      kind(() => library.prompt(text), 'InvalidPromptSpec');
      kind(() => wasm.ComponentLibrary.fromDefinition(text), 'InvalidJson');
    }
  }
  // Options may replace any field the generator reads, and it copies some
  // with serde_json: each, nested near the limit, gives a prompt or a typed
  // error, never a trap.
  for (const key of ['components', 'functions', 'actions', 'tools', 'componentGroups', 'examples', 'additionalRules']) {
    for (const shape of ['object', 'array'] as const) {
      const deep = nested(shape, 797);
      for (const text of [`{"${key}":{"x":${deep}}}`, `{"${key}":[{"name":"t","inputSchema":${deep}}]}`]) {
        for (const make of [() => library.prompt(text), () => library.cloudConfig(text)]) {
          try { expect(typeof make()).toBe('string'); } catch (e: any) { expect(e.kind).toBe('Prompt'); }
        }
      }
    }
  }
  expect(library.prompt()).toBe(plain);
  library.free();
});
