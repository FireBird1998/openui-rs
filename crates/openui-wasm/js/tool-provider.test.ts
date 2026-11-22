import { expect, test } from 'bun:test';
import { runInNewContext } from 'node:vm';
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

// Every provider shape that can write to its own session from a call. Each
// write makes the session want one more call.
const writingShapes = {
  await: write => ({read: async ({n}) => { await null; return write(n); }}),
  then: write => ({read: ({n}) => Promise.resolve(n).then(write)}),
  sync: write => ({read: ({n}) => write(n)}),
  getter: write => ({read: ({n}) => ({get value() { return write(n); }})}),
  toJSON: write => ({read: ({n}) => ({toJSON() { return write(n); }})}),
  thenable: write => ({read: ({n}) => ({then(resolve) { resolve(write(n)); }})}),
  mcp: write => ({callTool: async ({arguments: {n}}) => ({structuredContent: write(n)})}),
};
// The yield's task source per host: setImmediate (Bun, Node), else a
// MessageChannel (browsers), else a zero-delay timer.
const yieldHosts = {
  setImmediate: '',
  MessageChannel: 'var setImmediate;',
  setTimeout: 'var setImmediate, MessageChannel;',
};
const spin = (ms: number) => { const end = performance.now() + ms; while (performance.now() < end); };

function chain(pkg, shape, write) {
  const lib = new pkg.ComponentLibrary('{"$defs":{}}');
  const program = pkg.parseProgram('$n=0\nq=Query("read",{n:$n})', lib);
  const s = new pkg.RuntimeSession(lib);
  s.setToolProvider(writingShapes[shape](n => write(s, n)));
  return {s, start() { s.replaceProgram(program, undefined, undefined, false).free(); },
    [Symbol.dispose]() { s.free(); program.free(); lib.free(); }};
}

test.each(Object.keys(writingShapes))('a timer due before each %s round runs before the next call', async shape => {
  for (const host of ['setImmediate', 'setTimeout']) {
    const pkg = loadWith(yieldHosts[host]);
    const rounds = 12, seen = [], done = Promise.withResolvers<void>();
    let calls = 0;
    using c = chain(pkg, shape, (s, n) => {
      if (++calls < rounds) {
        setTimeout(() => seen.push(calls), 0);
        spin(2); // the timer is due before the driver's next task
        s.set('$n', String(n + 1));
      } else done.resolve();
      return n;
    });
    setTimeout(() => seen.push(calls), 0);
    spin(2);
    c.start();
    await done.promise;
    await nextToolTurn();
    expect([host, seen]).toEqual([host, Array.from({length: rounds}, (_, i) => Math.max(i, 1))]);
  }
});

test.each(Object.keys(yieldHosts))('the %s yield is a task: a microtask chain never sees the next call', async host => {
  const pkg = loadWith(yieldHosts[host]);
  for (const shape of Object.keys(writingShapes)) {
    let calls = 0, seen;
    const done = Promise.withResolvers<void>();
    using c = chain(pkg, shape, (s, n) => {
      if (++calls === 1) {
        let p = Promise.resolve();
        for (let i = 0; i < 2000; i++) p = p.then(() => {});
        p.then(() => { seen = calls; });
      }
      if (calls < 3) s.set('$n', String(n + 1));
      else done.resolve();
      return n;
    });
    c.start();
    await done.promise;
    expect([shape, seen]).toEqual([shape, 1]);
  }
});

