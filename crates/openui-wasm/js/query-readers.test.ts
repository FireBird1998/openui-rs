import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { nextToolTurn } from '../../../reference/tool-provider-scenarios';
import { rendererSelfCases } from '../../../reference/query-self-wait';
import { kind, wasm } from './support';

test('completion calls only stale reader functions under the session guard', () => {
  using library = new wasm.ComponentLibrary(JSON.stringify({
    $defs: {Box: {properties: {v: {}}}},
    functions: {Read: {params: {type: 'object', properties: {x:{}}}}}
  }));
  using s = new wasm.RuntimeSession(library);
  const seen = [];
  s.registerFunction('Read', args => {
    kind(() => s.takeQueryUpdate(), 'Worker');
    const {x} = JSON.parse(args);
    seen.push(args);
    return JSON.stringify(x);
  });
  s.attachToolProvider();
  using p = wasm.parseProgram('a=Query("a")\nb=Query("b",{v:@Read(a)})\nc=Query("c",@Read(99))\nroot=Box(b)', library);
  using draw = s.replaceProgram(p, undefined, undefined, true);
  using initial = s.takeQueryUpdate();
  expect(initial.callCount).toBe(2n);
  expect(initial.readerRoundsLimited).toBe(false);
  expect(seen).toEqual(['{}', '{"x":99}']);
  using src = initial.callAt(0n);
  s.completeToolCall(src, '{"value":42}');
  using update = s.takeQueryUpdate();
  using reader = update.callAt(0n);
  expect(seen).toEqual(['{}', '{"x":99}', '{"x":42}']);
  expect(reader.argumentsJson).toBe('{"v":42}');
  s.completeToolCall(reader, '{"value":"ok"}');
  using live = s.evaluate();
  expect(seen).toEqual(['{}', '{"x":99}', '{"x":42}']);
  expect(JSON.parse(live.json).root.props.v).toBe('ok');
});

test('a cached cycle reports the reader round cut once through the drain', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"v":{}}}}}');
  using s = new wasm.RuntimeSession(library);
  s.attachToolProvider();
  const anchors = '\na1=Query("A",{v:1})\nb0=Query("B",{v:0})';
  using p = wasm.parseProgram('a=Query("A",{v:0})\nb=Query("B",{v:1})' + anchors + '\nroot=Box(a)',library);
  using first = s.replaceProgram(p, undefined, undefined, true);
  using effects = s.takeQueryUpdate();
  for (const [i, value] of [1,1,0,0].entries()) {
    using call = effects.callAt(BigInt(i));
    s.completeToolCall(call, JSON.stringify({value}));
  }
  using cycle = wasm.parseProgram('a=Query("A",{v:b})\nb=Query("B",{v:a})' + anchors +
    '\nholdA=Query("A",{v:0})\nholdB=Query("B",{v:1})\nroot=Box(a)',library);
  using next = s.replaceProgram(cycle, undefined, undefined, true);
  using cut = s.takeQueryUpdate();
  expect(cut.readerRoundsLimited).toBe(true);
  expect(cut.callCount).toBe(0n);
  using again = s.takeQueryUpdate();
  expect(again.readerRoundsLimited).toBe(false);
});

