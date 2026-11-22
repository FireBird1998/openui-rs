import { expect, spyOn, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { wasmHost } from '../../../examples/next-compare/scripts/compare-tools.mjs';
import { nextToolTurn, rawBoundaryCases, rawTranscript } from '../../../reference/tool-provider-scenarios';
import boundaries from '../../../conformance/golden/tool-provider-response-boundaries.json';
import { kind, loadWith, wasm } from './support';

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

// ADR 0015: setResponse sends only what continues the last accepted text.
// A fresh session reading the whole text is today's full re-read; the
// streaming session must match it on every prefix, in every chunking and
// after edits and refusals.
function drawOf(session: InstanceType<typeof wasm.RuntimeSession>, text: string, streaming: boolean) {
  const { evaluation, name, attributes, context, error } = session.setResponse(text, streaming);
  using output = evaluation;
  // The store keeps values across programs, so only the draw is compared.
  return JSON.stringify([output.json, output.isQueryLoading, name, attributes, context, error]);
}

function freshDraw(library: InstanceType<typeof wasm.ComponentLibrary>, text: string, streaming: boolean) {
  using fresh = new wasm.RuntimeSession(library);
  return drawOf(fresh, text, streaming);
}

test('a response streamed in chunks draws as a fresh read of each prefix, edits and lone surrogates included', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  const program = '$n=1\nrows=[\n  Box("a"),\n  Box("b")\n]\nroot=Box(rows)\n';
  const texts = [
    program,
    program.replaceAll('\n', '\r\n'),
    ']]>openui:content?name=Daily\r\n' + program + ']]>openui:context\r\n{"user":"😀"}\r\n]]>openui:end\r\n',
    ']]>openui:content\n' + program + 'cut]]>openui:content?name=retry\n' + program + '\n]]>openui:scripts {"count":1}\n[{"name":"a","code":"1"}]\n]]>openui:end\n',
    'root=Box("😀 \ud800 é")\n]]>openui:end\n',
  ];
  let seed = 116;
  const next = () => (seed = (seed * 1103515245 + 12345) % 2147483648);
  for (const text of texts) {
    for (const size of [1, 3, 0]) {
      using s = new wasm.RuntimeSession(library);
      let at = 0;
      while (at < text.length) {
        at = Math.min(text.length, at + (size || 1 + next() % 17));
        const prefix = text.slice(0, at);
        expect(drawOf(s, prefix, true)).toBe(freshDraw(library, prefix, true));
      }
      expect(drawOf(s, text, false)).toBe(freshDraw(library, text, false));
      // Edits that are not appends: a shrink, a changed middle, the same length.
      const middle = text.length >> 1;
      for (const edit of [text.slice(0, middle), text.slice(0, middle) + 'x' + text.slice(middle + 1), text.slice(0, -1) + 'q', text]) {
        expect(drawOf(s, edit, true)).toBe(freshDraw(library, edit, true));
      }
    }
  }
});

test('a refused response update leaves the reader where it was for the next append', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 1024n;
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  using control = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const head = ']]>openui:content\nroot=Box("a")\n';
  expect(drawOf(s, head, true)).toBe(drawOf(control, head, true));
  // Too large to draw: refused after the read, which must not be kept.
  kind(() => s.setResponse(head + 'root=Box("' + 'x'.repeat(4096) + '")\n', true), 'LimitExceeded');
  for (const more of ['second=Box("b")\n', 'second=Box("b")\nroot=Box(second)\n']) {
    expect(drawOf(s, head + more, true)).toBe(drawOf(control, head + more, true));
  }
});

test('each raw draw reports the renderer blocked flag, which leaves the click gate as it was', () => {
  const corpus = JSON.parse(readFileSync(new URL('../../../conformance/session-blocked.json', import.meta.url), 'utf8'));
  const golden = readFileSync(new URL('../../../conformance/golden/session-blocked.jsonl', import.meta.url), 'utf8')
    .trimEnd().split('\n').map(row => JSON.parse(row));
  using library = new wasm.ComponentLibrary(JSON.stringify(corpus.schema));
  using s = new wasm.RuntimeSession(library);
  const [{steps}] = corpus.scenarios;
  expect(golden).toHaveLength(steps.length);
  let token;
  const seen = [];
  for (const [index, step] of steps.entries()) {
    let events = [];
    if ('click' in step) {
      using action = s.actionAt(token, ['props', step.click]);
      using run = s.run(action, 'click');
      events = run.events;
    }
    // Every other step redraws the last response, as the reference does.
    using output = 'response' in step ? s.setResponse(step.response, step.streaming ?? false).evaluation : s.evaluate();
    token?.free();
    token = output.evaluation;
    seen.push(output.blocked);
    expect([index, output.blocked, events.length]).toEqual([index, golden[index].blocked, golden[index].events.length]);
  }
  token?.free();
  expect(seen).toContain(true);
  expect(seen).toContain(false);
});

