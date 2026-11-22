// Shared by the built-package tests. Run scripts/wasm.sh first, then
// bun --no-install test crates/openui-wasm/js
import { expect } from 'bun:test';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { dirname } from 'node:path';

const require = createRequire(import.meta.url);
export const packagePath = require.resolve('../../../target/wasm-pkg/node/openui_wasm.js');
export const wasm = require(packagePath);
export const SCHEMA = '{"$defs":{"Box":{"properties":{"a":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}}}}';
export const PROGRAM = '$n=0\nm=Mutation("m",{})\nroot=Box(Action([@Run(m),@Set($n,$n+1)]))';

/** The thrown error's own `kind`, or a failure if nothing was thrown. */
export function kind(call: () => unknown, expected: string) {
  let thrown: any;
  try { call(); } catch (error) { thrown = error; }
  expect(thrown).toBeInstanceOf(Error);
  expect(Object.getOwnPropertyDescriptor(thrown, 'kind')?.value).toBe(expected);
  expect(typeof thrown.message).toBe('string');
  return thrown;
}

/** A session on PROGRAM with $n = 0 and the action at props.a. */
export function setup(w: any = wasm) {
  const library = new w.ComponentLibrary(SCHEMA);
  const s = new w.RuntimeSession(library);
  const p = w.parseProgram(PROGRAM, library); s.setProgram(p); p.free(); s.initialize();
  const out = s.evaluate(), evaluation = out.evaluation;
  const action = s.actionAt(evaluation, ['props', 'a']); evaluation.free(); out.free();
  return { library, s, action };
}
/** Run the action up to its mutation; returns the pending continuation. */
export function suspend(s: any, action: any) {
  const out = s.run(action, 'click'); const continuation = out.continuation; out.free();
  return continuation;
}

/** The generated glue and the appended guard, as built. */
export function packageSource() {
  const source = readFileSync(packagePath, 'utf8');
  const at = source.indexOf('// Appended inside both wasm-bindgen modules');
  if (at < 0) throw new Error('package guard not found in the built package');
  return { glue: source.slice(0, at), guard: source.slice(at) };
}
/** Evaluate the built package with `mutation` run between glue and guard. */
export function loadWith(mutation: string, host?: object) {
  const { glue, guard } = packageSource();
  const module = { exports: {} as any };
  new Function('exports', 'require', 'module', '__filename', '__dirname', 'host', `${glue}\n${mutation}\n${guard}`)(
    module.exports, require, module, packagePath, dirname(packagePath), host);
  return module.exports;
}
