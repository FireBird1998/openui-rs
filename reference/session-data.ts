// #58 / #60 observations from the pinned evaluator and renderer-action body.
// Renderer evidence at upstream commit 47b9cae0d3b26a8c9d7514d5887a6d6608801ac9:
// packages/react-lang/src/Renderer.tsx:147-165 (renderDeep),
// packages/react-lang/src/hooks/useOpenUIState.ts:350-415 (triggerAction),
// packages/react-ui/src/genui-lib/EditableTable/index.tsx:31-42,55-63 (toRows).
import { readFileSync, writeFileSync } from 'node:fs';
process.env.OPENUI_TELEMETRY_DISABLED = '1';
process.env.DO_NOT_TRACK = '1';
const { compileSchema, parse, createStreamParser } = await import('./lang-core/src/parser/parser');
const { evaluateRoot } = await import('./lang-core/src/runtime/evaluate-tree');
const { createStore } = await import('./lang-core/src/runtime/store');
const { rendererState } = await import('./renderer-actions');
const table = JSON.parse(readFileSync('../conformance/imported/schemas/react-ui.json', 'utf8')).$defs.EditableTable;
const schema = { $defs: { Box: { properties: { value: {}, action: {} } }, EditableTable: table }, actions: { Go: { params: {} } } };
const compiled = compileSchema(schema);
const rows = [];
for (const dynamic of [false, true]) for (const source of ['state', 'query', 'tool']) {
  const element = { type: 'element', typeName: 'Box', props: { action: { steps: [
    { type: 'set', target: '$n', valueAST: { k: 'Num', v: 7 } },
    { type: 'custom_action', name: 'Go', params: {} },
  ] } }, partial: false, hasDynamicProps: dynamic };
  const program = source === 'state' ? '$n=0\n$data=null\nroot=Box($data)' : '$n=0\nq=Query("q",{})\nroot=Box(q)';
  const saved = source === 'state' ? { $data: element } : {};
  const references = source === 'state' ? {} : { q: element };
  const parsed = parse(program, compiled), store = createStore();
  store.initialize(parsed.stateDeclarations ?? {}, saved);
  const errors = [], ctx = { getState: n => store.get(n), resolveRef: n => references[n] };
  const root = evaluateRoot(parsed, { ctx, library: { components: {} } as any, store, errors });
  const events = [];
  const runner = rendererState({ store, evaluationContext: ctx, schema, resultRef: { current: parsed }, queryManager: {}, propsRef: { current: { onAction: e => events.push(e) } } });
  await runner.triggerAction('', undefined, root.props.value.props.action);
  rows.push({ name: `${source}-dynamic-${dynamic}`, program, saved, references, treeJson: JSON.stringify({ root, errors }), snapshot: store.getSnapshot(), events });
  store.dispose();
}
{
  const program = 'root=Box({k:"Str",v:"y",z:2})', parsed = parse(program, compiled), store = createStore(), errors = [];
  const root = evaluateRoot(parsed, { ctx: { getState: () => null, resolveRef: () => null }, library: { components: {} } as any, store, errors });
  rows.push({ name: 'expression-kind-literal', program, saved: {}, references: {}, treeJson: JSON.stringify({ root, errors }), snapshot: {}, events: [] });
  store.dispose();
}
// A source transcription of the renderer's unconditional toRows operation.
// Both comparison panes use that same renderer; neither has a partial-row filter.
const program = 'root=EditableTable("t",[{type:"text",key:"a",header:"A"}],[{id:"r",values:["x"]}])';
const stream = createStreamParser(compiled), streamRows = [];
for (let i = 0; i < program.length; i++) {
  const parsed = stream.push(program[i]), store = createStore(), errors = [];
  const root = evaluateRoot(parsed, { ctx: { getState: () => null, resolveRef: () => null }, library: { components: {} } as any, store, errors });
  let rendererThrows = false;
  if (root) try {
    const columns = root.props.columns;
    const columnKeys = columns.map(c => c.key);
    root.props.data.map(row => {
      const values = columnKeys.reduce((acc, key, index) => { acc[key] = row.values[index]; return acc; }, {});
      return { ...values, id: row.id };
    });
  } catch { rendererThrows = true; }
  streamRows.push({ prefix: program.slice(0, i + 1), treeJson: JSON.stringify({ root, errors }), rendererThrows });
  store.dispose();
}
writeFileSync('../conformance/golden/runtime/session-data.json', JSON.stringify({ schema, rows, program, streamRows }) + '\n');
console.log(`Session data: ${rows.length} data observations; ${streamRows.length} table prefixes, ${streamRows.filter(r => r.rendererThrows).length} renderer throws.`);
