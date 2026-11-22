// A host must not be able to change which pointer generated code frees or
// reads (#211). wasm-bindgen's free() calls this.__destroy_into_raw()
// through the instance: an own property, a subclass field or a swapped
// prototype could once redirect it to another object's pointer, freeing that
// object's memory while it was still live (use-after-free with aliasing).
// A Proxy around an instance was, and stays, refused as a receiver.
import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { wasm, kind, setup, suspend, SCHEMA, packagePath } from './support';

/** The wasm-bindgen output alone, before scripts/wasm.sh appends our files. */
function generatedGlue(path: string) {
  const source = readFileSync(path, 'utf8');
  const appended = readFileSync(new URL('./tool-provider.js', import.meta.url), 'utf8');
  const at = source.indexOf(appended);
  if (at < 0) throw new Error(`${path}: appended tool-provider.js not found`);
  return source.slice(0, at);
}

/** A fresh instance, a read that depends on its memory, and its cleanup. */
type Made = { x: any, probe: () => unknown, done: () => void };
type Case = { name: string, make: (i: number) => Made, construct?: (i: number) => unknown[] };

const queryProgram = (i: number) => `q=Query("read${i}",{},"loading")\nroot=Box(q)`;
/** A session with a manual tool provider and a query waiting for its call. */
function queryWorld(i: number) {
  const library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  const s = new wasm.RuntimeSession(library);
  s.attachToolProvider();
  const p = wasm.parseProgram(queryProgram(i), library);
  const replaced = s.replaceProgram(p, undefined, undefined, false);
  const effects = s.takeQueryUpdate();
  return { library, s, effects, done: () => { effects.free(); replaced.free(); p.free(); s.free(); library.free(); } };
}
/** A session showing Box(i). */
function drawWorld(i: number) {
  const library = new wasm.ComponentLibrary(SCHEMA);
  const s = new wasm.RuntimeSession(library);
  const p = wasm.parseProgram(`root=Box(${i})`, library);
  s.setProgram(p); p.free();
  return { library, s, done: () => { s.free(); library.free(); } };
}

const cases: Case[] = [
  { name: 'ComponentLibrary', construct: i => [`{"$defs":{"B${i}":{"properties":{}}}}`], make: i => {
    const x = new wasm.ComponentLibrary(`{"$defs":{"B${i}":{"properties":{}}}}`);
    return { x, probe: () => x.jsonSchema(), done: () => x.free() };
  } },
  { name: 'RuntimeSession', construct: () => [new wasm.ComponentLibrary(SCHEMA)], make: i => {
    const library = new wasm.ComponentLibrary(SCHEMA), x = new wasm.RuntimeSession(library);
    x.set('$x', String(i));
    return { x, probe: () => [x.get('$x'), x.version()], done: () => { x.free(); library.free(); } };
  } },
  { name: 'StreamSession', construct: () => [new wasm.ComponentLibrary(SCHEMA)], make: i => {
    const library = new wasm.ComponentLibrary(SCHEMA), x = new wasm.StreamSession(library);
    x.push(`root=Box(${i})\n`);
    return { x, probe: () => x.result(), done: () => { x.free(); library.free(); } };
  } },
  { name: 'RuntimeLimits', construct: () => [], make: i => {
    const x = new wasm.RuntimeLimits(); x.maxJsonBytes = BigInt(100000 + i);
    return { x, probe: () => x.maxJsonBytes, done: () => x.free() };
  } },
  { name: 'ParseLimits', construct: () => [], make: i => {
    const x = new wasm.ParseLimits(); x.depth = BigInt(10 + i);
    return { x, probe: () => x.depth, done: () => x.free() };
  } },
  { name: 'ParsedProgram', make: i => {
    const library = new wasm.ComponentLibrary(SCHEMA), x = wasm.parseProgram(`root=Box(${i})`, library);
    const probe = () => {
      const s = new wasm.RuntimeSession(library); s.setProgram(x);
      const out = s.evaluate(), json = out.json; out.free(); s.free();
      return json;
    };
    return { x, probe, done: () => { x.free(); library.free(); } };
  } },
  { name: 'EvaluationOutput', make: i => {
    const world = drawWorld(i), x = world.s.evaluate();
    return { x, probe: () => x.json, done: () => { x.free(); world.done(); } };
  } },
  { name: 'EvaluationHandle', make: () => {
    const { library, s, action } = setup(); action.free();
    const out = s.evaluate(), x = out.evaluation; out.free();
    const probe = () => { const a = s.actionAt(x, ['props', 'a']); a.free(); return s.version(); };
    return { x, probe, done: () => { x.free(); s.free(); library.free(); } };
  } },
  { name: 'ActionHandle', make: () => {
    const { library, s, action: x } = setup();
    const probe = () => { const r = x.retain(); r.free(); return s.snapshot(); };
    return { x, probe, done: () => { x.free(); s.free(); library.free(); } };
  } },
  { name: 'ContinuationHandle', make: () => {
    const { library, s, action } = setup(), x = suspend(s, action);
    // Nothing reads a continuation without consuming it: resuming it last
    // shows it still names its own session's pending run.
    return { x, probe: () => s.pendingContinuations(), done: () => {
      if (x.__wbg_ptr !== 0) { s.resume(x, true); expect(s.snapshot()).toBe('{"$n":1}'); }
      x.free(); action.free(); s.free(); library.free();
    } };
  } },
  { name: 'ActionOutput', make: () => {
    const { library, s, action } = setup(), x = s.run(action, 'click');
    return { x, probe: () => [x.events, x.failure], done: () => {
      if (x.__wbg_ptr !== 0) x.continuation?.free();
      x.free(); action.free(); s.free(); library.free();
    } };
  } },
  { name: 'ReplacementOutput', make: i => {
    const library = new wasm.ComponentLibrary(SCHEMA), s = new wasm.RuntimeSession(library);
    const p = wasm.parseProgram(`$v=${i}\nroot=Box($v)`, library);
    const x = s.replaceProgram(p, undefined, undefined, false); p.free();
    return { x, probe: () => [x.version, x.snapshot], done: () => { x.free(); s.free(); library.free(); } };
  } },
  { name: 'QueryEffects', make: i => {
    const world = queryWorld(i), x = world.effects;
    return { x, probe: () => [x.callCount, x.changed], done: world.done };
  } },
  { name: 'WantedToolCall', make: i => {
    const world = queryWorld(i), x = world.effects.callAt(0n);
    return { x, probe: () => [x.toolName, x.pending], done: () => { x.free(); world.done(); } };
  } },
];

