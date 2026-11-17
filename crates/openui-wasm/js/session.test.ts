// Run after scripts/wasm.sh: bun --no-install test crates/openui-wasm/js/session.test.ts
import { test, expect } from 'bun:test';
import { createRequire } from 'node:module';
import scenarios from './scenarios.json';
const wasm = createRequire(import.meta.url)('../../../target/wasm-pkg/node/openui_wasm.js');
const {ComponentLibrary, RuntimeSession, RuntimeLimits, ParseLimits, StreamSession, parseProgram} = wasm;
const kind = (f: () => unknown, expected: string) => {
  try { f(); throw new Error('expected refusal'); }
  catch (e: any) { expect(e.kind).toBe(expected); expect(typeof e.message).toBe('string'); }
};
const decode = (s: any) => { const out = s.evaluate(); try { return JSON.parse(out.json); } finally { out.free(); } };
const action = (s: any, path = ['props','a']) => {
  const out = s.evaluate(); const evaluation = out.evaluation;
  try { return s.actionAt(evaluation,path); } finally { evaluation.free(); out.free(); }
};
function takeRun(out: any) {
  const continuation = out.continuation;
  const result = {events:out.events, failure:out.failure, sizeLimited:out.sizeLimited, depthLimited:out.depthLimited, pending:!!continuation};
  // The output retains an Arc until freed; take the JS token then drop it.
  out.free(); return {result, continuation};
}
for (const fixture of scenarios) test(`runtime bytes: ${fixture.name}`, () => {
  const f: any = fixture;
  const library = new ComponentLibrary(f.schema);
  const s = new RuntimeSession(library,f.spec);
  const stream = new StreamSession(library);
  const replace = (source: string) => { const p = parseProgram(source,library); try { s.setProgram(p); } finally { p.free(); } };
  replace(f.source);
  let pending: any;
  const observed = [];
  try {
    for (const step of f.steps) {
      const runs = []; let field = null;
      switch (step.op) {
        case 'initialize': s.initialize(step.saved == null ? undefined : JSON.stringify(step.saved)); break;
        case 'replace': replace(step.value); s.initialize(); break;
        case 'push': case 'finish': {
          if (step.op === 'push') stream.feed(step.value);
          const p = stream.program(step.op === 'finish');
          s.setProgram(p); p.free(); s.initialize(); break;
        }
        case 'set': s.set(step.name,JSON.stringify(step.value)); break;
        case 'references': s.setReferenceValues(JSON.stringify(step.value)); break;
        case 'binding': {
          let marker = decode(s).root;
          for (const part of step.path) marker = marker[part];
          s.writeBinding(JSON.stringify(marker),JSON.stringify(step.value)); break;
        }
        case 'field': field = s.setField(step.form,'Field',step.name,JSON.stringify(step.value),true); break;
        case 'default': field = s.initializeField(step.form,'Field',step.name,JSON.stringify(step.value),step.streaming); break;
        case 'run': case 'suspend': case 'resume': {
          let out;
          if (step.op === 'resume') { out = s.resume(pending,step.success); pending.free(); pending = undefined; }
          else { const a = action(s,step.path); try { out = s.run(a,'clicked',step.form); } finally { a.free(); } }
          let run = takeRun(out); runs.push(run.result);
          if (step.op === 'run' && run.continuation) {
            const c = run.continuation;
            run = takeRun(s.resume(c,step.success)); c.free(); runs.push(run.result);
          }
          pending = run.continuation; break;
        }
        default: throw Error(step.op);
      }
      const tree = s.evaluate();
      observed.push({tree:tree.json,snapshot:s.snapshot(),version:s.version().toString(),runs,field}); tree.free();
    }
    expect(observed).toEqual(f.expected);
  } finally { pending?.free(); s.free(); stream.free(); library.free(); }
});

