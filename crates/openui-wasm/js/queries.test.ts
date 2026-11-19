import { expect, test } from 'bun:test';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';
import { kind, loadWith, wasm } from './support';

test('the guarded package dispatches writes and accepts only a live call from the same session', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Arg":{"params":{"type":"object","properties":{}}}}}');
  using s = new wasm.RuntimeSession(library);
  using other = new wasm.RuntimeSession(library);
  let callbacks = 0;
  s.registerFunction('Arg', () => {
    callbacks++;
    kind(() => s.detachToolProvider(), 'Worker');
    return '7';
  });
  s.setReferenceValues('{"q":1}');
  kind(() => s.attachToolProvider(), 'ReferenceConflict');
  s.setReferenceValues('{}'); s.attachToolProvider(); other.attachToolProvider();
  kind(() => s.setReferenceValues('{}'), 'ReferenceConflict');
  using p = wasm.parseProgram('$n=1\nq=Query("search",{n:$n,x:@Arg()},@Arg())\nroot=Box(q)',library);
  using blocked = s.replaceProgram(p,undefined,undefined,true,{ streaming: true });
  using blockedTree = blocked.evaluation;
  expect(blockedTree.queryEvaluationError).toBeNull();
  using waiting = s.takeQueryUpdate();
  expect(waiting.callCount).toBe(0n);
  expect(callbacks).toBe(0);
  kind(() => s.evaluate(true), 'InvalidJson');
  kind(() => s.evaluate({}), 'InvalidJson');
  using draw = s.evaluate({ get streaming() {
    kind(() => s.detachToolProvider(), 'Worker');
    return false;
  } });
  using effects = s.takeQueryUpdate();
  expect(effects.callCount).toBe(1n);
  expect(effects.callAt(10n)).toBeUndefined();
  using call = effects.callAt(0n);
  expect(call.toolName).toBe('search');
  expect(call.argumentsJson).toBe('{"n":1,"x":7}');
  expect(JSON.parse(draw.querySnapshotJson).__openui_loading).toEqual(['q']);
  expect(effects.evaluationError).toBeNull();
  kind(() => s.completeToolCall(Object.create(Object.getPrototypeOf(call)), '{"value":1}'), 'InvalidHandle');
  kind(() => other.completeToolCall(call, '{"value":1}'), 'InvalidToolCall');
  kind(() => s.completeToolCall(call, '{}'), 'InvalidJson');
  s.completeToolCall(call, '{"mcpResponseJson":"{\\"structuredContent\\":42}"}');
  using done = s.takeQueryUpdate();
  expect(done.changed).toBe(true);
  kind(() => s.completeToolCall(call, '{"value":1}'), 'InvalidToolCall');
  expect(callbacks).toBe(2);
  using live = s.evaluate({ streaming: false });
  expect(callbacks).toBe(2);
  expect(JSON.parse(live.json).root.props.value).toBe(42);
  expect(s.set('$n','2')).toBe(true);
  using onSet = s.takeQueryUpdate();
  using setCall = onSet.callAt(0n);
  expect(setCall.argumentsJson).toBe('{"n":2,"x":7}');
  expect(s.writeBinding('{"__reactive":"assign","target":"$n","expr":{"k":"StateRef","n":"$value"}}','3')).toBe(true);
  using onBinding = s.takeQueryUpdate();
  using bindingCall = onBinding.callAt(0n);
  expect(bindingCall.argumentsJson).toBe('{"n":3,"x":7}');
  const field = s.setField(undefined,'Field','$n','4',true);
  expect(field.changed && field.saveRequested).toBe(true);
  using onField = s.takeQueryUpdate();
  using fieldCall = onField.callAt(0n);
  expect(fieldCall.argumentsJson).toBe('{"n":4,"x":7}');
  s.detachToolProvider();
  kind(() => s.completeToolCall(fieldCall,'{"value":5}'), 'InvalidToolCall');
  s.setReferenceValues('{"q":"supplied"}');
  using plain = s.evaluate();
  expect(plain.querySnapshotJson).toBeUndefined();
  expect(JSON.parse(plain.json).root.props.value).toBe('supplied');
});

