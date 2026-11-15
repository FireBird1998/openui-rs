// Freeing and argument conversion go through the guard (review of #42):
// no application code runs between the guard's checks and the wasm call, and
// a freed object is never handed to Rust.
import { test, expect } from 'bun:test';
import { wasm, kind, setup, suspend, SCHEMA } from './support';

/** An object whose every conversion hook counts and tries to free `victims`. */
function hostile(victims: any[]) {
  const calls = { count: 0 };
  const hook = () => { calls.count++; for (const v of victims) v[Symbol.dispose](); return 1; };
  const value = { valueOf: hook, toString: hook, [Symbol.toPrimitive]: hook, charCodeAt: hook,
    get length() { hook(); return 3; } };
  return { value, calls };
}

test('Symbol.dispose is the guarded free on every class', () => {
  for (const name of ['ActionHandle', 'ActionOutput', 'ComponentLibrary', 'ContinuationHandle',
    'EvaluationHandle', 'EvaluationOutput', 'ParseLimits', 'ParsedProgram', 'RuntimeLimits',
    'RuntimeSession', 'StreamSession']) {
    const prototype = wasm[name].prototype;
    expect(prototype[Symbol.dispose]).toBe(prototype.free);
  }
});

test('an object disposed with Symbol.dispose or using then throws Disposed', () => {
  const { s, library, action } = setup();
  const continuation = suspend(s, action);
  continuation[Symbol.dispose]();
  kind(() => s.resume(continuation, true), 'Disposed');
  expect(s.snapshot()).toBe('{"$n":0}');
  expect(s.pendingContinuations()).toBe(0n);
  let outer: any;
  { using inner = new wasm.RuntimeSession(library); outer = inner; expect(inner.version()).toBe(0n); }
  kind(() => outer.version(), 'Disposed');
  s[Symbol.dispose]();
  kind(() => s.version(), 'Disposed');
  kind(() => s.evaluate(), 'Disposed');
  s.free(); s[Symbol.dispose](); outer.free();
  action.free(); library.free();
});

test('a boolean argument is never converted by application code', () => {
  const { s, library, action } = setup();
  const continuation = suspend(s, action);
  const before = [s.snapshot(), s.version(), s.pendingContinuations()];
  const evil = hostile([continuation, s]);
  kind(() => s.resume(continuation, evil.value), 'InvalidJson');
  kind(() => s.resume(continuation, 1), 'InvalidJson');
  kind(() => s.setField(undefined, 'Field', 'x', '"v"', evil.value), 'InvalidJson');
  kind(() => s.initializeField(undefined, 'Field', 'x', '"v"', evil.value), 'InvalidJson');
  kind(() => s.initializeField(undefined, 'Field', 'x', '"v"'), 'InvalidJson');
  const stream = new wasm.StreamSession(library);
  kind(() => stream.program(evil.value), 'InvalidJson');
  expect(evil.calls.count).toBe(0);
  expect([s.snapshot(), s.version(), s.pendingContinuations()]).toEqual(before);
  // Both objects are still live: the continuation resumes exactly once.
  const out = s.resume(continuation, true); out.free();
  expect(s.snapshot()).toBe('{"$n":1}');
  kind(() => s.resume(continuation, true), 'ConsumedHandle');
  continuation.free(); stream.free(); action.free(); s.free(); library.free();
});

test('string arguments are checked before the generated code touches them', () => {
  const { s, library, action } = setup();
  const evil = hostile([s, action]);
  const v = evil.value;
  const calls: (() => unknown)[] = [
    () => s.get(v), () => s.set(v), () => s.set('$n', v), () => s.isPristine(v),
    () => s.markBinding(v, 'p'), () => s.writeBinding(v, '1'), () => s.getField(v, 'x'),
    () => s.getField(undefined, v), () => s.setField(undefined, v, 'x', '1', true),
    () => s.setField(undefined, undefined, v, '1', true), () => s.setField(undefined, undefined, 'x', v, true),
    () => s.initialize(v), () => s.setReferenceValues(v), () => s.run(action, v),
    () => s.run(action, 'm', v), () => s.runLegacy(v, 'm'), () => s.initializeField(v, 'F', 'x', '1', false),
    () => new wasm.ComponentLibrary(v), () => new wasm.RuntimeSession(library, v),
    () => wasm.parse(v, library), () => wasm.parseProgram('root=Box(1)', library, v),
  ];
  for (const call of calls) kind(call, 'InvalidJson');
  for (let i = 0; i < 1000; i++) kind(() => s.get(v), 'InvalidJson');
  expect(evil.calls.count).toBe(0);
  expect(s.version()).toBe(1n);
  action.free(); s.free(); library.free();
});

