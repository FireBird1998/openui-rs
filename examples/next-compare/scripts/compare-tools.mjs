// Run from reference/ with bun --no-install after scripts/wasm.sh.
import assert from 'node:assert/strict';
import wasm from '../../../target/wasm-pkg/node/openui_wasm.js';
import { program, schema, transcript } from '../../../reference/tool-provider-scenarios.ts';

export function wasmHost(limits, source = program) {
  const library = new wasm.ComponentLibrary(JSON.stringify(schema));
  const session = limits ? wasm.RuntimeSession.withLimits(library, undefined, limits) : new wasm.RuntimeSession(library);
  const parsed = wasm.parseProgram(source, library);
  session.setProgram(parsed);
  session.initialize();
  let response = { streaming: false };
  return {
    provider(value, options) { session.setToolProvider(value, options); },
    draw(state) {
      // A synchronous onChange can read the snapshot during this draw.
      const previous = response;
      response = typeof state === 'boolean' ? { streaming: state } : state;
      let output;
      try { output = session.evaluate(response); }
      catch (error) { response = previous; throw error; }
      try {
        return { tree: JSON.parse(output.json), snapshot: JSON.parse(output.querySnapshotJson),
          isQueryLoading: output.isQueryLoading, parserError: JSON.parse(output.parserErrorJson ?? 'null') };
      }
      finally { output.free(); }
    },
    snapshot() { return this.draw(response).snapshot; },
    set(n) { session.set('$n', JSON.stringify(n)); },
    extra(tool, values) { session.setQueryExtraValues(tool, JSON.stringify(values)); },
    retry() { session.retryFailedQueries(); },
    dispose() { session.free(); parsed.free(); library.free(); },
  };
}

if (import.meta.main) {
  const { referenceHost } = await import('../../../reference/tool-provider.ts');
  const reference = referenceHost(), rust = wasmHost();
  try {
    const upstream = await transcript(reference);
    const driven = await transcript(rust);
    assert.equal(driven, upstream);
    const expected = upstream.trimEnd().split('\n').map(line => JSON.parse(line));
    for (const [i, line] of driven.trimEnd().split('\n').entries()) {
      const {label, tree, snapshot} = JSON.parse(line);
      console.log(JSON.stringify({ label, upstream: expected[i].tree.root.props.value, wasm: tree.root.props.value,
        errors: snapshot.__openui_errors.map(error => error.code) }));
    }
  } finally { reference.dispose(); rust.dispose(); }
}
