import { expect, spyOn, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { wasmHost } from '../../../examples/next-compare/scripts/compare-tools.mjs';
import { nextToolTurn, rawBoundaryCases, rawTranscript } from '../../../reference/tool-provider-scenarios';
import boundaries from '../../../conformance/golden/tool-provider-response-boundaries.json';
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

test('setProgram keeps clicks and Retry blocked until a successful draw', () => {
  using lib = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}}}');
  using s = new wasm.RuntimeSession(lib);
  using p = wasm.parseProgram('q=Query("cloud",{},0)\nroot=Box(q)', lib);
  s.attachToolProvider();
  using initial = s.replaceProgram(p, undefined, undefined, true);
  using first = s.takeQueryUpdate();
  using call = first.callAt(0n);
  s.completeToolCall(call, '{"failed":"offline"}');
  using blocked = s.evaluate({ streaming: true, isBundle: true, complete: false });
  s.setProgram(p);
  kind(() => s.evaluate({ streaming: false, scripts: [1] }), 'InvalidJson');
  s.retryFailedQueries();
  using click = s.run(undefined, 'click');
  using legacy = s.runLegacy('{"type":"custom"}', 'click');
  using ignored = s.takeQueryUpdate();
  expect([click.events.length, legacy.events.length, ignored.callCount]).toEqual([0, 0, 0n]);
  using bare = s.evaluate({ streaming: true });
  s.retryFailedQueries();
  using retry = s.takeQueryUpdate();
  using allowed = s.run(undefined, 'click');
  expect([allowed.events.length, retry.callCount]).toEqual([1, 1n]);
});

const boxSchema = '{"$defs":{"Box":{"properties":{"value":{}}}}}';

test('bare responses draw like the streaming parser, complete a trailing state name and preserve user edits', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  using raw = new wasm.RuntimeSession(library);
  using manual = new wasm.RuntimeSession(library);
  using parser = new wasm.StreamSession(library);
  for (const [source, streaming] of [
    ['$n=1\nroot=Box($d', true],
    ['$n=1\nroot=Box($d', false],
    ['$n=9\nroot=Box($n)', false],
  ] as const) {
    parser.set(source);
    using program = parser.program(!streaming);
    using replace = manual.replaceProgram(program, undefined, undefined, false, {streaming});
    using expected = replace.evaluation;
    using actual = raw.setResponse(source, streaming).evaluation;
    expect([actual.json, raw.snapshot()]).toEqual([expected.json, manual.snapshot()]);
    manual.set('$n', '42');
    raw.set('$n', '42');
  }
  expect(raw.get('$n')).toBe('42');
});

test('a regenerated message exposes only its winning metadata and holds a cut marker out of an open string', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  using s = new wasm.RuntimeSession(library);
  const previous = ']]>openui:content?name=old\nroot=Box("discarded")\n]]>openui:context\nold\n]]>openui:end\n';
  const start = previous + 'cut]]>openui:content?name=new+name&library=a&library=b&__proto__=skip\nroot=Box("shown';
  for (let cut = 1; cut <= ']]>openui:context'.length; cut++) {
    const {evaluation, name, attributes, context} = s.setResponse(start + '\r\n' + ']]>openui:context'.slice(0, cut), true);
    using output = evaluation;
    expect(JSON.parse(output.json).root.props.value).toBe('shown');
    expect([name, attributes, context]).toEqual(['new name', {name:'new name', library:'b'}, null]);
  }
  const framed = start + '")\r\n]]>openui:context\n';
  for (const [body, expected] of [['{"n":', '{"n":'], ['{"n":2}', {n:2}], ['1e400', Infinity]] as const) {
    const {evaluation, context} = s.setResponse(framed + body, true);
    using output = evaluation;
    expect(context).toEqual(expected);
    expect(JSON.parse(output.json).root.props.value).toBe('shown');
  }
  // Bundle parsing masks bare scripts lines. Message metadata uses the original text.
  const text = ']]>openui:content?name=\u{10f800}&unit=\ud800\nroot=Box(0)\n]]>openui:context\nliteral\n]]>openui:scripts\nignored';
  const {evaluation, name, attributes, context} = s.setResponse(text, false);
  using output = evaluation;
  expect([name, attributes.unit, context]).toEqual(['\u{10f800}', '\ud800', 'literal']);
});

