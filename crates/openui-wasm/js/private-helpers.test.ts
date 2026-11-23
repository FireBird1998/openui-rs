// Host code must not run inside a guarded call through the generated private
// helpers (#217). _assertClass did `instance instanceof klass`: a minted
// object given a Proxy prototype ran the Proxy's getPrototypeOf trap inside
// the call, where private helpers (a class's static __wrap) are usable, and
// the trap could mint an unminted second handle to a live object. The
// finalizer of that alias later freed the victim: a use-after-free. __wrap
// also looked up the global Object.create at each wrap, so a replaced one
// could make it return the host's own object.
import { beforeAll, expect, test } from 'bun:test';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { readFileSync } from 'node:fs';
import { wasm, kind, setup, suspend, SCHEMA, packagePath, packageSource, loadWith } from './support';
import { generatedGlue, generatedFunctions } from './glue-scan';
import { cases } from './class-cases';

// The web build, loaded the way package.test.ts loads it.
const webDir = join(dirname(packagePath), '../web');
// Loaded when the tests run, as its own copy (the query keeps it apart from the
// module other files import, response-resend.test.ts loads one under a patched
// WeakMap.prototype.set): never while another file's top level has an
// intrinsic replaced.
let web: any;
beforeAll(async () => {
  web = await import(join(webDir, 'openui_wasm.js') + '?private-helpers');
  web.initSync({ module: readFileSync(join(webDir, 'openui_wasm_bg.wasm')) });
});
const targets: [string, () => any][] = [['node', () => wasm], ['web', () => web]];

const gc = async () => { for (let i = 0; i < 5; i++) { Bun.gc(true); await Bun.sleep(0); } };

/** A handler whose every trap counts, then does what an ordinary object would. */
function counting() {
  const state = { hits: 0, names: [] as string[] };
  const handler: ProxyHandler<any> = {};
  for (const trap of ['getPrototypeOf', 'setPrototypeOf', 'isExtensible', 'preventExtensions', 'getOwnPropertyDescriptor',
    'defineProperty', 'has', 'get', 'set', 'deleteProperty', 'ownKeys', 'apply', 'construct'] as const) {
    (handler as any)[trap] = (...args: any[]) => { state.hits++; state.names.push(trap); return (Reflect as any)[trap](...args); };
  }
  return { state, handler };
}

for (const [target, getPkg] of targets) {
  test(`a Proxy prototype on a minted library cannot reach __wrap through the instanceof check (${target})`, async () => {
    const pkg = getPkg();
    const { library: victimLibrary, s: victimSession, action: victim } = setup(pkg);
    const library = new pkg.ComponentLibrary(SCHEMA);
    let alias: any, trapRuns = 0, reached: unknown;
    Object.setPrototypeOf(library, new Proxy({}, {
      getPrototypeOf() {
        trapRuns++;
        try { alias = pkg.ActionHandle.__wrap(victim.__wbg_ptr); } catch (error) { reached = error; }
        return pkg.ComponentLibrary.prototype;
      },
    }));
    const s = new pkg.RuntimeSession(library);
    s.free();
    expect(trapRuns).toBe(0);
    expect(alias).toBeUndefined();
    // Outside a guarded call the private helper still refuses.
    kind(() => pkg.ActionHandle.__wrap(victim.__wbg_ptr), 'InvalidHandle');
    alias = undefined;
    await gc();
    // The victim is still alive and usable: its retain() does not trap.
    const retained = victim.retain();
    retained.free();
    expect(victimSession.snapshot()).toBe('{"$n":0}');
    await gc();
    victim.retain().free();
    Object.setPrototypeOf(library, pkg.ComponentLibrary.prototype);
    victim.free(); victimSession.free(); victimLibrary.free(); library.free();
  });
}

test('the same Proxy prototype on a library passed to every other entry runs no trap', () => {
  const { handler, state } = counting();
  const library = new wasm.ComponentLibrary(SCHEMA);
  Object.setPrototypeOf(library, new Proxy(wasm.ComponentLibrary.prototype, handler));
  wasm.parseProgram('root=Box(1)', library).free();
  wasm.parse('root=Box(1)', library);
  new wasm.StreamSession(library).free();
  const s = new wasm.RuntimeSession(library); s.free();
  expect(state.names).toEqual([]);
  library.free();
});