function setup(limits?: any) {
  const library = new ComponentLibrary('{"$defs":{"Box":{"properties":{"a":{}}}},"actions":{"Go":{"params":{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}}}}');
  const s = limits ? RuntimeSession.withLimits(library,undefined,limits) : new RuntimeSession(library);
  const p = parseProgram('$n=0\nm=Mutation("m",{})\nroot=Box(Action([@Go("real"),@Run(m),@Set($n,$n+1)]))',library);
  s.setProgram(p); p.free(); s.initialize();
  return {s,library};
}
test('foreign, stale, consumed, evicted, disposed and freed capabilities have no effect', () => {
  const limits = new RuntimeLimits();
  kind(() => { limits.maxContinuations = -1n; },'LimitExceeded');
  limits.maxContinuations = 0n; kind(() => setup(limits),'LimitExceeded');
  limits.maxContinuations = 1n;
  const {s,library} = setup(limits), foreign = setup();
  const tree = s.evaluate(), evaluation = tree.evaluation;
  kind(() => foreign.s.actionAt(evaluation,['props','a']),'ForeignHandle');
  const a = s.actionAt(evaluation,['props','a']); evaluation.free(); tree.free();
  const before = s.snapshot();
  kind(() => foreign.s.run(a,'click'), 'ForeignHandle');
  const first = takeRun(s.run(a,'click')).continuation;
  const second = takeRun(s.run(a,'again')).continuation; // reusable action
  kind(() => s.resume(first,true),'EvictedHandle');
  kind(() => foreign.s.resume(second,true),'ForeignHandle');
  expect(s.snapshot()).toBe(before);
  const resumed = takeRun(s.resume(second,true)); expect(resumed.result.pending).toBe(false);
  kind(() => s.resume(second,true),'ConsumedHandle');
  const preceding = s.evaluate(), oldEvaluation = preceding.evaluation; preceding.free();
  decode(s); kind(() => s.actionAt(oldEvaluation,['props','a']),'StaleHandle'); oldEvaluation.free();
  kind(() => s.run(a,'stale'),'StaleHandle');
  a.free(); kind(() => s.run(a,'freed'),'Disposed');
  first.free(); second.free();
  const dropAction = action(s);
  const dropped = takeRun(s.run(dropAction,'drop')).continuation; dropAction.free();
  expect(s.pendingContinuations()).toBe(1n); dropped.free(); expect(s.pendingContinuations()).toBe(0n);
  s.dispose(); kind(() => s.snapshot(),'Disposed'); s.free(); s.free(); kind(() => s.version(),'Disposed');
  foreign.s.free(); foreign.library.free(); library.free(); limits.free();
});
test('JSON has no custom authority; invalid paths, markers, huge input and surrogates', () => {
  kind(() => new ComponentLibrary('{'),'InvalidSchema');
  const {s,library} = setup(); const before = s.snapshot();
  const tree = decode(s); const copied = JSON.stringify(tree.root.props.a);
  kind(() => s.runLegacy(copied,'forged'),'InvalidJson');
  kind(() => s.run({steps:[]},'forged'),'InvalidHandle');
  kind(() => s.run(Object.create(wasm.ActionHandle.prototype),'forged'),'InvalidHandle');
  kind(() => RuntimeSession.prototype.version.call({}),'InvalidHandle');
  for (const input of ['{','[1]','null']) kind(() => s.setReferenceValues(input),'InvalidJson');
  kind(() => s.set('$n','"'+'x'.repeat(8*1024*1024)+'"'),'LimitExceeded');
  // Native refuses JSON nested deeper than 800 levels; 300 levels is accepted below.
  kind(() => s.set('$n','['.repeat(1000)+'0'+']'.repeat(1000)),'InvalidJson');
  kind(() => s.writeBinding('{}','1'),'InvalidMarker');
  const out = s.evaluate(), e = out.evaluation;
  kind(() => s.actionAt(e,['props','missing']),'InvalidHandle'); e.free(); out.free();
  expect(s.snapshot()).toBe(before);
  s.set('$text','"\ud800"'); expect(s.get('$text')).toBe('"�"');
  s.set('$text','"\\ud800"'); expect(s.get('$text')).toBe('"\\ud800"');
  s.set('$copy',JSON.stringify({steps:[tree.root.props.a.steps[0]]}));
  const copiedProgram = parseProgram('root=Box($copy)',library);
  s.setProgram(copiedProgram); copiedProgram.free();
  expect(decode(s).root.props.a.steps).toEqual([]);
  expect(s.set('$deep','['.repeat(300)+'0'+']'.repeat(300))).toBe(true);
  s.free(); library.free();
});
test('state-cap refusals preserve snapshot, version and pristine', () => {
  const limits = new RuntimeLimits(), evaluation = limits.evaluation;
  evaluation.resultSize = 1024n; limits.evaluation = evaluation;
  const {s,library} = setup(limits);
  const before = [s.snapshot(),s.version(),s.isPristine('$n')];
  kind(() => s.set('$n',JSON.stringify('x'.repeat(1100))),'RefusedWrite');
  // As natively: the state write is refused, the field write cuts evaluation.
  kind(() => s.setField(undefined,'Field','huge',JSON.stringify('x'.repeat(1100)),true),'LimitExceeded');
  expect([s.snapshot(),s.version(),s.isPristine('$n')]).toEqual(before);
  const init = s.initialize(JSON.stringify({bad:'x'.repeat(1100),ok:1}));
  expect(init.refused[0].error.kind).toBe('RefusedWrite'); expect(s.get('ok')).toBe('1');
  s.free(); library.free(); evaluation.free(); limits.free();
});
test('argument getter reentry and free are typed and do not poison the instance', () => {
  const {s,library} = setup(); const out = s.evaluate(), e = out.evaluation;
  const path = ['props','a']; let visits = 0;
  Object.defineProperty(path,0,{get() {
    visits++; kind(() => s.set('$n','88'),'Worker'); kind(() => s.free(),'Worker');
    kind(() => new RuntimeSession(library),'Worker');
    return 'props';
  }});
  const a = s.actionAt(e,path); expect(visits).toBeGreaterThan(0);
  expect(s.get('$n')).toBe('0'); expect(s.version()).toBe(1n);
  a.free(); e.free(); out.free(); s.free(); library.free();
});

