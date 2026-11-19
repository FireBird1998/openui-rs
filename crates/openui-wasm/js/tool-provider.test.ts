import { expect, test } from 'bun:test';
import { kind, loadWith, wasm } from './support';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';

test('manual and driven attachments require a detach before ownership changes', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using program = wasm.parseProgram('q=Query("read",{})', library);
  for (const attach of [
    s => s.attachToolProvider(), s => s.attachToolProviderWithMinimum(1000n),
    s => s.attachToolProviderAt(0n, 1000n),
  ]) {
    using s = new wasm.RuntimeSession(library);
    let calls = 0;
    const drive = () => s.setToolProvider({read: () => ++calls});
    attach(s);
    kind(drive, 'RefusedWrite');
    using initial = s.replaceProgram(program, undefined, undefined, false);
    kind(drive, 'RefusedWrite');
    using update = s.takeQueryUpdate();
    using receipt = update.callAt(0n);
    kind(drive, 'RefusedWrite');
    expect(receipt.pending).toBe(true);
    s.completeToolCall(receipt, '{"value":7}');
    using manual = s.evaluate();
    expect(JSON.parse(manual.querySnapshotJson).q).toBe(7);
    expect(calls).toBe(0);

    s.setToolProvider(null);
    kind(() => s.attachToolProviderWithMinimum(0n), 'LimitExceeded');
    drive();
    kind(() => attach(s), 'RefusedWrite');
    kind(() => s.completeToolCall(receipt, '{"value":0}'), 'RefusedWrite');
    using started = s.evaluate();
    await nextToolTurn();
    using done = s.evaluate();
    expect(JSON.parse(done.querySnapshotJson).q).toBe(1);
    s.detachToolProvider();
    attach(s);
    using again = s.evaluate();
    using wanted = s.takeQueryUpdate();
    using last = wanted.callAt(0n);
    expect(last.pending).toBe(true);
    s.detachToolProvider();
    drive();
    kind(() => s.takeQueryUpdate(), 'RefusedWrite');
    using restarted = s.evaluate();
    await nextToolTurn();
    using finished = s.evaluate();
    expect(JSON.parse(finished.querySnapshotJson).q).toBe(2);
  }
});

test.each(['await', 'then', 'getter'])('%s writes yield to a real timer scheduled before dispatch', async mode => {
  using lib = new wasm.ComponentLibrary('{"$defs":{}}');
  using program = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', lib);
  using s = new wasm.RuntimeSession(lib);
  let calls = 0;
  const write = n => {
    if (++calls < 64) s.set('$n', String(n + 1));
    return n;
  };
  const providers = {
    await: async ({n}) => { await null; return write(n); },
    then: ({n}) => Promise.resolve(n).then(write),
    getter: ({n}) => ({get value() { return write(n); }}),
  };
  s.setToolProvider({read: providers[mode]});
  const timer = new Promise<number>(resolve => setTimeout(() => resolve(calls), 0));
  using out = s.replaceProgram(program, undefined, undefined, false);
  expect(await timer).toBe(1);
});

test('a rerun requested during a pending call waits for a host task after completion', async () => {
  using lib = new wasm.ComponentLibrary('{"$defs":{}}');
  using s = new wasm.RuntimeSession(lib);
  using program = wasm.parseProgram('q=Query("read",{})', lib);
  const first = Promise.withResolvers<number>();
  let calls = 0;
  s.setToolProvider({read: () => ++calls === 1 ? first.promise : 2});
  using out = s.replaceProgram(program, undefined, undefined, false);
  s.retryFailedQueries();
  await nextToolTurn();
  const atWake = new Promise<number>(resolve => setTimeout(() => resolve(calls), 0));
  first.resolve(1);
  expect(await atWake).toBe(1);
  await nextToolTurn();
  expect(calls).toBe(2);
  using done = s.evaluate();
  expect(JSON.parse(done.querySnapshotJson).q).toBe(2);
});

test('two thousand async dispatch rounds release their tickets and reuse WebAssembly memory', async () => {
  const pkg = loadWith(`
    const start = openuiStartTool;
    openuiStartTool = function(driver, call) {
      start(driver, call);
      const meter = driver.session.meter;
      meter.peak = Math.max(meter.peak, driver.pending.size);
      if (meter.round++ % 128 === 0) {
        for (const ticket of driver.pending) meter.samples.push(new WeakRef(ticket));
        meter.pages.push(wasm.memory.buffer.byteLength);
      }
    };`);
  using lib = new pkg.ComponentLibrary('{"$defs":{}}');
  using s = new pkg.RuntimeSession(lib);
  using program = pkg.parseProgram('$n=0\nq=Query("read",{n:$n})', lib);
  s.meter = {peak: 0, round: 0, samples: [], pages: []};
  const done = Promise.withResolvers<void>();
  const rounds = 2048;
  let calls = 0;
  s.setToolProvider({read: async ({n}) => {
    await null;
    if (++calls < rounds) s.set('$n', String(n + 1));
    else done.resolve();
    return n;
  }});
  const timeout = setTimeout(() => done.reject(Error('dispatch deadline')), 12_000);
  try {
    using out = s.replaceProgram(program, undefined, undefined, false);
    await done.promise;
    await nextToolTurn();
    using final = s.evaluate();
    expect(JSON.parse(final.querySnapshotJson).q).toBe(rounds - 1);
    expect(s.meter.round).toBe(rounds);
    expect(s.meter.peak).toBe(1);
    expect(new Set(s.meter.pages.slice(1)).size).toBe(1);
    Bun.gc(true);
    expect(s.meter.samples.filter(sample => sample.deref())).toHaveLength(0);
  } finally { clearTimeout(timeout); }
}, 15_000);

