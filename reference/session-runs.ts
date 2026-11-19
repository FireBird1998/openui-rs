// MIT, upstream 47b9cae0, packages/react-lang/src/hooks/useOpenUIState.ts.
// A session with a tool provider: a `@Run` of a query in an action plan and
// the renderer's Retry. Clicks go through the transcribed triggerAction
// (reference/renderer-actions.ts, which calls queryManager.invalidate for a
// query run); Retry transcribes retryQueries (lines 444–451) and the query
// effect transcribes lines 207–233, as reference/session-queries.ts does.
// Effects run synchronously after a changed dependency, then the tree is drawn.
// Lines 91–95 give two conditions: `blocked` (streaming, or a bundle that is
// incomplete or has an error) gates the query effect; `bundleBlocked`
// (`isBundle && blocked`) gates clicks and Retry. A scenario with `bundle`
// stands for a response bundle; otherwise the program is bare.
import { readFileSync, writeFileSync } from 'node:fs';
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse } = await import('./lang-core/src/parser/parser');
const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
const { createStore } = await import('./lang-core/src/runtime/store');
const { evaluateRoot } = await import('./lang-core/src/runtime/evaluate-tree');
const { rendererState, unwrapFieldValue } = await import('./renderer-actions');

const scriptDeps = () => undefined;
function queryEffect(useEffect, blocked, result, queryManager, evaluationContext, storeSnapshot) {
  useEffect(() => {
    if (blocked) return;
    queryManager.evaluateStatements(result?.queryStatements ?? [], evaluationContext, {
      extraDeps: scriptDeps,
    });
  }, [blocked, result?.queryStatements, queryManager, evaluationContext, scriptDeps, storeSnapshot]);
}

console.error = () => {}; // Scripted rejections are observed through the snapshot.
const corpus = JSON.parse(readFileSync('../conformance/session-runs.json', 'utf8'));
const schema = compileSchema(corpus.schema);
const rows: string[] = [];
for (const scenario of corpus.scenarios) {
  const store = createStore();
  const calls = [];
  const manager = createQueryManager({ callTool(toolName, args) {
    // Resume after executeFetch's already-queued then/catch/finally.
    const promise = new Promise((resolve, reject) => calls.push({ toolName, args,
      async finish(step) {
        if ('error' in step) reject(new Error(step.error)); else resolve(step.value);
        await promise.catch(() => {});
      } }));
    return promise;
  } });
  const ctx = {
    getState: name => unwrapFieldValue(store.get(name)),
    resolveRef: name => manager.getMutationResult(name) || manager.getResult(name),
  };
  const events = [];
  const resultRef = { current: undefined };
  let previous: unknown[] | undefined;
  function useEffect(run: () => void, deps: unknown[]) {
    if (!previous || deps.some((d, i) => !Object.is(d, previous[i]))) run();
    previous = deps;
  }
  let result, root, blocked = false;
  const isBundle = !!scenario.bundle;
  try {
    for (const step of scenario.steps) {
      const first = calls.length;
      const before = JSON.stringify(manager.getSnapshot());
      events.length = 0;
      if ('replace' in step || 'evaluate' in step) {
        blocked = !!step.streaming || step.complete === false || !!step.error;
      }
      const bundleBlocked = isBundle && blocked;
      if ('replace' in step) {
        result = resultRef.current = parse(step.replace, schema);
        store.initialize(result.stateDeclarations ?? {}, {});
      }
      if ('finish' in step) await calls[step.finish].finish(step);
      if ('click' in step) {
        const writer = rendererState({ store, evaluationContext: ctx, schema: corpus.schema, resultRef,
          queryManager: manager, propsRef: { current: { onAction: event => events.push(event) } }, bundleBlocked });
        await writer.triggerAction('clicked', undefined, root.props[step.click]);
      }
      if ('retry' in step) {
        // retryQueries, with querySnapshot read from the manager.
        const queryErrors = manager.getSnapshot().__openui_errors.filter(error => error.source === 'query');
        if (!bundleBlocked) manager.invalidate(queryErrors.map(error => error.statementId));
      }
      queryEffect(useEffect, blocked, result, manager, ctx, store.getSnapshot());

      const errors = [];
      root = evaluateRoot(result, { ctx, library: { components: {} } as any, store, errors });
      const snapshot = manager.getSnapshot();
      rows.push(JSON.stringify({ tree: { root, errors }, snapshot,
        calls: calls.slice(first).map(({toolName, args}) => ({toolName, args})),
        events: [...events],
        changed: before !== JSON.stringify(snapshot) }));
    }
  } finally {
    manager.dispose();
    store.dispose();
  }
}
writeFileSync('../conformance/golden/session-runs.jsonl', rows.join('\n') + '\n');