test('a refused raw update leaves the blocked flag the last draw reported, until dispose', () => {
  using library = new wasm.ComponentLibrary(boxSchema);
  using limits = new wasm.RuntimeLimits();
  using parse = limits.evaluation;
  parse.programSize = 4096n;
  limits.evaluation = parse;
  using s = wasm.RuntimeSession.withLimits(library, undefined, limits);
  const bundle = ']]>openui:content\nroot=Box(0)\n]]>openui:scripts {"count":1}\n[{"name":"cloud","code":"A"}]';
  for (const [text, streaming, blocked] of [[bundle, true, true], [bundle, false, false], [bundle + '?', false, true], ['root=Box(0)', true, true], ['root=Box(0)', false, false]] as const) {
    using drawn = s.setResponse(text, streaming).evaluation;
    expect(drawn.blocked).toBe(blocked);
    kind(() => s.setResponse('root=Box("' + 'V'.repeat(5000) + '")', !streaming), 'LimitExceeded');
    using redrawn = s.evaluate();
    expect(redrawn.blocked).toBe(blocked);
  }
  using invalid = s.setResponse(bundle + '?', false).evaluation;
  s.pauseQueries();
  using paused = s.evaluate();
  s.resumeQueries();
  using resumed = s.evaluate();
  expect([paused.blocked, resumed.blocked]).toEqual([true, true]);
  using p = wasm.parseProgram('root=Box(1)', library);
  using replaced = s.replaceProgram(p, undefined, undefined, false);
  using replacedOutput = replaced.evaluation;
  expect(replacedOutput.blocked).toBe(false);
  s.dispose();
  kind(() => s.evaluate(), 'Disposed');
});

test('the raw draw export holds the scripts JSON to maxJsonBytes before reading it', () => {
  // The generated method as it is before the guard wraps it: a host calling
  // the raw export (unsupported, ADR 0007) with text the guard would refuse.
  const w = loadWith('globalThis.__openuiRawEvaluate = RuntimeSession.prototype.evaluate;');
  const raw = (globalThis as any).__openuiRawEvaluate;
  delete (globalThis as any).__openuiRawEvaluate;
  using lib = new w.ComponentLibrary('{}');
  using limits = new w.RuntimeLimits();
  limits.maxJsonBytes = 32n;
  using session = w.RuntimeSession.withLimits(lib, undefined, limits);
  const draw = (scriptsJson: string) => raw.call(session, { streaming: false, isBundle: true, complete: true, scriptsJson });
  // Rust's refusals arrive as `openui:<Kind>` text. A draw Rust accepted
  // still throws here, from the guard's private output wrapper, not Rust.
  const refusal = (scriptsJson: string) => {
    try { draw(scriptsJson); } catch (thrown) {
      const text = String(thrown);
      return text.includes('private generated helper') ? 'drawn' : text.split('\n')[0];
    }
    return 'drawn';
  };
  // One byte over the limit and not JSON: refused for its length, so never read.
  expect(refusal('!'.repeat(33))).toBe('openui:LimitExceeded');
  expect(refusal('!'.repeat(32))).toBe('openui:InvalidJson');
  // Four bytes per name: eight empty names fit 32 bytes, nine do not, though
  // their JSON (28 bytes) does.
  expect(refusal(JSON.stringify(Array(8).fill('')))).toBe('drawn');
  expect(refusal(JSON.stringify(Array(9).fill('')))).toBe('openui:LimitExceeded');
});

test('the package charges four bytes per script name, as Rust does', () => {
  using lib = new wasm.ComponentLibrary('{}');
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 32n;
  using session = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  session.evaluate({ streaming: false, scripts: Array(8).fill('') }).free();
  kind(() => session.evaluate({ streaming: false, scripts: Array(9).fill('') }), 'LimitExceeded');
  session.evaluate({ streaming: false, scripts: ['a'.repeat(24), ''] }).free();
  kind(() => session.evaluate({ streaming: false, scripts: ['a'.repeat(25), ''] }), 'LimitExceeded');
});

test('the scripts JSON is refused at maxJsonBytes measured in UTF-8 bytes', () => {
  using lib = new wasm.ComponentLibrary('{}');
  using limits = new wasm.RuntimeLimits();
  limits.maxJsonBytes = 32n;
  using session = wasm.RuntimeSession.withLimits(lib, undefined, limits);
  const draw = (scripts: string[], error?: string) => session.evaluate({ streaming: false, isBundle: true, scripts, error }).free();
  // `["é…é"]`: 2 + 2 + 2 bytes per é. Under the limit in UTF-16 units either way.
  draw(['é'.repeat(14)]);
  kind(() => draw(['é'.repeat(15)]), 'LimitExceeded');
  // Four bytes per astral scalar, two units.
  draw(['😀'.repeat(7)]);
  kind(() => draw(['😀'.repeat(8)]), 'LimitExceeded');
  // A control character is one byte of name and six of JSON.
  draw(['\u0001'.repeat(4)]);
  kind(() => draw(['\u0001'.repeat(5)]), 'LimitExceeded');
  // A lone surrogate crosses as a four-byte stand-in.
  draw(['a\ud800'.repeat(5)]);
  kind(() => draw(['a\ud800'.repeat(6)]), 'LimitExceeded');
  // The error text counts its bytes beside the names.
  draw([], 'é'.repeat(16));
  kind(() => draw([], 'é'.repeat(16) + 'e'), 'LimitExceeded');
});
