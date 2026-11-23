// One maker per exported class (and the two result-only ones), shared by the
// tests that must visit every class: pointer-redirection.test.ts, private-helpers.test.ts.
import { expect } from 'bun:test';
import { wasm, setup, suspend, SCHEMA } from './support';

/** A fresh instance, a read that depends on its memory, and its cleanup. */
export type Made = { x: any, probe: () => unknown, done: () => void };
export type Case = { name: string, make: (i: number) => Made, construct?: (i: number) => unknown[] };

export const queryProgram = (i: number) => `q=Query("read${i}",{},"loading")\nroot=Box(q)`;
/** A session with a manual tool provider and a query waiting for its call. */
export function queryWorld(i: number) {
  const library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  const s = new wasm.RuntimeSession(library);
  s.attachToolProvider();
  const p = wasm.parseProgram(queryProgram(i), library);
  const replaced = s.replaceProgram(p, undefined, undefined, false);
  const effects = s.takeQueryUpdate();
  return { library, s, effects, done: () => { effects.free(); replaced.free(); p.free(); s.free(); library.free(); } };
}
/** A session showing Box(i). */
export function drawWorld(i: number) {
  const library = new wasm.ComponentLibrary(SCHEMA);
  const s = new wasm.RuntimeSession(library);
  const p = wasm.parseProgram(`root=Box(${i})`, library);
  s.setProgram(p); p.free();
  return { library, s, done: () => { s.free(); library.free(); } };
}

export const cases: Case[] = [
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
export const classOf = (x: any) => Object.getPrototypeOf(x).constructor;
