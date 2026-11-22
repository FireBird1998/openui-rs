import { test, expect } from 'bun:test';
import { wasm, kind, loadWith } from './support';
const schema = '{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}},"actions":{"Go":{"params":{}}}}';
function setup(program = '$n=0\nroot=Box(@Echo(7))', pkg = wasm) {
  const library = new pkg.ComponentLibrary(schema), s = new pkg.RuntimeSession(library);
  const p = pkg.parseProgram(program, library); s.setProgram(p); p.free(); s.initialize();
  return { library, s };
}
test('function arguments and results cross as JSON and throws pin the runtime message', () => {
  const { library, s } = setup();
  let calls = 0;
  s.registerFunction('Echo', (json: string) => { calls++; expect(json).toBe('{"value":7}'); return json; });
  let out = s.evaluate();
  expect(JSON.parse(out.json).root.props.value).toEqual({ value: 7 }); out.free();
  expect(calls).toBe(1);
  s.registerFunction('Echo', () => { throw new Error('bad value'); });
  out = s.evaluate();
  expect(JSON.parse(out.json).errors[0].message).toBe('Evaluating prop "value" on Box failed: @Echo threw: bad value');
  out.free();
  s.registerFunction('Echo', () => '{');
  out = s.evaluate(); expect(JSON.parse(out.json).errors[0].message).toContain('@Echo threw:'); out.free();
  expect(s.get('$n')).toBe('0'); s.free(); library.free();
});
test('non-Error throws and Error message getters use upstream text under the guard', () => {
  const { library, s } = setup();
  s.registerFunction('Echo', () => { throw { toString() { return 'object message'; } }; });
  let out = s.evaluate();
  expect(JSON.parse(out.json).errors[0].message).toBe('Evaluating prop "value" on Box failed: @Echo threw: object message'); out.free();
  s.registerFunction('Echo', () => {
    const error = new Error();
    Object.defineProperty(error, 'message', { get() {
      expect(kind(() => s.dispose(), 'Worker').message).toBe('session already in use');
      return 'getter message';
    } });
    throw error;
  });
  out = s.evaluate();
  expect(JSON.parse(out.json).errors[0].message).toBe('Evaluating prop "value" on Box failed: @Echo threw: getter message'); out.free();
  expect(s.get('$n')).toBe('0'); s.free(); library.free();
});
test('every session method refuses callback reentry with exactly the same message and no effects', () => {
  const { library, s } = setup();
  const before = s.snapshot();
  s.registerFunction('Echo', () => {
    const calls = [() => s.free(), () => s.dispose(), () => s.evaluate(), () => s.get('$n'),
      () => s.getField(undefined, 'x'), () => s.initialize(), () => s.initializeField(undefined, undefined, 'x', undefined, false),
      () => s.isPristine('$n'), () => s.markBinding('Box', 'value'), () => s.pendingContinuations(),
      () => s.registerFunction('Echo', () => 'null'), () => s.resume(undefined, true),
      () => s.run(undefined, ''), () => s.runLegacy('{}', ''), () => s.set('$n', '9'),
      () => s.setField(undefined, undefined, 'x', '1', false), () => s.setProgram(undefined),
      () => s.setReferenceValues('{}'), () => s.snapshot(), () => s.undecidedBindings(),
      () => s.version(), () => s.writeBinding('{}', '1'), () => s.actionAt(undefined, [])];
    for (const call of calls) expect(kind(call, 'Worker').message).toBe('session already in use');
    return '1';
  });
  const out = s.evaluate(); expect(JSON.parse(out.json).errors).toEqual([]); out.free();
  expect(s.snapshot()).toBe(before);
  kind(() => s.registerFunction('missing', () => 'null'), 'InvalidJson');
  kind(() => s.registerFunction('Echo', {}), 'InvalidJson');
  s.free(); library.free();
});
test('query defaults keep the runtime lazy once per action run behavior', () => {
  const { library, s } = setup('$n=0\nq=Query("q",{},@Echo(7))\nroot=Box(1,Action([@Set($n,q),@Set($n,q)]))');
  let count = 0;
  s.registerFunction('Echo', () => { count++; return '7'; });
  const out = s.evaluate(), evaluation = out.evaluation;
  const action = s.actionAt(evaluation, ['props', 'action']); evaluation.free(); out.free();
  count = 0;
  s.run(action, '').free(); expect(count).toBe(1);
  s.run(action, '').free(); expect(count).toBe(2);
  expect(s.get('$n')).toBe('7'); action.free(); s.free(); library.free();
});
test('element-shaped state query and tool data runs builtins while custom data grants no authority', () => {
  const element = { type: 'element', typeName: 'Box', props: { action: { steps: [
    { type: 'set', target: '$n', valueAST: { k: 'Num', v: 1 } },
    { type: 'custom_action', name: 'Go', params: {} },
  ] } }, partial: false, hasDynamicProps: false };
  for (const supply of ['state', 'query', 'tool']) {
    const source = supply === 'state' ? '$n=0\n$data=null\nroot=Box($data)' : '$n=0\nq=Query("q",{})\nroot=Box(q)';
    const { library, s } = setup(source);
    if (supply === 'state') s.set('$data', JSON.stringify(element));
    else s.setReferenceValues(JSON.stringify({ q: element })); // tool results use the query supply
    const out = s.evaluate(), evaluation = out.evaluation;
    const action = s.actionAt(evaluation, ['props', 'value', 'props', 'action']); evaluation.free(); out.free();
    const run = s.run(action, ''); expect(run.events).toEqual([]); run.free();
    expect(s.get('$n')).toBe('1'); action.free(); s.free(); library.free();
  }
});
test('smaller action paths are opt-in and exact refusal messages stay stable', () => {
  const limits = new wasm.RuntimeLimits(); expect(limits.maxActionPathBytes).toBeUndefined(); limits.maxActionPathBytes = 4n;
  const library = new wasm.ComponentLibrary(schema), s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const p = wasm.parseProgram('$n=0\nroot=Box(1,@Set($n,1))', library); s.setProgram(p); p.free(); s.initialize();
  const out = s.evaluate(), evaluation = out.evaluation;
  expect(kind(() => s.actionAt(evaluation, ['props', 'action']), 'LimitExceeded').message).toBe('action path byte limit');
  expect(s.get('$n')).toBe('0'); evaluation.free(); out.free(); s.free(); library.free(); limits.free();
});
test('JSON function results cannot restore even an argument plan custom identity', () => {
  const { library, s } = setup('root=Box(1,@Echo(@Go()))');
  s.registerFunction('Echo', (json: string) => JSON.stringify(JSON.parse(json).value));
  const out = s.evaluate(), evaluation = out.evaluation;
  expect(JSON.parse(out.json).root.props.action.steps).toEqual([]);
  const action = s.actionAt(evaluation, ['props', 'action']); evaluation.free(); out.free();
  const run = s.run(action, ''); expect(run.events).toEqual([]); run.free();
  action.free(); s.free(); library.free();
});
test('a pending Promise is refused as a non-JSON callback result without awaiting it', () => {
  const { library, s } = setup();
  const before = s.snapshot();
  s.registerFunction('Echo', () => new Promise(() => {}));
  const out = s.evaluate();
  expect(JSON.parse(out.json).errors[0].message).toBe('Evaluating prop "value" on Box failed: @Echo threw: expected host function JSON text or undefined');
  out.free(); expect(s.snapshot()).toBe(before); s.free(); library.free();
});