test('setResponse wakes self-readers on previous keys at the same completion as manual draws', async () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  const upstream = readFileSync(new URL('../../../conformance/golden/session-self-wait.jsonl', import.meta.url), 'utf8')
    .trimEnd().split('\n').map(line => JSON.parse(line));
  let row = 0;
  for (const scenario of rendererSelfCases()) {
    using manual = new wasm.RuntimeSession(library);
    using raw = new wasm.RuntimeSession(library);
    manual.attachToolProvider();
    const receipts = [], driven = [];
    raw.setToolProvider({u(args) {
      const result = Promise.withResolvers();
      driven.push({args, result});
      return result.promise;
    }});
    let streaming = true, text = '';
    try {
      for (const step of scenario.steps) {
        const renderer = upstream[row++];
        // The manager replay checks the final targeted invalidate.
        if ('invalidate' in step) continue;
        const before = driven.length;
        if ('replace' in step || 'evaluate' in step) streaming = !!step.streaming;
        if ('finish' in step) {
          manual.completeToolCall(receipts[step.finish], JSON.stringify({value:step.value}));
          driven[step.finish].result.resolve(step.value);
          await nextToolTurn();
          if (step.finish === 1) expect(driven.slice(before).map(c => c.args)).toEqual([{x:step.value}]);
        }
        if ('set' in step) {
          manual.set('$n', JSON.stringify(step.set));
          raw.set('$n', JSON.stringify(step.set));
        }
        let output;
        if ('replace' in step) {
          text = step.replace;
          using program = wasm.parseProgram(text, library);
          using replaced = manual.replaceProgram(program, undefined, undefined, false, {streaming});
          output = replaced.evaluation;
        } else output = manual.evaluate({streaming});
        using expected = output;
        using actual = raw.setResponse(text, streaming).evaluation;
        await nextToolTurn();
        expect(actual.json).toBe(expected.json);
        expect(actual.querySnapshotJson).toBe(expected.querySnapshotJson);
        using effects = manual.takeQueryUpdate();
        const calls = [];
        for (let j = 0n; j < effects.callCount; j++) {
          const call = effects.callAt(j);
          receipts.push(call);
          calls.push({toolName:call.toolName, args:JSON.parse(call.argumentsJson)});
        }
        expect(driven.slice(before).map(c => ({toolName:'u', args:c.args}))).toEqual(calls);
        const tree = JSON.parse(actual.json);
        if ('finish' in step && step.finish === 1) {
          expect(tree.root.props.value).toEqual([step.value, step.value]);
          expect(renderer.calls).toEqual(step.value === null ? calls : []);
          expect(renderer.tree.root.props.value).toEqual([step.value, null]);
        } else {
          expect(tree).toEqual(renderer.tree);
          expect(calls).toEqual(renderer.calls);
        }
      }
    } finally {
      for (const call of receipts) call.free();
    }
  }
  expect(row).toBe(upstream.length);
});

test('a refused raw response preserves the undrained round cut and its pending source', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"v":{}}}}}');
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 16384n;
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  s.attachToolProvider();
  const keep = '\na1=Query("A",{v:1})\nb0=Query("B",{v:0})\ns=Query("source")\nr=Query("reader",{v:s})\nroot=Box(r)';
  using seed = s.setResponse('a=Query("A",{v:0})\nb=Query("B",{v:1})' + keep, false).evaluation;
  using first = s.takeQueryUpdate();
  expect(first.callCount).toBe(5n);
  using source = first.callAt(4n);
  for (const [i, value] of [1,1,0,0].entries()) {
    using call = first.callAt(BigInt(i));
    s.completeToolCall(call, JSON.stringify({value}));
  }
  using cut = s.setResponse('a=Query("A",{v:b})\nb=Query("B",{v:a})\nholdA=Query("A",{v:0})\nholdB=Query("B",{v:1})' + keep, false).evaluation;
  kind(() => s.setResponse('q=Query("other")\nroot=Box("' + 'x'.repeat(17000) + '")', false), 'LimitExceeded');
  expect(source.pending).toBe(true);
  using undrained = s.takeQueryUpdate();
  expect(undrained.readerRoundsLimited).toBe(true);
  expect(undrained.callCount).toBe(0n);
  using after = s.evaluate();
  expect(after.querySnapshotJson).toBe(cut.querySnapshotJson);
  using empty = s.takeQueryUpdate();
  expect(empty.readerRoundsLimited).toBe(false);
  s.completeToolCall(source, '{"value":7}');
  using awakened = s.takeQueryUpdate();
  expect(awakened.readerRoundsLimited).toBe(true);
  expect(awakened.callCount).toBe(1n);
  using reader = awakened.callAt(0n);
  expect([reader.toolName, reader.argumentsJson]).toEqual(['reader','{"v":7}']);
  expect(source.pending).toBe(false);
});