for (const [target, getPkg] of targets) {
  test(`a replaced Object.create cannot make __wrap return the host's object (${target})`, () => {
    const pkg = getPkg();
    const { library, s, action } = setup(pkg);
    const original = Object.create;
    const forged: unknown[] = [];
    let calls = 0;
    try {
      Object.create = ((...args: [any, any?]) => {
        calls++;
        const made = original(...args);
        const proxy = new Proxy(made, {});
        forged.push(proxy);
        return proxy;
      }) as typeof Object.create;
      // __wrap runs here: retain() wraps its result, evaluate() wraps its output.
      const retained = action.retain();
      const out = s.evaluate();
      expect(calls).toBe(0);
      expect(forged).toEqual([]);
      expect(forged.includes(retained)).toBe(false);
      expect(Object.getPrototypeOf(retained)).toBe(pkg.ActionHandle.prototype);
      // A genuine guarded instance: minted, with the own non-configurable pointer.
      const field = Object.getOwnPropertyDescriptor(retained, '__wbg_ptr')!;
      expect(field.configurable).toBe(false);
      expect(typeof field.get).toBe('function');
      expect(retained.__wbg_ptr).not.toBe(0);
      retained.free(); out.free();
    } finally { Object.create = original; }
    action.free(); s.free(); library.free();
  });
}

for (const [target, getPkg] of targets) {
  test(`a replaced FinalizationRegistry method or an Object.prototype pointer setter runs no host code in a wrap (${target})`, () => {
    const pkg = getPkg();
    const { library, s, action } = setup(pkg);
    const proto = FinalizationRegistry.prototype;
    const originalRegister = proto.register;
    let hits = 0;
    Object.defineProperty(Object.prototype, '__wbg_ptr', { configurable: true, set() { hits++; }, get() { return 0; } });
    proto.register = function () { hits++; } as any;
    try {
      const retained = action.retain();
      expect(hits).toBe(0);
      expect(retained.__wbg_ptr).not.toBe(0);
      retained.free();
    } finally {
      proto.register = originalRegister;
      delete (Object.prototype as any).__wbg_ptr;
    }
    action.free(); s.free(); library.free();
  });
}

// Every generated entry that checks an object argument with _assertClass,
// read from the generated glue: `Owner.name:parameter`.
const objectEntries = () => {
  const { found } = generatedFunctions(generatedGlue(packagePath), false);
  const entries: { key: string, klass: string }[] = [];
  for (const f of found) {
    for (const m of f.body.matchAll(/_assertClass\((\w+), (\w+)\)/g)) {
      expect(f.params).toContain(m[1]);
      entries.push({ key: `${f.owner}.${f.name}:${m[1]}`, klass: m[2] });
    }
  }
  return entries;
};

