// #117 fix round: freeing or disposing the session from inside a driver
// callback. Rule (ADR 0014): every failure the session has produced reaches
// the app exactly once, in order, and no callback runs after free() or
// dispose() returns. A free made inside a callback delivers what is left
// before it returns.
import { expect, test } from 'bun:test';
import { wasm } from './support';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';

const SCHEMA = '{"$defs":{"Box":{"properties":{"value":{},"a":{},"b":{},"c":{}}}}}';

function program(count) {
  const names = ['a', 'b', 'c'].slice(0, count);
  const mutations = names.map(n => `${n}=Mutation("${n}")`).join('\n');
  const actions = names.map(n => `Action([@Run(${n}),@Set($n,1)])`).join(',');
  return `$n=0\n${mutations}\nroot=Box($n,${actions})`;
}

// Where the app stops the session: the callback, which one of the 2n
// deliveries, how, and whether the callback then throws.
async function run({ count, phase, inside, at, stop, throws }) {
  using library = new wasm.ComponentLibrary(SCHEMA);
  const s = new wasm.RuntimeSession(library);
  using parsed = wasm.parseProgram(program(count), library);
  const log = [];
  let stopped = false;
  let seen = 0;
  const after = [];
  const callback = name => output => {
    if (stopped) after.push(name);
    if (name === 'error') log.push(['error', output.kind]);
    else log.push(['action', output.failure?.error.kind]);
    if (seen++ === at && name === inside) {
      s[stop]();
      stopped = true;
      if (throws) throw Error('after stop');
    }
  };
  s.setToolProvider({ a: () => new Promise(() => {}), b: () => new Promise(() => {}), c: () => new Promise(() => {}) },
    { onError: callback('error'), onAction: callback('action') });
  s.replaceProgram(parsed, undefined, undefined, true).free();
  for (const prop of ['a', 'b', 'c'].slice(0, count)) {
    using draw = s.evaluate();
    using action = s.actionAt(draw.evaluation, ['props', prop]);
    s.run(action, 'click').free();
  }
  await nextToolTurn();
  s.pauseQueries();
  if (phase === 'flush') {
    // The pause's drain waits for a task; free first.
    s.free();
    stopped = true;
  }
  await nextToolTurn();
  if (!stopped) s.free();
  if (stop === 'dispose') s.free();
  return { log, after, stoppedInside: seen > at };
}

const expected = count => Array.from({ length: count }, () => [['error', 'QueriesPaused'], ['action', 'QueriesPaused']]).flat();

for (const count of [1, 2, 3]) {
  for (const phase of ['drain', 'flush']) {
    for (const stop of ['free', 'dispose']) {
      for (const inside of ['error', 'action']) {
        for (const throws of [false, true]) {
          test(`${count} waiting, ${stop} inside on${inside} during the ${phase}${throws ? ', then throw' : ''}`, async () => {
            for (let at = 0; at < 2 * count; at++) {
              if ((at % 2 === 0) !== (inside === 'error')) continue;
              const { log, after, stoppedInside } = await run({ count, phase, inside, at, stop, throws });
              expect(stoppedInside).toBe(true);
              expect(log).toEqual(expected(count));
              expect(after).toEqual([]);
            }
          });
        }
      }
    }
  }
}

// Fix round 2: segments produced while the free delivers, detach, and no
// app code at all after the outermost free returns.
function three(options = {}) {
  const library = new wasm.ComponentLibrary(SCHEMA);
  const s = new wasm.RuntimeSession(library);
  const parsed = wasm.parseProgram(program(3), library);
  library.free();
  const log = [];
  const tap = { stopped: false, after: [] };
  const never = () => new Promise(() => {});
  s.setToolProvider(options.provider ?? { a: never, b: never, c: never }, {
    onError: e => { if (tap.stopped) tap.after.push('onError'); log.push(['error', e.kind]); return options.onError?.(e); },
    onAction: o => { if (tap.stopped) tap.after.push('onAction'); log.push(['action', o.failure?.error.kind]); return options.onAction?.(o); },
  });
  s.replaceProgram(parsed, undefined, undefined, true).free();
  parsed.free();
  const press = prop => {
    using draw = s.evaluate();
    using action = s.actionAt(draw.evaluation, ['props', prop]);
    s.run(action, 'click').free();
  };
  return { s, log, tap, press };
}

async function settle() {
  for (let i = 0; i < 4; i++) { await nextToolTurn(); await null; await null; }
}

for (const end of ['free', 'dispose', 'detach']) {
  test(`a segment produced by a callback during the ${end} is delivered once`, async () => {
    let first = true;
    const ctx = three({ onAction: () => {
      if (!first) return;
      first = false;
      ctx.s.resumeQueries(); ctx.press('c'); ctx.s.pauseQueries();
    } });
    const { s, log, tap, press } = ctx;
    press('a'); press('b');
    await nextToolTurn();
    s.pauseQueries();
    if (end === 'detach') s.setToolProvider(null); else s[end]();
    tap.stopped = end !== 'detach';
    await settle();
    expect(log).toEqual(expected(3));
    expect(tap.after).toEqual([]);
    if (end === 'detach') {
      using rest = s.takeQueryUpdate();
      expect(rest.actionCount).toBe(0n);
    }
    if (end !== 'free') s.free();
  });
}