test('a query limit reaches the driver and the displayed tree until a successful registration', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  using limits = new wasm.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 2048n;
  limits.evaluation = evaluation;
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  using p = wasm.parseProgram('$n="a"\nq=Query("read",{term:$n+$n},"waiting")\nroot=Box(q)', library);
  s.attachToolProvider();
  using first = s.replaceProgram(p, undefined, undefined, true);
  using wanted = s.takeQueryUpdate();
  using call = wanted.callAt(0n);
  s.completeToolCall(call, '{"value":"visible"}');
  using completed = s.takeQueryUpdate();
  expect(completed.changed).toBe(true);
  expect(s.set('$n', JSON.stringify('x'.repeat(1500)))).toBe(true);
  using update = s.takeQueryUpdate();
  expect(update.evaluationError.kind).toBe('LimitExceeded');
  using draw = s.evaluate();
  expect(draw.queryEvaluationError).toEqual(update.evaluationError);
  expect(JSON.parse(draw.json).root.props.value).toBe('visible');
  s.set('$n', '"b"');
  using recovered = s.evaluate();
  expect(recovered.queryEvaluationError).toBeNull();
  using dispatch = s.takeQueryUpdate();
  expect(dispatch.callCount).toBe(1n);
});

test('the driver matches renderer calls, trees, loading, refetching and errors through provider swaps and Retry', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const { transcript } = await import('../../../reference/tool-provider-scenarios');
  const expected = await Bun.file(new URL('../../../conformance/golden/tool-provider.jsonl', import.meta.url)).text();
  const host = wasmHost();
  try { expect(await transcript(host)).toBe(expected); }
  finally { host.dispose(); }
});

test('refused tool results settle as visible failures at the JavaScript and session limits', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const { limitTranscript } = await import('../../../reference/tool-provider-scenarios');
  const expected = await Bun.file(new URL('../../../conformance/divergences/tool-provider-limits.jsonl', import.meta.url)).text();
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 4096n;
  using evaluation = limits.evaluation;
  evaluation.depth = 1024n;
  limits.evaluation = evaluation;
  const host = wasmHost(limits);
  try { expect(await limitTranscript(host)).toBe(expected); }
  finally { host.dispose(); }
});

test('retired calls, paused calls and disposed sessions ignore late fulfillment and rejection', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using program = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', library);
  for (const stop of ['dispose', 'free', 'detach']) {
    using session = new wasm.RuntimeSession(library);
    const calls = [];
    let changes = 0, reads = 0;
    session.setToolProvider({ read: () => new Promise((resolve, reject) => calls.push({resolve, reject})) },
      { onChange: () => changes++ });
    using replace = session.replaceProgram(program, undefined, undefined, false);
    session.set('$n', '1');
    session.set('$n', '2');
    expect(calls.length).toBe(3);
    const before = changes;
    calls[0].resolve({ get value() { reads++; return 1; } });
    calls[1].reject('retired');
    await Bun.sleep(0);
    expect(reads).toBe(0);
    expect(changes).toBe(before);
    session.pauseQueries();
    session.resumeQueries();
    expect(calls.length).toBe(4);
    calls[2].reject('paused');
    if (stop === 'detach') session.setToolProvider(null);
    else session[stop]();
    const stopped = changes;
    calls[3].resolve({ get value() { reads++; return 2; } });
    await Bun.sleep(0);
    expect(reads).toBe(0);
    expect(changes).toBe(stopped);
    if (stop === 'detach') {
      session.setReferenceValues('{"q":42}');
      using draw = session.evaluate();
      expect(draw.querySnapshotJson).toBeUndefined();
    }
  }
});

test('a replacement provider takes future calls while the original promise can still finish', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('q=Query("read",{})', library);
  const first = Promise.withResolvers<string>();
  let changes = 0, newCalls = 0;
  session.setToolProvider({ read: () => first.promise }, { onChange: () => changes++ });
  using replacement = session.replaceProgram(p, undefined, undefined, false);
  session.setToolProvider({ read() { newCalls++; return 'next'; } });
  expect(newCalls).toBe(0);
  const before = changes;
  first.resolve('original');
  await nextToolTurn();
  expect(changes).toBe(before + 1);
  using draw = session.evaluate();
  expect(changes).toBe(before + 1);
  expect(JSON.parse(draw.querySnapshotJson).q).toBe('original');
  session.retryFailedQueries();
  expect(newCalls).toBe(1);
});

