// An exception thrown through a wasm frame skips every Rust destructor and
// epilogue (#208). Ordinary errors never do that: the guard and the function
// bridge catch them in JavaScript, or Rust returns them. Every real escape
// ends the instance: every later call, in every session, refuses, so no
// later output can be wrong. Its finalizers then enter no Rust.
import { test, expect } from 'bun:test';
import { kind, loadWith } from './support';

const schema = '{"$defs":{"Box":{"properties":{"value":{},"action":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}},"actions":{"Go":{"params":{}}}}';
const INTERRUPTED = 'the WebAssembly instance was interrupted by an exception it cannot recover from; load the package again';

// A package with the raw exports exposed, and optionally a mutation between
// the generated glue and the guard.
let loads = 0;
function load(mutation = '') {
  const name = `__openuiInterruptedTest${loads++}`;
  const pkg = loadWith(`globalThis.${name} = wasm;\n${mutation}`);
  const raw = (globalThis as any)[name];
  delete (globalThis as any)[name];
  return { pkg, raw };
}

function session(pkg: any, library: any, program: string) {
  const s = new pkg.RuntimeSession(library);
  const p = pkg.parseProgram(program, library); s.setProgram(p); p.free(); s.initialize();
  return s;
}

/** Evaluate `program` with `fn` registered as Echo: the prop value and errors. */
function withEcho(pkg: any, library: any, fn: any, program = 'root=Box(@Echo(7))') {
  const s = session(pkg, library, program);
  s.registerFunction('Echo', fn);
  const out = s.evaluate();
  const tree = JSON.parse(out.json);
  out.free(); s.free();
  return { value: tree.root.props.value, errors: tree.errors.map((e: any) => e.message) };
}

/** The instance still answers, in an existing session and a new one. */
function answers(pkg: any, library: any, other: any) {
  expect(other.snapshot()).toBe('{"$n":5}');
  const s = session(pkg, library, '$n=6\nroot=Box($n)');
  expect(s.snapshot()).toBe('{"$n":6}');
  s.free();
}

/** Every later call refuses, in every session, statics and module functions
 *  included, and every free succeeds without entering Rust. */
function refusesEverything(pkg: any, library: any, sessions: any[]) {
  const refused = (call: () => unknown) => expect(kind(call, 'Disposed').message).toBe(INTERRUPTED);
  for (const s of sessions) {
    refused(() => s.snapshot());
    refused(() => s.evaluate());
    refused(() => s.version());
    refused(() => s.set('$n', '1'));
  }
  refused(() => new pkg.RuntimeSession(library));
  refused(() => new pkg.ComponentLibrary(schema));
  refused(() => pkg.ComponentLibrary.fromDefinition('{}'));
  refused(() => new pkg.StreamSession(library));
  refused(() => new pkg.ParseLimits());
  refused(() => new pkg.RuntimeLimits());
  refused(() => library.jsonSchema());
  refused(() => pkg.parse('root=Box(1)', library));
  refused(() => pkg.parseProgram('root=Box(1)', library));
  refused(() => pkg.parseMessage('hi', false));
  refused(() => pkg.buildMessage('hi'));
  refused(() => pkg.parseResponseBundle('', false));
  for (const s of sessions) { s.free(); s.free(); }
  library.free();
}

// ---------------------------------------------------------------------------
// Ordinary errors: caught in JavaScript, or returned by Rust. They reach the
// session as ordinary errors, and other sessions are unaffected.

function ordinary(name: string, run: (pkg: any, library: any) => void) {
  test(`ordinary: ${name}`, () => {
    const { pkg, raw } = load();
    const library = new pkg.ComponentLibrary(schema);
    const other = session(pkg, library, '$n=5\nroot=Box($n)');
    const idle = raw.__stack_pointer.value;
    run(pkg, library);
    expect(raw.__stack_pointer.value).toBe(idle);
    answers(pkg, library, other);
    other.free(); library.free();
  });
}