// Main's behaviour for values a callback returns, pinned so that a change
// to it is deliberate (ADR 0014): the driver hears a rejection through
// Promise.resolve, a microtask after the callback.
test('a thenable a callback returns is followed as Promise.resolve follows it', async () => {
  for (const shape of ['plain', 'throws', 'rejecting thenable']) {
    const seen = [];
    let inCallback = false;
    const returned = {
      plain: { then() { seen.push(inCallback ? 'sync' : 'later'); } },
      throws: { then() { seen.push(inCallback ? 'sync' : 'later'); throw Error('then threw'); } },
      'rejecting thenable': { then(ok) { seen.push(inCallback ? 'sync' : 'later'); ok({ then(_, bad) { bad(Error('inner rejected')); } }); } },
    }[shape];
    let first = true;
    const ctx = three({
      onAction: () => {
        if (!first) return undefined;
        first = false;
        inCallback = true;
        try { return returned; } finally { inCallback = false; }
      },
      onError: e => { if (e.kind !== 'QueriesPaused') seen.push(['onError', e.message, inCallback ? 'sync' : 'later']); },
    });
    const { s, press } = ctx;
    press('a');
    await nextToolTurn();
    s.pauseQueries();
    await settle();
    const reported = { plain: [], throws: [['onError', 'then threw', 'later']],
      'rejecting thenable': [['onError', 'inner rejected', 'later']] }[shape];
    expect([shape, seen]).toEqual([shape, ['later', ...reported]]);
    s.free();
  }
});

test('a callback promise rejecting after the free runs no app code', async () => {
  let stopped = false, touched = 0;
  const later = Promise.withResolvers();
  const { s, press } = three({ onAction: () => later.promise });
  press('a');
  await nextToolTurn();
  s.pauseQueries();
  s.free();
  stopped = true;
  later.reject({ toString() { if (stopped) touched++; return 'late'; }, get message() { if (stopped) touched++; return 'late'; } });
  await settle();
  expect(touched).toBe(0);
});

// The accepted limit (ADR 0014): JavaScript's own promise machinery may still
// run a provider thenable's `then` from a job queued before the free. It
// cannot deliver, report or throw.
test('a provider thenable may have its then run after a free in the same block, to no effect', async () => {
  let stopped = false, ranAfter = false;
  const ctx = three({ provider: { a: () => ({ then(ok) { if (stopped) ranAfter = true; ok(5); } }) } });
  const { s, log, tap, press } = ctx;
  press('a');
  s.free();
  stopped = true;
  tap.stopped = true;
  const before = log.length;
  await settle();
  expect(ranAfter).toBe(true);
  expect(log.length).toBe(before);
  expect(tap.after).toEqual([]);
});

// The narrowed rule: after the outermost free or dispose returns, no
// callback runs and no provider is invoked, whatever the app does.
test('after the outermost free no callback runs and no provider is invoked', async () => {
  const sequences = ['pause', 'free-inside-onAction', 'pending', 'settling', 'detach-then-free',
    'free-inside-provider', 'free-inside-onChange'];
  for (const sequence of sequences) {
    for (const stop of ['free', 'dispose']) {
      const flag = { stopped: false, after: [] };
      const touch = what => { if (flag.stopped) flag.after.push(what); };
      let end;
      const poison = name => ({
        get value() { return 1; }, toJSON() { return 1; }, toString() { return name; },
      });
      const failing = { toString() { return 'e'; }, get message() { return 'e'; } };
      const ctx = three({
        provider: {
          a: () => { touch('provider a'); if (sequence === 'free-inside-provider') end(); return { then(ok) { ok(poison('a')); } }; },
          b: () => { touch('provider b'); const p = Promise.withResolvers(); setTimeout(() => p.reject(failing), 0); return p.promise; },
          c: () => { touch('provider c'); return poison('c'); },
        },
        onError: () => { touch('onError'); return { then() {} }; },
        onAction: () => {
          touch('onAction');
          if (sequence === 'free-inside-onAction') end();
          return Promise.reject(failing);
        },
      });
      const { s, press } = ctx;
      end = () => {
        if (flag.stopped) return;
        if (sequence === 'detach-then-free') s.setToolProvider(null);
        s[stop]();
        flag.stopped = true;
      };
      if (sequence === 'free-inside-onChange') {
        // Every completion redraws; the first change after the clicks ends it.
        s.setToolProvider({ a: () => ({ then(ok) { ok(1); } }), b: () => 2, c: () => 3 }, { onChange: () => { touch('onChange'); end(); }, onError: () => touch('onError'), onAction: () => touch('onAction') });
      }
      // A free made inside the provider ends the session during a click.
      for (const prop of ['a', 'b', 'c']) {
        if (flag.stopped) break;
        try { press(prop); } catch (error) { if (!flag.stopped) throw error; }
      }
      if (sequence !== 'pending' && !flag.stopped) await nextToolTurn();
      if (sequence !== 'settling' && !flag.stopped) s.pauseQueries();
      end();
      await settle();
      // Clicks and resumes after the end are refused; they call nothing.
      try { press('a'); } catch { /* the session is gone */ }
      await settle();
      expect([sequence, stop, flag.after]).toEqual([sequence, stop, []]);
      if (stop === 'dispose') s.free();
    }
  }
});