test('redraw keeps a blocked response until a successful explicit draw or program handoff', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  using s = new wasm.RuntimeSession(library);
  using manual = wasm.parseProgram('root=Box("manual")', library);
  const text = ']]>openui:content\nq=Query("cloud",{},0)\nroot=Box("raw")';
  for (const handoff of ['explicit', 'set', 'replace']) {
    using framed = s.setResponse(text, true).evaluation;
    kind(() => s.evaluate({streaming: 1}), 'InvalidJson');
    using blocked = s.evaluate();
    using ignored = s.run(undefined, 'tap');
    expect([blocked.isQueryLoading, ignored.events.length]).toEqual([true, 0]);
    if (handoff === 'explicit') s.evaluate({streaming: false}).free();
    if (handoff === 'set') s.setProgram(manual);
    if (handoff === 'replace') s.replaceProgram(manual, undefined, undefined, false).free();
    using bare = s.evaluate();
    using allowed = s.run(undefined, 'tap');
    expect([bare.isQueryLoading, allowed.events.length]).toEqual([false, 1]);
    using again = s.setResponse(text, true).evaluation;
    expect(JSON.parse(again.json).root.props.value).toBe('raw');
  }
});

test('raw response size, depth and script-state refusals preserve the store, call and current evaluation', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  using parse = limits.evaluation;
  parse.programSize = 8192n;
  parse.depth = 8n;
  limits.evaluation = parse;
  limits.maxJsonBytes = 2048n;
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  s.attachToolProvider();
  using output = s.setResponse('$n=1\nq=Query("read",{},0)\nroot=Box(Action([@Set($n,2)]))', false).evaluation;
  using token = output.evaluation;
  using effects = s.takeQueryUpdate();
  using call = effects.callAt(0n);
  const old = [s.snapshot(), s.version()];
  const deep = '['.repeat(57) + '0' + ']'.repeat(57);
  for (const text of [
    ']]>openui:content?name=' + 'é'.repeat(4096) + '\nroot=Box(0)',
    'root=Box(0)\n]]>openui:scripts {"count":1}\n' + JSON.stringify([{name:'x',code:'é'.repeat(4096)}]),
    'root=Box(0)\n]]>openui:context\n' + deep,
    'root=Box(0)\n]]>openui:scripts ' + deep + '\n[]',
    'root=Box(0)\n]]>openui:scripts {"count":1}\n' + deep,
    'root=Box(0)\n]]>openui:scripts {"count":1}\n' + JSON.stringify([{name:'x',code:'z'.repeat(2048)}]),
    '$new=2\nroot=Box("' + 'V'.repeat(2100) + '")',
  ]) {
    kind(() => s.setResponse(text, false), 'LimitExceeded');
    expect([s.snapshot(), s.version()]).toEqual(old);
    expect(call.pending).toBe(true);
    using retained = s.actionAt(token, ['props','value']);
    using allowed = s.run(undefined, 'tap');
    expect(allowed.events.length).toBe(1);
  }
  s.completeToolCall(call, '{"value":9}');
  using drawn = s.evaluate();
  expect(JSON.parse(drawn.querySnapshotJson).q).toBe(9);
  const {evaluation, error} = s.setResponse('root=Box(0)\n]]>openui:scripts {"count":9007199254740991}\n[]', false);
  using invalid = evaluation;
  expect(error).toBe('Invalid scripts bundle');
});

