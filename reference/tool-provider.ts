// Renderer.tsx 267–289 and useOpenUIState.ts query effect and Retry at
// 47b9cae0 (MIT, react-lang-LICENSE). No React scheduler is required.
import { writeFileSync } from 'node:fs';
import { compileSchema, createStreamingParser, parse } from './lang-core/src/parser/parser';
import { createQueryManager } from './lang-core/src/runtime/queryManager';
import { ToolNotFoundError } from './lang-core/src/runtime/toolProvider';
import { extractToolResult } from './lang-core/src/runtime/mcp';
import { createStore } from './lang-core/src/runtime/store';
import { evaluateRoot } from './lang-core/src/runtime/evaluate-tree';
import { parseResponseBundle } from './responseBundle';
import { parseMessage } from './lang-core/src/message';
import { rendererState } from './renderer-actions';
import { schema, program, transcript, limitTranscript, failureTranscript, nonJsonTranscript, diagnosticTranscript, sharedProgram, rawTranscript, rawBoundaryCases } from './tool-provider-scenarios';

export function referenceHost(source = program) {
  let current;
  const manager = createQueryManager({ async callTool(name, args) {
    if (typeof current.callTool === 'function') {
      return extractToolResult(await current.callTool({ name, arguments: args }));
    }
    const fn = current[name];
    if (!fn) throw new ToolNotFoundError(name, Object.keys(current));
    return fn(args);
  } });
  let result = parse(source, compileSchema(schema));
  const store = createStore();
  store.initialize(result.stateDeclarations ?? {}, {});
  const ctx = { getState: name => store.get(name), resolveRef: name => manager.getResult(name) };
  const extra = new Map();
  let bundleBlocked = false;
  const parser = createStreamingParser(schema);
  let previous: {program: string; streaming: boolean; error?: string} | undefined;
  return {
    provider(value) { current = value; },
    draw(state: boolean | { streaming: boolean; isBundle?: boolean; complete?: boolean;
      error?: string | null; scripts?: string[]; scriptRevision?: string | null } = false) {
      const { streaming, isBundle, complete = true, error, scripts = [], scriptRevision = null } =
        typeof state === 'boolean' ? { streaming: state } : state;
      const blocked = streaming || !complete || !!error;
      bundleBlocked = !!isBundle && blocked;
      if (!blocked) {
        manager.evaluateStatements(result.queryStatements ?? [], ctx, {
          extraDeps: tool => Object.assign({}, extra.get(tool),
            scripts.includes(tool) ? { __openuiScriptRevision: scriptRevision } : undefined),
        });
      }
      const errors = [];
      const root = evaluateRoot(result, { ctx, library: { components: {} } as any, store, errors });
      const snapshot = manager.getSnapshot();
      return { tree: { root, errors }, snapshot,
        isQueryLoading: snapshot.__openui_loading.length > 0 ||
          (streaming && bundleBlocked && (result.queryStatements?.length ?? 0) > 0),
        parserError: error ? { source: 'parser', code: 'parse-failed', message: error } : null };
    },
    raw(text: string | null, streaming: boolean) {
      const bundle = parseResponseBundle(text, streaming);
      const {context, attributes} = parseMessage(text ?? '', {streaming});
      if (previous?.program !== bundle.program || previous.streaming !== streaming || previous.error !== bundle.error) {
        result = parser.set(bundle.program, !streaming);
        store.initialize(result.stateDeclarations ?? {}, {});
      }
      previous = {program: bundle.program, streaming, error: bundle.error};
      return {...this.draw({...bundle, streaming, scripts: [...bundle.scripts]}),
        name: bundle.metadata.name, attributes, context, error: bundle.error};
    },
    async click() {
      const events = [];
      await rendererState({store, evaluationContext: ctx, schema, resultRef: {current: result},
        queryManager: manager, propsRef: {current: {onAction: e => events.push(e)}}, bundleBlocked})
        .triggerAction('click', undefined, undefined);
      return events.length;
    },
    snapshot() { return manager.getSnapshot(); },
    set(n) { store.set('$n', n); },
    extra(tool, values) { extra.set(tool, values); },
    retry() {
      if (bundleBlocked) return;
      const errors = manager.getSnapshot().__openui_errors.filter(e => e.source === 'query');
      manager.invalidate(errors.map(e => e.statementId));
    },
    dispose() { manager.dispose(); store.dispose(); },
  };
}

if (import.meta.main) {
  const host = referenceHost();
  try {
    writeFileSync('../conformance/golden/tool-provider.jsonl', await transcript(host));
  } finally { host.dispose(); }
  const raw = referenceHost('');
  try {
    writeFileSync('../conformance/golden/tool-provider-response.jsonl', await rawTranscript(raw));
    const bounds = rawBoundaryCases.map(({label, text, streaming}) => {
      const {tree, context, name} = raw.raw(text, streaming);
      return {label, value: tree.root?.props.value, context, name};
    });
    writeFileSync('../conformance/golden/tool-provider-response-boundaries.json', JSON.stringify(bounds) + '\n');
  } finally { raw.dispose(); }
  const limits = referenceHost();
  try {
    writeFileSync('../conformance/golden/tool-provider-limits.jsonl', await limitTranscript(limits));
  } finally { limits.dispose(); }
  writeFileSync('../conformance/golden/tool-provider-non-json.jsonl', await nonJsonTranscript(referenceHost));
  writeFileSync('../conformance/golden/tool-provider-diagnostics.jsonl', await diagnosticTranscript(referenceHost));
  const shared = referenceHost(sharedProgram);
  try {
    writeFileSync('../conformance/golden/tool-provider-error-limit.json', await failureTranscript(shared));
  } finally { shared.dispose(); }
}