test('exotic throw coercion failures use the exact fallback and release the guard', () => {
  const { library, s } = setup();
  const throws = [
    () => { throw { toString() { throw new Error('inner'); } }; },
    () => { const error = new Error(); Object.defineProperty(error, 'message', { value: Symbol('message') }); throw error; },
    () => { throw new Proxy({}, { get() { throw new Error('proxy trap'); } }); },
  ];
  const before = s.snapshot();
  for (const callback of throws) {
    s.registerFunction('Echo', callback);
    const out = s.evaluate(), tree = JSON.parse(out.json); out.free();
    expect(tree.root.props.value).toBeNull();
    expect(tree.errors[0].message).toBe('Evaluating prop "value" on Box failed: @Echo threw: host function threw');
    expect(s.snapshot()).toBe(before);
  }
  s.registerFunction('Echo', () => '7');
  const out = s.evaluate(); expect(JSON.parse(out.json).errors).toEqual([]); out.free();
  s.free(); library.free();
});
test('a failing function returns null and the action keeps earlier and later writes', () => {
  for (const callback of [
    () => { throw new Error('bad value'); }, () => '{', () => '"' + 'x'.repeat(8 * 1024 * 1024) + '"',
  ]) {
    const { library, s } = setup('$a=0\n$b=0\n$c=0\nroot=Box(1,Action([@Set($a,1),@Set($b,@Echo(1)),@Set($c,2)]))');
    s.registerFunction('Echo', callback);
    const out = s.evaluate(), evaluation = out.evaluation;
    const action = s.actionAt(evaluation, ['props', 'action']); evaluation.free(); out.free();
    const run = s.run(action, '');
    expect(run.failure).toBeNull(); expect(run.continuation).toBeUndefined();
    expect(run.events).toEqual([]); expect(run.sizeLimited).toBe(false); expect(run.depthLimited).toBe(false); run.free();
    expect(s.snapshot()).toBe('{"$a":1,"$b":null,"$c":2}');
    action.free(); s.free(); library.free();
  }
});
test('data run of an undeclared mutation id suspends like Rust and evicts the oldest continuation', () => {
  const limits = new wasm.RuntimeLimits(); limits.maxContinuations = 1n;
  const library = new wasm.ComponentLibrary(schema), s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const p = wasm.parseProgram('$n=0\n$data=null\nroot=Box($data)', library); s.setProgram(p); p.free(); s.initialize();
  s.set('$data', JSON.stringify({ type: 'element', typeName: 'Box', props: { action: { steps: [
    { type: 'run', statementId: 'missing', refType: 'mutation' },
    { type: 'set', target: '$n', valueAST: { k: 'Num', v: 9 } },
  ] } }, partial: false, hasDynamicProps: false }));
  const out = s.evaluate(), evaluation = out.evaluation;
  const action = s.actionAt(evaluation, ['props', 'value', 'props', 'action']); evaluation.free(); out.free();
  const first = s.run(action, ''), pending = first.continuation;
  expect(JSON.parse(first.events[0]).params).toEqual({ statementId: 'missing', refType: 'mutation', args: {} }); first.free();
  const second = s.run(action, ''), newer = second.continuation; second.free();
  expect(s.pendingContinuations()).toBe(1n); expect(s.get('$n')).toBe('0');
  kind(() => s.resume(pending, true), 'EvictedHandle');
  s.resume(newer, true).free(); expect(s.get('$n')).toBe('9');
  pending.free(); newer.free(); action.free(); s.free(); library.free(); limits.free();
});