test('driver installation checks the receiver, options and reentry before changing providers', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  let invalidReads = 0;
  kind(() => session.setToolProvider.call(Object.create(wasm.RuntimeSession.prototype), {}, {
    get onChange() { invalidReads++; },
  }), 'InvalidHandle');
  expect(invalidReads).toBe(0);
  kind(() => session.setToolProvider(3), 'InvalidJson');
  kind(() => session.setToolProvider({}, {onChange: false}), 'InvalidJson');
  kind(() => session.setToolProvider({}, {minimumMs: 5}), 'LimitExceeded');
  session.setReferenceValues('{"x":0}');
  kind(() => session.setToolProvider({}), 'ReferenceConflict');
  session.setReferenceValues('{}');
  let optionReads = 0;
  session.setToolProvider({}, {get onChange() {
    optionReads++;
    kind(() => session.free(), 'Worker');
    kind(() => session.setToolProvider(null), 'Worker');
    return () => {};
  }});
  expect(optionReads).toBe(1);
  session.setToolProvider({}, {});
  kind(() => session.setToolProvider({}, {minimumMs: 5n}), 'RefusedWrite');
  session.free();
  kind(() => session.setToolProvider({}), 'Disposed');
});

test('a provider can dispose during dispatch and during result serialization without starting or delivering stale work', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using p = wasm.parseProgram('a=Query("first",{})\nb=Query("second",{})', library);
  for (const during of ['dispatch', 'serialization', 'lookup', 'MCP lookup']) {
    using session = new wasm.RuntimeSession(library);
    const calls = [], errors = [];
    let reads = 0;
    const provider = {
      get first() {
        if (during === 'lookup') session.dispose();
        return () => {
          calls.push('first');
          if (during === 'dispatch') session.dispose();
          return { get data() { reads++; session.dispose(); return 42; } };
        };
      },
      second() { calls.push('second'); return null; },
    };
    if (during === 'MCP lookup') {
      let lookups = 0;
      Object.defineProperty(provider, 'callTool', {get() {
        if (++lookups === 2) session.dispose();
        return () => calls.push('MCP');
      }});
    }
    session.setToolProvider(provider, { onError: error => errors.push(error) });
    using out = session.replaceProgram(p, undefined, undefined, false);
    await Bun.sleep(0);
    const expected = {dispatch:['first'], serialization:['first', 'second'], lookup:[], 'MCP lookup':[]};
    expect(calls).toEqual(expected[during]);
    expect(reads).toBe(during === 'serialization' ? 1 : 0);
    expect(errors).toEqual([]);
    kind(() => session.version(), 'Disposed');
  }
});

test('four readers settle with a compact error and recover together on Retry', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const { failureTranscript, sharedProgram } = await import('../../../reference/tool-provider-scenarios');
  const expected = await Bun.file(new URL('../../../conformance/divergences/tool-provider-error-limit.json', import.meta.url)).text();
  using limits = new wasm.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 256n;
  limits.evaluation = evaluation;
  const host = wasmHost(limits, sharedProgram);
  try { expect(await failureTranscript(host)).toBe(expected); }
  finally { host.dispose(); }
});

test('a synchronous write inside a tool dispatches the new call after the provider returns', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using p = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', library);
  using session = new wasm.RuntimeSession(library);
  const calls = [];
  const next = Promise.withResolvers<void>();
  session.setToolProvider({ read({n}) {
    calls.push('enter ' + n);
    if (n === 0) session.set('$n', '1');
    calls.push('leave ' + n);
    if (n === 1) next.resolve();
    return n;
  } });
  using out = session.replaceProgram(p, undefined, undefined, false);
  await next.promise;
  await Promise.resolve();
  expect(calls).toEqual(['enter 0', 'leave 0', 'enter 1', 'leave 1']);
  using draw = session.evaluate();
  expect(JSON.parse(draw.querySnapshotJson).q).toBe(1);
});