const BOX = '{"$defs":{"Box":{"properties":{"value":{}}}}}';
type World = { made: any[], make: () => any, call: (arg: any) => unknown, done: () => void };
/** A fresh session world; `make` mints an object of the entry's class. */
function world(klass: () => any, call: (w: { library: any, s: any, action: any, limits: any }, arg: any) => unknown): World {
  const made: any[] = [];
  const w = { ...setup(), limits: new wasm.RuntimeLimits() };
  return { made, call: arg => call(w, arg),
    make: () => { const x = klass.call(w); made.push(x); return x; },
    done: () => { for (const x of made) x.free(); w.limits.free(); w.action.free(); w.s.free(); w.library.free(); } };
}
const parseLimits = () => new wasm.ParseLimits();
const newLibrary = () => new wasm.ComponentLibrary(SCHEMA);
const newProgram = function (this: any) { return wasm.parseProgram('root=Box(1)', this.library); };
const queryWorld = () => {
  const library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  const s = new wasm.RuntimeSession(library);
  s.attachToolProvider();
  const p = wasm.parseProgram('q=Query("read",{},"loading")\nroot=Box(q)', library);
  const replaced = s.replaceProgram(p, undefined, undefined, false);
  const effects = s.takeQueryUpdate();
  return { s, library, done: () => { effects.free(); replaced.free(); p.free(); s.free(); library.free(); }, effects };
};
const wantedWorld = (): World => {
  const q = queryWorld(), made: any[] = [];
  return { made, make: () => { const x = q.effects.callAt(0n); made.push(x); return x; },
    call: arg => q.s.completeToolCall(arg, '{"ok":true}', true),
    done: () => { for (const x of made) x.free(); q.done(); } };
};
const worlds: Record<string, () => World> = {
  'ComponentLibrary.fromDefinitionWithLimits:limits': () => world(() => new wasm.RuntimeLimits(),
    (_w, arg) => wasm.ComponentLibrary.fromDefinitionWithLimits('{}', arg)),
  'ComponentLibrary.fromSchemaWithLimits:limits': () => world(() => new wasm.RuntimeLimits(),
    (_w, arg) => wasm.ComponentLibrary.fromSchemaWithLimits(BOX, arg)),
  'RuntimeLimits.evaluation:value': () => world(parseLimits, (w, arg) => { w.limits.evaluation = arg; }),
  'RuntimeLimits.referencePreparation:value': () => world(parseLimits, (w, arg) => { w.limits.referencePreparation = arg; }),
  'RuntimeSession.actionAt:evaluation': () => world(function (this: any) {
    const out = this.s.evaluate(), evaluation = out.evaluation; out.free(); return evaluation;
  }, (w, arg) => w.s.actionAt(arg, ['props', 'a'])),
  'RuntimeSession.completeToolCall:call': wantedWorld,
  'RuntimeSession.constructor:library': () => world(newLibrary, (_w, arg) => new wasm.RuntimeSession(arg)),
  'RuntimeSession.replaceProgram:program': () => world(newProgram,
    (w, arg) => w.s.replaceProgram(arg, undefined, undefined, false)),
  'RuntimeSession.resume:continuation': () => world(function (this: any) { return suspend(this.s, this.action); },
    (w, arg) => w.s.resume(arg, true)),
  'RuntimeSession.run:action': () => world(function (this: any) { return this.action.retain(); },
    (w, arg) => w.s.run(arg, 'click')),
  'RuntimeSession.setProgram:program': () => world(newProgram, (w, arg) => w.s.setProgram(arg)),
  'RuntimeSession.withLimits:library': () => world(newLibrary,
    (w, arg) => wasm.RuntimeSession.withLimits(arg, undefined, w.limits)),
  'RuntimeSession.withLimits:limits': () => world(() => new wasm.RuntimeLimits(),
    (w, arg) => wasm.RuntimeSession.withLimits(w.library, undefined, arg)),
  'StreamSession.constructor:library': () => world(newLibrary, (_w, arg) => new wasm.StreamSession(arg)),
  'module.extractMcpToolResult:limits': () => world(parseLimits, (_w, arg) => wasm.extractMcpToolResult('{}', arg)),
  'module.parse:library': () => world(newLibrary, (_w, arg) => wasm.parse('root=Box(1)', arg)),
  'module.parseProgram:library': () => world(newLibrary, (_w, arg) => wasm.parseProgram('root=Box(1)', arg)),
};

/** What a call did, with whatever it returned released. */
function outcome(call: () => unknown) {
  try {
    const result: any = call();
    if (result !== null && typeof result === 'object' && typeof result.free === 'function') result.free();
    return 'ok';
  } catch (error: any) { return `${error?.kind}: ${error?.message}`; }
}

test('the object-taking entries read from the generated glue are all driven here', () => {
  const entries = objectEntries();
  expect(entries.map(e => e.key).sort()).toEqual(Object.keys(worlds).sort());
  expect(entries.length).toBe(17);
});

for (const { key } of objectEntries()) {
  const fresh = worlds[key];
  test(`${key}: a Proxy prototype on a minted argument runs no trap, and the call behaves as for a normal one`, () => {
    const control = fresh();
    const expected = outcome(() => control.call(control.make()));
    control.done();
    const w = fresh();
    const { handler, state } = counting();
    const arg = w.make();
    Object.setPrototypeOf(arg, new Proxy(Object.getPrototypeOf(arg), handler));
    expect(outcome(() => w.call(arg))).toBe(expected);
    expect(state.names).toEqual([]);
    w.done();
  });

  test(`${key}: a Proxy around a real instance is refused and runs no trap`, () => {
    const w = fresh();
    const { handler, state } = counting();
    const proxy = new Proxy(w.make(), handler);
    const result = outcome(() => w.call(proxy));
    expect(result).toStartWith('InvalidHandle:');
    expect(state.names).toEqual([]);
    w.done();
  });

  test(`${key}: a real instance whose prototype is another class's is not mistaken, and runs no trap`, () => {
    const control = fresh();
    const expected = outcome(() => control.call(control.make()));
    control.done();
    const w = fresh();
    const arg = w.make();
    const other = Object.getPrototypeOf(arg) === wasm.EvaluationOutput.prototype ? wasm.ActionOutput.prototype : wasm.EvaluationOutput.prototype;
    const own = Object.getPrototypeOf(arg);
    Object.setPrototypeOf(arg, other);
    // The guard's own record names its class: it is neither refused as another class nor let through to anything else.
    const result = outcome(() => w.call(arg));
    Object.setPrototypeOf(arg, own);
    expect(result).toBe(expected);
    w.done();
  });

  test(`${key}: a minted object of another class is refused`, () => {
    const w = fresh();
    const wrong = new wasm.ParseLimits();
    const wrongOne = Object.getPrototypeOf(w.make()) === wasm.ParseLimits.prototype ? new wasm.RuntimeLimits() : wrong;
    expect(outcome(() => w.call(wrongOne))).toStartWith('InvalidHandle:');
    wrong.free(); if (wrongOne !== wrong) wrongOne.free();
    w.done();
  });
}