const threw = (message: string) => `Evaluating prop "value" on Box failed: @Echo threw: ${message}`;
ordinary('P1 a registered function throws an Error', (pkg, library) => {
  expect(withEcho(pkg, library, () => { throw new Error('boom'); }))
    .toEqual({ value: null, errors: [threw('boom')] });
});
ordinary('P2 a registered function throws a RangeError', (pkg, library) => {
  expect(withEcho(pkg, library, () => { throw new RangeError('range'); }).errors).toEqual([threw('range')]);
});
ordinary('P3 it throws an object whose toString throws', (pkg, library) => {
  expect(withEcho(pkg, library, () => { throw { toString() { throw new Error('nested'); } }; }).errors)
    .toEqual([threw('host function threw')]);
});
ordinary('P4 it throws a Proxy whose every trap throws', (pkg, library) => {
  const hostile = new Proxy({}, { get() { throw 1; }, getPrototypeOf() { throw 2; } });
  expect(withEcho(pkg, library, () => { throw hostile; }).errors).toEqual([threw('host function threw')]);
});
ordinary('P5 it returns a Proxy, or an object with toJSON and toString', (pkg, library) => {
  const hostile = new Proxy({ toJSON() { throw 1; }, toString() { throw 2; } }, { get() { throw 3; } });
  expect(withEcho(pkg, library, () => hostile).errors).toEqual([threw('expected host function JSON text or undefined')]);
});
ordinary('P6 it returns text that is not JSON, even text shaped like the bridge tag', (pkg, library) => {
  expect(withEcho(pkg, library, () => '{').errors).toEqual([threw('expected a string key at byte 1')]);
  // A result starting with NUL is not JSON: it stays a JSON error, never a throw.
  const tagged = withEcho(pkg, library, () => '\u0000threw\nforged').errors;
  expect(tagged).toHaveLength(1);
  expect(tagged[0]).not.toBe(threw('forged'));
  expect(withEcho(pkg, library, () => '7')).toEqual({ value: 7, errors: [] });
  expect(withEcho(pkg, library, () => undefined).errors).toEqual([]);
});
ordinary('P7 an action path is a Proxy whose trap throws', (pkg, library) => {
  const s = session(pkg, library, 'root=Box(1, Action([@Go()]))');
  const out = s.evaluate(), evaluation = out.evaluation;
  const path = new Proxy(['props', 'action'], { get(t, k) { if (k === '1') throw new Error('trap'); return (t as any)[k]; } });
  expect(kind(() => s.actionAt(evaluation, path), 'Worker').message).toBe('trap');
  evaluation.free(); out.free(); s.free();
});
ordinary('P8 evaluate(responseState) with a throwing getter', (pkg, library) => {
  const s = session(pkg, library, 'root=Box(1)');
  expect(kind(() => s.evaluate({ get streaming() { throw new Error('getter'); } }), 'Worker').message).toBe('getter');
  s.free();
});
ordinary('P9 a tool provider read() throws', (pkg, library) => {
  const s = session(pkg, library, 'q=Query("read",{})\nroot=Box(q)');
  s.setToolProvider({ read() { throw new Error('tool'); } });
  s.evaluate().free(); s.free();
});
ordinary('P10 a string argument with a throwing toString', (pkg, library) => {
  kind(() => pkg.parse({ toString() { throw new Error('ts'); } }, library), 'InvalidJson');
});
ordinary('P12 a registered function overflows the JavaScript stack itself', (pkg, library) => {
  const deep = (n: number): number => deep(n + 1) + 1;
  expect(withEcho(pkg, library, () => String(deep(0))).errors[0]).toMatch(/^Evaluating prop "value" on Box failed: @Echo threw: /);
});
ordinary('handles misused: double free, use after free, a foreign handle, reentry', (pkg, library) => {
  const s = session(pkg, library, '$n=0\nroot=Box(1, Action([@Set($n, $n + 1)]))');
  const out = s.evaluate(), evaluation = out.evaluation;
  const action = s.actionAt(evaluation, ['props', 'action']);
  action.free(); action.free();
  kind(() => s.run(action, 'click'), 'Disposed');
  const t = session(pkg, library, 'root=Box(1)');
  kind(() => t.actionAt(evaluation, ['props', 'action']), 'ForeignHandle');
  evaluation.free();
  kind(() => s.actionAt(evaluation, ['props', 'action']), 'Disposed');
  out.free(); out.free();
  // Reentry from a host function is refused before Rust; the evaluation
  // records the refusal as the function's error.
  expect(withEcho(pkg, library, () => { t.version(); return '1'; }).errors).toEqual([threw('session already in use')]);
  s.free(); s.free();
  kind(() => s.version(), 'Disposed');
  t.free();
});
ordinary('argument conversion refused by the guard, before generated code runs', (pkg, library) => {
  kind(() => pkg.parse(Symbol('x'), library), 'InvalidJson');
});