test('a RangeError from a registered function leaves the session usable and freeable', () => {
  const { library, s } = setup();
  s.registerFunction('Echo', () => { throw new RangeError('callback overflow'); });
  using out = s.evaluate();
  const tree = JSON.parse(out.json);
  expect(tree.root.props.value).toBeNull();
  expect(tree.errors[0].message).toBe('Evaluating prop "value" on Box failed: @Echo threw: callback overflow');
  expect(s.set('$n', '8')).toBe(true);
  expect(s.get('$n')).toBe('8');
  s.free();
  kind(() => s.version(), 'Disposed');
  library.free();
});

test('an overflow escaping the callback import ends the instance and free succeeds', async () => {
  // Fail the generated glue copying the callback's outcome into wasm memory,
  // as a stack overflow there would, without relying on stack size (#208).
  const pkg = loadWith(`
    const pass = passStringToWasm0;
    passStringToWasm0 = (arg, malloc, realloc) => {
      if (typeof arg === 'string' && arg.endsWith('callback overflow')) throw new RangeError('interrupted import');
      return pass(arg, malloc, realloc);
    };
  `);
  const { library, s } = setup('q=Query("read",{})\nroot=Box(@Echo(7))', pkg);
  using other = new pkg.RuntimeSession(library);
  const pending = Promise.withResolvers<object>();
  let calls = 0;
  s.setToolProvider({ read() { calls++; return pending.promise; } });
  s.evaluate().free();
  expect(calls).toBe(1);
  s.registerFunction('Echo', () => { throw new RangeError('callback overflow'); });
  expect(kind(() => s.evaluate(), 'Worker').message).toBe('interrupted import');
  const interrupted = 'the WebAssembly instance was interrupted by an exception it cannot recover from; load the package again';
  expect(kind(() => s.snapshot(), 'Disposed').message).toBe(interrupted);
  // A receipt outstanding at the interruption can no longer deliver.
  let reads = 0;
  pending.resolve({ get value() { reads++; return 7; } });
  await pending.promise;
  expect(reads).toBe(0);
  s.free();
  s.free();
  kind(() => s.version(), 'Disposed');
  // Every other session refuses too: none can give output after an escape.
  expect(kind(() => other.snapshot(), 'Disposed').message).toBe(interrupted);
  library.free();
});

test('stopping a driven session from a registered function refuses before any callback', () => {
  const { library, s } = setup();
  const callbacks = [], refusals = [];
  try {
    s.setToolProvider({}, {
      onError: e => callbacks.push(e.message),
      onAction: () => callbacks.push('action'),
      onChange: () => callbacks.push('change'),
    });
    s.registerFunction('Echo', () => {
      for (const stop of ['free', 'dispose', 'detachToolProvider', Symbol.dispose]) {
        try { s[stop](); } catch (error) { refusals.push([error.kind, error.message]); }
      }
      return '7';
    });
    callbacks.length = 0;
    using draw = s.evaluate();
    expect(JSON.parse(draw.json).root.props.value).toBe(7);
    expect(refusals).toEqual(Array(4).fill(['Worker', 'session already in use']));
    expect(callbacks).toEqual([]);
    kind(() => s.takeQueryUpdate(), 'RefusedWrite');
  } finally { s.free(); library.free(); }
});