test('a tiny result limit reaches onError once per failed registration episode', async () => {
  using limits = new wasm.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 64n;
  limits.evaluation = evaluation;
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  using crowded = wasm.parseProgram(Array.from({length: 8}, (_, i) => 'q' + i + '=Query("read",{})').join('\n'), library);
  using empty = wasm.parseProgram('', library);
  const errors = [];
  s.setToolProvider({read() { throw Error('offline'); }}, {onError: error => errors.push(error)});
  for (let episode = 1; episode <= 2; episode++) {
    using out = s.replaceProgram(crowded, undefined, undefined, false);
    await nextToolTurn();
    using draw = s.evaluate();
    expect(draw.queryEvaluationError?.kind).toBe('LimitExceeded');
    expect(errors).toHaveLength(episode);
    expect(errors.at(-1)).toBeInstanceOf(Error);
    expect(errors.at(-1).kind).toBe('LimitExceeded');
    expect(errors.at(-1).message).toBe(draw.queryEvaluationError.message);
    using clear = s.replaceProgram(empty, undefined, undefined, false);
    using clean = s.evaluate();
    expect(clean.queryEvaluationError).toBeNull();
  }
});

test('unexpected delivery failures retry only a pending receipt and detach if recovery also fails', async () => {
  const pkg = loadWith(`
    const pending = Object.getOwnPropertyDescriptor(WantedToolCall.prototype, 'pending').get;
    const complete = RuntimeSession.prototype.completeToolCall;
    RuntimeSession.prototype.completeToolCall = function(call, json, terminal) {
      this.attempts.push({outcome: JSON.parse(json), terminal, pending: pending.call(call)});
      if (this.mode === 'always' || this.mode === 'fallbackAfter' || this.attempts.length === 1) {
        if (this.mode === 'after' || (this.mode === 'fallbackAfter' && this.attempts.length === 2)) complete.call(this, call, json, terminal);
        throw 'openui:Worker\\ninjected delivery failure';
      }
      return complete.call(this, call, json, terminal);
    };`);
  using lib = new pkg.ComponentLibrary('{"$defs":{}}');
  using program = pkg.parseProgram('q=Query("read",{})', lib);
  for (const mode of ['before', 'after', 'always', 'fallbackAfter']) {
    using s = new pkg.RuntimeSession(lib);
    s.mode = mode;
    s.attempts = [];
    const errors = [];
    s.setToolProvider({read: () => 7}, {onError: e => errors.push([e.kind, e.message])});
    using out = s.replaceProgram(program, undefined, undefined, false);
    await nextToolTurn();
    using draw = s.evaluate();
    const first = {outcome: {value: 7}, terminal: true, pending: true};
    const fallback = {outcome: {failed: 'Tool result delivery failed'}, terminal: true, pending: true};
    expect(s.attempts).toEqual(mode === 'after' ? [first] : [first, fallback]);
    expect(errors).toEqual(Array(mode === 'always' || mode === 'fallbackAfter' ? 2 : 1).fill(['Worker', 'injected delivery failure']));
    if (mode === 'always') {
      expect(draw.querySnapshotJson).toBeUndefined();
      s.attachToolProvider();
    } else {
      kind(() => s.takeQueryUpdate(), 'RefusedWrite');
      const snapshot = JSON.parse(draw.querySnapshotJson);
      expect(snapshot.__openui_loading).toEqual([]);
      if (mode === 'after') expect(snapshot.q).toBe(7);
      else expect(snapshot.__openui_errors[0]).toMatchObject({
        code: 'tool-error', message: 'Query "read" failed: Tool result delivery failed',
      });
    }
  }
});

test('a late Cloud script delivery leaves the settled edit visible and loading ends', async () => {
  using lib = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  using s = new wasm.RuntimeSession(lib);
  const firstCall = Promise.withResolvers<string>();
  let calls = 0;
  s.setToolProvider({cloud: () => ++calls === 1 ? firstCall.promise : 'edited'});
  const raw = 'q=Query("cloud",{},0)\nroot=Box(q)\n]]>openui:scripts {"count":1}\n[{"name":"cloud","code":"A"}]';
  try {
    using first = s.setResponse(raw, false).evaluation;
    using partial = s.setResponse(raw.replace('"A"', '"B"'), true).evaluation;
    expect(calls).toBe(1);
    using edit = s.setResponse(raw.replace('"A"', '"B"'), false).evaluation;
    await nextToolTurn();
    expect(calls).toBe(2);
    firstCall.resolve('old');
    await nextToolTurn();
    using result = s.evaluate();
    expect([JSON.parse(result.json).root.props.value, result.isQueryLoading]).toEqual(['edited', false]);
  } finally { firstCall.resolve('old'); }
});
