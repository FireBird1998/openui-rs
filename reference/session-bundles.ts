// MIT, thesysdev/openui 47b9cae0, useOpenUIState.ts. Replay the renderer's
// response gates, scriptDeps, parse exception and loading projection around
// the real bundle reader, streaming parser, manager, action loop and store.
import { readFileSync, writeFileSync } from 'node:fs';
import { parseResponseBundle } from './responseBundle';
import { createStreamingParser } from './lang-core/src/parser/parser';
import { createQueryManager } from './lang-core/src/runtime/queryManager';
import { createStore } from './lang-core/src/runtime/store';
import { evaluateRoot } from './lang-core/src/runtime/evaluate-tree';
import { registerMutations } from './mutation-registration';
import { rendererState, unwrapFieldValue } from './renderer-actions';

// The blocked corpus also records `blocked`, which the renderer hands its
// components as `isStreaming` (useOpenUIState.ts lines 92 and 483), so they
// draw buttons and inputs disabled. The first corpus's rows predate it.
for (const [input, output, withBlocked] of [
  ['session-bundles.json', 'session-bundles.jsonl', false],
  ['session-blocked.json', 'session-blocked.jsonl', true],
] as const) {
  const corpus = JSON.parse(readFileSync('../conformance/' + input, 'utf8'));
  const rows: string[] = [];
  for (const { name, steps } of corpus.scenarios) {
    const calls = [], events = [];
    const manager = createQueryManager({ callTool(toolName, args) {
      return new Promise((resolve, reject) => calls.push({ toolName, args, resolve, reject }));
    } });
    const store = createStore(), parser = createStreamingParser(corpus.schema);
    const ctx = { getState: n => unwrapFieldValue(store.get(n)),
      resolveRef: n => manager.getMutationResult(n) || manager.getResult(n) };
    const resultRef = { current: undefined };
    const app = new Map();
    let bundle, streaming, blocked, root;
    let paused = false;
    let settled = { names: new Set<string>(), revision: null as string | null };
    try {
      for (const step of steps) {
        const first = calls.length, state = store.getSnapshot();
        let register = false;
        if ('response' in step) {
          const previous = bundle, wasStreaming = streaming, wasBlocked = blocked;
          streaming = !!step.streaming;
          bundle = parseResponseBundle(step.response, streaming);
          blocked = streaming || !bundle.complete || !!bundle.error;
          if (!blocked) settled = { names: bundle.scripts, revision: bundle.scriptRevision ?? null };
          // The hook memoizes the parse on program, streaming and parser error.
          const reparse = previous?.program !== bundle.program || wasStreaming !== streaming ||
            previous?.error !== bundle.error;
          if (reparse) {
            resultRef.current = parser.set(bundle.program, !streaming);
            store.initialize(resultRef.current.stateDeclarations ?? {}, {});
          }
          register = reparse || wasBlocked !== blocked;
        }
        // Model unmount and remount as in paused-actions.ts. The real
        // manager retains its query results across dispose/activate.
        if (step.pause) {
          manager.dispose();
          paused = true;
        }
        if (step.resume) {
          manager.activate();
          paused = false;
          register = true;
        }
        const bundleBlocked = bundle.isBundle && blocked;
        if ('set' in step) store.set(step.set, step.value);
        if ('extra' in step) app.set(step.extra, step.values);
        if ('click' in step) {
          const renderer = rendererState({ store, evaluationContext: ctx, schema: corpus.schema, resultRef,
            queryManager: manager, propsRef: { current: { onAction: e => events.push(e) } }, bundleBlocked });
          void renderer.triggerAction('click', undefined, step.click === 'legacy' ? { type: 'custom' } :
            step.click === 'default' ? undefined : root.props[step.click]);
        }
        if (step.retry && !bundleBlocked) {
          const errors = manager.getSnapshot().__openui_errors.filter(e => e.source === 'query');
          manager.invalidate(errors.map(e => e.statementId));
        }
        if ('finish' in step) {
          if (step.error) calls[step.finish].reject(new Error(step.error));
          else calls[step.finish].resolve(step.value);
        }
        await new Promise<void>(resolve => setImmediate(resolve));
        // evaluateStatements itself ignores replacements while disposed.
        const queryInputsChanged = 'response' in step || step.resume || 'extra' in step || state !== store.getSnapshot();
        if (!blocked && queryInputsChanged) {
          manager.evaluateStatements(resultRef.current.queryStatements, ctx, {
            // The session combines its app hook and the renderer hook at the single lang-core seam.
            extraDeps: tool => Object.assign({}, app.get(String(tool)),
              settled.names.has(tool) ? { __openuiScriptRevision: settled.revision } : undefined),
          });
        }
        if (!paused && register) registerMutations(blocked, resultRef.current, ctx, manager);
        const errors = [];
        root = evaluateRoot(resultRef.current, { ctx, library: { components: {} } as any, store, errors });
        const snapshot = manager.getSnapshot();
        rows.push(JSON.stringify({ name, tree: { root, errors }, snapshot, ...(withBlocked ? { blocked } : {}),
          isQueryLoading: snapshot.__openui_loading.length > 0 ||
            (streaming && bundleBlocked && resultRef.current.queryStatements.length > 0),
          parserError: bundle.error ? { source: 'parser', code: 'parse-failed', message: bundle.error } : null,
          calls: calls.slice(first).map(({ toolName, args }) => ({ toolName, args })), events: events.splice(0) }));
      }
    } finally {
      manager.dispose();
      store.dispose();
    }
  }
  writeFileSync('../conformance/golden/' + output, rows.join('\n') + '\n');
}