test('pause reports cancellation as a failed completion before releasing the outstanding receipt', () => {
  const pkg = loadWith(`
    const complete = RuntimeSession.prototype.completeToolCall;
    RuntimeSession.prototype.completeToolCall = function(call, json) {
      this.reported = JSON.parse(json);
      this.receipt = call;
      return complete.call(this, call, json);
    };`);
  using library = new pkg.ComponentLibrary('{"$defs":{}}');
  using session = new pkg.RuntimeSession(library);
  using p = pkg.parseProgram('q=Query("read",{})', library);
  session.setToolProvider({ read: () => new Promise(() => {}) });
  using output = session.replaceProgram(p, undefined, undefined, false);
  session.pauseQueries();
  expect(session.reported).toEqual({failed:'cancelled'});
  kind(() => session.receipt.pending, 'Disposed');
});

test('withLimits carries the cache cap: a result past it is the query\'s tool-error (#90)', () => {
  using limits = new wasm.RuntimeLimits();
  expect(limits.maxCacheBytes).toBeUndefined();
  limits.maxCacheBytes = 100n;
  expect(limits.maxCacheBytes).toBe(100n);
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  s.attachToolProvider();
  using p = wasm.parseProgram('a=Query("a")\nb=Query("b")\nroot=Box([a,b])', library);
  using replaced = s.replaceProgram(p, undefined, undefined, true);
  using effects = s.takeQueryUpdate();
  using first = effects.callAt(0n);
  using second = effects.callAt(1n);
  s.completeToolCall(first, JSON.stringify({ value: 'x'.repeat(38) }));
  s.completeToolCall(second, JSON.stringify({ value: 'x'.repeat(59) }));
  using draw = s.evaluate();
  const snapshot = JSON.parse(draw.querySnapshotJson);
  expect(snapshot.a).toBe('x'.repeat(38));
  expect(snapshot.__openui_errors[0].message).toBe('Query "b" failed: tool result does not fit the cache limit of 100 bytes');
  kind(() => s.completeToolCall(second, '{"value":1}'), 'InvalidToolCall');
});

test('non-JSON values settle as readable tool errors in function-map and MCP results', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const { nonJsonTranscript } = await import('../../../reference/tool-provider-scenarios');
  const expected = await Bun.file(new URL('../../../conformance/divergences/tool-provider-non-json.jsonl', import.meta.url)).text();
  expect(await nonJsonTranscript(wasmHost)).toBe(expected);
});

test('only the attached driver can advance time, claim effects or complete a call', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', library);
  const calls = [];
  session.setToolProvider({ read({n}) {
    calls.push(n);
    kind(() => session.takeQueryUpdate(), 'RefusedWrite');
    return n;
  } });
  for (const [method, args] of [
    ['sampleQueryClock', [10_000_000n]], ['advanceQueryClock', [BigInt(Date.now())]],
    ['takeQueryUpdate', []], ['completeToolCall', [null, '{}']], ['resumeQueriesAt', [10_000_000n]],
    ['attachToolProviderAt', [10_000_000n, 1000n]], ['attachToolProviderWithMinimum', [1000n]],
    ['attachToolProvider', []],
  ]) kind(() => session[method](...args), 'RefusedWrite');
  using out = session.replaceProgram(p, undefined, undefined, false);
  session.set('$n', '1');
  await Bun.sleep(0);
  expect(calls).toEqual([0, 1]);
  using draw = session.evaluate();
  expect(JSON.parse(draw.querySnapshotJson).q).toBe(1);
  session.setToolProvider(null);
  session.attachToolProviderAt(0n, 1n);
  session.sampleQueryClock(1n);
  session.advanceQueryClock(2n);
  using manual = session.takeQueryUpdate();
  expect(manual.callCount).toBe(0n);
});

test('synchronously writing providers let a real zero-delay timer run between rounds', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', library);
  const cap = 128;
  let calls = 0;
  session.setToolProvider({ read({n}) {
    if (++calls < cap) session.set('$n', String(n + 1));
    return n;
  } });
  const turn = new Promise<number>(resolve => setTimeout(() => resolve(calls), 0));
  using out = session.replaceProgram(p, undefined, undefined, false);
  const atTimer = await turn;
  expect(atTimer).toBeGreaterThan(0);
  expect(atTimer).toBeLessThan(cap);
});