/** The class an instance belongs to, as the host can reach it. */
const classOf = (x: any) => Object.getPrototypeOf(x).constructor;

/**
 * Make an attacker whose free tries to release the victim's pointer, free it
 * the way a host would (free or Symbol.dispose, looked up through the
 * attacker), mint a fresh instance, and check the victim is untouched: its
 * pointer is not the fresh one's, and the read that depends on its memory
 * still gives what it gave before.
 */
function check(c: Case, makeAttacker: (steal: () => number) => Made, release: 'free' | 'dispose' = 'free') {
  const victim = c.make(0), before = victim.probe();
  const pointer = victim.x.__wbg_ptr;
  expect(pointer).not.toBe(0);
  const attacker = makeAttacker(() => victim.x.__wbg_ptr);
  expect(attacker.x.__wbg_ptr).not.toBe(pointer);
  if (release === 'free') attacker.x.free(); else attacker.x[Symbol.dispose]();
  expect(attacker.x.__wbg_ptr).toBe(0);
  const fresh = c.make(2);
  expect(fresh.x.__wbg_ptr).not.toBe(pointer);
  expect(victim.x.__wbg_ptr).toBe(pointer);
  expect(victim.probe()).toEqual(before);
  fresh.done(); victim.done(); attacker.done();
}
/** c.make(1), changed by `plant` before it is freed. */
const planted = (c: Case, plant: (x: any, steal: () => number) => void) => (steal: () => number) => {
  const s = c.make(1); plant(s.x, steal); return s;
};

test('every exported class, and both result-only classes, are covered here', () => {
  const exported = Object.keys(wasm).filter(key => typeof wasm[key] === 'function' && wasm[key].prototype?.free);
  const named = cases.map(c => c.name);
  expect(exported.sort()).toEqual(named.filter(n => n !== 'QueryEffects' && n !== 'WantedToolCall').sort());
  // The result-only classes are reachable only through instances.
  const world = queryWorld(0), call = world.effects.callAt(0n);
  expect(classOf(world.effects).name).toBe('QueryEffects');
  expect(classOf(call).name).toBe('WantedToolCall');
  call.free(); world.done();
});

