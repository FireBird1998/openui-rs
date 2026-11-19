// The pinned renderer's action loop awaits the real fireMutation promise.
// Observe tree, events and dispatch at the same boundaries as the session drain.
import { readFileSync, writeFileSync } from 'node:fs';
import { compileSchema, parse } from './lang-core/src/parser/parser';
import { createStore } from './lang-core/src/runtime/store';
import { createQueryManager } from './lang-core/src/runtime/queryManager';
import { evaluateRoot } from './lang-core/src/runtime/evaluate-tree';
import { rendererState, unwrapFieldValue } from './renderer-actions';
import { registerMutations } from './mutation-registration';

process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const corpus = JSON.parse(readFileSync('../conformance/session-mutations.json', 'utf8'));
const rows = [];
for (const scenario of corpus.scenarios) {
  const store = createStore();
  const calls = [];
  const events = [];
  const manager = createQueryManager({
    callTool(toolName, args) {
      return new Promise((resolve, reject) => calls.push({ toolName, args, resolve, reject }));
    },
  });
  const ctx = {
    getState: name => unwrapFieldValue(store.get(name)),
    resolveRef: name => manager.getMutationResult(name) ?? manager.getResult(name),
  };
  const resultRef = { current: null };
  const schema = compileSchema(corpus.schema);
  const renderer = rendererState({
    store, evaluationContext: ctx, schema: corpus.schema, resultRef, queryManager: manager,
    propsRef: { current: { onAction: event => events.push(event) } },
  });
  function draw() {
    const errors = [];
    const root = evaluateRoot(resultRef.current, { ctx, library: { components: {} } as any, store, errors });
    return { root, errors };
  }

  let streaming = true;
  try {
    for (const step of scenario.steps) {
      const before = JSON.stringify(manager.getSnapshot());
      const initial = calls.length;
      const state = store.getSnapshot();
      const wasStreaming = streaming;
      if ('replace' in step) {
        resultRef.current = parse(step.replace, schema);
        // A manager can register both kinds under one id. The surface parser
        // keeps the last declaration, so supply its two statement lists separately.
        if (step.alsoRegisterMutations) {
          resultRef.current.mutationStatements = parse(step.alsoRegisterMutations, schema).mutationStatements;
        }
        store.initialize(resultRef.current.stateDeclarations ?? {}, {});
      }
      if ('replace' in step || 'settle' in step) streaming = !!step.streaming;
      if ('set' in step) store.set(step.set, step.value);
      if ('click' in step) void renderer.triggerAction('click', undefined, draw().root.props.action);
      if ('finish' in step) {
        const call = calls[step.finish];
        if ('failed' in step) call.reject(new Error(step.failed));
        else call.resolve(step.value);
      }
      // Observe all microtasks from the resolved provider and awaited action
      // segment, including a segment that has already suspended a second time.
      await new Promise<void>(resolve => setImmediate(resolve));
      const register = 'replace' in step || (wasStreaming && !streaming);
      const wroteState = state !== store.getSnapshot();
      if (!streaming && (register || wroteState)) {
        manager.evaluateStatements(resultRef.current?.queryStatements ?? [], ctx);
      }
      if (register) registerMutations(streaming, resultRef.current, ctx, manager);
      const snapshot = manager.getSnapshot();
      rows.push(JSON.stringify({
        tree: draw(), snapshot, events: events.splice(0),
        calls: calls.slice(initial).map(({ toolName, args }) => ({ toolName, args })),
        ...('maxContinuations' in scenario ? { failure: null } : {}),
        changed: before !== JSON.stringify(snapshot) || ('finish' in step && wroteState),
      }));
    }
  } finally {
    manager.dispose();
    store.dispose();
  }
}
writeFileSync('../conformance/golden/session-mutations.jsonl', rows.join('\n') + '\n');