test('the driver sends one terminal outcome even when a provider diagnostic exceeds admission', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const { diagnosticTranscript } = await import('../../../reference/tool-provider-scenarios');
  const expected = await Bun.file(new URL('../../../conformance/divergences/tool-provider-diagnostics.jsonl', import.meta.url)).text();
  const pkg = loadWith(`
    const complete = RuntimeSession.prototype.completeToolCall;
    RuntimeSession.prototype.completeToolCall = function(...args) {
      this.deliveries = (this.deliveries ?? 0) + 1;
      return complete.apply(this, args);
    };`);
  using library = new pkg.ComponentLibrary('{"$defs":{}}');
  using limits = new pkg.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 256n;
  limits.evaluation = evaluation;
  using regular = new wasm.RuntimeLimits();
  using small = regular.evaluation;
  small.resultSize = 256n;
  regular.evaluation = small;
  expect(await diagnosticTranscript(() => wasmHost(regular))).toBe(expected);
  using p = pkg.parseProgram('q=Query("read",{})', library);
  for (const provider of [
    {read() { throw 'x'.repeat(1024); }},
    {['x'.repeat(1024)]: () => 1},
    {read: () => 'x'.repeat(1024)},
    {callTool: () => null},
  ]) {
    using session = pkg.RuntimeSession.withLimits(library, undefined, limits);
    const bindingErrors = [];
    session.setToolProvider(provider, {onError: e => bindingErrors.push(e)});
    using out = session.replaceProgram(p, undefined, undefined, false);
    await Bun.sleep(0);
    using draw = session.evaluate();
    const snapshot = JSON.parse(draw.querySnapshotJson);
    expect(snapshot.__openui_loading).toEqual([]);
    expect(snapshot.__openui_errors[0].code).toBe('tool-error');
    expect(bindingErrors).toEqual([]);
    expect(session.deliveries).toBe(1);
  }
});

test('rejected redraw callbacks reach onError and stay contained when the error callback is cleared', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', library);
  const errors = [], draws = [];
  const provider = {read: ({n}) => n};
  const onChange = async () => {
    draws.push('redraw');
    throw Error('async redraw');
  };
  session.setToolProvider(provider, {onChange, onError: error => errors.push(error.message)});
  using out = session.replaceProgram(p, undefined, undefined, false);
  await nextToolTurn();
  const before = draws.length;
  expect(before).toBeGreaterThan(0);
  expect(errors).toEqual(draws.map(() => 'async redraw'));
  session.setToolProvider(provider, {onChange});
  expect(session.set('$n', '1')).toBe(true);
  await nextToolTurn();
  expect(draws.length).toBeGreaterThan(before);
  expect(errors).toHaveLength(before);
  using draw = session.evaluate();
  expect(JSON.parse(draw.querySnapshotJson).q).toBe(1);
});

test('Retry includes compact and individually listed failures and leaves a healthy query alone', async () => {
  const { sharedProgram } = await import('../../../reference/tool-provider-scenarios');
  using limits = new wasm.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 256n;
  limits.evaluation = evaluation;
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = wasm.RuntimeSession.withLimits(library, undefined, limits);
  using p = wasm.parseProgram(sharedProgram + '\ns=Query("single",{})\nh=Query("healthy",{})', library);
  let healthy = 0;
  const fail = () => { throw Error('offline'); };
  session.setToolProvider({read: fail, single: fail, healthy: () => ++healthy});
  using out = session.replaceProgram(p, undefined, undefined, false);
  await nextToolTurn();
  using before = session.evaluate();
  const errors = JSON.parse(before.querySnapshotJson).__openui_errors;
  expect(errors.map(e => e.statementId)).toContain('s');
  expect(errors.some(e => e.message.startsWith('4 queries failed'))).toBe(true);
  session.setToolProvider({read: () => 1, single: () => 2, healthy: () => ++healthy});
  session.retryFailedQueries();
  await nextToolTurn();
  using draw = session.evaluate();
  const snapshot = JSON.parse(draw.querySnapshotJson);
  expect(snapshot.__openui_errors).toEqual([]);
  expect([snapshot.q0, snapshot.q1, snapshot.q2, snapshot.q3, snapshot.s, snapshot.h]).toEqual([1,1,1,1,2,1]);
  expect(healthy).toBe(1);
});