test.each(Object.keys(yieldHosts))('200 dependent calls through the %s yield finish well under 100 ms', async host => {
  const pkg = loadWith(yieldHosts[host]);
  for (const shape of Object.keys(writingShapes)) {
    const done = Promise.withResolvers<void>();
    let calls = 0;
    using c = chain(pkg, shape, (s, n) => {
      if (++calls < 200) s.set('$n', String(n + 1));
      else done.resolve();
      return n;
    });
    const started = performance.now();
    c.start();
    await done.promise;
    const elapsed = performance.now() - started;
    if (host !== 'setTimeout') expect([shape, elapsed < 100]).toEqual([shape, true]);
    else expect(elapsed).toBeGreaterThan(100); // the fallback still waits a timer turn per round
  }
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
  spin(2); // the timer is due before the completion posts its turn
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

test('terminal delivery keeps the admitted source when evaluation of its reader is cut', async () => {
  const pkg = loadWith(`
    const nameOf = Object.getOwnPropertyDescriptor(WantedToolCall.prototype, 'toolName').get;
    const pending = Object.getOwnPropertyDescriptor(WantedToolCall.prototype, 'pending').get;
    const complete = RuntimeSession.prototype.completeToolCall;
    RuntimeSession.prototype.completeToolCall = function(call, json, terminal) {
      const name = nameOf.call(call);
      const value = complete.call(this, call, json, terminal);
      this.deliveries.push([name, terminal, pending.call(call)]);
      return value;
    };`);
  using limits = new pkg.RuntimeLimits();
  using evaluation = limits.evaluation;
  evaluation.resultSize = 4096n;
  limits.evaluation = evaluation;
  using lib = new pkg.ComponentLibrary('{"$defs":{}}');
  using s = pkg.RuntimeSession.withLimits(lib, undefined, limits);
  using program = pkg.parseProgram('$n=0\ns=Query("source",{n:$n})\nr=Query("reader",{x:s,y:s+s})', lib);
  s.deliveries = [];
  const inputs = [], errors = [], large = 'x'.repeat(3000);
  s.setToolProvider({
    source: ({n}) => n ? large : undefined,
    reader: ({x}) => { inputs.push(x); return 'ready'; },
  }, {onError: e => errors.push([e.kind, e.message])});
  using initial = s.replaceProgram(program, undefined, undefined, false);
  expect(inputs).toEqual([]);
  await nextToolTurn();
  await nextToolTurn();
  expect(inputs).toEqual([null]);
  s.set('$n', '1');
  await nextToolTurn();
  using drew = s.evaluate();
  const snapshot = JSON.parse(drew.querySnapshotJson);
  expect(snapshot.s).toBe(large);
  expect(snapshot.r).toBeNull();
  expect(snapshot.__openui_loading).toEqual([]);
  expect(snapshot.__openui_errors).toMatchObject([{code: 'reader-limit', statementId: 'r'}]);
  expect(errors).toEqual([['LimitExceeded', drew.queryEvaluationError.message]]);
  expect(inputs).toEqual([null]);
  expect(s.deliveries).toEqual([['source', true, false], ['reader', true, false], ['source', true, false]]);
});

test('registration and a released completion each report a bounded reader cycle once', async () => {
  const pkg = loadWith(`
    const args = Object.getOwnPropertyDescriptor(WantedToolCall.prototype, 'argumentsJson').get;
    const pending = Object.getOwnPropertyDescriptor(WantedToolCall.prototype, 'pending').get;
    const complete = RuntimeSession.prototype.completeToolCall;
    RuntimeSession.prototype.completeToolCall = function(call, json, terminal) {
      if (!pending.call(call)) this.releasedArgs.push(args.call(call));
      return complete.call(this, call, json, terminal);
    };`);
  using lib = new pkg.ComponentLibrary('{"$defs":{}}');
  using s = new pkg.RuntimeSession(lib);
  s.releasedArgs = [];
  const anchors = '\na1=Query("A",{v:1})\nb0=Query("B",{v:0})';
  using seed = pkg.parseProgram('a=Query("A",{v:0})\nb=Query("B",{v:1})' + anchors + '\ngone=Query("gone")\nold=Query("old")', lib);
  using cycle = pkg.parseProgram('a=Query("A",{v:b})\nb=Query("B",{v:a})' + anchors +
    '\nholdA=Query("A",{v:0})\nholdB=Query("B",{v:1})', lib);
  const errors = [];
  const late = Promise.withResolvers<object>();
  const revoked = Promise.withResolvers<object>();
  s.setToolProvider({A: ({v}) => 1-v, B: ({v}) => v, gone: () => late.promise, old: () => revoked.promise},
    {onError: e => errors.push([e.kind, e.message])});
  using init = s.replaceProgram(seed, undefined, undefined, false);
  await nextToolTurn();
  expect(errors).toEqual([]);
  using edit = s.replaceProgram(cycle, undefined, undefined, false);
  using redraw = s.evaluate();
  await nextToolTurn();
  const cut = ['LimitExceeded', 'Query readers were still changing at the reader round limit.'];
  expect(errors).toEqual([cut]);
  let reads = 0;
  late.resolve({get value() { reads++; return 42; }});
  await nextToolTurn();
  expect(reads).toBe(0);
  expect(errors).toEqual([cut, cut]);
  expect(s.releasedArgs).toStrictEqual([undefined]);
  s.pauseQueries();
  s.resumeQueries();
  await nextToolTurn();
  const before = errors.length;
  revoked.reject(Error('old provider failure'));
  await nextToolTurn();
  expect(errors).toHaveLength(before);
  expect(s.releasedArgs).toStrictEqual([undefined]);
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

// The value a query publishes for one function-map result, or its error.
async function published(result: () => unknown, mcp = false) {
  using lib = new wasm.ComponentLibrary('{"$defs":{}}');
  using s = new wasm.RuntimeSession(lib);
  using program = wasm.parseProgram('q=Query("read",{})', lib);
  s.setToolProvider(mcp ? {callTool: () => ({structuredContent: result()})} : {read: result});
  using out = s.replaceProgram(program, undefined, undefined, false);
  await nextToolTurn();
  using draw = s.evaluate();
  const snapshot = JSON.parse(draw.querySnapshotJson);
  if (snapshot.__openui_errors.length) {
    expect(snapshot.__openui_errors[0].code).toBe('tool-error');
    return snapshot.__openui_errors[0].message;
  }
  return snapshot.q;
}
const cannot = what => `Query "read" failed: Tool result contains ${what}, which JSON cannot represent`;

test.each([
  ['Date with a replaced prototype', () => Object.setPrototypeOf(new Date(123), {}), {}],
  ['Number with a replaced prototype', () => Object.setPrototypeOf(new Number(5), {}), cannot('a Number')],
  ['Map with a replaced prototype', () => Object.setPrototypeOf(new Map([['k', 7]]), {}), {}],
  ['typed array with a replaced prototype', () => Object.setPrototypeOf(new Uint8Array(0), {}), cannot('a Uint8Array')],
  ['Map with a null prototype', () => Object.setPrototypeOf(new Map(), null), {}],
  ['typed array with a null prototype', () => Object.setPrototypeOf(new Uint8Array(0), null), cannot('a Uint8Array')],
  ['ordinary Date', () => new Date(123), '1970-01-01T00:00:00.123Z'],
  ['ordinary Map', () => new Map([['k', 7]]), cannot('a Map')],
  ['ordinary typed array', () => new Uint8Array(0), cannot('a Uint8Array')],
] as const)('query publication for %s', async (_label, result, expected) => {
  for (const mcp of [false, true]) expect(await published(result, mcp)).toEqual(expected);
});

test('a null-prototype Number whose valueOf returns NaN fails after one conversion', async () => {
  for (const mcp of [false, true]) {
    let reads = 0, calls = 0;
    const number = Object.defineProperty(Object.setPrototypeOf(new Number(5), null), 'valueOf', {
      get() { reads++; return () => { calls++; return NaN; }; },
    });
    expect(await published(() => number, mcp)).toBe(cannot('a Number'));
    expect([reads, calls]).toEqual([1, 1]);
  }
});

test('a finite replacement conversion still runs once and a typed array keeps its intrinsic name', async () => {
  let calls = 0;
  const number = Object.setPrototypeOf(new Number(5), {valueOf() { calls++; return 7; }});
  expect(await published(() => number)).toBe(7);
  expect(calls).toBe(1);
  const prototype = {get [Symbol.toStringTag]() { throw Error('unexpected tag lookup'); }};
  expect(await published(() => Object.setPrototypeOf(new Int16Array([3]), prototype))).toBe(cannot('an Int16Array'));
});

test('a Map or Date on its original prototype in another realm keeps its JSON projection', async () => {
  const values = runInNewContext(`
    Object.freeze(Object);
    delete Map.prototype.constructor;
    [new Map([['k', 7]]), Object.assign(new Date(0), {toJSON: null})]
  `);
  expect(await published(() => values)).toEqual([{}, {toJSON: null}]);
});

test('a view or non-finite Number from another realm fails by name on its original prototype', async () => {
  const values = runInNewContext(`[[new Uint8Array(0), 'a Uint8Array'], [new Int16Array([3]), 'an Int16Array'],
    [new DataView(new ArrayBuffer(2)), 'a DataView'], [new Number(NaN), 'a Number'], [new Number(-Infinity), 'a Number']]`);
  for (const [value, what] of values) {
    for (const mcp of [false, true]) expect(await published(() => [value], mcp)).toBe(cannot(what));
  }
});

test('a Number on a prototype taken from another realm fails when its conversion is not finite', async () => {
  // A plain object from the other realm, as an iframe would supply. Its
  // inherited conversion yields "[object Number]", hence NaN, even for 5.
  for (const number of [NaN, Infinity, 5]) {
    const value = Object.setPrototypeOf(new Number(number), runInNewContext('({})'));
    for (const mcp of [false, true]) expect(await published(() => [value], mcp)).toBe(cannot('a Number'));
  }
  const finite = Object.assign(Object.setPrototypeOf(new Number(NaN), runInNewContext('({})')), {valueOf: () => 7});
  for (const mcp of [false, true]) expect(await published(() => [finite], mcp)).toEqual([7]);
});

test('a DataView fails by name on a replaced chain, local or foreign', async () => {
  const chains = [() => ({}), () => null, () => Object.create(Object.create(null)), () => runInNewContext('({})')];
  for (const realm of [false, true]) {
    for (const chain of chains) {
      const view = realm ? runInNewContext('new DataView(new ArrayBuffer(2))') : new DataView(new ArrayBuffer(2));
      Object.setPrototypeOf(view, chain());
      for (const mcp of [false, true]) expect(await published(() => [view], mcp)).toBe(cannot('a DataView'));
    }
  }
});

test('a shared foreign Map publishes its fields before and after its prototype changes', async () => {
  const map = runInNewContext('new Map([[1, 2]])');
  const result = [map, map];
  Object.defineProperty(result, 1, {get() { return Object.setPrototypeOf(map, Object.prototype); }});
  expect(await published(() => result)).toEqual([{}, {}]);
});

test('collection entries stay invisible after replacing the prototype with an ordinary object or null', async () => {
  const key = {};
  for (const proto of [{}, Object.prototype, null]) {
    const values = [new Map([[key, 1]]), new Set([key]), new WeakMap([[key, 1]]), new WeakSet([key])];
    for (const value of values) Object.setPrototypeOf(value, proto);
    for (const mcp of [false, true]) expect(await published(() => values, mcp)).toEqual([{}, {}, {}, {}]);
  }
});

test('own and inherited tags cannot impersonate a built-in or observe projection', async () => {
  let reads = 0;
  for (const own of [true, false]) {
    for (const descriptor of [{value: 'Date'}, {value: 'Number'}, {get() { reads++; throw Error('tag read'); }}]) {
      const proto = Object.create(null);
      const record = Object.create(proto);
      const date = Object.setPrototypeOf(new Date(123), proto);
      const number = Object.assign(Object.setPrototypeOf(new Number(5), proto), {valueOf: () => NaN});
      for (const target of own ? [record, date, number] : [proto]) {
        Object.defineProperty(target, Symbol.toStringTag, descriptor);
      }
      for (const mcp of [false, true]) {
        expect(await published(() => [record, date], mcp)).toEqual([{}, {}]);
        expect(await published(() => number, mcp)).toBe(cannot('a Number'));
      }
    }
  }
  expect(reads).toBe(0);
});

test('a Proxy on the value or prototype sees only the existing JSON projection operations', async () => {
  for (const mcp of [false, true]) {
    for (const wrapValue of [true, false]) {
      const trace = [];
      const proxy = new Proxy({}, {
        get(target, key, receiver) { trace.push(['get', key]); return Reflect.get(target, key, receiver); },
        getOwnPropertyDescriptor(target, key) {
          trace.push(['descriptor', key]);
          return Reflect.getOwnPropertyDescriptor(target, key);
        },
        getPrototypeOf(target) { trace.push(['prototype']); return Reflect.getPrototypeOf(target); },
        ownKeys(target) { trace.push(['keys']); return Reflect.ownKeys(target); },
        has(target, key) { trace.push(['has', key]); return Reflect.has(target, key); },
      });
      // Nest the value so promise resolution does not read its 'then'.
      expect(await published(() => [wrapValue ? proxy : Object.create(proxy)], mcp)).toEqual([{}]);
      expect(trace).toEqual(wrapValue
        ? [['get', 'toJSON'], ['descriptor', 'length'], ['prototype'], ['keys']]
        : [['get', 'toJSON'], ['prototype']]);
    }
    const {proxy, revoke} = Proxy.revocable({}, {});
    revoke();
    expect(await published(() => [proxy], mcp)).toMatch(/^Query "read" failed: .+revoked/i);
  }
});

test('a null-rooted replacement chain refuses without a constructor or tag lookup on a Proxy', async () => {
  for (const mcp of [false, true]) {
    const keys = [];
    const root = new Proxy(Object.create(null), {
      getOwnPropertyDescriptor(target, key) { keys.push(key); return Reflect.getOwnPropertyDescriptor(target, key); },
      get(target, key, receiver) { keys.push(key); return Reflect.get(target, key, receiver); },
    });
    const proto = Object.create(root);
    const number = Object.assign(Object.setPrototypeOf(new Number(5), proto), {valueOf: () => NaN});
    const typed = Object.setPrototypeOf(new Uint8Array([7]), proto);
    expect(await published(() => [number], mcp)).toBe(cannot('a Number'));
    expect(await published(() => [typed], mcp)).toBe(cannot('a Uint8Array'));
    expect(keys).toEqual(['toJSON', Symbol.toPrimitive, 'toJSON']);

    // A direct Proxy prototype behaves the same way.
    Object.setPrototypeOf(number, root);
    Object.setPrototypeOf(typed, root);
    keys.length = 0;
    expect(await published(() => [number], mcp)).toBe(cannot('a Number'));
    expect(await published(() => [typed], mcp)).toBe(cannot('a Uint8Array'));
    expect(keys).toEqual(['toJSON', Symbol.toPrimitive, 'toJSON']);
  }
});

test('a shared foreign typed array is refused after its prototype changes to a local record', async () => {
  const typed = runInNewContext('new Uint8Array([7])');
  const result = [typed, typed];
  Object.defineProperty(result, 1, {get() { return Object.setPrototypeOf(typed, Object.prototype); }});
  expect(await published(() => result)).toBe(cannot('a Uint8Array'));
});

test('foreign and null-prototype Dates keep the ordinary conversion or their enumerable fields', async () => {
  const values = runInNewContext(`[new Date(123), Object.setPrototypeOf(new Date(123), null),
    Object.assign(Object.setPrototypeOf(new Date(123), {}), {label: 'date'})]`);
  for (const mcp of [false, true]) {
    expect(await published(() => values, mcp)).toEqual(['1970-01-01T00:00:00.123Z', {}, {label: 'date'}]);
  }
});

test('results JSON would lose or change in kind fail, in every disguise the driver can see', async () => {
  class Bag extends Map {}
  const invalid = new Date(NaN);
  for (const [result, what] of [
    [() => new Bag([[1, 2]]), 'a Map'],
    [() => ({a: [new Set()]}), 'a Set'],
    [() => ({a: Object(Symbol('s'))}), 'a symbol'],
    [() => [Object(1n)], 'a BigInt'],
    [() => ({a: new Number(-Infinity)}), '-Infinity'],
    [() => [invalid], 'an invalid Date'],
    [() => invalid, 'an invalid Date'],
    [() => ({held: {get at() { return 1; }, at2: invalid}}), 'an invalid Date'],
    [() => new Int8Array(1), 'an Int8Array'],
    [() => ({b: new ArrayBuffer(2)}), 'an ArrayBuffer'],
    [() => ({v: new DataView(new ArrayBuffer(2))}), 'a DataView'],
    [() => { const b = new ArrayBuffer(2), v = new DataView(b); b.transfer(); return [v]; }, 'a DataView'],
    [() => ({at: Object.assign(new Date(0), {toJSON: undefined})}), 'a Date'],
    [() => ({at: Object.assign(new Date(0), {valueOf: () => NaN})}), 'an invalid Date'],
  ] as const) {
    expect(await published(result)).toBe(cannot(what));
  }
});

test('a result keeps JSON.stringify where an author chose a conversion or JSON keeps the value', async () => {
  class Point { x = 1; y = 2; }
  const named = Object.assign(new Map([[1, 2]]), {toJSON: () => 'map'});
  const invalid = Object.assign(new Date(NaN), {toJSON: () => 'unknown'});
  const bare = Object.assign(Object.create(null), {a: 1});
  const fake = Object.assign(Object.create(Map.prototype), {a: 1});
  expect(await published(() => ({named, invalid, point: new Point(), bare, fake,
    boxed: [new Number(3), new String('s'), new Boolean(false)], gone: undefined, [Symbol('k')]: 1})))
    .toEqual({named: 'map', invalid: 'unknown', point: {x: 1, y: 2}, bare: {a: 1}, fake: {a: 1},
      boxed: [3, 's', false]});
});

test('text that JSON.stringify writes is charged to the transport budget each time it appears', async () => {
  const text = 'x'.repeat(1 << 20);
  const disguised = Object.setPrototypeOf(new String('a'), {toString() { return text; }});
  for (const chunk of [new String(text), JSON.rawJSON(JSON.stringify(text))]) {
    expect(await published(() => Array(16).fill(chunk))).toBe('Query "read" failed: tool outcome size limit');
  }
  // A disguised boxed string publishes its own value, never its toString.
  expect(await published(() => Array(16).fill(disguised))).toEqual(Array(16).fill('a'));
});

// A session whose provider writes $n + 1 until $n reaches `until`.
function chained(pkg, until, options = {}, provider = (s, calls) => ({read: ({n}) => {
  calls.push(n);
  if (n < until) s.set('$n', String(n + 1));
  return n;
}})) {
  const lib = new pkg.ComponentLibrary('{"$defs":{}}');
  const s = new pkg.RuntimeSession(lib);
  const program = pkg.parseProgram('$n=0\nq=Query("read",{n:$n})', lib);
  const calls = [], errors = [];
  s.setToolProvider(provider(s, calls), {onError: e => errors.push(e.message), ...options(s)});
  return {s, calls, errors, start() { s.replaceProgram(program, undefined, undefined, false).free(); },
    [Symbol.dispose]() { s.free(); program.free(); lib.free(); }};
}
const turns = async (count: number) => { for (let i = 0; i < count; i++) await nextToolTurn(); };

test('a refused first or second setImmediate hop falls back to a timer and the chain goes on', async () => {
  for (const refused of [1, 2]) {
    const pkg = loadWith(`var hops = 0, setImmediate = callback => {
      if (++hops === ${refused}) throw Error('task queue refused');
      return globalThis.setImmediate(callback);
    };`);
    using c = chained(pkg, 3, () => ({}));
    c.start();
    await turns(6);
    expect([refused, c.calls, c.errors]).toEqual([refused, [0, 1, 2, 3], []]);
    c.s.set('$n', '10');
    await turns(2);
    expect(c.calls).toEqual([0, 1, 2, 3, 10]);
  }
});

test('a runaway provider on a host whose setImmediate always refuses still lets a timer in', async () => {
  const pkg = loadWith(`var setImmediate = () => { throw Error('task queue refused'); };`);
  using c = chained(pkg, Infinity, () => ({}));
  const atTimer = new Promise<number>(resolve => setTimeout(() => resolve(c.calls.length), 0));
  spin(2);
  c.start();
  expect(await atTimer).toBe(1);
  c.s.setToolProvider(null);
  expect(c.errors).toEqual([]);
});

// No task source left: every refusal used to release the turn, and an onError
// that writes drained at once, so the next completion was a microtask again.
test.each([1, 2])('when every task source refuses hop %i, the driver reports once and holds no loop', async hop => {
  const pkg = loadWith(`var hops = 0, MessageChannel, setImmediate = callback => {
    if (++hops >= ${hop}) throw Error('setImmediate refused');
    return globalThis.setImmediate(callback);
  }, setTimeout = () => { throw Error('setTimeout refused'); };`);
  let n = 0;
  const reported = [];
  using c = chained(pkg, Infinity, s => ({onError: e => { reported.push(e.message); s.set('$n', String(++n)); }}),
    (s, calls) => ({read: async ({n}) => { calls.push(n); await null; return n; }}));
  const atTimer = new Promise<number>(resolve => setTimeout(() => resolve(c.calls.length), 0));
  spin(2);
  c.start();
  expect(await atTimer).toBeLessThanOrEqual(2);
  await turns(3);
  expect(reported).toEqual(['setTimeout refused']);
  // The write made inside onError waits for the next session operation,
  // which drains once and dispatches the wanted call.
  const before = c.calls.length;
  using draw = c.s.evaluate();
  await turns(3);
  expect(c.calls.length).toBe(before + 1);
  expect(reported).toEqual(['setTimeout refused']);
});


// Session operations that cancel or replace the driver, run from provider
// code while a result is read or settles, or from a driver callback. Each
// used to be able to dispatch at once from inside that code, so calls ran
// back to back in microtasks while the event loop waited.
test.each(['pause and resume', 'detach and reattach', 'replace the provider'])(
  'a timer due before dispatch still runs when provider code or a callback calls %s', async disturbance => {
  const seen = {};
  for (const place of ['toJSON', 'getter', 'then', 'onChange']) {
    using lib = new wasm.ComponentLibrary('{"$defs":{}}');
    using program = wasm.parseProgram('q=Query("read",{})', lib);
    using s = new wasm.RuntimeSession(lib);
    let calls = 0;
    const disturb = () => {
      if (calls >= 300) return;
      if (disturbance === 'pause and resume') { s.pauseQueries(); s.resumeQueries(); }
      else if (disturbance === 'detach and reattach') { s.setToolProvider(null); s.setToolProvider(provider, options); }
      else s.setToolProvider(provider, options);
    };
    const results = {
      toJSON: () => ({toJSON() { disturb(); return 1; }}),
      getter: () => ({get value() { disturb(); return 1; }}),
      then: () => ({then(resolve) { disturb(); resolve(1); }}),
      onChange: () => 1,
    };
    const provider = {read: () => { calls++; return results[place](); }};
    const options = place === 'onChange' ? {onChange: disturb} : {};
    s.setToolProvider(provider, options);
    const atTimer = new Promise<number>(resolve => setTimeout(() => resolve(calls), 0));
    spin(2);
    using out = s.replaceProgram(program, undefined, undefined, false);
    seen[place] = await atTimer;
    s.setToolProvider(null);
  }
  for (const place in seen) expect([place, seen[place] <= 3]).toEqual([place, true]);
});

// A boxed string can sit on any prototype. Whatever its prototype's
// conversions return, the output is the string's own value or a tool error,
// and text that reaches the output is charged before Rust sees it.
test('a boxed string over any prototype publishes its own value or a tool error, never uncharged text', async () => {
  const text = 'x'.repeat(2 << 20);
  const typed = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array,
    Float32Array, Float64Array, BigInt64Array, BigUint64Array, ...(typeof Float16Array === 'function' ? [Float16Array] : [])];
  const bases = [Map, Set, Date, ArrayBuffer, DataView, Number, Boolean, Symbol, BigInt, RegExp, Error, Promise,
    Array, Function, String, Object, ...typed].map(type => [type.name, type.prototype]);
  bases.push(['TypedArray', Object.getPrototypeOf(Uint8Array.prototype)], ['null', null]);
  // The string's own value, or a tool error from the driver. Rust's own
  // size refusal means uncharged text crossed the boundary.
  const allowed = out => JSON.stringify(out) === JSON.stringify(Array(8).fill('a')) ||
    (typeof out === 'string' && out.startsWith('Query "read" failed: ') && !out.includes('result size limit'));
  const outcomes = new Set();
  const before = process.memoryUsage().rss;
  for (const [name, base] of bases) {
    for (const viaProxy of [false, true]) {
      for (const withToJSON of [false, true]) {
        const proto = Object.create(base);
        for (const key of ['toString', 'valueOf', Symbol.toPrimitive, ...(withToJSON ? ['toJSON'] : [])]) {
          Object.defineProperty(proto, key, {value: () => text});
        }
        const boxed = Object.setPrototypeOf(new String('a'), viaProxy ? new Proxy(proto, {}) : proto);
        const out = await published(() => Array(8).fill(boxed));
        const label = `${name}${viaProxy ? ' via Proxy' : ''}${withToJSON ? ' with toJSON' : ''}`;
        if (!allowed(out)) outcomes.add(label + ': ' + String(JSON.stringify(out)).slice(0, 80));
        // A date's toISOString is a conversion too.
        const date = Object.assign(new Date(0), {toISOString: () => boxed});
        const dated = await published(() => Array(8).fill(date));
        if (!allowed(dated)) {
          outcomes.add(label + ' from toISOString: ' + String(JSON.stringify(dated)).slice(0, 80));
        }
      }
    }
  }
  expect([...outcomes]).toEqual([]);
  Bun.gc(true);
  expect(process.memoryUsage().rss - before).toBeLessThan(256 << 20);
}, 30_000);

test('a host whose setTimeout throws reports the refresh timer and keeps the session usable', async () => {
  const pkg = loadWith(`var timersRefused = true, setTimeout = (callback, delay) => {
    if (timersRefused) throw Error('timers refused');
    return globalThis.setTimeout(callback, delay);
  };`);
  using lib = new pkg.ComponentLibrary('{"$defs":{}}');
  using s = new pkg.RuntimeSession(lib);
  using program = pkg.parseProgram('$x=0\nq=Query("read",{},null,0.05)', lib);
  const errors = [];
  let calls = 0;
  s.setToolProvider({read: () => ++calls}, {minimumMs: 10n, onError: e => errors.push(e.message)});
  using out = s.replaceProgram(program, undefined, undefined, false);
  expect(s.set('$x', '1')).toBe(true);
  await nextToolTurn();
  expect(errors.length).toBeGreaterThan(0);
  expect(new Set(errors)).toEqual(new Set(['timers refused']));
  // No deadline is kept without a timer, so each later drain tries again
  // and reports again instead of throwing out of the write.
  const reported = errors.length;
  expect(s.set('$x', '2')).toBe(true);
  await nextToolTurn();
  expect(errors.length).toBeGreaterThan(reported);
  using draw = s.evaluate();
  expect(JSON.parse(draw.querySnapshotJson).q).toBe(calls);
});

// Each disturbance cancels or replaces the driver from provider code or a
// callback, and is then followed by an operation that makes the session want
// a call. A due timer must run within a few calls, and the chain must still
// finish once real time passes: no fix may trade the loop for a stuck driver.
const disturbances = {
  'pause, resume': s => { s.pauseQueries(); s.resumeQueries(); },
  'detach, reattach': (s, p, o) => { s.setToolProvider(null); s.setToolProvider(p, o); },
  'pause, detach, reattach': (s, p, o) => { s.pauseQueries(); s.setToolProvider(null); s.setToolProvider(p, o); },
};
const followUps = {
  evaluate: s => { s.evaluate().free(); },
  write: (s, n) => { s.set('$n', String(n)); },
};
const disturbedPlaces = ['toJSON', 'getter', 'then', 'onChange', 'after await', 'deep microtask after return'];
const cases = Object.keys(disturbances).flatMap(d => Object.keys(followUps).map(f => [d, f]));

test.each(cases)('%s then %s from provider code holds no loop and finishes', async (disturbance, followUp) => {
  const cap = 50, seen = {};
  for (const place of disturbedPlaces) {
    using lib = new wasm.ComponentLibrary('{"$defs":{}}');
    using program = wasm.parseProgram('$n=0\nq=Query("read",{n:$n})', lib);
    using s = new wasm.RuntimeSession(lib);
    let calls = 0;
    const disturb = () => {
      if (calls >= cap) return;
      disturbances[disturbance](s, provider, options);
      followUps[followUp](s, calls);
    };
    const results = {
      toJSON: () => ({toJSON() { disturb(); return 1; }}),
      getter: () => ({get value() { disturb(); return 1; }}),
      then: () => ({then(resolve) { disturb(); resolve(1); }}),
      onChange: () => 1,
      'after await': async () => { await null; disturb(); return 1; },
      'deep microtask after return': () => { Promise.resolve().then().then().then().then(disturb); return 1; },
    };
    const provider = {read: () => { calls++; return results[place](); }};
    const options = place === 'onChange' ? {onChange: disturb} : {};
    s.setToolProvider(provider, options);
    const atTimer = new Promise<number>(resolve => setTimeout(() => resolve(calls), 0));
    spin(2);
    using out = s.replaceProgram(program, undefined, undefined, false);
    const first = await atTimer;
    for (const end = performance.now() + 500; calls < cap && performance.now() < end;) await nextToolTurn();
    // Pausing and resuming after a delivery leaves nothing to fetch, so that
    // chain ends early; the driver must then be idle, not stuck.
    let alive = calls >= cap;
    if (!alive) {
      using draw = s.evaluate();
      const idle = JSON.parse(draw.querySnapshotJson).__openui_loading.length === 0;
      const before = calls;
      s.set('$n', '999');
      await turns(2);
      alive = idle && calls === before + 1;
    }
    seen[place] = [first <= 3, alive];
    s.setToolProvider(null);
  }
  expect(seen).toEqual(Object.fromEntries(disturbedPlaces.map(place => [place, [true, true]])));
}, 15_000);

test('pause and resume from provider code on a host that refuses every task holds no loop, and recovers', async () => {
  const pkg = loadWith(`var setImmediate, MessageChannel, setTimeout = (callback, delay) => {
    if (globalThis.__openuiRefuseTimers) throw Error('timers refused');
    return globalThis.setTimeout(callback, delay);
  };`);
  globalThis.__openuiRefuseTimers = true;
  try {
    using lib = new pkg.ComponentLibrary('{"$defs":{}}');
    using program = pkg.parseProgram('q=Query("read",{})', lib);
    using s = new pkg.RuntimeSession(lib);
    let calls = 0;
    const errors = [];
    s.setToolProvider({read: async () => { calls++; await null; if (calls < 50) { s.pauseQueries(); s.resumeQueries(); } return 1; }},
      {onError: e => errors.push(e.message)});
    const atTimer = new Promise<number>(resolve => setTimeout(() => resolve(calls), 0));
    spin(2);
    using out = s.replaceProgram(program, undefined, undefined, false);
    expect(await atTimer).toBeLessThanOrEqual(3);
    expect(errors).toEqual(['timers refused']);
    // Once a task source works again, the next operation recovers the chain.
    globalThis.__openuiRefuseTimers = false;
    s.evaluate().free();
    for (const end = performance.now() + 500; calls < 50 && performance.now() < end;) await nextToolTurn();
    expect(calls).toBe(50);
  } finally { delete globalThis.__openuiRefuseTimers; }
});

test('a host whose clearTimeout throws reports it and the write still returns', async () => {
  const pkg = loadWith(`var clearTimeout = () => { throw Error('clear refused'); };`);
  using lib = new pkg.ComponentLibrary('{"$defs":{}}');
  using s = new pkg.RuntimeSession(lib);
  using program = pkg.parseProgram('$x=0\nq=Query("read",{x:$x},null,0.05)', lib);
  const errors = [];
  let calls = 0;
  s.setToolProvider({read: () => ++calls}, {minimumMs: 10n, onError: e => errors.push(e.message)});
  using out = s.replaceProgram(program, undefined, undefined, false);
  expect(s.set('$x', '1')).toBe(true);
  await turns(2);
  expect(errors.length).toBeGreaterThan(0);
  expect(new Set(errors)).toEqual(new Set(['clear refused']));
  using draw = s.evaluate();
  expect(JSON.parse(draw.querySnapshotJson).q).toBe(calls);
});

test.each([false, true])('a Date conversion getter is read once and its chosen conversion decides the result (MCP %s)', async mcp => {
  for (const builtin of [true, false]) {
    let reads = 0, conversions = 0, propertyReads = 0;
    const date = Object.defineProperty(new Date(NaN), 'toJSON', { get() {
      reads++;
      const custom = function (key) {
        conversions++;
        expect(this).toBe(date);
        expect(key).toBe('at');
        return null;
      };
      return (reads === 1 ? builtin : !builtin) ? Date.prototype.toJSON : custom;
    } });
    for (const getter of [false, true]) {
      reads = conversions = propertyReads = 0;
      const output = await published(() => getter ? { get at() { propertyReads++; return date; } } : {at: date}, mcp);
      expect(output).toEqual(builtin ? cannot('an invalid Date') : { at: null });
      expect([propertyReads, reads, conversions]).toEqual([getter ? 1 : 0, 1, builtin ? 0 : 1]);
    }
  }
});

test('Proxy descriptors and values are observed once per key during tool result projection', async () => {
  const descriptors = [], gets = [];
  let enumerations = 0;
  const date = new Date(NaN);
  const result = new Proxy({length: null, at: date}, {
    ownKeys(target) { enumerations++; return Reflect.ownKeys(target); },
    getOwnPropertyDescriptor(target, key) {
      const repeated = descriptors.includes(key);
      descriptors.push(key);
      return repeated ? {configurable: true, enumerable: true, value: null} : Reflect.getOwnPropertyDescriptor(target, key);
    },
    get(target, key) { gets.push(key); return Reflect.get(target, key); },
  });
  expect(await published(() => ({result}))).toBe(cannot('an invalid Date'));
  expect(enumerations).toBe(1);
  expect(descriptors.sort()).toEqual(['at', 'length']);
  expect(gets.sort()).toEqual(['at', 'length', 'toJSON']);
});

test('a stale busy count recovers after a host turn without re-deferring forever', async () => {
  const host = { tasks: [] as Array<() => void>, leaveBusy: (_session: unknown) => {} };
  const pkg = loadWith(`
    var setImmediate = callback => host.tasks.push(callback);
    host.leaveBusy = session => openuiEnter(session);
  `, host);
  using c = chained(pkg, 0, () => ({}));
  host.leaveBusy(c.s);
  c.start();
  // Bound the fake loop so a stuck driver fails the assertion.
  for (let hop = 0; hop < 8 && !c.calls.length; hop++) {
    host.tasks.shift()?.();
    await null;
  }
  expect(c.calls).toEqual([0]);
});

test('a fake task run inside provider code waits for the stack and another host task', async () => {
  const host = { tasks: [] as Array<() => void> };
  const pkg = loadWith('var setImmediate = callback => host.tasks.push(callback);', host);
  let reposted;
  using c = chained(pkg, 1, () => ({}), (s, calls) => ({read: ({n}) => {
    calls.push(n);
    if (n === 0) {
      s.set('$n', '1');
      // Run the two hops of the reserved turn on this provider's stack.
      host.tasks.shift()();
      host.tasks.shift()();
      reposted = host.tasks.length;
    }
    return n;
  }}));
  c.start();
  expect([c.calls, reposted]).toEqual([[0], 0]);
  await null;
  expect(c.calls).toEqual([0]);
  for (let hop = 0; hop < 8 && c.calls.length === 1; hop++) {
    host.tasks.shift()?.();
    await null;
  }
  expect(c.calls).toEqual([0, 1]);
});

test('an enumerable toJSON getter is read once when its value is data', async () => {
  let reads = 0;
  const result = { get toJSON() { return ++reads; }, empty: null };
  expect(await published(() => result)).toEqual({toJSON: 1, empty: null});
  expect(reads).toBe(1);
});

test('a shared conversion receives the key of each serialized occurrence', async () => {
  const keys = [];
  const shared = { toJSON(key) { keys.push(key); return key; } };
  expect(await published(() => [shared, {again: shared}])).toEqual(['0', {again: 'again'}]);
  expect(keys).toEqual(['0', 'again']);
});

test('buffered projection preserves escaped field names and observes a shared record again', () => {
  const host = {} as { project(value: unknown): string };
  loadWith('host.project = openuiToolJson;', host);
  // These lone surrogates would form a pair if field names were concatenated.
  const adjacent = {['\ud800']: 1, ['\udc00']: 2};
  expect(host.project(adjacent)).toBe(JSON.stringify(adjacent));

  const wide = Object.fromEntries(Array.from({length: 3000}, (_, i) => ['field' + i, i]));
  wide['"\\\n'] = 3000;
  const shared = {n: 1};
  const result = [shared, wide, {get omitted() { shared.n++; return undefined; }}, shared];
  expect(host.project(result)).toBe(JSON.stringify([{n: 1}, wide, {}, {n: 2}]));
});
