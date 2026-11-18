// The built package's shape: every generated public member is guarded, and
// host callbacks execute under the same guard as every public entry point.
import { test, expect } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { wasm, kind, setup, loadWith, packagePath, packageSource } from './support';

const CLASSES = ['ActionHandle', 'ActionOutput', 'ComponentLibrary', 'ContinuationHandle',
  'EvaluationHandle', 'EvaluationOutput', 'ParseLimits', 'ParsedProgram', 'ReplacementOutput', 'RuntimeLimits',
  'RuntimeSession', 'StreamSession'];

test('the package exports only guarded classes and functions', () => {
  expect(Object.keys(wasm).sort()).toEqual([...CLASSES, 'parse', 'parseProgram', 'parseMessage', 'buildMessage', 'parseResponseBundle'].sort());
});

test('an unmodified package loads with the guard', () => {
  const w = loadWith('');
  const { s, library } = setup(w);
  expect(s.version()).toBe(1n);
  s.free(); library.free();
});

test('a generated member the guard does not describe stops the package loading', () => {
  const mutations: [string, RegExp][] = [
    ['RuntimeSession.prototype.extra = function () {};', /RuntimeSession\.extra is not covered/],
    ['ContinuationHandle.prototype[Symbol.asyncDispose] = ContinuationHandle.prototype.free;',
      /ContinuationHandle\.Symbol\(Symbol\.asyncDispose\) is not covered/],
    ['Object.defineProperty(ActionOutput.prototype, "peek", { get() { return 1; } });', /ActionOutput\.peek is not covered/],
    ['Object.defineProperty(ActionOutput.prototype, "events", { get() { return []; }, set(v) {} });', /ActionOutput\.events \(set\) is not covered/],
    ['delete RuntimeLimits.prototype.maxJsonBytes; Object.defineProperty(RuntimeLimits.prototype, "maxJsonBytes", { get() { return 1n; }, configurable: true });', /RuntimeLimits\.maxJsonBytes \(set missing\)/],
    ['ParsedProgram.forge = function () {};', /ParsedProgram\.forge is not covered/],
    ['delete RuntimeSession.prototype.snapshot;', /RuntimeSession\.snapshot \(missing\)/],
    ['exports.extra = function () {};', /exports\.extra is not covered/],
  ];
  for (const [mutation, message] of mutations) expect(() => loadWith(mutation)).toThrow(message);
});

test('the package imports the guarded callback call and error conversions', () => {
  const bytes = readFileSync(join(dirname(packagePath), 'openui_wasm_bg.wasm'));
  const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes)).map(i => i.name);
  expect(imports.some(name => /^__wbg_call_/.test(name))).toBe(true);
  expect(imports.some(name => /^__wbg___wbindgen_throw_/.test(name))).toBe(true);
  const { glue, guard } = packageSource();
  expect(glue).toContain('handleError'); // the callback import catches throws
  expect(guard).toContain('getStringFromWasm0 = function');
  expect(guard).toContain('registerFunction: openuiSpec([openuiStr, openuiFunction])');
});

test('pointer fields and private helpers cannot free or forge objects', () => {
  const { s, library } = setup();
  const t = new wasm.RuntimeSession(library);
  kind(() => { s.__wbg_ptr = t.__wbg_ptr; }, 'InvalidHandle');
  kind(() => { s.__wbg_ptr = 0; }, 'InvalidHandle');
  kind(() => s.__destroy_into_raw(), 'InvalidHandle');
  kind(() => wasm.RuntimeSession.__wrap(t.__wbg_ptr), 'InvalidHandle');
  kind(() => wasm.ActionOutput.__wrap(1), 'InvalidHandle');
  expect(() => delete (s as any).__wbg_ptr).toThrow(TypeError);
  // Frozen: a guarded member, or a private helper the guarded free calls,
  // cannot be replaced to redirect a free.
  expect(() => { wasm.RuntimeSession.prototype.__destroy_into_raw = () => t.__wbg_ptr; }).toThrow(TypeError);
  expect(() => Object.defineProperty(wasm.RuntimeSession.prototype, 'free', { value() {} })).toThrow(TypeError);
  expect(() => { wasm.RuntimeSession.withLimits = () => {}; }).toThrow(TypeError);
  expect(s.version()).toBe(1n); expect(t.version()).toBe(0n);
  s.free(); t.free(); t.free(); library.free();
  kind(() => t.version(), 'Disposed');
});

test('opaque classes have no constructor and plain objects are not capabilities', () => {
  const { s, library, action } = setup();
  for (const name of ['ActionHandle', 'ContinuationHandle', 'EvaluationHandle', 'ParsedProgram', 'EvaluationOutput', 'ActionOutput', 'ReplacementOutput']) {
    kind(() => new wasm[name](), 'InvalidHandle');
  }
  kind(() => s.run({ steps: [] }, ''), 'InvalidHandle');
  kind(() => s.run(Object.create(wasm.ActionHandle.prototype), ''), 'InvalidHandle');
  kind(() => wasm.RuntimeSession.prototype.version.call({}), 'InvalidHandle');
  kind(() => s.setProgram(action), 'InvalidHandle');
  action.free(); s.free(); library.free();
});

test('the web package loads with the same guard', async () => {
  const dir = join(dirname(packagePath), '../web');
  const web = await import(join(dir, 'openui_wasm.js'));
  web.initSync({ module: readFileSync(join(dir, 'openui_wasm_bg.wasm')) });
  const { s, library, action } = setup(web);
  expect(web.RuntimeSession.prototype[Symbol.dispose]).toBe(web.RuntimeSession.prototype.free);
  s[Symbol.dispose]();
  kind(() => s.version(), 'Disposed');
  kind(() => { action.__wbg_ptr = 1; }, 'InvalidHandle');
  action.free(); library.free();
});