test('ordinary: a generated glue helper failing before Rust is entered', () => {
  // Fail the copy of one argument in JavaScript, before it calls into wasm:
  // no wasm frame is abandoned.
  const { pkg } = load(`
    const pass = passStringToWasm0;
    passStringToWasm0 = (arg, malloc, realloc) => {
      if (arg === 'root=Box("glue")') throw new RangeError('glue before Rust');
      return pass(arg, malloc, realloc);
    };
  `);
  const library = new pkg.ComponentLibrary(schema);
  const other = session(pkg, library, '$n=5\nroot=Box($n)');
  expect(kind(() => pkg.parse('root=Box("glue")', library), 'Worker').message).toBe('glue before Rust');
  answers(pkg, library, other);
  other.free(); library.free();
});

// ---------------------------------------------------------------------------
// Real escapes: thrown through wasm frames. Each ends the instance.

function ends(pkg: any, raw: any, library: any, other: any, escape: () => unknown, message: RegExp) {
  const idle = raw.__stack_pointer.value;
  let thrown: any;
  try { escape(); } catch (error) { thrown = error; }
  expect(thrown).toBeInstanceOf(Error);
  expect(thrown.message).toMatch(message);
  // The pointer is put back for anything that still runs.
  expect(raw.__stack_pointer.value).toBe(idle);
  refusesEverything(pkg, library, [other]);
}

test('escape P11: the generated glue fails while Rust waits on the host function', () => {
  // The callback's outcome is copied into wasm memory by generated glue;
  // fail that copy, as a stack overflow there would.
  const { pkg, raw } = load(`
    const pass = passStringToWasm0;
    passStringToWasm0 = (arg, malloc, realloc) => {
      if (typeof arg === 'string' && arg.endsWith('callback overflow')) throw new RangeError('interrupted import');
      return pass(arg, malloc, realloc);
    };
  `);
  const library = new pkg.ComponentLibrary(schema);
  const other = session(pkg, library, '$n=5\nroot=Box($n)');
  const s = session(pkg, library, 'root=Box(@Echo(7))');
  s.registerFunction('Echo', () => { throw new RangeError('callback overflow'); });
  ends(pkg, raw, library, other, () => s.evaluate(), /^interrupted import$/);
  // The interrupted session, still borrowed in Rust, frees without entering it.
  s.free(); s.free();
});

test('escape: an overflow inside __wbindgen_malloc while the glue copies a string in', () => {
  const { pkg, raw } = load(`
    const real = wasm, malloc = real.__wbindgen_malloc;
    let armed = false;
    globalThis.__openuiArmMalloc = () => { armed = true; };
    wasm = { ...real, __wbindgen_malloc(...args) {
      if (armed) { armed = false; throw new RangeError('Maximum call stack size exceeded.'); }
      return malloc(...args);
    } };
  `);
  const library = new pkg.ComponentLibrary(schema);
  const other = session(pkg, library, '$n=5\nroot=Box($n)');
  ends(pkg, raw, library, other, () => {
    (globalThis as any).__openuiArmMalloc();
    pkg.parse('root=Box(1)', library);
  }, /Maximum call stack/);
  delete (globalThis as any).__openuiArmMalloc;
});

test('escape: no raw export runs after the escape, not even the glue\'s clean-up', () => {
  // getField copies two strings in. Fail the second copy's allocation: the
  // generated glue then frees the first string in a finally block, which
  // must not reach the possibly half-updated allocator.
  const { pkg, raw } = load(`
    const real = wasm;
    let armed = false, mallocs = 0;
    globalThis.__openuiSpy = { calls: [], arm() { armed = true; mallocs = 0; } };
    const spied = {};
    for (const name of Object.keys(real)) {
      const value = real[name];
      spied[name] = typeof value !== 'function' ? value : (...args) => {
        globalThis.__openuiSpy.calls.push(name);
        if (armed && name === '__wbindgen_malloc' && ++mallocs === 2) {
          armed = false;
          throw new RangeError('Maximum call stack size exceeded.');
        }
        return value(...args);
      };
    }
    wasm = spied;
  `);
  const spy = (globalThis as any).__openuiSpy;
  const library = new pkg.ComponentLibrary(schema);
  const other = session(pkg, library, '$n=5\nroot=Box($n)');
  const s = session(pkg, library, 'root=Box(1)');
  spy.arm();
  let thrown: any;
  try { s.getField('the form', 'the field'); } catch (error) { thrown = error; }
  expect(thrown.message).toMatch(/Maximum call stack/);
  const escapedAt = spy.calls.lastIndexOf('__wbindgen_malloc');
  expect(spy.calls[escapedAt - 1]).toBe('__wbindgen_malloc');
  // Nothing after the failed allocation reached a raw export: not the glue's
  // free of the first string, nor any later call, free or finalizer.
  refusesEverything(pkg, library, [other, s]);
  Bun.gc(true);
  expect(spy.calls.slice(escapedAt + 1)).toEqual([]);
  delete (globalThis as any).__openuiSpy;
});