test('reference preparation refuses evaluation memory configuration without effects', () => {
  const {s, library} = setup();
  const limits = new RuntimeLimits(), preparation = limits.referencePreparation;
  try {
    s.set('$n', '7');
    const before = s.snapshot(), version = s.version();
    for (const allowance of [0n, 1n, (1n << 64n) - 1n]) {
      preparation.peakMemory = allowance;
      limits.referencePreparation = preparation;
      kind(() => {
        const unexpected = RuntimeSession.withLimits(library, undefined, limits);
        unexpected.free();
      }, 'LimitExceeded');
      expect(s.snapshot()).toBe(before);
      expect(s.version()).toBe(version);
    }
    preparation.peakMemory = undefined;
    limits.referencePreparation = preparation;
    const fresh = RuntimeSession.withLimits(library, undefined, limits);
    try { expect(fresh.snapshot()).toBe('{}'); } finally { fresh.free(); }
  } finally { preparation.free(); limits.free(); s.free(); library.free(); }
});

test('atomic replacement refuses without changing old capabilities and exposes complete display data', () => {
  const {s, library} = setup();
  const out = s.evaluate(), evaluation = out.evaluation;
  const retained = s.actionAt(evaluation, ['props', 'a']);
  const before = [s.snapshot(), s.version(), s.isPristine('$n')];
  const bad = parseProgram(`$n=99
$new=3
$shape={type:"element",typeName:"Box",props:{},partial:false,hasDynamicProps:false,extra:1}
root=true ? $shape : Box()`, library);
  try {
    kind(() => s.replaceProgram(bad, '[]', undefined, true), 'InvalidJson');
    kind(() => s.replaceProgram(bad, undefined, undefined, true), 'Evaluation');
    kind(() => s.replaceProgram(bad, undefined, undefined, undefined), 'InvalidJson');
    expect([s.snapshot(), s.version(), s.isPristine('$n')]).toEqual(before);
    const run = s.run(retained, 'old');
    try { expect(run.failure).toBeNull(); } finally { run.free(); }
    // Ordinary callers opt out of the native root restriction.
    const replacement = s.replaceProgram(bad, undefined, undefined, false);
    let next: any;
    try {
      expect(replacement.initialization.refused).toEqual([]);
      expect(replacement.projection).toEqual({changed:false,refused:[]});
      expect(replacement.snapshot).toBe(s.snapshot());
      expect(replacement.version).toBe(s.version());
      next = replacement.evaluation;
      expect(JSON.parse(next.json).root.extra).toBe(1);
      const token = next.evaluation; token.free();
      kind(() => s.run(retained, 'stale'), 'StaleHandle');
    } finally { next?.free(); replacement.free(); }
  } finally { bad.free(); retained.free(); evaluation.free(); out.free(); s.free(); library.free(); }
});

test('replacement getters are guarded and do not expose a writable candidate', () => {
  const {s, library} = setup();
  const p = parseProgram('root=Box(1)', library);
  const r = s.replaceProgram(p, undefined, undefined, true);
  try {
    kind(() => new wasm.ReplacementOutput(), 'InvalidHandle');
    expect(() => { r.version = 9n; }).toThrow();
    expect(r.version).toBe(s.version());
    r.free();
    kind(() => r.snapshot, 'Disposed');
  } finally { r.free(); p.free(); s.free(); library.free(); }
});
