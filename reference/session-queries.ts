// MIT, upstream 47b9cae0, packages/react-lang/src/hooks/useOpenUIState.ts.
// Lines 92, 193–205, 207–233 and 455–465. Execute the effects synchronously
// after a changed dependency and observe the subsequent draw. React scheduling
// is outside this harness. These plain tools have no Cloud script revision.
import { readFileSync, writeFileSync } from 'node:fs';
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse } = await import('./lang-core/src/parser/parser');
const { createQueryManager } = await import('./lang-core/src/runtime/queryManager');
const { createStore } = await import('./lang-core/src/runtime/store');
const { evaluateRoot } = await import('./lang-core/src/runtime/evaluate-tree');
const { rendererState, unwrapFieldValue } = await import('./renderer-actions');

const scriptDeps = () => undefined;
function queryEffect(useEffect, isStreaming, bundle, result, queryManager, evaluationContext, storeSnapshot) {
  const blocked = isStreaming || !bundle.complete || !!bundle.error;
  useEffect(() => {
    if (blocked) return;
    queryManager.evaluateStatements(result?.queryStatements ?? [], evaluationContext, {
      extraDeps: scriptDeps,
    });
  }, [blocked, result?.queryStatements, queryManager, evaluationContext, scriptDeps, storeSnapshot]);
}

const corpus = JSON.parse(readFileSync('../conformance/session-queries.json', 'utf8'));
const selfWait = process.argv[2] === '--self-wait';
if (selfWait) corpus.scenarios = (await import('./query-self-wait')).rendererSelfCases();
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
  const writer = rendererState({ store, evaluationContext: ctx, schema: corpus.schema,
    resultRef: {}, queryManager: manager, propsRef: { current: {} } });
  let previous: unknown[] | undefined;
  function useEffect(run: () => void, deps: unknown[]) {
    if (!previous || deps.some((d, i) => !Object.is(d, previous[i]))) run();
    previous = deps;
  }
  let result, streaming = false, bundle = { complete: true, error: undefined };
  try {
    for (const step of scenario.steps) {
      const first = calls.length;
      const before = JSON.stringify(manager.getSnapshot());
      if ('replace' in step || 'evaluate' in step) {
        streaming = !!step.streaming;
        bundle = { complete: step.complete ?? true, error: step.error };
      }
      if ('replace' in step) {
        result = parse(step.replace, schema);
        store.initialize(result.stateDeclarations ?? {}, {});
      }
      if ('set' in step || 'binding' in step) store.set('$n', step.set ?? step.binding);
      if ('field' in step) writer.setFieldValue(undefined, 'Input', '$n', step.field, true);
      if ('finish' in step) await calls[step.finish].finish(step);
      if ('invalidate' in step) manager.invalidate(step.invalidate);
      queryEffect(useEffect, streaming, bundle, result, manager, ctx, store.getSnapshot());

      const errors = [];
      const root = evaluateRoot(result, { ctx, library: { components: {} } as any, store, errors });
      const snapshot = manager.getSnapshot();
      rows.push(JSON.stringify({ tree: { root, errors }, snapshot,
        calls: calls.slice(first).map(({toolName, args}) => ({toolName, args})),
        changed: before !== JSON.stringify(snapshot) }));
    }
  } finally {
    manager.dispose();
    store.dispose();
  }
}
writeFileSync(selfWait ? '../conformance/golden/session-self-wait.jsonl' : '../conformance/golden/session-queries.jsonl', rows.join('\n') + '\n');