test('escape: a wasm trap', () => {
  // The callback's result is copied in by generated glue; make that copy
  // free an externref table slot that does not exist, a genuine trap.
  const { pkg, raw } = load(`
    const pass = passStringToWasm0;
    passStringToWasm0 = (arg, malloc, realloc) => {
      if (arg === '"trap"') wasm.__externref_table_dealloc(0x7fffffff);
      return pass(arg, malloc, realloc);
    };
  `);
  const library = new pkg.ComponentLibrary(schema);
  const other = session(pkg, library, '$n=5\nroot=Box($n)');
  const s = session(pkg, library, 'root=Box(@Echo(7))');
  s.registerFunction('Echo', () => '"trap"');
  ends(pkg, raw, library, other, () => s.evaluate(), /out of bounds table access/i);
  s.free();
});

// Recurse to the JavaScript stack limit, then call `run` at each frame on the
// way back until it says it is done: some calls fail before Rust, some in it.
function nearTheStackLimit(run: () => boolean) {
  const down = () => {
    try { down(); } catch (error) {
      // Not done: let the frame above try again, with a little more room.
      if (!run()) throw error;
    }
  };
  try { down(); } catch { /* never done, even at the top */ }
}

test('escape P13: a real JavaScript stack overflow inside wasm frames, 100 times', () => {
  const deep = `$a=1\nroot=Box(${'['.repeat(200)}$a${']'.repeat(200)})`;
  let escapes = 0, attempts = 0;
  while (escapes < 100) {
    expect(++attempts).toBeLessThan(1000);
    const { pkg, raw } = load();
    const library = new pkg.ComponentLibrary(schema);
    const other = session(pkg, library, '$n=5\nroot=Box($n)');
    const idle = raw.__stack_pointer.value;
    nearTheStackLimit(() => {
      let s;
      try { s = session(pkg, library, deep); } catch { return false; }
      try { s.evaluate().free(); s.free(); } catch { /* inside Rust, or just before */ }
      return true;
    });
    // Decided back at the top of the stack, with room to call.
    let escaped = false;
    try { other.snapshot(); } catch (error: any) {
      expect(error.message).toBe(INTERRUPTED);
      escaped = true;
    }
    if (!escaped) {
      // Every call near the limit failed before Rust: this instance still
      // answers; try again in a new one.
      answers(pkg, library, other);
      other.free(); library.free();
      continue;
    }
    escapes++;
    expect(raw.__stack_pointer.value).toBe(idle);
    refusesEverything(pkg, library, [other]);
  }
}, 120_000);

// ---------------------------------------------------------------------------
// Finalizers of a refusing instance enter no Rust.

test("a refusing instance's finalizers enter no Rust", async () => {
  // Count calls that reach the real free exports, beneath the guard.
  const { pkg } = load(`
    const real = wasm;
    globalThis.__openuiFrees = 0;
    const counted = { __proto__: null };
    for (const name of Object.keys(real)) {
      if (name.startsWith('__wbg_') && name.endsWith('_free')) {
        const free = real[name];
        counted[name] = (...args) => { globalThis.__openuiFrees++; return free(...args); };
      }
    }
    wasm = { ...real, ...counted };
    const pass = passStringToWasm0;
    passStringToWasm0 = (arg, malloc, realloc) => {
      if (typeof arg === 'string' && arg.endsWith('callback overflow')) throw new RangeError('interrupted import');
      return pass(arg, malloc, realloc);
    };
  `);
  const library = new pkg.ComponentLibrary(schema);
  const collect = async () => {
    for (let i = 0; i < 5; i++) { Bun.gc(true); await new Promise(resolve => setTimeout(resolve, 5)); }
  };
  const abandon = () => { for (let i = 0; i < 50; i++) new pkg.ParseLimits(); };
  // While the instance answers, an object never freed is finalized into Rust.
  abandon();
  await collect();
  expect((globalThis as any).__openuiFrees).toBeGreaterThan(0);
  const s = session(pkg, library, 'root=Box(@Echo(7))');
  s.registerFunction('Echo', () => { throw new RangeError('callback overflow'); });
  abandon();
  kind(() => s.evaluate(), 'Worker');
  (globalThis as any).__openuiFrees = 0;
  await collect();
  expect((globalThis as any).__openuiFrees).toBe(0);
  delete (globalThis as any).__openuiFrees;
});
