// Shared inputs and observations. Each side supplies its own renderer operations.
export const schema = { $defs: { Box: { properties: { value: {} } } } };
export const program = '$n=1\nq=Query("read",{n:$n},"waiting")\nroot=Box(q)';

export async function nextToolTurn() {
  // Let promise reactions schedule the next driver drain before observing it.
  await Bun.sleep(0);
  await new Promise<void>(resolve => setTimeout(resolve, 0));
}

export async function transcript(host) {
  const rows: string[] = [];
  const calls: unknown[] = [];
  const map = Object.assign(Object.create({ read(args) {
    calls.push(['map', this === undefined, args]);
    return Promise.resolve('first');
  } }), { callTool: 0 });
  host.provider(map);
  const observe = (label, streaming = false) => {
    const {tree, snapshot} = host.draw(streaming);
    rows.push(JSON.stringify({label, tree, snapshot, calls: calls.splice(0)}));
  };
  // Give the provider and the upstream manager's then/catch/finally a turn.
  const settled = async label => { await nextToolTurn(); observe(label); };
  observe('streaming', true);
  observe('settled');
  await settled('map success');

  class Client {
    async callTool(request) {
      calls.push(['MCP', this === client, request]);
      return { structuredContent: { result: 2 }, content: [{ type: 'text', text: 'ignored' }] };
    }
  }
  const client = new Client();
  host.provider(client);
  observe('swapped');
  host.set(2);
  observe('refetch');
  await settled('MCP success');

  for (const [name, provider] of [
    ['MCP error', { callTool: () => ({ isError: true, content: [{ type: 'text', text: 'bad' }] }) }],
    ['missing', { zebra() {}, read: null, apple() {} }],
    ['rejection', { read: () => Promise.reject(new Error('offline')) }],
    ['thrown string', { read() { throw 'denied'; } }],
    ['undefined result', { read() {} }],
  ]) {
    host.provider(provider);
    host.retry();
    await settled(name);
  }
  return rows.join('\n') + '\n';
}

// Observe the received value compactly so a size-limit fixture fits in git.
export async function limitTranscript(host) {
  let nested: unknown = 'leaf';
  for (let i = 0; i < 257; i++) nested = [nested];
  const rows: string[] = [];
  const bindingErrors: string[] = [];
  for (const [label, value] of [
    ['deep', nested], ['wide', 'x'.repeat(8 * 1024 * 1024)], ['session limit', 'z'.repeat(8192)],
  ]) {
    host.provider({ read: () => value }, {onError: error => bindingErrors.push(error.kind)});
    if (rows.length) host.retry();
    else host.draw(false);
    await nextToolTurn();
    const snapshot = host.snapshot();
    let data = snapshot.q, depth = 0;
    for (; depth < 258 && Array.isArray(data); depth++) data = data[0];
    rows.push(JSON.stringify({ label, depth, length: data.length, sample: data.slice(0, 8),
      loading: snapshot.__openui_loading, refetching: snapshot.__openui_refetching, errors: snapshot.__openui_errors, bindingErrors }));
  }
  return rows.join('\n') + '\n';
}

export const sharedProgram = Array.from({length: 4}, (_, i) => 'q' + i + '=Query("read",{})').join('\n');

export async function failureTranscript(host) {
  const bindingErrors: string[] = [];
  let calls = 0;
  host.provider({ read() { calls++; throw Error('offline'); } }, {onError: error => bindingErrors.push(error.kind)});
  host.draw(false);
  await nextToolTurn();
  const failed = host.snapshot();
  host.provider({ read() { calls++; return 1; } });
  host.retry();
  await nextToolTurn();
  return JSON.stringify({failed, retried: host.snapshot(), calls, bindingErrors}) + '\n';
}

