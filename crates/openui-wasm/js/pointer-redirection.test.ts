// A host must not be able to change which pointer generated code frees or
// reads (#211). wasm-bindgen's free() calls this.__destroy_into_raw()
// through the instance: an own property, a subclass field or a swapped
// prototype could once redirect it to another object's pointer, freeing that
// object's memory while it was still live (use-after-free with aliasing).
// A Proxy around an instance was, and stays, refused as a receiver.
import { expect, test } from 'bun:test';
import { wasm, kind, setup, suspend, SCHEMA, packagePath } from './support';
import { cases, classOf, queryWorld, type Made, type Case } from './class-cases';
import { generatedGlue, generatedFunctions, type Generated } from './glue-scan';

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
// _assertClass and __wrap are rebuilt by the guard (#217, private-helpers.test.ts);
// code that replaces other intrinsics the glue calls can still forge objects,
// which ADR 0007 leaves outside the guard.
function allowedRead(owner: string, method: string, subject: string, member: string) {
  if (member === '__wbg_ptr') return true;
  if (member !== '__destroy_into_raw') return false;
  if (subject === 'this') return method === 'free';
  return owner === 'RuntimeSession' && method === 'run' && subject === 'action';
}

/** Reads through an instance or an object argument that are not allowed. */
function unexpectedReads(found: Generated['found']) {
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

// Each shape the scan must refuse to skip (#217): feed one of each, and it fails.
const unreadable: Record<string, string[]> = {
  'a destructuring read': ['    get size() {', '        const { __inner } = this;', '        return __inner;', '    }'],
  'an array destructuring read': ['    get size() {', '        const [first] = this.__parts;', '        return first;', '    }'],
  'an alias of this': ['    get size() {', '        const self = this;', '        return wasm.sample_size(self.__owner);', '    }'],
  'Reflect.get': ['    get size() {', "        return Reflect.get(this, '__owner');", '    }'],
  'an async method': ['    async size() {', '        return wasm.sample_size(this.__wbg_ptr);', '    }'],
  'a generator method': ['    *size() {', '        yield wasm.sample_size(this.__wbg_ptr);', '    }'],
  'an async generator method': ['    async *size() {', '        yield 1;', '    }'],
  'a computed method': ['    [Symbol.iterator]() {', '        return this.__owner;', '    }'],
  'a one-line method': ['    size() { return wasm.sample_size(this.__owner); }'],
  'a class field': ['    size = () => this.__owner;'],
};
for (const [shape, member] of Object.entries(unreadable)) {
  test(`scan: a method-like line it cannot parse fails it (${shape})`, () => {
    const glue = ['class Sample {', ...member, '}'].join('\n');
    const { unparsed } = generatedFunctions(glue, false);
    expect(unparsed.length).toBeGreaterThan(0);
  });
}
test('scan: an exported async or generator function, or a class with a heritage, fails it', () => {
  for (const line of ['export async function read(handle) {', 'export function* read(handle) {', 'export class Sample extends Base {']) {
    expect(generatedFunctions([line, '}'].join('\n'), true).unparsed.length).toBeGreaterThan(0);
  }
  for (const line of ['async function read(handle) {', 'function* read(handle) {']) {
    expect(generatedFunctions([line, '}', 'exports.read = read;'].join('\n'), false).unparsed.length).toBeGreaterThan(0);
  }
});
test('scan: parseable methods, doc comments and blank lines do not fail it', () => {
  const glue = ['class Sample {', '    /**', '     * Doc.', '     */', '    free() {', '        const ptr = this.__destroy_into_raw();', '    }', '',
    '    static __wrap(ptr) {', '        return ptr;', '    }', '}', 'function read(handle) {', '    return handle;', '}', 'exports.read = read;'].join('\n');
  expect(generatedFunctions(glue, false).unparsed).toEqual([]);
});

for (const target of ['node', 'web']) {
  test(`scan (${target}): no generated method reads another member through an instance`, () => {
    const path = target === 'node' ? packagePath : packagePath.replace('/node/', '/web/');
    const { found, classes, unparsed } = generatedFunctions(generatedGlue(path), target === 'web');
    // A line the scan cannot parse fails it (#217); it is never skipped.
    expect(unparsed).toEqual([]);
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
