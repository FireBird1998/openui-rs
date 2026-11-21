// #117: the pinned renderer's action loop against a disposed query manager.
// Pause is the manager's dispose (the renderer unmounting); resume is
// activate with the mount's query evaluation and mutation registration.
// Each row lists the calls a step started, the host events and `$n`.
import { readFileSync, writeFileSync } from 'node:fs';
import { compileSchema, parse } from './lang-core/src/parser/parser';
import { createStore } from './lang-core/src/runtime/store';
import { createQueryManager } from './lang-core/src/runtime/queryManager';
import { evaluateRoot } from './lang-core/src/runtime/evaluate-tree';
import { rendererState, unwrapFieldValue } from './renderer-actions';
import { registerMutations } from './mutation-registration';

process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const corpus = JSON.parse(readFileSync('../conformance/paused-actions.json', 'utf8'));
const rows = [];
for (const scenario of corpus.scenarios) {
  const store = createStore();
  const calls = [];
  const events = [];
  const manager = createQueryManager({
    callTool(toolName, args) {
      return new Promise((resolve, reject) => calls.push({ toolName, resolve, reject }));
    },
  });
  const ctx = {
    getState: name => unwrapFieldValue(store.get(name)),
    resolveRef: name => manager.getMutationResult(name) ?? manager.getResult(name),
  };
  const schema = compileSchema(corpus.schema);
  const resultRef = { current: parse(scenario.program, schema) };
  store.initialize(resultRef.current.stateDeclarations ?? {}, {});
  const renderer = rendererState({
    store, evaluationContext: ctx, schema: corpus.schema, resultRef, queryManager: manager,
    propsRef: { current: { onAction: event => events.push(event) } },
  });
  const mount = () => {
    manager.evaluateStatements(resultRef.current.queryStatements ?? [], ctx);
    registerMutations(false, resultRef.current, ctx, manager);
  };
  mount();
  try {
    for (const step of scenario.steps) {
      const initial = calls.length;
      if (step.pause) manager.dispose();
      if (step.resume) { manager.activate(); mount(); }
      if (step.click) {
        const errors = [];
        const root = evaluateRoot(resultRef.current, { ctx, library: { components: {} } as any, store, errors });
        void renderer.triggerAction('click', undefined, root.props.action);
      }
      if ('finish' in step) {
        const call = calls[step.finish];
        if ('failed' in step) call.reject(new Error(step.failed));
        else call.resolve(step.value);
      }
      await new Promise<void>(resolve => setImmediate(resolve));
      rows.push(JSON.stringify({
        calls: calls.slice(initial).map(call => call.toolName),
        events: events.splice(0),
        n: unwrapFieldValue(store.get('$n')),
      }));
    }
  } finally {
    manager.dispose();
    store.dispose();
  }
}
writeFileSync('../conformance/golden/paused-actions.jsonl', rows.join('\n') + '\n');
