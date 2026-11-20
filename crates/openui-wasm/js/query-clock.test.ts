import { expect, test } from 'bun:test';
import { kind, loadWith, wasm } from './support';

// These tests own every timer. A host without setImmediate or MessageChannel
// also takes the driver's turn between rounds from setTimeout.
const timerHost = 'var setImmediate, MessageChannel;';

test('the driver can rearm one guarded deadline and consume a paused call once', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using s = new wasm.RuntimeSession(library);
  kind(() => s.attachToolProviderWithMinimum(0n), 'LimitExceeded');
  s.attachToolProviderWithMinimum(40n);
  using p = wasm.parseProgram('q=Query("read",{},null,0.01)', library);
  using out = s.replaceProgram(p, undefined, undefined, false);
  using first = s.takeQueryUpdate();
  expect(first.nextRefreshDueMs).toBe(40n);
  using call = first.callAt(0n);
  s.completeToolCall(call, '{"value":3}');
  s.advanceQueryClock(40n);
  using tick = s.takeQueryUpdate();
  expect(tick.callCount).toBe(1n);
  expect(tick.nextRefreshDueMs).toBe(80n);
  kind(() => s.advanceQueryClock(39n), 'LimitExceeded');
  s.pauseQueries();
  using paused = s.takeQueryUpdate();
  expect(paused.nextRefreshDueMs).toBeUndefined();
  using late = tick.callAt(0n);
  s.completeToolCall(late, '{"failed":"offline"}');
  kind(() => s.completeToolCall(late, '{"value":3}'), 'InvalidToolCall');
  s.advanceQueryClock(400n);
  s.resumeQueries();
  using resumed = s.takeQueryUpdate();
  expect(resumed.callCount).toBe(0n);
  expect(resumed.nextRefreshDueMs).toBe(440n);
});

test('attaching and resuming at host time schedule a full period through the package', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using s = new wasm.RuntimeSession(library);
  s.attachToolProviderAt(8000n, 125n);
  using p = wasm.parseProgram('q=Query("read",{},null,0.01)', library);
  using out = s.replaceProgram(p, undefined, undefined, false);
  kind(() => s.attachToolProviderWithMinimum(0n), 'RefusedWrite');
  using first = s.takeQueryUpdate();
  expect(first.nextRefreshDueMs).toBe(8125n);
  s.pauseQueries();
  s.resumeQueriesAt(16000n);
  using resumed = s.takeQueryUpdate();
  expect(resumed.nextRefreshDueMs).toBe(16125n);
});

test('one refresh timer follows absolute deadlines across delayed registration, late wakes, pause and resume', async () => {
  let now = 100, nextId = 0, calls = 0;
  const timers = new Map<number, {delay: number, wake(): void}>();
  const { spyOn } = await import('bun:test');
  using time = spyOn(performance, 'now').mockImplementation(() => now);
  using setTimer = spyOn(globalThis, 'setTimeout').mockImplementation((wake, delay) => {
    timers.set(++nextId, {delay, wake});
    return nextId;
  });
  using clearTimer = spyOn(globalThis, 'clearTimeout').mockImplementation(id => { timers.delete(id); });
  const timer = () => { expect(timers.size).toBe(1); return [...timers.values()][0]; };
  const wake = () => {
    const callback = timer().wake;
    timers.clear(); // platform one-shot timer
    callback();
  };
  const pkg = loadWith(timerHost);
  using library = new pkg.ComponentLibrary('{"$defs":{}}');
  using session = new pkg.RuntimeSession(library);
  using p = pkg.parseProgram('q=Query("read",{},null,0.01)', library);
  const hanging = Promise.withResolvers<number>();
  session.setToolProvider({ read() { return ++calls === 1 ? hanging.promise : calls; } }, {minimumMs:50n});
  now = 450;
  using out = session.replaceProgram(p, undefined, undefined, false);
  expect(calls).toBe(1);
  expect(timer().delay).toBe(50);
  const firstId = nextId;
  now = 490;
  using redraw = session.evaluate();
  expect(nextId).toBe(firstId);
  now = 627;
  wake(); // late while the first call is still in flight
  expect(calls).toBe(1);
  expect(timer().delay).toBe(50);
  hanging.resolve(3);
  await Promise.resolve();
  const turn = [...timers.entries()].find(([, timer]) => timer.delay === 0);
  expect(turn).toBeDefined();
  timers.delete(turn[0]);
  turn[1].wake();
  now = 700;
  using overdue = session.evaluate(); // a draw before delivery of the queued callback
  expect(calls).toBe(1);
  wake();
  expect(calls).toBe(2);
  expect(timer().delay).toBe(50);
  session.pauseQueries();
  expect(timers.size).toBe(0);
  now = 5000;
  session.resumeQueries();
  expect(timer().delay).toBe(50);
  session.free();
  expect(timers.size).toBe(0);
});

test('a late real Bun wake waits a full period before the next driven refresh', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{}}');
  using session = new wasm.RuntimeSession(library);
  using p = wasm.parseProgram('q=Query("read",{},null,0.1)', library);
  const times: number[] = [];
  const done = Promise.withResolvers<void>();
  session.setToolProvider({ read() {
    times.push(performance.now());
    if (times.length === 3) done.resolve();
    return 1;
  } }, {minimumMs:1n});
  using out = session.replaceProgram(p, undefined, undefined, false);
  await Promise.resolve();
  Bun.sleepSync(250);
  const watchdog = setTimeout(() => done.reject(Error('three tool calls timed out')), 3000);
  try { await done.promise; }
  finally { clearTimeout(watchdog); }
  expect(times[1] - times[0]).toBeGreaterThanOrEqual(248);
  expect(times[2] - times[1]).toBeGreaterThanOrEqual(97);
});

test('a refused timer wake reports its error and the next draw re-arms the session deadline', async () => {
  const { spyOn } = await import('bun:test');
  const pkg = loadWith(timerHost + `
    const advance = RuntimeSession.prototype.advanceQueryClock;
    RuntimeSession.prototype.advanceQueryClock = function(now) {
      if (!this.refused) { this.refused = true; throw Error('timer wake'); }
      return advance.call(this, now);
    };`);
  let now = 0, wake;
  const wakes = [];
  using time = spyOn(performance, 'now').mockImplementation(() => now);
  using timer = spyOn(globalThis, 'setTimeout').mockImplementation(fn => { wakes.push(fn); wake = fn; return wakes.length; });
  using clear = spyOn(globalThis, 'clearTimeout').mockImplementation(() => {});
  using library = new pkg.ComponentLibrary('{"$defs":{}}');
  using session = new pkg.RuntimeSession(library);
  using p = pkg.parseProgram('q=Query("read",{},null,0.1)', library);
  let calls = 0;
  const errors = [];
  session.setToolProvider({read: () => ++calls}, {minimumMs:1n, onError: e => errors.push(e.message)});
  using out = session.replaceProgram(p, undefined, undefined, false);
  await Promise.resolve();
  expect(wakes.length).toBe(2);
  wakes[1](); // completion drain
  now = 100;
  wakes[0](); // refresh deadline
  expect(errors).toEqual(['timer wake']);
  const first = wake;
  using draw = session.evaluate();
  expect(wake).not.toBe(first);
  wake();
  expect(calls).toBe(2);
});