test('raw response arguments, receivers, callbacks and freed sessions obey the package guard', () => {
  using lib = new wasm.ComponentLibrary('{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{}}}}');
  using s = new wasm.RuntimeSession(lib);
  const setter = s.setResponse;
  let coerced = 0;
  const object = {toString() { coerced++; return ''; }, valueOf() { coerced++; return false; }};
  kind(() => setter.call({}, '', true), 'InvalidHandle');
  kind(() => setter.call(lib, '', true), 'InvalidHandle');
  for (const [text, streaming] of [[object,true], [undefined,true], ['',object], ['',undefined]]) {
    kind(() => s.setResponse(text, streaming), 'InvalidJson');
  }
  expect(coerced).toBe(0);
  let callbacks = 0;
  s.registerFunction('Echo', () => {
    kind(() => s.setResponse(null, false), 'Worker');
    kind(() => s.free(), 'Worker');
    callbacks++;
    return '7';
  });
  using out = s.setResponse('root=Box(@Echo())', false).evaluation;
  expect([callbacks, JSON.parse(out.json).root.props.value]).toEqual([1, 7]);
  s.dispose();
  kind(() => s.setResponse(null, false), 'Disposed');
  s.free();
  kind(() => s.setResponse(null, false), 'Disposed');
});

test('the raw bundle driver matches renderer draws, clicks, deliveries and script-only refetches', async () => {
  const host = wasmHost(undefined, null);
  try {
    const expected = readFileSync(new URL('../../../conformance/golden/tool-provider-response.jsonl', import.meta.url), 'utf8');
    expect(await rawTranscript(host)).toBe(expected);
  } finally { host.dispose(); }
});

test('whole-response limits refuse renderer inputs and a split program surrogate follows bare wasm string lowering', () => {
  using lib = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  using parse = limits.evaluation;
  parse.programSize = 128n;
  parse.depth = 1n;
  limits.evaluation = parse;
  using small = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  using ordinary = new wasm.RuntimeSession(lib);
  for (const [i, {label, text, streaming}] of rawBoundaryCases.entries()) {
    const upstream = boundaries[i];
    expect(upstream.label).toBe(label);
    if (i < 3) {
      expect(upstream.value).toBe('small');
      kind(() => small.setResponse(text, streaming), 'LimitExceeded');
    } else {
      using out = ordinary.setResponse(text, streaming).evaluation;
      expect(upstream.value).toBe(streaming ? 'a\ud83d' : 'a😀');
      expect(JSON.parse(out.json).root.props.value).toBe(streaming ? 'a�' : upstream.value);
    }
  }
});

test('a refused raw replacement keeps the timer origin and the next successful registration gets its full interval', () => {
  let now = 100;
  using time = spyOn(performance, 'now').mockImplementation(() => now);
  const delays = [];
  using timer = spyOn(globalThis, 'setTimeout').mockImplementation((_, delay) => {
    delays.push(delay);
    return delays.length;
  });
  using clear = spyOn(globalThis, 'clearTimeout').mockImplementation(() => {});
  using lib = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 2048n;
  using s = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  let calls = 0;
  s.setToolProvider({read: () => ++calls}, {minimumMs:50n});
  now = 2000;
  kind(() => s.setResponse('root=Box("' + 'x'.repeat(2048) + '")', false), 'LimitExceeded');
  now = 450; // Would be refused as backwards if the failed draw committed its sample.
  using drawn = s.setResponse('q=Query("read",{},null,0.01)\nroot=Box(q)', false).evaluation;
  expect([calls, delays, drawn.isQueryLoading]).toEqual([1, [50], true]);
});

test('a refused append preserves pristine container identity on the next accepted edit', () => {
  using lib = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 1500n;
  using s = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  const prefix = '$xs=[1,2]\nroot=Box(1)\n';
  using first = s.setResponse(prefix, true).evaluation;
  const version = s.version();
  kind(() => s.setResponse(prefix + 'root=Box("' + 'z'.repeat(2000) + '")', true), 'LimitExceeded');
  using accepted = s.setResponse(prefix + '# more', true).evaluation;
  expect(s.version()).toBe(version);
  expect(accepted.json).toBe(first.json);
});

