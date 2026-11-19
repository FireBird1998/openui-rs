import { expect, test } from 'bun:test';
import { kind, wasm } from './support';

test('the bundle reader feeds draws, script refetches and parser diagnostics through the package', () => {
  using library = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  using session = new wasm.RuntimeSession(library);
  const program = 'q=Query("cloud",{},"waiting")\nroot=Box(q)';
  const raw = program + '\n]]>openui:scripts {"count":1}\n[{"name":"cloud","code":"A"}]';
  const bundle = JSON.parse(wasm.parseResponseBundle(raw, false));
  using parsed = wasm.parseProgram(bundle.program, library);
  using replacement = session.replaceProgram(parsed, undefined, undefined, true, { streaming: true, ...bundle });
  using waiting = replacement.evaluation;
  expect(waiting.isQueryLoading).toBe(true);
  expect(JSON.parse(waiting.json).root.props.value).toBe("waiting");
  using empty = session.takeQueryUpdate();
  expect(empty.callCount).toBe(0n);
  session.attachToolProvider();
  using first = session.evaluate({ streaming: false, ...bundle });
  using calls = session.takeQueryUpdate();
  using call = calls.callAt(0n);
  session.completeToolCall(call, '{"value":1}');
  session.setQueryExtraValues('cloud', '{"tenant":1}');
  using configured = session.evaluate({ streaming: false, ...bundle });
  using configuredCalls = session.takeQueryUpdate();
  using configuredCall = configuredCalls.callAt(0n);
  session.completeToolCall(configuredCall, '{"value":2}');
  using second = session.evaluate({ streaming: false, ...JSON.parse(wasm.parseResponseBundle(raw.replace('"A"', '"B"'), false)) });
  using refreshed = session.takeQueryUpdate();
  expect(refreshed.callCount).toBe(1n);
  using fresh = refreshed.callAt(0n);
  session.completeToolCall(fresh, '{"value":3}');
  using invalid = session.evaluate({ streaming: false, ...JSON.parse(wasm.parseResponseBundle(raw + '?', false)) });
  expect(JSON.parse(invalid.json).root.props.value).toBe(3);
  expect(invalid.isQueryLoading).toBe(false);
  expect(JSON.parse(invalid.parserErrorJson)).toEqual({ source: 'parser', code: 'parse-failed', message: 'Invalid scripts bundle' });
  using click = session.run(undefined, 'ignored');
  expect(click.events).toEqual([]);
});

test('response conversion bounds the scripts array and refuses reentry before the draw', () => {
  using lib = new wasm.ComponentLibrary('{}');
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 32n;
  using session = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  let reads = 0;
  const names = new Proxy(Array(33), { get(target, key) {
    if (key !== 'length') reads++;
    return Reflect.get(target, key);
  } });
  kind(() => session.evaluate({ streaming: false, scripts: names }), 'LimitExceeded');
  expect(reads).toBe(0);
  kind(() => session.evaluate({ streaming: false, scripts: [1] }), 'InvalidJson');
  kind(() => session.evaluate({ streaming: false, complete: 1 }), 'InvalidJson');
  kind(() => session.evaluate({ streaming: false, scripts: ['é'.repeat(16)] }), 'LimitExceeded');
  using draw = session.evaluate({ streaming: true, isBundle: true, scripts: [ 'cloud' ], get scriptRevision() {
    kind(() => session.dispose(), 'Worker');
    return 'A';
  } });
  expect(draw.isQueryLoading).toBe(false);
  using ignored = session.run(undefined, 'ignored');
  expect(ignored.events).toEqual([]);
});