for (const c of cases) {
  // The reproduction from #211, for this class.
  for (const release of ['free', 'dispose'] as const) {
    test(`${c.name}: an own __destroy_into_raw cannot redirect ${release}`, () => {
      check(c, planted(c, (s, steal) => {
        Object.defineProperty(s, '__destroy_into_raw', { value: steal });
      }), release);
    });
  }

  test(`${c.name}: a subclass prototype set with setPrototypeOf cannot redirect free`, () => {
    check(c, planted(c, (s, steal) => {
      const Evil = class extends classOf(s) { __destroy_into_raw() { return steal(); } };
      Object.setPrototypeOf(s, Evil.prototype);
      expect(s.free).toBe(classOf(s).prototype.free);
    }));
  });

  test(`${c.name}: a Proxy prototype set with setPrototypeOf cannot redirect free`, () => {
    check(c, planted(c, (s, steal) => {
      const prototype = Object.getPrototypeOf(s);
      Object.setPrototypeOf(s, new Proxy(prototype, { get: (o, key, receiver) =>
        key === '__destroy_into_raw' ? steal : Reflect.get(o, key, receiver) }));
    }));
  });

  test(`${c.name}: a Proxy around an instance is refused as a receiver`, () => {
    check(c, planted(c, (s, steal) => {
      const p = new Proxy(s, { get: (o, key) => key === '__destroy_into_raw' ? steal
        : key === '__wbg_ptr' ? steal() : Reflect.get(o, key) });
      kind(() => p.free(), 'InvalidHandle');
      kind(() => classOf(s).prototype.free.call(p), 'InvalidHandle');
      kind(() => p[Symbol.dispose](), 'InvalidHandle');
      expect(s.__wbg_ptr).not.toBe(0);
    }));
  });

  if (c.construct) {
    const construct = c.construct;
    /** An instance made through subclass `Klass`, and its cleanup. */
    const throughSubclass = (Klass: any, i: number): Made => {
      const args = construct(i), x = new Klass(...args);
      return { x, probe: () => { throw new Error('not probed'); }, done: () => {
        x.free(); for (const a of args as any[]) a?.free?.();
      } };
    };
    test(`${c.name}: a subclass field or constructor cannot redirect free`, () => {
      check(c, steal => throughSubclass(class extends wasm[c.name] { __destroy_into_raw = steal; }, 3));
      check(c, steal => throughSubclass(class extends wasm[c.name] {
        constructor(...args: unknown[]) { super(...args); Object.defineProperty(this, '__destroy_into_raw', { value: steal }); }
      }, 4));
    });
    test(`${c.name}: a subclass method never reaches its instances`, () => {
      // The package mints the base class: new.target is not honoured.
      check(c, steal => {
        const made = throughSubclass(class extends wasm[c.name] { __destroy_into_raw() { return steal(); } }, 5);
        expect(Object.getPrototypeOf(made.x)).toBe(wasm[c.name].prototype);
        return made;
      });
    });
  }

  test(`${c.name}: the class reached through an instance is the guarded one`, () => {
    const m = c.make(0);
    const reached = classOf(m.x);
    if (Object.hasOwn(wasm, c.name)) expect(reached).toBe(wasm[c.name]);
    if (!c.construct) kind(() => new reached(), 'InvalidHandle');
    m.done();
  });
}

test('the generated constructor cannot be reached to read a forged pointer', () => {
  const library = new wasm.ComponentLibrary(SCHEMA);
  const forged = Object.create(wasm.ComponentLibrary.prototype, { __wbg_ptr: { value: library.__wbg_ptr } });
  const Reached = wasm.RuntimeSession.prototype.constructor;
  kind(() => new Reached(forged), 'InvalidHandle');
  kind(() => new (classOf(library))(42), 'InvalidJson');
  library.free();
});

// The scan: what generated code may read through an instance (`this`) or
// an object argument. A wasm-bindgen upgrade that generates any other read
// fails here, so it is checked before it ships.
// - __wbg_ptr, anywhere: on every minted instance an own, non-configurable
//   accessor (the guard's), which no property, prototype or subclass can
//   shadow; an object argument is a minted capability the guard checked.
// - this.__destroy_into_raw, only in free: the guard runs the generated free
//   on a stand-in holding the helper captured at load, never on the instance.
// - action.__destroy_into_raw, only in run: that argument is the fresh token
//   the guard retains inside the call, which the host never receives.
// Code that replaces intrinsics the glue calls (Object.create, say) can still
// forge objects; ADR 0007 leaves that outside the guard.
function allowedRead(owner: string, method: string, subject: string, member: string) {
  if (member === '__wbg_ptr') return true;
  if (member !== '__destroy_into_raw') return false;
  if (subject === 'this') return method === 'free';
  return owner === 'RuntimeSession' && method === 'run' && subject === 'action';
}

const parameters = (list: string) => list.split(',').map(p => p.trim()).filter(Boolean);

