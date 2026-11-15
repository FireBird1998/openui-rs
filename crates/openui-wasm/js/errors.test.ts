// Errors and records are built by the guard after Rust returns, with
// intrinsics captured at load: hostile globals can neither run inside Rust
// frames nor strand a session's borrow (which made free() fail forever).
import { test, expect } from 'bun:test';
import { wasm, kind, setup, suspend } from './support';
const RealError = Error;

/** Run `body` with `object[key]` replaced, restoring it afterwards. */
function patched(object: any, key: PropertyKey, replacement: PropertyDescriptor, body: () => void) {
  const original = Object.getOwnPropertyDescriptor(object, key);
  // Bun makes some intrinsics non-configurable but writable: assign those.
  if (original && !original.configurable) {
    object[key] = replacement.value;
    try { body(); } finally { object[key] = original.value; }
    return;
  }
  Object.defineProperty(object, key, { configurable: true, ...replacement });
  try { body(); } finally {
    if (original) Object.defineProperty(object, key, original); else delete object[key];
  }
}

test('an Object.prototype.kind setter never runs and leaves the session freeable', () => {
  const { s, library, action } = setup();
  let runs = 0;
  patched(Object.prototype, 'kind', { set() { runs++; s.version(); throw new Error('setter'); } }, () => {
    for (let i = 0; i < 5; i++) kind(() => s.set('$n', '{'), 'InvalidJson');
    kind(() => s.writeBinding('{}', '1'), 'InvalidMarker');
    kind(() => s.version.call({}), 'InvalidHandle');
  });
  expect(runs).toBe(0);
  expect(s.set('$n', '5')).toBe(true); expect(s.get('$n')).toBe('5');
  action.free(); s.free(); library.free();
  kind(() => s.version(), 'Disposed');
});

/** Records, events, a refusal and a path lookup from a fresh session. */
function exercise() {
  const { s, library, action } = setup();
  const init = s.initialize(), bindings = s.undecidedBindings();
  const field = s.setField(undefined, 'Field', 'x', '"v"', true);
  const out = s.run(action, 'click'), continuation = out.continuation;
  const run = { events: out.events, failure: out.failure }; out.free(); continuation.free();
  let refusal; try { s.set('$n', '{'); } catch (e: any) { refusal = [e.kind, e instanceof RealError]; }
  const tree = s.evaluate(), evaluation = tree.evaluation;
  const found = s.actionAt(evaluation, ['props', 'a']); evaluation.free(); tree.free();
  return { observed: { init, bindings, field, run, refusal, snapshot: s.snapshot() }, s, library, found, action };
}
test('patched JSON, Reflect, Error, TextDecoder and Table globals change no result', () => {
  const expected = exercise();
  let patchedRun: any;
  const thrower = { value() { throw new Error('patched'); } };
  patched(JSON, 'parse', thrower, () => patched(JSON, 'stringify', thrower, () =>
    patched(Reflect, 'set', thrower, () => patched(Reflect, 'apply', thrower, () =>
      patched(TextDecoder.prototype, 'decode', thrower, () =>
        patched(WebAssembly.Table.prototype, 'get', thrower, () =>
          patched(globalThis, 'Error', { value: function () { throw new TypeError('patched Error'); } }, () => {
            patchedRun = exercise();
          })))))));
  expect(patchedRun.observed).toEqual(expected.observed);
  expect(expected.observed.refusal).toEqual(['InvalidJson', true]);
  for (const run of [expected, patchedRun]) {
    run.found.free(); run.action.free(); run.s.free(); run.library.free();
    kind(() => run.s.snapshot(), 'Disposed');
  }
});

test('a patched string intrinsic used before the call gives a typed error and strands nothing', () => {
  const { s, library, action } = setup();
  patched(String.prototype, 'charCodeAt', { value() { throw new Error('patched charCodeAt'); } }, () => {
    const error = kind(() => s.set('$n', '7'), 'Worker');
    expect(error.message).toBe('patched charCodeAt');
  });
  expect(s.set('$n', '7')).toBe(true);
  const continuation = suspend(s, action);
  continuation.free(); action.free(); s.free(); library.free();
});

test('nested refusals in records keep the error kind and message shape', () => {
  const limits = new wasm.RuntimeLimits(), evaluation = limits.evaluation;
  evaluation.resultSize = 1024n; limits.evaluation = evaluation;
  const library = new wasm.ComponentLibrary('{"$defs":{}}');
  const s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const init = s.initialize(JSON.stringify({ bad: 'x'.repeat(1100), ok: 1 }));
  expect(init.refused.length).toBe(1);
  expect(init.refused[0].name).toBe('bad');
  expect(init.refused[0].error.kind).toBe('RefusedWrite');
  expect(typeof init.refused[0].error.message).toBe('string');
  s.free(); library.free(); evaluation.free(); limits.free();
});