test('Symbol.hasInstance defined by host code on a class runs nothing', () => {
  let hits = 0;
  const hook = { value: () => { hits++; return true; }, configurable: true };
  for (const klass of [wasm.ComponentLibrary, wasm.RuntimeLimits, wasm.ParseLimits, wasm.ParsedProgram, wasm.ActionHandle,
    wasm.EvaluationHandle, wasm.ContinuationHandle, Object.getPrototypeOf(wasm.ParseLimits.prototype).constructor]) {
    try { Object.defineProperty(klass, Symbol.hasInstance, hook); } catch { /* frozen: the usual outcome */ }
  }
  const library = new wasm.ComponentLibrary(SCHEMA);
  // Neither the real class nor a forged stand-in is admitted through it.
  const forged = Object.create(wasm.ComponentLibrary.prototype, { __wbg_ptr: { value: library.__wbg_ptr } });
  kind(() => new wasm.RuntimeSession(forged), 'InvalidHandle');
  kind(() => new wasm.StreamSession({ __wbg_ptr: library.__wbg_ptr }), 'InvalidHandle');
  new wasm.RuntimeSession(library).free();
  expect(hits).toBe(0);
  library.free();
});

// The rebuilt _assertClass itself, reached by evaluating the built package
// with one line appended after the guard.
function loadAssert() {
  const { glue, guard } = packageSource();
  const require = createRequire(import.meta.url);
  const module = { exports: {} as any };
  new Function('exports', 'require', 'module', '__filename', '__dirname',
    `${glue}\n${guard}\nexports.__assertClass = _assertClass;`)(
    module.exports, require, module, packagePath, dirname(packagePath));
  return module.exports;
}
test('_assertClass decides by the guard\'s own record and runs no host code', () => {
  const pkg = loadAssert();
  const { handler, state } = counting();
  const library = new pkg.ComponentLibrary(SCHEMA);
  pkg.__assertClass(library, pkg.ComponentLibrary);
  // A Proxy prototype, a Proxy around the instance, a forged object, a
  // primitive, and another class's minted instance: no trap, and refused
  // with the generated message unless it is the minted instance itself.
  Object.setPrototypeOf(library, new Proxy(pkg.ComponentLibrary.prototype, handler));
  pkg.__assertClass(library, pkg.ComponentLibrary);
  const limits = new pkg.ParseLimits();
  for (const bad of [new Proxy(limits, handler), Object.create(pkg.ComponentLibrary.prototype), { __wbg_ptr: 1 }, 3, null, undefined, 'x', limits]) {
    let thrown: any;
    try { pkg.__assertClass(bad, pkg.ComponentLibrary); } catch (error) { thrown = error; }
    expect(thrown).toBeInstanceOf(Error);
    expect(thrown.message).toBe('expected instance of ComponentLibrary');
  }
  expect(state.names).toEqual([]);
  limits.free(); library.free();
});

// free() and [Symbol.dispose]() run the generated __destroy_into_raw, which
// called FinalizationRegistry.prototype.unregister through a live lookup,
// inside the guarded call. A replaced one could mint an alias there with
// __wrap (and the rebuilt __wrap mints its result, so the alias would be fully
// trusted). Every class, both ways of releasing it.
for (const c of cases) {
  for (const release of ['free', 'dispose'] as const) {
    test(`${c.name}: a replaced FinalizationRegistry.prototype.unregister never runs inside ${release}`, async () => {
      const { library: victimLibrary, s: victimSession, action: victim } = setup();
      const made = c.make(7);
      const proto = FinalizationRegistry.prototype;
      const original = proto.unregister;
      let hits = 0, alias: any;
      proto.unregister = function () {
        hits++;
        try { alias = wasm.ActionHandle.__wrap(victim.__wbg_ptr); } catch { /* refused */ }
        return true;
      } as any;
      try {
        if (release === 'free') made.x.free(); else made.x[Symbol.dispose]();
      } finally { proto.unregister = original; }
      expect(hits).toBe(0);
      expect(alias).toBeUndefined();
      expect(made.x.__wbg_ptr).toBe(0);
      alias = undefined;
      await gc();
      victim.retain().free();
      expect(victimSession.snapshot()).toBe('{"$n":0}');
      made.done(); victim.free(); victimSession.free(); victimLibrary.free();
    });
  }
}