test('manual completion can correct a refused envelope or mark it terminal and consume the receipt', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  session.attachToolProvider();
  using p = wasm.parseProgram('q=Query("read",{})', library);
  using out = session.replaceProgram(p, undefined, undefined, false);
  using update = session.takeQueryUpdate();
  using call = update.callAt(0n);
  kind(() => session.completeToolCall(call, '{"value":1}', 'yes'), 'InvalidJson');
  kind(() => session.completeToolCall(call, '{}', null), 'InvalidJson');
  expect(call.pending).toBe(true);
  session.completeToolCall(call, '{}', true);
  expect(call.pending).toBe(false);
  kind(() => session.completeToolCall(call, '{"failed":"fallback"}'), 'InvalidToolCall');
  using draw = session.evaluate();
  const snapshot = JSON.parse(draw.querySnapshotJson);
  expect(snapshot.__openui_loading).toEqual([]);
  expect(snapshot.__openui_errors[0].code).toBe('tool-error');
});

test('removing a query during dispatch discards its result before JS projection', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('q=Query("read",{})', library);
  using empty = wasm.parseProgram('', library);
  let reads = 0;
  session.setToolProvider({read() {
    using removed = session.replaceProgram(empty, undefined, undefined, false);
    return { get data() { reads++; return 1; } };
  }});
  using out = session.replaceProgram(p, undefined, undefined, false);
  await Bun.sleep(0);
  using draw = session.evaluate();
  expect(JSON.parse(draw.querySnapshotJson).__openui_loading).toEqual([]);
  expect(reads).toBe(0);
});

test('driven draws match upstream for bundle script revisions and app cache keys', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const corpus = await Bun.file(new URL('../../../conformance/session-bundles.json', import.meta.url)).json();
  const name = 'script-cache-keys';
  const {steps} = corpus.scenarios.find(scenario => scenario.name === name);
  const golden = await Bun.file(new URL('../../../conformance/golden/session-bundles.jsonl', import.meta.url)).text();
  const expected = golden.trimEnd().split('\n').filter(row => JSON.parse(row).name === name);
  expect(expected).toHaveLength(steps.length);
  const {program} = JSON.parse(wasm.parseResponseBundle(steps[0].response, false));

  const host = wasmHost(undefined, program);
  const calls = [], outcomes = [];
  const tool = toolName => args => {
    calls.push({toolName, args});
    const pending = Promise.withResolvers();
    outcomes.push(pending);
    return pending.promise;
  };
  try {
    host.provider({cloud: tool('cloud'), ordinary: tool('ordinary')});
    let state;
    for (const [index, step] of steps.entries()) {
      if ('response' in step) {
        const streaming = step.streaming ?? false;
        state = {streaming, ...JSON.parse(wasm.parseResponseBundle(step.response, streaming))};
      } else if ('extra' in step) {
        host.extra(step.extra, step.values);
      } else {
        outcomes[step.finish].resolve(step.value);
        await nextToolTurn();
      }
      const {tree, snapshot, isQueryLoading, parserError} = host.draw(state);
      await nextToolTurn();
      const row = {name, tree, snapshot, isQueryLoading, parserError, calls: calls.splice(0), events: []};
      expect(JSON.stringify(row)).toBe(expected[index]);
    }
  } finally { host.dispose(); }
});

test('reading a driven snapshot preserves blocked bundle Retry until a bare streaming draw', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const host = wasmHost();
  let calls = 0;
  try {
    host.provider({read() { calls++; throw Error('offline'); }});
    expect(host.draw(false).isQueryLoading).toBe(true);
    await nextToolTurn();
    for (const flags of [{streaming:true}, {complete:false}, {error:'invalid bundle'}]) {
      const state = {streaming:false, isBundle:true, ...flags};
      const draw = host.draw(state);
      expect(draw.isQueryLoading).toBe(state.streaming);
      expect(draw.parserError).toEqual(flags.error
        ? {source:'parser', code:'parse-failed', message:flags.error} : null);
      expect(host.snapshot().__openui_errors[0].code).toBe('tool-error');
      host.retry();
      expect(calls).toBe(1);
    }
    host.provider({read() { calls++; return 'retried'; }});
    host.draw(true);
    host.retry();
    await nextToolTurn();
    expect(calls).toBe(2);
    expect(host.snapshot().q).toBe('retried');
  } finally { host.dispose(); }
});