test('bigint limits are checked before the generated code converts them', () => {
  const limits = new wasm.RuntimeLimits(), parseLimits = new wasm.ParseLimits();
  const evil = hostile([limits, parseLimits]);
  kind(() => { limits.maxContinuations = evil.value; }, 'LimitExceeded');
  kind(() => { limits.maxJsonBytes = 5; }, 'LimitExceeded');
  kind(() => { limits.maxContinuations = -1n; }, 'LimitExceeded');
  kind(() => { limits.maxContinuations = 18446744073709551616n; }, 'LimitExceeded');
  kind(() => { parseLimits.depth = evil.value; }, 'LimitExceeded');
  expect(evil.calls.count).toBe(0);
  limits.maxContinuations = 2n; parseLimits.depth = undefined; parseLimits.work = 7n;
  expect([limits.maxContinuations, parseLimits.depth, parseLimits.work]).toEqual([2n, undefined, 7n]);
  limits.free(); parseLimits.free();
});

test('handle arguments accept only live objects of their own class', () => {
  const { s, library, action } = setup();
  const continuation = suspend(s, action);
  kind(() => s.resume(action, true), 'InvalidHandle');
  kind(() => s.run(continuation, 'm'), 'InvalidHandle');
  kind(() => s.run({ __wbg_ptr: action.__wbg_ptr }, 'm'), 'InvalidHandle');
  kind(() => wasm.RuntimeSession.withLimits(library, undefined, {}), 'InvalidHandle');
  const limits = new wasm.RuntimeLimits(), copy = limits.evaluation;
  copy.free();
  kind(() => { limits.evaluation = copy; }, 'Disposed');
  action.free();
  kind(() => s.run(action, 'm'), 'Disposed');
  const out = s.run(undefined, 'message'); out.free();
  continuation.free(); limits.free(); s.free(); library.free();
});

test('a path getter can neither free nor reenter while actionAt converts it', () => {
  const { s, library, action } = setup();
  const out = s.evaluate(), evaluation = out.evaluation;
  const path = ['props', 'a']; let visits = 0;
  Object.defineProperty(path, 0, { get() {
    visits++;
    kind(() => s[Symbol.dispose](), 'Worker'); kind(() => s.free(), 'Worker');
    kind(() => evaluation[Symbol.dispose](), 'Worker'); kind(() => action.free(), 'Worker');
    kind(() => s.set('$n', '5'), 'Worker'); kind(() => new wasm.RuntimeSession(library), 'Worker');
    return 'props';
  } });
  const found = s.actionAt(evaluation, path);
  expect(visits).toBe(1);
  kind(() => s.actionAt(evaluation, ['props', 1]), 'InvalidHandle');
  kind(() => s.actionAt(evaluation, 'props'), 'InvalidHandle');
  const run = s.run(found, 'click'); run.free();
  expect(s.get('$n')).toBe('0');
  found.free(); evaluation.free(); out.free(); action.free(); s.free(); library.free();
});

test('lone surrogates in a path become U+FFFD per segment, as wasm-bindgen converts strings', () => {
  const { s, library, action } = setup();
  const out = s.evaluate(), evaluation = out.evaluation;
  // A high surrogate ending one segment and a low one starting the next stay
  // two replacement characters, so neither segment names a real key.
  kind(() => s.actionAt(evaluation, ['props\ud800', '\udc00a']), 'InvalidHandle');
  evaluation.free(); out.free(); action.free(); s.free(); library.free();
});

/** Every public member of `object`'s class, called on it after it was freed. */
function expectEveryMemberDisposed(object: any) {
  const prototype = Object.getPrototypeOf(object);
  object.free();
  for (const key of Reflect.ownKeys(prototype)) {
    if (key === 'constructor' || (typeof key === 'string' && key.startsWith('__'))) continue;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, key)!;
    if (key === 'free' || key === Symbol.dispose) { expect(object[key]()).toBeUndefined(); continue; }
    if (descriptor.value) kind(() => object[key](), 'Disposed');
    if (descriptor.get) kind(() => object[key], 'Disposed');
    if (descriptor.set) kind(() => { object[key] = undefined; }, 'Disposed');
  }
}
test('every member of a freed object throws Disposed', () => {
  const { s, library, action } = setup();
  const continuation = suspend(s, action);
  const run = s.run(action, 'again');
  const evaluated = s.evaluate(), evaluation = evaluated.evaluation;
  const limits = new wasm.RuntimeLimits(), parseLimits = limits.evaluation;
  const program = wasm.parseProgram('root=Box(1)', library), stream = new wasm.StreamSession(library);
  for (const object of [continuation, evaluation, evaluated, run, action, limits, parseLimits, program, stream, s]) {
    expectEveryMemberDisposed(object);
  }
  const other = new wasm.RuntimeSession(library);
  kind(() => other.setProgram(program), 'Disposed');
  kind(() => other.resume(continuation, true), 'Disposed');
  other.free(); expectEveryMemberDisposed(library);
  kind(() => new wasm.RuntimeSession(library), 'Disposed');
});

test('ComponentLibrary refuses a schema that is not JSON with InvalidSchema', () => {
  kind(() => new wasm.ComponentLibrary('{'), 'InvalidSchema');
  const library = new wasm.ComponentLibrary(SCHEMA); library.free();
});