test('an interrupted instance releases objects without running a replaced unregister', () => {
  // A genuine escape (a wasm trap during a copy), as in interrupted.test.ts.
  const pkg = loadWith(`
    const pass = passStringToWasm0;
    passStringToWasm0 = (arg, malloc, realloc) => {
      if (arg === '"trap"') wasm.__externref_table_dealloc(0x7fffffff);
      return pass(arg, malloc, realloc);
    };
  `);
  const schema = '{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}}}';
  const library = new pkg.ComponentLibrary(schema);
  const s = new pkg.RuntimeSession(library);
  const p = pkg.parseProgram('root=Box(@Echo(7))', library); s.setProgram(p); p.free();
  s.registerFunction('Echo', () => '"trap"');
  expect(() => s.evaluate()).toThrow();
  kind(() => library.jsonSchema(), 'Disposed');
  const { library: victimLibrary, s: victimSession, action: victim } = setup();
  const proto = FinalizationRegistry.prototype, original = proto.unregister;
  let hits = 0, alias: any;
  proto.unregister = function () { hits++; try { alias = wasm.ActionHandle.__wrap(victim.__wbg_ptr); } catch { /* refused */ } return true; } as any;
  try { library.free(); s[Symbol.dispose](); } finally { proto.unregister = original; }
  expect(hits).toBe(0);
  expect(alias).toBeUndefined();
  expect(library.__wbg_ptr).toBe(0);
  victim.retain().free();
  victim.free(); victimSession.free(); victimLibrary.free();
});

// The same, against the web build (the per-class tests above run on node).
for (const release of ['free', 'dispose'] as const) {
  test(`web: a replaced FinalizationRegistry.prototype.unregister never runs inside ${release}`, async () => {
    const { library: victimLibrary, s: victimSession, action: victim } = setup(web);
    const library = new web.ComponentLibrary(SCHEMA);
    const proto = FinalizationRegistry.prototype, original = proto.unregister;
    let hits = 0, alias: any;
    proto.unregister = function () { hits++; try { alias = web.ActionHandle.__wrap(victim.__wbg_ptr); } catch { /* refused */ } return true; } as any;
    try { if (release === 'free') library.free(); else library[Symbol.dispose](); } finally { proto.unregister = original; }
    expect(hits).toBe(0);
    expect(alias).toBeUndefined();
    await gc();
    victim.retain().free();
    expect(victimSession.snapshot()).toBe('{"$n":0}');
    victim.free(); victimSession.free(); victimLibrary.free();
  });
}

for (const [target, getPkg] of targets) {
  test(`a replaced Function.prototype.call, DataView.prototype.setInt32 or Table.prototype.set never runs in a guarded call (${target})`, () => {
    const pkg = getPkg();
    const { library: victimLibrary, s: victimSession, action: victim } = setup(pkg);
    const schema = '{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{"type":"object","properties":{"value":{}}}}}}';
    const library = new pkg.ComponentLibrary(schema);
    const s = new pkg.RuntimeSession(library);
    const p = pkg.parseProgram('root=Box(@Echo(7))', library); s.setProgram(p); p.free();
    s.registerFunction('Echo', () => '8');
    const originals = [[Function.prototype, 'call'], [DataView.prototype, 'setInt32'], [WebAssembly.Table.prototype, 'set']] as const;
    const saved = originals.map(([o, k]) => (o as any)[k]);
    let hits = 0, alias: any;
    const trap = function () { hits++; try { alias = pkg.ActionHandle.__wrap(victim.__wbg_ptr); } catch { /* refused */ } };
    try {
      for (const [o, k] of originals) Object.defineProperty(o, k, { value: trap, configurable: true, writable: true });
      const out = s.evaluate(); out.free();
    } finally {
      originals.forEach(([o, k], i) => Object.defineProperty(o, k, { value: saved[i], configurable: true, writable: true }));
    }
    expect(hits).toBe(0);
    expect(alias).toBeUndefined();
    victim.retain().free();
    s.free(); library.free(); victim.free(); victimSession.free(); victimLibrary.free();
  });
}
