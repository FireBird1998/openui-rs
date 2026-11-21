// A library definition through the WebAssembly package (#137): the
// plain-data form goes in as JSON text and the component library comes out,
// with its JSON Schema and prompt spec as text byte for byte as upstream
// writes them, usable everywhere a library supplied as JSON Schema is. The
// samples are conformance/golden/library-definitions.json, written by the
// reference harness running upstream's code.
import { test, expect } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import golden from '../../../conformance/golden/library-definitions.json';
import { wasm, kind } from './support';

type Sample = { name: string; definition: any; schema?: any; spec?: any; error?: string };
const samples = golden as Sample[];
const sample = (name: string) => {
  const found = samples.find(s => s.name === name);
  if (!found) throw new Error(`no sample ${name}`);
  return found;
};
const define = (definition: unknown) => wasm.ComponentLibrary.fromDefinition(JSON.stringify(definition));
const fromUpstream = (s: Sample) => new wasm.ComponentLibrary(JSON.stringify(s.schema));

test('every sample gives upstream\'s schema and spec text, or its error as InvalidJson', () => {
  expect(samples.length).toBe(93);
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
  expect([errors, libraries]).toEqual([23, 70]);
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
});

// A chain of library actions, each one's param an action slot for the next.
// The parser's library copies each action's shape into every param that
// refers to it, as upstream does, so this costs memory and time quadratic in
// its length and nests the copies about four JSON levels per action (#140).
function actionChain(n: number) {
  const actions = [];
  for (let i = 0; i < n; i++) {
    actions.push({ name: `A${i}`, description: '', params: [
      { name: 'p', type: i + 1 < n ? { kind: 'action', name: `A${i + 1}` } : { kind: 'string' } }] });
  }
  return JSON.stringify({ components: [{ name: 'C', description: '', props: [
    { name: 'go', type: { kind: 'action', name: 'A0' } }] }], actions, root: 'C' });
}

// Measured (2026-11-22): 100 actions take about 10 ms here. Freeing the
// library recurses on the JavaScript engine's stack and overflows it between
// 600 and 1,000 actions in Node 22 and between 1,750 and 2,000 in Bun 1.4,
// thrown as Worker; making 2,600 exhausts WebAssembly's 4 GiB.
test('a long action chain of a handled size is made, used and freed', () => {
  const started = performance.now();
  const library = wasm.ComponentLibrary.fromDefinition(actionChain(100));
  const program = 'root = C(Action([@A0(Action([@A1("x")]))]))';
  const stream = new wasm.StreamSession(library);
  for (const c of program) stream.push(c);
  expect(stream.finish()).toBe(wasm.parse(program, library));
  stream.free();
  const session = new wasm.RuntimeSession(library);
  const parsed = wasm.parseProgram(program, library);
  session.setProgram(parsed); parsed.free();
  const draw = session.evaluate();
  expect(draw.json).toContain('"name":"A0"');
  draw.free(); session.free(); library.free();
  expect(performance.now() - started).toBeLessThan(10_000);
});