test('a blocked bundle suppresses actions and Retry for compact errors until the driven session unblocks', async () => {
  const { sharedProgram } = await import('../../../reference/tool-provider-scenarios');
  using limits = new wasm.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 256n;
  limits.evaluation = evaluation;
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"action":{}}}}}');
  using session = wasm.RuntimeSession.withLimits(library, undefined, limits);
  using parsed = wasm.parseProgram(sharedProgram + '\ns=Query("single",{})\nh=Query("healthy",{})\nroot=Box(Action([@Run(q0)]))', library);
  const calls = [];
  const tracked = (name, fn) => () => {
    calls.push(name);
    return fn();
  };
  const fail = () => { throw Error('offline'); };
  session.setToolProvider({
    read: tracked('shared', fail), single: tracked('single', fail), healthy: tracked('healthy', () => 3),
  });
  using initial = session.replaceProgram(parsed, undefined, undefined, true, {streaming:true, isBundle:true});
  using waiting = initial.evaluation;
  expect(waiting.isQueryLoading).toBe(true);
  expect(calls).toEqual([]);
  const settled = {streaming:false, isBundle:true};
  using started = session.evaluate(settled);
  await nextToolTurn();
  expect(calls.splice(0)).toEqual(['shared','single','healthy']);
  using blocked = session.evaluate({...settled, complete:false});
  const failures = JSON.parse(blocked.querySnapshotJson).__openui_errors;
  expect(failures.some(e => e.message.startsWith('4 queries failed'))).toBe(true);
  expect(failures.some(e => e.statementId === 's')).toBe(true);
  session.setToolProvider({
    read: tracked('shared', () => 1), single: tracked('single', () => 2), healthy: tracked('healthy', () => 3),
  });
  using tree = blocked.evaluation;
  using action = session.actionAt(tree, ['props', 'action']);
  using ignored = session.run(action, 'click');
  expect(ignored.events).toEqual([]);
  expect(ignored.failure).toBeNull();
  using defaultClick = session.run(undefined, 'click');
  expect(defaultClick.events).toEqual([]);
  session.retryFailedQueries();
  using unchanged = session.evaluate({...settled, complete:false});
  expect(JSON.parse(unchanged.querySnapshotJson).__openui_errors).toEqual(failures);
  expect(calls).toEqual([]);
  using unblocked = session.evaluate(settled);
  expect(calls).toEqual([]);
  session.retryFailedQueries();
  await nextToolTurn();
  using recovered = session.evaluate(settled);
  const snapshot = JSON.parse(recovered.querySnapshotJson);
  expect(snapshot.__openui_errors).toEqual([]);
  expect([snapshot.q0, snapshot.q1, snapshot.q2, snapshot.q3, snapshot.s, snapshot.h]).toEqual([1,1,1,1,2,3]);
  expect(calls).toEqual(['shared','single','shared']);
  using bare = session.evaluate({streaming:true});
  using liveTree = bare.evaluation;
  using liveAction = session.actionAt(liveTree, ['props', 'action']);
  using run = session.run(liveAction, 'click');
  await nextToolTurn();
  expect(calls.at(-1)).toBe('shared');
  expect(calls.length).toBe(4);
});

test('a snapshot read during onChange uses the new response and a refused draw preserves it', async () => {
  const { wasmHost } = await import('../../../examples/next-compare/scripts/compare-tools.mjs');
  const host = wasmHost();
  let calls = 0, readOnChange = false;
  try {
    host.provider({read: () => ++calls}, {onChange() {
      if (readOnChange) {
        readOnChange = false;
        host.snapshot();
      }
    }});
    readOnChange = true;
    host.draw({streaming:false, isBundle:true, scripts:['read'], scriptRevision:'A'});
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(readOnChange).toBe(false);
    expect(calls).toBe(1);
    kind(() => host.draw({streaming:false, scripts:[1]}), 'InvalidJson');
    expect(host.snapshot().q).toBe(1);
    expect(calls).toBe(1);
  } finally { host.dispose(); }
});