// Observe each native JS result before projecting the observation to JSON.
export async function nonJsonTranscript(createHost) {
  const rows = [];
  for (const [label, provider] of [
    ['function', { read: () => function result() {} }],
    ['undefined MCP response', { callTool: async () => undefined }],
    ['BigInt', { read: () => 42n }],
    ['nested function', { read: () => ({a: function result() {}}) }],
    ['NaN', { read: () => NaN }],
    ['nested NaN', { read: () => [NaN] }],
    ['Infinity', { read: () => Infinity }],
    ['negative Infinity', { read: () => -Infinity }],
    ['MCP nested function', { callTool: () => ({structuredContent: {a: function result() {}}}) }],
    ['MCP nested NaN', { callTool: () => ({structuredContent: [NaN]}) }],
  ]) {
    const host = createHost();
    const bindingErrors = [];
    try {
      host.provider(provider, {onError: error => bindingErrors.push(error.kind)});
      host.draw(false);
      await nextToolTurn();
      const {q, ...status} = host.snapshot();
      const value = typeof q === 'function' ? q.name : typeof q === 'bigint' ? String(q) : q;
      rows.push(JSON.stringify({label, type: typeof q, value, ...status, bindingErrors}, (_key, item) => {
        if (typeof item === 'function') return {type: 'function', name: item.name};
        if (typeof item === 'number' && !Number.isFinite(item)) return {type: 'number', value: String(item)};
        return item;
      }));
    } finally { host.dispose(); }
  }
  return rows.join('\n') + '\n';
}

export async function diagnosticTranscript(createHost) {
  const rows = [];
  for (const [label, provider] of [
    ['long rejection', { read() { throw Error('x'.repeat(1024)); } }],
    ['long available name', { ['x'.repeat(1024)]: () => 1 }],
    ['invalid MCP response', { callTool: () => null }],
  ]) {
    const host = createHost();
    const bindingErrors = [];
    try {
      host.provider(provider, {onError: error => bindingErrors.push(error.kind)});
      host.draw(false);
      await nextToolTurn();
      rows.push(JSON.stringify({label, snapshot: host.snapshot(), bindingErrors}));
    } finally { host.dispose(); }
  }
  return rows.join('\n') + '\n';
}

// Observe the renderer before and after the newline that admits each marker.
export async function rawTranscript(host) {
  const calls: string[] = [];
  let deliveries = 0;
  host.provider({cloud: () => { calls.push('cloud'); return 'cloud ' + ++deliveries; },
    other: () => { calls.push('other'); return 'local'; }});
  const rows = [];
  const query = 'q=Query("cloud",{},"wait")\nr=Query("other",{},"other")\nroot=Box([q,r])';
  const frame = ']]>openui:content?name=New+report&library=ui&library=two\n';
  const section = '\n]]>openui:scripts {"count":1}\n';
  const scripts = '[{"name":"cloud","code":"A"}]';
  const tail = '\n]]>openui:context\n{"page":2}\n]]>openui:end';
  const full = frame + query + section + scripts + tail;
  for (const [label, text, streaming] of [
    ['empty', null, true],
    ['bare', 'root=Box("first")', false],
    ['opening marker', 'root=Box("first")\n]]>openui:con', true],
    ['mid-line retry', 'root=Box("first")]]>openui:con', true],
    ['legacy scripts header held', query + section.trimEnd(), true],
    ['legacy scripts payload', query + section + '[', true],
    ['legacy scripts valid', query + section + scripts, true],
    ['open frame', frame + query, true],
    ['scripts marker', frame + query + '\n]]>openui:scri', true],
    ['scripts header', frame + query + section.trimEnd(), true],
    ['scripts payload', frame + query + section + '[{"name":', true],
    ['end without newline', full, true],
    ['end with newline', full + '\n', true],
    ['settled', full, false],
    ['delivered', full, false],
    ['invalid', frame + 'root=Box("visible")' + section + 'root=Box("leaked")', false],
    ['edited code streaming', full.replace('"A"', '"B"'), true],
    ['edited code settled', full.replace('"A"', '"B"'), false],
    ['refetched', full.replace('"A"', '"B"'), false],
  ] as const) {
    const result = host.raw(text, streaming);
    const clicked = await host.click();
    rows.push(JSON.stringify({label, ...result, clicked, calls: calls.splice(0)}));
    await nextToolTurn();
  }
  return rows.join('\n') + '\n';
}

// Keep the reference side of the package's input-limit and string-boundary differences.
export const rawBoundaryCases = [
  {label: 'marker bytes', text: ']]>openui:content?name=' + 'é'.repeat(64) + '\nroot=Box("small")', streaming: false},
  {label: 'scripts bytes', text: 'root=Box("small")\n]]>openui:scripts {"count":1}\n' +
    JSON.stringify([{name:'cloud',code:'é'.repeat(64)}]), streaming: false},
  {label: 'context depth', text: 'root=Box("small")\n]]>openui:context\n' + '['.repeat(36) + '0' + ']'.repeat(36), streaming: false},
  {label: 'split surrogate', text: 'root=Box("a\ud83d', streaming: true},
  {label: 'paired surrogate', text: 'root=Box("a😀")', streaming: false},
];