/** Each method and exported function in the glue, with its parameters and body. */
function generatedFunctions(glue: string, web: boolean) {
  const lines = glue.split('\n');
  const found: { owner: string, name: string, params: string[], body: string }[] = [];
  const classes: string[] = [];
  const exported = new Set<string>();
  for (const line of lines) {
    const m = /^exports\.(\w+) = \1;$/.exec(line);
    if (m) exported.add(m[1]);
  }
  for (let i = 0; i < lines.length; i++) {
    const klass = /^(?:export )?class (\w+) \{$/.exec(lines[i]);
    if (klass) {
      classes.push(klass[1]);
      for (i++; lines[i] !== '}'; i++) {
        const method = /^ {4}(?:static )?(?:get |set )?(\w+)\(([^)]*)\) \{$/.exec(lines[i]);
        if (!method) continue;
        const start = i;
        while (lines[i] !== '    }') i++;
        found.push({ owner: klass[1], name: method[1], params: parameters(method[2]),
          body: lines.slice(start + 1, i).join('\n') });
      }
      continue;
    }
    const fn = /^(export )?function (\w+)\(([^)]*)\) \{$/.exec(lines[i]);
    if (fn && (web ? fn[1] !== undefined : exported.has(fn[2]))) {
      const start = i;
      while (lines[i] !== '}') i++;
      found.push({ owner: 'module', name: fn[2], params: parameters(fn[3]),
        body: lines.slice(start + 1, i).join('\n') });
    }
  }
  return { found, classes };
}

/** Reads through an instance or an object argument that are not allowed. */
function unexpectedReads(found: ReturnType<typeof generatedFunctions>['found']) {
  const unexpected: string[] = [];
  let reads = 0;
  for (const f of found) {
    for (const subject of ['this', ...f.params]) {
      for (const m of f.body.matchAll(new RegExp(`(?<![\\w.$])${subject}\\s*(?:\\.|\\[|\\?\\.)\\s*([\\w$]*)`, 'g'))) {
        reads++;
        if (!allowedRead(f.owner, f.name, subject, m[1])) unexpected.push(`${f.owner}.${f.name}: ${subject}.${m[1] || m[0]}`);
      }
    }
  }
  return { unexpected, reads };
}

test('scan: a read of another member through an instance or argument is reported', () => {
  const glue = [
    'class Sample {',
    '    free() {',
    '        const ptr = this.__destroy_into_raw();',
    '        this.__finalize(ptr);',
    '    }',
    '    get size() {',
    '        return wasm.sample_size(this.__wbg_ptr, this.__destroy_into_raw());',
    '    }',
    '    take(other, text) {',
    '        wasm.sample_take(this.__wbg_ptr, other.__wbg_ptr, other.__owner, other[0], text);',
    '    }',
    '}',
    'function read(handle) {',
    '    return handle?.__inner;',
    '}',
    'exports.read = read;',
  ].join('\n');
  const { unexpected, reads } = unexpectedReads(generatedFunctions(glue, false).found);
  expect(unexpected).toEqual(['Sample.free: this.__finalize', 'Sample.size: this.__destroy_into_raw', 'Sample.take: other.__owner', 'Sample.take: other.0',
    'module.read: handle.__inner']);
  expect(reads).toBe(9);
});

for (const target of ['node', 'web']) {
  test(`scan (${target}): no generated method reads another member through an instance`, () => {
    const path = target === 'node' ? packagePath : packagePath.replace('/node/', '/web/');
    const { found, classes } = generatedFunctions(generatedGlue(path), target === 'web');
    expect(classes.sort()).toEqual(cases.map(c => c.name).sort());
    // Every class has its free, and the scan saw the free functions too.
    for (const name of classes) expect(found.some(f => f.owner === name && f.name === 'free')).toBe(true);
    expect(found.some(f => f.owner === 'module' && f.name === 'parseProgram')).toBe(true);
    const { unexpected, reads } = unexpectedReads(found);
    expect(unexpected).toEqual([]);
    expect(reads).toBeGreaterThan(100);
  });
}

test('scan: free on every class releases only what __destroy_into_raw returns', () => {
  const { found } = generatedFunctions(generatedGlue(packagePath), false);
  for (const f of found.filter(f => f.name === 'free')) {
    expect(f.body.trim().split('\n').map(l => l.trim())).toEqual([
      'const ptr = this.__destroy_into_raw();',
      `wasm.__wbg_${f.owner.toLowerCase()}_free(ptr, 0);`,
    ]);
  }
});
