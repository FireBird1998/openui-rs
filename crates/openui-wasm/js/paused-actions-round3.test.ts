// #117 round 3: the narrowed rule (ADR 0014). After free() or dispose()
// returns, no driver of the session calls a callback, invokes a provider or
// delivers into the session; what the session produced before is delivered
// once, before the free returns, within a bound.
import { expect, test } from 'bun:test';
import { wasm } from './support';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';

async function settle() {
  for (let i = 0; i < 4; i++) { await nextToolTurn(); await null; await null; }
}

test('a driver detached earlier calls nothing after the session is freed', async () => {
  for (const stop of ['free', 'dispose']) {
    using library = new wasm.ComponentLibrary('{"$defs":{}}');
    const s = new wasm.RuntimeSession(library);
    using program = wasm.parseProgram('q=Query("read")', library);
    let stopped = false;
    const after = [];
    const late = Promise.withResolvers();
    s.setToolProvider({ read: () => 1 }, {
      onChange: () => late.promise,
      onError: () => { if (stopped) after.push('onError'); },
    });
    s.replaceProgram(program, undefined, undefined, false).free();
    await settle();
    s.setToolProvider(null);
    s[stop]();
    stopped = true;
    late.reject({ toString() { if (stopped) after.push('toString'); return 'late'; } });
    await settle();
    expect(after).toEqual([]);
    if (stop === 'dispose') s.free();
  }
});

test('a free inside onChange after a detach calls no callback once it returns', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  const s = new wasm.RuntimeSession(library);
  using program = wasm.parseProgram('q=Query("read")', library);
  let stopped = false, armed = false;
  const after = [];
  s.setToolProvider({ read: () => 1 }, {
    onChange() {
      // The read's result, delivered in a later task, is the first change.
      if (!armed) return;
      armed = false;
      s.setToolProvider(null);
      s.free();
      stopped = true;
      throw Error('after free');
    },
    onError: () => { if (stopped) after.push('onError'); },
  });
  s.replaceProgram(program, undefined, undefined, false).free();
  armed = true;
  await settle();
  expect(stopped).toBe(true);
  expect(after).toEqual([]);
});

const SCHEMA = '{"$defs":{"Box":{"properties":{"value":{},"a":{},"b":{}}}}}';
const PROGRAM = '$n=0\na=Mutation("a")\nb=Mutation("b")\nroot=Box($n,Action([@Run(a)]),Action([@Run(b)]))';

function paused(onAction, maxContinuations?: bigint) {
  const library = new wasm.ComponentLibrary(SCHEMA);
  using limits = new wasm.RuntimeLimits();
  if (maxContinuations !== undefined) limits.maxContinuations = maxContinuations;
  const s = maxContinuations === undefined ? new wasm.RuntimeSession(library)
    : wasm.RuntimeSession.withLimits(library, undefined, limits);
  const parsed = wasm.parseProgram(PROGRAM, library);
  library.free();
  const errors = [];
  let deliveries = 0;
  const never = () => new Promise(() => {});
  s.setToolProvider({ a: never, b: never }, {
    onError: e => errors.push([e.kind, e.message]),
    onAction: () => { deliveries++; onAction?.(s, press); },
  });
  s.replaceProgram(parsed, undefined, undefined, true).free();
  parsed.free();
  function press(prop) {
    using draw = s.evaluate();
    using action = s.actionAt(draw.evaluation, ['props', prop]);
    s.run(action, 'click').free();
  }
  return { s, errors, press, count: () => deliveries };
}

test('free bounds callback-produced rounds by the session continuation limit', async () => {
  using defaults = new wasm.RuntimeLimits();
  for (const configured of [1n, 4n, undefined]) {
    const rounds = Number(configured ?? defaults.maxContinuations) + 1;
    const { s, errors, press, count } = paused((session, click) => {
      session.resumeQueries(); click('b'); session.pauseQueries();
    }, configured);
    press('a');
    await nextToolTurn();
    s.pauseQueries();
    s.free();
    expect(count()).toBe(rounds);
    await settle();
    expect(errors.filter(([kind]) => kind === 'LimitExceeded')).toEqual([
      ['LimitExceeded', `free() stopped delivering after ${rounds} rounds: callbacks kept producing action segments`],
    ]);
  }
});

test('the delivery before a detach reports the evaluation error it takes', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  const limits = new wasm.RuntimeLimits();
  const parse = new wasm.ParseLimits();
  parse.resultSize = 2048n;
  limits.evaluation = parse;
  const s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const errors = [];
  // `slow` never settles, so a drain asked for after a write waits for a task.
  s.setToolProvider({ search: () => 'data', slow: () => new Promise(() => {}) },
    { onError: e => errors.push(e.kind) });
  using first = wasm.parseProgram('$n="a"\nq=Query("search",{term:$n})\nw=Query("slow")\nroot=Box(q)', library);
  s.replaceProgram(first, undefined, undefined, false).free();
  await settle();
  expect(errors).toEqual([]);
  // The write makes the registration exceed the limit; its drain waits for
  // a task, and the detach comes first.
  s.set('$n', JSON.stringify('x'.repeat(1500)));
  expect(errors).toEqual([]);
  s.setToolProvider(null);
  expect(errors).toEqual(['LimitExceeded']);
  await settle();
  expect(errors).toEqual(['LimitExceeded']);
  s.free();
});

test('a driver attached inside a closing drain and the closing one both end at free', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  const s = new wasm.RuntimeSession(library);
  using program = wasm.parseProgram('q=Query("read")', library);
  let stopped = false, armed = false;
  const after = [];
  const late = Promise.withResolvers();
  const record = name => () => { if (stopped) after.push(name); };
  s.setToolProvider({ read: () => 1 }, {
    onChange() {
      // The detached driver's last drain redraws: attach another, then free.
      if (!armed) return undefined;
      armed = false;
      s.setToolProvider({ read: () => 2 }, { onChange: record('new onChange'), onError: record('new onError') });
      s.free();
      stopped = true;
      return late.promise;
    },
    onError: record('old onError'),
  });
  s.replaceProgram(program, undefined, undefined, false).free();
  await settle();
  armed = true;
  s.setToolProvider(null);
  expect(stopped).toBe(true);
  late.reject(Error('late'));
  await settle();
  expect(after).toEqual([]);
});

test('a free inside a flush shares the remaining rounds', async () => {
  const { s, count, errors, press } = paused((session, click) => {
    if (count() === 6) return; // keep a regression finite
    session.resumeQueries(); click('b'); session.pauseQueries();
    session.free();
  }, 1n);
  press('a');
  await nextToolTurn();
  s.pauseQueries();
  s.free();
  expect(count()).toBe(2);
  expect(errors.filter(([kind]) => kind === 'LimitExceeded')).toHaveLength(1);
});