test('a scripts-only edit retains the previously registered mutation', () => {
  using lib = new wasm.ComponentLibrary(boxSchema);
  using s = new wasm.RuntimeSession(lib);
  s.attachToolProvider();
  const raw = '$tool="original"\nm=Mutation($tool)\nroot=Box(Action([@Run(m)]))\n]]>openui:scripts {"count":1}\n[{"name":"cloud","code":"A"}]';
  using first = s.setResponse(raw, false).evaluation;
  s.set('$tool', '"edited"');
  using edited = s.setResponse(raw.replace('"A"', '"B"'), false).evaluation;
  using token = edited.evaluation;
  using action = s.actionAt(token, ['props','value']);
  using run = s.run(action, 'tap');
  using update = s.takeQueryUpdate();
  using call = update.callAt(0n);
  expect(call.toolName).toBe('original');
  s.completeToolCall(call, '{"value":1}');
});

test('redraw callbacks see the accepted response and Retry waits for a settled bundle', async () => {
  using lib = new wasm.ComponentLibrary(boxSchema);
  using s = new wasm.RuntimeSession(lib);
  const raw = ']]>openui:content\nq=Query("read",{},0)\nroot=Box(q)';
  let calls = 0;
  const displays = [];
  s.setToolProvider({read() {
    if (++calls === 1) throw Error('offline');
    return 9;
  }}, {onChange() {
    using drawn = s.evaluate();
    displays.push(JSON.parse(drawn.json).root?.props.value);
  }});
  using initial = s.setResponse(raw, false).evaluation;
  await nextToolTurn();
  using blocked = s.setResponse(raw, true).evaluation;
  s.retryFailedQueries();
  await nextToolTurn();
  expect(calls).toBe(1);
  using settled = s.setResponse(raw, false).evaluation;
  s.retryFailedQueries();
  for (let turn = 0; turn < 8 && !displays.includes(9); turn++) await nextToolTurn();
  using final = s.evaluate();
  expect([calls, final.isQueryLoading, JSON.parse(final.querySnapshotJson).q]).toEqual([2, false, 9]);
  expect(displays).toContain(9);
});

test('raising evaluation limits keeps the package reader ceilings on raw responses', () => {
  using lib = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  using parse = limits.evaluation;
  parse.programSize = 2n * 1024n * 1024n;
  parse.depth = 1024n;
  limits.evaluation = parse;
  using s = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  const text = 'root=Box(1)\n]]>openui:context\n';
  kind(() => s.setResponse(text + 'é'.repeat(512 * 1024), true), 'LimitExceeded');
  kind(() => s.setResponse(text + '['.repeat(801), true), 'LimitExceeded');
  using draw = s.setResponse(text + '[]', false).evaluation;
  expect(JSON.parse(draw.json).root.props.value).toBe(1);
});

test('the comparison page’s availability bundle draws the delivered Cloud script result', async () => {
  const {response, tools} = await import('../../../examples/next-compare/lib/response-example');
  const schema = readFileSync(new URL('../../../conformance/imported/schemas/react-ui.json', import.meta.url), 'utf8');
  using lib = new wasm.ComponentLibrary(schema);
  using s = new wasm.RuntimeSession(lib);
  const values = [];
  s.setToolProvider(tools, {onChange() {
    using output = s.evaluate();
    values.push(JSON.parse(output.json).root?.props.text);
  }});
  const {name, evaluation} = s.setResponse(response, false);
  using initial = evaluation;
  expect([name, JSON.parse(initial.json).root.props.text]).toEqual(['Availability', 'Checking']);
  for (let turn = 0; turn < 8 && !values.includes('Available'); turn++) await nextToolTurn();
  expect(values).toContain('Available');
});

test('a raw program without a root statement uses the root component from the session’s prompt spec', () => {
  using lib = new wasm.ComponentLibrary(boxSchema);
  const program = 'description=Box("implicit entry")';
  for (const spec of [{root:'Box'}, {library:{root:'Box'}}]) {
    using s = new wasm.RuntimeSession(lib, JSON.stringify(spec));
    using drawn = s.setResponse(program, false).evaluation;
    expect(JSON.parse(drawn.json).root.props.value).toBe('implicit entry');
  }
});
